//! Custom title bar: hamburger, browser-style search box (centred) and window buttons.

use eframe::egui::{
    self, Align, Align2, Color32, Frame, Id, Key, Layout, Rect, RichText, Sense, Stroke,
    StrokeKind, TextEdit, Ui, UiBuilder, ViewportCommand, pos2, vec2,
};

use crate::model;

use super::{
    Action, Cx,
    icons::{self, Icon},
    widgets::{hit, hline, icon_button},
};

/// Widest the search box grows; it shrinks with the window.
const SEARCH_MAX_W: f32 = 560.0;
const SEARCH_MIN_W: f32 = 160.0;

pub fn show(ui: &mut Ui, cx: &mut Cx<'_>, r: Rect) {
    let th = cx.theme;
    let (pal, m) = (&th.palette, &th.metrics);
    let p = ui.painter().clone();
    let ctx = ui.ctx().clone();

    p.rect_filled(r, 0.0, pal.chrome);
    hline(&p, r.bottom() - 1.0, r.left(), r.right(), pal.border);

    // Whole bar drags the window; double-click toggles maximise. Registered first so the
    // controls drawn after it take priority.
    let drag = hit(ui, r, "titlebar_drag", Sense::click_and_drag());
    if drag.drag_started() {
        ctx.send_viewport_cmd(ViewportCommand::StartDrag);
    }
    if drag.double_clicked() {
        let maximized = ctx.input(|i| i.viewport().maximized.unwrap_or(false));
        ctx.send_viewport_cmd(ViewportCommand::Maximized(!maximized));
    }

    // Left control
    let cy = r.top() + 16.5;
    let menu_right = r.left() + 19.5 + 16.0;
    // Hamburger toggles the left sidebar (Ctrl+B).
    if icon_button(
        ui,
        th,
        Icon::Menu,
        pos2(r.left() + 19.5, cy),
        vec2(32.0, 26.0),
        "menu",
        true,
    )
    .clicked()
    {
        cx.actions.push(Action::ToggleSidebar);
    }

    // Preview panel toggle (Alt+P), just left of the window buttons.
    let preview_center = pos2(r.right() - m.win_btn_w * 3.0 - 22.0, cy);
    if cx.state.preview_open {
        p.rect_filled(
            Rect::from_center_size(preview_center, vec2(30.0, 26.0)),
            m.radius,
            pal.hover,
        );
    }
    if icon_button(
        ui,
        th,
        Icon::PanelRight,
        preview_center,
        vec2(30.0, 26.0),
        "preview_toggle",
        true,
    )
    .clicked()
    {
        cx.actions.push(Action::TogglePreview);
    }

    // Window buttons, right-aligned
    let bw = m.win_btn_w;
    let button = |slot: f32| {
        Rect::from_min_size(
            pos2(r.right() - bw * (slot + 1.0), r.top()),
            vec2(bw, r.height() - 1.0),
        )
    };
    let icon_cy = r.top() + 16.0;

    if window_button(ui, th, button(2.0), Icon::Minimize, "win_min", pal.hover, icon_cy).clicked() {
        ctx.send_viewport_cmd(ViewportCommand::Minimized(true));
    }
    if window_button(ui, th, button(1.0), Icon::Maximize, "win_max", pal.hover, icon_cy).clicked() {
        let maximized = ctx.input(|i| i.viewport().maximized.unwrap_or(false));
        ctx.send_viewport_cmd(ViewportCommand::Maximized(!maximized));
    }
    if window_button(
        ui,
        th,
        button(0.0),
        Icon::Close,
        "win_close",
        pal.close_hover,
        icon_cy,
    )
    .clicked()
    {
        ctx.send_viewport_cmd(ViewportCommand::Close);
    }

    // Search box: centred in the window when there is room, otherwise squeezed between the
    // menu button and the window buttons.
    let lo = menu_right + 14.0;
    let hi = r.right() - bw * 3.0 - 44.0;
    let width = (hi - lo).min((r.width() * 0.5).clamp(SEARCH_MIN_W, SEARCH_MAX_W));
    if width >= 60.0 {
        let x = (r.center().x - width * 0.5).clamp(lo, (hi - width).max(lo));
        let rect = Rect::from_min_size(
            pos2(x, cy - th.metrics.search_h * 0.5),
            vec2(width, th.metrics.search_h),
        );
        search_box(ui, cx, rect);
    }
}

