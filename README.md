# Aster Browser

A modular, experimental Rust browser shell. P0 uses Slint as the replaceable UI adapter and Chromium Embedded Framework (CEF) as an opt-in engine.

> **Development prototype, not a hardened browser.** The CEF path still requires compilation and desktop validation on a supported Linux host. OSR input, popups, accessibility, sandbox provisioning, downloads and production packaging are not complete.

## Crate boundaries

- `browser-contracts`: UI/engine-neutral commands, snapshots, events and BGRA paint frames.
- `browser-core`: tab state machine, normalization, engine event application and testable mock engine.
- `frontend-slint`: replaceable GUI, Slint widgets, CPU image buffering and optional Slint WGPU presentation; it knows no CEF APIs.
- `engine-cef`: CEF wrapper, renderer/client callbacks, event pump, per-tab OSR surfaces, DMA-BUF import and close acknowledgments.
- `browser-render-wgpu`: optional backend-neutral WGPU texture bridge and incremental RGBA8 shader composition.
- `aster-desktop`: composition root; selects mock or CEF at compile time.

## Requirements

- Rust stable >= 1.95 (Rust 1.100 was used by the original contributor).
- Linux x86_64, C/C++ compiler, CMake, Ninja and other CEF Linux runtime libraries.
- CEF binary distribution matching `cef = 156.7.0` (first build can download a large SDK).
- On Linux, Chromium sandbox support must be configured properly. Never solve failures by permanently disabling the sandbox.

## Mock-only smoke tests

```sh
cargo fmt --all -- --check
cargo test -p browser-core
cargo check -p aster-desktop
cargo run -p aster-desktop
```

## CEF OSR test (Linux)

Install a matching CEF binary release using upstream tooling (perform once per CEF version):

```sh
git clone --depth 1 --branch 'cef-v156.7.0+156.0.31' https://github.com/tauri-apps/cef-rs.git /tmp/aster-cef-rs
cd /tmp/aster-cef-rs
cargo run -p export-cef-dir -- --force "$HOME/.local/share/cef"
```

Then from the Aster Browser repository, in **Fish**:

```fish
set -gx CEF_PATH "$HOME/.local/share/cef"
set -gx LD_LIBRARY_PATH $CEF_PATH $LD_LIBRARY_PATH
set -gx SLINT_STYLE fluent-dark
cargo check -p aster-desktop --features cef
cargo run -p aster-desktop --features cef
```

Or in Bash:

```sh
export CEF_PATH="$HOME/.local/share/cef"
export LD_LIBRARY_PATH="$CEF_PATH${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"
export SLINT_STYLE=fluent-dark
cargo run -p aster-desktop --features cef
```

The CEF build can download and extract hundreds of megabytes; this is expected. If a preinstalled CEF binary exists, point `CEF_PATH` to its directory instead. Never run Chromium as root. Consult upstream CEF Linux sandbox requirements if initialization aborts because `chrome-sandbox` is unavailable.

## P0 operation

- Open a new tab, enter a URL/search query and press Enter or click **Go**.
- Navigate via **Back**, **Forward**, **Reload** and links within a rendered page.
- Use the wheel to scroll. Window resizing updates the offscreen CEF viewport.
- Close individual tabs with `x`. The last tab may be closed without closing the shell.
- Close the Slint window to request all remaining Chromium browsers to close. The CEF loop waits up to 15 seconds for `OnBeforeClose` before shutting down. Unacknowledged close is reported as an error rather than a successful shutdown.

## Current limitations

- The CEF path **has not been compiled or runtime-tested in the artifact authoring environment**. The actual desktop host must validate it before calling P0 complete.
- The default CPU OSR path maintains incremental BGRA frames (maximum 4096x2160); experimental GPU DMA-BUF imports are available only with `--features cef-gpu` and compatible hardware.
- Mouse clicks, wheel and basic keyboard navigation are forwarded. Native context menus, popup/select surfaces, drag/drop and IME are not complete.
- CEF popup windows are not accepted/managed; the first version is for simple page/navigation testing.
- CEF runtime resources, Linux sandbox and bundling need platform testing.
- The message pump follows `OnScheduleMessagePumpWork` hints, sampled by a configurable Slint timer. It is not yet a fully event-driven wakeup mechanism.
- `cargo check` with `--features cef` updates `Cargo.lock`. Commit the lockfile for reproducible builds.

See [docs/P0-CEF.md](docs/P0-CEF.md), [docs/decisions.md](docs/decisions.md) and [docs/CONTRIBUTING.md](docs/CONTRIBUTING.md).

For OSR paint-rate controls, input latency diagnostics and A/B comparisons, see [docs/P0-PERFORMANCE.md](docs/P0-PERFORMANCE.md).

## P0 input forwarding (experimental)

See [docs/P0-INPUT.md](docs/P0-INPUT.md) for the independent JavaScript fixture, accepted inputs and runtime test checklist. Keyboard support uses Slint logical keys; native XKB scancodes, IME composition and CEF popup rendering are not yet complete.

## Incremental OSR and adaptive pump (P0-DAMAGE-RENDERING)

The experimental damage path compares CEF dirty rectangles against a persistent BGRA framebuffer, emits either full frames or incremental BGRA patches, and applies those patches to a retained Slint RGB pixel buffer. Unchanged paints do not cause Aster copies or presentation updates. CEF's message pump follows scheduling callbacks with a 33 ms maximum fallback interval, sampled by Slint's 8 ms UI timer.

```sh
ASTER_PERF=1 ASTER_OSR_FPS=60 ASTER_PUMP_MS=8 ASTER_PUMP_MAX_MS=33 cargo run --release -p aster-desktop --features cef
```

See [docs/P0-DAMAGE-RENDERING.md](docs/P0-DAMAGE-RENDERING.md) for validation, telemetry and copy-on-write limitations. Test manually using `tests/manual/damage.html`.

## Experimental CPU/GPU presentation (P0-GPU-BUFFERING)

Set `--renderer cpu|gpu|auto` and `--buffers 1|2` at launch (or use
`ASTER_RENDERER` and `ASTER_BUFFERS`). The default is the established CPU path
with one buffer; the GPU renderer is optional and requires `--features cef-gpu`.

The CPU path supports incremental dirty patches in one or two Slint image
buffers. The GPU path uses shared CEF textures, **owned GPU import copies**,
partial RGBA8 shader composition with scissor regions, and a same-device Slint WGPU renderer.
GPU import is conditional on drivers and device compatibility and falls back
to CPU OSR after failures. It is not fully zero-copy and remains experimental.

See [docs/P0-GPU-BUFFERING.md](docs/P0-GPU-BUFFERING.md) for configuration,
limitations, the GPU memory-copy distinction, test commands and acceptance gates.
