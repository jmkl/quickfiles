//! Tab strip above the toolbar. Only shown when there is more than one tab.

use eframe::egui::{Align2, Rect, Sense, Stroke, StrokeKind, Ui, pos2, vec2};

use super::{
    Action, Cx,
    icons::{self, Icon},
    widgets::{galley, hit, hline},
};

pub const HEIGHT: f32 = 30.0;
const MAX_TAB_W: f32 = 190.0;
const PLUS_W: f32 = 30.0;

pub fn show(ui: &mut Ui, cx: &mut Cx<'_>, r: Rect) {
    let th = cx.theme;
    let pal = &th.palette;
    let p = ui.painter().clone();
    let n = cx.core.tab_count();
    let active = cx.core.active_tab();

    p.rect_filled(r, 0.0, pal.chrome);
    hline(&p, r.bottom() - 1.0, r.left(), r.right(), pal.border);

    let tab_w = ((r.width() - PLUS_W - 8.0) / n as f32).clamp(70.0, MAX_TAB_W);
    for i in 0..n {
        let rect = Rect::from_min_size(
            pos2(r.left() + 6.0 + tab_w * i as f32, r.top() + 4.0),
            vec2(tab_w - 3.0, HEIGHT - 5.0),
        );
        let response = hit(ui, rect, ("tab", i), Sense::click());
        if i == active {
            p.rect_filled(rect, 6.0, pal.main);
            p.rect_stroke(rect, 6.0, Stroke::new(1.0, pal.border), StrokeKind::Inside);
        } else if response.hovered() {
            p.rect_filled(rect, 6.0, pal.hover);
        }
        let title = cx.core.tab_title(i);
        let color = if i == active { pal.text } else { pal.text_dim };
        let g = galley(&p, &title, &th.font, color, tab_w - 46.0);
        p.galley(
            pos2(rect.left() + 10.0, rect.center().y - g.size().y * 0.5),
            g,
            color,
        );

        if n > 1 {
            let c = pos2(rect.right() - 13.0, rect.center().y);
            let close_rect = Rect::from_center_size(c, vec2(16.0, 16.0));
            let close = hit(ui, close_rect, ("tab_close", i), Sense::click());
            if close.hovered() {
                p.rect_filled(close_rect, 8.0, pal.hover);
            }
            icons::paint_scaled(
                &p,
                pal,
                Icon::Close,
                c,
                if close.hovered() {
                    pal.icon_strong
                } else {
                    pal.text_section
                },
                0.65,
            );
            if close.clicked() {
                cx.actions.push(Action::CloseTab(i));
                continue;
            }
        }
        if response.clicked() {
            cx.actions.push(Action::SwitchTab(i));
        }
        if response.middle_clicked() {
            cx.actions.push(Action::CloseTab(i));
        }
    }

    let plus = Rect::from_center_size(
        pos2(
            r.left() + 6.0 + tab_w * n as f32 + PLUS_W * 0.5,
            r.center().y,
        ),
        vec2(24.0, 22.0),
    );
    let r2 = hit(ui, plus, "tab_new", Sense::click());
    if r2.hovered() {
        p.rect_filled(plus, 6.0, pal.hover);
    }
    p.text(
        plus.center(),
        Align2::CENTER_CENTER,
        "+",
        th.font.clone(),
        pal.text,
    );
    if r2.clicked() {
        cx.actions.push(Action::NewTab);
    }
}
