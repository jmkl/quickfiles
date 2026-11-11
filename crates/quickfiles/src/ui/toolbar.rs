//! Navigation row: back / forward / up, history dropdown, bookmark, breadcrumb, overflow menu.

use eframe::egui::{Color32, Rect, Sense, Ui, pos2, vec2};
use quickfiles_core::{EngineStatus, JobState, ViewState, breadcrumbs, group_digits};

use super::{
    Action, Cx, MenuItem, MenuState, dnd,
    icons::{self, Icon},
    widgets::{btn_size, galley, hit, icon_button, text_right, vline},
};

pub fn show(ui: &mut Ui, cx: &mut Cx<'_>, r: Rect) {
    let th = cx.theme;
    let pal = &th.palette;
    let p = ui.painter().clone();
    let cy = r.top() + 15.5;
    let x0 = r.left();
    let size = btn_size();

    let (can_back, can_forward, can_up) = {
        let nav = cx.core.nav();
        (nav.can_back(), nav.can_forward(), nav.can_up())
    };

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
    // Recent folders.
    let recents = cx.core.data().recents.clone();
    let history = icon_button(
        ui,
        th,
        Icon::ChevronDown,
        pos2(x0 + 105.5, cy),
        vec2(22.0, 24.0),
        "nav_history",
        !recents.is_empty(),
    );
    if history.clicked() {
        let items = recents
            .iter()
            .take(12)
            .map(|p| {
                MenuItem::new(
                    quickfiles_core::display_name(p),
                    Action::Navigate(p.clone()),
                )
            })
            .collect();
        cx.state.menu =
            Some(MenuState::at(history.rect.left_bottom(), items).with_header("Recent folders"));
    }

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
    // Bookmark the current folder (filled accent colour when it is bookmarked).
    let here = cx
        .core
        .nav()
        .location()
        .filter(|p| !p.as_os_str().is_empty())
        .cloned();
    let marked = here.as_ref().is_some_and(|p| cx.core.is_bookmarked(p));
    if marked {
        p.rect_filled(
            Rect::from_center_size(pos2(x0 + 178.5, cy), size),
            th.metrics.radius,
            pal.accent.gamma_multiply(0.25),
        );
    }
    if icon_button(
        ui,
        th,
        Icon::Bookmark,
        pos2(x0 + 178.5, cy),
        size,
        "nav_bookmark",
        here.is_some(),
    )
    .clicked()
    {
        cx.actions.push(Action::ToggleBookmark);
    }
    vline(&p, x0 + 199.0, r.top() + 7.0, r.top() + 25.0, pal.border);

    let more = icon_button(
        ui,
        th,
        Icon::Kebab,
        pos2(r.right() - 20.5, cy),
        vec2(24.0, 26.0),
        "nav_more",
        true,
    );
    if more.clicked() {
        let q = cx.core.nav().query();
        let (hidden, regex) = (q.show_hidden, q.regex);
        let grid = cx.state.grid;
        let items = vec![
            MenuItem::new("Show hidden files", Action::ToggleHidden).checked(hidden),
            MenuItem::new("Regular expression", Action::ToggleRegex).checked(regex),
            MenuItem::new("Thumbnail grid", Action::ToggleGrid).checked(grid),
            MenuItem::separator(),
            MenuItem::new("New tab", Action::NewTab),
            MenuItem::new("New folder", Action::NewFolder),
            MenuItem::new("Copy path", Action::CopyPath),
            MenuItem::new("Refresh", Action::Refresh),
            MenuItem::separator(),
            MenuItem::new("Clear history", Action::ClearHistory),
            MenuItem::new("Clear thumbnail cache", Action::ClearThumbCache),
        ];
        cx.state.menu = Some(MenuState::at(
            more.rect.left_bottom() - vec2(190.0, 0.0),
            items,
        ));
    }

    let info_w = info(ui, cx, r.right() - 40.0, cy);
    breadcrumb(ui, cx, x0 + 210.0, cy, r.right() - 56.0 - info_w);
}

/// "Roaming > Adobe > ... > current". Earlier segments are dim and clickable; the last is bright.
fn breadcrumb(ui: &mut Ui, cx: &mut Cx<'_>, start_x: f32, cy: f32, limit_x: f32) {
    let th = cx.theme;
    let pal = &th.palette;
    let p = ui.painter().clone();
    let mut x = start_x;
    // Search mode has no folder: the single crumb says what the list is.
    let crumbs = match cx.core.nav().location() {
        Some(path) => breadcrumbs(path),
        None => vec![(
            if cx.core.nav().text().trim().is_empty() {
                "Search everything".to_owned()
            } else {
                "Search results".to_owned()
            },
            std::path::PathBuf::new(),
        )],
    };
    let in_search = cx.core.nav().is_search_mode();
    let last = crumbs.len() - 1;

    for (i, (label, target)) in crumbs.iter().enumerate() {
        if x >= limit_x {
            break;
        }
        let is_last = i == last || in_search;
        // Laid out once with a placeholder colour; the real colour is chosen at draw time.
        let g = galley(&p, label, &th.font, Color32::PLACEHOLDER, limit_x - x);
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

        if !is_last && !target.as_os_str().is_empty() {
            dnd::target(ui, cx, hit_rect, target);
        }
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

/// Right-aligned status: running file job, Everything state, or result count and timing.
/// Returns the width taken so the breadcrumb can stop before it.
fn info(ui: &mut Ui, cx: &mut Cx<'_>, right: f32, cy: f32) -> f32 {
    let th = cx.theme;
    let pal = &th.palette;
    let p = ui.painter().clone();

    let job = cx
        .core
        .jobs()
        .into_iter()
        .find(|j| j.state == JobState::Running);
    let text = if let Some(j) = &job {
        format!(
            "{} {} ({}/{}, {:.0}%) · click to cancel",
            j.kind.verb(),
            j.current,
            j.done,
            j.total,
            j.fraction() * 100.0
        )
    } else if cx.core.status() != EngineStatus::Ready {
        "Everything is indexing…".to_owned()
    } else {
        let view = cx.core.view();
        match view.state() {
            ViewState::Loading => "Searching…".to_owned(),
            ViewState::Ready => {
                let n = group_digits(view.len() as u64);
                let ms = view.took().as_secs_f64() * 1000.0;
                let noun = if view.len() == 1 { "item" } else { "items" };
                format!("{n} {noun} · {ms:.0} ms")
            }
            _ => String::new(),
        }
    };
    if text.is_empty() {
        return 0.0;
    }
    let g = galley(&p, &text, &th.font, pal.text_dim, 420.0);
    let w = g.size().x;
    let rect = Rect::from_min_size(pos2(right - w - 6.0, cy - 11.0), vec2(w + 12.0, 22.0));
    let r = hit(
        ui,
        rect,
        "toolbar_info",
        if job.is_some() {
            Sense::click()
        } else {
            Sense::hover()
        },
    );
    let color = if job.is_some() && r.hovered() {
        pal.text
    } else {
        pal.text_dim
    };
    text_right(&p, right, cy, &text, &th.font, color, 420.0);
    if let (Some(j), true) = (job, r.clicked()) {
        cx.actions.push(Action::CancelJob(j.id));
    }
    w + 24.0
}
