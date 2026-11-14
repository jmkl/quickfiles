//! The windowed result set the UI holds. It owns no rows itself: it points at pages that the
//! engine thread keeps in a small cache around the viewport.
//!
//! The UI calls [`View::want`] with the visible range every frame (cheap: it only talks to the
//! engine when the range changes) and reads rows with [`View::get`]. A `None` row means "page
//! not loaded yet": draw a placeholder; the engine is already fetching it.

use std::{
    collections::{HashMap, HashSet},
    ops::{Deref, Range},
    path::PathBuf,
    sync::{Arc, Mutex, MutexGuard, mpsc::Sender},
    time::Duration,
};

use crate::{
    engine::Command,
    model::{Entry, Query},
    paging::{Layout, PageId},
};

/// Pages kept per view (512 rows each); the ones farthest from the viewport go first.
pub const MAX_PAGES: usize = 64;

#[derive(Debug)]
pub struct Page {
    pub entries: Vec<Entry>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EmptyReason {
    /// Search mode with nothing typed yet.
    NoQuery,
    NoResults,
    /// The folder only contains hidden items (and they are hidden).
    OnlyHidden,
    /// The folder has items but Everything does not index it.
    NotIndexed,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ViewState {
    Loading,
    Ready,
    Empty(EmptyReason),
    Error(String),
}

/// Folder child counts, valid for one revision of the listing.
#[derive(Clone, Copy, Debug)]
pub struct ChildCount {
    pub folders: u32,
    pub files: u32,
    pub rev: u64,
}

pub(crate) struct Inner {
    pub state: ViewState,
    pub layout: Layout,
    pub pages: HashMap<PageId, Arc<Page>>,
    /// Visible range as last reported by the UI.
    pub wanted: Range<usize>,
    pub took_us: u64,
    /// Bumped whenever the content is replaced.
    pub revision: u64,
    pub counts: HashMap<u64, ChildCount>,
    /// (entry key, revision) pairs a count was already attempted for.
    pub tried: HashSet<(u64, u64)>,
}

impl Inner {
    fn new() -> Self {
        Self {
            state: ViewState::Loading,
            layout: Layout::default(),
            pages: HashMap::new(),
            wanted: 0..0,
            took_us: 0,
            revision: 0,
            counts: HashMap::new(),
            tried: HashSet::new(),
        }
    }

    /// Replace everything atomically (open or refresh): readers never see a half-built list.
    pub fn replace(&mut self, layout: Layout, pages: Vec<(PageId, Vec<Entry>)>, state: ViewState) {
        self.layout = layout;
        self.pages.clear();
        for (id, entries) in pages {
            self.pages.insert(id, Arc::new(Page { entries }));
        }
        self.state = state;
        self.revision += 1;
    }

    pub fn reset_empty(&mut self, reason: EmptyReason) {
        self.replace(Layout::default(), Vec::new(), ViewState::Empty(reason));
    }

    pub fn insert_page(&mut self, id: PageId, entries: Vec<Entry>) {
        self.pages.insert(id, Arc::new(Page { entries }));
        while self.pages.len() > MAX_PAGES {
            let target = self.wanted.start;
            let victim = self
                .pages
                .keys()
                .copied()
                .max_by_key(|id| self.layout.base(*id).abs_diff(target));
            match victim {
                Some(v) => {
                    self.pages.remove(&v);
                }
                None => break,
            }
        }
    }
}

pub(crate) struct Shared {
    pub id: u64,
    inner: Mutex<Inner>,
}

impl Shared {
    pub fn new(id: u64) -> Arc<Self> {
        Arc::new(Self {
            id,
            inner: Mutex::new(Inner::new()),
        })
    }

    pub fn lock(&self) -> MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }
}

/// Cheap handle to one row: keeps its page alive, derefs to the [`Entry`].
#[derive(Clone)]
pub struct EntryRef {
    page: Arc<Page>,
    row: usize,
}

impl Deref for EntryRef {
    type Target = Entry;
    fn deref(&self) -> &Entry {
        &self.page.entries[self.row]
    }
}

#[derive(Clone)]
pub struct View {
    shared: Arc<Shared>,
    tx: Sender<Command>,
    query: Arc<Query>,
}

impl View {
    pub(crate) fn new(shared: Arc<Shared>, tx: Sender<Command>, query: Query) -> Self {
        Self {
            shared,
            tx,
            query: Arc::new(query),
        }
    }

