//! Right-hand preview panel. Shown when the active item is a selected file.
//! All content is dummy: a fake "code" view generated from the file kind.

use eframe::egui::{Align2, Color32, Rect, Stroke, StrokeKind, Ui, pos2, vec2};

use super::{
    Action, Cx,
    icons::{self, Icon},
    widgets::{galley, hline, icon_button, text_left, vline},
};
use crate::model::{self, EntryKind, FileEntry, format_size};

pub fn show(ui: &mut Ui, cx: &mut Cx<'_>, r: Rect) {
    let th = cx.theme;
    let (pal, m) = (&th.palette, &th.metrics);
    let p = ui.painter().clone();

    p.rect_filled(r, 0.0, pal.sidebar);
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
        pal.text_section,
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
    let active = cx.state.preview_index().zip(cx.state.listing.clone());
    let Some((idx, listing)) = active else {
        p.text(
            pos2((left + r.right()) * 0.5, r.top() + head_h + 60.0),
            Align2::CENTER_CENTER,
            "Select a file to preview",
            th.font.clone(),
            pal.text_dim,
        );
        return;
    };
    let entry = &listing.entries[idx as usize];

    // Big icon card
    let mid_x = (left + r.right()) * 0.5;
    let card = Rect::from_center_size(pos2(mid_x, r.top() + head_h + 62.0), vec2(96.0, 96.0));
    p.rect_filled(card, 10.0, pal.main);
    p.rect_stroke(card, 10.0, Stroke::new(1.0, pal.border), StrokeKind::Inside);
    icons::paint_scaled(
        &p,
        pal,
        Icon::for_kind(entry.kind),
        card.center(),
        pal.text,
        3.0,
    );

    // Name + type, centred
    let max_w = r.width() - 32.0;
    let name_y = card.bottom() + 20.0;
    let g = galley(&p, &entry.name, &th.font, pal.text, max_w);
    p.galley(pos2(mid_x - g.size().x * 0.5, name_y - g.size().y * 0.5), g, pal.text);
    let g = galley(&p, entry.type_label, &th.font, pal.text_dim, max_w);
    p.galley(
        pos2(mid_x - g.size().x * 0.5, name_y + 20.0 - g.size().y * 0.5),
        g,
        pal.text_dim,
    );

    // Details
    let mut y = name_y + 52.0;
    hline(&p, y - 12.0, left + 16.0, r.right() - 16.0, pal.border);
    let location = if cx.state.path.as_os_str().is_empty() {
        model::ROOT_LABEL.to_owned()
    } else {
        cx.state.path.display().to_string()
    };
    for (label, value) in [
        ("Size", format_size(entry.size_bytes)),
        ("Modified", entry.modified.clone()),
        ("Location", location),
    ] {
        text_left(&p, left + 16.0, y, label, &th.font, pal.text_dim, 70.0);
        text_left(&p, left + 90.0, y, &value, &th.font, pal.text, r.right() - left - 90.0 - 16.0);
        y += 24.0;
    }

    // Content preview
    y += 4.0;
    text_left(&p, left + 16.0, y, "Content", &th.font, pal.text_section, 120.0);
    let body = Rect::from_min_max(
        pos2(left + 16.0, y + 14.0),
        pos2(r.right() - 16.0, r.bottom() - 16.0),
    );
    if body.height() < 40.0 {
        return;
    }
    p.rect_filled(body, m.radius, pal.main);
    p.rect_stroke(body, m.radius, Stroke::new(1.0, pal.border), StrokeKind::Inside);

    let lines = dummy_lines(entry);
    if lines.is_empty() {
        p.text(
            body.center(),
            Align2::CENTER_CENTER,
            "No preview available",
            th.font.clone(),
            pal.text_dim,
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
        text_left(&p, body.left() + gutter, cy, line, &th.mono, code_color(pal, line), text_w);
    }
}

/// Very rough tint so the fake code is not one flat colour.
fn code_color(pal: &crate::theme::Palette, line: &str) -> Color32 {
    let t = line.trim_start();
    if t.starts_with("//") || t.starts_with("<!--") {
        pal.text_dim
    } else {
        pal.text_section
    }
}

fn dummy_lines(entry: &FileEntry) -> Vec<String> {
    let template: &[&str] = match entry.kind {
        EntryKind::Json => &[
            "{",
            "  \"id\": \"%NAME%\",",
            "  \"name\": \"%NAME%\",",
            "  \"version\": \"1.0.0\",",
            "  \"main\": \"index.html\",",
            "  \"host\": [",
            "    { \"app\": \"PS\", \"minVersion\": \"24.2\" }",
            "  ],",
            "  \"entrypoints\": [",
            "    { \"type\": \"panel\", \"id\": \"main\" }",
            "  ]",
            "}",
        ],
        EntryKind::Script => &[
            "// %NAME%",
            "const { app } = require(\"photoshop\");",
            "",
            "async function run() {",
            "  const doc = app.activeDocument;",
            "  console.log(\"active:\", doc?.name);",
            "}",
            "",
            "document",
            "  .getElementById(\"btn\")",
            "  .addEventListener(\"click\", run);",
        ],
        EntryKind::Html => &[
            "<!DOCTYPE html>",
            "<html>",
            "  <head>",
            "    <meta charset=\"utf-8\" />",
            "    <title>%NAME%</title>",
            "  </head>",
            "  <body>",
            "    <button id=\"btn\">Run</button>",
            "    <script src=\"index.js\"></script>",
            "  </body>",
            "</html>",
        ],
        EntryKind::Folder | EntryKind::Other => &[],
    };
    template
        .iter()
        .map(|l| l.replace("%NAME%", &entry.name))
        .collect()
}
