# P0-GPU: rendering lifecycle guard

## Symptom

On Linux/XWayland, successful CEF accelerated imports (`first_valid_frame` and
`[aster:gpu] imported=60/s`) were followed by an inaccurate
`Slint WGPU rendering device lost` fallback **as the application window closed**.
The previous engine considered every temporary absence of WGPU `Device` an
irrecoverable device loss.

## Root cause and change

Slint's `RenderingTeardown` is also fired during normal window destruction.
The frontend calls `GpuHub::clear()` on this notification. The CEF engine
continued to pump messages briefly and incorrectly interpreted the cleared
context as a GPU failure, which could recreate browsers while shutting down.

`GpuHub` now records the renderer teardown instant and distinguishes it from
an initialization failure. The CEF engine allows a **three-second grace period**
for Slint to reinitialize WGPU, and only falls back to CPU if the renderer stays
absent while the browser continues running. Accelerated paint callbacks arriving
during teardown are discarded rather than counted as import failures. On
`RenderingSetup`, `GpuHub::set_device` resets the teardown state.

Real import/compose errors still trigger the existing immediate CPU fallback.
The initial eight-second no-first-frame watchdog is unchanged. No CEF switches,
sandboxing options, contracts, or rendering buffers are modified.

## Validation on Linux

```sh
cargo fmt --all
cargo test -p browser-render-wgpu
cargo test -p engine-cef --features gpu
cargo check -p aster-desktop --features cef-gpu
ASTER_PERF=1 ASTER_CEF_OZONE=x11 cargo run --release \
  -p aster-desktop --features cef-gpu -- --renderer gpu --buffers 1
```

1. Check for `first_valid_frame` and positive GPU imports.
2. Let a static page remain open 15 seconds; no CPU fallback should occur.
3. Exercise scrolling, animation, resize, tab changes, and `--buffers 2`.
4. Close the window normally. No `Slint WGPU rendering device lost` message
   or CPU-browser recreation should appear during shutdown.
5. A *genuine* WGPU teardown without subsequent setup while Aster keeps running
   should cause CPU fallback after the grace interval.

**Limitation:** These tests distinguish renderer lifecycle notifications from
proven GPU import failures. They do not detect all hardware device losses;
WGPU-specific device error callbacks and automatic recovery are separate work.
`frames_delivered` in `[aster:slint]` only measures CPU frame transport and can
remain zero during successful GPU rendering.
