//! CEF external message pump. Chromium can schedule from any thread; the actual
//! cef::do_message_loop_work call is always on the UI thread.
use browser_contracts::PumpWakeMode;
use std::{
    cell::{Cell, RefCell},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};

#[derive(Clone, Copy)]
struct WorkDeadline {
    at: Instant,
    requested_at: Instant,
}
#[derive(Default)]
struct Deadlines {
    next: Option<WorkDeadline>,
    /// The UI event loop's actual next single-shot firing time.
    armed: Option<Instant>,
}

#[derive(Clone)]
pub(crate) struct PumpRequests {
    inner: Arc<PumpShared>,
}
struct PumpShared {
    state: Mutex<Deadlines>,
    pumping: AtomicBool,
    wake_mode: PumpWakeMode,
    count: AtomicU64,
    wakes_avoided: AtomicU64,
    wakes_issued: AtomicU64,
    waker: Mutex<Option<Arc<dyn Fn() + Send + Sync>>>,
}
impl Default for PumpRequests {
    fn default() -> Self {
        Self::new(PumpWakeMode::Deadline)
    }
}
impl PumpRequests {
    pub fn new(wake_mode: PumpWakeMode) -> Self {
        Self {
            inner: Arc::new(PumpShared {
                state: Mutex::new(Deadlines::default()),
                pumping: AtomicBool::new(false),
                wake_mode,
                count: AtomicU64::new(0),
                wakes_avoided: AtomicU64::new(0),
                wakes_issued: AtomicU64::new(0),
                waker: Mutex::new(None),
            }),
        }
    }

    /// A positive delay *replaces* the previous CEF request, including when
    /// it moves an earlier deadline to a later time. Never hold a mutex while
    /// invoking the Slint wake callback.
    pub fn schedule(&self, delay_ms: i64) {
        let now = Instant::now();
        let deadline = now + Duration::from_millis(delay_ms.clamp(0, 60_000) as u64);
        let should_notify = if let Ok(mut state) = self.inner.state.lock() {
            let previous = state.next;
            state.next = Some(WorkDeadline {
                at: deadline,
                requested_at: now,
            });
            self.inner.count.fetch_add(1, Ordering::Relaxed);
            if self.inner.pumping.load(Ordering::Acquire) {
                false // after the pump returns, re-arm with the latest deadline
            } else {
                match self.inner.wake_mode {
                    PumpWakeMode::Eager => previous.is_none_or(|old| deadline < old.at),
                    PumpWakeMode::Deadline => state.armed.is_none_or(|armed| deadline < armed),
                }
            }
        } else {
            false
        };
        if should_notify {
            self.inner.wakes_issued.fetch_add(1, Ordering::Relaxed);
            if let Some(wake) = self.inner.waker.lock().ok().and_then(|w| w.clone()) {
                wake();
            }
        } else {
            self.inner.wakes_avoided.fetch_add(1, Ordering::Relaxed);
        }
    }
    pub fn set_waker(&self, wake: Arc<dyn Fn() + Send + Sync>) {
        if let Ok(mut waker) = self.inner.waker.lock() {
            *waker = Some(wake);
        }
    }
    pub fn clear_waker(&self) {
        if let Ok(mut waker) = self.inner.waker.lock() {
            *waker = None;
        }
    }
    fn arm_next(&self, now: Instant, fallback: Instant) -> Duration {
        let Ok(mut state) = self.inner.state.lock() else {
            return fallback
                .saturating_duration_since(now)
                .max(Duration::from_millis(1));
        };
        let deadline = state.next.map_or(fallback, |work| work.at.min(fallback));
        let delay = deadline
            .saturating_duration_since(now)
            .max(Duration::from_millis(1));
        // Installed before releasing the mutex so another Chromium thread can
        // detect whether its new deadline really needs an early wake.
        state.armed = Some(now + delay);
        delay
    }
    fn take_due(&self, now: Instant) -> Option<WorkDeadline> {
        let Ok(mut state) = self.inner.state.lock() else {
            return None;
        };
        if state.next.is_some_and(|work| now >= work.at) {
            state.next.take()
        } else {
            None
        }
    }
    fn count(&self) -> u64 {
        self.inner.count.load(Ordering::Relaxed)
    }
    fn wakes_avoided(&self) -> u64 {
        self.inner.wakes_avoided.load(Ordering::Relaxed)
    }
    fn wakes_issued(&self) -> u64 {
        self.inner.wakes_issued.load(Ordering::Relaxed)
    }
}

#[derive(Default)]
struct PumpStats {
    started: Option<Instant>,
    checks: u64,
    scheduled: u64,
    fallback: u64,
    max_duration: Duration,
    total_duration: Duration,
    late_ms: Vec<f64>,
    request_to_work_ms: Vec<f64>,
    previous_requests: u64,
    previous_avoided: u64,
    previous_issued: u64,
}

