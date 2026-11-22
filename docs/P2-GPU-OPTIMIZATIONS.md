# Phase 2: GPU adaptive damage, triple buffering, import filtering, sync and lifecycle

This phase extends the existing CEF 156 / Slint 1.19 / WGPU 30 Linux shared-
texture path. All new optimizations are opt-in. CPU mode and the existing GPU
path remain available as controls. The confirmed working configuration for the
target NVIDIA laptop uses `ASTER_CEF_OZONE=x11` (XWayland); native Wayland is
outside this phase.

## Independent switches

| CLI / environment | Values | Default | Meaning |
| --- | --- | --- | --- |
| `--renderer` / `ASTER_RENDERER` | `cpu`, `gpu`, `auto` | `cpu` | Rendering path |
| `--buffers` / `ASTER_BUFFERS` | `1`, `2`, **`3`** | `1` | CPU supports 1/2; GPU supports 1/2/3 |
| `--gpu-damage` / `ASTER_GPU_DAMAGE` | `legacy`, `adaptive`, `full`, `partial` | `legacy` | Select full-vs-partial decision logic |
| `--gpu-threshold` / `ASTER_GPU_THRESHOLD_PCT` | 1..100 | `65` | Unique dirty-area percentage for adaptive full update |
| `--gpu-rect-penalty` / `ASTER_GPU_RECT_PENALTY_PIXELS` | 0..262144 | `4096` | Approximate overhead per partial scissor, in pixel-equivalent units |
| `--gpu-lazy-init` / `ASTER_GPU_LAZY_INIT` | `on`, `off` | `off` | Initialize only the first GPU presentation slot eagerly |
| `--gpu-sync` / `ASTER_GPU_SYNC` | `queue`, `wait` | `queue` | Queue ordering, or explicit blocking completion wait for A/B |
| `--gpu-import-filter` / `ASTER_GPU_IMPORT_FILTER` | `off`, `empty` | `off` | Skip a CEF full import only when `dirtyRects=Some([])` and a previously presented GPU image exists |
| `ASTER_LIFECYCLE` | `0`, `1` | `0` | Log Slint teardown and CEF shutdown acknowledgment |
| `ASTER_PERF` | `0`, `1` | `0` | Report GPU performance estimates every ~2 seconds |

CLI overrides environment values. All flags are handled by the desktop
application before launching a browser; unsupported values are errors. If
`--renderer auto --buffers 3` falls back to CPU, the CPU frontend uses at most
two buffers. `--renderer cpu --buffers 3` is rejected explicitly.

## GPU damage planning

Legacy: sum the clipped `dirtyRects`, fall back to full beyond 64 rectangles
or 65% of the frame. This **overcounts overlaps**, kept solely for A/B.

Adaptive: clip damage to the framebuffer; calculate an exact union using
sorted horizontal strips, merge adjacent strips, and avoid drawing overlapping
regions. A full frame is selected if:

* the unique dirty area exceeds `--gpu-threshold` percent, or
* `unique_pixels + rect_count * gpu_rect_penalty >= frame_pixels`, or
* the input is pathological (>256 rectangles) or decomposition would require
  >96 scissor draws.

An empty or unspecified damage list remains a **full update** in the
presentation planner to avoid a stale image. `--gpu-damage=partial` forces
partial draws when safe, while `full` always draws the entire frame.

Each presentation buffer accumulates damage it missed while other buffers were
active. Two and three buffers therefore remain coherent without assuming that
a dirty region in the current frame is the only region the target slot needs.

In `--gpu-lazy-init=on`, new presentation textures are allocated at once but
only the first is populated initially. The others are fully populated on their
first actual use. In a 3-buffer workload this reduces startup/resize work,
but the first two subsequent updates may include additional full draws.

## Import copies and synchronization: supported scope and limitations

**Aster cannot yet issue a sub-region native DMA-BUF import.** The upstream
`cef-rs` 156 `SharedTextureHandle::import_texture` copies the entire CEF
resource to an owned WGPU texture **inside the CEF callback**, then blocks
until that copy completes. The handles are borrowed and invalid outside the
callback; blindly caching them or submitting a copy after the callback is
unsafe. A true partial import requires a separate, audited Vulkan external-
memory importer with explicit queue-family ownership and synchronization.

The independently enabled `--gpu-import-filter=empty` prevents the full
upstream import for *explicitly empty* dirty sets after the first valid frame.
Unknown (`None`) damage and the first frame still import in full. This may
have **no measurable benefit** if Chromium never produces empty-damage
callbacks; `import_filter_skips` and `import_saved_estimate` quantify when it
does. The copy for all other accelerated images is retained for correctness.

