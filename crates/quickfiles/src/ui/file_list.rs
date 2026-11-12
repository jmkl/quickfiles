//! Column header + virtualised, zebra-striped rows.
//!
//! Only the rows that are on screen are laid out (`ScrollArea::show_rows`), so a folder with
//! tens of thousands of entries costs the same per frame as one with ten.

use eframe::egui::{
    Align2, Color32, CursorIcon, Id, Key, Pos2, Rect, ScrollArea, Sense, Stroke, StrokeKind,
    TextEdit, Ui, UiBuilder, pos2, vec2,
};
use quickfiles_core::{EmptyReason, EntryRef, Kind, SortDir, SortKey, ViewState, count_text};

use super::{
    Action, Cx, MenuItem, MenuState, dnd,
    icons::{self, Icon},
    widgets::{drag_sense, hit, text_left, text_right, vline},
};
use crate::state::ColW;

/// Rows past the viewport whose icons / thumbnails are decoded at low priority.
const PREFETCH_ROWS: usize = 12;

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

    fn new(area: Rect, list_pad: f32, w: &ColW) -> Self {
        let left = area.left() + list_pad;
        let right = area.right() - list_pad;
        let avail = right - left;

        // Widths are user-adjustable (drag the column dividers); defaults come from `Metrics`.
        let (w_size, w_files, w_folders, w_date, w_type) =
            (w.size, w.files, w.folders, w.date, w.ty);

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
    if super::drives::is_drives(cx) {
        super::drives::show(ui, cx, header, rows);
        return;
    }
    if cx.state.grid {
        super::grid::show(ui, cx, header, rows);
        return;
    }
    let cols = Columns::new(rows, cx.theme.metrics.list_pad, &cx.state.col_w);
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
    let total = cx.core.view().len();
    let all_selected = total > 0 && cx.state.selected.len() >= total;
    let some_selected = !cx.state.selected.is_empty();
    if all_selected {
        p.rect_filled(check, 3.0, pal.accent);
        draw_check_mark(&p, check, pal.icon_active);
    } else {
        let edge = if check_response.hovered() {
            pal.text_secondary
        } else {
            pal.text_muted
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
            cx.state.clear_selection();
        } else {
            cx.actions.push(Action::SelectAll);
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
        let sort = cx.core.nav().query().sort;
        let active = sort.key == key;
        let color = if active || response.hovered() {
            pal.text
        } else {
            pal.text_muted
        };
        text_left(&p, text_x, cy, label, &th.font, color, x1 - text_x - 24.0);

        if active {
            let arrow = match sort.dir {
                SortDir::Asc => Icon::ArrowUp,
                SortDir::Desc => Icon::ArrowDown,
            };
            icons::paint_scaled(&p, pal, arrow, pos2(x1 - 19.5, cy), pal.text_secondary, 0.7);
        }
        if response.clicked() {
            cx.core.toggle_sort(key);
        }
    }

    // Static columns.
    if let Some(x) = cols.folders {
        text_left(&p, x + pad, cy, "Folders", &th.font, pal.text_muted, 60.0);
    }
    if let Some(x) = cols.files {
        text_left(&p, x + pad, cy, "Files", &th.font, pal.text_muted, 50.0);
    }

    // Drag a divider to resize the column on its right; double-click resets it. The columns
    // are anchored to the right edge, so dragging left grows the column.
    let defaults = ColW::from_metrics(&cx.theme.metrics);
    let edges: [(Option<f32>, &str); 5] = [
        (cols.ty, "ty"),
        (cols.date, "date"),
        (cols.folders, "folders"),
        (cols.files, "files"),
        (Some(cols.size), "size"),
    ];
    for (edge, name) in edges {
        let Some(x) = edge else { continue };
        let grip = Rect::from_min_max(pos2(x - 3.0, r.top()), pos2(x + 3.0, r.bottom()));
        let response = hit(ui, grip, ("colresize", name), Sense::click_and_drag());
        if response.hovered() || response.dragged() {
            ui.ctx().set_cursor_icon(CursorIcon::ResizeColumn);
        }
        let w = &mut cx.state.col_w;
        let (field, default) = match name {
            "ty" => (&mut w.ty, defaults.ty),
            "date" => (&mut w.date, defaults.date),
            "folders" => (&mut w.folders, defaults.folders),
            "files" => (&mut w.files, defaults.files),
            _ => (&mut w.size, defaults.size),
        };
        if response.double_clicked() {
            *field = default;
        } else if response.dragged() {
            *field = (*field - response.drag_delta().x).clamp(ColW::MIN, ColW::MAX);
        }
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
    let view = cx.core.view().clone();
    let total = view.len();

    if total == 0 {
        let text = match view.state() {
            ViewState::Loading => "Searching…".to_owned(),
            ViewState::Ready => "No items".to_owned(),
            ViewState::Error(e) => format!("Search failed: {e}"),
            ViewState::Empty(EmptyReason::NoQuery) => {
                "Type to search everything · or pick a place on the left".to_owned()
            }
            ViewState::Empty(EmptyReason::NoResults) => {
                if view.query().is_browse() {
                    "This folder is empty".to_owned()
                } else {
                    "No results".to_owned()
                }
            }
            ViewState::Empty(EmptyReason::OnlyHidden) => {
                "Only hidden items here · Ctrl+H shows them".to_owned()
            }
            ViewState::Empty(EmptyReason::NotIndexed) => {
                format!(
                    "Everything has not indexed this folder (check its Indexes settings)\n{}",
                    cx.core
                        .nav()
                        .location()
                        .map(|p| p.display().to_string())
                        .unwrap_or_default()
                )
            }
        };
        message(ui, area, &text, cx);
        view.want(0..0);
        return;
    }

    let pitch = th.metrics.row_pitch;
    let mut scroll_ui = ui.new_child(UiBuilder::new().max_rect(area));
    let mut scroll = ScrollArea::vertical()
        // New id per view: each query/navigation starts scrolled to the top.
        .id_salt(("file_list", view.id()))
        .auto_shrink([false, false]);

    if let Some(y) = cx.state.scroll_to.take() {
        scroll = scroll.vertical_scroll_offset(y);
    }

    // Keyboard navigation: scroll just enough to bring the active row into view.
    if std::mem::take(&mut cx.state.scroll_to_focus) {
        if let Some(pos) = cx.state.focused.and_then(|k| view.find(k)) {
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

    let search_mode = !view.query().is_browse() || cx.core.nav().is_search_mode();
    let mut seen = 0..0;
    let output = scroll.show_rows(&mut scroll_ui, pitch, total, |ui, range| {
        seen = range.clone();
        // Tell the engine what is on screen; it fetches missing pages and folder counts.
        view.want(range.clone());
        for i in range {
            row(ui, cx, &view, cols, i, search_mode);
        }
    });
    cx.state.list_scroll = output.state.clamped_offset().y;

    // Scroll gating, and a low-priority look at the rows ahead in the scroll direction.
    let ctx = ui.ctx().clone();
    let offset = cx.state.list_scroll;
    super::tex::track_scroll(cx, &ctx, view.id(), offset);
    let ahead = super::tex::rows_ahead(
        &seen,
        cx.state.scroll_gate.direction(),
        PREFETCH_ROWS,
        total,
    );
    cx.tex.prefetch_rows(cx.core.images(), &view, ahead);
}

fn message(ui: &Ui, area: Rect, text: &str, cx: &Cx<'_>) {
    ui.painter().text(
        pos2(area.center().x, area.top() + 40.0),
        Align2::CENTER_CENTER,
        text,
        cx.theme.font.clone(),
        cx.theme.palette.text_muted,
    );
}

fn row(
    ui: &mut Ui,
    cx: &mut Cx<'_>,
    view: &quickfiles_core::View,
    cols: &Columns,
    i: usize,
    search_mode: bool,
) {
    let th = cx.theme;
    let (pal, m) = (&th.palette, &th.metrics);

    let (slot, response) =
        ui.allocate_exact_size(vec2(ui.available_width(), m.row_pitch), drag_sense(ui));
    let row_rect = Rect::from_min_max(
        pos2(cols.left, slot.top()),
        pos2(cols.right, slot.top() + m.row_h),
    );
    let cy = row_rect.top() + 13.0;

    let Some(entry) = view.get(i) else {
        // Page still loading: keep the zebra rhythm, draw nothing else.
        if i % 2 == 1 {
            ui.painter().rect_filled(row_rect, m.radius, pal.row_alt);
        }
        return;
    };
    let key = entry.key;
    let selected = cx.state.selected.contains(&key);
    let focused = cx.state.focused == Some(key);
    let renaming = cx.state.rename.as_ref().is_some_and(|r| r.key == key);
    let pad = m.col_text_pad;

    {
        let p = ui.painter();

        // Background: selection > hover > zebra.
        let bg = if selected {
            Some(pal.accent.gamma_multiply(0.28))
        } else if response.hovered() {
            Some(pal.hover)
        } else if i % 2 == 1 {
            Some(pal.row_alt)
        } else {
            None
        };
        if let Some(color) = bg {
            p.rect_filled(row_rect, m.radius, color);
        }
        if focused {
            p.rect_stroke(
                row_rect,
                m.radius,
                Stroke::new(1.0, pal.accent),
                StrokeKind::Inside,
            );
        }
    }

    draw_icon(ui, cx, &entry, pos2(cols.left + 14.5, cy));

    let name_x = cols.left + 31.0;
    let name_w = cols.name_right - name_x - 12.0;
    if renaming {
        rename_box(
            ui,
            cx,
            Rect::from_min_size(
                pos2(name_x - 4.0, row_rect.top() + 1.0),
                vec2(name_w + 4.0, m.row_h - 2.0),
            ),
        );
    } else {
        let p = ui.painter();
        let w = text_left(p, name_x, cy, &entry.name, &th.font, pal.text, name_w);
        // Search results come from everywhere: show where each one lives.
        if search_mode {
            let x = name_x + w + 12.0;
            let room = cols.name_right - x - 12.0;
            if room > 60.0 {
                text_left(
                    p,
                    x,
                    cy,
                    &entry.parent,
                    &th.font,
                    pal.text_muted.gamma_multiply(0.7),
                    room,
                );
            }
        }
    }

    {
        let p = ui.painter();
        if let (Some(ty), Some(date)) = (cols.ty, cols.date) {
            let type_x = ty + pad;
            text_left(
                p,
                type_x,
                cy,
                &entry.type_label,
                &th.font,
                pal.text_muted,
                date - type_x - 10.0,
            );
        }
        if let Some(date) = cols.date {
            let date_x = date + pad;
            text_left(
                p,
                date_x,
                cy,
                &entry.modified_text,
                &th.font,
                pal.text_muted,
                cols.date_right() - date_x - 6.0,
            );
        }
        if cols.folders.is_some() || cols.files.is_some() {
            let counts = (entry.kind == Kind::Folder)
                .then(|| view.counts(key))
                .flatten();
            if let Some(x) = cols.folders {
                let t = if entry.kind == Kind::Folder {
                    count_text(counts.map(|c| c.0))
                } else {
                    "".into()
                };
                text_left(p, x + pad, cy, &t, &th.font, pal.text_muted, 50.0);
            }
            if let Some(x) = cols.files {
                let t = if entry.kind == Kind::Folder {
                    count_text(counts.map(|c| c.1))
                } else {
                    "".into()
                };
                text_left(p, x + pad, cy, &t, &th.font, pal.text_muted, 50.0);
            }
        }
        text_right(
            p,
            cols.right - 19.0,
            cy,
            &entry.size_text,
            &th.font,
            pal.text_muted,
            70.0,
        );
    }

    if renaming {
        return;
    }
    if response.drag_started() {
        if !cx.state.selected.contains(&key) {
            cx.state.select_only(key, i);
        }
        dnd::begin(cx);
    }
    if entry.kind == Kind::Folder {
        dnd::target(ui, cx, row_rect, &entry.path());
    }
    if response.clicked() {
        let (command, shift) = ui.input(|input| (input.modifiers.command, input.modifiers.shift));
        if shift {
            range_select(cx, view, i);
        } else if command {
            if !cx.state.selected.remove(&key) {
                cx.state.selected.insert(key);
            }
            cx.state.focused = Some(key);
            cx.state.anchor = Some(i);
        } else {
            cx.state.select_only(key, i);
        }
    }
    if response.double_clicked() {
        cx.actions.push(Action::OpenPath {
            path: entry.path(),
            is_dir: entry.is_dir(),
        });
    }
    if response.secondary_clicked() {
        if !selected {
            cx.state.select_only(key, i);
        }
        let pos = ui
            .input(|input| input.pointer.interact_pos())
            .unwrap_or(row_rect.center());
        cx.state.menu = Some(row_menu(cx, &entry, pos));
    }
}

fn range_select(cx: &mut Cx<'_>, view: &quickfiles_core::View, i: usize) {
    let anchor = cx.state.anchor.unwrap_or(i);
    let (lo, hi) = (anchor.min(i), anchor.max(i));
    cx.state.selected.clear();
    for j in lo..=hi {
        if let Some(e) = view.get(j) {
            cx.state.selected.insert(e.key);
        }
    }
    if let Some(e) = view.get(i) {
        cx.state.focused = Some(e.key);
    }
}

pub(super) fn row_menu(cx: &Cx<'_>, entry: &EntryRef, pos: Pos2) -> MenuState {
    let drive = entry.kind == Kind::Drive;
    let open_label = if entry.is_dir() {
        "Open"
    } else {
        "Open with default app"
    };
    let mut items = vec![
        MenuItem::new(
            open_label,
            Action::OpenPath {
                path: entry.path(),
                is_dir: entry.is_dir(),
            },
        ),
        MenuItem::new("Show in Explorer", Action::Reveal),
        MenuItem::separator(),
        MenuItem::new("Copy", Action::Copy).disabled(drive),
        MenuItem::new("Cut", Action::Cut).disabled(drive),
        MenuItem::new("Paste", Action::Paste).disabled(!cx.core.can_paste()),
        MenuItem::separator(),
        MenuItem::new("Rename", Action::Rename).disabled(drive),
        MenuItem::new("Move to Recycle Bin", Action::Delete { permanent: false }).disabled(drive),
        MenuItem::new("Delete permanently", Action::Delete { permanent: true }).disabled(drive),
    ];
    if entry.is_dir() {
        items.insert(
            2,
            MenuItem::new(
                if cx.core.is_bookmarked(&entry.path()) {
                    "Remove bookmark"
                } else {
                    "Bookmark"
                },
                Action::ToggleBookmark,
            ),
        );
    }
    MenuState::at(pos, items)
}

/// Thumbnail for media, the shell icon for executables and shortcuts, else a painted icon.
fn draw_icon(ui: &mut Ui, cx: &mut Cx<'_>, entry: &EntryRef, c: Pos2) {
    let th = cx.theme;
    let ctx = ui.ctx().clone();
    let size = 18.0;
    let rect = Rect::from_center_size(c, vec2(size, size));

    if let Some(t) = cx.tex.thumb_or_icon(&ctx, cx.core.images(), entry) {
        // Fit inside the square, keep the aspect ratio.
        let k = (size / t.size.x).min(size / t.size.y);
        let r = Rect::from_center_size(c, t.size * k);
        ui.painter().image(
            t.id,
            r,
            Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
            Color32::WHITE,
        );
        return;
    }
    super::fileicon::paint(ui.painter(), &th.palette, entry, c, 17.0);
}

/// Inline rename editor. Enter commits, Esc cancels, losing focus commits.
pub(super) fn rename_box(ui: &mut Ui, cx: &mut Cx<'_>, rect: Rect) {
    let th = cx.theme;
    let pal = &th.palette;
    let id = Id::new("rename_edit");
    let Some(r) = cx.state.rename.as_mut() else {
        return;
    };
    ui.painter().rect_filled(rect, 3.0, pal.background);
    ui.painter()
        .rect_stroke(rect, 3.0, Stroke::new(1.0, pal.accent), StrokeKind::Inside);
    let mut edit_ui = ui.new_child(UiBuilder::new().max_rect(rect.shrink2(vec2(4.0, 1.0))));
    let output = TextEdit::singleline(&mut r.text)
        .id(id)
        .frame(eframe::egui::Frame::NONE)
        .desired_width(rect.width() - 8.0)
        .show(&mut edit_ui);
    let response = output.response;
    if r.fresh {
        r.fresh = false;
        response.request_focus();
        // Select the stem, not the extension.
        let stem = r
            .text
            .rfind('.')
            .filter(|&d| d > 0)
            .map_or(r.text.chars().count(), |d| r.text[..d].chars().count());
        let mut state = output.state;
        state
            .cursor
            .set_char_range(Some(eframe::egui::text::CCursorRange::two(
                eframe::egui::text::CCursor::new(0),
                eframe::egui::text::CCursor::new(stem),
            )));
        state.store(ui.ctx(), id);
        return;
    }
    let (enter, escape) = ui.input(|i| (i.key_pressed(Key::Enter), i.key_pressed(Key::Escape)));
    if escape {
        cx.state.rename = None;
    } else if enter || response.lost_focus() {
        let (path, text) = (r.path.clone(), r.text.clone());
        cx.state.rename = None;
        if text
            != path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default()
        {
            match cx.core.rename(&path, &text) {
                Ok(_) => cx.state.say("Renamed"),
                Err(e) => cx.state.say(format!("Rename failed: {e}")),
            }
        }
    }
}
