//! Rubber-band selection: press on empty space in the list or grid and drag to select every item
//! the rectangle touches. Ctrl or Shift keeps the existing selection. A plain click on empty
//! space clears it. Ctrl + drag starts the band on top of an item too.
//!
//! The band lives in content coordinates (relative to the list's top-left, scrolled with it), so
//! it stays attached to the items while the view autoscrolls near the top or bottom edge.

use std::collections::HashSet;

use eframe::egui::{Pos2, Rect, StrokeKind, Ui, pos2, vec2};
use quickfiles_core::View;

use super::Cx;

/// How the items are laid out, so hits can be computed without the item rectangles.
#[derive(Clone, Copy)]
pub enum Layout {
    /// Full-width rows `pitch` apart; only `left..right` (relative to the area) counts as a row.
    List { pitch: f32, left: f32, right: f32 },
    /// Tiles of `cell_w` x `cell_h` in `cols` columns, the first starting `left` from the area.
    Grid {
        cols: usize,
        left: f32,
        cell_w: f32,
        cell_h: f32,
    },
}

impl Layout {
    /// Index of the item under `p` (content coordinates), if there is one.
    fn item_at(&self, p: Pos2, total: usize) -> Option<usize> {
        if p.y < 0.0 {
            return None;
        }
        match *self {
            Self::List { pitch, left, right } => {
                if p.x < left || p.x > right {
                    return None;
                }
                let i = (p.y / pitch) as usize;
                (i < total).then_some(i)
            }
            Self::Grid {
                cols,
                left,
                cell_w,
                cell_h,
            } => {
                let x = p.x - left;
                if x < 0.0 || x >= cols as f32 * cell_w {
                    return None;
                }
                // The 4 px gap between tiles belongs to the empty space.
                if x % cell_w > cell_w - 4.0 || p.y % cell_h > cell_h - 4.0 {
                    return None;
                }
                let i = (p.y / cell_h) as usize * cols + (x / cell_w) as usize;
                (i < total).then_some(i)
            }
        }
    }

    /// Indices of every item the content-space rectangle `r` touches.
    fn items_in(&self, r: Rect, total: usize) -> Vec<usize> {
        if total == 0 || r.bottom() < 0.0 {
            return Vec::new();
        }
        let top = r.top().max(0.0);
        match *self {
            Self::List { pitch, left, right } => {
                if r.right() < left || r.left() > right {
                    return Vec::new();
                }
                let lo = (top / pitch) as usize;
                let hi = ((r.bottom() / pitch) as usize).min(total - 1);
                if lo > hi {
                    Vec::new()
                } else {
                    (lo..=hi).collect()
                }
            }
            Self::Grid {
                cols,
                left,
                cell_w,
                cell_h,
            } => {
                let width = cols as f32 * cell_w;
                if r.right() < left || r.left() > left + width {
                    return Vec::new();
                }
                let c_lo = ((r.left() - left).max(0.0) / cell_w) as usize;
                let c_hi = (((r.right() - left) / cell_w) as usize).min(cols - 1);
                let r_lo = (top / cell_h) as usize;
                let r_hi = (r.bottom() / cell_h) as usize;
                let mut out = Vec::new();
                for row in r_lo..=r_hi {
                    for c in c_lo..=c_hi {
                        let i = row * cols + c;
                        if i < total {
                            out.push(i);
                        }
                    }
                }
                out
            }
        }
    }
}

/// A drag in progress.
pub struct Marquee {
    /// Where the press happened, in content coordinates.
    origin: Pos2,
    /// The selection to keep (Ctrl / Shift held at the press), else empty.
    base: HashSet<u64>,
    /// Set once the pointer has moved far enough to count as a drag rather than a click.
    moved: bool,
    /// Hit range of the last frame, to skip recomputing the selection when it did not change.
    last: Vec<usize>,
}

