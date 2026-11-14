//! Glue: owns the theme, the UI state and the core; turns UI `Action`s into core calls.

use std::{collections::HashSet, sync::Arc};

use eframe::egui;
use quickfiles_core::{
    Config, Conflict, Core, CoreError, JobState, Kind,
    dropdata::Dropped,
    global_hotkey::{Combo, GlobalHotkey},
    key_of_path,
    ole_drag::Outcome,
    ole_drop::DropTarget,
    sys,
    sys::WindowState,
    tray::Tray,
};

use crate::{
    config,
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
    global: GlobalHotkey,
    /// Notification-area icon: closing the window hides it there instead of quitting.
    tray: Tray,
    /// Receives files and browser images dragged onto the window (`None`: egui's own events).
    drop_target: Option<DropTarget>,
    saved_widths: (f32, f32),
    saved_cols: crate::state::ColW,
    window: WindowState,
    /// When the window size last changed and is not saved yet. Saving waits until the size has
    /// been still for a moment: a drag-resize changes it every frame, and a file write per frame
    /// froze the window.
    window_dirty: Option<std::time::Instant>,
    ctx: egui::Context,
}

/// Everything read from `config.conf`.
struct Loaded {
    hotkeys: Hotkeys,
    theme: Theme,
    global: Option<Combo>,
    problems: Vec<String>,
}

/// Default for `[general] global_hotkey`.
const GLOBAL_DEFAULT: &str = "Ctrl+Alt+Space";

/// Reads `config.conf` (writing a commented one the first time). Never fails: a bad line is
/// skipped and listed in `problems`.
fn load_config() -> Loaded {
    if sys::read_config(config::FILE).is_none() {
        // Keep bindings from the old hotkeys.conf (active lines only).
        let legacy: Vec<String> = sys::read_config("hotkeys.conf")
            .unwrap_or_default()
            .lines()
            .map(str::trim)
            .filter(|l| !l.is_empty() && !l.starts_with('#'))
            .map(str::to_owned)
            .collect();
        let theme: Vec<(&str, String)> = Theme::dark()
            .palette
            .entries()
            .into_iter()
            .map(|(n, c)| (n, config::color_text(c.to_srgba_unmultiplied())))
            .collect();
        sys::write_config_if_missing(
            config::FILE,
            &config::template(
                GLOBAL_DEFAULT,
                &theme,
                crate::hotkeys::DEFAULTS,
                &crate::hotkeys::action_names(),
                &legacy,
            ),
        );
    }
    let sections = config::parse(&sys::read_config(config::FILE).unwrap_or_default());
    let mut problems = sections.errors.clone();

    let (hotkeys, p) = Hotkeys::from_config(&sections.hotkeys);
    problems.extend(p);
    let (theme, p) = Theme::configured(&sections.theme, &sections.themes);
    problems.extend(p);

    let mut global = Combo::parse(GLOBAL_DEFAULT).ok();
    for e in &sections.general {
        match e.key.as_str() {
            "global_hotkey" => {
                if matches!(e.value.to_ascii_lowercase().as_str(), "none" | "off" | "") {
                    global = None;
                } else {
                    match Combo::parse(&e.value) {
                        Ok(c) => global = Some(c),
                        Err(err) => {
                            problems.push(format!("config line {}: global_hotkey: {err}", e.line))
                        }
                    }
                }
            }
            other => problems.push(format!(
                "config line {}: unknown setting \"{other}\"",
                e.line
            )),
        }
    }
    Loaded {
        hotkeys,
        theme,
        global,
        problems,
    }
}

impl App {
    /// Ctrl+Shift+R: apply `config.conf` again without restarting.
    fn reload_config(&mut self) {
        let l = load_config();
        self.theme = l.theme;
        self.theme.restyle(&self.ctx);
        self.hotkeys = l.hotkeys;
        self.state.help_rows = self.hotkeys.help_rows();
        self.global.set(l.global);
        match l.problems.first() {
            None => self.state.say("Config reloaded"),
            Some(first) => self.state.say(format!(
                "Config reloaded with {} problem(s): {first}",
                l.problems.len()
            )),
        }
    }

