//! Left sidebar: collapsible Recents / Bookmarks / Storage / Places.

use std::path::{Path, PathBuf};

use eframe::egui::{Rect, ScrollArea, Sense, Stroke, StrokeKind, Ui, UiBuilder, pos2, vec2};
use quickfiles_core::{DriveInfo, display_name};

use super::{
    Action, Cx, dnd,
    icons::{self, Icon},
    widgets::{click_sense, text_left, vline},
};
use crate::theme::Theme;

pub fn show(ui: &mut Ui, cx: &mut Cx<'_>, r: Rect) {
    let pal = &cx.theme.palette;
    let p = ui.painter().clone();

    p.rect_filled(r, 0.0, pal.sidebar);
    vline(&p, r.right() - 1.0, r.top(), r.bottom(), pal.border);
    let inner_right = r.right() - 1.0;

    // Scrollable sections.
    let scroll_rect = Rect::from_min_max(r.min, pos2(inner_right, r.bottom()));
    let mut scroll_ui = ui.new_child(UiBuilder::new().max_rect(scroll_rect));
    ScrollArea::vertical()
        .id_salt("sidebar_scroll")
        .auto_shrink([false, false])
        .show(&mut scroll_ui, |ui| sections(ui, cx));
}

fn sections(ui: &mut Ui, cx: &mut Cx<'_>) {
    let th = cx.theme;
    let gap = th.metrics.side_section_gap;
    let data = cx.core.data();
    let drives = cx.core.drives();
    let places: Vec<(String, PathBuf)> = std::iter::once(("This PC".to_owned(), PathBuf::new()))
        .chain(
            cx.core
                .places()
                .iter()
                .map(|p| (p.label.clone(), p.path.clone())),
        )
        .collect();

    ui.add_space(5.0);
    if section_header(
        ui,
        th,
        Icon::Clock,
        "Recents",
        &mut cx.state.section_open[0],
    ) {
        if data.recents.is_empty() {
            hint(ui, cx, "Folders you open show up here");
        }
        for path in data.recents.iter().take(10) {
            side_item(ui, cx, Icon::Folder, &display_name(path), path, false);
        }
    }

    ui.add_space(gap);
    if section_header(
        ui,
        th,
        Icon::Bookmark,
        "Bookmarks",
        &mut cx.state.section_open[1],
    ) {
        if data.bookmarks.is_empty() {
            hint(ui, cx, "Bookmark the folder you are in");
        }
        for b in &data.bookmarks {
            side_item(ui, cx, Icon::Folder, &b.label, &b.path, true);
        }
    }

    ui.add_space(gap);
    if section_header(
        ui,
        th,
        Icon::Storage,
        "Storage",
        &mut cx.state.section_open[2],
    ) {
        for drive in drives.iter() {
            drive_item(ui, cx, drive);
        }
    }

    ui.add_space(gap);
    if section_header(ui, th, Icon::Pin, "Places", &mut cx.state.section_open[3]) {
        for (label, path) in &places {
            let icon = if path.as_os_str().is_empty() {
                Icon::ThisPc
            } else {
                Icon::Folder
            };
            side_item(ui, cx, icon, label, path, false);
        }
    }
    ui.add_space(gap);
}

fn hint(ui: &mut Ui, cx: &Cx<'_>, text: &str) {
    let m = &cx.theme.metrics;
    let (rect, _) =
        ui.allocate_exact_size(vec2(ui.available_width(), m.side_row_h), Sense::hover());
    text_left(
        ui.painter(),
        rect.left() + m.side_text_x,
        rect.center().y,
        text,
        &cx.theme.font,
        cx.theme.palette.text_dim,
        rect.width() - m.side_text_x - 10.0,
    );
}