/// Distance in points before a press on empty space becomes a drag.
const SLOP: f32 = 4.0;
/// Autoscroll: distance from the top / bottom edge where it starts, and points per frame.
const EDGE: f32 = 24.0;
const SPEED: f32 = 14.0;
/// Width on the right kept for the scrollbar.
const SCROLLBAR: f32 = 14.0;

/// Call once per frame after the list or grid was drawn. `area` is the scrolling region.
pub fn run(ui: &mut Ui, cx: &mut Cx<'_>, view: &View, area: Rect, layout: Layout) {
    let (pos, pressed, down, additive, ctrl) = ui.input(|i| {
        (
            i.pointer.latest_pos(),
            i.pointer.primary_pressed(),
            i.pointer.primary_down(),
            i.modifiers.command || i.modifiers.shift,
            i.modifiers.command,
        )
    });
    let total = view.len();
    let scroll = cx.state.list_scroll;
    let to_content = |p: Pos2| pos2(p.x - area.left(), p.y - area.top() + scroll);

    if cx.state.marquee.is_none() {
        let Some(p) = pos else { return };
        let usable = Rect::from_min_max(area.min, pos2(area.right() - SCROLLBAR, area.bottom()));
        if !pressed
            || !usable.contains(p)
            || cx.state.menu.is_some()
            || cx.state.rename.is_some()
            || ui.ctx().layer_id_at(p) != Some(ui.layer_id())
            // Ctrl + drag starts the band even on top of an item (and then no file drag starts).
            || (!ctrl && layout.item_at(to_content(p), total).is_some())
        {
            return;
        }
        cx.state.marquee = Some(Marquee {
            origin: to_content(p),
            base: if additive {
                cx.state.selected.clone()
            } else {
                HashSet::new()
            },
            moved: false,
            last: Vec::new(),
        });
        return;
    }

    let Some(mut m) = cx.state.marquee.take() else {
        return;
    };
    let Some(p) = pos else { return };

    if !down {
        // Released without moving: a click on empty space.
        if !m.moved && !additive {
            cx.state.clear_selection();
        }
        return;
    }

    let cur = to_content(p);
    if !m.moved && (cur - m.origin).length() < SLOP {
        cx.state.marquee = Some(m);
        return;
    }
    m.moved = true;

    // Autoscroll while the pointer is near (or past) the top or bottom edge.
    let dy = if p.y < area.top() + EDGE {
        -SPEED
    } else if p.y > area.bottom() - EDGE {
        SPEED
    } else {
        0.0
    };
    if dy != 0.0 {
        let rows_h = match layout {
            Layout::List { pitch, .. } => pitch * total as f32,
            Layout::Grid { cols, cell_h, .. } => cell_h * total.div_ceil(cols.max(1)) as f32,
        };
        let max = (rows_h - area.height()).max(0.0);
        cx.state.scroll_to = Some((scroll + dy).clamp(0.0, max));
        ui.ctx().request_repaint();
    }

    let band = Rect::from_two_pos(m.origin, cur);
    let hits = layout.items_in(band, total);
    if hits != m.last {
        let mut selected = m.base.clone();
        let mut last_key = None;
        for &i in &hits {
            if let Some(e) = view.get(i) {
                selected.insert(e.key);
                last_key = Some(e.key);
            }
        }
        cx.state.selected = selected;
        if let Some(k) = last_key {
            cx.state.focused = Some(k);
        }
        cx.state.anchor = hits.first().copied();
        m.last = hits;
    }

    // Feedback.
    let th = cx.theme;
    let screen = Rect::from_min_max(
        area.min + vec2(band.left(), band.top() - scroll),
        area.min + vec2(band.right(), band.bottom() - scroll),
    )
    .intersect(area);
    let p = ui.painter().with_clip_rect(area);
    p.rect_filled(screen, 2.0, th.palette.selection_band.gamma_multiply(0.18));
    p.rect_stroke(
        screen,
        2.0,
        eframe::egui::Stroke::new(1.0, th.palette.selection_band),
        StrokeKind::Inside,
    );
    cx.state.marquee = Some(m);
}
