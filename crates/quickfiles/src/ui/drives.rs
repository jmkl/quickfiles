//! "This PC": the drive listing with usage. Detail list (File System, Free Space, Size and a
//! Percent Full bar) and a tile grid (big drive icon, usage bar, "X free of Y").

use eframe::egui::{Align2, Color32, Rect, Sense, Stroke, StrokeKind, Ui, pos2, vec2};
use quickfiles_core::{DriveInfo, EntryRef, View, format_size};

use super::{
    Action, Cx,
    icons::{self, Icon},
    widgets::{hit, text_left, vline},
};

/// True while the listing is the virtual "This PC" root (no folder, no search text).
pub fn is_drives(cx: &Cx<'_>) -> bool {
    let q = cx.core.nav().query();
    q.text.is_empty()
        && q.location
            .as_ref()
            .is_some_and(|p| p.as_os_str().is_empty())
}

pub fn show(ui: &mut Ui, cx: &mut Cx<'_>, header: Rect, area: Rect) {
    let view = cx.core.view().clone();
    let drives = cx.core.drives();
    cx.state.scroll_to = None;
    cx.state.scroll_to_focus = false;
    cx.state.list_scroll = 0.0;
    view.want(0..view.len());
    if cx.state.grid {
        ui.painter().rect_filled(header, 0.0, cx.theme.palette.main);
        grid(ui, cx, &view, &drives, area);
    } else {
        list(ui, cx, &view, &drives, header, area);
    }
}

fn info_for<'a>(drives: &'a [DriveInfo], entry: &EntryRef) -> Option<&'a DriveInfo> {
    drives
        .iter()
        .find(|d| d.root.to_string_lossy().eq_ignore_ascii_case(&entry.parent))
}

fn bar_color(cx: &Cx<'_>, d: &DriveInfo) -> Color32 {
    if d.nearly_full() {
        cx.theme.palette.danger
    } else {
        cx.theme.palette.accent
    }
}

/// Selection, double-click, context menu: the same behaviour as a normal row.
fn interact(ui: &Ui, cx: &mut Cx<'_>, entry: &EntryRef, i: usize, rect: Rect) -> bool {
    let response = hit(ui, rect, ("drive_row", i), Sense::click());
    let key = entry.key;
    if response.clicked() {
        cx.state.select_only(key, i);
    }
    if response.double_clicked() {
        cx.actions.push(Action::OpenPath {
            path: entry.path(),
            is_dir: true,
        });
    }
    if response.secondary_clicked() {
        if !cx.state.selected.contains(&key) {
            cx.state.select_only(key, i);
        }
        let pos = ui
            .input(|input| input.pointer.interact_pos())
            .unwrap_or(rect.center());
        cx.state.menu = Some(super::file_list::row_menu(cx, entry, pos));
    }
    response.hovered()
}

fn highlight(ui: &Ui, cx: &Cx<'_>, entry: &EntryRef, rect: Rect, hovered: bool, radius: f32) {
    let pal = &cx.theme.palette;
    let p = ui.painter();
    if cx.state.selected.contains(&entry.key) {
        p.rect_filled(rect, radius, pal.accent.gamma_multiply(0.28));
    } else if hovered {
        p.rect_filled(rect, radius, pal.hover);
    }
    if cx.state.focused == Some(entry.key) {
        p.rect_stroke(
            rect,
            radius,
            Stroke::new(1.0, pal.accent),
            StrokeKind::Inside,
        );
    }
}