    pub fn new(
        cc: &eframe::CreationContext<'_>,
        dummy: bool,
        start: Option<std::path::PathBuf>,
    ) -> Self {
        let loaded = load_config();
        let theme = loaded.theme;
        theme.apply(&cc.egui_ctx);

        // Background threads wake the UI through this; the UI never polls.
        let ctx = cc.egui_ctx.clone();
        let wake: quickfiles_core::Wake = Arc::new(move || ctx.request_repaint());
        let drop_target = DropTarget::register("quickfiles", wake.clone());
        let global = GlobalHotkey::start("quickfiles", wake.clone());
        global.set(loaded.global);
        let tray = Tray::start("quickfiles", wake.clone());
        let mut core = Core::start(Config {
            dummy,
            data_dir: None,
            wake,
        });
        // A folder given on the command line wins over the one restored from last time.
        if let Some(dir) = start {
            core.open_folder(dir);
        }
        let state = State::new(&core.data());
        let widths = (state.left_w, state.preview_w);

        let hotkeys = loaded.hotkeys;
        let mut state = state;
        state.help_rows = hotkeys.help_rows();
        if let Some(first) = loaded.problems.first() {
            state.say(first.clone());
        }
        let saved_cols = state.col_w;
        Self {
            theme,
            state,
            core,
            tex: Tex::new(),
            hotkeys,
            global,
            tray,
            drop_target,
            saved_widths: widths,
            saved_cols,
            window_dirty: None,
            window: sys::load_window().unwrap_or(WindowState {
                w: 1152.0,
                h: 760.0,
                maximized: false,
            }),
            ctx: cc.egui_ctx.clone(),
        }
    }

    /// Where a drop from another program lands: the selected folder, else the open folder.
    fn drop_destination(&self) -> Option<std::path::PathBuf> {
        if let Some(e) = self.focused_entry()
            && e.is_dir()
            && self.state.selected.contains(&e.key)
        {
            return Some(e.path());
        }
        self.core
            .nav()
            .location()
            .filter(|p| !p.as_os_str().is_empty())
            .cloned()
    }

