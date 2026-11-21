# P0-GPU-WATCHDOG: first-frame startup regression

## Symptom

With `ASTER_CEF_OZONE=x11`, the Linux shared-texture path imports and presents
frames (`[aster:gpu] imported=.../s`), but the original watchdog may still
report `no accelerated paint within 8 seconds` and close/recreate browsers in
CPU mode. The log message is inaccurate once successful accelerated paint has
occurred.

## Correction

The CEF mailbox now maintains a first-frame latch for each native browser. The latch is set only after `SharedTextureHandle::import_texture` and
`GpuHub::present` both succeed. The startup watchdog checks this latch instead
of using the presence of the current texture in the presentation cache.

A new browser with no successful accelerated paint still times out after eight
seconds. Errors reported by `GpuHub::take_failure()` still trigger the CPU
fallback. Losing the WGPU device after the browser was created also triggers
a separate fallback, without conflating it with a missing first frame. A static page does not trigger a timeout after its first good frame.
The latch is per-browser and cleared on close, so recreating a browser starts a new timeout period.

This patch does not change CEF startup switches, sandboxing, the image format,
GPU synchronization, CPU fallback, or the GPU presentation pipeline.

## Regression checks

```sh
cargo fmt --all
cargo test -p engine-cef --features gpu
cargo check -p aster-desktop --features cef-gpu
ASTER_PERF=1 ASTER_CEF_OZONE=x11 cargo run --release -p aster-desktop \
  --features cef-gpu -- --renderer gpu --buffers 1
```

With `ASTER_PERF=1`, `first_valid_frame tab=<id>` is logged once after
the first successful GPU import/presentation for each browser.

Test a **static page for at least 15 seconds** after the first successful GPU
frame. It must stay on GPU. Repeat with a JavaScript animation and 2 buffers,
and check navigation, resizing, and orderly tab closure. If the GPU renderer
loses its graphics context after producing a first frame, investigate device
teardown/reinitialization separately; this patch only corrects the startup
first-frame detection, not a general device-loss recovery mechanism.

A console line such as `[aster:slint] frames_delivered=0.0/s` reports CPU
image deliveries and can be normal during GPU presentation. The positive GPU
signal is `[aster:gpu] imported=.../s` while no fallback occurs.
