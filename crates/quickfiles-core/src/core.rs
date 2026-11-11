//! The one handle the UI holds. Single-threaded use (`&mut self`); everything slow happens on
//! background threads and shows up as state the UI reads next frame.

use std::{
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
        mpsc::{Sender, channel},
    },
    thread,
    time::Duration,
};

use crate::{
    engine::{self, Command, EngineCtx, EngineStatus, StatusCell, Wake, dummy},
    images::Images,
    jobs::{self, Conflict, JobInfo, Jobs},
    model::{DriveInfo, Entry, Place, Query, Sort, SortKey},
    nav::Nav,
    store::{Snapshot, Store},
    sys,
    view::{Shared, View},
    watch::Watch,
};

#[derive(Debug)]
pub struct CoreError(pub String);

impl std::fmt::Display for CoreError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for CoreError {}

impl From<std::io::Error> for CoreError {
    fn from(e: std::io::Error) -> Self {
        Self(e.to_string())
    }
}

pub struct Config {
    /// Fake file system, no Everything (UI development).
    pub dummy: bool,
    /// Where the database and thumbnail cache live; `None` = the usual app-data folder.
    pub data_dir: Option<PathBuf>,
    /// Requests a repaint; called from background threads.
    pub wake: Wake,
}

/// What `paste` does with the internal clipboard.
struct Clipboard {
    paths: Vec<PathBuf>,
    cut: bool,
}

pub struct Core {
    nav: Nav,
    view: View,
    tx: Sender<Command>,
    status: Arc<StatusCell>,
    current_id: Arc<AtomicU64>,
    next_id: u64,
    store: Store,
    images: Images,
    jobs: Jobs,
    watch: Watch,
    drives: Arc<Mutex<Arc<[DriveInfo]>>>,
    places: Vec<Place>,
    clipboard: Option<Clipboard>,
    dummy: bool,
    /// Tab slots. The live tab is `self.nav`; its slot here is refreshed when you switch away.
    tabs: Vec<Nav>,
    active_tab: usize,
    /// Clipboard sequence number after our own last copy/cut.
    os_clip_seq: u32,
}

fn default_sort(snap: &Snapshot) -> Sort {
    let key = match snap.pref("sort_key") {
        Some("type") => SortKey::Type,
        Some("modified") => SortKey::Modified,
        Some("size") => SortKey::Size,
        _ => SortKey::Name,
    };
    let dir = if snap.pref("sort_dir") == Some("desc") {
        crate::model::SortDir::Desc
    } else {
        crate::model::SortDir::Asc
    };
    Sort { key, dir }
}

impl Core {
    pub fn start(cfg: Config) -> Self {
        let data = cfg.data_dir.clone().unwrap_or_else(sys::data_dir);
        let _ = std::fs::create_dir_all(&data);
        let wake = cfg.wake.clone();

        let store = Store::open(&data.join("quickfiles.db"), wake.clone());
        let snap = store.snapshot();

        // Drives are polled on their own thread: a sleeping network drive must never block anything.
        let drives: Arc<Mutex<Arc<[DriveInfo]>>> = Arc::new(Mutex::new(Arc::from(Vec::new())));
        let dummy_mode = cfg.dummy || !cfg!(windows);
        {
            let (d, w) = (drives.clone(), wake.clone());
            thread::Builder::new()
                .name("qf-drives".into())
                .spawn(move || {
                    loop {
                        let list = if dummy_mode {
                            dummy::dummy_drives()
                        } else {
                            sys::drives()
                        };
                        {
                            let mut g = d.lock().unwrap_or_else(|e| e.into_inner());
                            if **g != *list {
                                *g = list;
                                drop(g);
                                w();
                            }
                        }
                        thread::sleep(Duration::from_secs(10));
                    }
                })
                .expect("failed to spawn the drives thread");
        }
        let drives_fn: engine::DriveFn = {
            let d = drives.clone();
            Arc::new(move || d.lock().unwrap_or_else(|e| e.into_inner()).clone())
        };

        let status = Arc::new(StatusCell::new(EngineStatus::Connecting));
        let backend: Box<dyn engine::Backend> = if dummy_mode {
            Box::new(dummy::Dummy::new())
        } else {
            #[cfg(windows)]
            {
                Box::new(engine::everything::Everything::new(sys::tz_offset()))
            }
            #[cfg(not(windows))]
            {
                Box::new(dummy::Dummy::new())
            }
        };
        let (tx, rx) = channel();
        engine::spawn(
            backend,
            EngineCtx {
                wake: wake.clone(),
                status: status.clone(),
                drives: drives_fn,
                launch: Arc::new(sys::launch_everything),
            },
            rx,
        );

        let show_hidden = snap.pref_bool("show_hidden", false);
        let mut nav = Nav::new(None, default_sort(&snap), show_hidden);
        if snap.pref_bool("regex", false) {
            nav.toggle_regex();
        }
        let shared = Shared::new(1);
        let view = View::new(shared.clone(), tx.clone(), nav.query().clone());
        let _ = tx.send(Command::Open {
            shared,
            query: nav.query().clone(),
        });

        let current_id = Arc::new(AtomicU64::new(1));
        let watch = {
            let (tx, id) = (tx.clone(), current_id.clone());
            Watch::new(wake.clone(), move || {
                let _ = tx.send(Command::Refresh(id.load(Ordering::Relaxed)));
            })
        };

        let nav_for_tabs = nav.clone();
        let threads = thread::available_parallelism().map_or(2, |n| n.get().clamp(2, 4));
        Self {
            images: Images::new(
                &data,
                wake.clone(),
                threads,
                snap.pref("thumb_cache_mb")
                    .and_then(|v| v.parse().ok())
                    .unwrap_or(crate::images::cache::DEFAULT_BUDGET_MB),
            ),
            jobs: Jobs::new(wake),
            nav,
            view,
            tx,
            status,
            current_id,
            next_id: 2,
            store,
            watch,
            drives,
            places: if dummy_mode {
                dummy_places()
            } else {
                sys::places()
            },
            clipboard: None,
            dummy: dummy_mode,
            tabs: vec![nav_for_tabs],
            active_tab: 0,
            os_clip_seq: 0,
        }
    }

