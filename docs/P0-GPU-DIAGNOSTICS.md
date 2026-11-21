# P0-GPU-DIAGNOSTICS: Linux shared-texture OSR startup

## Scope

The `cef-gpu` path depends on CEF producing `OnAcceleratedPaint` frames. Aster's WGPU importer cannot run until CEF successfully allocates and exports the native shared surface. A Chromium error such as `Unable to allocate frame for first frame capture: OOM?` originates before Aster's imported texture or presentation code. It does not establish that the system has exhausted VRAM.

## Linux compatibility switches

When `--renderer gpu` or `--renderer auto` is requested (with feature `cef-gpu`), Aster now opts into these CEF command-line settings on Linux unless Chromium already specifies them:

- `--use-angle=gl-egl` for EGL/ANGLE.
- `--ozone-platform=x11` when `DISPLAY` exists (preferred even inside a Wayland desktop with XWayland); otherwise `wayland` if `WAYLAND_DISPLAY` exists.

The program does not change CEF sandboxing, overwrite explicit Chromium switches, or apply compatibility switches to CPU-only mode.

Environment controls:

| Variable | Values | Purpose |
| --- | --- | --- |
| `ASTER_CEF_GPU_COMPAT` | `1` (default), `0` | Disable or enable automatic EGL/Ozone compatibility settings. |
| `ASTER_CEF_OZONE` | `auto` (default), `x11`, `wayland`, `none` | Explicitly choose or disable Aster's Ozone suggestion. |
| `ASTER_GPU_POWER` | `high` (default), `low` | Slint WGPU adapter power preference. Does not control the CEF GPU. |

On Linux with Intel and NVIDIA GPUs, CEF and Slint may select different physical devices. Vulkan DMA-BUF import may then fail even if `OnAcceleratedPaint` starts firing. Test both choices while keeping the same browser window and workload. The settings above do **not** guarantee device affinity or solve all CEF/driver bugs.

## Reproduction matrix

Run one browser process at a time, stopping it before the next test. Keep CEF resources loaded from `CEF_PATH` and `LD_LIBRARY_PATH`, as documented in the README.

```sh
# A. CPU baseline (known-good)
ASTER_PERF=1 cargo run --release -p aster-desktop --features cef-gpu -- --renderer cpu --buffers 2

# B. CEF GPU with automatic EGL and XWayland when DISPLAY is present
ASTER_PERF=1 cargo run --release -p aster-desktop --features cef-gpu -- --renderer gpu --buffers 2

# C. Direct Wayland CEF mode
ASTER_PERF=1 ASTER_CEF_OZONE=wayland cargo run --release -p aster-desktop --features cef-gpu -- --renderer gpu --buffers 2

# D. Without startup compatibility switches, to establish a control case
ASTER_PERF=1 ASTER_CEF_GPU_COMPAT=0 cargo run --release -p aster-desktop --features cef-gpu -- --renderer gpu --buffers 2

# E. WGPU low-power adapter preference (often the iGPU, not guaranteed)
ASTER_PERF=1 ASTER_GPU_POWER=low cargo run --release -p aster-desktop --features cef-gpu -- --renderer gpu --buffers 2
```

The GPU path currently falls back to CPU when no accelerated frame arrives within eight seconds. **Do not report CPU fallback as a successful GPU benchmark.** For each test, record whether the log contains `[aster:gpu]` frame metrics, a native-import error, or an eight-second fallback. Compare GPU usage using `nvidia-smi` and `vulkaninfo --summary` where available.

## External CEF limitations

CEF upstream issue #3953 documents Linux accelerated OSR requiring EGL/ANGLE and an X11/Wayland Ozone backend. Issue #4237 documents NVIDIA shared-image allocation failures and a follow-up fix. Issue #4313 describes a synchronization regression affecting CEF 156 on NVIDIA; even when the first paint succeeds, validate resizing and image correctness before enabling GPU mode by default.

References:

- https://github.com/chromiumembedded/cef/issues/3953
- https://github.com/chromiumembedded/cef/issues/4237
- https://github.com/chromiumembedded/cef/issues/4313

## Acceptance gate

- `cargo test -p engine-cef --features cef-runtime` passes the Linux platform-selection tests.
- `cargo check -p aster-desktop --features cef` and `--features cef-gpu` pass.
- CPU baseline works and shuts down cleanly.
- GPU produces **actual `OnAcceleratedPaint` frames** without fallback.
- GPU input, scrolling, updates, resizing, two-buffer image coherence and tab close work.
- Confirm browser process sandbox remains enabled.

If the GPU capture allocator still fails, keep CPU mode as the default. Do not disable the sandbox or use unrelated GPU-disable switches to force progress.
