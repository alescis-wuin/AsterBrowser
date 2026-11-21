//! External CEF message pump scheduling, independent of the GUI toolkit.
//! `OnScheduleMessagePumpWork` may arrive from any thread. We only store a
//! deadline there; the actual CEF call always runs on the UI thread.
use std::{
    cell::{Cell, RefCell},
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};

#[derive(Clone, Default)]
pub(crate) struct PumpRequests {
    inner: Arc<PumpShared>,
}
#[derive(Default)]
struct PumpShared {
    deadline: Mutex<Option<Instant>>,
    count: AtomicU64,
}
impl PumpRequests {
    pub fn schedule(&self, delay_ms: i64) {
        let deadline = Instant::now() + Duration::from_millis(delay_ms.max(0).min(60_000) as u64);
        if let Ok(mut next) = self.inner.deadline.lock() {
            // A new CEF notification replaces the previous requested deadline.
            *next = Some(deadline);
            self.inner.count.fetch_add(1, Ordering::Relaxed);
        }
    }
    fn take_due(&self, now: Instant) -> bool {
        let Ok(mut deadline) = self.inner.deadline.lock() else {
            return false;
        };
        if deadline.is_some_and(|time| now >= time) {
            *deadline = None;
            true
        } else {
            false
        }
    }
    fn count(&self) -> u64 {
        self.inner.count.load(Ordering::Relaxed)
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
    previous_requests: u64,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum PumpReason {
    Scheduled,
    Fallback,
}

/// All methods on this controller except `requests.schedule` must run on the
/// same UI thread that called cef::initialize.
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
        if self.requests.take_due(now) {
            return Some(PumpReason::Scheduled);
        }
        if now.duration_since(self.last_work.get()) >= self.max_interval {
            return Some(PumpReason::Fallback);
        }
        None
    }
    pub fn begin(&self) {
        self.in_work.set(true);
    }
    pub fn end(&self, reason: PumpReason, elapsed: Duration) {
        self.last_work.set(Instant::now());
        self.in_work.set(false);
        if self.perf_enabled {
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
                let requested = self.requests.count();
                let num_pumps = (stats.scheduled + stats.fallback).max(1);
                eprintln!(
                    "[aster:pump] checks={:.1}/s calls={:.1}/s scheduled={} fallback={} skipped={} requests={} pump_avg={:.2}ms pump_max={:.2}ms",
                    stats.checks as f64 / elapsed_secs,
                    num_pumps as f64 / elapsed_secs,
                    stats.scheduled,
                    stats.fallback,
                    stats.checks.saturating_sub(num_pumps),
                    requested.saturating_sub(stats.previous_requests),
                    stats.total_duration.as_secs_f64() * 1000.0 / num_pumps as f64,
                    stats.max_duration.as_secs_f64() * 1000.0,
                );
                *stats = PumpStats {
                    previous_requests: requested,
                    ..Default::default()
                };
            }
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
        assert!(!requests.take_due(Instant::now()));
        requests.schedule(0);
        assert!(requests.take_due(Instant::now()));
    }
}
