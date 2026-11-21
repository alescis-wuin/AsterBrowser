# P0-PERFORMANCE: input latency and OSR instrumentation

## Scope

The initial CPU-backed CEF off-screen renderer is functional but expensive. This
change fixes a reversed wheel direction, separates high-frequency input from
state snapshots, raises the OSR paint cap to 60 FPS by default, and exposes
opt-in lightweight performance counters. It does **not** replace the CPU copy
with GPU texture sharing.

## Changes

- Slint forwards the wheel's vertical delta without the previous sign inversion.
- The Slint event-loop timer defaults to 8 ms (previously 16 ms). This is a
  polling interval, **not** a guaranteed latency or display frame rate.
- `BrowserSettings.windowless_frame_rate` defaults to 60 (previously 30).
- Mouse, scroll, keyboard and focus commands use an input-only dispatcher. No
  `BrowserSnapshot` is cloned or compared for these high-frequency actions.
- Frames belonging to background tabs are dropped before RGB conversion. The
  engine explicitly invalidates the page on tab activation.
- `ASTER_PERF=1` reports CEF paint throughput, replaced (unpresented) paint
  buffers, copied bytes and copy times, plus UI pump and conversion times.
  The UI's `frames_delivered` is **not** a display FPS measurement.

## Runtime controls

| Environment variable | Default | Valid values |
|---|---|---|
| `ASTER_PUMP_MS` | `8` | `4..=33`, otherwise default |
| `ASTER_OSR_FPS` | `60` | `1..=60`, otherwise default |
| `ASTER_PERF` | off | `1` enables diagnostics |

## Reproducible comparison

On the same page and window size, compare **release builds**, not just debug
performance. Use a local fixture for repeatability:

```sh
python -m http.server 8765 --bind 127.0.0.1 --directory tests/manual
```

In a separate terminal, launch the default configuration:

```sh
ASTER_PERF=1 cargo run --release -p aster-desktop --features cef
```

Load `http://127.0.0.1:8765/performance.html`, type, scroll and navigate.
Also confirm the existing `interaction.html` input regression fixture.
Record `[aster:cef]` and `[aster:slint]` lines, and repeat with:

```sh
ASTER_PERF=1 ASTER_OSR_FPS=30 ASTER_PUMP_MS=16 cargo run --release -p aster-desktop --features cef
```

Use the same conditions to compare visual responsiveness and CPU use. Changing
parameters is diagnostic: 60 FPS can make a CPU-bound renderer *slower*. If
copy/conversion times dominate, prioritise CEF accelerated OSR and GPU texture
interop rather than increasing the timer frequency.

## Acceptance

- [ ] `cargo fmt --all -- --check`
- [ ] `cargo test -p browser-core` and `cargo test -p frontend-slint`
- [ ] `cargo test -p engine-cef --features cef-runtime`
- [ ] `cargo check -p aster-desktop --features cef`
- [ ] Mouse wheel scrolls in the expected direction on the input fixture.
- [ ] Typing, mouse clicks and tab switching still work.
- [ ] Page content is updated after changing tabs and resizing.
- [ ] Closing tabs and the main window still exits without an orphan CEF process.
- [ ] Compare `ASTER_PERF=1` measurements for the default and baseline settings.

## Limitations

The UI and CEF pump still share one thread and the CPU BGRA -> RGB path still
performs two full-frame copies and uploads new images to Slint. OSR rendering is
not a guarantee of 60 displayed FPS. Native scan codes, IME, GPU-backed OSR
interop, true frame presentation timing and event-driven CEF work scheduling are
out of scope. The Chromium sandbox must remain enabled.