    /// Files are copied, an image address is downloaded; both run as jobs with progress.
    fn handle_drop(&mut self, dropped: Dropped) {
        // The folder the pointer was over when released wins over the selection.
        let over = self.state.drop_over.clone();
        let Some(dest) = over.or_else(|| self.drop_destination()) else {
            self.state.say("Open a folder first, then drop");
            return;
        };
        let name = quickfiles_core::display_name(&dest);
        match dropped {
            Dropped::Files(paths) => {
                let (found, gone): (Vec<_>, Vec<_>) = paths.into_iter().partition(|p| p.exists());
                if found.is_empty() {
                    self.state.say("Those files are not available any more");
                    return;
                }
                if !gone.is_empty() {
                    self.state.say(format!(
                        "{} item(s) could not be read and were skipped",
                        gone.len()
                    ));
                }
                self.apply(Action::Transfer {
                    paths: found,
                    dest,
                    copy: true,
                    policy: None,
                });
            }
            Dropped::Image(source) => {
                self.state.say(format!("Downloading the image into {name}"));
                self.core.download_images(vec![source], dest);
            }
            Dropped::Unsupported(why) => self.state.say(format!("Cannot drop that: {why}")),
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

    /// Selects a just-created file or folder and opens its name for editing. The new item is
    /// never entered or opened.
    fn start_rename_of_new(&mut self, made: Result<std::path::PathBuf, CoreError>, what: &str) {
        match made {
            Ok(path) => {
                let key = key_of_path(&path);
                self.state.selected = HashSet::from([key]);
                self.state.focused = Some(key);
                self.state.scroll_to_focus = true;
                let text = path
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default();
                self.state.rename = Some(Rename {
                    key,
                    path,
                    text,
                    fresh: true,
                });
            }
            Err(e) => self.state.say(format!("Could not create {what}: {e}")),
        }
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
            Action::ReloadConfig => self.reload_config(),
            Action::ToggleHelp => self.state.help_open = !self.state.help_open,
            Action::Escape => {
                if self.state.help_open {
                    self.state.help_open = false;
                } else if self.state.rename.take().is_some() {
                } else if !self.core.nav().text().is_empty() {
                    self.core.escape();
                } else {
                    self.state.clear_selection();
                }
            }
            Action::EditPath => {
                let current = self
                    .core
                    .nav()
                    .location()
                    .map(|p| p.to_string_lossy().into_owned())
                    .unwrap_or_default();
                self.state.path_edit = Some(current);
                self.state.path_suggest_for.clear();
                self.state.path_focus = true;
            }
            Action::GoToPath(text) => {
                self.state.path_edit = None;
                match quickfiles_core::pathinput::resolve(&text) {
                    Some(dir) => self.core.open_folder(dir),
                    None => self.state.say(format!("No such folder: {}", text.trim())),
                }
            }
            Action::CloseWindow => {
                if self.tray.active() {
                    self.tray.hide_window();
                } else {
                    self.ctx.send_viewport_cmd(egui::ViewportCommand::Close);
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
                    self.state.say(format!(
                        "Already in {}",
                        quickfiles_core::display_name(&dest)
                    ));
                    return;
                }
                if policy.is_none() {
                    self.state.say(format!(
                        "{} {} item{} to {}",
                        if copy { "Copying" } else { "Moving" },
                        paths.len(),
                        if paths.len() == 1 { "" } else { "s" },
                        quickfiles_core::display_name(&dest)
                    ));
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
            Action::DragDrop { paths, dest, copy } => {
                if copy {
                    self.apply(Action::Transfer {
                        paths,
                        dest,
                        copy,
                        policy: None,
                    });
                    return;
                }
                // Dropping an item into the folder it is already in does nothing.
                let paths: Vec<_> = paths
                    .into_iter()
                    .filter(|p| p.parent() != Some(dest.as_path()))
                    .collect();
                if paths.is_empty() {
                    self.state.say(format!(
                        "Already in {}",
                        quickfiles_core::display_name(&dest)
                    ));
                    return;
                }
                self.ask_move(paths, dest);
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
            Action::NewFolder => {
                let made = self.core.new_folder();
                self.start_rename_of_new(made, "folder");
            }
            Action::NewFile => {
                let made = self.core.new_file();
                self.start_rename_of_new(made, "file");
            }
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

    /// Asks before a drag-and-drop move: "Move", "Copy instead" or "Cancel".
    fn ask_move(&mut self, paths: Vec<std::path::PathBuf>, dest: std::path::PathBuf) {
        let n = paths.len();
        let what = if n == 1 {
            paths[0]
                .file_name()
                .map(|f| format!("\"{}\"", f.to_string_lossy()))
                .unwrap_or_else(|| "1 item".to_owned())
        } else {
            format!("{n} items")
        };
        let header = format!("Move {what} to {}?", quickfiles_core::display_name(&dest));
        let go = |copy| Action::Transfer {
            paths: paths.clone(),
            dest: dest.clone(),
            copy,
            policy: None,
        };
        let items = vec![
            MenuItem::new("Move", go(false)),
            MenuItem::new("Copy instead", go(true)),
            MenuItem::separator(),
            MenuItem::new("Cancel", Action::CloseMenu),
        ];
        self.state.menu = Some(MenuState::at(egui::pos2(400.0, 260.0), items).with_header(header));
    }

    /// Asks once how to resolve name clashes for a copy / move / paste.
    fn ask_conflict(&mut self, n: usize, make: impl Fn(Conflict) -> Action) {
        let items = vec![
            MenuItem::new(
                "Replace files that exist (folders are merged)",
                make(Conflict::Replace),
            ),
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
                self.window_dirty = Some(std::time::Instant::now());
            }
        }
        if let Some(t) = self.window_dirty {
            const QUIET: std::time::Duration = std::time::Duration::from_millis(400);
            if t.elapsed() >= QUIET {
                self.window_dirty = None;
                sys::save_window(self.window);
            } else {
                ctx.request_repaint_after(QUIET);
            }
        }
    }

    /// A drag that started on a row and has left the window becomes a real drag-and-drop: other
    /// programs (Explorer, a browser, a chat) can take the files. Blocks until the button is let go.
    fn drag_out(&mut self, ctx: &egui::Context) {
        let Some(drag) = self.state.drag.as_ref() else {
            return;
        };
        // Read the window rect first: a nested `ctx.input` inside `ctx.input` can deadlock when
        // another thread (a repaint request) is waiting for the context's lock.
        let window = ctx.content_rect();
        let outside = ctx.input(|i| {
            i.pointer.primary_down() && i.pointer.latest_pos().is_none_or(|p| !window.contains(p))
        });
        if !outside {
            return;
        }
        let paths = drag.paths.clone();
        self.state.drag = None;
        match quickfiles_core::ole_drag::start(&paths) {
            Ok(Outcome::Cancelled) => {}
            Ok(_) => self.state.say("Dragged out"),
            Err(e) => self.state.say(format!("Drag failed: {e}")),
        }
        // The release happened inside the drag loop, so egui never saw it.
        ctx.request_repaint();
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
                    quickfiles_core::jobs::Kind::Copy => "Copied".to_owned(),
                    quickfiles_core::jobs::Kind::Move => "Moved".to_owned(),
                    quickfiles_core::jobs::Kind::Recycle => "Moved to the Recycle Bin".to_owned(),
                    quickfiles_core::jobs::Kind::Delete => "Deleted".to_owned(),
                    quickfiles_core::jobs::Kind::Download => format!("Downloaded {}", job.current),
                },
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
        // The window's X (or Alt+F4) hides it to the tray; Exit in the tray menu really quits.
        if self.tray.active()
            && !self.tray.quitting()
            && ui.ctx().input(|i| i.viewport().close_requested())
        {
            ui.ctx()
                .send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.tray.hide_window();
        }
        for message in self.global.take_messages() {
            self.state.say(message);
        }
        self.report_jobs();
        self.drag_out(ui.ctx());
        // Another launch (`qf.exe C:\x`) asked this window to open a folder.
        if let Some(dir) = quickfiles_core::single_instance::take_request() {
            self.core.open_folder(dir);
        }

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

        // Things dragged in from other programs (Explorer files, browser images). The OLE target
        // sees both; egui's own events cover the case where it could not be registered.
        let mut drops = Vec::new();
        match &self.drop_target {
            Some(t) => {
                drops.extend(t.take_drops());
                self.state.drop_hover = t.hover();
                let ppp = ui.ctx().pixels_per_point();
                self.state.drop_pos = t
                    .hover_pos()
                    .map(|(x, y)| egui::pos2(x as f32 / ppp, y as f32 / ppp));
            }
            None => {
                let files = ui::dropped(ui.ctx());
                if !files.is_empty() {
                    drops.push(Dropped::Files(files));
                }
                self.state.drop_hover = ui
                    .ctx()
                    .input(|i| !i.raw.hovered_files.is_empty())
                    .then_some(quickfiles_core::ole_drop::Hover::Files(
                        ui.ctx().input(|i| i.raw.hovered_files.len()),
                    ));
            }
        }
        self.state.drop_dest = self
            .state
            .drop_hover
            .as_ref()
            .and_then(|_| self.drop_destination())
            .map(|d| quickfiles_core::display_name(&d));
        for dropped in drops {
            self.handle_drop(dropped);
        }
        // Drawing sets it again for the folder under the pointer.
        self.state.drop_over = None;
        self.state.tab_hits.clear();

        // Hotkeys are applied before drawing, so their effect shows in this very frame.
        let mut actions = Vec::new();
        self.hotkeys
            .poll(ui.ctx(), self.state.menu.is_some(), &mut actions);
        self.state.hints = self.hotkeys.hints();
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
