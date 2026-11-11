//! Persistent state: recent folders, bookmarks, search history, per-folder sort, preferences.
//!
//! A dedicated thread owns the SQLite connection. Writes are fire-and-forget messages; the UI
//! reads an immutable [`Snapshot`] (`Arc`, swapped after every write), so reading never blocks
//! and never touches the disk.

mod db;

use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::{
        Arc, RwLock,
        mpsc::{Receiver, Sender, channel},
    },
    thread,
};

use crate::{
    engine::Wake,
    model::{Bookmark, Sort},
};

#[derive(Clone, Debug, Default)]
pub struct Snapshot {
    /// Newest first.
    pub recents: Vec<PathBuf>,
    pub bookmarks: Vec<Bookmark>,
    /// Newest first.
    pub queries: Vec<String>,
    pub prefs: HashMap<String, String>,
    folder_sorts: HashMap<String, Sort>,
}

impl Snapshot {
    pub fn is_bookmarked(&self, path: &Path) -> bool {
        let n = db::norm(path);
        self.bookmarks.iter().any(|b| db::norm(&b.path) == n)
    }

    pub fn folder_sort(&self, path: &Path) -> Option<Sort> {
        self.folder_sorts.get(&db::norm(path)).copied()
    }

    pub fn pref(&self, key: &str) -> Option<&str> {
        self.prefs.get(key).map(String::as_str)
    }

    pub fn pref_bool(&self, key: &str, default: bool) -> bool {
        match self.pref(key) {
            Some("1") => true,
            Some("0") => false,
            _ => default,
        }
    }

    pub fn pref_f32(&self, key: &str) -> Option<f32> {
        self.pref(key).and_then(|v| v.parse().ok())
    }
}

enum Msg {
    Visit(PathBuf),
    Query(String),
    ToggleBookmark(PathBuf, String),
    RemoveBookmark(PathBuf),
    FolderSort(PathBuf, Sort),
    Pref(String, String),
    ClearHistory,
    Stop,
}

#[derive(Clone)]
pub struct Store {
    tx: Sender<Msg>,
    snap: Arc<RwLock<Arc<Snapshot>>>,
}

impl Store {
    pub fn open(path: &Path, wake: Wake) -> Self {
        let snap = Arc::new(RwLock::new(Arc::new(Snapshot::default())));
        let (tx, rx) = channel();
        let (ready_tx, ready_rx) = channel();
        let (snap2, path) = (snap.clone(), path.to_owned());
        thread::Builder::new()
            .name("qf-store".into())
            .spawn(move || run(path, rx, snap2, wake, ready_tx))
            .expect("failed to spawn the store thread");
        // The first snapshot carries the saved preferences: wait for it (briefly).
        let _ = ready_rx.recv_timeout(std::time::Duration::from_secs(2));
        Self { tx, snap }
    }

    pub fn snapshot(&self) -> Arc<Snapshot> {
        self.snap.read().unwrap_or_else(|e| e.into_inner()).clone()
    }

    pub fn visit(&self, path: &Path) {
        // This PC (the empty path) is not worth remembering.
        if !path.as_os_str().is_empty() {
            let _ = self.tx.send(Msg::Visit(path.to_owned()));
        }
    }

    pub fn add_query(&self, text: &str) {
        let t = text.trim();
        if t.len() >= 2 {
            let _ = self.tx.send(Msg::Query(t.to_owned()));
        }
    }

    pub fn toggle_bookmark(&self, path: &Path, label: &str) {
        let _ = self
            .tx
            .send(Msg::ToggleBookmark(path.to_owned(), label.to_owned()));
    }

    pub fn remove_bookmark(&self, path: &Path) {
        let _ = self.tx.send(Msg::RemoveBookmark(path.to_owned()));
    }

    pub fn set_folder_sort(&self, path: &Path, sort: Sort) {
        let _ = self.tx.send(Msg::FolderSort(path.to_owned(), sort));
    }

    pub fn set_pref(&self, key: &str, value: &str) {
        let _ = self.tx.send(Msg::Pref(key.to_owned(), value.to_owned()));
    }

    pub fn set_pref_bool(&self, key: &str, value: bool) {
        self.set_pref(key, if value { "1" } else { "0" });
    }

    pub fn clear_history(&self) {
        let _ = self.tx.send(Msg::ClearHistory);
    }

    pub(crate) fn stop(&self) {
        let _ = self.tx.send(Msg::Stop);
    }
}

/// Opens the database; a corrupt file is moved aside and replaced, and as a last resort the
/// store works in memory so the app still runs.
fn open_db(path: &Path) -> rusqlite::Connection {
    match db::open(path) {
        Ok(c) => c,
        Err(_) => {
            let _ = std::fs::rename(path, path.with_extension("db.corrupt"));
            db::open(path)
                .or_else(|_| db::memory())
                .expect("in-memory sqlite")
        }
    }
}

fn run(
    path: PathBuf,
    rx: Receiver<Msg>,
    snap: Arc<RwLock<Arc<Snapshot>>>,
    wake: Wake,
    ready: Sender<()>,
) {
    let conn = open_db(&path);
    let publish = |conn: &rusqlite::Connection| {
        if let Ok(s) = db::snapshot(conn) {
            *snap.write().unwrap_or_else(|e| e.into_inner()) = Arc::new(s);
            wake();
        }
    };
    publish(&conn);
    let _ = ready.send(());
    while let Ok(first) = rx.recv() {
        let mut batch = vec![first];
        while let Ok(m) = rx.try_recv() {
            batch.push(m);
        }
        let mut stop = false;
        for m in batch {
            let _ = match m {
                Msg::Visit(p) => db::visit(&conn, &p),
                Msg::Query(t) => db::add_query(&conn, &t),
                Msg::ToggleBookmark(p, l) => db::toggle_bookmark(&conn, &p, &l).map(|_| ()),
                Msg::RemoveBookmark(p) => db::remove_bookmark(&conn, &p),
                Msg::FolderSort(p, s) => db::set_folder_sort(&conn, &p, s),
                Msg::Pref(k, v) => db::set_pref(&conn, &k, &v),
                Msg::ClearHistory => db::clear_visits(&conn),
                Msg::Stop => {
                    stop = true;
                    Ok(())
                }
            };
        }
        publish(&conn);
        if stop {
            return;
        }
    }
}