    // --- State the UI reads ---------------------------------------------------------------

    pub fn nav(&self) -> &Nav {
        &self.nav
    }

    pub fn view(&self) -> &View {
        &self.view
    }

    pub fn status(&self) -> EngineStatus {
        self.status.get()
    }

    pub fn data(&self) -> Arc<Snapshot> {
        self.store.snapshot()
    }

    pub fn images(&self) -> &Images {
        &self.images
    }

    pub fn drives(&self) -> Arc<[DriveInfo]> {
        self.drives
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    pub fn places(&self) -> &[Place] {
        &self.places
    }

    pub fn is_dummy(&self) -> bool {
        self.dummy
    }

    pub fn jobs(&self) -> Vec<JobInfo> {
        self.jobs.snapshot()
    }

    pub fn can_paste(&self) -> bool {
        self.paste_source().is_some()
            && self
                .nav
                .location()
                .is_some_and(|p| !p.as_os_str().is_empty())
    }

    // --- Navigation -----------------------------------------------------------------------

    /// Re-run the current query as a new view.
    fn reopen(&mut self) {
        let id = self.next_id;
        self.next_id += 1;
        let query: Query = self.nav.query().clone();
        let shared = Shared::new(id);
        self.view = View::new(shared.clone(), self.tx.clone(), query.clone());
        self.current_id.store(id, Ordering::Relaxed);
        let _ = self.tx.send(Command::Open { shared, query });
        self.watch.set(if self.nav.query().is_browse() {
            self.nav.location().map(|p| p.as_path())
        } else {
            None
        });
    }

    /// Applies a navigation that changed the location: restores the folder's own sort, records
    /// the visit and opens the view.
    fn moved(&mut self) {
        if let Some(dir) = self.nav.location().cloned() {
            if let Some(s) = self.data().folder_sort(&dir) {
                self.nav.set_sort(s);
            }
            self.store.visit(&dir);
        }
        self.reopen();
    }

    pub fn set_text(&mut self, text: &str) {
        if self.nav.set_text(text) {
            self.reopen();
        }
    }

    /// Enter pressed in the search box: remember the query.
    pub fn commit_query(&self) {
        self.store.add_query(self.nav.text());
    }

    pub fn open_folder(&mut self, path: PathBuf) {
        if self.nav.text().trim().len() >= 2 {
            self.store.add_query(self.nav.text());
        }
        if self.nav.open_folder(path) {
            self.moved();
        }
    }

    pub fn up(&mut self) {
        if self.nav.up() {
            self.moved();
        }
    }

    pub fn back(&mut self) {
        if self.nav.back() {
            self.moved();
        }
    }

    pub fn forward(&mut self) {
        if self.nav.forward() {
            self.moved();
        }
    }

    /// The x button: global search again.
    pub fn clear(&mut self) {
        if self.nav.clear() {
            self.reopen();
        }
    }

    /// Esc in the search box.
    pub fn escape(&mut self) {
        if self.nav.escape() {
            self.reopen();
        }
    }

    pub fn toggle_regex(&mut self) {
        self.nav.toggle_regex();
        self.store.set_pref_bool("regex", self.nav.query().regex);
        if !self.nav.text().trim().is_empty() {
            self.reopen();
        }
    }

    pub fn toggle_hidden(&mut self) {
        self.nav.toggle_hidden();
        self.store
            .set_pref_bool("show_hidden", self.nav.query().show_hidden);
        self.reopen();
    }

    pub fn toggle_sort(&mut self, key: SortKey) {
        self.nav.toggle_sort(key);
        let s = self.nav.query().sort;
        if let Some(dir) = self.nav.location().filter(|p| !p.as_os_str().is_empty()) {
            self.store.set_folder_sort(dir, s);
        }
        self.store.set_pref(
            "sort_key",
            match s.key {
                SortKey::Name => "name",
                SortKey::Type => "type",
                SortKey::Modified => "modified",
                SortKey::Size => "size",
            },
        );
        self.store.set_pref(
            "sort_dir",
            if s.dir == crate::model::SortDir::Desc {
                "desc"
            } else {
                "asc"
            },
        );
        self.reopen();
    }

    pub fn refresh(&self) {
        self.view.refresh();
    }

    /// Everything gate: try to connect again.
    pub fn retry(&self) {
        let _ = self.tx.send(Command::Retry);
    }

    pub fn launch_everything(&self) {
        let _ = self.tx.send(Command::Launch);
    }

    // --- Tabs -------------------------------------------------------------------------------

    pub fn tab_count(&self) -> usize {
        self.tabs.len()
    }

    pub fn active_tab(&self) -> usize {
        self.active_tab
    }

    /// Label for a tab: the folder name, the search text, or "Search".
    pub fn tab_title(&self, i: usize) -> String {
        let nav = if i == self.active_tab {
            &self.nav
        } else {
            &self.tabs[i]
        };
        match nav.location() {
            Some(p) if p.as_os_str().is_empty() => "This PC".to_owned(),
            Some(p) => crate::model::display_name(p),
            None if nav.text().trim().is_empty() => "Search".to_owned(),
            None => nav.text().trim().to_owned(),
        }
    }

    /// Switches to the tab that is already in slot `i` (the live tab was saved by the caller).
    fn load_tab(&mut self, i: usize) {
        self.active_tab = i;
        self.nav = self.tabs[i].clone();
        self.reopen();
    }

    fn save_tab(&mut self) {
        self.tabs[self.active_tab] = self.nav.clone();
    }

    pub fn switch_tab(&mut self, i: usize) {
        if i >= self.tabs.len() || i == self.active_tab {
            return;
        }
        self.save_tab();
        self.load_tab(i);
    }

    pub fn next_tab(&mut self, step: i32) {
        let n = self.tabs.len() as i32;
        let i = (self.active_tab as i32 + step).rem_euclid(n) as usize;
        self.switch_tab(i);
    }

    /// New tab next to the current one, starting in Search mode.
    pub fn new_tab(&mut self) {
        self.save_tab();
        let q = self.nav.query();
        let mut nav = Nav::new(None, q.sort, q.show_hidden);
        if q.regex {
            nav.toggle_regex();
        }
        self.tabs.insert(self.active_tab + 1, nav);
        self.load_tab(self.active_tab + 1);
    }

    pub fn close_tab(&mut self, i: usize) {
        if self.tabs.len() <= 1 || i >= self.tabs.len() {
            return;
        }
        self.save_tab();
        self.tabs.remove(i);
        let next = if i < self.active_tab {
            self.active_tab - 1
        } else {
            self.active_tab.min(self.tabs.len() - 1)
        };
        self.load_tab(next);
    }

    // --- Bookmarks, prefs -----------------------------------------------------------------

    pub fn is_bookmarked(&self, path: &Path) -> bool {
        self.data().is_bookmarked(path)
    }

    pub fn toggle_bookmark(&self, path: &Path) {
        let label = crate::model::display_name(path);
        self.store.toggle_bookmark(path, &label);
    }

    pub fn remove_bookmark(&self, path: &Path) {
        self.store.remove_bookmark(path);
    }

    pub fn clear_history(&self) {
        self.store.clear_history();
    }

    pub fn set_pref(&self, key: &str, value: &str) {
        self.store.set_pref(key, value);
    }

    pub fn set_pref_bool(&self, key: &str, value: bool) {
        self.store.set_pref_bool(key, value);
    }

    // --- Files ----------------------------------------------------------------------------

    pub fn open(&self, path: &Path) -> Result<(), CoreError> {
        if self.dummy {
            return Ok(());
        }
        sys::open(path).map_err(CoreError)
    }

    pub fn reveal(&self, path: &Path) -> Result<(), CoreError> {
        if self.dummy {
            return Ok(());
        }
        sys::reveal(path).map_err(CoreError)
    }

    fn refresher(&self) -> impl FnOnce() + Send + 'static {
        let (tx, id) = (self.tx.clone(), self.current_id.clone());
        move || {
            let _ = tx.send(Command::Refresh(id.load(Ordering::Relaxed)));
        }
    }

