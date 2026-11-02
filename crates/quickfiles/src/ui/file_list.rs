//! Column header + virtualised, zebra-striped rows.
//!
//! Only the rows that are on screen are laid out (`ScrollArea::show_rows`), so a folder with
//! tens of thousands of entries costs the same per frame as one with ten.

use eframe::egui::{
    Align2, Pos2, Rect, ScrollArea, Sense, Stroke, StrokeKind, Ui, UiBuilder, pos2, vec2,
};

use super::{
    Action, Cx,
    icons::{self, Icon},
    widgets::{hit, text_left, text_right, vline},
};
use crate::{
    model::{Listing, SortDir, SortKey, count_text, format_size},
    theme::Metrics,
};

/// Left edge of every column, in absolute coordinates. Name is flexible; the others are
/// anchored to the right edge. When the list gets narrow, columns are dropped (Folders and
/// Files first, then Type, then Date) so the name always keeps room.
struct Columns {
    left: f32,
    right: f32,
    /// Right edge of the Name column.
    name_right: f32,
    ty: Option<f32>,
    date: Option<f32>,
    folders: Option<f32>,
    files: Option<f32>,
    size: f32,
}

impl Columns {
    const NAME_MIN: f32 = 200.0;

    fn new(area: Rect, m: &Metrics) -> Self {
        let left = area.left() + m.list_pad;
        let right = area.right() - m.list_pad;
        let avail = right - left;

        // Widths derived from the measured offsets in `Metrics`.
        let w_size = m.col_size;
        let w_files = m.col_files - m.col_size;
        let w_folders = m.col_folders - m.col_files;
        let w_date = m.col_date - m.col_folders;
        let w_type = m.col_type - m.col_date;

        let show_counts = avail >= Self::NAME_MIN + w_size + w_files + w_folders + w_date + w_type;
        let show_type = avail >= Self::NAME_MIN + w_size + w_date + w_type;
        let show_date = avail >= Self::NAME_MIN + w_size + w_date;

        let size = right - w_size;
        let mut x = size;
        let mut take = |show: bool, w: f32| {
            show.then(|| {
                x -= w;
                x
            })
        };
        let files = take(show_counts, w_files);
        let folders = take(show_counts, w_folders);
        let date = take(show_date, w_date);
        let ty = take(show_type, w_type);

        Self {
            left,
            right,
            name_right: x,
            ty,
            date,
            folders,
            files,
            size,
        }
    }

    /// Right edge of the Date column.
    fn date_right(&self) -> f32 {
        self.folders.unwrap_or(self.size)
    }
}

pub fn show(ui: &mut Ui, cx: &mut Cx<'_>, header: Rect, rows: Rect) {
    let cols = Columns::new(rows, &cx.theme.metrics);
    draw_header(ui, cx, header, &cols);
    draw_rows(ui, cx, rows, &cols);
}

