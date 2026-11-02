//! Left sidebar: collapsible Recents / Bookmarks / Storage / Places.

use std::path::PathBuf;

use eframe::egui::{Rect, ScrollArea, Sense, Stroke, StrokeKind, Ui, UiBuilder, pos2, vec2};

use super::{
    Action, Cx,
    icons::{self, Icon},
    widgets::{text_left, vline},
};
use crate::{
    data,
    model::{DriveInfo, SideItem, SideKind},
    theme::Theme,
};

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

    ui.add_space(5.0);
    if section_header(
        ui,
        th,
        Icon::Clock,
        "Recents",
        &mut cx.state.section_open[0],
    ) {
        for item in &data::RECENTS {
            side_item(ui, cx, item);
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
        for item in &data::BOOKMARKS {
            side_item(ui, cx, item);
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
        for drive in &data::DRIVES {
            drive_item(ui, cx, drive);
        }
    }

    ui.add_space(gap);
    if section_header(ui, th, Icon::Pin, "Places", &mut cx.state.section_open[3]) {
        for item in &data::PLACES {
            side_item(ui, cx, item);
        }
    }
    ui.add_space(gap);
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

fn side_item(ui: &mut Ui, cx: &mut Cx<'_>, item: &SideItem) {
    let th = cx.theme;
    let (pal, m) = (&th.palette, &th.metrics);
    let (rect, response) =
        ui.allocate_exact_size(vec2(ui.available_width(), m.side_row_h), Sense::click());
    let p = ui.painter();
    let cy = rect.center().y;

    if response.hovered() {
        p.rect_filled(rect.shrink2(vec2(4.0, 0.0)), m.radius, pal.hover);
    }
    let icon = match item.kind {
        SideKind::Folder => Icon::Folder,
        SideKind::Drive => Icon::Drive,
        SideKind::Pc => Icon::ThisPc,
    };
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
        item.label,
        &th.font,
        pal.text,
        rect.right() - rect.left() - m.side_text_x - 10.0,
    );

    if response.clicked() {
        let target = match item.kind {
            SideKind::Pc => PathBuf::new(),
            _ => PathBuf::from(item.label),
        };
        cx.actions.push(Action::Navigate(target));
    }
}

/// Drive name with a capacity bar underneath (blue normally, red when >= 90% full).
fn drive_item(ui: &mut Ui, cx: &mut Cx<'_>, drive: &DriveInfo) {
    let th = cx.theme;
    let (pal, m) = (&th.palette, &th.metrics);
    let (rect, response) =
        ui.allocate_exact_size(vec2(ui.available_width(), m.side_drive_h), Sense::click());
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
        drive.label,
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
    let color = if drive.nearly_full() {
        pal.danger
    } else {
        pal.accent
    };
    p.rect_filled(filled, 1.0, color);

    if response.clicked() {
        cx.actions
            .push(Action::Navigate(PathBuf::from(drive.label)));
    }
}