`--gpu-sync=queue` uses submission ordering on the Slint WGPU device/queue and
avoids adding an additional wait after each Aster presentation. It does not
remove `cef-rs`'s mandatory import wait. `--gpu-sync=wait` explicitly waits
up to two seconds for *Aster's presentation submission* to complete; it is a
control used to measure synchronization costs, not the recommended default.

## Lifecycle and orderly shutdown

`ASTER_LIFECYCLE=1` enables these observable milestones:

* `[aster:lifecycle] gpu_renderer_setup ...`
* `[aster:lifecycle] frontend_enter` and `slint_run_return ...`
* `[aster:lifecycle] gpu_renderer_teardown ...` and `slint_window_dropped`
* `[aster:lifecycle] frontend_exit ... starting_browser_close`
* `[aster:lifecycle] cef_shutdown_begin browsers_pending=...`
* `[aster:lifecycle] cef_shutdown_complete browsers_pending=0 ...`

`RenderingTeardown` is normal on window closure and is not itself a GPU
failure. CEF shutdown waits for `OnBeforeClose` acknowledgment for all native
browsers, with its previous 15-second timeout. Confirm closure with exit code
0 and no CEF shutdown timeout; do not infer closure correctness solely from
`gpu_renderer_teardown`.

## Manual A/B protocol

Use the same application window size, `ASTER_OSR_FPS`, manual page and
animation time in every experiment. Change **one variable at a time** and
save both stdout and stderr to a separate log file. Example for fish:

```fish
set -gx ASTER_PERF 1
set -gx ASTER_LIFECYCLE 1
set -gx ASTER_CEF_OZONE x11

# Reference GPU with legacy damage logic.
cargo run --release -p aster-desktop --features cef-gpu -- \
  --renderer gpu --buffers 1 --gpu-damage legacy 2>&1 | tee /tmp/aster-legacy.log

# Exact overlap accounting, otherwise same setup.
cargo run --release -p aster-desktop --features cef-gpu -- \
  --renderer gpu --buffers 1 --gpu-damage adaptive 2>&1 | tee /tmp/aster-adaptive.log

# Isolate the cost/benefit of the extra GPU presentation buffer.
cargo run --release -p aster-desktop --features cef-gpu -- \
  --renderer gpu --buffers 3 --gpu-damage adaptive 2>&1 | tee /tmp/aster-triple.log

python scripts/compare-rendering.py \
  legacy=/tmp/aster-legacy.log adaptive=/tmp/aster-adaptive.log \
  triple=/tmp/aster-triple.log
```

For triple-buffer startup benchmarking, compare `--gpu-lazy-init off` vs `on`.
For synchronization, compare `--gpu-sync queue` vs `wait`. For importer filtering,
compare `--gpu-import-filter off` vs `empty`. If no empty callbacks occur, a
zero `import_filter_skips` count is expected and no saving should be claimed.

Metrics in `[aster:gpu]` include:

* `importer_full_copy` (Mio/s estimated from dimensions; not GPU bandwidth),
  `import_wall_avg` (wall clock, includes upstream blocking GPU wait),
  `import_filter_skips`, `import_saved_estimate` (estimated avoided bytes);
* `presentation_writes` (estimated GPU shader writes), `full_regions`,
  `partial_regions`, `draws`, `lazy_full`;
* `raw_damage_pct` (sum of rect areas, may exceed 100%) and
  `unique_damage_pct` (exact union in all modes, including legacy), plus `overlap_saved_pct`;
* `plan_avg` CPU time spent planning, `submission_cpu_avg` total Aster
  composition+submission wall time, and `sync_wait_avg/max`.

The external comparer prints **arithmetic averages of periodic log windows**.
It is useful for relative A/B comparison but not a calibrated hardware timer,
end-to-end frame latency or total GPU memory throughput. Run controlled
repetitions before drawing conclusions.

## Validation gate

```sh
cargo fmt --all -- --check
cargo test -p browser-contracts
cargo test -p browser-render-wgpu
cargo test -p engine-cef --features gpu
cargo test -p aster-desktop --features cef-gpu
cargo check -p aster-desktop --features cef
cargo check -p aster-desktop --features cef-gpu
```

Check CPU 1/2-buffer visuals, GPU 1/2/3-buffer visuals (especially alternating
partial updates and resizing), tab close/reopen, no stale pixels, and exit
without a renderer teardown failure. On unsupported hardware, fallback to CPU
must still work. This proof requires real hardware; unit tests cover only the
plan decisions and neutral configuration boundaries.