fn draw_header(ui: &mut Ui, cx: &mut Cx<'_>, r: Rect, cols: &Columns) {
    let th = cx.theme;
    let (pal, m) = (&th.palette, &th.metrics);
    let pad = m.col_text_pad;
    let p = ui.painter().clone();
    let cy = r.top() + 13.5;

    for x in [cols.ty, cols.date, cols.folders]
        .into_iter()
        .flatten()
        .chain([cols.size])
    {
        vline(&p, x, r.top() + 4.0, r.top() + 23.0, pal.border);
    }

    // Select-all checkbox, aligned with the row icons below it.
    let check = Rect::from_center_size(pos2(cols.left + 14.5, cy), vec2(16.0, 16.0));
    let check_response = hit(ui, check.expand(3.0), "select_all", Sense::click());
    let all_selected =
        !cx.state.view.is_empty() && cx.state.view.iter().all(|i| cx.state.selected.contains(i));
    let some_selected = !cx.state.selected.is_empty();
    if all_selected {
        p.rect_filled(check, 3.0, pal.accent);
        draw_check_mark(&p, check, pal.icon_strong);
    } else {
        let edge = if check_response.hovered() {
            pal.text_section
        } else {
            pal.text_dim
        };
        p.rect_stroke(check, 3.0, Stroke::new(1.0, edge), StrokeKind::Inside);
        if some_selected {
            p.line_segment(
                [
                    pos2(check.left() + 4.0, check.center().y),
                    pos2(check.right() - 4.0, check.center().y),
                ],
                Stroke::new(1.5, pal.accent),
            );
        }
    }
    if check_response.clicked() {
        if all_selected {
            cx.state.selected.clear();
        } else {
            cx.state.selected.extend(cx.state.view.iter().copied());
        }
    }

    // Sortable columns: (key, label, hit-left, hit-right, text-x)
    let mut sortable = vec![(
        SortKey::Name,
        "Name",
        cols.left + 26.0,
        cols.name_right,
        cols.left + 31.0,
    )];
    if let (Some(ty), Some(date)) = (cols.ty, cols.date) {
        sortable.push((SortKey::Type, "Type", ty, date, ty + pad));
    }
    if let Some(date) = cols.date {
        sortable.push((
            SortKey::Modified,
            "Date Modified",
            date,
            cols.date_right(),
            date + pad,
        ));
    }
    sortable.push((
        SortKey::Size,
        "Size",
        cols.size,
        cols.right,
        cols.size + pad - 1.0,
    ));
    for (key, label, x0, x1, text_x) in sortable {
        let rect = Rect::from_min_max(pos2(x0, r.top()), pos2(x1, r.bottom()));
        let response = hit(ui, rect, ("column", label), Sense::click());
        let active = cx.state.sort.key == key;
        let color = if active || response.hovered() {
            pal.text
        } else {
            pal.text_dim
        };
        text_left(&p, text_x, cy, label, &th.font, color, x1 - text_x - 24.0);

        if active {
            let arrow = match cx.state.sort.dir {
                SortDir::Asc => Icon::ArrowUp,
                SortDir::Desc => Icon::ArrowDown,
            };
            icons::paint_scaled(&p, pal, arrow, pos2(x1 - 19.5, cy), pal.text_section, 0.7);
        }
        if response.clicked() {
            cx.state.toggle_sort(key);
        }
    }

    // Static columns.
    if let Some(x) = cols.folders {
        text_left(&p, x + pad, cy, "Folders", &th.font, pal.text_dim, 60.0);
    }
    if let Some(x) = cols.files {
        text_left(&p, x + pad, cy, "Files", &th.font, pal.text_dim, 50.0);
    }

    // Filter affordance in the Type column (decorative in this prototype).
    if let Some(date) = cols.date.filter(|_| cols.ty.is_some()) {
        icons::paint_scaled(
            &p,
            pal,
            Icon::Filter,
            pos2(date - 22.5, cy),
            pal.text_section,
            0.8,
        );
    }
}

fn draw_check_mark(p: &eframe::egui::Painter, rect: Rect, color: eframe::egui::Color32) {
    let c: Pos2 = rect.center();
    let st = Stroke::new(1.5, color);
    p.line_segment([c + vec2(-3.5, 0.0), c + vec2(-1.0, 2.5)], st);
    p.line_segment([c + vec2(-1.0, 2.5), c + vec2(3.5, -2.5)], st);
}

fn draw_rows(ui: &mut Ui, cx: &mut Cx<'_>, area: Rect, cols: &Columns) {
    let th = cx.theme;

    let Some(listing) = cx.state.listing.clone() else {
        message(ui, area, "Loading…", cx);
        return;
    };
    let total = cx.state.view.len();
    if total == 0 {
        message(ui, area, "No items", cx);
        return;
    }

    let pitch = th.metrics.row_pitch;
    let mut scroll_ui = ui.new_child(UiBuilder::new().max_rect(area));
    let mut scroll = ScrollArea::vertical()
        // New id per request: each navigation starts scrolled to the top.
        .id_salt(("file_list", cx.state.request_id))
        .auto_shrink([false, false]);

    // Keyboard navigation: scroll just enough to bring the active row into view.
    if std::mem::take(&mut cx.state.scroll_to_focus) {
        let pos = cx
            .state
            .focused
            .and_then(|f| cx.state.view.iter().position(|&i| i == f));
        if let Some(pos) = pos {
            let (top, bottom) = (pos as f32 * pitch, (pos + 1) as f32 * pitch);
            let offset = cx.state.list_scroll;
            let target = if top < offset {
                top
            } else if bottom > offset + area.height() {
                bottom - area.height()
            } else {
                offset
            };
            scroll = scroll.vertical_scroll_offset(target);
        }
    }

    let output = scroll.show_rows(&mut scroll_ui, pitch, total, |ui, range| {
        for i in range {
            row(ui, cx, &listing, cols, i);
        }
    });
    cx.state.list_scroll = output.state.clamped_offset().y;
}

