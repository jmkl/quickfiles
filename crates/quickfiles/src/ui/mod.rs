//! Immediate-mode UI. Each region lives in its own module and draws itself into a fixed rect
//! computed by [`layout::Regions`]. Modules read the [`Theme`], mutate view-state directly,
//! and request anything with side effects (navigation) through [`Action`].

mod file_list;
mod icons;
mod layout;
mod preview;
mod resize;
mod sidebar;
mod splitter;
mod title_bar;
mod toolbar;
mod widgets;

use std::path::PathBuf;

use eframe::egui::{Context, Id, Ui, UiBuilder};

use crate::{state::State, theme::Theme};

/// Side effects the UI asks the app to perform after the frame.
#[derive(Debug)]
pub enum Action {
    Navigate(PathBuf),
    OpenFolder(String),
    Back,
    Forward,
    Up,
    ToggleSidebar,
    TogglePreview,
    FocusSearch,
    Refresh,
    SelectAll,
    /// Esc: clears the search first, then the selection.
    Escape,
    /// Moves the active item up/down in the list.
    MoveFocus(i32),
    /// Enter: opens the active item if it is a folder.
    OpenFocused,
}

/// Everything a region needs while drawing.
pub struct Cx<'a> {
    pub theme: &'a Theme,
    pub state: &'a mut State,
    pub actions: &'a mut Vec<Action>,
}

/// Seconds a side panel takes to slide in or out.
const SLIDE_SECS: f32 = 0.18;

/// Animated 0..=1 "how open" value for a panel, eased out.
fn slide(ctx: &Context, key: &'static str, open: bool) -> f32 {
    let t = ctx.animate_bool_with_time(Id::unique(key), open, SLIDE_SECS);
    t * (2.0 - t)
}

/// A child `Ui` that lays out into `panel.layout` but only paints/interacts inside `panel.visible`.
fn panel_ui(ui: &mut Ui, panel: layout::Panel) -> Ui {
    let mut child = ui.new_child(UiBuilder::new().max_rect(panel.layout));
    child.set_clip_rect(panel.visible.intersect(ui.clip_rect()));
    child
}

pub fn draw(ui: &mut Ui, cx: &mut Cx<'_>) {
    let theme = cx.theme;
    let ctx = ui.ctx().clone();
    let full = ui.max_rect();

    let left = layout::Slide {
        width: cx.state.left_w,
        t: slide(&ctx, "left_panel", cx.state.left_open),
    };
    let right = layout::Slide {
        width: cx.state.preview_w,
        t: slide(&ctx, "right_panel", cx.state.preview_open),
    };
    let regions = layout::Regions::compute(full, &theme.metrics, left, right);

    ui.painter().rect_filled(full, 0.0, theme.palette.main);

    file_list::show(ui, cx, regions.header, regions.list);
    toolbar::show(ui, cx, regions.toolbar);
    if let Some(panel) = regions.sidebar {
        let mut child = panel_ui(ui, panel);
        sidebar::show(&mut child, cx, panel.layout);
    }
    if let Some(panel) = regions.preview {
        let mut child = panel_ui(ui, panel);
        preview::show(&mut child, cx, panel.layout);
    }
    let left_ready = cx.state.left_open && left.t >= 0.999;
    let right_ready = cx.state.preview_open && right.t >= 0.999;
    splitter::show(ui, cx, full, &regions, left_ready, right_ready);
    title_bar::show(ui, cx, regions.title);
    // Last, so the resize grips sit on top of everything else.
    resize::show(ui, full);
}
