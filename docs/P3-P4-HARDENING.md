# Aster Browser — P3/P4 hardening (CEF 156, Slint 1.19, WGPU 30)

## Scope and verified limitations

P3: strict native Wayland preflight, ANGLE diagnostic variants, CPU-first or accelerated probe, bounded fallback on missing first frame, read-only system diagnostics. **Native GPU Wayland is not fixed by this code**: the target NVIDIA + CEF 156 failed to allocate its initial Viz capture frame (`OOM?`). This can reflect a capture-buffer allocation problem, not exhausted physical memory. CEF issue chromiumembedded/cef#4313 identifies an NVIDIA sync defect in CEF 156; fix targeted at a later Chromium/CEF branch. No unsafe GPU synchronization assumption or sandbox disabling is added here.

P4: independent CEF wake scheduling and GPU image presentation, opt-in deadline-aware wake coalescing, heartbeat at a bounded interval, response-time telemetry, process CPU usage. The external pump is always called on the UI thread. The legacy repeated 8 ms Slint timer remains available.

> The `input_to_ui` metrics are a proxy: timestamp of the latest Slint input -> the next image property update. They cannot prove that the new image is caused by that input, nor when the display scanned it out. `gpu_submit_to_ui` measures from GPU *submission* to a Slint image property update (not Vulkan completion or screen presentation). `process_cpu_pct` measures only Aster's process, excluding Chromium renderer/GPU child processes.

## New independent flags

| Flag | Environment | Values | Default |
|---|---|---|---|
| `--cef-angle` | `ASTER_CEF_ANGLE` | `auto`, `gl-egl`, `vulkan`, `default` | `auto` (EGL) |
| `--wayland-gpu` | `ASTER_WAYLAND_GPU` | `probe`, `cpu` | `probe` |
| `--gpu-first-frame-timeout-ms` | `ASTER_GPU_FIRST_FRAME_TIMEOUT_MS` | `1000..30000` | `8000` |
| `--pump-mode` | `ASTER_PUMP_MODE` | `legacy`, `event` | `legacy` |
| `--pump-wake` | `ASTER_PUMP_WAKE` | `eager`, `deadline` | `deadline` (event mode) |
| `--pump-idle-ms` | `ASTER_PUMP_IDLE_MS` | `16..1000` | `33` |
| `--gpu-presentation` | `ASTER_GPU_PRESENTATION` | `poll`, `notify` | `poll` |

The existing `--renderer`, `--linux-display`, `--gpu-damage`, and `--buffers` remain independent. `--wayland-gpu cpu` only changes `--renderer auto` on a native Wayland session; forced `--renderer gpu --linux-display wayland --wayland-gpu cpu` is rejected rather than silently overriding an explicit GPU request. CPU OSR still supports at most two buffers. `--cef-angle=default` suppresses Aster's ANGLE override; Chromium chooses its own default. `--cef-angle=vulkan` is a **diagnostic**, not a known working DMA-BUF path.

With `--linux-display wayland`, Aster now refuses `DISPLAY` being set or a missing Wayland socket, to prevent accidental XWayland usage. Use `env -u DISPLAY`; the Winit/Slint backend must be selected. This strictness is opt-in. `--linux-display x11` remains the validated GPU baseline.

### P3 tests (fish)

Launch from the project root, after applying P1/P2/P3/P4 and this patch.

```fish
sh scripts/diagnose-wayland.sh | tee /tmp/aster-wayland-preflight.log

# Native Wayland, immediate CPU mode, no 8-second GPU blank screen
 env -u DISPLAY ASTER_PERF=1 ASTER_LIFECYCLE=1 ASTER_FONTCONFIG_MODE=auto \
  cargo run --release -p aster-desktop --features cef-gpu -- \
  --linux-display wayland --renderer auto --wayland-gpu cpu \
  --buffers 2 --pump-mode legacy \
  2>&1 | tee /tmp/aster-wayland-cpu-first.log

# Native Wayland, explicit EGL GPU probe + CPU fallback if no first frame
 env -u DISPLAY ASTER_PERF=1 ASTER_LIFECYCLE=1 ASTER_FONTCONFIG_MODE=auto \
  cargo run --release -p aster-desktop --features cef-gpu -- \
  --linux-display wayland --renderer auto --wayland-gpu probe \
  --cef-angle gl-egl --gpu-first-frame-timeout-ms 5000 \
  --buffers 2 --pump-mode legacy \
  2>&1 | tee /tmp/aster-wayland-egl.log

# Vulkan ANGLE experiment (may not support the capture/export path)
 env -u DISPLAY ASTER_PERF=1 ASTER_LIFECYCLE=1 ASTER_FONTCONFIG_MODE=auto \
  cargo run --release -p aster-desktop --features cef-gpu -- \
  --linux-display wayland --renderer auto --wayland-gpu probe \
  --cef-angle vulkan --gpu-first-frame-timeout-ms 5000 \
  --buffers 2 --pump-mode legacy \
  2>&1 | tee /tmp/aster-wayland-vulkan.log
```

`first_valid_frame` + no `switching all browsers to CPU OSR` is the minimal sign of successful CEF GPU capture. Visually test scrolling, resize, rapid resize, navigation and tab close. If GPU fails, the CPU fallback is **not** evidence of GPU Wayland compatibility.

