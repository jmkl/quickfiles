//! Context menu, confirmation dialog and toast. Painted on the foreground layer and driven by
//! pointer position, so they need no egui widget machinery. While a menu is open every other
//! widget is hover-only (`widgets::hit`), so a click never reaches what is underneath.

use std::time::Duration;

use eframe::egui::{
    Color32, Id, Key, LayerId, Order, Pos2, Rect, Stroke, StrokeKind, Ui, pos2, vec2,
};

use super::{Action, Cx, widgets::text_left};

const WIDTH: f32 = 220.0;
const ROW_H: f32 = 26.0;
const PAD: f32 = 5.0;
const HEADER_H: f32 = 30.0;

pub struct MenuItem {
    pub label: String,
    /// `None` draws a separator.
    pub action: Option<Action>,
    pub enabled: bool,
    pub checked: bool,
}

impl MenuItem {
    pub fn new(label: impl Into<String>, action: Action) -> Self {
        Self {
            label: label.into(),
            action: Some(action),
            enabled: true,
            checked: false,
        }
    }

    pub fn disabled(mut self, disabled: bool) -> Self {
        self.enabled = !disabled;
        self
    }

    pub fn checked(mut self, on: bool) -> Self {
        self.checked = on;
        self
    }

    pub fn separator() -> Self {
        Self {
            label: String::new(),
            action: None,
            enabled: false,
            checked: false,
        }
    }
}

pub struct MenuState {
    pub pos: Pos2,
    pub header: Option<String>,
    pub items: Vec<MenuItem>,
    /// Opened this frame: ignore the click that opened it.
    pub fresh: bool,
}

impl MenuState {
    pub fn at(pos: Pos2, items: Vec<MenuItem>) -> Self {
        Self {
            pos,
            header: None,
            items,
            fresh: true,
        }
    }

    pub fn with_header(mut self, header: impl Into<String>) -> Self {
        self.header = Some(header.into());
        self
    }

    fn height(&self) -> f32 {
        let items: f32 = self
            .items
            .iter()
            .map(|i| if i.action.is_some() { ROW_H } else { 9.0 })
            .sum();
        items + PAD * 2.0 + if self.header.is_some() { HEADER_H } else { 0.0 }
    }
}

pub fn show(ui: &mut Ui, cx: &mut Cx<'_>, full: Rect) {
    let ctx = ui.ctx().clone();
    let Some(menu) = cx.state.menu.as_mut() else {
        return;
    };

    let th = cx.theme;
    let pal = &th.palette;
    let size = vec2(WIDTH, menu.height());
    let min = pos2(
        menu.pos
            .x
            .min(full.right() - size.x - 4.0)
            .max(full.left() + 4.0),
        menu.pos
            .y
            .min(full.bottom() - size.y - 4.0)
            .max(full.top() + 4.0),
    );
    let rect = Rect::from_min_size(min, size);
    let p = ctx.layer_painter(LayerId::new(Order::Foreground, Id::new("qf_menu")));

    p.rect_filled(
        rect.translate(vec2(0.0, 3.0)).expand(1.0),
        8.0,
        Color32::from_black_alpha(70),
    );
    p.rect_filled(rect, 8.0, pal.chrome);
    p.rect_stroke(
        rect,
        8.0,
        Stroke::new(1.0, pal.track_border),
        StrokeKind::Inside,
    );

    let hover = ctx.input(|i| i.pointer.hover_pos());
    let clicked = ctx.input(|i| i.pointer.primary_clicked() || i.pointer.secondary_clicked());
    let mut y = rect.top() + PAD;
    let mut chosen: Option<Action> = None;

    if let Some(header) = &menu.header {
        text_left(
            &p,
            rect.left() + 12.0,
            y + HEADER_H * 0.5 - 2.0,
            header,
            &th.font,
            pal.text_section,
            WIDTH - 24.0,
        );
        y += HEADER_H;
    }
    for item in &menu.items {
        let Some(action) = &item.action else {
            let ly = y + 4.0;
            super::widgets::hline(&p, ly, rect.left() + 8.0, rect.right() - 8.0, pal.border);
            y += 9.0;
            continue;
        };
        let row = Rect::from_min_size(pos2(rect.left() + 4.0, y), vec2(WIDTH - 8.0, ROW_H));
        let over = hover.is_some_and(|h| row.contains(h));
        if over && item.enabled {
            p.rect_filled(row, th.metrics.radius, pal.hover);
        }
        let color = if item.enabled {
            pal.text
        } else {
            pal.icon_disabled
        };
        text_left(
            &p,
            row.left() + 24.0,
            row.center().y,
            &item.label,
            &th.font,
            color,
            row.width() - 34.0,
        );
        if item.checked {
            let c = pos2(row.left() + 12.0, row.center().y);
            let st = Stroke::new(1.5, pal.accent);
            p.line_segment([c + vec2(-3.5, 0.0), c + vec2(-1.0, 2.5)], st);
            p.line_segment([c + vec2(-1.0, 2.5), c + vec2(3.5, -2.5)], st);
        }
        if over && item.enabled && clicked && !menu.fresh {
            chosen = Some(action.clone());
        }
        y += ROW_H;
    }

    let outside = hover.is_none_or(|h| !rect.contains(h));
    let escape = ctx.input(|i| i.key_pressed(Key::Escape));
    let fresh = std::mem::replace(&mut menu.fresh, false);
    let close = chosen.is_some() || escape || (clicked && outside && !fresh);
    if let Some(a) = chosen {
        cx.actions.push(a);
    }
    if close {
        cx.state.menu = None;
    }
    ctx.request_repaint();
}

/// Short message at the bottom centre; fades out by itself.
pub fn toast(ui: &mut Ui, cx: &mut Cx<'_>, full: Rect) {
    const SHOW: f32 = 3.5;
    let Some((text, since)) = cx.state.toast.clone() else {
        return;
    };
    let age = since.elapsed().as_secs_f32();
    if age > SHOW {
        cx.state.toast = None;
        return;
    }
    let ctx = ui.ctx().clone();
    ctx.request_repaint_after(Duration::from_millis(100));
    let alpha = ((SHOW - age) / 0.5).clamp(0.0, 1.0);
    let th = cx.theme;
    let pal = &th.palette;
    let p = ctx.layer_painter(LayerId::new(Order::Tooltip, Id::new("qf_toast")));
    let g = super::widgets::galley(&p, &text, &th.font, pal.text, full.width() - 120.0);
    let size = g.size() + vec2(32.0, 18.0);
    let rect = Rect::from_center_size(pos2(full.center().x, full.bottom() - 34.0), size);
    p.rect_filled(rect, size.y * 0.5, pal.chrome.gamma_multiply(alpha));
    p.rect_stroke(
        rect,
        size.y * 0.5,
        Stroke::new(1.0, pal.track_border.gamma_multiply(alpha)),
        StrokeKind::Inside,
    );
    p.galley(
        rect.center() - g.size() * 0.5,
        g,
        pal.text.gamma_multiply(alpha),
    );
}