    #[cfg(test)]
    pub(crate) fn shared(&self) -> &Arc<Shared> {
        &self.shared
    }

    pub fn id(&self) -> u64 {
        self.shared.id
    }

    pub fn query(&self) -> &Query {
        &self.query
    }

    pub fn len(&self) -> usize {
        self.shared.lock().layout.total()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn state(&self) -> ViewState {
        self.shared.lock().state.clone()
    }

    /// Time the last (re)load took.
    pub fn took(&self) -> Duration {
        Duration::from_micros(self.shared.lock().took_us)
    }

    /// Changes whenever the content is replaced (open, refresh).
    pub fn revision(&self) -> u64 {
        self.shared.lock().revision
    }

    /// Row `i`, or `None` while its page is still loading.
    pub fn get(&self, i: usize) -> Option<EntryRef> {
        let g = self.shared.lock();
        let (id, row) = g.layout.locate(i)?;
        let page = g.pages.get(&id)?;
        (row < page.entries.len()).then(|| EntryRef {
            page: page.clone(),
            row,
        })
    }

    /// Tell the engine which rows are on screen. Call every frame; it is a no-op unless the
    /// range changed.
    pub fn want(&self, range: Range<usize>) {
        {
            let mut g = self.shared.lock();
            if g.wanted == range {
                return;
            }
            g.wanted = range;
        }
        let _ = self.tx.send(Command::Want);
    }

    /// Whether every row is loaded (small folders are loaded completely in the background).
    pub fn is_complete(&self) -> bool {
        let g = self.shared.lock();
        g.state == ViewState::Ready && g.pages.len() >= g.layout.page_count()
    }

    /// Keys of all loaded rows, in view order.
    pub fn loaded_keys(&self) -> Vec<u64> {
        let g = self.shared.lock();
        let mut keys = Vec::new();
        for id in g.layout.all_pages() {
            if let Some(page) = g.pages.get(&id) {
                keys.extend(page.entries.iter().map(|e| e.key));
            }
        }
        keys
    }

    /// Paths of the loaded rows whose key is in `keys`, in view order (one pass).
    pub fn paths_of(&self, keys: &HashSet<u64>) -> Vec<PathBuf> {
        let g = self.shared.lock();
        let mut out = Vec::new();
        for id in g.layout.all_pages() {
            if let Some(page) = g.pages.get(&id) {
                out.extend(
                    page.entries
                        .iter()
                        .filter(|e| keys.contains(&e.key))
                        .map(|e| e.path()),
                );
            }
        }
        out
    }

    /// View index of the loaded row with this key.
    pub fn find(&self, key: u64) -> Option<usize> {
        let g = self.shared.lock();
        for id in g.layout.all_pages() {
            if let Some(page) = g.pages.get(&id) {
                if let Some(pos) = page.entries.iter().position(|e| e.key == key) {
                    return Some(g.layout.base(id) + pos);
                }
            }
        }
        None
    }

    /// (folders, files) inside a folder row, once the engine has counted them.
    pub fn counts(&self, key: u64) -> Option<(u32, u32)> {
        self.shared
            .lock()
            .counts
            .get(&key)
            .map(|c| (c.folders, c.files))
    }

    /// Ask the engine to re-run the query and swap the content in place (selection, scroll and
    /// the visible range stay valid).
    pub fn refresh(&self) {
        let _ = self.tx.send(Command::Refresh(self.shared.id));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fmt::Builder;
    use crate::model::{Kind, Sort};
    use std::sync::mpsc::channel;

    fn rows(b: &mut Builder, prefix: &str, n: usize, kind: Kind) -> Vec<Entry> {
        (0..n)
            .map(|i| b.entry(r"C:\x", format!("{prefix}{i}"), kind, 1, 0, 0, 0))
            .collect()
    }

    fn query() -> Query {
        Query {
            location: Some("x".into()),
            text: String::new(),
            regex: false,
            show_hidden: false,
            sort: Sort::DEFAULT,
        }
    }

    fn view_with(
        layout: Layout,
        pages: Vec<(PageId, Vec<Entry>)>,
    ) -> (View, std::sync::mpsc::Receiver<Command>) {
        let (tx, rx) = channel();
        let shared = Shared::new(7);
        shared.lock().replace(layout, pages, ViewState::Ready);
        (View::new(shared, tx, query()), rx)
    }

    #[test]
    fn get_maps_through_segments() {
        let mut b = Builder::new();
        let layout = Layout::new([3, 2]);
        let pages = vec![
            (
                PageId { seg: 0, idx: 0 },
                rows(&mut b, "d", 3, Kind::Folder),
            ),
            (PageId { seg: 1, idx: 0 }, rows(&mut b, "f", 2, Kind::File)),
        ];
        let (v, _rx) = view_with(layout, pages);
        assert_eq!(v.len(), 5);
        assert_eq!(&*v.get(0).unwrap().name, "d0");
        assert_eq!(&*v.get(2).unwrap().name, "d2");
        assert_eq!(&*v.get(3).unwrap().name, "f0");
        assert_eq!(&*v.get(4).unwrap().name, "f1");
        assert!(v.get(5).is_none());
        assert!(v.is_complete());
        assert_eq!(v.loaded_keys().len(), 5);
        let key = v.get(3).unwrap().key;
        assert_eq!(v.find(key), Some(3));
    }

    #[test]
    fn missing_page_is_none_and_incomplete() {
        let mut b = Builder::new();
        let layout = Layout::new([1000, 0]);
        let pages = vec![(
            PageId { seg: 0, idx: 0 },
            rows(&mut b, "a", 512, Kind::File),
        )];
        let (v, _rx) = view_with(layout, pages);
        assert!(v.get(10).is_some());
        assert!(v.get(600).is_none());
        assert!(!v.is_complete());
    }

    #[test]
    fn want_only_signals_on_change() {
        let layout = Layout::new([10, 0]);
        let (v, rx) = view_with(layout, vec![]);
        v.want(0..5);
        v.want(0..5);
        v.want(2..8);
        let sent: Vec<_> = rx.try_iter().collect();
        assert_eq!(sent.len(), 2);
        assert!(matches!(sent[0], Command::Want));
    }

    #[test]
    fn eviction_keeps_pages_near_the_viewport() {
        let mut b = Builder::new();
        let shared = Shared::new(1);
        let total = (MAX_PAGES as u32 + 10) * 512;
        {
            let mut g = shared.lock();
            g.replace(Layout::new([total, 0]), vec![], ViewState::Ready);
            g.wanted = (40 * 512)..(40 * 512 + 30);
            for idx in 0..(MAX_PAGES as u32 + 5) {
                g.insert_page(PageId { seg: 0, idx }, rows(&mut b, "r", 1, Kind::File));
            }
            assert_eq!(g.pages.len(), MAX_PAGES);
            assert!(g.pages.contains_key(&PageId { seg: 0, idx: 40 }));
            assert!(!g.pages.contains_key(&PageId { seg: 0, idx: 0 }));
        }
    }

    #[test]
    fn replace_bumps_revision() {
        let (v, _rx) = view_with(Layout::new([1, 0]), vec![]);
        let r = v.revision();
        v.shared().lock().reset_empty(EmptyReason::NoResults);
        assert_eq!(v.revision(), r + 1);
        assert_eq!(v.state(), ViewState::Empty(EmptyReason::NoResults));
        assert_eq!(v.len(), 0);
    }
}
