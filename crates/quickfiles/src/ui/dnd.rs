//! Drag and drop. In-app: drag rows onto a folder (list, grid, sidebar, breadcrumb) to move,
//! hold Ctrl to copy. From Explorer: files dropped on the window are copied into the current
//! folder. (Dragging out to Explorer needs OLE `DoDragDrop`; not implemented.)

use std::path::{Path, PathBuf};

use eframe::egui::{
    Align2, Color32, CursorIcon, Id, Key, LayerId, Order, Rect, Stroke, StrokeKind, Ui, pos2, vec2,
};

use super::{Action, Cx, widgets::galley};
use crate::state::Drag;

/// Starts dragging the selection (called when a row's drag begins).
pub fn begin(cx: &mut Cx<'_>) {
    let paths = cx.core.view().paths_of(&cx.state.selected);
    if paths.is_empty() {
        return;
    }
    let label = if paths.len() == 1 {
        paths[0]
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default()
    } else {
        format!("{} items", paths.len())
    };
    cx.state.drag = Some(Drag { paths, label });
}

/// A folder is a valid target unless it is one of the dragged items or inside one.
fn valid_target(drag: &Drag, dest: &Path) -> bool {
    !dest.as_os_str().is_empty() && !drag.paths.iter().any(|p| dest.starts_with(p))
}

/// Marks `rect` as a drop target for `dest`. Highlights it while a drag hovers it and queues
/// the transfer when the button is released there. Returns whether it is hovered.
pub fn target(ui: &Ui, cx: &mut Cx<'_>, rect: Rect, dest: &Path) -> bool {
    let Some(drag) = cx.state.drag.as_ref() else {
        return false;
    };
    let Some(pos) = ui.input(|i| i.pointer.hover_pos()) else {
        return false;
    };
    if !rect.contains(pos) || !valid_target(drag, dest) {
        return false;
    }
    ui.painter().rect_stroke(
        rect,
        cx.theme.metrics.radius,
        Stroke::new(1.5, cx.theme.palette.accent),
        StrokeKind::Inside,
    );
    if ui.input(|i| i.pointer.any_released()) {
        let copy = ui.input(|i| i.modifiers.command);
        cx.actions.push(Action::Transfer {
            paths: drag.paths.clone(),
            dest: dest.to_owned(),
            copy,
            policy: None,
        });
    }
    true
}

/// Last thing each frame: the floating label, Esc / release ends the drag, and the banner
/// for files hovering over the window from Explorer.
pub fn overlay(ui: &mut Ui, cx: &mut Cx<'_>, full: Rect) {
    let ctx = ui.ctx().clone();
    let th = cx.theme;
    let pal = &th.palette;

    if let Some(drag) = cx.state.drag.clone() {
        ctx.set_cursor_icon(CursorIcon::Grabbing);
        if let Some(pos) = ctx.input(|i| i.pointer.hover_pos()) {
            let copy = ctx.input(|i| i.modifiers.command);
            let text = if copy {
                format!("Copy {}", drag.label)
            } else {
                drag.label.clone()
            };
            let p = ctx.layer_painter(LayerId::new(Order::Tooltip, Id::new("qf_drag")));
            let g = galley(&p, &text, &th.font, pal.text, 260.0);
            let rect = Rect::from_min_size(pos + vec2(14.0, 14.0), g.size() + vec2(20.0, 10.0));
            p.rect_filled(rect, 6.0, pal.chrome);
            p.rect_stroke(rect, 6.0, Stroke::new(1.0, pal.accent), StrokeKind::Inside);
            p.galley(rect.min + vec2(10.0, 5.0), g, pal.text);
        }
        ctx.request_repaint();
        if ctx.input(|i| i.pointer.any_released() || i.key_pressed(Key::Escape)) {
            cx.state.drag = None;
        }
    }

    // Files coming in from another program.
    if ctx.input(|i| !i.raw.hovered_files.is_empty()) {
        let p = ctx.layer_painter(LayerId::new(Order::Foreground, Id::new("qf_drop_banner")));
        p.rect_filled(full, 0.0, Color32::from_black_alpha(110));
        let text = match cx
            .core
            .nav()
            .location()
            .filter(|l| !l.as_os_str().is_empty())
        {
            Some(l) => format!("Drop to copy into {}", quickfiles_core::display_name(l)),
            None => "Open a folder first".to_owned(),
        };
        p.text(
            pos2(full.center().x, full.center().y),
            Align2::CENTER_CENTER,
            text,
            th.font.clone(),
            pal.text,
        );
    }
}

/// Paths of files dropped on the window this frame.
pub fn dropped(ctx: &eframe::egui::Context) -> Vec<PathBuf> {
    ctx.input(|i| {
        i.raw
            .dropped_files
            .iter()
            .map(|f| f.path().to_path_buf())
            .filter(|p| !p.as_os_str().is_empty())
            .collect()
    })
}
