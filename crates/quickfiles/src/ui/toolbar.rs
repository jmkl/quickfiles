//! Navigation row: back / forward / up, history dropdown, bookmark, breadcrumb, overflow menu.

use eframe::egui::{Color32, Rect, Sense, Ui, pos2, vec2};

use crate::model;

use super::{
    Action, Cx,
    icons::{self, Icon},
    widgets::{btn_size, galley, hit, icon_button, vline},
};

pub fn show(ui: &mut Ui, cx: &mut Cx<'_>, r: Rect) {
    let th = cx.theme;
    let pal = &th.palette;
    let p = ui.painter().clone();
    let cy = r.top() + 15.5;
    let x0 = r.left();
    let size = btn_size();

    let can_back = !cx.state.back.is_empty();
    let can_forward = !cx.state.forward.is_empty();
    let can_up = model::parent_of(&cx.state.path).is_some();

    if icon_button(
        ui,
        th,
        Icon::ArrowLeft,
        pos2(x0 + 18.5, cy),
        size,
        "nav_back",
        can_back,
    )
    .clicked()
    {
        cx.actions.push(Action::Back);
    }
    if icon_button(
        ui,
        th,
        Icon::ArrowRight,
        pos2(x0 + 47.5, cy),
        size,
        "nav_fwd",
        can_forward,
    )
    .clicked()
    {
        cx.actions.push(Action::Forward);
    }
    if icon_button(
        ui,
        th,
        Icon::ArrowUp,
        pos2(x0 + 76.5, cy),
        size,
        "nav_up",
        can_up,
    )
    .clicked()
    {
        cx.actions.push(Action::Up);
    }
    // History dropdown (decorative in this prototype).
    icon_button(
        ui,
        th,
        Icon::ChevronDown,
        pos2(x0 + 105.5, cy),
        vec2(22.0, 24.0),
        "nav_history",
        false,
    );

    vline(&p, x0 + 126.0, r.top() + 7.0, r.top() + 25.0, pal.border);
    icon_button(
        ui,
        th,
        Icon::Lock,
        pos2(x0 + 147.5, cy),
        size,
        "nav_lock",
        true,
    );
    icon_button(
        ui,
        th,
        Icon::Bookmark,
        pos2(x0 + 178.5, cy),
        size,
        "nav_bookmark",
        true,
    );
    vline(&p, x0 + 199.0, r.top() + 7.0, r.top() + 25.0, pal.border);

    icon_button(
        ui,
        th,
        Icon::Kebab,
        pos2(r.right() - 20.5, cy),
        vec2(24.0, 26.0),
        "nav_more",
        true,
    );

    breadcrumb(ui, cx, x0 + 210.0, cy, r.right() - 56.0);
}

/// "Roaming > Adobe > ... > current". Earlier segments are dim and clickable; the last is bright.
fn breadcrumb(ui: &mut Ui, cx: &mut Cx<'_>, start_x: f32, cy: f32, limit_x: f32) {
    let th = cx.theme;
    let pal = &th.palette;
    let p = ui.painter().clone();
    let mut x = start_x;
    let crumbs = model::breadcrumbs(&cx.state.path);
    let last = crumbs.len() - 1;

    for (i, (label, target)) in crumbs.iter().enumerate() {
        if x >= limit_x {
            break;
        }
        let is_last = i == last;
        // Laid out once with a placeholder colour; the real colour is chosen at draw time.
        let g = galley(
            &p,
            label,
            &th.font,
            Color32::PLACEHOLDER,
            limit_x - x,
        );
        let w = g.size().x;

        let hit_rect = Rect::from_min_size(pos2(x - 4.0, cy - 11.0), vec2(w + 8.0, 22.0));
        let sense = if is_last {
            Sense::hover()
        } else {
            Sense::click()
        };
        let response = hit(ui, hit_rect, ("crumb", i), sense);
        let color = if is_last || response.hovered() {
            pal.text
        } else {
            pal.text_dim
        };
        p.galley(pos2(x, cy - g.size().y * 0.5), g, color);

        if response.clicked() {
            cx.actions.push(Action::Navigate(target.clone()));
        }

        x += w;
        if !is_last {
            icons::paint_scaled(
                &p,
                pal,
                Icon::ChevronRight,
                pos2(x + 15.5, cy),
                pal.text_dim,
                0.7,
            );
            x += 30.0;
        }
    }
}