/// Returns whether the section is open (after handling a click on the header).
fn section_header(ui: &mut Ui, th: &Theme, icon: Icon, title: &str, open: &mut bool) -> bool {
    let pal = &th.palette;
    let (rect, response) = ui.allocate_exact_size(
        vec2(ui.available_width(), th.metrics.side_row_h),
        Sense::click(),
    );
    let p = ui.painter();
    let cy = rect.center().y;

    if response.hovered() {
        p.rect_filled(rect.shrink2(vec2(4.0, 0.0)), th.metrics.radius, pal.hover);
    }
    icons::paint_scaled(
        p,
        pal,
        icon,
        pos2(rect.left() + 19.5, cy),
        pal.text_section,
        0.8,
    );
    text_left(
        p,
        rect.left() + 36.0,
        cy,
        title,
        &th.font,
        pal.text_section,
        rect.width() - 80.0,
    );

    let chevron = if *open {
        Icon::ChevronDown
    } else {
        Icon::ChevronRight
    };
    icons::paint_scaled(
        p,
        pal,
        chevron,
        pos2(rect.right() - 28.5, cy),
        pal.text_section,
        1.1,
    );

    if response.clicked() {
        *open = !*open;
    }
    *open
}

fn side_item(ui: &mut Ui, cx: &mut Cx<'_>, icon: Icon, label: &str, path: &Path, removable: bool) {
    let th = cx.theme;
    let (pal, m) = (&th.palette, &th.metrics);
    let (rect, response) =
        ui.allocate_exact_size(vec2(ui.available_width(), m.side_row_h), click_sense(ui));
    let p = ui.painter();
    let cy = rect.center().y;

    let current = cx.core.nav().location().is_some_and(|l| l == path);
    if current {
        p.rect_filled(
            rect.shrink2(vec2(4.0, 0.0)),
            m.radius,
            pal.accent.gamma_multiply(0.18),
        );
    } else if response.hovered() {
        p.rect_filled(rect.shrink2(vec2(4.0, 0.0)), m.radius, pal.hover);
    }
    icons::paint(
        p,
        pal,
        icon,
        pos2(rect.left() + m.side_icon_x, cy),
        pal.text,
    );
    text_left(
        p,
        rect.left() + m.side_text_x,
        cy,
        label,
        &th.font,
        pal.text,
        rect.width() - m.side_text_x - 10.0,
    );

    dnd::target(ui, cx, rect.shrink2(vec2(4.0, 0.0)), path);
    if response.clicked() {
        cx.actions.push(Action::Navigate(path.to_owned()));
    }
    if removable && response.secondary_clicked() {
        cx.actions.push(Action::RemoveBookmark(path.to_owned()));
    }
}

/// Drive name with a capacity bar underneath (blue normally, red when >= 90% full).
fn drive_item(ui: &mut Ui, cx: &mut Cx<'_>, drive: &DriveInfo) {
    let th = cx.theme;
    let (pal, m) = (&th.palette, &th.metrics);
    let (rect, response) =
        ui.allocate_exact_size(vec2(ui.available_width(), m.side_drive_h), click_sense(ui));
    let p = ui.painter();

    if response.hovered() {
        p.rect_filled(rect.shrink2(vec2(4.0, 0.0)), m.radius, pal.hover);
    }

    let name_cy = rect.top() + 10.5;
    let icon = if drive.system {
        Icon::DriveWindows
    } else {
        Icon::Drive
    };
    icons::paint(
        p,
        pal,
        icon,
        pos2(rect.left() + m.side_icon_x, name_cy),
        pal.text,
    );
    text_left(
        p,
        rect.left() + m.side_text_x,
        name_cy,
        &drive.label,
        &th.font,
        pal.text,
        rect.width() - m.side_text_x - 26.0,
    );

    let bar = Rect::from_min_max(
        pos2(rect.left() + m.side_text_x, rect.top() + 22.0),
        pos2(rect.right() - 26.0, rect.top() + 27.0),
    );
    p.rect_filled(bar, 1.0, pal.track);
    p.rect_stroke(
        bar,
        1.0,
        Stroke::new(1.0, pal.track_border),
        StrokeKind::Inside,
    );
    let filled = Rect::from_min_size(
        bar.min,
        vec2(bar.width() * drive.used.clamp(0.0, 1.0), bar.height()),
    );
    p.rect_filled(
        filled,
        1.0,
        if drive.nearly_full() {
            pal.danger
        } else {
            pal.accent
        },
    );

    dnd::target(ui, cx, rect.shrink2(vec2(4.0, 0.0)), &drive.root);
    if response.clicked() {
        cx.actions.push(Action::Navigate(drive.root.clone()));
    }
}