fn search_box(ui: &mut Ui, cx: &mut Cx<'_>, rect: Rect) {
    let th = cx.theme;
    let pal = &th.palette;
    let p = ui.painter().clone();
    let ctx = ui.ctx().clone();
    let id = Id::new("title_search");

    // Ctrl+F / Ctrl+L (see hotkeys.rs) ask for focus through this one-shot flag.
    if std::mem::take(&mut cx.state.focus_search) {
        ctx.memory_mut(|mem| mem.request_focus(id));
    }

    // The whole pill is clickable, not only the text field.
    let pill = hit(ui, rect, "search_pill", Sense::click());
    if pill.clicked() {
        ctx.memory_mut(|mem| mem.request_focus(id));
    }

    let focused = ctx.memory(|mem| mem.has_focus(id));
    let radius = rect.height() * 0.5;
    p.rect_filled(rect, radius, pal.main);
    let edge = if focused {
        pal.accent
    } else if pill.hovered() {
        pal.track_border
    } else {
        pal.border
    };
    p.rect_stroke(rect, radius, Stroke::new(1.0, edge), StrokeKind::Inside);

    icons::paint_scaled(
        &p,
        pal,
        Icon::Search,
        pos2(rect.left() + 15.0, rect.center().y),
        if focused { pal.text_section } else { pal.text_dim },
        0.75,
    );

    let has_text = !cx.state.filter.is_empty();
    let clear_w = if has_text { 26.0 } else { 0.0 };

    let edit_rect = Rect::from_min_max(
        pos2(rect.left() + 29.0, rect.top() + 2.0),
        pos2(rect.right() - 12.0 - clear_w, rect.bottom() - 2.0),
    );
    let hint = format!("Search in {}", model::display_name(&cx.state.path));
    let mut edit_ui = ui.new_child(
        UiBuilder::new()
            .max_rect(edit_rect)
            .layout(Layout::left_to_right(Align::Center)),
    );
    let response = edit_ui.add(
        TextEdit::singleline(&mut cx.state.filter)
            .id(id)
            .hint_text(RichText::new(hint).color(pal.text_dim))
            .frame(Frame::NONE)
            .desired_width(edit_rect.width())
            .align(Align2::CENTER_CENTER),
    );
    if response.changed() {
        cx.state.rebuild_view();
    }
    // Esc inside the box clears it and hands focus back to the list.
    if response.has_focus() && ctx.input(|i| i.key_pressed(Key::Escape)) {
        cx.state.filter.clear();
        cx.state.rebuild_view();
        response.surrender_focus();
    }

    // Clear button (only while there is text).
    if has_text {
        let center = pos2(rect.right() - 17.0, rect.center().y);
        let clear_rect = Rect::from_center_size(center, vec2(18.0, 18.0));
        let clear = hit(ui, clear_rect, "search_clear", Sense::click());
        if clear.hovered() {
            p.rect_filled(clear_rect, 9.0, pal.hover);
        }
        icons::paint_scaled(
            &p,
            pal,
            Icon::Close,
            center,
            if clear.hovered() { pal.icon_strong } else { pal.text_section },
            0.8,
        );
        if clear.clicked() {
            cx.state.filter.clear();
            cx.state.rebuild_view();
        }
    }
}

fn window_button(
    ui: &Ui,
    th: &crate::theme::Theme,
    rect: Rect,
    icon: Icon,
    salt: &'static str,
    hover_fill: Color32,
    icon_cy: f32,
) -> egui::Response {
    let pal = &th.palette;
    let response = hit(ui, rect, salt, Sense::click());
    let p = ui.painter();
    if response.hovered() {
        p.rect_filled(rect, 0.0, hover_fill);
    }
    let tint = if response.hovered() && hover_fill == pal.close_hover {
        Color32::WHITE
    } else {
        pal.icon
    };
    icons::paint_scaled(p, pal, icon, pos2(rect.center().x, icon_cy), tint, 1.0);
    response
}
