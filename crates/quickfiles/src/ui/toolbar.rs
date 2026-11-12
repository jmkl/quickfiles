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
    if icon_button(
        ui,
        th,
        Icon::Refresh,
        pos2(x0 + 147.5, cy),
        size,
        "nav_refresh",
        true,
    )
    .clicked()
    {
        cx.actions.push(Action::Refresh);
    }
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
    if cx.state.path_edit.is_some() {
        path_editor(ui, cx, start_x, cy, limit_x);
        return;
    }
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
            pal.text_muted
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
                pal.text_muted,
                0.7,
            );
            x += 30.0;
        }
    }

    // The rest of the bar is the "type a path" area; so is Alt+D.
    if limit_x - x > 12.0 {
        let rest = Rect::from_min_max(pos2(x + 6.0, cy - 11.0), pos2(limit_x, cy + 11.0));
        let r = hit(ui, rest, "crumb_rest", Sense::click());
        if r.hovered() {
            ui.ctx().set_cursor_icon(eframe::egui::CursorIcon::Text);
        }
        if r.clicked() {
            cx.actions.push(Action::EditPath);
        }
    }
}

/// The address bar while editing: folders matching the text are offered below, the bar turns red
/// when no such folder exists, Enter opens it, Esc or clicking away cancels. Tab completes,
/// Up / Down pick a suggestion.
fn path_editor(ui: &mut Ui, cx: &mut Cx<'_>, start_x: f32, cy: f32, limit_x: f32) {
    use eframe::egui::{
        Align, Area, Frame, Id, Key, Layout, Modifiers, Order, Stroke, StrokeKind, TextEdit,
        UiBuilder,
    };

    let th = cx.theme;
    let pal = &th.palette;
    let ctx = ui.ctx().clone();
    let id = Id::new("path_edit");
    let rect = Rect::from_min_max(pos2(start_x - 8.0, cy - 12.0), pos2(limit_x, cy + 12.0));

    // Recompute the suggestions only when the text changed.
    let changed = cx
        .state
        .path_edit
        .as_ref()
        .filter(|t| **t != cx.state.path_suggest_for)
        .cloned();
    if let Some(text) = changed {
        cx.state.path_valid =
            text.trim().is_empty() || quickfiles_core::pathinput::resolve(&text).is_some();
        cx.state.path_suggest = quickfiles_core::pathinput::suggest(&text, 8);
        cx.state.path_suggest_for = text;
        cx.state.path_sel = None;
    }
    let valid = cx.state.path_valid;
    let edge = if valid { pal.accent } else { pal.danger };
    ui.painter().rect_filled(rect, 6.0, pal.background);
    ui.painter()
        .rect_stroke(rect, 6.0, Stroke::new(1.0, edge), StrokeKind::Inside);

    // Tab and the arrows belong to the list, not to egui's focus navigation.
    let (tab, down, up) = ctx.input_mut(|i| {
        (
            i.consume_key(Modifiers::NONE, Key::Tab),
            i.consume_key(Modifiers::NONE, Key::ArrowDown),
            i.consume_key(Modifiers::NONE, Key::ArrowUp),
        )
    });
    let count = cx.state.path_suggest.len();
    if count > 0 && (down || up) {
        let next = match (cx.state.path_sel, down) {
            (None, true) => 0,
            (None, false) => count - 1,
            (Some(i), true) => (i + 1) % count,
            (Some(i), false) => (i + count - 1) % count,
        };
        cx.state.path_sel = Some(next);
    }
    if tab && count > 0 {
        let pick = cx.state.path_sel.unwrap_or(0).min(count - 1);
        let sep = std::path::MAIN_SEPARATOR;
        cx.state.path_edit = Some(format!(
            "{}{sep}",
            cx.state.path_suggest[pick].to_string_lossy()
        ));
        cx.state.path_focus = true;
    }

    let fresh = std::mem::take(&mut cx.state.path_focus);
    let mut edit_ui = ui.new_child(
        UiBuilder::new()
            .max_rect(rect.shrink2(vec2(8.0, 1.0)))
            .layout(Layout::left_to_right(Align::Center)),
    );
    let Some(text) = cx.state.path_edit.as_mut() else {
        return;
    };
    let output = TextEdit::singleline(text)
        .id(id)
        .font(th.font.clone())
        .text_color(if valid { pal.text } else { pal.danger })
        .frame(Frame::NONE)
        .desired_width(rect.width() - 16.0)
        .show(&mut edit_ui);
    let response = output.response;
    // egui moves keyboard focus on Tab and the arrows at the start of the next frame unless the
    // focused widget says it wants them; the list needs both.
    ctx.memory_mut(|m| {
        m.set_focus_lock_filter(
            id,
            eframe::egui::EventFilter {
                tab: true,
                horizontal_arrows: true,
                vertical_arrows: true,
                escape: false,
            },
        );
    });
    if fresh {
        response.request_focus();
        // Everything selected for a first open (typing replaces the old path); the caret at the
        // end after Tab completion.
        let mut state = output.state;
        let len = text.chars().count();
        let from = if cx.state.path_suggest_for.is_empty() || *text != cx.state.path_suggest_for {
            len
        } else {
            0
        };
        state
            .cursor
            .set_char_range(Some(eframe::egui::text::CCursorRange::two(
                eframe::egui::text::CCursor::new(from.min(len)),
                eframe::egui::text::CCursor::new(len),
            )));
        state.store(&ctx, id);
    }

    // The suggestion list.
    let mut picked: Option<std::path::PathBuf> = None;
    cx.state.path_list_rect = None;
    if !cx.state.path_suggest.is_empty() {
        let row_h = 24.0;
        let list = Rect::from_min_size(
            rect.left_bottom() + vec2(0.0, 3.0),
            vec2(
                rect.width(),
                row_h * cx.state.path_suggest.len() as f32 + 6.0,
            ),
        );
        cx.state.path_list_rect = Some(list);
        Area::new(Id::new("path_suggest"))
            .order(Order::Foreground)
            .fixed_pos(list.min)
            .show(&ctx, |ui| {
                let p = ui.painter().clone();
                p.rect_filled(list, 6.0, pal.surface);
                p.rect_stroke(list, 6.0, Stroke::new(1.0, pal.border), StrokeKind::Inside);
                for (i, path) in cx.state.path_suggest.iter().enumerate() {
                    let row = Rect::from_min_size(
                        list.min + vec2(3.0, 3.0 + row_h * i as f32),
                        vec2(list.width() - 6.0, row_h),
                    );
                    let r = hit(ui, row, ("suggest", i), Sense::click());
                    let selected = cx.state.path_sel == Some(i);
                    if selected || r.hovered() {
                        p.rect_filled(row, 4.0, pal.hover);
                    }
                    let g = galley(
                        &p,
                        &path.to_string_lossy(),
                        &th.font,
                        pal.text,
                        row.width() - 16.0,
                    );
                    p.galley(
                        pos2(row.left() + 8.0, row.center().y - g.size().y * 0.5),
                        g,
                        pal.text,
                    );
                    if r.clicked() {
                        picked = Some(path.clone());
                    }
                }
            });
    }
    if let Some(path) = picked {
        cx.actions
            .push(Action::GoToPath(path.to_string_lossy().into_owned()));
        return;
    }

    // Enter makes the box lose focus in the same frame, so look for that, not for focus.
    let enter = response.lost_focus() && ctx.input(|i| i.key_pressed(Key::Enter));
    let escape = ctx.input(|i| i.key_pressed(Key::Escape));
    if enter {
        let target = match cx.state.path_sel {
            Some(i) if i < cx.state.path_suggest.len() => {
                cx.state.path_suggest[i].to_string_lossy().into_owned()
            }
            _ => text.clone(),
        };
        let usable = quickfiles_core::pathinput::resolve(&target).is_some();
        if usable {
            cx.actions.push(Action::GoToPath(target));
        } else {
            // Nothing to open: stay in the box so the text can be fixed.
            response.request_focus();
        }
    } else if escape {
        cx.state.path_edit = None;
    } else if response.lost_focus() {
        // A click on the suggestion list takes focus away first; that is not "clicked away".
        let on_list = cx.state.path_list_rect.is_some_and(|r| {
            ctx.input(|i| i.pointer.hover_pos().or(i.pointer.latest_pos()))
                .is_some_and(|p| r.contains(p))
        });
        if !on_list && !fresh {
            cx.state.path_edit = None;
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
    let g = galley(&p, &text, &th.font, pal.text_muted, 420.0);
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
        pal.text_muted
    };
    text_right(&p, right, cy, &text, &th.font, color, 420.0);
    if let (Some(j), true) = (job, r.clicked()) {
        cx.actions.push(Action::CancelJob(j.id));
    }
    w + 24.0
}
