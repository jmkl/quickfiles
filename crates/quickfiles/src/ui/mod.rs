//! Immediate-mode UI. Each region lives in its own module and draws itself into a fixed rect
//! computed by [`layout::Regions`]. Modules read the [`Theme`], mutate view-state directly,
//! and request anything with side effects (navigation, file operations) through [`Action`].

mod dnd;
mod drives;
mod file_list;
mod gate;
mod grid;
mod icons;
mod layout;
mod menu;
mod perf;
mod preview;
mod resize;
mod sidebar;
mod splitter;
mod tabs;
mod tex;
mod title_bar;
mod toolbar;
mod widgets;

use std::path::PathBuf;

use eframe::egui::{Context, Id, Ui, UiBuilder};
use quickfiles_core::{Conflict, Core, EngineStatus};

use crate::{state::State, theme::Theme};

pub use dnd::dropped;
pub use menu::{MenuItem, MenuState};
pub use perf::Perf;
pub use tex::Tex;
pub use tex::TexStats;

/// Side effects the UI asks the app to perform after the frame.
#[derive(Clone, Debug)]
pub enum Action {
    /// Enter a folder (empty path = This PC).
    Navigate(PathBuf),
    /// Double-click / Enter on a row.
    OpenPath {
        path: PathBuf,
        is_dir: bool,
    },
    Back,
    Forward,
    Up,
    ToggleSidebar,
    TogglePreview,
    FocusSearch,
    Refresh,
    SelectAll,
    /// Esc: clears the search text first, then the selection.
    Escape,
    /// The x button: clear the text and search everything again.
    ClearSearch,
    /// Moves the active item up/down in the list (selecting it).
    MoveFocus(i32),
    /// Shift+Arrow: grows the selection.
    ExtendFocus(i32),
    /// Enter: opens the active item (or commits the query when nothing is active).
    OpenFocused,
    ToggleRegex,
    ToggleHidden,
    ToggleBookmark,
    RemoveBookmark(PathBuf),
    Copy,
    Cut,
    Paste,
    Delete {
        permanent: bool,
    },
    ConfirmDelete,
    Rename,
    NewFolder,
    Reveal,
    ClearHistory,
    /// Menu: delete every cached thumbnail (memory and disk).
    ClearThumbCache,
    CancelJob(u64),
    GateStartEverything,
    GateDownload,
    GateRetry,
    CloseMenu,
    /// Left/Right arrows (grid view only).
    MoveSide(i32),
    ToggleGrid,
    /// F12: performance overlay; Ctrl+F12 copies a report; Shift+F12 resets the counters.
    TogglePerf,
    CopyPerf,
    ResetPerf,
    /// Ctrl + mouse wheel: up = grid, down = list.
    SetGrid(bool),
    CopyPath,
    NewTab,
    CloseTab(usize),
    CloseCurrentTab,
    SwitchTab(usize),
    /// Ctrl+Tab / Ctrl+Shift+Tab.
    CycleTab(i32),
    /// Paste after the user chose how to resolve name clashes.
    PasteWith(Conflict),
    /// Move or copy into a folder (drag and drop). `policy: None` asks when names clash.
    Transfer {
        paths: Vec<PathBuf>,
        dest: PathBuf,
        copy: bool,
        policy: Option<Conflict>,
    },
}

/// Everything a region needs while drawing.
pub struct Cx<'a> {
    pub theme: &'a Theme,
    pub state: &'a mut State,
    pub core: &'a mut Core,
    pub tex: &'a mut Tex,
    pub actions: &'a mut Vec<Action>,
}

/// Seconds a side panel takes to slide in or out.
const SLIDE_SECS: f32 = 0.18;

/// Animated 0..=1 "how open" value for a panel, eased out.
fn slide(ctx: &Context, key: &'static str, open: bool) -> f32 {
    let t = ctx.animate_bool_with_time(Id::new(key), open, SLIDE_SECS);
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
    // Widgets go inert while a menu is open (it was opened in an earlier frame).
    widgets::set_modal(&ctx, cx.state.menu.as_ref().is_some_and(|m| !m.fresh));
    cx.tex.begin_frame();
    cx.core.images().begin_frame();

    let left = layout::Slide {
        width: cx.state.left_w,
        t: slide(&ctx, "left_panel", cx.state.left_open),
    };
    let right = layout::Slide {
        width: cx.state.preview_w,
        t: slide(&ctx, "right_panel", cx.state.preview_open),
    };
    let regions = layout::Regions::compute(
        full,
        &theme.metrics,
        left,
        right,
        if cx.core.tab_count() > 1 {
            tabs::HEIGHT
        } else {
            0.0
        },
    );

    ui.painter().rect_filled(full, 0.0, theme.palette.main);

    // Everything is required: without it the whole body is the gate screen.
    let gate = matches!(cx.core.status(), EngineStatus::Missing(_));
    if gate {
        let body = eframe::egui::Rect::from_min_max(
            eframe::egui::pos2(full.left(), regions.title.bottom()),
            full.max,
        );
        gate::show(ui, cx, body);
    } else {
        file_list::show(ui, cx, regions.header, regions.list);
        if regions.tabs.height() > 0.0 {
            tabs::show(ui, cx, regions.tabs);
        }
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
    }
    title_bar::show(ui, cx, regions.title);
    menu::show(ui, cx, full);
    menu::toast(ui, cx, full);
    dnd::overlay(ui, cx, full);
    perf::overlay(ui, cx, full);
    // Last, so the resize grips sit on top of everything else.
    resize::show(ui, full);
}