fn list(ui: &mut Ui, cx: &mut Cx<'_>, view: &View, drives: &[DriveInfo], header: Rect, area: Rect) {
    let th = cx.theme;
    let (pal, m) = (&th.palette, &th.metrics);
    let pad = m.col_text_pad;
    let p = ui.painter().clone();

    // Right-anchored columns; drop the leftmost ones when the window is narrow.
    let right = area.right() - m.list_pad;
    let defs: [(&str, f32); 5] = [
        ("Percent Full", 250.0),
        ("Size", 96.0),
        ("Free Space", 104.0),
        ("File System", 92.0),
        ("Type", 120.0),
    ];
    let name_min = 220.0;
    let mut xs: Vec<(&str, f32, f32)> = Vec::new(); // (label, left, width)
    let mut x = right;
    for (label, w) in defs {
        if x - w < area.left() + m.list_pad + name_min {
            break;
        }
        x -= w;
        xs.push((label, x, w));
    }
    let col = |name: &str| xs.iter().find(|c| c.0 == name).copied();

    // Header.
    let cy = header.top() + 13.5;
    text_left(
        &p,
        area.left() + m.list_pad + 34.0,
        cy,
        "Name",
        &th.font,
        pal.text_dim,
        120.0,
    );
    for (label, left, _) in &xs {
        vline(
            &p,
            *left,
            header.top() + 4.0,
            header.top() + 23.0,
            pal.border,
        );
        text_left(&p, left + pad, cy, label, &th.font, pal.text_dim, 110.0);
    }

    // Rows.
    let total = view.len();
    for i in 0..total {
        let Some(entry) = view.get(i) else { continue };
        let top = area.top() + i as f32 * m.row_pitch;
        let rect = Rect::from_min_size(
            pos2(area.left() + m.list_pad, top),
            vec2(area.width() - m.list_pad * 2.0, m.row_pitch),
        );
        if rect.bottom() > area.bottom() {
            break;
        }
        let hovered = interact(ui, cx, &entry, i, rect);
        highlight(ui, cx, &entry, rect, hovered, m.radius);

        let cy = rect.center().y;
        let info = info_for(drives, &entry);
        let icon = if info.is_some_and(|d| d.system) {
            Icon::DriveWindows
        } else {
            Icon::Drive
        };
        icons::paint(&p, pal, icon, pos2(rect.left() + 14.5, cy), pal.text);
        let name_right = xs.last().map_or(rect.right(), |c| c.1) - 8.0;
        text_left(
            &p,
            rect.left() + 34.0,
            cy,
            &entry.name,
            &th.font,
            pal.text,
            name_right - rect.left() - 34.0,
        );

        if let Some((_, left, _)) = col("Type") {
            text_left(
                &p,
                left + pad,
                cy,
                &entry.type_label,
                &th.font,
                pal.text_dim,
                100.0,
            );
        }
        let Some(d) = info else { continue };
        if let Some((_, left, _)) = col("File System") {
            text_left(&p, left + pad, cy, &d.fs, &th.font, pal.text_dim, 80.0);
        }
        if let Some((_, left, _)) = col("Free Space") {
            text_left(
                &p,
                left + pad,
                cy,
                &format_size(d.free),
                &th.font,
                pal.text_dim,
                90.0,
            );
        }
        if let Some((_, left, _)) = col("Size") {
            text_left(
                &p,
                left + pad,
                cy,
                &format_size(d.total),
                &th.font,
                pal.text_dim,
                82.0,
            );
        }
        if let Some((_, left, w)) = col("Percent Full") {
            let bar =
                Rect::from_min_max(pos2(left + pad, cy - 8.0), pos2(left + w - 12.0, cy + 8.0));
            p.rect_filled(bar, 2.0, pal.track);
            p.rect_stroke(
                bar,
                2.0,
                Stroke::new(1.0, pal.track_border),
                StrokeKind::Inside,
            );
            let fill = Rect::from_min_size(
                bar.min,
                vec2(bar.width() * d.used.clamp(0.0, 1.0), bar.height()),
            );
            p.rect_filled(fill, 2.0, bar_color(cx, d));
            p.text(
                bar.center(),
                Align2::CENTER_CENTER,
                format!("{:.1}%", d.used * 100.0),
                th.font.clone(),
                pal.text,
            );
        }
    }
}

const CELL_W: f32 = 150.0;
const CELL_H: f32 = 140.0;

fn grid(ui: &mut Ui, cx: &mut Cx<'_>, view: &View, drives: &[DriveInfo], area: Rect) {
    let th = cx.theme;
    let pal = &th.palette;
    let p = ui.painter().clone();
    let avail = area.width() - 24.0;
    let cols = ((avail / CELL_W).floor() as usize).max(1);
    cx.state.grid_cols = cols;

    for i in 0..view.len() {
        let Some(entry) = view.get(i) else { continue };
        let (row, col) = (i / cols, i % cols);
        let rect = Rect::from_min_size(
            pos2(
                area.left() + 12.0 + col as f32 * CELL_W,
                area.top() + 8.0 + row as f32 * CELL_H,
            ),
            vec2(CELL_W - 6.0, CELL_H - 6.0),
        );
        if rect.top() > area.bottom() {
            break;
        }
        let hovered = interact(ui, cx, &entry, i, rect);
        highlight(ui, cx, &entry, rect, hovered, 8.0);

        let info = info_for(drives, &entry);
        let icon = if info.is_some_and(|d| d.system) {
            Icon::DriveWindows
        } else {
            Icon::Drive
        };
        icons::paint_scaled(
            &p,
            pal,
            icon,
            pos2(rect.center().x, rect.top() + 40.0),
            pal.text,
            3.4,
        );
        text_left(
            &p,
            rect.left() + 8.0,
            rect.top() + 84.0,
            &entry.name,
            &th.font,
            pal.text,
            rect.width() - 16.0,
        );

        let Some(d) = info else { continue };
        let bar = Rect::from_min_size(
            pos2(rect.left() + 8.0, rect.top() + 98.0),
            vec2(rect.width() - 16.0, 10.0),
        );
        p.rect_filled(bar, 1.0, pal.track);
        p.rect_stroke(
            bar,
            1.0,
            Stroke::new(1.0, pal.track_border),
            StrokeKind::Inside,
        );
        let fill = Rect::from_min_size(
            bar.min,
            vec2(bar.width() * d.used.clamp(0.0, 1.0), bar.height()),
        );
        p.rect_filled(fill, 1.0, bar_color(cx, d));
        let text = format!("{} free of {}", format_size(d.free), format_size(d.total));
        text_left(
            &p,
            rect.left() + 8.0,
            rect.top() + 118.0,
            &text,
            &th.font,
            pal.text_dim,
            rect.width() - 16.0,
        );
    }
}
