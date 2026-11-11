//! Glue: owns the theme, the UI state and the core; turns UI `Action`s into core calls.

use std::{collections::HashSet, sync::Arc};

use eframe::egui;
use quickfiles_core::{Config, Conflict, Core, JobState, Kind, key_of_path, sys, sys::WindowState};

use crate::{
    hotkeys::Hotkeys,
    state::{Rename, State},
    theme::Theme,
    ui::{self, Action, Cx, MenuItem, MenuState, Tex},
};

pub struct App {
    theme: Theme,
    state: State,
    core: Core,
    tex: Tex,
    hotkeys: Hotkeys,
    saved_widths: (f32, f32),
    saved_cols: crate::state::ColW,
    window: WindowState,
    ctx: egui::Context,
}

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>, dummy: bool) -> Self {
        let theme = Theme::dark();
        theme.apply(&cc.egui_ctx);

        // Background threads wake the UI through this; the UI never polls.
        let ctx = cc.egui_ctx.clone();
        let core = Core::start(Config {
            dummy,
            data_dir: None,
            wake: Arc::new(move || ctx.request_repaint()),
        });
        let state = State::new(&core.data());
        let widths = (state.left_w, state.preview_w);

        // Hotkeys: built-in map plus the user's `hotkeys.conf` (a template is written once).
        sys::write_config_if_missing("hotkeys.conf", &crate::hotkeys::config_template());
        let (hotkeys, problems) =
            Hotkeys::from_config(&sys::read_config("hotkeys.conf").unwrap_or_default());
        let mut state = state;
        if let Some(first) = problems.first() {
            state.say(first.clone());
        }
        let saved_cols = state.col_w;
        Self {
            theme,
            state,
            core,
            tex: Tex::new(),
            hotkeys,
            saved_widths: widths,
            saved_cols,
            window: sys::load_window().unwrap_or(WindowState {
                w: 1152.0,
                h: 760.0,
                maximized: false,
            }),
            ctx: cc.egui_ctx.clone(),
        }
    }

    /// Paths of the selected rows (view order).
    fn selected_paths(&self) -> Vec<std::path::PathBuf> {
        self.core.view().paths_of(&self.state.selected)
    }

    /// Moves the active row; `extend` grows the selection instead (Shift+Arrow).
    fn step(&mut self, delta: i32, extend: bool) {
        let view = self.core.view().clone();
        let len = view.len();
        if len == 0 {
            return;
        }
        let current = self.state.focused.and_then(|k| view.find(k));
        let next = match current {
            Some(pos) => (pos as i64 + i64::from(delta)).clamp(0, len as i64 - 1) as usize,
            None if delta >= 0 => 0,
            None => len - 1,
        };
        let Some(entry) = view.get(next) else {
            return;
        };
        if extend {
            let anchor = self.state.anchor.or(current).unwrap_or(next);
            let (lo, hi) = (anchor.min(next), anchor.max(next));
            self.state.selected.clear();
            for i in lo..=hi {
                if let Some(e) = view.get(i) {
                    self.state.selected.insert(e.key);
                }
            }
            self.state.focused = Some(entry.key);
            self.state.anchor = Some(anchor);
        } else {
            self.state.select_only(entry.key, next);
        }
        self.state.scroll_to_focus = true;
    }

    fn focused_entry(&self) -> Option<quickfiles_core::EntryRef> {
        let view = self.core.view();
        view.find(self.state.focused?).and_then(|i| view.get(i))
    }

    fn apply(&mut self, action: Action) {
        #[cfg(debug_assertions)]
        eprintln!("action: {action:?}");
        match action {
            Action::Navigate(path) => self.core.open_folder(path),
            Action::OpenPath { path, is_dir } => {
                if is_dir {
                    self.core.open_folder(path);
                } else if let Err(e) = self.core.open(&path) {
                    self.state.say(format!("Could not open: {e}"));
                }
            }
            Action::Back => {
                self.state.restore_scroll_next = true;
                self.core.back();
            }
            Action::Forward => {
                self.state.restore_scroll_next = true;
                self.core.forward();
            }
            Action::Up => self.core.up(),
            Action::ToggleSidebar => {
                self.state.left_open = !self.state.left_open;
                self.core.set_pref_bool("left_open", self.state.left_open);
            }
            Action::TogglePreview => {
                self.state.preview_open = !self.state.preview_open;
                self.core
                    .set_pref_bool("preview_open", self.state.preview_open);
            }
            Action::FocusSearch => self.state.focus_search = true,
            Action::Refresh => self.core.refresh(),
            Action::SelectAll => {
                let keys = self.core.view().loaded_keys();
                self.state.selected.extend(keys);
            }
            Action::Escape => {
                if self.state.rename.take().is_some() {
                } else if !self.core.nav().text().is_empty() {
                    self.core.escape();
                } else {
                    self.state.clear_selection();
                }
            }
            Action::ClearSearch => {
                self.core.clear();
                self.state.focus_search = true;
            }
            Action::MoveFocus(d) => {
                let d = self.grid_step(d);
                self.step(d, false)
            }
            Action::ExtendFocus(d) => {
                let d = self.grid_step(d);
                self.step(d, true)
            }
            Action::MoveSide(d) => {
                if self.state.grid {
                    self.step(d, false);
                }
            }
            Action::ToggleGrid => {
                let grid = !self.state.grid;
                self.apply(Action::SetGrid(grid));
            }
            Action::TogglePerf => self.state.perf.visible = !self.state.perf.visible,
            Action::ResetPerf => {
                self.state.perf.reset();
                self.core.images().reset_stats();
                self.state.say("Performance counters reset");
            }
            Action::CopyPerf => {
                let context = format!(
                    "{}, {}",
                    if self.state.grid { "grid" } else { "list" },
                    self.core
                        .nav()
                        .location()
                        .map_or_else(|| "search".to_owned(), |p| p.display().to_string())
                );
                let report = self.state.perf.report(
                    &self.core.images().stats(),
                    &self.tex.stats(),
                    &context,
                );
                self.ctx.copy_text(report);
                self.state.say("Performance report copied");
            }
            Action::SetGrid(grid) => {
                if self.state.grid != grid {
                    self.state.grid = grid;
                    self.state.scroll_to = None;
                    self.core.set_pref_bool("grid", grid);
                }
            }
            Action::CopyPath => {
                let mut paths = self.selected_paths();
                if paths.is_empty() {
                    paths.extend(
                        self.core
                            .nav()
                            .location()
                            .filter(|p| !p.as_os_str().is_empty())
                            .cloned(),
                    );
                }
                if !paths.is_empty() {
                    let text: Vec<String> = paths.iter().map(|p| p.display().to_string()).collect();
                    self.ctx.copy_text(text.join("\n"));
                    self.state.say(if paths.len() == 1 {
                        "Path copied".to_owned()
                    } else {
                        format!("{} paths copied", paths.len())
                    });
                }
            }
            Action::NewTab => {
                self.core.new_tab();
                self.state.focus_search = true;
            }
            Action::CloseTab(i) => {
                self.state.restore_scroll_next = true;
                self.core.close_tab(i);
            }
            Action::CloseCurrentTab => {
                self.state.restore_scroll_next = true;
                let i = self.core.active_tab();
                self.core.close_tab(i);
            }
            Action::SwitchTab(i) => {
                self.state.restore_scroll_next = true;
                self.core.switch_tab(i);
            }
            Action::CycleTab(d) => {
                self.state.restore_scroll_next = true;
                self.core.next_tab(d);
            }
            Action::PasteWith(policy) => self.core.paste(policy),
            Action::Transfer {
                mut paths,
                dest,
                copy,
                policy,
            } => {
                if !copy {
                    // Dropping an item into the folder it is already in does nothing.
                    paths.retain(|p| p.parent() != Some(dest.as_path()));
                }
                if paths.is_empty() {
                    return;
                }
                match policy {
                    Some(p) => self.core.transfer(paths, dest, copy, p),
                    None => {
                        let n = self.core.transfer_conflicts(&paths, &dest);
                        if n == 0 {
                            self.core.transfer(paths, dest, copy, Conflict::KeepBoth);
                        } else {
                            self.ask_conflict(n, move |p| Action::Transfer {
                                paths: paths.clone(),
                                dest: dest.clone(),
                                copy,
                                policy: Some(p),
                            });
                        }
                    }
                }
            }
            Action::OpenFocused => match self.focused_entry() {
                Some(e) => self.apply(Action::OpenPath {
                    path: e.path(),
                    is_dir: e.is_dir(),
                }),
                None => self.core.commit_query(),
            },
            Action::ToggleRegex => self.core.toggle_regex(),
            Action::ToggleHidden => self.core.toggle_hidden(),
            Action::ToggleBookmark => {
                // A selected folder wins; otherwise the folder you are in.
                let target = self
                    .focused_entry()
                    .filter(|e| e.kind == Kind::Folder && self.state.selected.contains(&e.key))
                    .map(|e| e.path())
                    .or_else(|| self.core.nav().location().cloned())
                    .filter(|p| !p.as_os_str().is_empty());
                if let Some(p) = target {
                    self.core.toggle_bookmark(&p);
                }
            }
            Action::RemoveBookmark(p) => self.core.remove_bookmark(&p),
            Action::Copy | Action::Cut => {
                let paths = self.selected_paths();
                if !paths.is_empty() {
                    let n = paths.len();
                    if matches!(action, Action::Cut) {
                        self.core.cut(paths);
                    } else {
                        self.core.copy(paths);
                    }
                    self.state.say(format!(
                        "{n} item{} ready to paste",
                        if n == 1 { "" } else { "s" }
                    ));
                }
            }
            Action::Paste => {
                let n = self.core.paste_conflicts();
                if n == 0 {
                    self.core.paste(Conflict::KeepBoth);
                } else {
                    self.ask_conflict(n, Action::PasteWith);
                }
            }
            Action::Delete { permanent } => {
                let paths = self.selected_paths();
                if paths.is_empty() {
                    return;
                }
                if permanent {
                    let n = paths.len();
                    self.state.confirm = paths;
                    let pos = egui::pos2(400.0, 260.0);
                    self.state.menu = Some(
                        MenuState::at(
                            pos,
                            vec![
                                MenuItem::new(
                                    format!(
                                        "Delete {n} item{} permanently",
                                        if n == 1 { "" } else { "s" }
                                    ),
                                    Action::ConfirmDelete,
                                ),
                                MenuItem::new("Cancel", Action::CloseMenu),
                            ],
                        )
                        .with_header("This cannot be undone"),
                    );
                } else {
                    self.core.remove(paths, false);
                }
            }
            Action::ConfirmDelete => {
                let paths = std::mem::take(&mut self.state.confirm);
                self.core.remove(paths, true);
            }
            Action::Rename => {
                if self.state.selected.len() == 1 {
                    if let Some(e) = self.focused_entry().filter(|e| e.kind != Kind::Drive) {
                        self.state.rename = Some(Rename {
                            key: e.key,
                            path: e.path(),
                            text: e.name.to_string(),
                            fresh: true,
                        });
                    }
                }
            }
            Action::NewFolder => match self.core.new_folder() {
                Ok(path) => {
                    let key = key_of_path(&path);
                    self.state.selected = HashSet::from([key]);
                    self.state.focused = Some(key);
                    self.state.scroll_to_focus = true;
                    self.state.rename = Some(Rename {
                        key,
                        text: path
                            .file_name()
                            .map(|n| n.to_string_lossy().into_owned())
                            .unwrap_or_default(),
                        path,
                        fresh: true,
                    });
                }
                Err(e) => self.state.say(format!("Could not create folder: {e}")),
            },
            Action::Reveal => {
                let target = self
                    .selected_paths()
                    .into_iter()
                    .next()
                    .or_else(|| self.core.nav().location().cloned());
                if let Some(p) = target {
                    if let Err(e) = self.core.reveal(&p) {
                        self.state.say(format!("Could not open Explorer: {e}"));
                    }
                }
            }
            Action::ClearHistory => {
                self.core.clear_history();
                self.state.say("History cleared");
            }
            Action::ClearThumbCache => {
                self.core.images().clear_cache();
                self.state.say("Thumbnail cache cleared");
            }
            Action::CancelJob(id) => self.core.cancel_job(id),
            Action::GateStartEverything => self.core.launch_everything(),
            Action::GateDownload => sys::open_url(sys::EVERYTHING_URL),
            Action::GateRetry => self.core.retry(),
            Action::CloseMenu => {
                self.state.menu = None;
                self.state.confirm.clear();
            }
        }
    }

    /// Up/Down move by a whole row of tiles in the grid.
    fn grid_step(&self, d: i32) -> i32 {
        if self.state.grid && d.abs() == 1 {
            d * self.state.grid_cols.max(1) as i32
        } else {
            d
        }
    }

    /// Asks once how to resolve name clashes for a copy / move / paste.
    fn ask_conflict(&mut self, n: usize, make: impl Fn(Conflict) -> Action) {
        let items = vec![
            MenuItem::new("Replace the existing items", make(Conflict::Replace)),
            MenuItem::new("Keep both (rename the new ones)", make(Conflict::KeepBoth)),
            MenuItem::new("Skip the ones that exist", make(Conflict::Skip)),
            MenuItem::separator(),
            MenuItem::new("Cancel", Action::CloseMenu),
        ];
        let header = format!(
            "{n} item{} already exist{}",
            if n == 1 { "" } else { "s" },
            if n == 1 { "s" } else { "" }
        );
        self.state.menu = Some(MenuState::at(egui::pos2(400.0, 260.0), items).with_header(header));
    }

    /// Per-frame housekeeping that must happen before drawing.
    fn sync(&mut self, ctx: &egui::Context) {
        // A new view (query, folder, sort) invalidates the selection.
        let id = self.core.view().id();
        if id != self.state.view_id {
            // Remember where the old view was scrolled; Back/Forward/tab switches restore it.
            let q = self.core.nav().query();
            let new_key = format!("{:?}|{}", q.location, q.text);
            let old = std::mem::replace(&mut self.state.view_key, new_key);
            if !old.is_empty() {
                self.state.scroll_memory.insert(old, self.state.list_scroll);
                if self.state.scroll_memory.len() > 500 {
                    self.state.scroll_memory.clear();
                }
            }
            self.state.scroll_to = if std::mem::take(&mut self.state.restore_scroll_next) {
                self.state.scroll_memory.get(&self.state.view_key).copied()
            } else {
                None
            };
            self.state.list_scroll = 0.0;
            self.state.view_id = id;
            self.state.clear_selection();
            self.state.rename = None;
            self.state.text_preview = None;
        }
        // Navigation (back, x, folder click) rewrites the text; the box follows.
        if self.state.search != self.core.nav().text() {
            self.state.search = self.core.nav().text().to_owned();
        }
        // Save the panel widths once a drag has ended.
        let widths = (self.state.left_w, self.state.preview_w);
        if widths != self.saved_widths && !ctx.input(|i| i.pointer.any_down()) {
            self.saved_widths = widths;
            self.core.set_pref("left_w", &widths.0.to_string());
            self.core.set_pref("preview_w", &widths.1.to_string());
        }
        // Column widths, likewise.
        if self.state.col_w != self.saved_cols && !ctx.input(|i| i.pointer.any_down()) {
            self.saved_cols = self.state.col_w;
            self.core.set_pref("col_w", &self.saved_cols.to_pref());
        }
        // Window size (kept when it is maximised, so un-maximising returns to it).
        if !ctx.input(|i| i.pointer.any_down()) {
            let (inner, maximized) = ctx.input(|i| {
                (
                    i.viewport().inner_rect,
                    i.viewport().maximized.unwrap_or(false),
                )
            });
            let mut ws = self.window;
            ws.maximized = maximized;
            if !maximized {
                if let Some(r) = inner.filter(|r| r.width() >= 300.0 && r.height() >= 200.0) {
                    ws.w = r.width().round();
                    ws.h = r.height().round();
                }
            }
            if ws != self.window {
                self.window = ws;
                sys::save_window(ws);
            }
        }
    }

    /// Toasts for finished file operations.
    fn report_jobs(&mut self) {
        let mut finished = false;
        for job in self.core.jobs() {
            if job.state == JobState::Running || !self.state.reported_jobs.insert(job.id) {
                continue;
            }
            finished = true;
            let text = match job.state {
                JobState::Done => match job.kind {
                    quickfiles_core::jobs::Kind::Copy => "Copied",
                    quickfiles_core::jobs::Kind::Move => "Moved",
                    quickfiles_core::jobs::Kind::Recycle => "Moved to the Recycle Bin",
                    quickfiles_core::jobs::Kind::Delete => "Deleted",
                }
                .to_owned(),
                JobState::Cancelled => "Cancelled".to_owned(),
                JobState::Failed(e) => format!("Failed: {e}"),
                JobState::Running => continue,
            };
            self.state.say(text);
        }
        if finished {
            self.core.prune_jobs();
        }
    }
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        // Timed for the F12 overlay: how long our own UI code takes this frame.
        let frame_start = std::time::Instant::now();
        let interval_ms = ui.input(|i| i.unstable_dt) * 1000.0;
        self.sync(ui.ctx());
        self.report_jobs();

        // Ctrl + wheel switches the view: wheel up = thumbnail grid, wheel down = detail list.
        let wheel: f32 = ui.ctx().input(|i| {
            i.events
                .iter()
                .filter_map(|e| match e {
                    egui::Event::MouseWheel {
                        delta, modifiers, ..
                    } if modifiers.command => Some(delta.y),
                    _ => None,
                })
                .sum()
        });
        if wheel > 0.0 {
            self.apply(Action::SetGrid(true));
        } else if wheel < 0.0 {
            self.apply(Action::SetGrid(false));
        }

        // Files dropped from Explorer are copied into the current folder.
        let dropped = ui::dropped(ui.ctx());
        if !dropped.is_empty() {
            match self
                .core
                .nav()
                .location()
                .filter(|p| !p.as_os_str().is_empty())
                .cloned()
            {
                Some(dest) => self.apply(Action::Transfer {
                    paths: dropped,
                    dest,
                    copy: true,
                    policy: None,
                }),
                None => self.state.say("Open a folder first, then drop the files"),
            }
        }

        // Hotkeys are applied before drawing, so their effect shows in this very frame.
        let mut actions = Vec::new();
        self.hotkeys
            .poll(ui.ctx(), self.state.menu.is_some(), &mut actions);
        for action in actions.drain(..) {
            self.apply(action);
        }

        egui::CentralPanel::default()
            .frame(egui::Frame::NONE)
            .show(ui, |panel_ui| {
                let mut cx = Cx {
                    theme: &self.theme,
                    state: &mut self.state,
                    core: &mut self.core,
                    tex: &mut self.tex,
                    actions: &mut actions,
                };
                ui::draw(panel_ui, &mut cx);
            });

        if !actions.is_empty() {
            // Actions change state after this frame was drawn; draw the result right away.
            ui.ctx().request_repaint();
        }
        for action in actions {
            self.apply(action);
        }
        let cpu_ms = frame_start.elapsed().as_secs_f32() * 1000.0;
        self.state
            .perf
            .record(cpu_ms, interval_ms, &self.tex.stats());
    }
}
