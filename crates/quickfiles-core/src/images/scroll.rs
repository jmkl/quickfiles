//! Scroll-velocity gate: while the user drags the scrollbar through thousands of files, draw
//! icons only and queue no image work; load as soon as the scroll slows down.

use std::time::Instant;

/// Pixels per second above which images are not requested.
const FAST: f32 = 2400.0;
/// Below this the gate opens again (hysteresis, so it does not flicker).
const SLOW: f32 = 900.0;
/// How strongly a new sample moves the smoothed speed (0..1).
const SMOOTH: f32 = 0.5;

#[derive(Default)]
pub struct ScrollGate {
    last: Option<(f32, Instant)>,
    speed: f32,
    dir: i8,
    fast: bool,
}

impl ScrollGate {
    /// Feed the scroll offset once per frame. Frames with no time passed are ignored.
    pub fn update(&mut self, offset: f32, now: Instant) {
        if let Some((o, t)) = self.last {
            let dt = now.saturating_duration_since(t).as_secs_f32();
            if dt < 0.001 {
                return;
            }
            let v = (offset - o) / dt;
            self.speed += (v.abs() - self.speed) * SMOOTH;
            if v > 1.0 {
                self.dir = 1;
            } else if v < -1.0 {
                self.dir = -1;
            }
            if v == 0.0 {
                // Nothing moved: decay quickly instead of waiting for the next sample.
                self.speed *= 0.5;
            }
            self.fast = if self.fast {
                self.speed > SLOW
            } else {
                self.speed > FAST
            };
        }
        self.last = Some((offset, now));
    }

    /// Scrolling too fast to bother loading images.
    pub fn fast(&self) -> bool {
        self.fast
    }

    /// -1 = towards the top, 1 = towards the bottom, 0 = not scrolled yet.
    pub fn direction(&self) -> i8 {
        self.dir
    }

    pub fn speed(&self) -> f32 {
        self.speed
    }

    /// Forget the history (a new folder or view starts at rest).
    pub fn reset(&mut self) {
        *self = Self::default();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn feed(g: &mut ScrollGate, t0: Instant, samples: &[(u64, f32)]) {
        for &(ms, off) in samples {
            g.update(off, t0 + Duration::from_millis(ms));
        }
    }

    #[test]
    fn slow_scrolling_never_gates() {
        let t0 = Instant::now();
        let mut g = ScrollGate::default();
        for i in 0..60 {
            g.update(i as f32 * 5.0, t0 + Duration::from_millis(i * 16)); // ~300 px/s
        }
        assert!(!g.fast());
        assert_eq!(g.direction(), 1);
    }

    #[test]
    fn dragging_the_scrollbar_gates_and_stopping_releases() {
        let t0 = Instant::now();
        let mut g = ScrollGate::default();
        for i in 0..20 {
            g.update(i as f32 * 120.0, t0 + Duration::from_millis(i * 16)); // 7500 px/s
        }
        assert!(g.fast());
        // The scrollbar is released: the offset stops changing.
        for i in 20..40 {
            g.update(19.0 * 120.0, t0 + Duration::from_millis(i * 16));
        }
        assert!(!g.fast());
    }

    #[test]
    fn direction_follows_the_last_movement() {
        let t0 = Instant::now();
        let mut g = ScrollGate::default();
        feed(&mut g, t0, &[(0, 500.0), (16, 480.0), (32, 460.0)]);
        assert_eq!(g.direction(), -1);
    }

    #[test]
    fn hysteresis_keeps_the_gate_closed_between_the_thresholds() {
        let t0 = Instant::now();
        let mut g = ScrollGate::default();
        for i in 0..20 {
            g.update(i as f32 * 120.0, t0 + Duration::from_millis(i * 16));
        }
        assert!(g.fast());
        // ~1500 px/s: below FAST but above SLOW
        let base = 19.0 * 120.0;
        for i in 0..30 {
            g.update(
                base + i as f32 * 24.0,
                t0 + Duration::from_millis(320 + i * 16),
            );
        }
        assert!(g.fast());
    }

    #[test]
    fn zero_time_frames_are_ignored() {
        let t0 = Instant::now();
        let mut g = ScrollGate::default();
        g.update(0.0, t0);
        g.update(100000.0, t0);
        assert!(!g.fast());
    }
}
