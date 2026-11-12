//! "Everything is required" screen, shown instead of the file list while Everything is not
//! reachable. The engine keeps retrying in the background, so it clears by itself.

use eframe::egui::{Align2, Rect, Sense, Stroke, StrokeKind, Ui, pos2, vec2};
use quickfiles_core::{EngineStatus, sys};

use super::{
    Action, Cx,
    icons::{self, Icon},
    widgets::{galley, hit},
};

pub fn show(ui: &mut Ui, cx: &mut Cx<'_>, area: Rect) {
    let th = cx.theme;
    let pal = &th.palette;
    let p = ui.painter().clone();
    let EngineStatus::Missing(reason) = cx.core.status() else {
        return;
    };
    let installed = sys::find_everything().is_some();

    let mid = pos2(area.center().x, area.top() + area.height() * 0.38);
    icons::paint_scaled(
        &p,
        pal,
        Icon::Search,
        mid - vec2(0.0, 70.0),
        pal.text_muted,
        4.0,
    );
    p.text(
        mid,
        Align2::CENTER_CENTER,
        "quickfiles needs Everything",
        th.font.clone(),
        pal.text,
    );
    let g = galley(&p, &reason, &th.font, pal.text_muted, area.width() - 80.0);
    p.galley(
        pos2(mid.x - g.size().x * 0.5, mid.y + 22.0),
        g,
        pal.text_muted,
    );

    let mut buttons: Vec<(&'static str, Action)> = Vec::new();
    if installed {
        buttons.push(("Start Everything", Action::GateStartEverything));
    } else {
        buttons.push(("Download Everything", Action::GateDownload));
    }
    buttons.push(("Retry", Action::GateRetry));

    let (w, h, gap) = (170.0, 30.0, 12.0);
    let total = buttons.len() as f32 * w + (buttons.len() as f32 - 1.0) * gap;
    let mut x = mid.x - total * 0.5;
    for (i, (label, action)) in buttons.into_iter().enumerate() {
        let rect = Rect::from_min_size(pos2(x, mid.y + 62.0), vec2(w, h));
        let r = hit(ui, rect, ("gate_btn", i), Sense::click());
        let primary = i == 0;
        let fill = if primary { pal.accent } else { pal.control };
        let fill = if r.hovered() {
            fill.gamma_multiply(1.15)
        } else {
            fill
        };
        p.rect_filled(rect, th.metrics.radius, fill);
        if !primary {
            p.rect_stroke(
                rect,
                th.metrics.radius,
                Stroke::new(1.0, pal.control_border),
                StrokeKind::Inside,
            );
        }
        p.text(
            rect.center(),
            Align2::CENTER_CENTER,
            label,
            th.font.clone(),
            pal.icon_active,
        );
        if r.clicked() {
            cx.actions.push(action);
        }
        x += w + gap;
    }
    ui.ctx()
        .request_repaint_after(std::time::Duration::from_millis(500));
}
