//! All mutable UI state, free of drawing code. File data itself lives in `quickfiles-core`.

use std::{
    collections::{HashMap, HashSet},
    path::PathBuf,
    time::Instant,
};

use std::time::Duration;

use quickfiles_core::{Debounce, ScrollGate, Snapshot};

use crate::{theme::Metrics, ui::MenuState};

pub const SIDEBAR_MIN_W: f32 = 180.0;
pub const SIDEBAR_MAX_W: f32 = 480.0;
pub const PREVIEW_MIN_W: f32 = 220.0;
pub const PREVIEW_MAX_W: f32 = 560.0;
/// How long the selection must stay put before the preview image is decoded.
pub const PREVIEW_DEBOUNCE_MS: u64 = 120;

/// Inline rename of one row (F2).
pub struct Rename {
    pub key: u64,
    pub path: PathBuf,
    pub text: String,
    /// First frame: grab focus and select the stem.
    pub fresh: bool,
}

/// Widths of the right-anchored list columns (the Name column takes the rest).
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct ColW {
    pub ty: f32,
    pub date: f32,
    pub folders: f32,
    pub files: f32,
    pub size: f32,
}

impl ColW {
    pub const MIN: f32 = 56.0;
    pub const MAX: f32 = 420.0;

    pub fn from_metrics(m: &Metrics) -> Self {
        Self {
            ty: m.col_type - m.col_date,
            date: m.col_date - m.col_folders,
            folders: m.col_folders - m.col_files,
            files: m.col_files - m.col_size,
            size: m.col_size,
        }
    }

    pub fn to_pref(self) -> String {
        format!(
            "{},{},{},{},{}",
            self.ty, self.date, self.folders, self.files, self.size
        )
    }

    pub fn from_pref(text: &str) -> Option<Self> {
        let v: Vec<f32> = text
            .split(',')
            .filter_map(|p| p.trim().parse().ok())
            .collect();
        let [ty, date, folders, files, size] = v[..] else {
            return None;
        };
        let c = |x: f32| x.clamp(Self::MIN, Self::MAX);
        Some(Self {
            ty: c(ty),
            date: c(date),
            folders: c(folders),
            files: c(files),
            size: c(size),
        })
    }
}

/// Files being dragged inside the app (drop on a folder to move, Ctrl to copy).
#[derive(Clone)]
pub struct Drag {
    pub paths: Vec<PathBuf>,
    pub label: String,
}

pub struct State {
    // Window chrome
    pub left_open: bool,
    pub left_w: f32,
    pub preview_open: bool,
    pub preview_w: f32,
    /// Recents, Bookmarks, Storage, Places
    pub section_open: [bool; 4],
    /// One-shot requests set by hotkeys, consumed by the widget they concern.
    pub focus_search: bool,
    /// The address bar while it is being edited (the typed text), else `None`.
    pub path_edit: Option<String>,
    /// Focus and select the address bar text on the next frame.
    pub path_focus: bool,
    /// Folders offered while typing a path, for the text in `path_suggest_for`.
    pub path_suggest: Vec<PathBuf>,
    pub path_suggest_for: String,
    /// Whether that text names an existing folder (the bar turns red when not).
    pub path_valid: bool,
    /// Highlighted suggestion (arrow keys); `None` until the user moves in the list.
    pub path_sel: Option<usize>,
    /// Where the suggestion list was drawn last frame (a click there must not close the editor).
    pub path_list_rect: Option<eframe::egui::Rect>,
    pub scroll_to_focus: bool,
    /// Vertical scroll offset of the file list as of the last frame.
    pub list_scroll: f32,
    /// Scroll speed tracking: icons only while the scrollbar is being dragged through thousands
    /// of files (`PLAN-PERF.md` phase 5), and which view it belongs to.
    pub scroll_gate: ScrollGate,
    pub scroll_gate_view: u64,
    /// The preview is requested only once the selection has been steady for a moment.
    pub preview_debounce: Debounce<u64>,

    // Search box text (mirrors `core.nav().text()`).
    pub search: String,

    // Selection (by stable row key, so it survives refreshes).
    pub selected: HashSet<u64>,
    pub focused: Option<u64>,
    /// View index of the last plain click, for Shift+click ranges.
    pub anchor: Option<usize>,
    /// View id the selection belongs to; a new view clears it.
    pub view_id: u64,

    pub rename: Option<Rename>,
    /// Paths waiting for the permanent-delete confirmation.
    pub confirm: Vec<PathBuf>,
    pub menu: Option<MenuState>,
    pub toast: Option<(String, Instant)>,
    /// Text preview cache: (row key, lines).
    pub text_preview: Option<(u64, Vec<String>)>,
    /// Jobs already reported with a toast.
    pub reported_jobs: HashSet<u64>,

