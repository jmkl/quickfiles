//! Cheap counters for the image pipeline (decode time, disk cache hits, repaint requests).
//! Plain atomics: recording costs a few nanoseconds, so they are always on. The UI reads a
//! [`ImageStats`] snapshot for its F12 overlay; see `PLAN-PERF.md`.

use std::{
    sync::atomic::{AtomicU64, Ordering::Relaxed},
    time::Duration,
};

use super::ImageKey;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum JobKind {
    Icon,
    Thumb,
    Preview,
}

impl JobKind {
    pub const ALL: [Self; 3] = [Self::Icon, Self::Thumb, Self::Preview];

    pub fn name(self) -> &'static str {
        match self {
            Self::Icon => "icon",
            Self::Thumb => "thumb",
            Self::Preview => "preview",
        }
    }

    pub fn of(key: &ImageKey) -> Self {
        match key {
            ImageKey::Icon { .. } | ImageKey::PathIcon(_) => Self::Icon,
            ImageKey::Thumb(_) => Self::Thumb,
            ImageKey::Preview(..) => Self::Preview,
        }
    }

    fn index(self) -> usize {
        self as usize
    }
}

#[derive(Default)]
struct KindCounters {
    jobs: AtomicU64,
    failed: AtomicU64,
    total_us: AtomicU64,
    max_us: AtomicU64,
}

#[derive(Default)]
pub struct Stats {
    kinds: [KindCounters; 3],
    disk_hits: AtomicU64,
    disk_misses: AtomicU64,
    disk_read_us: AtomicU64,
    disk_write_us: AtomicU64,
    disk_writes: AtomicU64,
    wakes: AtomicU64,
    wake_requests: AtomicU64,
    cancelled: AtomicU64,
}

fn us(d: Duration) -> u64 {
    d.as_micros().min(u128::from(u64::MAX)) as u64
}

impl Stats {
    /// One finished job (whole job, including the disk cache read and write).
    pub fn job(&self, kind: JobKind, took: Duration, ok: bool) {
        let k = &self.kinds[kind.index()];
        let t = us(took);
        k.jobs.fetch_add(1, Relaxed);
        k.total_us.fetch_add(t, Relaxed);
        k.max_us.fetch_max(t, Relaxed);
        if !ok {
            k.failed.fetch_add(1, Relaxed);
        }
    }

    pub fn disk_read(&self, hit: bool, took: Duration) {
        let counter = if hit {
            &self.disk_hits
        } else {
            &self.disk_misses
        };
        counter.fetch_add(1, Relaxed);
        self.disk_read_us.fetch_add(us(took), Relaxed);
    }

    /// A write-behind batch of `n` thumbnails was committed to the disk cache.
    pub fn disk_write_batch(&self, n: u64, took: Duration) {
        self.disk_writes.fetch_add(n, Relaxed);
        self.disk_write_us.fetch_add(us(took), Relaxed);
    }

    /// A job stopped early because the UI no longer wanted its result.
    pub fn cancelled(&self) {
        self.cancelled.fetch_add(1, Relaxed);
    }

    /// A worker finished something and wants a repaint (before coalescing).
    pub fn wake_request(&self) {
        self.wake_requests.fetch_add(1, Relaxed);
    }

    /// A repaint was actually sent to the UI.
    pub fn wake(&self) {
        self.wakes.fetch_add(1, Relaxed);
    }

    pub fn reset(&self) {
        for k in &self.kinds {
            for c in [&k.jobs, &k.failed, &k.total_us, &k.max_us] {
                c.store(0, Relaxed);
            }
        }
        for c in [
            &self.disk_hits,
            &self.disk_misses,
            &self.disk_read_us,
            &self.disk_write_us,
            &self.disk_writes,
            &self.wakes,
            &self.wake_requests,
            &self.cancelled,
        ] {
            c.store(0, Relaxed);
        }
    }

    pub(super) fn snapshot(&self) -> ImageStats {
        let mut out = ImageStats::default();
        for (i, k) in self.kinds.iter().enumerate() {
            let jobs = k.jobs.load(Relaxed);
            out.kinds[i] = KindStats {
                jobs,
                failed: k.failed.load(Relaxed),
                avg_ms: avg_ms(k.total_us.load(Relaxed), jobs),
                max_ms: k.max_us.load(Relaxed) as f64 / 1000.0,
            };
        }
        let (hits, misses) = (self.disk_hits.load(Relaxed), self.disk_misses.load(Relaxed));
        out.disk_hits = hits;
        out.disk_misses = misses;
        out.disk_read_avg_ms = avg_ms(self.disk_read_us.load(Relaxed), hits + misses);
        out.disk_writes = self.disk_writes.load(Relaxed);
        out.disk_write_avg_ms = avg_ms(self.disk_write_us.load(Relaxed), out.disk_writes);
        out.wakes = self.wakes.load(Relaxed);
        out.wake_requests = self.wake_requests.load(Relaxed);
        out.cancelled = self.cancelled.load(Relaxed);
        out
    }
}

fn avg_ms(total_us: u64, n: u64) -> f64 {
    if n == 0 {
        0.0
    } else {
        total_us as f64 / n as f64 / 1000.0
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct KindStats {
    pub jobs: u64,
    pub failed: u64,
    pub avg_ms: f64,
    pub max_ms: f64,
}

/// A point-in-time copy of the counters plus the queue state, for display.
#[derive(Clone, Debug, Default)]
pub struct ImageStats {
    /// Indexed like [`JobKind::ALL`].
    pub kinds: [KindStats; 3],
    pub disk_hits: u64,
    pub disk_misses: u64,
    pub disk_read_avg_ms: f64,
    pub disk_writes: u64,
    pub disk_write_avg_ms: f64,
    /// Repaints actually sent to the UI, and how many were requested before coalescing.
    pub wakes: u64,
    pub wake_requests: u64,
    /// Jobs abandoned half way (the selection or scroll position moved on).
    pub cancelled: u64,
    /// Requests dropped because a queue was over its depth cap.
    pub dropped: u64,
    /// Wanted but not started.
    pub queued: usize,
    pub in_flight: usize,
    /// Decoded images held in memory, and their size.
    pub cached: usize,
    pub cached_mb: f64,
    /// Size of the on-disk thumbnail cache, and writes still queued.
    pub disk_mb: f64,
    pub disk_pending: u64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counts_averages_and_resets() {
        let s = Stats::default();
        s.job(JobKind::Thumb, Duration::from_millis(10), true);
        s.job(JobKind::Thumb, Duration::from_millis(30), false);
        s.disk_read(true, Duration::from_millis(2));
        s.disk_read(false, Duration::from_millis(4));
        s.disk_write_batch(1, Duration::from_millis(6));
        s.cancelled();
        s.wake_request();
        s.wake_request();
        s.wake();
        let snap = s.snapshot();
        let t = snap.kinds[JobKind::Thumb.index()];
        assert_eq!((t.jobs, t.failed), (2, 1));
        assert!((t.avg_ms - 20.0).abs() < 0.01 && (t.max_ms - 30.0).abs() < 0.01);
        assert_eq!(
            (
                snap.disk_hits,
                snap.disk_misses,
                snap.wakes,
                snap.wake_requests
            ),
            (1, 1, 1, 2)
        );
        assert_eq!(snap.cancelled, 1);
        assert!((snap.disk_read_avg_ms - 3.0).abs() < 0.01);
        assert!((snap.disk_write_avg_ms - 6.0).abs() < 0.01);
        s.reset();
        assert_eq!(s.snapshot().kinds[JobKind::Thumb.index()].jobs, 0);
    }
}
