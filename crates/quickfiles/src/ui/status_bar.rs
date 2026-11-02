//! Bottom bar: file filter on the left, scan stats / view mode / progress pill on the right.

use eframe::egui::{
    Align, Align2, CornerRadius, Frame, Layout, Rect, RichText, Shape, Stroke, StrokeKind,
    TextEdit, Ui, UiBuilder, pos2, vec2,
};

use super::{
    Cx,
    icons::{self, Icon},
    widgets::{text_left, top_outline, vline},
};

pub fn show(ui: &mut Ui, cx: &mut Cx<'_>, r: Rect) {
    let th = cx.theme;
    let (pal, m) = (&th.palette, &th.metrics);
    let p = ui.painter().clone();
    let cy = r.top() + 15.5;
    let border = Stroke::new(1.0, pal.border);

    // --- Filter box (left). Starts one pixel left so its border overlays the sidebar divider.
    let box_rect = Rect::from_min_size(pos2(r.left() - 1.0, r.top()), vec2(248.0, r.height()));
    p.rect_filled(
        box_rect,
        CornerRadius {
            nw: 0,
            ne: 8,
            sw: 0,
            se: 0,
        },
        pal.sidebar,
    );
    p.add(Shape::line(top_outline(box_rect, 0.0, 8.0), border));

    icons::paint(
        &p,
        pal,
        Icon::Search,
        pos2(r.left() + 17.5, cy),
        pal.text_dim,
    );

    let hint = format!("Filter {} files...", cx.state.entry_count());
    let edit_rect = Rect::from_min_max(
        pos2(r.left() + 28.0, r.top() + 5.0),
        pos2(r.left() + 205.0, r.bottom() - 4.0),
    );
    let mut edit_ui = ui.new_child(
        UiBuilder::new()
            .max_rect(edit_rect)
            .layout(Layout::left_to_right(Align::Center)),
    );
    let response = edit_ui.add(
        TextEdit::singleline(&mut cx.state.filter)
            .hint_text(RichText::new(hint).color(pal.text_dim))
            .frame(Frame::NONE)
            .desired_width(edit_rect.width())
            .align(Align2::CENTER_CENTER),
    );
    if response.changed() {
        cx.state.rebuild_view();
    }
    icons::paint_scaled(
        &p,
        pal,
        Icon::TreeList,
        pos2(r.left() + 227.5, cy),
        pal.text_section,
        0.95,
    );

    // --- Stats pill (right). Offsets are measured from the window's right edge.
    let right = r.right();
    let pill = Rect::from_min_max(pos2(right - 329.0, r.top()), r.right_bottom());
    p.rect_filled(
        pill,
        CornerRadius {
            nw: 8,
            ne: 0,
            sw: 0,
            se: 0,
        },
        pal.sidebar,
    );
    p.add(Shape::line(top_outline(pill, 8.0, 0.0), border));

    let (secs, folders, files) = cx
        .state
        .listing
        .as_ref()
        .map_or((0.0, 0, 0), |l| (l.scan_secs, l.folder_count, l.file_count));
    let font = &th.font;
    let icon_tint = pal.text_section;

    icons::paint_scaled(
        &p,
        pal,
        Icon::Clock,
        pos2(right - 312.0, cy),
        icon_tint,
        0.95,
    );
    text_left(
        &p,
        right - 297.0,
        cy,
        &format!("{secs:.2} sec"),
        font,
        pal.text,
        60.0,
    );

    icons::paint_scaled(
        &p,
        pal,
        Icon::FolderOutline,
        pos2(right - 230.5, cy),
        icon_tint,
        0.95,
    );
    text_left(
        &p,
        right - 212.0,
        cy,
        &folders.to_string(),
        font,
        pal.text,
        40.0,
    );

    icons::paint_scaled(
        &p,
        pal,
        Icon::FileOutline,
        pos2(right - 189.0, cy),
        icon_tint,
        0.9,
    );
    text_left(
        &p,
        right - 171.0,
        cy,
        &files.to_string(),
        font,
        pal.text,
        40.0,
    );

    icons::paint_scaled(
        &p,
        pal,
        Icon::Storage,
        pos2(right - 145.0, cy),
        icon_tint,
        0.95,
    );
    vline(
        &p,
        right - 125.0,
        r.top() + 6.0,
        r.bottom() - 6.0,
        pal.border,
    );

    icons::paint_scaled(
        &p,
        pal,
        Icon::ViewDetails,
        pos2(right - 109.0, cy),
        icon_tint,
        0.9,
    );
    text_left(&p, right - 93.0, cy, "Details", font, pal.text, 50.0);

    // Progress box ("0%").
    let progress = Rect::from_min_size(pos2(right - 45.0, r.top() + 5.0), vec2(41.0, 22.0));
    p.rect_filled(progress, m.radius, pal.track);
    p.rect_stroke(
        progress,
        m.radius,
        Stroke::new(1.0, pal.track_border),
        StrokeKind::Inside,
    );
    p.text(
        progress.center(),
        Align2::CENTER_CENTER,
        "0%",
        font.clone(),
        pal.text,
    );
}
