//! Right-hand preview panel. Shown when the active item is a selected file: a picture for
//! images, video, PDF and documents (decoded off-thread), the first lines for text files.
//!
//! Smooth arrowing through large images (`PLAN-PERF.md` phase 2): the big picture is requested
//! only after the selection has been steady for [`PREVIEW_DEBOUNCE_MS`], decoded at about twice
//! the panel size, and the cached thumbnail is shown scaled up until it arrives. Once it is
//! there, the previous and next image are decoded at low priority.

use std::io::Read;

use std::time::{Duration, Instant};

use eframe::egui::{Align2, Color32, Rect, Stroke, StrokeKind, Ui, pos2, vec2};
use quickfiles_core::{Category, EntryRef, ImageKey, format_size, is_text_like, preview_px};

use super::{
    Action, Cx,
    icons::Icon,
    tex::Want,
    widgets::{galley, hline, icon_button, text_left, vline},
};

const TEXT_BYTES: u64 = 16 * 1024;
const TEXT_LINES: usize = 400;

pub fn show(ui: &mut Ui, cx: &mut Cx<'_>, r: Rect) {
    let th = cx.theme;
    let (pal, m) = (&th.palette, &th.metrics);
    let p = ui.painter().clone();

    p.rect_filled(r, 0.0, pal.panel);
    vline(&p, r.left(), r.top(), r.bottom(), pal.border);
    let left = r.left() + 1.0; // inside the divider

    // Header
    let head_h = m.toolbar_h;
    text_left(
        &p,
        left + 16.0,
        r.top() + 15.5,
        "Preview",
        &th.font,
        pal.text_secondary,
        r.width() - 70.0,
    );
    if icon_button(
        ui,
        th,
        Icon::Close,
        pos2(r.right() - 22.0, r.top() + 15.5),
        vec2(24.0, 24.0),
        "preview_close",
        true,
    )
    .clicked()
    {
        cx.actions.push(Action::TogglePreview);
    }
    hline(&p, r.top() + head_h - 1.0, left, r.right(), pal.border);

    // Nothing to show unless the active item is a selected file.
    let view = cx.core.view().clone();
    let active: Option<EntryRef> = cx
        .state
        .focused
        .filter(|k| cx.state.selected.contains(k))
        .and_then(|k| view.find(k))
        .and_then(|i| view.get(i))
        .filter(|e| e.kind == quickfiles_core::Kind::File);
    let Some(entry) = active else {
        p.text(
            pos2((left + r.right()) * 0.5, r.top() + head_h + 60.0),
            Align2::CENTER_CENTER,
            "Select a file to preview",
            th.font.clone(),
            pal.text_muted,
        );
        return;
    };

    // Big icon card
    let mid_x = (left + r.right()) * 0.5;
    let card = Rect::from_center_size(pos2(mid_x, r.top() + head_h + 62.0), vec2(96.0, 96.0));
    p.rect_filled(card, 10.0, pal.background);
    p.rect_stroke(card, 10.0, Stroke::new(1.0, pal.border), StrokeKind::Inside);
    let ctx = ui.ctx().clone();
    let thumb = cx.tex.entry(&ctx, cx.core.images(), &entry, Want::Thumb);
    match &thumb {
        Some(t) => {
            let k = ((card.width() - 8.0) / t.size.x).min((card.height() - 8.0) / t.size.y);
            let r = Rect::from_center_size(card.center(), t.size * k);
            p.image(
                t.id,
                r,
                Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
                Color32::WHITE,
            );
        }
        None => super::fileicon::paint(&p, pal, &entry, card.center(), 48.0),
    }

    // Name + type, centred
    let max_w = r.width() - 32.0;
    let name_y = card.bottom() + 20.0;
    let g = galley(&p, &entry.name, &th.font, pal.text, max_w);
    p.galley(
        pos2(mid_x - g.size().x * 0.5, name_y - g.size().y * 0.5),
        g,
        pal.text,
    );
    let g = galley(&p, &entry.type_label, &th.font, pal.text_muted, max_w);
    p.galley(
        pos2(mid_x - g.size().x * 0.5, name_y + 20.0 - g.size().y * 0.5),
        g,
        pal.text_muted,
    );

    // Details
    let mut y = name_y + 52.0;
    hline(&p, y - 12.0, left + 16.0, r.right() - 16.0, pal.border);
    for (label, value) in [
        ("Size", format_size(entry.size)),
        ("Modified", entry.modified_text.to_string()),
        ("Location", entry.parent.to_string()),
    ] {
        text_left(&p, left + 16.0, y, label, &th.font, pal.text_muted, 70.0);
        text_left(
            &p,
            left + 90.0,
            y,
            &value,
            &th.font,
            pal.text,
            r.right() - left - 90.0 - 16.0,
        );
        y += 24.0;
    }

    // Content preview
    y += 4.0;
    text_left(
        &p,
        left + 16.0,
        y,
        "Content",
        &th.font,
        pal.text_secondary,
        120.0,
    );
    let body = Rect::from_min_max(
        pos2(left + 16.0, y + 14.0),
        pos2(r.right() - 16.0, r.bottom() - 16.0),
    );
    if body.height() < 40.0 {
        return;
    }
    p.rect_filled(body, m.radius, pal.background);
    p.rect_stroke(
        body,
        m.radius,
        Stroke::new(1.0, pal.border),
        StrokeKind::Inside,
    );

    // Pictures first (image preview, or the shell thumbnail for video / pdf / documents).
    let px = preview_px(body.width().max(body.height()) * ctx.pixels_per_point());
    if ImageKey::quick_preview(&entry, px).is_some() {
        let steady = cx.state.preview_debounce.poll(&entry.key, Instant::now());
        let full = match steady {
            Ok(()) => cx
                .tex
                .entry(&ctx, cx.core.images(), &entry, Want::Preview(px)),
            Err(wait) => {
                // Still moving: decode nothing, come back when it would be steady.
                ctx.request_repaint_after(wait + Duration::from_millis(5));
                None
            }
        };
        let avail = body.shrink(6.0);
        let draw = |id, size: eframe::egui::Vec2, max_zoom: f32| {
            let k = (avail.width() / size.x)
                .min(avail.height() / size.y)
                .min(max_zoom);
            p.image(
                id,
                Rect::from_center_size(body.center(), size * k),
                Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
                Color32::WHITE,
            );
        };
        match (&full, &thumb) {
            (Some(t), _) => {
                draw(t.id, t.size, 4.0);
                // Idle: warm up the neighbours so the next arrow press is instant.
                let view = cx.core.view().clone();
                if let Some(at) = view.find(entry.key) {
                    for i in at.checked_sub(1).into_iter().chain([at + 1]) {
                        if let Some(n) = view.get(i) {
                            cx.tex.prefetch(cx.core.images(), &n, Want::Preview(px));
                        }
                    }
                }
            }
            // The thumbnail, scaled up, until the real picture arrives.
            (None, Some(t)) if t.thumb => {
                draw(t.id, t.size, 8.0);
                p.text(
                    body.right_bottom() - vec2(8.0, 8.0),
                    Align2::RIGHT_BOTTOM,
                    "Loading…",
                    th.font.clone(),
                    pal.text_muted,
                );
            }
            _ => {
                p.text(
                    body.center(),
                    Align2::CENTER_CENTER,
                    "Loading preview…",
                    th.font.clone(),
                    pal.text_muted,
                );
            }
        }
        return;
    }

    let ext = entry.ext().unwrap_or_default();
    let lines = if is_text_like(entry.category, &ext) && entry.category != Category::Folder {
        text_lines(cx, &entry)
    } else {
        Vec::new()
    };
    if lines.is_empty() {
        p.text(
            body.center(),
            Align2::CENTER_CENTER,
            "No preview available",
            th.font.clone(),
            pal.text_muted,
        );
        return;
    }

    let line_h = 17.0;
    let gutter = 30.0;
    let max_rows = ((body.height() - 16.0) / line_h).floor().max(0.0) as usize;
    let text_w = body.width() - gutter - 16.0;
    for (i, line) in lines.iter().take(max_rows).enumerate() {
        let cy = body.top() + 8.0 + line_h * (i as f32 + 0.5);
        text_left(
            &p,
            body.left() + 8.0,
            cy,
            &(i + 1).to_string(),
            &th.mono,
            pal.icon_disabled,
            gutter - 10.0,
        );
        text_left(
            &p,
            body.left() + gutter,
            cy,
            line,
            &th.mono,
            pal.text_secondary,
            text_w,
        );
    }
}

/// First lines of a text file, read once per selected file (16 KB at most).
fn text_lines(cx: &mut Cx<'_>, entry: &EntryRef) -> Vec<String> {
    if let Some((key, lines)) = &cx.state.text_preview {
        if *key == entry.key {
            return lines.clone();
        }
    }
    let mut buf = Vec::new();
    let ok = std::fs::File::open(entry.path())
        .and_then(|f| f.take(TEXT_BYTES).read_to_end(&mut buf))
        .is_ok();
    let lines = if ok && !buf.contains(&0) {
        String::from_utf8_lossy(&buf)
            .lines()
            .take(TEXT_LINES)
            .map(|l| l.replace('\t', "    "))
            .collect()
    } else {
        Vec::new()
    };
    cx.state.text_preview = Some((entry.key, lines.clone()));
    lines
}