fn message(ui: &Ui, area: Rect, text: &str, cx: &Cx<'_>) {
    ui.painter().text(
        pos2(area.center().x, area.top() + 40.0),
        Align2::CENTER_CENTER,
        text,
        cx.theme.font.clone(),
        cx.theme.palette.text_dim,
    );
}

fn row(ui: &mut Ui, cx: &mut Cx<'_>, listing: &Listing, cols: &Columns, i: usize) {
    let th = cx.theme;
    let (pal, m) = (&th.palette, &th.metrics);
    let idx = cx.state.view[i];
    let entry = &listing.entries[idx as usize];

    let (slot, response) =
        ui.allocate_exact_size(vec2(ui.available_width(), m.row_pitch), Sense::click());
    let row = Rect::from_min_max(
        pos2(cols.left, slot.top()),
        pos2(cols.right, slot.top() + m.row_h),
    );
    let cy = row.top() + 13.0;
    let pad = m.col_text_pad;
    let selected = cx.state.selected.contains(&idx);
    let focused = cx.state.focused == Some(idx);

    {
        let p = ui.painter();

        // Background: selection > hover > zebra.
        let bg = if selected {
            Some(pal.accent.gamma_multiply(0.28))
        } else if response.hovered() {
            Some(pal.hover)
        } else if i % 2 == 1 {
            Some(pal.stripe)
        } else {
            None
        };
        if let Some(color) = bg {
            p.rect_filled(row, m.radius, color);
        }
        if focused {
            p.rect_stroke(
                row,
                m.radius,
                Stroke::new(1.0, pal.accent),
                StrokeKind::Inside,
            );
        }

        let icon = Icon::for_kind(entry.kind);
        icons::paint(p, pal, icon, pos2(cols.left + 14.5, cy), pal.text);

        let name_x = cols.left + 31.0;
        text_left(
            p,
            name_x,
            cy,
            &entry.name,
            &th.font,
            pal.text,
            cols.name_right - name_x - 12.0,
        );

        if let (Some(ty), Some(date)) = (cols.ty, cols.date) {
            let type_x = ty + pad;
            text_left(
                p,
                type_x,
                cy,
                entry.type_label,
                &th.font,
                pal.text_dim,
                date - type_x - 10.0,
            );
        }

        if let Some(date) = cols.date {
            let date_x = date + pad;
            text_left(
                p,
                date_x,
                cy,
                &entry.modified,
                &th.font,
                pal.text_dim,
                cols.date_right() - date_x - 6.0,
            );
        }

        if let Some(x) = cols.folders {
            text_left(
                p,
                x + pad,
                cy,
                &count_text(entry.folders),
                &th.font,
                pal.text_dim,
                50.0,
            );
        }
        if let Some(x) = cols.files {
            text_left(
                p,
                x + pad,
                cy,
                &count_text(entry.files),
                &th.font,
                pal.text_dim,
                50.0,
            );
        }
        text_right(
            p,
            cols.right - 19.0,
            cy,
            &format_size(entry.size_bytes),
            &th.font,
            pal.text_dim,
            70.0,
        );
    }

    if response.clicked() {
        let additive = ui.input(|input| input.modifiers.command);
        if additive {
            if !cx.state.selected.remove(&idx) {
                cx.state.selected.insert(idx);
            }
        } else {
            cx.state.selected.clear();
            cx.state.selected.insert(idx);
        }
        cx.state.focused = Some(idx);
    }
    if response.double_clicked() && entry.is_dir() {
        cx.actions.push(Action::OpenFolder(entry.name.clone()));
    }
}