/// Exact percentiles for the bounded (2s) telemetry window, no new dependency.
fn percentiles(samples: &[f64]) -> (f64, f64, f64) {
    if samples.is_empty() {
        return (0.0, 0.0, 0.0);
    }
    let mut values = samples.to_vec();
    values.sort_by(f64::total_cmp);
    let at = |p: f64| values[((values.len() - 1) as f64 * p).ceil() as usize];
    (at(0.50), at(0.95), at(0.99))
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum PumpReason {
    Scheduled,
    Fallback,
}

pub(crate) struct PumpController {
    requests: PumpRequests,
    last_work: Cell<Instant>,
    in_work: Cell<bool>,
    max_interval: Duration,
    stats: RefCell<PumpStats>,
    perf_enabled: bool,
}
impl PumpController {
    pub fn new(requests: PumpRequests, max_interval: Duration, perf_enabled: bool) -> Self {
        Self {
            requests,
            last_work: Cell::new(Instant::now() - max_interval),
            in_work: Cell::new(false),
            max_interval,
            stats: RefCell::new(PumpStats::default()),
            perf_enabled,
        }
    }
    pub fn due(&self, now: Instant) -> Option<PumpReason> {
        if self.in_work.get() {
            return None;
        }
        if self.perf_enabled {
            self.stats.borrow_mut().checks += 1;
        }
        if let Some(work) = self.requests.take_due(now) {
            if self.perf_enabled {
                let mut stats = self.stats.borrow_mut();
                stats
                    .late_ms
                    .push(now.saturating_duration_since(work.at).as_secs_f64() * 1000.0);
                stats.request_to_work_ms.push(
                    now.saturating_duration_since(work.requested_at)
                        .as_secs_f64()
                        * 1000.0,
                );
            }
            return Some(PumpReason::Scheduled);
        }
        if now.saturating_duration_since(self.last_work.get()) >= self.max_interval {
            return Some(PumpReason::Fallback);
        }
        None
    }
    pub fn next_delay(&self, now: Instant) -> Duration {
        self.requests
            .arm_next(now, self.last_work.get() + self.max_interval)
    }
    pub fn set_waker(&self, wake: Arc<dyn Fn() + Send + Sync>) {
        self.requests.set_waker(wake);
    }
    pub fn clear_waker(&self) {
        self.requests.clear_waker();
    }
    pub fn begin(&self) {
        self.in_work.set(true);
        self.requests.inner.pumping.store(true, Ordering::Release);
    }
    pub fn end(&self, reason: PumpReason, elapsed: Duration) {
        self.last_work.set(Instant::now());
        self.in_work.set(false);
        self.requests.inner.pumping.store(false, Ordering::Release);
        if !self.perf_enabled {
            return;
        }
        let mut stats = self.stats.borrow_mut();
        stats.started.get_or_insert_with(Instant::now);
        match reason {
            PumpReason::Scheduled => stats.scheduled += 1,
            PumpReason::Fallback => stats.fallback += 1,
        }
        stats.total_duration += elapsed;
        stats.max_duration = stats.max_duration.max(elapsed);
        if stats
            .started
            .is_some_and(|t| t.elapsed() >= Duration::from_secs(2))
        {
            let elapsed_secs = stats.started.unwrap().elapsed().as_secs_f64();
            let calls = (stats.scheduled + stats.fallback).max(1);
            let requests = self.requests.count();
            let avoided = self.requests.wakes_avoided();
            let issued = self.requests.wakes_issued();
            let (p50, p95, p99) = percentiles(&stats.late_ms);
            let (r50, r95, r99) = percentiles(&stats.request_to_work_ms);
            eprintln!(
                "[aster:pump] checks={:.1}/s calls={:.1}/s scheduled={} fallback={} skipped={} requests={} pump_avg={:.2}ms pump_max={:.2}ms wake_avoided={} wake_issued={} deadline_late_p50_ms={:.2} deadline_late_p95_ms={:.2} deadline_late_p99_ms={:.2} request_to_pump_p50_ms={:.2} request_to_pump_p95_ms={:.2} request_to_pump_p99_ms={:.2}",
                stats.checks as f64 / elapsed_secs,
                calls as f64 / elapsed_secs,
                stats.scheduled,
                stats.fallback,
                stats.checks.saturating_sub(calls),
                requests.saturating_sub(stats.previous_requests),
                stats.total_duration.as_secs_f64() * 1000.0 / calls as f64,
                stats.max_duration.as_secs_f64() * 1000.0,
                avoided.saturating_sub(stats.previous_avoided),
                issued.saturating_sub(stats.previous_issued),
                p50,
                p95,
                p99,
                r50,
                r95,
                r99,
            );
            *stats = PumpStats {
                previous_requests: requests,
                previous_avoided: avoided,
                previous_issued: issued,
                ..Default::default()
            };
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn immediate_request_has_priority_over_fallback() {
        let requests = PumpRequests::default();
        let controller = PumpController::new(requests.clone(), Duration::from_millis(33), false);
        requests.schedule(0);
        assert_eq!(controller.due(Instant::now()), Some(PumpReason::Scheduled));
        controller.begin();
        assert_eq!(controller.due(Instant::now()), None);
        controller.end(PumpReason::Scheduled, Duration::ZERO);
        assert_eq!(controller.due(Instant::now()), None);
    }
    #[test]
    fn fallback_is_bounded_and_requests_can_be_delayed() {
        let requests = PumpRequests::default();
        let controller = PumpController::new(requests.clone(), Duration::from_millis(33), false);
        assert_eq!(controller.due(Instant::now()), Some(PumpReason::Fallback));
        controller.begin();
        controller.end(PumpReason::Fallback, Duration::ZERO);
        requests.schedule(1000);
        assert_eq!(controller.due(Instant::now()), None);
        assert_eq!(
            controller.due(Instant::now() + Duration::from_millis(34)),
            Some(PumpReason::Fallback)
        );
    }
    #[test]
    fn latest_schedule_replaces_earlier_deadline() {
        let requests = PumpRequests::default();
        requests.schedule(0);
        requests.schedule(1000);
        assert!(requests.take_due(Instant::now()).is_none());
        requests.schedule(0);
        assert!(requests.take_due(Instant::now()).is_some());
    }
    #[test]
    fn one_shot_timer_obeys_requested_deadline_and_idle_limit() {
        let requests = PumpRequests::default();
        let idle = Duration::from_millis(33);
        let controller = PumpController::new(requests.clone(), idle, false);
        let now = Instant::now();
        assert!(controller.next_delay(now) <= Duration::from_millis(1));
        controller.begin();
        controller.end(PumpReason::Fallback, Duration::ZERO);
        let interval = controller.next_delay(Instant::now());
        assert!(interval <= idle && interval >= Duration::from_millis(1));
        requests.schedule(10);
        assert!(controller.next_delay(Instant::now()) <= Duration::from_millis(11));
        requests.schedule(0);
        assert_eq!(controller.due(Instant::now()), Some(PumpReason::Scheduled));
    }
    #[test]
    fn deadline_mode_defers_positive_callback_until_armed_timer() {
        let requests = PumpRequests::new(PumpWakeMode::Deadline);
        let calls = Arc::new(AtomicU64::new(0));
        let receiver = calls.clone();
        requests.set_waker(Arc::new(move || {
            receiver.fetch_add(1, Ordering::Relaxed);
        }));
        let controller = PumpController::new(requests.clone(), Duration::from_millis(33), false);
        controller.begin();
        controller.end(PumpReason::Fallback, Duration::ZERO);
        controller.next_delay(Instant::now()); // timer armed for ~33 ms
        requests.schedule(100); // should not cause an early UI wake
        requests.schedule(20); // should cause a wake to update timer
        assert_eq!(calls.load(Ordering::Relaxed), 1);
        assert_eq!(requests.wakes_avoided(), 1);
    }
    #[test]
    fn deadline_mode_suppresses_wakes_during_pump_and_preserves_request() {
        let requests = PumpRequests::default();
        let calls = Arc::new(AtomicU64::new(0));
        let receiver = calls.clone();
        requests.set_waker(Arc::new(move || {
            receiver.fetch_add(1, Ordering::Relaxed);
        }));
        let controller = PumpController::new(requests.clone(), Duration::from_millis(33), false);
        controller.begin();
        requests.schedule(0);
        controller.end(PumpReason::Fallback, Duration::ZERO);
        assert_eq!(calls.load(Ordering::Relaxed), 0);
        assert_eq!(controller.due(Instant::now()), Some(PumpReason::Scheduled));
    }
    #[test]
    fn eager_mode_keeps_previous_behavior() {
        let requests = PumpRequests::new(PumpWakeMode::Eager);
        let notified = Arc::new(AtomicU64::new(0));
        let receiver = notified.clone();
        requests.set_waker(Arc::new(move || {
            receiver.fetch_add(1, Ordering::Relaxed);
        }));
        requests.schedule(500);
        requests.schedule(1000);
        requests.schedule(0);
        assert_eq!(notified.load(Ordering::Relaxed), 2);
    }
    #[test]
    fn thread_safe_waker_is_called_without_a_cef_ui_call() {
        let requests = PumpRequests::default();
        let notified = Arc::new(AtomicU64::new(0));
        let receiver = notified.clone();
        requests.set_waker(Arc::new(move || {
            receiver.fetch_add(1, Ordering::Relaxed);
        }));
        let thread_requests = requests.clone();
        std::thread::spawn(move || {
            thread_requests.schedule(0);
        })
        .join()
        .unwrap();
        assert_eq!(notified.load(Ordering::Relaxed), 1);
        assert!(requests.take_due(Instant::now()).is_some());
    }
    #[test]
    fn percentile_estimates_are_monotonic() {
        let (a, b, c) = percentiles(&(0..100).map(|x| x as f64).collect::<Vec<_>>());
        assert_eq!((a, b, c), (50.0, 95.0, 99.0));
        assert_eq!(percentiles(&[]), (0.0, 0.0, 0.0));
    }
}
