//! Small drawing helpers shared by all regions.
use std::{fmt::Debug, hash::Hash, sync::Arc};

use eframe::egui::{
    Color32, FontId, Galley, Painter, Pos2, Rect, Response, Sense, Ui, Vec2, pos2, text::LayoutJob,
    vec2,
};

use super::icons::{self, Icon};
use crate::theme::Theme;

/// Crisp 1px horizontal line occupying pixel row `y`.
pub fn hline(p: &Painter, y: f32, x0: f32, x1: f32, color: Color32) {
    p.rect_filled(
        Rect::from_min_max(pos2(x0, y), pos2(x1, y + 1.0)),
        0.0,
        color,
    );
}

/// Crisp 1px vertical line occupying pixel column `x`.
pub fn vline(p: &Painter, x: f32, y0: f32, y1: f32, color: Color32) {
    p.rect_filled(
        Rect::from_min_max(pos2(x, y0), pos2(x + 1.0, y1)),
        0.0,
        color,
    );
}

pub fn hit(ui: &Ui, rect: Rect, salt: impl Hash + Debug, sense: Sense) -> Response {
    ui.interact(rect, ui.id().with(salt), sense)
}

/// Single-line galley that ends with "…" instead of overflowing `max_w`.
pub fn galley(p: &Painter, text: &str, font: &FontId, color: Color32, max_w: f32) -> Arc<Galley> {
    let mut job = LayoutJob::simple_singleline(text.to_owned(), font.clone(), color);
    job.wrap.max_width = max_w.max(1.0);
    job.wrap.max_rows = 1;
    job.wrap.break_anywhere = true;
    job.wrap.overflow_character = Some('…');
    p.layout_job(job)
}

/// Draws `text` with its left edge at `x`, vertically centred on `cy`. Returns the drawn width.
pub fn text_left(
    p: &Painter,
    x: f32,
    cy: f32,
    text: &str,
    font: &FontId,
    color: Color32,
    max_w: f32,
) -> f32 {
    let g = galley(p, text, font, color, max_w);
    let width = g.size().x;
    p.galley(pos2(x, cy - g.size().y * 0.5), g, color);
    width
}

/// Draws `text` with its right edge at `x`.
pub fn text_right(
    p: &Painter,
    x: f32,
    cy: f32,
    text: &str,
    font: &FontId,
    color: Color32,
    max_w: f32,
) {
    let g = galley(p, text, font, color, max_w);
    p.galley(pos2(x - g.size().x, cy - g.size().y * 0.5), g, color);
}

/// Hover-highlighted icon button centred on `center`.
pub fn icon_button(
    ui: &Ui,
    theme: &Theme,
    icon: Icon,
    center: Pos2,
    size: Vec2,
    salt: impl Hash + Debug,
    enabled: bool,
) -> Response {
    let rect = Rect::from_center_size(center, size);
    let sense = if enabled {
        Sense::click()
    } else {
        Sense::hover()
    };
    let response = hit(ui, rect, salt, sense);
    let pal = &theme.palette;
    let p = ui.painter();

    if enabled && response.hovered() {
        p.rect_filled(rect, theme.metrics.radius, pal.hover);
    }
    let tint = if !enabled {
        pal.icon_disabled
    } else if response.hovered() {
        pal.icon_strong
    } else {
        pal.icon_nav
    };
    icons::paint(p, pal, icon, center, tint);
    response
}

/// Default footprint of a toolbar icon button.
pub fn btn_size() -> Vec2 {
    vec2(28.0, 24.0)
}