    pub fn copy(&mut self, paths: Vec<PathBuf>) {
        self.set_clipboard(paths, false);
    }

    pub fn cut(&mut self, paths: Vec<PathBuf>) {
        self.set_clipboard(paths, true);
    }

    /// Internal clipboard plus the OS one, so Explorer can paste what we copied.
    fn set_clipboard(&mut self, paths: Vec<PathBuf>, cut: bool) {
        if !self.dummy {
            sys::clipboard_set_files(&paths, cut);
            self.os_clip_seq = sys::clipboard_sequence();
        }
        self.clipboard = Some(Clipboard { paths, cut });
    }

    /// What a paste would use: files someone else put on the OS clipboard win over ours
    /// (detected by the clipboard sequence number), otherwise our own copy/cut.
    fn paste_source(&self) -> Option<(Vec<PathBuf>, bool)> {
        if !self.dummy && sys::clipboard_sequence() != self.os_clip_seq {
            if let Some(c) = sys::clipboard_files() {
                return Some((c.paths, c.cut));
            }
        }
        self.clipboard.as_ref().map(|c| (c.paths.clone(), c.cut))
    }

    fn paste_dest(&self) -> Option<PathBuf> {
        self.nav
            .location()
            .filter(|p| !p.as_os_str().is_empty())
            .cloned()
    }

