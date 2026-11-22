# P3 — Native Wayland / XWayland experiments

CEF 156 and Slint 1.19 remain separately encapsulated. This phase adds **an
explicit Linux Ozone selection and a Wayland socket preflight**, without
changing the existing X11/XWayland GPU path. The Slint backend continues to
use Winit (Wayland-capable); with `DISPLAY` unset, the UI cannot silently
create its window through XWayland.

## Independent controls

* `--linux-display=auto|x11|wayland|none`, or `ASTER_LINUX_DISPLAY`, default
  `auto`; `ASTER_CEF_OZONE` is still read when the new variable is absent.
* `--renderer=cpu|gpu|auto` independently selects CEF OSR presentation.
* `ASTER_CEF_GPU_COMPAT=0` independently disables the `gl-egl` ANGLE
  compatibility switch for diagnosis; **not recommended** for standard use.
* Explicit Chromium `--ozone-platform=...` switches still take precedence;
  conflicting explicit Wayland/X11 selectors are rejected when passed as
  `--ozone-platform=VALUE`.
* `--linux-display=wayland` requires an accessible UNIX socket named by
  `WAYLAND_DISPLAY` (absolute path or below `XDG_RUNTIME_DIR`). If the socket
  does not exist, the browser process exits with an explanatory error instead
  of silently using X11.
* Selecting `none` is intended only for diagnostics; headless Ozone currently
  does **not** export the DMA-BUF backing required by shared-texture OSR.

**Definition of native Wayland used in the manual test:** launch with
`DISPLAY` *unset*, `WAYLAND_DISPLAY` and `XDG_RUNTIME_DIR` pointing to a valid
Wayland socket, `--linux-display=wayland`, and Slint's Winit backend. This
precludes an X11/XWayland window and asks CEF to use Wayland Ozone. The
`[aster:display] native_wayland_candidate=true` line proves configuration,
not that the compositor accepted or displayed an accelerated frame.

CEF on the target NVIDIA Vulkan/ANGLE stack previously failed to deliver a
Wayland accelerated frame. **This patch does not repair that upstream DMA-BUF
allocation/synchronization problem.** A Wayland CPU OSR path is available, and
`--renderer=auto` may fall back from GPU OSR to CPU OSR *while retaining Ozone
Wayland*. Do not call a CPU fallback a successful accelerated Wayland test.
Never disable Chromium's sandbox to make an experiment succeed.

## Tests (fish shell)

Build once:

```fish
cargo test -p browser-contracts
cargo test -p aster-desktop --features cef-gpu
cargo test -p engine-cef --features gpu
cargo check -p aster-desktop --features cef-gpu
```

Reference XWayland-compatible GPU experiment (may use native Wayland for the
**Slint shell**, while CEF itself uses X11 Ozone):

```fish
env ASTER_PERF=1 ASTER_LIFECYCLE=1 ASTER_CEF_OZONE=x11 \
  cargo run --release -p aster-desktop --features cef-gpu -- \
  --renderer gpu --buffers 2 --linux-display x11 --pump-mode legacy \
  2>&1 | tee /tmp/aster-p3-x11.log
```

Native Wayland, **CPU reference without XWayland**:

```fish
env -u DISPLAY ASTER_PERF=1 ASTER_LIFECYCLE=1 \
  cargo run --release -p aster-desktop --features cef-gpu -- \
  --renderer cpu --buffers 2 --linux-display wayland --pump-mode legacy \
  2>&1 | tee /tmp/aster-p3-wayland-cpu.log
```

Native Wayland, **experimental accelerated CEF**:

```fish
env -u DISPLAY ASTER_PERF=1 ASTER_LIFECYCLE=1 \
  cargo run --release -p aster-desktop --features cef-gpu -- \
  --renderer auto --buffers 2 --linux-display wayland --pump-mode legacy \
  2>&1 | tee /tmp/aster-p3-wayland-gpu.log
```

Check `[aster:gpu] first_valid_frame`, whether a CPU fallback occurred, visible
input/scroll/resize, and `cef_shutdown_complete browsers_pending=0`. Run
`python scripts/compare-loop.py x11=/tmp/aster-p3-x11.log
wayland=/tmp/aster-p3-wayland-gpu.log` only for diagnostic telemetry;
these are different backends and not controlled performance benchmarks.

## Known limitations

* CEF 156 + NVIDIA + Ozone Wayland shared-texture frame delivery needs real
  hardware validation and may fail with a misleading Chromium `OOM?` warning.
  None of the CEF source or GPU allocator internals are patched here.
* Winit's actual chosen backend is not introspected by this patch. Unsetting
  `DISPLAY` is the strict, reproducible way to exclude XWayland.
* Changing the Chromium Ozone platform **after** CEF initialization is not
  supported. A failed native GPU attempt can fall back to CPU OSR, but not
  automatically switch Chromium from Wayland to X11 in the same process.
* CEF's separate known NVIDIA DMA-BUF synchronization problem can produce
  partially drawn frames even when accelerated callbacks are delivered.
