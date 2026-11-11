//! Thumbnail grid (Ctrl+G): the same view, selection and actions as the list, drawn as tiles.
//! Uses the Thumb tier for images / video / PDF and painted icons for everything else.
//! Virtualised by rows of tiles, so a folder of 100k files costs the same as one of ten.

use eframe::egui::{Color32, Rect, ScrollArea, Stroke, StrokeKind, Ui, UiBuilder, pos2, vec2};
use quickfiles_core::{EntryRef, Kind, SortDir, SortKey, ViewState};

use super::{
    Action, Cx, dnd,
    icons::{self, Icon},
    widgets::{click_sense, drag_sense, galley, hit, text_left},
};

const CELL_W: f32 = 116.0;
const CELL_H: f32 = 118.0;
const PAD: f32 = 12.0;
/// Rows of tiles past the viewport whose thumbnails are decoded at low priority.
const PREFETCH_ROWS: usize = 2;

pub fn show(ui: &mut Ui, cx: &mut Cx<'_>, header: Rect, area: Rect) {
    let th = cx.theme;
    // The column headers do not exist in grid mode: the header row carries the sort chips.
    ui.painter().rect_filled(header, 0.0, th.palette.main);
    sort_bar(ui, cx, header);

    let view = cx.core.view().clone();
    let total = view.len();
    if total == 0 {
        let text = match view.state() {
            ViewState::Loading => "Searching…",
            _ => "No items",
        };
        ui.painter().text(
            pos2(area.center().x, area.top() + 40.0),
            eframe::egui::Align2::CENTER_CENTER,
            text,
            th.font.clone(),
            th.palette.text_dim,
        );
        view.want(0..0);
        return;
    }

    let avail = area.width() - PAD * 2.0;
    let cols = ((avail / CELL_W).floor() as usize).max(1);
    cx.state.grid_cols = cols;
    let rows = total.div_ceil(cols);
    let left = area.left() + PAD + (avail - cols as f32 * CELL_W).max(0.0) * 0.5;

    let mut scroll_ui = ui.new_child(UiBuilder::new().max_rect(area));
    let mut scroll = ScrollArea::vertical()
        .id_salt(("grid", view.id()))
        .auto_shrink([false, false]);
    if let Some(y) = cx.state.scroll_to.take() {
        scroll = scroll.vertical_scroll_offset(y);
    }
    if std::mem::take(&mut cx.state.scroll_to_focus) {
        if let Some(pos) = cx.state.focused.and_then(|k| view.find(k)) {
            let row = pos / cols;
            let (top, bottom) = (row as f32 * CELL_H, (row + 1) as f32 * CELL_H);
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

    let mut seen = 0..0;
    let out = scroll.show_rows(&mut scroll_ui, CELL_H, rows, |ui, range| {
        seen = range.clone();
        view.want(range.start * cols..(range.end * cols).min(total));
        for row in range {
            let (slot, _) = ui.allocate_exact_size(
                vec2(ui.available_width(), CELL_H),
                eframe::egui::Sense::hover(),
            );
            for c in 0..cols {
                let i = row * cols + c;
                if i >= total {
                    break;
                }
                let rect = Rect::from_min_size(
                    pos2(left + c as f32 * CELL_W, slot.top()),
                    vec2(CELL_W - 4.0, CELL_H - 4.0),
                );
                tile(ui, cx, &view, i, rect);
            }
        }
    });
    cx.state.list_scroll = out.state.clamped_offset().y;

    // Scroll gating, and a low-priority look at the rows ahead in the scroll direction.
    let ctx = ui.ctx().clone();
    let offset = cx.state.list_scroll;
    super::tex::track_scroll(cx, &ctx, view.id(), offset);
    let ahead =
        super::tex::rows_ahead(&seen, cx.state.scroll_gate.direction(), PREFETCH_ROWS, rows);
    cx.tex.prefetch_rows(
        cx.core.images(),
        &view,
        ahead.start * cols..(ahead.end * cols).min(total),
    );
}

fn tile(ui: &mut Ui, cx: &mut Cx<'_>, view: &quickfiles_core::View, i: usize, rect: Rect) {
    let th = cx.theme;
    let pal = &th.palette;
    let Some(entry) = view.get(i) else {
        return;
    };
    let response = hit(ui, rect, ("tile", i), drag_sense(ui));
    let key = entry.key;
    let selected = cx.state.selected.contains(&key);
    let focused = cx.state.focused == Some(key);

    let p = ui.painter().clone();
    if selected {
        p.rect_filled(rect, 8.0, pal.accent.gamma_multiply(0.28));
    } else if response.hovered() {
        p.rect_filled(rect, 8.0, pal.hover);
    }
    if focused {
        p.rect_stroke(rect, 8.0, Stroke::new(1.0, pal.accent), StrokeKind::Inside);
    }

    draw_thumb(
        ui,
        cx,
        &entry,
        Rect::from_min_size(rect.min + vec2(10.0, 6.0), vec2(rect.width() - 20.0, 72.0)),
    );

    let g = galley(&p, &entry.name, &th.font, pal.text, rect.width() - 10.0);
    let x = rect.center().x - g.size().x * 0.5;
    p.galley(
        pos2(x.max(rect.left() + 5.0), rect.top() + 86.0),
        g,
        pal.text,
    );
    if !entry.size_text.is_empty() {
        let s = galley(
            &p,
            &entry.size_text,
            &th.font,
            pal.text_dim,
            rect.width() - 10.0,
        );
        p.galley(
            pos2(rect.center().x - s.size().x * 0.5, rect.top() + 102.0),
            s,
            pal.text_dim,
        );
    }

    if response.drag_started() {
        if !cx.state.selected.contains(&key) {
            cx.state.select_only(key, i);
        }
        dnd::begin(cx);
    }
    if entry.kind == Kind::Folder {
        dnd::target(ui, cx, rect, &entry.path());
    }
    if response.clicked() {
        let (command, shift) = ui.input(|input| (input.modifiers.command, input.modifiers.shift));
        if shift {
            let anchor = cx.state.anchor.unwrap_or(i);
            let (lo, hi) = (anchor.min(i), anchor.max(i));
            cx.state.selected.clear();
            for j in lo..=hi {
                if let Some(e) = view.get(j) {
                    cx.state.selected.insert(e.key);
                }
            }
            cx.state.focused = Some(key);
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
            .unwrap_or(rect.center());
        cx.state.menu = Some(super::file_list::row_menu(cx, &entry, pos));
    }
}

/// Thumbnail fitted into `area`, or the painted icon while it loads / when there is none.
fn draw_thumb(ui: &mut Ui, cx: &mut Cx<'_>, entry: &EntryRef, area: Rect) {
    let th = cx.theme;
    let ctx = ui.ctx().clone();
    if let Some(t) = cx.tex.thumb_or_icon(&ctx, cx.core.images(), entry) {
        let max_zoom = if t.thumb { 4.0 } else { 2.0 };
        let k = (area.width() / t.size.x)
            .min(area.height() / t.size.y)
            .min(max_zoom);
        let r = Rect::from_center_size(area.center(), t.size * k);
        ui.painter().image(
            t.id,
            r,
            Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
            Color32::WHITE,
        );
        return;
    }
    icons::paint_scaled(
        ui.painter(),
        &th.palette,
        Icon::for_category(entry.category),
        area.center(),
        th.palette.text,
        3.0,
    );
}

/// "Sort by" chips in the header row; clicking the active one flips the direction.
fn sort_bar(ui: &mut Ui, cx: &mut Cx<'_>, header: Rect) {
    let th = cx.theme;
    let pal = &th.palette;
    let p = ui.painter().clone();
    let cy = header.center().y;
    let sort = cx.core.nav().query().sort;
    let mut x = header.left() + PAD + 4.0;
    text_left(&p, x, cy, "Sort by", &th.font, pal.text_dim, 60.0);
    x += 62.0;
    for (key, label) in [
        (SortKey::Name, "Name"),
        (SortKey::Type, "Type"),
        (SortKey::Modified, "Date Modified"),
        (SortKey::Size, "Size"),
    ] {
        let w = galley(&p, label, &th.font, pal.text, 200.0).size().x + 36.0;
        let rect = Rect::from_min_size(pos2(x, header.top() + 3.0), vec2(w, header.height() - 6.0));
        let response = hit(ui, rect, ("grid_sort", label), click_sense(ui));
        let active = sort.key == key;
        if active {
            p.rect_filled(rect, 6.0, pal.accent.gamma_multiply(0.2));
        } else if response.hovered() {
            p.rect_filled(rect, 6.0, pal.hover);
        }
        let color = if active || response.hovered() {
            pal.text
        } else {
            pal.text_dim
        };
        text_left(&p, x + 10.0, cy, label, &th.font, color, w - 34.0);
        if active {
            let arrow = match sort.dir {
                SortDir::Asc => Icon::ArrowUp,
                SortDir::Desc => Icon::ArrowDown,
            };
            icons::paint_scaled(
                &p,
                pal,
                arrow,
                pos2(rect.right() - 13.0, cy),
                pal.text_section,
                0.7,
            );
        }
        if response.clicked() {
            cx.core.toggle_sort(key);
        }
        x += w + 4.0;
    }
}
