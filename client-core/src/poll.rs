//! Shared request scheduling for native clients. A slow request never overlaps its successor.
use std::time::{Duration, Instant};

#[derive(Debug)]
pub struct PollGate {
    minimum: Duration,
    maximum: Duration,
    delay: Duration,
    next: Option<Instant>,
    running: bool,
    pending: bool,
}

impl PollGate {
    pub fn new(minimum: Duration, maximum: Duration) -> Self {
        Self {
            minimum,
            maximum,
            delay: minimum,
            next: None,
            running: false,
            pending: false,
        }
    }

    pub fn start(&mut self, now: Instant) -> bool {
        if self.running || self.next.is_some_and(|next| now < next) {
            return false;
        }
        self.running = true;
        true
    }

    pub fn finish(&mut self, now: Instant, active: bool) {
        self.running = false;
        self.delay = if active {
            self.minimum
        } else {
            (self.delay * 2).min(self.maximum)
        };
        self.next = if self.pending {
            None
        } else {
            Some(now + self.delay)
        };
        self.pending = false;
    }

    /// Account/scope changes or foregrounding request one fresh result, after any active request.
    pub fn wake(&mut self) {
        self.delay = self.minimum;
        self.next = None;
        self.pending = self.running;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn serializes_slow_requests_and_coalesces_wakeups() {
        let now = Instant::now();
        let mut gate = PollGate::new(Duration::from_secs(1), Duration::from_secs(5));
        assert!(gate.start(now));
        for _ in 0..10 {
            gate.wake();
            assert!(!gate.start(now + Duration::from_secs(20)));
        }
        gate.finish(now, false);
        assert!(gate.start(now));
        gate.finish(now, false);
        assert!(!gate.start(now + Duration::from_secs(3)));
        assert!(gate.start(now + Duration::from_secs(4)));
    }
    #[test]
    fn caps_idle_backoff_and_restores_active_interval() {
        let mut gate = PollGate::new(Duration::from_millis(1500), Duration::from_secs(5));
        let mut now = Instant::now();
        for delay in [3000, 5000, 5000] {
            assert!(gate.start(now));
            gate.finish(now, false);
            assert!(!gate.start(now + Duration::from_millis(delay - 1)));
            now += Duration::from_millis(delay);
        }
        assert!(gate.start(now));
        gate.finish(now, true);
        assert!(gate.start(now + Duration::from_millis(1500)));
    }
}
