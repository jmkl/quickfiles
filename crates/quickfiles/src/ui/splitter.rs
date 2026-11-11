//! Drag handles on the inner edges of the side panels. Only active once a panel has finished
//! sliding open.

use eframe::egui::{CursorIcon, Id, Rect, Response, Sense, Ui, pos2, vec2};

use super::{
    Cx,
    layout::{MIN_LIST_W, Regions},
};
use crate::state::{PREVIEW_MAX_W, PREVIEW_MIN_W, SIDEBAR_MAX_W, SIDEBAR_MIN_W};

pub fn show(
    ui: &mut Ui,
    cx: &mut Cx<'_>,
    full: Rect,
    regions: &Regions,
    left_ready: bool,
    right_ready: bool,
) {
    let top = regions.title.bottom();
    let bottom = full.bottom();
    let left_vis = regions.sidebar.map_or(0.0, |p| p.visible.width());
    let right_vis = regions.preview.map_or(0.0, |p| p.visible.width());

    if let (true, Some(panel)) = (left_ready, regions.sidebar) {
        let handle = handle(ui, cx, "split_left", panel.visible.right(), top, bottom);
        if handle.dragged() {
            if let Some(pos) = handle.interact_pointer_pos() {
                let max = (full.width() - right_vis - MIN_LIST_W)
                    .min(SIDEBAR_MAX_W)
                    .max(SIDEBAR_MIN_W);
                cx.state.left_w = (pos.x - full.left()).clamp(SIDEBAR_MIN_W, max);
            }
        }
    }

    if let (true, Some(panel)) = (right_ready, regions.preview) {
        let handle = handle(ui, cx, "split_right", panel.visible.left(), top, bottom);
        if handle.dragged() {
            if let Some(pos) = handle.interact_pointer_pos() {
                let max = (full.width() - left_vis - MIN_LIST_W)
                    .min(PREVIEW_MAX_W)
                    .max(PREVIEW_MIN_W);
                cx.state.preview_w = (full.right() - pos.x).clamp(PREVIEW_MIN_W, max);
            }
        }
    }
}

/// A 6px-wide grab strip centred on `x`; shows a thin accent line while hovered or dragged.
fn handle(ui: &Ui, cx: &Cx<'_>, salt: &'static str, x: f32, y0: f32, y1: f32) -> Response {
    let rect = Rect::from_min_max(pos2(x - 3.0, y0), pos2(x + 3.0, y1));
    let response = ui.interact(rect, Id::new(salt), Sense::drag());
    if response.hovered() || response.dragged() {
        ui.ctx().set_cursor_icon(CursorIcon::ResizeHorizontal);
        let line = Rect::from_center_size(pos2(x, (y0 + y1) * 0.5), vec2(2.0, y1 - y0));
        ui.painter().rect_filled(line, 0.0, cx.theme.palette.accent);
    }
    response
}
