//! Drag and drop. In-app: drag rows onto a folder (list, grid, sidebar, breadcrumb) to move,
//! hold Ctrl to copy. From other programs (`quickfiles_core::ole_drop`): files from Explorer are
//! copied, an image dragged from a browser is downloaded, into the selected folder or else the
//! open one. (Dragging out to Explorer needs OLE `DoDragDrop`; not implemented.)

use std::path::{Path, PathBuf};

use eframe::egui::{
    Color32, CursorIcon, Id, Key, LayerId, Order, Rect, Stroke, StrokeKind, Ui, pos2, vec2,
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
    // Something dragged in from another program: light up the folder under the pointer and
    // remember it, so releasing there drops into it (see `App::handle_drop`).
    if cx.state.drop_hover.is_some() {
        let Some(pos) = cx.state.drop_pos else {
            return false;
        };
        if !rect.contains(pos) || dest.as_os_str().is_empty() {
            return false;
        }
        ui.painter().rect_filled(
            rect,
            cx.theme.metrics.radius,
            cx.theme.palette.accent.gamma_multiply(0.25),
        );
        ui.painter().rect_stroke(
            rect,
            cx.theme.metrics.radius,
            Stroke::new(1.5, cx.theme.palette.accent),
            StrokeKind::Inside,
        );
        cx.state.drop_over = Some(dest.to_owned());
        return true;
    }
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
            p.rect_filled(rect, 6.0, pal.surface);
            p.rect_stroke(rect, 6.0, Stroke::new(1.0, pal.accent), StrokeKind::Inside);
            p.galley(rect.min + vec2(10.0, 5.0), g, pal.text);
        }
        ctx.request_repaint();
        let (released, escape, at, copy) = ctx.input(|i| {
            (
                i.pointer.any_released(),
                i.key_pressed(Key::Escape),
                i.pointer.hover_pos().or(i.pointer.latest_pos()),
                i.modifiers.command,
            )
        });
        if released && !escape {
            // Dropped on a tab button: into the folder that tab shows.
            let hit = at.and_then(|p| cx.state.tab_hits.iter().find(|(r, _)| r.contains(p)));
            match hit.map(|(_, dir)| dir.clone()) {
                Some(Some(dest)) if valid_target(&drag, &dest) => {
                    cx.actions.push(Action::Transfer {
                        paths: drag.paths.clone(),
                        dest,
                        copy,
                        policy: None,
                    });
                }
                Some(_) => cx
                    .state
                    .say("That tab is not showing a folder to drop into".to_owned()),
                // Not on a tab: dropped on the list itself (empty space or a file row) goes into
                // the open folder, unless a folder row already took it.
                None => {
                    let in_list =
                        at.is_some_and(|p| cx.state.list_area.is_some_and(|r| r.contains(p)));
                    let taken = cx
                        .actions
                        .iter()
                        .any(|a| matches!(a, Action::Transfer { .. }));
                    let here = cx
                        .core
                        .nav()
                        .location()
                        .filter(|d| valid_target(&drag, d))
                        .cloned();
                    if let (true, false, Some(dest)) = (in_list, taken, here) {
                        // Items already in this folder stay put (a drop back where it started).
                        let paths: Vec<_> = drag
                            .paths
                            .iter()
                            .filter(|p| p.parent() != Some(dest.as_path()))
                            .cloned()
                            .collect();
                        if !paths.is_empty() {
                            cx.actions.push(Action::Transfer {
                                paths,
                                dest,
                                copy,
                                policy: None,
                            });
                        }
                    }
                }
            }
        }
        if released || escape {
            cx.state.drag = None;
        }
    }

    // Something coming in from another program (files from Explorer, an image from a browser).
    // A folder under the pointer lights up on its own; with none, the whole window is the target.
    if let Some(hover) = cx.state.drop_hover.clone() {
        use quickfiles_core::ole_drop::Hover;
        let over = cx
            .state
            .drop_over
            .as_deref()
            .map(quickfiles_core::display_name);
        let dest = over.clone().or_else(|| cx.state.drop_dest.clone());
        let p = ctx.layer_painter(LayerId::new(Order::Foreground, Id::new("qf_drop_banner")));
        if over.is_none() {
            p.rect_filled(full, 0.0, Color32::from_black_alpha(90));
            p.rect_stroke(
                full.shrink(2.0),
                6.0,
                Stroke::new(3.0, pal.accent),
                StrokeKind::Inside,
            );
        }
        let text = match (&hover, dest.as_deref()) {
            (Hover::Other, _) => "This cannot be dropped here".to_owned(),
            (_, None) => "Open a folder first".to_owned(),
            (Hover::Files(1), Some(d)) => format!("Copy 1 item into \"{d}\""),
            (Hover::Files(n), Some(d)) => format!("Copy {n} items into \"{d}\""),
            (Hover::Image, Some(d)) => format!("Download the image into \"{d}\""),
        };
        let g = galley(&p, &text, &th.font, pal.text, full.width() - 80.0);
        let pill = Rect::from_center_size(
            pos2(full.center().x, full.bottom() - 46.0),
            g.size() + vec2(28.0, 14.0),
        );
        p.rect_filled(pill, 8.0, pal.surface);
        p.rect_stroke(pill, 8.0, Stroke::new(1.0, pal.accent), StrokeKind::Inside);
        p.galley(pill.min + vec2(14.0, 7.0), g, pal.text);
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
