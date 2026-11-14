//! The engine thread: owns the backend (Everything), answers [`Command`]s and keeps the pages
//! of the current view filled.
//!
//! Rules it follows:
//! * Only the newest `Open` matters (older ones are dropped, like the old worker did).
//! * One backend query at a time (Everything runs one query per client).
//! * Commands are checked between every unit of work, so a new query is never stuck behind
//!   prefetching.
//! * The status (connecting / missing / loading index / ready) is tracked here and drives the
//!   "Everything is required" screen in the UI.

pub mod dummy;
#[cfg(windows)]
pub mod everything;

use std::{
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        mpsc::{Receiver, RecvTimeoutError},
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

use crate::{
    compose::{Compose, Only, Plan, Seg, compose},
    fmt::{Builder, flags_from_attrs},
    model::{DriveInfo, Entry, Kind, Query, SortDir, SortKey},
    paging::{Layout, MAX_SEGS, PAGE, PageId},
    view::{ChildCount, EmptyReason, Shared, ViewState},
};

/// Wakes the UI (`ctx.request_repaint()`); called from any thread.
pub type Wake = Arc<dyn Fn() + Send + Sync>;

/// Folders with at most this many rows are loaded completely in the background, so Select All
/// and file operations see every row.
pub const EAGER_MAX: usize = 20_000;

const BEAT: Duration = Duration::from_secs(1);
const BACKOFF: Duration = Duration::from_secs(1);
/// After a local change (new folder, rename) a browse is listed from disk for this long, until
/// the index has caught up.
const FS_WINDOW: Duration = Duration::from_secs(5);

pub(crate) enum Command {
    Open {
        shared: Arc<Shared>,
        query: Query,
    },
    Want,
    Refresh(u64),
    /// Refresh now from the file system: the index has not heard about a change we just made.
    RefreshFs(u64),
    Retry,
    Launch,
    Stop,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EngineStatus {
    Connecting,
    /// Everything is not installed or not running (the message says what was tried).
    Missing(String),
    /// Everything is running but still reading its index.
    LoadingIndex,
    Ready,
}

pub(crate) struct StatusCell(Mutex<EngineStatus>);

impl StatusCell {
    pub fn new(s: EngineStatus) -> Self {
        Self(Mutex::new(s))
    }

    pub fn get(&self) -> EngineStatus {
        self.0.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }

    /// Returns whether the value changed.
    pub fn set(&self, s: EngineStatus) -> bool {
        let mut g = self.0.lock().unwrap_or_else(|e| e.into_inner());
        if *g == s {
            return false;
        }
        *g = s;
        true
    }
}

pub enum Health {
    Missing(String),
    Loading,
    Ready,
}

pub struct QueryResult {
    pub total: u32,
    pub rows: Vec<Entry>,
}

/// A source of rows. Everything in production, a generator for UI work (`--dummy`).
pub trait Backend: Send {
    fn name(&self) -> &'static str;
    fn connect(&mut self) -> Result<(), String>;
    fn health(&mut self) -> Health;
    /// Rows `offset..offset+count` of segment `seg` of `plan`, plus the segment's total.
    fn query(
        &mut self,
        plan: &Plan,
        seg: usize,
        offset: u32,
        count: u32,
        builder: &mut Builder,
    ) -> Result<QueryResult, String>;
    /// (folders, files) directly inside `dir`.
    fn count_children(&mut self, dir: &Path) -> Result<(u32, u32), String>;
    /// Diagnostic only, used to tell an empty folder from one that is not indexed.
    fn dir_has_entries(&mut self, _dir: &Path) -> bool {
        false
    }
}

pub(crate) type DriveFn = Arc<dyn Fn() -> Arc<[DriveInfo]> + Send + Sync>;
pub(crate) type LaunchFn = Arc<dyn Fn() + Send + Sync>;

pub(crate) struct EngineCtx {
    pub wake: Wake,
    pub status: Arc<StatusCell>,
    pub drives: DriveFn,
    pub launch: LaunchFn,
}

struct Current {
    shared: Arc<Shared>,
    query: Query,
    compose: Compose,
}

enum Work {
    Page(PageId),
    Count(u64, PathBuf),
}

struct Engine {
    backend: Box<dyn Backend>,
    ctx: EngineCtx,
    builder: Builder,
    current: Option<Current>,
    connected: bool,
    ready: bool,
    last_beat: Instant,
    backoff_until: Option<Instant>,
    /// Browses are read from disk until then (see [`FS_WINDOW`]).
    fs_until: Option<Instant>,
}

pub(crate) fn spawn(
    backend: Box<dyn Backend>,
    ctx: EngineCtx,
    rx: Receiver<Command>,
) -> JoinHandle<()> {
    thread::Builder::new()
        .name("qf-engine".into())
        .spawn(move || {
            let engine = Engine {
                backend,
                ctx,
                builder: Builder::new(),
                current: None,
                connected: false,
                ready: false,
                last_beat: Instant::now(),
                backoff_until: None,
                fs_until: None,
            };
            run(engine, rx);
        })
        .expect("failed to spawn the engine thread")
}

fn run(mut e: Engine, rx: Receiver<Command>) {
    e.try_connect();
    loop {
        let mut cmds: Vec<Command> = Vec::new();
        if e.next_work().is_some() {
            // Busy: only peek at the queue so new requests pre-empt prefetching.
            while let Ok(c) = rx.try_recv() {
                cmds.push(c);
            }
        } else {
            match rx.recv_timeout(Duration::from_millis(250)) {
                Ok(c) => {
                    cmds.push(c);
                    while let Ok(c) = rx.try_recv() {
                        cmds.push(c);
                    }
                }
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => return,
            }
        }

        let mut open = None;
        let mut refresh = false;
        let (mut retry, mut launch) = (false, false);
        for c in cmds {
            match c {
                // Coalesce: the newest Open wins.
                Command::Open { shared, query } => {
                    open = Some((shared, query));
                    refresh = false;
                }
                Command::Refresh(id) => {
                    let is_current = e.current.as_ref().is_some_and(|c| c.shared.id == id);
                    if is_current && open.is_none() {
                        refresh = true;
                    }
                }
                Command::RefreshFs(id) => {
                    let is_current = e.current.as_ref().is_some_and(|c| c.shared.id == id);
                    if is_current && open.is_none() {
                        refresh = true;
                        e.fs_until = Some(Instant::now() + FS_WINDOW);
                    }
                }
                Command::Want => {} // the range lives in the view; next_work reads it
                Command::Retry => retry = true,
                Command::Launch => launch = true,
                Command::Stop => return,
            }
        }

        if launch {
            (e.ctx.launch)();
        }
        if retry || launch {
            e.try_connect();
        }
        if let Some((shared, query)) = open {
            e.open(shared, query);
        } else if refresh {
            e.reload_current();
        }
        e.beat();
        e.do_work();
    }
}

impl Engine {
    fn wake(&self) {
        (self.ctx.wake)();
    }

    fn set_status(&self, s: EngineStatus) {
        if self.ctx.status.set(s) {
            self.wake();
        }
    }

    fn try_connect(&mut self) {
        match self.backend.connect() {
            Ok(()) => {
                self.connected = true;
                self.evaluate();
            }
            Err(msg) => {
                self.connected = false;
                self.set_ready(false);
                self.set_status(EngineStatus::Missing(msg));
            }
        }
        self.last_beat = Instant::now();
    }

    /// Read the backend's health and publish it.
    fn evaluate(&mut self) {
        match self.backend.health() {
            Health::Missing(msg) => {
                self.connected = false;
                self.set_ready(false);
                self.set_status(EngineStatus::Missing(msg));
            }
            Health::Loading => {
                self.set_ready(false);
                self.set_status(EngineStatus::LoadingIndex);
            }
            Health::Ready => {
                self.set_ready(true);
                self.set_status(EngineStatus::Ready);
            }
        }
    }

    fn beat(&mut self) {
        if self.last_beat.elapsed() < BEAT {
            return;
        }
        self.last_beat = Instant::now();
        if self.connected {
            self.evaluate();
        } else {
            self.try_connect();
        }
    }

    fn set_ready(&mut self, ready: bool) {
        let was = self.ready;
        self.ready = ready;
        if ready && !was {
            // Everything just became usable: (re)run whatever the UI is waiting for.
            self.reload_current();
        }
    }

    fn open(&mut self, shared: Arc<Shared>, query: Query) {
        let compose = compose(&query);
        self.current = Some(Current {
            shared,
            query,
            compose,
        });
        self.backoff_until = None;
        self.reload_current();
    }

    /// Run the current query from scratch and swap the result into the view.
    fn reload_current(&mut self) {
        let Some(cur) = &self.current else {
            return;
        };
        let shared = cur.shared.clone();
        match cur.compose.clone() {
            Compose::Empty => {
                shared.lock().reset_empty(EmptyReason::NoQuery);
                self.wake();
            }
            Compose::Drives => self.load_drives(&shared),
            Compose::Search(plan) => {
                if !self.ready {
                    shared.lock().state = ViewState::Loading;
                    self.wake();
                    return;
                }
                self.load(&shared, &plan);
            }
        }
    }

    fn load_drives(&mut self, shared: &Arc<Shared>) {
        let t0 = Instant::now();
        let drives = (self.ctx.drives)();
        let rows: Vec<Entry> = drives
            .iter()
            .map(|d| {
                self.builder
                    .drive(&d.label, &d.root.to_string_lossy(), d.system)
            })
            .collect();
        let n = rows.len() as u32;
        let mut g = shared.lock();
        if rows.is_empty() {
            g.reset_empty(EmptyReason::NoResults);
        } else {
            g.replace(
                Layout::new([n, 0]),
                vec![(PageId { seg: 0, idx: 0 }, rows)],
                ViewState::Ready,
            );
        }
        g.took_us = t0.elapsed().as_micros() as u64;
        drop(g);
        self.wake();
    }

    fn load(&mut self, shared: &Arc<Shared>, plan: &Plan) {
        let t0 = Instant::now();
        // Just after a local change the index is stale: list a plain folder from disk.
        if self.fs_until.is_some_and(|t| Instant::now() < t) {
            if let Some(dir) = plan.children_of.as_ref() {
                let [folders, files] = list_dir(dir, plan, &mut self.builder);
                if !folders.is_empty() || !files.is_empty() {
                    let (layout, got) = fs_pages(folders, files);
                    let mut g = shared.lock();
                    g.replace(layout, got, ViewState::Ready);
                    g.took_us = t0.elapsed().as_micros() as u64;
                    drop(g);
                    self.backoff_until = None;
                    self.wake();
                    return;
                }
            }
        }
        let mut counts = [0u32; MAX_SEGS];
        let mut got: Vec<(PageId, Vec<Entry>)> = Vec::new();

        for seg in 0..plan.segs.len().min(MAX_SEGS) {
            match self.backend.query(plan, seg, 0, PAGE, &mut self.builder) {
                Ok(r) => {
                    counts[seg] = r.total;
                    got.push((
                        PageId {
                            seg: seg as u8,
                            idx: 0,
                        },
                        r.rows,
                    ));
                }
                Err(e) => {
                    self.fail(shared, e);
                    return;
                }
            }
        }
        let layout = Layout::new(counts);

        // Pages the view already asked for (a refresh keeps the viewport filled).
        let wanted = shared.lock().wanted.clone();
        for id in layout.pages_in(wanted) {
            if got.iter().any(|(g, _)| *g == id) {
                continue;
            }
            if let Ok(r) = self.backend.query(
                plan,
                id.seg as usize,
                id.idx * PAGE,
                PAGE,
                &mut self.builder,
            ) {
                got.push((id, r.rows));
            }
        }

        let mut layout = layout;
        let mut state = ViewState::Ready;
        if layout.total() == 0 {
            let reason = self.empty_reason(plan);
            state = ViewState::Empty(reason);
            // Everything has nothing for a folder that has items on disk (not indexed): list
            // it straight from the file system instead of showing an empty window.
            if reason == EmptyReason::NotIndexed {
                if let Some(dir) = plan.children_of.as_ref() {
                    let [folders, files] = list_dir(dir, plan, &mut self.builder);
                    if !folders.is_empty() || !files.is_empty() {
                        (layout, got) = fs_pages(folders, files);
                        state = ViewState::Ready;
                    }
                }
            }
        }
        let mut g = shared.lock();
        g.replace(layout, got, state);
        g.took_us = t0.elapsed().as_micros() as u64;
        drop(g);
        self.backoff_until = None;
        self.wake();
    }

    fn fail(&mut self, shared: &Arc<Shared>, message: String) {
        shared.lock().state = ViewState::Error(message);
        self.backoff_until = Some(Instant::now() + BACKOFF);
        self.wake();
    }

    /// Why a browse came back empty: nothing there, only hidden items, or not indexed.
    fn empty_reason(&mut self, plan: &Plan) -> EmptyReason {
        let Some(dir) = &plan.children_of else {
            return EmptyReason::NoResults;
        };
        if !plan.show_hidden {
            if let Some(probe) = &plan.probe {
                let probe_plan = Plan {
                    segs: vec![Seg {
                        text: probe.clone(),
                        only: Only::Both,
                    }],
                    ..plan.clone()
                };
                if let Ok(r) = self.backend.query(&probe_plan, 0, 0, 1, &mut self.builder) {
                    if r.total > 0 {
                        return EmptyReason::OnlyHidden;
                    }
                }
            }
        }
        if self.backend.dir_has_entries(dir) {
            EmptyReason::NotIndexed
        } else {
            EmptyReason::NoResults
        }
    }

    /// The next unit of background work for the current view, if any.
    fn next_work(&self) -> Option<Work> {
        if !self.ready {
            return None;
        }
        if self.backoff_until.is_some_and(|t| Instant::now() < t) {
            return None;
        }
        let cur = self.current.as_ref()?;
        if !matches!(cur.compose, Compose::Search(_)) {
            return None;
        }
        let g = cur.shared.lock();
        if g.state != ViewState::Ready {
            return None;
        }
        let wanted = g.wanted.clone();

        // 1. The rows on screen.
        for id in g.layout.pages_in(wanted.clone()) {
            if !g.pages.contains_key(&id) {
                return Some(Work::Page(id));
            }
        }
        // 2. One page of lookahead on each side.
        let around =
            wanted.start.saturating_sub(PAGE as usize)..wanted.end.saturating_add(PAGE as usize);
        for id in g.layout.pages_in(around) {
            if !g.pages.contains_key(&id) {
                return Some(Work::Page(id));
            }
        }
        // 3. Folders of a sane size are loaded completely.
        if cur.query.location.is_some() && g.layout.total() <= EAGER_MAX {
            for id in g.layout.all_pages() {
                if !g.pages.contains_key(&id) {
                    return Some(Work::Page(id));
                }
            }
        }
        // 4. Child counts of the visible folders.
        for i in wanted {
            let Some((id, row)) = g.layout.locate(i) else {
                break;
            };
            let Some(page) = g.pages.get(&id) else {
                continue;
            };
            let Some(e) = page.entries.get(row) else {
                continue;
            };
            if e.kind != Kind::Folder {
                continue;
            }
            let fresh = g.counts.get(&e.key).is_some_and(|c| c.rev == g.revision);
            if !fresh && !g.tried.contains(&(e.key, g.revision)) {
                return Some(Work::Count(e.key, e.path()));
            }
        }
        None
    }

    fn do_work(&mut self) {
        let Some(work) = self.next_work() else {
            return;
        };
        let Some(cur) = &self.current else {
            return;
        };
        let shared = cur.shared.clone();
        let Compose::Search(plan) = cur.compose.clone() else {
            return;
        };
        match work {
            Work::Page(id) => {
                match self.backend.query(
                    &plan,
                    id.seg as usize,
                    id.idx * PAGE,
                    PAGE,
                    &mut self.builder,
                ) {
                    Ok(r) => {
                        shared.lock().insert_page(id, r.rows);
                        self.wake();
                    }
                    Err(_) => self.backoff_until = Some(Instant::now() + BACKOFF),
                }
            }
            Work::Count(key, path) => {
                let result = self.backend.count_children(&path);
                let mut g = shared.lock();
                let rev = g.revision;
                g.tried.insert((key, rev));
                if let Ok((folders, files)) = result {
                    g.counts.insert(
                        key,
                        ChildCount {
                            folders,
                            files,
                            rev,
                        },
                    );
                }
                drop(g);
                self.wake();
            }
        }
    }
}

/// Pages for a folder listed from disk: folders first, then files, `PAGE` rows each.
fn fs_pages(folders: Vec<Entry>, files: Vec<Entry>) -> (Layout, Vec<(PageId, Vec<Entry>)>) {
    let layout = Layout::new([folders.len() as u32, files.len() as u32]);
    let mut got = Vec::new();
    for (seg, mut rows) in [folders, files].into_iter().enumerate() {
        let mut idx = 0;
        while !rows.is_empty() {
            let rest = rows.split_off(rows.len().min(PAGE as usize));
            got.push((
                PageId {
                    seg: seg as u8,
                    idx,
                },
                rows,
            ));
            rows = rest;
            idx += 1;
        }
    }
    (layout, got)
}

/// Rows of `dir` read straight from the file system: `[folders, files]`, sorted like the plan.
/// Used when Everything has no data for a folder (it is not indexed). Hidden items follow
/// `plan.show_hidden`. Sorting by Type falls back to name.
pub(crate) fn list_dir(dir: &Path, plan: &Plan, builder: &mut Builder) -> [Vec<Entry>; 2] {
    let tz = crate::sys::tz_offset();
    let parent = dir.to_string_lossy().into_owned();
    let mut out: [Vec<Entry>; 2] = [Vec::new(), Vec::new()];
    let Ok(rd) = std::fs::read_dir(dir) else {
        return out;
    };
    for item in rd.flatten() {
        let Ok(meta) = item.metadata() else {
            continue;
        };
        #[cfg(windows)]
        let attrs = std::os::windows::fs::MetadataExt::file_attributes(&meta);
        #[cfg(not(windows))]
        let attrs = 0u32;
        let flags = flags_from_attrs(attrs);
        let name = item.file_name().to_string_lossy().into_owned();
        #[cfg(not(windows))]
        let hidden = name.starts_with('.');
        #[cfg(windows)]
        let hidden = flags & crate::model::FLAG_HIDDEN != 0;
        if hidden && !plan.show_hidden {
            continue;
        }
        let is_dir = meta.is_dir();
        let modified = meta
            .modified()
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map_or(0, |d| d.as_secs() as i64);
        let size = if is_dir { 0 } else { meta.len() };
        let kind = if is_dir { Kind::Folder } else { Kind::File };
        out[usize::from(!is_dir)]
            .push(builder.entry(&parent, name, kind, size, modified, flags, tz));
    }
    for rows in &mut out {
        rows.sort_by(|a, b| {
            let by_name = a.name.to_lowercase().cmp(&b.name.to_lowercase());
            let ord = match plan.sort.key {
                SortKey::Modified => a.modified.cmp(&b.modified).then(by_name),
                SortKey::Size => a.size.cmp(&b.size).then(by_name),
                SortKey::Name | SortKey::Type => by_name,
            };
            if plan.sort.dir == SortDir::Desc {
                ord.reverse()
            } else {
                ord
            }
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn list_dir_reads_the_file_system_folders_first_split() {
        use crate::model::{Sort, SortDir, SortKey};
        let root = std::env::temp_dir().join(format!("qf_listdir_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("sub")).unwrap();
        std::fs::write(root.join("b.txt"), "12").unwrap();
        std::fs::write(root.join("a.txt"), "1").unwrap();
        let plan = Plan {
            segs: Vec::new(),
            sort: Sort {
                key: SortKey::Name,
                dir: SortDir::Asc,
            },
            show_hidden: true,
            children_of: Some(root.clone()),
            scope: None,
            needle: String::new(),
            regex: false,
            probe: None,
        };
        let [folders, files] = list_dir(&root, &plan, &mut Builder::new());
        assert_eq!(folders.len(), 1);
        assert_eq!(&*folders[0].name, "sub");
        let names: Vec<&str> = files.iter().map(|e| &*e.name).collect();
        assert_eq!(names, ["a.txt", "b.txt"]);
        assert_eq!(files[1].size, 2);
        let _ = std::fs::remove_dir_all(root);
    }
    use crate::model::Sort;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::mpsc::channel;
    use std::time::Instant;

    fn start(
        warmup: u32,
    ) -> (
        std::sync::mpsc::Sender<Command>,
        Arc<StatusCell>,
        Arc<AtomicUsize>,
    ) {
        let (tx, rx) = channel();
        let wakes = Arc::new(AtomicUsize::new(0));
        let w = wakes.clone();
        let status = Arc::new(StatusCell::new(EngineStatus::Connecting));
        let ctx = EngineCtx {
            wake: Arc::new(move || {
                w.fetch_add(1, Ordering::Relaxed);
            }),
            status: status.clone(),
            drives: Arc::new(dummy::dummy_drives),
            launch: Arc::new(|| {}),
        };
        let mut d = dummy::Dummy::new();
        d.warmup = warmup;
        spawn(Box::new(d), ctx, rx);
        (tx, status, wakes)
    }

    fn q(location: Option<&str>, text: &str) -> Query {
        Query {
            location: location.map(PathBuf::from),
            text: text.into(),
            regex: false,
            show_hidden: false,
            sort: Sort::DEFAULT,
        }
    }

    fn wait<F: Fn() -> bool>(f: F) {
        let t = Instant::now();
        while !f() {
            assert!(t.elapsed() < Duration::from_secs(10), "timed out");
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    #[test]
    fn open_search_stitches_and_pages() {
        let (tx, status, _w) = start(0);
        let shared = Shared::new(1);
        tx.send(Command::Open {
            shared: shared.clone(),
            query: q(None, "report"),
        })
        .unwrap();
        wait(|| shared.lock().state == ViewState::Ready);
        assert_eq!(status.get(), EngineStatus::Ready);
        assert_eq!(shared.lock().layout.total(), 3000);
        // Ask for a far page; the engine must fetch it.
        shared.lock().wanted = 2000..2030;
        tx.send(Command::Want).unwrap();
        wait(|| {
            let g = shared.lock();
            let (id, _) = g.layout.locate(2010).unwrap();
            g.pages.contains_key(&id)
        });
        tx.send(Command::Stop).unwrap();
    }

    #[test]
    fn browse_empty_query_is_no_query() {
        let (tx, _s, _w) = start(0);
        let shared = Shared::new(2);
        tx.send(Command::Open {
            shared: shared.clone(),
            query: q(None, ""),
        })
        .unwrap();
        wait(|| shared.lock().state == ViewState::Empty(EmptyReason::NoQuery));
        tx.send(Command::Stop).unwrap();
    }

    #[test]
    fn browse_folder_loads_completely_and_counts_children() {
        let (tx, _s, _w) = start(0);
        let shared = Shared::new(3);
        tx.send(Command::Open {
            shared: shared.clone(),
            query: q(Some("some/dir"), ""),
        })
        .unwrap();
        // Make sure there are folders: find a dir with some.
        wait(|| matches!(shared.lock().state, ViewState::Ready | ViewState::Empty(_)));
        let g = shared.lock();
        if g.state == ViewState::Ready {
            let n = g.layout.total();
            drop(g);
            shared.lock().wanted = 0..n.min(30);
            tx.send(Command::Want).unwrap();
            wait(|| {
                let g = shared.lock();
                g.pages.len() >= g.layout.page_count()
            });
            // Child counts show up for visible folders.
            let first_folder = {
                let g = shared.lock();
                let page = g.pages.get(&PageId { seg: 0, idx: 0 });
                page.and_then(|p| p.entries.first().map(|e| e.key))
            };
            if let Some(k) = first_folder {
                wait(|| shared.lock().counts.contains_key(&k));
            }
        }
        tx.send(Command::Stop).unwrap();
    }

    #[test]
    fn this_pc_lists_drives() {
        let (tx, _s, _w) = start(0);
        let shared = Shared::new(4);
        tx.send(Command::Open {
            shared: shared.clone(),
            query: q(Some(""), ""),
        })
        .unwrap();
        wait(|| shared.lock().state == ViewState::Ready);
        assert_eq!(shared.lock().layout.total(), 3);
        tx.send(Command::Stop).unwrap();
    }

    #[test]
    fn newest_open_wins_and_warmup_resolves() {
        let (tx, status, _w) = start(2);
        let a = Shared::new(5);
        let b = Shared::new(6);
        tx.send(Command::Open {
            shared: a.clone(),
            query: q(None, "one"),
        })
        .unwrap();
        tx.send(Command::Open {
            shared: b.clone(),
            query: q(None, "two"),
        })
        .unwrap();
        wait(|| b.lock().state == ViewState::Ready);
        assert_eq!(status.get(), EngineStatus::Ready);
        tx.send(Command::Stop).unwrap();
    }

    #[test]
    fn refresh_swaps_in_place() {
        let (tx, _s, _w) = start(0);
        let shared = Shared::new(7);
        tx.send(Command::Open {
            shared: shared.clone(),
            query: q(None, "x"),
        })
        .unwrap();
        wait(|| shared.lock().state == ViewState::Ready);
        let rev = shared.lock().revision;
        tx.send(Command::Refresh(7)).unwrap();
        wait(|| shared.lock().revision > rev);
        assert_eq!(shared.lock().state, ViewState::Ready);
        tx.send(Command::Stop).unwrap();
    }
}