    /// How many items of the pending paste already exist in the current folder.
    pub fn paste_conflicts(&self) -> usize {
        match (self.paste_source(), self.paste_dest()) {
            (Some((paths, _)), Some(dest)) => jobs::conflicts(&paths, &dest),
            _ => 0,
        }
    }

    pub fn paste(&mut self, policy: Conflict) {
        let (Some(dest), Some((paths, cut))) = (self.paste_dest(), self.paste_source()) else {
            return;
        };
        let kind = if cut {
            jobs::Kind::Move
        } else {
            jobs::Kind::Copy
        };
        if cut {
            // A cut is used once.
            self.clipboard = None;
            if !self.dummy {
                sys::clipboard_clear();
                self.os_clip_seq = sys::clipboard_sequence();
            }
        }
        self.jobs
            .start(kind, paths, Some(dest), policy, self.refresher());
    }

    /// Copy or move `paths` into `dest` (drag and drop).
    pub fn transfer(&mut self, paths: Vec<PathBuf>, dest: PathBuf, copy: bool, policy: Conflict) {
        let kind = if copy {
            jobs::Kind::Copy
        } else {
            jobs::Kind::Move
        };
        self.jobs
            .start(kind, paths, Some(dest), policy, self.refresher());
    }

    pub fn transfer_conflicts(&self, paths: &[PathBuf], dest: &Path) -> usize {
        jobs::conflicts(paths, dest)
    }

    /// Recycle bin (`permanent = false`) or irreversible delete.
    pub fn remove(&mut self, paths: Vec<PathBuf>, permanent: bool) {
        for p in &paths {
            self.images.invalidate(p);
        }
        let kind = if permanent {
            jobs::Kind::Delete
        } else {
            jobs::Kind::Recycle
        };
        self.jobs
            .start(kind, paths, None, Conflict::Skip, self.refresher());
    }

    pub fn rename(&self, path: &Path, new_name: &str) -> Result<PathBuf, CoreError> {
        let target = jobs::rename(path, new_name)?;
        self.view.refresh();
        Ok(target)
    }

    pub fn new_folder(&self) -> Result<PathBuf, CoreError> {
        let dir = self
            .nav
            .location()
            .filter(|p| !p.as_os_str().is_empty())
            .ok_or_else(|| CoreError("open a folder first".into()))?;
        let p = jobs::new_folder(dir)?;
        self.view.refresh();
        Ok(p)
    }

    pub fn cancel_job(&self, id: u64) {
        self.jobs.cancel(id);
    }

    pub fn prune_jobs(&self) {
        self.jobs.prune_finished();
    }

    /// Full path of a row, handy for the UI.
    pub fn path_of(e: &Entry) -> PathBuf {
        e.path()
    }
}

fn dummy_places() -> Vec<Place> {
    ["Home", "Desktop", "Documents", "Downloads", "Pictures"]
        .iter()
        .map(|n| Place {
            label: (*n).to_owned(),
            path: PathBuf::from(format!(r"C:\Users\dummy\{n}")),
        })
        .collect()
}

impl Drop for Core {
    fn drop(&mut self) {
        let _ = self.tx.send(Command::Stop);
        self.store.stop();
        self.images.stop();
    }
}
