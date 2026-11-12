//! Performance overlay (F12). Shows how long our own UI code takes per frame, texture uploads
//! and the image pipeline's counters, so jank can be measured instead of guessed. Ctrl+F12
//! copies a plain-text report (paste it into the table in `PLAN-PERF.md`), Shift+F12 resets.

use std::{collections::VecDeque, time::Instant};

use eframe::egui::{Align2, FontId, Rect, Stroke, StrokeKind, Ui, pos2, vec2};
use quickfiles_core::{ImageStats, JobKind};

use super::{Cx, tex::TexStats};

/// Frames kept for the rolling statistics.
const WINDOW: usize = 240;
/// Frame intervals longer than this are idle gaps (nothing was repainting), not jank.
const IDLE_MS: f32 = 250.0;

#[derive(Default)]
pub struct Perf {
    pub visible: bool,
    /// Milliseconds our UI code took, per frame.
    cpu: VecDeque<f32>,
    /// Milliseconds between consecutive frames while animating or scrolling.
    interval: VecDeque<f32>,
    uploads: VecDeque<u32>,
    upload_ms: VecDeque<f32>,
    /// When recent frames started, for frames per second.
    stamps: VecDeque<Instant>,
    frames: u64,
    worst_cpu: f32,
}

fn push(q: &mut VecDeque<f32>, v: f32) {
    q.push_back(v);
    if q.len() > WINDOW {
        q.pop_front();
    }
}

fn percentile(q: &VecDeque<f32>, p: f32) -> f32 {
    if q.is_empty() {
        return 0.0;
    }
    let mut v: Vec<f32> = q.iter().copied().collect();
    v.sort_by(|a, b| a.total_cmp(b));
    v[((v.len() - 1) as f32 * p).round() as usize]
}

fn avg(q: &VecDeque<f32>) -> f32 {
    if q.is_empty() {
        0.0
    } else {
        q.iter().sum::<f32>() / q.len() as f32
    }
}

fn max(q: &VecDeque<f32>) -> f32 {
    q.iter().copied().fold(0.0, f32::max)
}

impl Perf {
    /// Record one finished frame. `interval_ms` is the time since the previous frame started.
    pub fn record(&mut self, cpu_ms: f32, interval_ms: f32, tex: &TexStats) {
        self.frames += 1;
        push(&mut self.cpu, cpu_ms);
        self.worst_cpu = self.worst_cpu.max(cpu_ms);
        if interval_ms < IDLE_MS {
            push(&mut self.interval, interval_ms);
        }
        self.uploads.push_back(tex.uploads_frame);
        if self.uploads.len() > WINDOW {
            self.uploads.pop_front();
        }
        push(&mut self.upload_ms, tex.upload_ms_frame);

        let now = Instant::now();
        self.stamps.push_back(now);
        while self
            .stamps
            .front()
            .is_some_and(|t| now.duration_since(*t).as_secs_f32() > 1.0)
        {
            self.stamps.pop_front();
        }
    }

    pub fn reset(&mut self) {
        *self = Self {
            visible: self.visible,
            ..Self::default()
        };
    }

    /// Frames drawn during the last second (repaints per second).
    fn fps(&self) -> usize {
        self.stamps.len()
    }

