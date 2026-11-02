//! All mutable UI state, free of any drawing code.

use std::{collections::HashSet, path::PathBuf, sync::Arc};

use crate::{
    data,
    model::{Listing, Sort, SortDir, SortKey},
    theme::Metrics,
};

pub const SIDEBAR_MIN_W: f32 = 180.0;
pub const SIDEBAR_MAX_W: f32 = 480.0;
pub const PREVIEW_MIN_W: f32 = 220.0;
pub const PREVIEW_MAX_W: f32 = 560.0;

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
    pub scroll_to_focus: bool,
    /// Vertical scroll offset of the file list as of the last frame.
    pub list_scroll: f32,

    // Navigation
    /// Current folder. Empty = the virtual root "This PC".
    pub path: PathBuf,
    pub back: Vec<PathBuf>,
    pub forward: Vec<PathBuf>,

    // Listing
    pub listing: Option<Arc<Listing>>,
    pub request_id: u64,
    pub loading: bool,
    pub sort: Sort,
    pub filter: String,
    /// Indices into `listing.entries`, already filtered and sorted. Rebuilt only on change.
    pub view: Vec<u32>,
    pub selected: HashSet<u32>,
    pub focused: Option<u32>,
}

impl State {
    pub fn new() -> Self {
        let path = data::default_path();
        Self {
            left_open: true,
            left_w: Metrics::DEFAULT.sidebar_w,
            preview_open: false,
            preview_w: Metrics::DEFAULT.preview_w,
            section_open: [true; 4],
            focus_search: false,
            scroll_to_focus: false,
            list_scroll: 0.0,
            path,
            back: Vec::new(),
            forward: Vec::new(),
            listing: None,
            request_id: 0,
            loading: false,
            sort: Sort {
                key: SortKey::Name,
                dir: SortDir::Asc,
            },
            filter: String::new(),
            view: Vec::new(),
            selected: HashSet::new(),
            focused: None,
        }
    }

    /// The active item, if it is a selected *file*: this is what the preview panel shows.
    /// Folders never get a preview.
    pub fn preview_index(&self) -> Option<u32> {
        let idx = self.focused?;
        if !self.selected.contains(&idx) {
            return None;
        }
        let listing = self.listing.as_ref()?;
        let entry = listing.entries.get(idx as usize)?;
        (!entry.is_dir()).then_some(idx)
    }

    /// Moves the active item up/down in the visible list and selects it.
    pub fn move_focus(&mut self, delta: i32) {
        let len = self.view.len();
        if len == 0 {
            return;
        }
        let current = self
            .focused
            .and_then(|f| self.view.iter().position(|&i| i == f));
        let next = match current {
            Some(pos) => (pos as i64 + i64::from(delta)).clamp(0, len as i64 - 1) as usize,
            None if delta >= 0 => 0,
            None => len - 1,
        };
        let idx = self.view[next];
        self.focused = Some(idx);
        self.selected.clear();
        self.selected.insert(idx);
        self.scroll_to_focus = true;
    }

    pub fn set_listing(&mut self, listing: Arc<Listing>) {
        self.listing = Some(listing);
        self.loading = false;
        self.selected.clear();
        self.rebuild_view();
        self.focused = self.view.first().copied();
    }

    pub fn toggle_sort(&mut self, key: SortKey) {
        self.sort.toggle(key);
        self.rebuild_view();
    }

    /// Re-filter and re-sort. Called only when the listing, filter or sort changes - never per frame.
    pub fn rebuild_view(&mut self) {
        self.view.clear();
        let Some(listing) = self.listing.clone() else {
            return;
        };
        let entries = &listing.entries;
        let needle = self.filter.to_lowercase();

        self.view.extend(
            (0..entries.len() as u32)
                .filter(|&i| needle.is_empty() || entries[i as usize].key.contains(&needle)),
        );

        let (key, dir) = (self.sort.key, self.sort.dir);
        self.view.sort_by(|&a, &b| {
            let (a, b) = (&entries[a as usize], &entries[b as usize]);
            let inner = match key {
                SortKey::Name => a.key.cmp(&b.key),
                SortKey::Type => a
                    .type_label
                    .cmp(b.type_label)
                    .then_with(|| a.key.cmp(&b.key)),
                SortKey::Modified => a.modified.cmp(&b.modified),
                SortKey::Size => a.size_bytes.cmp(&b.size_bytes),
            };
            let inner = if dir == SortDir::Desc {
                inner.reverse()
            } else {
                inner
            };
            // Folders always first.
            b.is_dir().cmp(&a.is_dir()).then(inner)
        });
    }
}