### P4 reproducible test scenarios

Serve a local static page (no external network dependency):

```fish
python -m http.server 8765 --bind 127.0.0.1
```

Navigate Aster to `http://127.0.0.1:8765/tests/manual/scheduler.html?mode=idle` for quiet load and `?mode=animate` for a CSS animation, keeping browser window dimensions unchanged and giving each test the same dwell time. Run an additional input scenario with a fixed sequence of clicks/typing.

With a separate shell for each Aster process, hold all options identical except for the tested switch:

```fish
# Reference: timer with legacy UI polls
 env ASTER_PERF=1 ASTER_LIFECYCLE=1 ASTER_CEF_OZONE=x11 ASTER_FONTCONFIG_MODE=auto \
  cargo run --release -p aster-desktop --features cef-gpu -- \
  --renderer gpu --buffers 2 --gpu-damage adaptive \
  --gpu-presentation poll --pump-mode legacy \
  2>&1 | tee /tmp/p4-legacy.log

# Event mode: old eager notifications, 33 ms safety interval
 env ASTER_PERF=1 ASTER_LIFECYCLE=1 ASTER_CEF_OZONE=x11 ASTER_FONTCONFIG_MODE=auto \
  cargo run --release -p aster-desktop --features cef-gpu -- \
  --renderer gpu --buffers 2 --gpu-damage adaptive \
  --gpu-presentation poll --pump-mode event --pump-wake eager --pump-idle-ms 33 \
  2>&1 | tee /tmp/p4-eager33.log

# Event mode: wake only when an earlier deadline requires rescheduling
 env ASTER_PERF=1 ASTER_LIFECYCLE=1 ASTER_CEF_OZONE=x11 ASTER_FONTCONFIG_MODE=auto \
  cargo run --release -p aster-desktop --features cef-gpu -- \
  --renderer gpu --buffers 2 --gpu-damage adaptive \
  --gpu-presentation poll --pump-mode event --pump-wake deadline --pump-idle-ms 33 \
  2>&1 | tee /tmp/p4-deadline33.log

# Isolate GPU presentation from CEF-pump wakeups
 env ASTER_PERF=1 ASTER_LIFECYCLE=1 ASTER_CEF_OZONE=x11 ASTER_FONTCONFIG_MODE=auto \
  cargo run --release -p aster-desktop --features cef-gpu -- \
  --renderer gpu --buffers 2 --gpu-damage adaptive \
  --gpu-presentation notify --pump-mode event --pump-wake deadline --pump-idle-ms 33 \
  2>&1 | tee /tmp/p4-deadline33-notify.log
```

To test 16 or 250 ms, rerun the *same* command with `--pump-idle-ms 16` or `250` and unique log filenames. 250 ms is a diagnostic stress-test, not the recommended default: CEF documentation requires periodic work even without scheduled notifications. Zero-delay work must not be throttled to the display refresh interval; the optimizer removes only redundant early **wakeups**, not due Chromium tasks.

```fish
python scripts/compare-loop.py \
  legacy=/tmp/p4-legacy.log \
  eager33=/tmp/p4-eager33.log \
  deadline33=/tmp/p4-deadline33.log \
  notify33=/tmp/p4-deadline33-notify.log
```

For each setting repeat idle and animation 3+ times before calling a difference reproducible. Look for `pump_calls_per_s`, `wake_avoided`, `deadline_late_p95_ms`, `input_to_ui_p95_ms` (proxy), `gpu_submit_to_ui_p95_ms` (proxy), and `process_cpu_pct` (parent process only). Percentiles in `compare-loop.py` are unweighted means of each 2-second-window percentile, not a mathematically correct combined percentile. This first-stage telemetry does not measure actual power, total Chromium CPU, or GPU scanout latency. `ASTER_LIFECYCLE=1` is needed for the shutdown status.

### Validation gates

```fish
cargo fmt --all
cargo fmt --all -- --check
cargo test -p browser-contracts
cargo test -p browser-render-wgpu
cargo test -p engine-cef --features gpu
cargo test -p frontend-slint --features gpu
cargo test -p aster-desktop --features cef-gpu
cargo check -p aster-desktop --features cef
cargo check -p aster-desktop --features cef-gpu
python -m unittest discover -s scripts -p 'test_*.py'
```

Manual gates: accelerated X11 unchanged, native Wayland CPU, native Wayland GPU probe with a bounded fallback, event mode idle/animated/typing, tab close, resize and app close. The source patch is *not yet verified by the Rust toolchain or on NVIDIA hardware* in this environment; do not merge before local tests. The CEF 156 accelerated Wayland capture problem can require an upstream CEF/Chromium fix, not a standalone Aster option.

### Upstream references

- CEF external message loop and `OnScheduleMessagePumpWork`: https://chromiumembedded.github.io/cef/general_usage.html
- CEF Linux EGL/ANGLE import constraints: https://github.com/chromiumembedded/cef/issues/3953
- CEF 156 NVIDIA DMA-BUF write fences: https://github.com/chromiumembedded/cef/issues/4313
- Chromium Viz capture pool can report `OOM?` on allocation failure: https://chromium.googlesource.com/chromium/src/+/999b6ed985f5dfd612ea70d5172b64db8e6751fd/components/viz/service/frame_sinks/video_capture/frame_sink_video_capturer_impl.cc