    /// Lines shared by the overlay and the clipboard report.
    pub fn lines(&self, img: &ImageStats, tex: &TexStats) -> Vec<String> {
        let mut out = vec![
            format!(
                "ui code ms     last {:.1}  avg {:.1}  p99 {:.1}  max {:.1}  worst {:.1}",
                self.cpu.back().copied().unwrap_or(0.0),
                avg(&self.cpu),
                percentile(&self.cpu, 0.99),
                max(&self.cpu),
                self.worst_cpu
            ),
            format!(
                "frame gap ms   avg {:.1}  p99 {:.1}  max {:.1}  frames/s {}",
                avg(&self.interval),
                percentile(&self.interval, 0.99),
                max(&self.interval),
                self.fps()
            ),
            format!(
                "uploads/frame  last {}  max {}   upload ms last {:.1}  max {:.1}  total {}",
                tex.uploads_frame,
                self.uploads.iter().copied().max().unwrap_or(0),
                tex.upload_ms_frame,
                max(&self.upload_ms),
                tex.uploads_total
            ),
            format!(
                "textures       {} ({:.1} MB)  deferred uploads {}  scroll gate {}",
                tex.textures,
                tex.texture_mb,
                tex.deferred_total,
                if tex.gated {
                    "CLOSED (fast scroll)"
                } else {
                    "open"
                }
            ),
            format!(
                "image queue    waiting {}  running {}  in memory {} ({:.1} MB)  cancelled {}  dropped {}",
                img.queued, img.in_flight, img.cached, img.cached_mb, img.cancelled, img.dropped
            ),
        ];
        for kind in JobKind::ALL {
            let k = img.kinds[kind as usize];
            out.push(format!(
                "  {:<8}     jobs {}  failed {}  avg {:.1} ms  max {:.1} ms",
                kind.name(),
                k.jobs,
                k.failed,
                k.avg_ms,
                k.max_ms
            ));
        }
        out.push(format!(
            "disk cache     hit {}  miss {}  read avg {:.2} ms  writes {} avg {:.2} ms  {:.1} MB  queued {}",
            img.disk_hits,
            img.disk_misses,
            img.disk_read_avg_ms,
            img.disk_writes,
            img.disk_write_avg_ms,
            img.disk_mb,
            img.disk_pending
        ));
        out.push(format!(
            "repaints       requested by workers {}  sent {}  (batched)",
            img.wake_requests, img.wakes
        ));
        out
    }

    pub fn report(&self, img: &ImageStats, tex: &TexStats, context: &str) -> String {
        let mut s = format!(
            "quickfiles perf report ({context})\nframes sampled: {} (window {})\n",
            self.frames,
            self.cpu.len()
        );
        s.push_str(&self.lines(img, tex).join("\n"));
        s
    }
}

/// The overlay panel, drawn last in the frame.
pub fn overlay(ui: &mut Ui, cx: &mut Cx<'_>, full: Rect) {
    if !cx.state.perf.visible {
        return;
    }
    let th = cx.theme;
    let pal = &th.palette;
    let lines = cx
        .state
        .perf
        .lines(&cx.core.images().stats(), &cx.tex.stats());
    let font = FontId::monospace(11.5);
    let p = ui.painter().clone();

    let line_h = 15.0;
    let width = 520.0_f32.min(full.width() - 20.0);
    let height = lines.len() as f32 * line_h + 34.0;
    let rect = Rect::from_min_size(
        pos2(full.right() - width - 10.0, full.top() + 44.0),
        vec2(width, height),
    );
    p.rect_filled(rect, 8.0, pal.surface.gamma_multiply(0.96));
    p.rect_stroke(rect, 8.0, Stroke::new(1.0, pal.border), StrokeKind::Inside);
    for (i, line) in lines.iter().enumerate() {
        p.text(
            pos2(rect.left() + 10.0, rect.top() + 10.0 + i as f32 * line_h),
            Align2::LEFT_TOP,
            line,
            font.clone(),
            pal.text,
        );
    }
    p.text(
        pos2(rect.left() + 10.0, rect.bottom() - 18.0),
        Align2::LEFT_TOP,
        "F12 hide · Ctrl+F12 copy report · Shift+F12 reset",
        font,
        pal.text_muted,
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn percentile_and_window() {
        let mut q = VecDeque::new();
        for i in 1..=100 {
            push(&mut q, i as f32);
        }
        assert_eq!(percentile(&q, 0.0), 1.0);
        assert_eq!(percentile(&q, 1.0), 100.0);
        assert_eq!(percentile(&q, 0.99), 99.0);
        for _ in 0..500 {
            push(&mut q, 1.0);
        }
        assert_eq!(q.len(), WINDOW);
        assert_eq!(percentile(&VecDeque::new(), 0.99), 0.0);
    }

    #[test]
    fn idle_gaps_are_not_counted_as_jank() {
        let mut p = Perf::default();
        let tex = TexStats::default();
        p.record(2.0, 16.0, &tex);
        p.record(3.0, 5000.0, &tex);
        assert_eq!(p.cpu.len(), 2);
        assert_eq!(p.interval.len(), 1);
    }
}
