//! Resize grips for the borderless window. The OS draws no frame (`with_decorations(false)`),
//! so thin invisible strips along the edges and corners start a native resize drag.

use eframe::egui::{
    CursorIcon, Id, PointerButton, Rect, ResizeDirection, Sense, Ui, ViewportCommand, pos2, vec2,
};

/// Thickness of the edge grips, in logical pixels.
const EDGE: f32 = 5.0;
/// Size of the square corner grips.
const CORNER: f32 = 10.0;

pub fn show(ui: &mut Ui, full: Rect) {
    let ctx = ui.ctx().clone();
    let (maximized, fullscreen) = ctx.input(|i| {
        let v = i.viewport();
        (v.maximized.unwrap_or(false), v.fullscreen.unwrap_or(false))
    });
    if maximized || fullscreen {
        return;
    }

    let (l, t, r, b) = (full.left(), full.top(), full.right(), full.bottom());
    let w = full.width();
    let h = full.height();

    // Edges first, corners last so the corners win where they overlap.
    let grips = [
        (
            "n",
            Rect::from_min_size(pos2(l + CORNER, t), vec2((w - 2.0 * CORNER).max(0.0), EDGE)),
            ResizeDirection::North,
            CursorIcon::ResizeVertical,
        ),
        (
            "s",
            Rect::from_min_size(
                pos2(l + CORNER, b - EDGE),
                vec2((w - 2.0 * CORNER).max(0.0), EDGE),
            ),
            ResizeDirection::South,
            CursorIcon::ResizeVertical,
        ),
        (
            "w",
            Rect::from_min_size(pos2(l, t + CORNER), vec2(EDGE, (h - 2.0 * CORNER).max(0.0))),
            ResizeDirection::West,
            CursorIcon::ResizeHorizontal,
        ),
        (
            "e",
            Rect::from_min_size(
                pos2(r - EDGE, t + CORNER),
                vec2(EDGE, (h - 2.0 * CORNER).max(0.0)),
            ),
            ResizeDirection::East,
            CursorIcon::ResizeHorizontal,
        ),
        (
            "nw",
            Rect::from_min_size(pos2(l, t), vec2(CORNER, CORNER)),
            ResizeDirection::NorthWest,
            CursorIcon::ResizeNwSe,
        ),
        (
            "ne",
            Rect::from_min_size(pos2(r - CORNER, t), vec2(CORNER, CORNER)),
            ResizeDirection::NorthEast,
            CursorIcon::ResizeNeSw,
        ),
        (
            "sw",
            Rect::from_min_size(pos2(l, b - CORNER), vec2(CORNER, CORNER)),
            ResizeDirection::SouthWest,
            CursorIcon::ResizeNeSw,
        ),
        (
            "se",
            Rect::from_min_size(pos2(r - CORNER, b - CORNER), vec2(CORNER, CORNER)),
            ResizeDirection::SouthEast,
            CursorIcon::ResizeNwSe,
        ),
    ];

    for (name, rect, direction, cursor) in grips {
        let response = ui.interact(rect, Id::new(("resize_grip", name)), Sense::drag());
        if response.hovered() || response.dragged() {
            ctx.set_cursor_icon(cursor);
        }
        if response.drag_started_by(PointerButton::Primary) {
            ctx.send_viewport_cmd(ViewportCommand::BeginResize(direction));
        }
    }
}
