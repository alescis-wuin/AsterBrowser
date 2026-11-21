# P0-INPUT: Interactive Chromium pages

## Scope

The Slint frontend translates logical keyboard and pointer events into engine-neutral commands. `browser-core` targets only the active tab; `engine-cef` forwards them to the Chromium OSR browser. Neither Slint nor CEF is imported into the shared contracts.

### Implemented

- Focus changes when the content surface is clicked or the address bar gains focus.
- Key presses and releases, standard editing/navigation keys, and text as UTF-16 `CHAR` events.
- Left, right, and middle mouse buttons, pointer movement, wheel scrolling, cancellation, and modifier flags.
- A deterministic HTML/JavaScript fixture that works without external internet access.
- A unit test asserting keyboard events never reach inactive tabs.

### Known constraints

- Slint exposes logical keys, not the native physical XKB scan codes required for full DOM `KeyboardEvent.code` fidelity on Linux. `native_key_code` is therefore `0`; complex shortcuts and non-Latin layouts remain experimental.
- IME composition, AltGr, clipboard, menus, dropdown/popups, text selection outside the main surface, drag/drop, and accessibility integration are not covered.
- The CEF OSR paint surface still uses CPU BGRA-to-RGB conversion. This feature does not change JavaScript performance or renderer scheduling.
- Right-click events are forwarded but CEF popup surfaces and native context menu composition are not implemented.
- Results require manual validation on the target Wayland/X11 desktop. Do not mark this feature done merely because `cargo check` passes.

## Run the test page

From the repository root, start a local HTTP server in a separate terminal:

```sh
python -m http.server 8765 --bind 127.0.0.1 --directory tests/manual
```

Launch Aster Browser with a matching CEF runtime:

```sh
cargo run -p aster-desktop --features cef
```

Navigate to `http://127.0.0.1:8765/interaction.html` and check:

- [ ] The page changes the tab title to **Aster: JavaScript active**.
- [ ] The clock increments every second without any user input (V8 execution).
- [ ] Clicking the first button increments the counter (mouse-to-JS event delivery).
- [ ] Clicking the input focuses it; text, Backspace and Enter work.
- [ ] Arrow keys, Tab, and Shift+Tab navigate or edit as expected; record deviations.
- [ ] Scrolling reveals the bottom button and clicking it changes the status.
- [ ] Right-click triggers the fixture's context-menu event (without a native menu).
- [ ] Switching tabs and typing does not send characters to an inactive tab.
- [ ] The URL field still accepts typing and Enter without typing into the webpage.
- [ ] Closing tabs and the window still stops all CEF processes cleanly.

If the counter or clock do not update, check event delivery and the CEF event pump separately. No need to assume JavaScript is disabled.
