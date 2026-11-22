# P4 — CEF/Slint event-driven scheduler (opt-in)

## Problem and behavioral constraints

The existing Slint loop runs a `TimerMode::Repeated` every `ASTER_PUMP_MS`
(default 8ms) and calls CEF's `do_message_loop_work()` only if a requested
deadline or the `ASTER_PUMP_MAX_MS` fallback is due. This preserves
correctness but calls Aster's poller ~125 times/s even on idle pages.

With `--pump-mode=event`, the CEF `OnScheduleMessagePumpWork` callback updates
its synchronized deadline and notifies Slint via `invoke_from_event_loop()`.
Requests are **coalesced** while a wake-up is queued. Only the main Slint UI
thread executes `cef::do_message_loop_work()` and applies `BrowserTick` frames
or metadata. The deadline is also tracked by a **single-shot Slint timer**
re-armed after each tick; an independent bounded heartbeat guarantees a
chance to process missed CEF notifications. The existing repeated timer is
kept as a baseline, entirely unmodified when `legacy` is selected.

## CLI / environment

| Setting | Values | Default | Description |
|---|---|---|---|
| `--pump-mode` / `ASTER_PUMP_MODE` | `legacy`, `event` | `legacy` | UI loop strategy |
| `--pump-idle-ms` / `ASTER_PUMP_IDLE_MS` | `33..=1000` | `250` | CEF fallback in event mode (milliseconds) |
| `ASTER_PUMP_MS` | `4..=33` | `8` | Existing Slint repeated poll interval, legacy only |
| `ASTER_PUMP_MAX_MS` | `8..=33` | `33` | Existing CEF watchdog, legacy only |
| `ASTER_PERF` | `0`, `1` | `0` | Enable both old and new telemetry |

`--pump-mode=event` and `--pump-idle-ms` do not modify GPU options or Linux
window-system selection. If the GPU path fails, CPU OSR is still driven by
the same scheduler. Mock engine continues using the legacy UI timer.

Metrics:
* `[aster:slint] ticks=.../s` — UI poll callbacks per second;
* `[aster:pump] calls=.../s scheduled=... fallback=...` — CEF work calls;
* `[aster:scheduler] wake_requested/wake_posted/wake_dispatched/wake_coalesced` —
  events received/posted/processed/merged over each telemetry window;
* `pump_avg` — wall-clock time inside CEF's `do_message_loop_work`, not
  an end-to-end frame latency or total CPU usage.

## A/B commands (fish shell)

Use the same pages, CSS animations, scrolls, window sizes, frame rate and
experiment duration in each run. Keep graphics options identical.

Legacy reference:

```fish
env ASTER_PERF=1 ASTER_LIFECYCLE=1 ASTER_CEF_OZONE=x11 \
  cargo run --release -p aster-desktop --features cef-gpu -- \
  --renderer gpu --buffers 2 --gpu-damage adaptive \
  --gpu-lazy-init on --gpu-sync queue --pump-mode legacy \
  2>&1 | tee /tmp/aster-p4-legacy.log
```

Event-driven, 250ms heartbeat:

```fish
env ASTER_PERF=1 ASTER_LIFECYCLE=1 ASTER_CEF_OZONE=x11 \
  cargo run --release -p aster-desktop --features cef-gpu -- \
  --renderer gpu --buffers 2 --gpu-damage adaptive \
  --gpu-lazy-init on --gpu-sync queue --pump-mode event \
  --pump-idle-ms 250 \
  2>&1 | tee /tmp/aster-p4-event.log
```

Event-driven, 100ms heartbeat (isolate the idle-timeout effect):

```fish
env ASTER_PERF=1 ASTER_LIFECYCLE=1 ASTER_CEF_OZONE=x11 \
  cargo run --release -p aster-desktop --features cef-gpu -- \
  --renderer gpu --buffers 2 --gpu-damage adaptive \
  --gpu-lazy-init on --gpu-sync queue --pump-mode event \
  --pump-idle-ms 100 \
  2>&1 | tee /tmp/aster-p4-event-100.log
```

Compare:

```fish
python scripts/compare-loop.py \
  legacy=/tmp/aster-p4-legacy.log \
  event250=/tmp/aster-p4-event.log \
  event100=/tmp/aster-p4-event-100.log
```

A quiet page and an animated page should be measured **separately**. Expected
benefit is fewer unnecessary UI polls and fallback CEF calls, not necessarily
higher FPS. The comparison tool reports arithmetic averages of periodic
windows; it cannot infer energy consumption, input latency percentiles or
actual frame pacing without additional instrumentation.

## Validation gate

```fish
cargo fmt --all -- --check
cargo test -p browser-contracts
cargo test -p engine-cef --features gpu
cargo test -p aster-desktop --features cef-gpu
cargo check -p aster-desktop --features cef
cargo check -p aster-desktop --features cef-gpu
python -m unittest scripts/test_compare_loop.py
```

Also test navigation, focus/typing, scrolling, several animated pages, GPU
resize and multi-tab changes, and clean CEF shutdown. A stuck page under
`event` that does not stick under `legacy` is a regression even if the idle
poll rate decreases. This code must be compiled and tested on a real desktop;
a static syntax review is not a substitute for that validation.
