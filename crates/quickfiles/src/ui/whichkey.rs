//! One overlay in the bottom-right corner, driven by the hotkey table (`hotkeys.rs`):
//! * `?` (Shift+/) lists every binding; Esc or `?` closes it;
//! * while a key sequence is half typed (which-key) the same panel switches to the chords that
//!   can come next, and goes back to the full list (or away) when the sequence ends.

use eframe::egui::{Align2, FontId, Painter, Rect, Stroke, StrokeKind, Ui, pos2, vec2};

use super::Cx;
use crate::theme::Palette;

const MARGIN: f32 = 10.0;
const PAD: f32 = 9.0;
const LINE_H: f32 = 21.0;
/// Horizontal padding inside a key chip, height of a chip, gap between chips.
const CHIP_PAD: f32 = 6.0;
const CHIP_H: f32 = 17.0;
const CHIP_GAP: f32 = 3.0;
/// Gap between the key chips and the description.
const GAP: f32 = 10.0;
/// Gap between the columns of the full list.
const COL_GAP: f32 = 26.0;

/// One line: the chords as separate chips, then a description.
struct Row {
    keys: Vec<String>,
    label: String,
    /// A prefix that opens another level: drawn dimmer.
    dim: bool,
}

pub fn overlay(ui: &mut Ui, cx: &mut Cx<'_>, full: Rect) {
    // The help panel is a reference card: any click, in the app or in another window, dismisses
    // it. (The which-key hints are cancelled by the key handling, not here.)
    if cx.state.help_open && cx.state.hints.is_none() {
        let dismiss = ui.input(|i| i.pointer.any_pressed() || !i.focused);
        if dismiss {
            cx.state.help_open = false;
            return;
        }
    }
    let (title, hint, rows) = if let Some(h) = &cx.state.hints {
        let rows: Vec<Row> = h
            .items
            .iter()
            .map(|i| Row {
                keys: vec![i.key.clone()],
                label: if i.group {
                    format!("+{}", i.label)
                } else {
                    i.label.clone()
                },
                dim: i.group,
            })
            .collect();
        (h.prefix.clone(), "Esc cancels".to_owned(), rows)
    } else if cx.state.help_open && !cx.state.help_rows.is_empty() {
        let rows: Vec<Row> = cx
            .state
            .help_rows
            .iter()
            .map(|r| Row {
                keys: r.keys.split(" / ").map(str::to_owned).collect(),
                label: r.label.clone(),
                dim: false,
            })
            .collect();
        ("Hotkeys".to_owned(), "? or Esc closes".to_owned(), rows)
    } else {
        return;
    };
    draw(ui, cx, full, &title, &hint, &rows);
}

fn text_w(p: &Painter, text: &str, font: &FontId) -> f32 {
    p.layout_no_wrap(text.to_owned(), font.clone(), eframe::egui::Color32::WHITE)
        .size()
        .x
}

fn chips_w(p: &Painter, keys: &[String], font: &FontId) -> f32 {
    keys.iter()
        .map(|k| text_w(p, k, font) + CHIP_PAD * 2.0)
        .sum::<f32>()
        + CHIP_GAP * keys.len().saturating_sub(1) as f32
}

fn draw_chips(p: &Painter, pal: &Palette, font: &FontId, mut x: f32, mid_y: f32, keys: &[String]) {
    for k in keys {
        let w = text_w(p, k, font) + CHIP_PAD * 2.0;
        let r = Rect::from_min_size(pos2(x, mid_y - CHIP_H / 2.0), vec2(w, CHIP_H));
        p.rect_filled(r, 4.0, pal.hover);
        p.text(
            r.center(),
            Align2::CENTER_CENTER,
            k,
            font.clone(),
            pal.accent,
        );
        x += w + CHIP_GAP;
    }
}

fn draw(ui: &mut Ui, cx: &mut Cx<'_>, full: Rect, title: &str, hint: &str, rows: &[Row]) {
    let pal = &cx.theme.palette;
    let font = cx.theme.font.clone();
    let key_font = FontId::proportional(font.size - 1.0);
    let p = ui.painter().clone();

    let keys_col = rows
        .iter()
        .map(|r| chips_w(&p, &r.keys, &key_font))
        .fold(0.0, f32::max);
    let label_col = rows
        .iter()
        .map(|r| text_w(&p, &r.label, &font))
        .fold(0.0, f32::max);
    let col_w = keys_col + GAP + label_col;

    // As many columns as fit across the window, as many rows as fit down it.
    let avail_w = full.width() - MARGIN * 2.0 - PAD * 2.0;
    let max_cols = (((avail_w + COL_GAP) / (col_w + COL_GAP)).floor() as usize).max(1);
    let avail_rows = (((full.height() - MARGIN * 2.0 - PAD * 2.0 - LINE_H - 40.0) / LINE_H).floor()
        as usize)
        .max(3);
    let cols = rows.len().div_ceil(avail_rows).clamp(1, max_cols);
    let per_col = rows.len().div_ceil(cols).min(avail_rows);
    let shown = (per_col * cols).min(rows.len());

    let hint = if shown < rows.len() {
        format!("+{} more in config.toml · {hint}", rows.len() - shown)
    } else {
        hint.to_owned()
    };
    let head_w = text_w(&p, title, &font) + COL_GAP + text_w(&p, &hint, &font);
    let body_w = col_w * cols as f32 + COL_GAP * (cols - 1) as f32;
    let w = (PAD * 2.0 + body_w.max(head_w)).min(full.width() - MARGIN * 2.0);
    let h = PAD * 2.0 + LINE_H * (per_col as f32 + 1.0) + 4.0;
    let rect = Rect::from_min_size(
        pos2(full.right() - MARGIN - w, full.bottom() - MARGIN - h),
        vec2(w, h),
    );

    p.rect_filled(rect, 8.0, pal.surface.gamma_multiply(0.97));
    p.rect_stroke(rect, 8.0, Stroke::new(1.0, pal.border), StrokeKind::Inside);

    let head_y = rect.top() + PAD + LINE_H / 2.0;
    p.text(
        pos2(rect.left() + PAD, head_y),
        Align2::LEFT_CENTER,
        title,
        font.clone(),
        pal.text,
    );
    p.text(
        pos2(rect.right() - PAD, head_y),
        Align2::RIGHT_CENTER,
        hint,
        font.clone(),
        pal.text_muted,
    );
    let sep_y = rect.top() + PAD + LINE_H + 1.0;
    p.hline(
        rect.left() + PAD..=rect.right() - PAD,
        sep_y,
        Stroke::new(1.0, pal.border),
    );

    for (i, row) in rows.iter().take(shown).enumerate() {
        let (c, r) = (i / per_col, i % per_col);
        let x = rect.left() + PAD + c as f32 * (col_w + COL_GAP);
        let mid_y = sep_y + 3.0 + LINE_H * (r as f32 + 0.5);
        draw_chips(&p, pal, &key_font, x, mid_y, &row.keys);
        p.text(
            pos2(x + keys_col + GAP, mid_y),
            Align2::LEFT_CENTER,
            &row.label,
            font.clone(),
            if row.dim { pal.text_muted } else { pal.text },
        );
    }
}
