# P0-CEF: offscreen Chromium rendering

## Purpose

Display a real Chromium page inside the Slint window without leaking any CEF or Slint type into the shared contracts or browser core.

## Implementation boundaries

1. `engine-cef` initializes CEF and handles subprocess exit, OSR rendering, main-thread CEF message work and shutdown.
2. A per-tab CEF browser emits metadata events and latest-only BGRA frames. Only the latest paint per tab is retained.
3. `browser-core` applies metadata events and owns tab state, while `frontend-slint` owns image conversion and widget state.
4. `aster-desktop` wires the actual engine into the neutral `Dispatcher` and `Poller` callbacks. Mock mode remains the default.
5. Closing a tab requests forced CEF close. The CEF `OnBeforeClose` callback decrements the pending-close count. CEF shutdown happens only when pending closures reach zero.

## Manual acceptance gate

On the target Linux Wayland desktop, with CEF binaries correctly installed:

- [ ] `cargo fmt --all -- --check`
- [ ] `cargo test -p browser-core` (5 tests)
- [ ] `cargo check -p aster-desktop`
- [ ] `cargo check -p aster-desktop --features cef`
- [ ] `cargo run -p aster-desktop --features cef` shows `https://example.org` inside the Slint content area.
- [ ] Address-bar navigation to `https://www.rust-lang.org` renders correctly.
- [ ] Click a hyperlink, then Back/Forward and Reload.
- [ ] Open two tabs, select either tab, close inactive then active, and reopen another.
- [ ] Resize the window, confirm viewport redraw and no distorted/blank page.
- [ ] Close the window; check the process exits without hanging, a crash, or remaining CEF children.
- [ ] Repeat close after navigating to a busy JavaScript page.
- [ ] Confirm that CEF is sandboxed; do not accept a `--no-sandbox` workaround.
- [ ] Verify `cargo run -p aster-desktop` still launches the mock UI.

## Error diagnosis

- CEF fails before opening Slint: verify `CEF_PATH`, shared library search path, CEF resources and sandbox configuration.
- Blank content with working toolbar: inspect `on_paint` callbacks, `windowless_rendering_enabled`, viewport size and GPU driver logs.
- Page stuck while loading: inspect CEF main-thread pump timer and network errors.
- Window does not exit: look for remaining `OnBeforeClose` acknowledgments and inspect renderer process state.

Do **not** mark this gate complete until the actual Linux build and all runtime checks pass.
