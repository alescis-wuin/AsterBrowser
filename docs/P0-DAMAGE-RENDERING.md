# P0-DAMAGE-RENDERING: Incremental OSR and adaptive CEF pump

## Scope and boundaries

- D1: Detect unchanged `OnPaint` buffers by comparing only CEF's dirty rectangles with the previous per-tab framebuffer. Skip copies and notifications when those pixels match.
- D2: Keep a BGRA framebuffer for each tab; accumulate damage over multiple CEF callbacks, then emit a coherent full frame or bounded BGRA patches at polling time. Resizes always reset to full frames. Invalid rectangles are clipped before access.
- D3: Keep an RGB `SharedPixelBuffer` per tab in the Slint adapter, convert only updated patches, and set the active `Image` only when content or the selected tab changes. Background-tab buffers are updated too, so activation never applies patches to a stale buffer.
- D4: Track `CefBrowserProcessHandler::OnScheduleMessagePumpWork` deadlines across threads and call `CefDoMessageLoopWork` only when due or by a bounded fallback. All actual CEF work stays on the main UI thread. Slint still polls the deadlines every 8 ms (configurable); this is not a fully event-driven Slint wakeup.
- D5: Log damage, copy and scheduling metrics with `ASTER_PERF=1`.

## Correctness and safety

- Dirty rectangle coordinates refer to physical pixels. The CEF BGRA pointer is borrowed for the duration of `OnPaint` only; no raw pointer survives the callback.
- A `Full` frame must precede all `Patches` for a tab and for every new size; patches are row-major BGRA8 without stride padding.
- All frontend patches are validated before mutation. If the frontend has no previous frame or the dimensions differ, patches are rejected instead of corrupting the display.
- Full BGRA frames use a shared immutable `Arc<Vec<u8>>` snapshot; dispatching does not clone the entire image. Subsequent writes use copy-on-write only if the previous BGRA snapshot remains in flight.
- Pending patches accumulate across multiple `OnPaint` callbacks. If patch area reaches 70% of the screen or over 64 regions accumulate, a full frame is used instead.
- No frame is produced or handed to Slint when CEF's reported pixels have not changed.
- We continue pumping Chromium at least every 33 ms, even without scheduling callbacks. Otherwise JS timers, networking and shutdown can stall.
- No sandbox or web security flags are changed.

## What is *not* optimized

- `SharedPixelBuffer::make_mut_bytes` performs copy-on-write when the displayed `Image` still owns the same pixel buffer. That can copy a whole RGB frame even when only a small patch changes. Slint's `Image` does not expose an OSR dirty-rectangle GPU upload API here. Zero-copy GPU shared textures remain a separate feature.
- DOM layout, V8 execution, network activity and Chromium's internal rendering are not suspended just because a page is visually static. We only avoid unnecessary Aster CPU work and Slint presentation.
- A CEF `OnPaint` event can mark a large dirty area even for a small visible change. Copy savings are workload-dependent.

## Build and run on CachyOS

```fish
cd /developpement/seynax/aster-browser
cargo fmt --all -- --check
cargo test -p browser-core
cargo test -p engine-cef --features cef-runtime
cargo test -p frontend-slint
cargo check -p aster-desktop --features cef

# With CEF_PATH and LD_LIBRARY_PATH already configured:
env ASTER_PERF=1 ASTER_OSR_FPS=60 ASTER_PUMP_MS=8 ASTER_PUMP_MAX_MS=33 \
    cargo run --release -p aster-desktop --features cef
```

Serve `tests/manual` and open `http://127.0.0.1:8765/damage.html`:

```fish
python -m http.server 8765 --bind 127.0.0.1 --directory tests/manual
```

## A/B measurements

1. Capture a baseline of the previous P0 performance implementation with the same viewport, 60 FPS and page modes.
2. Capture the patched implementation using the **static**, **small animation**, and **full background** modes for at least 10 seconds each.
3. Observe `[aster:cef]`: `dirty_area_pct`, `unchanged_frames`, `presentation_skipped`, `copied`, `emitted`, `compared`, `full_updates`, `partial_updates` and `paint_cpu_avg`.
4. Observe `[aster:slint]`: `partial_updates`, `full_updates`, `converted` in Mpx/s, `presentation_skipped`, `convert_avg`; and `[aster:pump]`: `calls`, `scheduled`, `fallback`, `skipped`, `requests`.
5. Check no visual tearing, missing pixels, delayed JS timers, missing keyboard/pointer events, or shutdown hangs after switching and resizing tabs.
6. Compare the same test with `ASTER_OSR_FPS=30` and `ASTER_PUMP_MAX_MS=33` to isolate frame-rate effects.

`frames_delivered` is an image update count, not proof of monitor-presented frames. Slint may still upload a full texture for a partial patch.

## Gate

- [ ] Cargo checks and tests pass on Linux with CEF installed
- [ ] Static pages render without redundant copies
- [ ] Small animations produce patch updates and correct pixels
- [ ] Full animations do not regress materially relative to baseline
- [ ] Switching tabs and resizing preserves correct pixels
- [ ] JS timers, navigation, keyboard, pointer and shutdown remain functional
- [ ] CPU and responsiveness compared with baseline in release mode
