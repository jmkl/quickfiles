//! "Steady for a while" timers, with the clock passed in so they are unit-testable.

use std::time::{Duration, Instant};

/// Reports a value as ready only after it has stayed the same for `delay`. Used so that holding
/// the Down arrow across 50 large images decodes none of them until the selection settles.
pub struct Debounce<K> {
    key: Option<K>,
    since: Instant,
    delay: Duration,
}

impl<K: PartialEq + Clone> Debounce<K> {
    pub fn new(delay: Duration) -> Self {
        Self {
            key: None,
            since: Instant::now(),
            delay,
        }
    }

    /// Call every frame with the current value. `Ok(())` once it has been steady for the delay;
    /// otherwise `Err(wait)`: how long until it would be (ask for a repaint after that).
    pub fn poll(&mut self, key: &K, now: Instant) -> Result<(), Duration> {
        if self.key.as_ref() != Some(key) {
            self.key = Some(key.clone());
            self.since = now;
        }
        let held = now.saturating_duration_since(self.since);
        if held >= self.delay {
            Ok(())
        } else {
            Err(self.delay - held)
        }
    }

    pub fn reset(&mut self) {
        self.key = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ready_only_after_the_delay() {
        let t0 = Instant::now();
        let mut d = Debounce::new(Duration::from_millis(120));
        assert_eq!(d.poll(&1, t0), Err(Duration::from_millis(120)));
        assert_eq!(
            d.poll(&1, t0 + Duration::from_millis(50)),
            Err(Duration::from_millis(70))
        );
        assert_eq!(d.poll(&1, t0 + Duration::from_millis(120)), Ok(()));
        assert_eq!(d.poll(&1, t0 + Duration::from_millis(500)), Ok(()));
    }

    #[test]
    fn a_change_restarts_the_clock() {
        let t0 = Instant::now();
        let mut d = Debounce::new(Duration::from_millis(100));
        let _ = d.poll(&1, t0);
        assert!(d.poll(&2, t0 + Duration::from_millis(99)).is_err());
        assert!(d.poll(&2, t0 + Duration::from_millis(150)).is_err());
        assert!(d.poll(&2, t0 + Duration::from_millis(199)).is_ok());
    }

    #[test]
    fn holding_down_the_arrow_never_becomes_ready() {
        let t0 = Instant::now();
        let mut d = Debounce::new(Duration::from_millis(120));
        for i in 0..50u64 {
            // a new image every 30 ms
            assert!(d.poll(&i, t0 + Duration::from_millis(i * 30)).is_err());
        }
        assert!(
            d.poll(&49, t0 + Duration::from_millis(49 * 30 + 130))
                .is_ok()
        );
    }
}