    pub col_w: ColW,
    /// Thumbnail grid instead of the detail list.
    pub grid: bool,
    /// Columns in the grid as of the last frame (arrow keys move by rows).
    pub grid_cols: usize,
    pub drag: Option<Drag>,
    /// Identity of the current view's query, and the scroll offset left behind per query.
    pub view_key: String,
    pub scroll_memory: HashMap<String, f32>,
    /// Set by Back/Forward/tab switch: the next new view restores its remembered scroll.
    pub restore_scroll_next: bool,
    /// One-shot scroll offset to apply to the list.
    pub scroll_to: Option<f32>,
    pub perf: crate::ui::Perf,
    /// Which-key panel (bottom left): set each frame while a key sequence is half typed.
    pub hints: Option<crate::hotkeys::Hints>,
    /// `?` panel (bottom right) and the rows it lists.
    pub help_open: bool,
    pub help_rows: Vec<crate::hotkeys::HelpRow>,
    /// What another program is dragging over the window (from the OLE drop target), and the
    /// name of the folder it would land in.
    pub drop_hover: Option<quickfiles_core::ole_drop::Hover>,
    pub drop_dest: Option<String>,
    /// Pointer position (egui points) of that drag, when known.
    pub drop_pos: Option<eframe::egui::Pos2>,
    /// The folder under the pointer during the last frame of such a drag; set by `dnd::target`.
    /// A drop goes there; with none, to the selected folder or the open one.
    pub drop_over: Option<PathBuf>,
}

impl State {
    pub fn new(saved: &Snapshot) -> Self {
        let m = Metrics::DEFAULT;
        Self {
            left_open: saved.pref_bool("left_open", true),
            left_w: saved
                .pref_f32("left_w")
                .unwrap_or(m.sidebar_w)
                .clamp(SIDEBAR_MIN_W, SIDEBAR_MAX_W),
            preview_open: saved.pref_bool("preview_open", false),
            preview_w: saved
                .pref_f32("preview_w")
                .unwrap_or(m.preview_w)
                .clamp(PREVIEW_MIN_W, PREVIEW_MAX_W),
            section_open: [true; 4],
            focus_search: true,
            path_edit: None,
            path_focus: false,
            path_suggest: Vec::new(),
            path_suggest_for: String::new(),
            path_valid: true,
            path_sel: None,
            path_list_rect: None,
            scroll_to_focus: false,
            list_scroll: 0.0,
            scroll_gate: ScrollGate::default(),
            scroll_gate_view: 0,
            preview_debounce: Debounce::new(Duration::from_millis(PREVIEW_DEBOUNCE_MS)),
            search: String::new(),
            selected: HashSet::new(),
            focused: None,
            anchor: None,
            view_id: 0,
            rename: None,
            confirm: Vec::new(),
            menu: None,
            toast: None,
            text_preview: None,
            reported_jobs: HashSet::new(),
            col_w: saved
                .pref("col_w")
                .and_then(ColW::from_pref)
                .unwrap_or_else(|| ColW::from_metrics(&m)),
            grid: saved.pref_bool("grid", false),
            grid_cols: 1,
            drag: None,
            view_key: String::new(),
            scroll_memory: HashMap::new(),
            restore_scroll_next: false,
            scroll_to: None,
            perf: crate::ui::Perf::default(),
            hints: None,
            help_open: false,
            help_rows: Vec::new(),
            drop_hover: None,
            drop_dest: None,
            drop_pos: None,
            drop_over: None,
        }
    }

    pub fn say(&mut self, text: impl Into<String>) {
        self.toast = Some((text.into(), Instant::now()));
    }

    pub fn clear_selection(&mut self) {
        self.selected.clear();
        self.focused = None;
        self.anchor = None;
    }

    pub fn select_only(&mut self, key: u64, index: usize) {
        self.selected.clear();
        self.selected.insert(key);
        self.focused = Some(key);
        self.anchor = Some(index);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn col_widths_round_trip_and_clamp() {
        let c = ColW::from_metrics(&Metrics::DEFAULT);
        assert_eq!(ColW::from_pref(&c.to_pref()), Some(c));
        assert_eq!(ColW::from_pref("1,2,3"), None);
        let wide = ColW::from_pref("1,9999,100,100,100").unwrap();
        assert_eq!((wide.ty, wide.date), (ColW::MIN, ColW::MAX));
    }
}
