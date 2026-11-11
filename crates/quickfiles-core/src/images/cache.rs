//! On-disk thumbnail cache: one SQLite file (`thumbs.db`) instead of a PNG per thumbnail.
//!
//! * Pixels are stored raw (96 px thumbnails are ~37 KB): no encoding on the hot path.
//! * Writes go through a queue to a background thread that batches them into transactions, so a
//!   decode job never waits for the disk.
//! * `last_used` gives a real LRU; hits refresh it lazily (and in the background).
//! * `PRAGMA user_version` is the cache format version: a mismatch drops the old content.
//! * Every failure degrades to "no disk cache"; thumbnails are simply decoded again.

use std::{
    fs,
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering::Relaxed},
        mpsc::{Receiver, SyncSender, TryRecvError, sync_channel},
    },
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use rusqlite::{Connection, OpenFlags, params};

use super::{Pixels, stats::Stats};

/// Bump when the stored format changes: the old content is dropped on the next start.
pub const CACHE_VERSION: i32 = 1;
pub const DEFAULT_BUDGET_MB: u64 = 512;
/// A hit refreshes `last_used` only when it is older than this.
const TOUCH_AFTER_SECS: i64 = 60 * 60;
/// Queue depth of the write-behind thread. When full, new thumbnails are simply not cached.
const QUEUE: usize = 512;
const BATCH: usize = 64;
const BATCH_WAIT: Duration = Duration::from_millis(40);

/// What to do with an existing database, given its `user_version`.
#[derive(Debug, PartialEq, Eq)]
pub enum OpenPlan {
    Keep,
    /// Wrong version (older or newer): clear it and stamp the current one.
    Reset,
}

pub fn plan_open(found: i32) -> OpenPlan {
    if found == CACHE_VERSION {
        OpenPlan::Keep
    } else {
        OpenPlan::Reset
    }
}

/// Which rows to delete (oldest first in `rows`: `(key, bytes)`) so that `total` drops to 3/4
/// of `budget`. Empty when the cache is within budget.
pub fn prune_plan(rows: impl IntoIterator<Item = (i64, u64)>, total: u64, budget: u64) -> Vec<i64> {
    if total <= budget {
        return Vec::new();
    }
    let target = budget / 4 * 3;
    let mut left = total;
    let mut out = Vec::new();
    for (key, bytes) in rows {
        if left <= target {
            break;
        }
        out.push(key);
        left = left.saturating_sub(bytes);
    }
    out
}

/// Cache key of a thumbnail: path (case-insensitive), modification time, size and pixel size.
pub fn key(path: &str, modified: i64, size: u64, px: u32) -> i64 {
    let mut h = 0xcbf29ce484222325u64;
    for b in path
        .bytes()
        .map(|b| b.to_ascii_lowercase())
        .chain(modified.to_le_bytes())
        .chain(size.to_le_bytes())
        .chain(px.to_le_bytes())
    {
        h ^= b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    h as i64
}

enum Msg {
    Put {
        key: i64,
        w: u32,
        h: u32,
        data: Vec<u8>,
    },
    Touch(i64),
    Clear,
    Stop,
}

struct Inner {
    read: Mutex<Connection>,
    tx: SyncSender<Msg>,
    bytes: Arc<AtomicU64>,
    budget: Arc<AtomicU64>,
    pending: Arc<AtomicU64>,
}

/// Cheap to share; `None` inside means the cache is unavailable (everything is a no-op).
pub struct DiskCache {
    inner: Option<Inner>,
}

fn now_secs() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64)
}

fn open_conn(path: &Path) -> rusqlite::Result<Connection> {
    let conn = Connection::open(path)?;
    // auto_vacuum only takes effect on a new file; it lets pruning give disk space back.
    conn.execute_batch(
        "PRAGMA auto_vacuum = INCREMENTAL;
         PRAGMA journal_mode = WAL;
         PRAGMA synchronous = OFF;",
    )?;
    conn.busy_timeout(Duration::from_millis(500))?;
    let found: i32 = conn.query_row("PRAGMA user_version", [], |r| r.get(0))?;
    if plan_open(found) == OpenPlan::Reset {
        conn.execute_batch("DROP TABLE IF EXISTS thumbs;")?;
    }
    conn.execute_batch(&format!(
        "CREATE TABLE IF NOT EXISTS thumbs (
             key INTEGER PRIMARY KEY,
             w INTEGER NOT NULL,
             h INTEGER NOT NULL,
             last_used INTEGER NOT NULL,
             data BLOB NOT NULL);
         CREATE INDEX IF NOT EXISTS thumbs_lru ON thumbs (last_used);
         PRAGMA user_version = {CACHE_VERSION};"
    ))?;
    Ok(conn)
}

fn open_pair(path: &Path) -> Option<(Connection, Connection)> {
    let writer = match open_conn(path) {
        Ok(c) => c,
        Err(_) => {
            // Corrupt or locked: move it aside once and start over.
            let _ = fs::rename(path, path.with_extension("db.corrupt"));
            open_conn(path).ok()?
        }
    };
    let reader = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .ok()?;
    Some((writer, reader))
}

impl DiskCache {
    pub fn open(dir: &Path, budget_mb: u64, stats: Arc<Stats>) -> Self {
        let _ = fs::create_dir_all(dir);
        // The one-PNG-per-thumbnail cache of earlier versions.
        let legacy: PathBuf = dir.join("thumbs");
        if legacy.is_dir() {
            thread::spawn(move || {
                let _ = fs::remove_dir_all(legacy);
            });
        }
        let Some((writer, reader)) = open_pair(&dir.join("thumbs.db")) else {
            return Self { inner: None };
        };
        let bytes = Arc::new(AtomicU64::new(db_bytes(&writer)));
        let budget = Arc::new(AtomicU64::new(budget_mb.max(16) * 1024 * 1024));
        let pending = Arc::new(AtomicU64::new(0));
        let (tx, rx) = sync_channel(QUEUE);
        {
            let (b, bu, p) = (bytes.clone(), budget.clone(), pending.clone());
            let spawned = thread::Builder::new()
                .name("qf-thumb-db".to_owned())
                .spawn(move || writer_loop(writer, rx, stats, b, bu, p));
            if spawned.is_err() {
                return Self { inner: None };
            }
        }
        Self {
            inner: Some(Inner {
                read: Mutex::new(reader),
                tx,
                bytes,
                budget,
                pending,
            }),
        }
    }

    pub fn enabled(&self) -> bool {
        self.inner.is_some()
    }

    /// The stored thumbnail (straight alpha), if any.
    pub fn get(&self, key: i64) -> Option<Pixels> {
        let inner = self.inner.as_ref()?;
        let (w, h, last, data): (u32, u32, i64, Vec<u8>) = {
            let conn = inner.read.lock().unwrap_or_else(|e| e.into_inner());
            let mut st = conn
                .prepare_cached("SELECT w, h, last_used, data FROM thumbs WHERE key = ?1")
                .ok()?;
            st.query_row(params![key], |r| {
                Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?))
            })
            .ok()?
        };
        if data.len() != w as usize * h as usize * 4 {
            return None;
        }
        if now_secs() - last > TOUCH_AFTER_SECS {
            let _ = inner.tx.try_send(Msg::Touch(key));
        }
        Some(Pixels::new(w, h, data))
    }

    /// Queues a thumbnail for writing. Never blocks; dropped when the queue is full.
    pub fn put(&self, key: i64, px: &Pixels) {
        if let Some(inner) = &self.inner {
            let msg = Msg::Put {
                key,
                w: px.w,
                h: px.h,
                data: px.rgba.clone(),
            };
            if inner.tx.try_send(msg).is_ok() {
                inner.pending.fetch_add(1, Relaxed);
            }
        }
    }

    /// Delete every cached thumbnail.
    pub fn clear(&self) {
        if let Some(inner) = &self.inner {
            let _ = inner.tx.send(Msg::Clear);
        }
    }

    pub fn set_budget_mb(&self, mb: u64) {
        if let Some(inner) = &self.inner {
            inner.budget.store(mb.max(16) * 1024 * 1024, Relaxed);
        }
    }

    /// Approximate size of the stored thumbnails.
    pub fn bytes(&self) -> u64 {
        self.inner.as_ref().map_or(0, |i| i.bytes.load(Relaxed))
    }

    /// Writes queued and not yet committed.
    pub fn pending(&self) -> u64 {
        self.inner.as_ref().map_or(0, |i| i.pending.load(Relaxed))
    }

    pub fn stop(&self) {
        if let Some(inner) = &self.inner {
            let _ = inner.tx.try_send(Msg::Stop);
        }
    }
}

fn db_bytes(conn: &Connection) -> u64 {
    conn.query_row(
        "SELECT COALESCE(SUM(length(data)), 0) FROM thumbs",
        [],
        |r| r.get::<_, i64>(0),
    )
    .map_or(0, |n| n.max(0) as u64)
}

fn writer_loop(
    mut conn: Connection,
    rx: Receiver<Msg>,
    stats: Arc<Stats>,
    bytes: Arc<AtomicU64>,
    budget: Arc<AtomicU64>,
    pending: Arc<AtomicU64>,
) {
    prune(&conn, &bytes, budget.load(Relaxed));
    let mut since_prune = 0u64;
    loop {
        let Ok(first) = rx.recv() else { return };
        let mut batch = vec![first];
        let deadline = Instant::now() + BATCH_WAIT;
        while batch.len() < BATCH {
            match rx.try_recv() {
                Ok(m) => batch.push(m),
                Err(TryRecvError::Empty) => {
                    if Instant::now() >= deadline {
                        break;
                    }
                    thread::sleep(Duration::from_millis(2));
                }
                Err(TryRecvError::Disconnected) => break,
            }
        }
        let started = Instant::now();
        let (mut puts, mut stop, mut cleared) = (0u64, false, false);
        let now = now_secs();
        if let Ok(tx) = conn.transaction() {
            for m in batch {
                match m {
                    Msg::Put { key, w, h, data } => {
                        puts += 1;
                        let len = data.len() as u64;
                        let ok = tx
                            .prepare_cached(
                                "INSERT OR REPLACE INTO thumbs (key, w, h, last_used, data)
                                 VALUES (?1, ?2, ?3, ?4, ?5)",
                            )
                            .and_then(|mut st| st.execute(params![key, w, h, now, data]))
                            .is_ok();
                        if ok {
                            bytes.fetch_add(len, Relaxed);
                            since_prune += len;
                        }
                    }
                    Msg::Touch(key) => {
                        let _ = tx.execute(
                            "UPDATE thumbs SET last_used = ?1 WHERE key = ?2",
                            params![now, key],
                        );
                    }
                    Msg::Clear => {
                        let _ = tx.execute("DELETE FROM thumbs", []);
                        bytes.store(0, Relaxed);
                        cleared = true;
                    }
                    Msg::Stop => stop = true,
                }
            }
            let _ = tx.commit();
        }
        if puts > 0 {
            pending.fetch_sub(puts.min(pending.load(Relaxed)), Relaxed);
            stats.disk_write_batch(puts, started.elapsed());
        }
        if cleared {
            let _ = conn.execute_batch("PRAGMA incremental_vacuum;");
        }
        let limit = budget.load(Relaxed);
        if since_prune > limit / 8 || bytes.load(Relaxed) > limit + limit / 4 {
            prune(&conn, &bytes, limit);
            since_prune = 0;
        }
        if stop {
            return;
        }
    }
}

/// Delete the least recently used rows until the cache is at 3/4 of its budget.
fn prune(conn: &Connection, bytes: &AtomicU64, budget: u64) {
    let total = db_bytes(conn);
    bytes.store(total, Relaxed);
    if total <= budget {
        return;
    }
    let victims = {
        let Ok(mut st) = conn.prepare("SELECT key, length(data) FROM thumbs ORDER BY last_used")
        else {
            return;
        };
        let Ok(rows) = st.query_map([], |r| {
            Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)? as u64))
        }) else {
            return;
        };
        prune_plan(rows.flatten(), total, budget)
    };
    if victims.is_empty() {
        return;
    }
    if let Ok(tx) = conn.unchecked_transaction() {
        for k in victims {
            let _ = tx.execute("DELETE FROM thumbs WHERE key = ?1", params![k]);
        }
        let _ = tx.commit();
    }
    let _ = conn.execute_batch("PRAGMA incremental_vacuum;");
    bytes.store(db_bytes(conn), Relaxed);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn key_is_stable_and_case_insensitive() {
        // Golden value: changing the key function silently orphans every cached thumbnail.
        assert_eq!(
            key("C:\\Pics\\a.jpg", 5, 7, 96),
            key("c:\\pics\\A.JPG", 5, 7, 96)
        );
        assert_eq!(key("C:\\Pics\\a.jpg", 5, 7, 96), -8_407_818_687_012_825_827);
    }

    #[test]
    fn key_changes_with_every_part() {
        let base = key("a", 1, 2, 96);
        assert_ne!(base, key("b", 1, 2, 96));
        assert_ne!(base, key("a", 2, 2, 96));
        assert_ne!(base, key("a", 1, 3, 96));
        assert_ne!(base, key("a", 1, 2, 128));
    }

    #[test]
    fn a_version_bump_resets_the_cache() {
        assert_eq!(plan_open(CACHE_VERSION), OpenPlan::Keep);
        assert_eq!(plan_open(CACHE_VERSION - 1), OpenPlan::Reset);
        assert_eq!(plan_open(CACHE_VERSION + 1), OpenPlan::Reset); // written by a newer build
        assert_eq!(plan_open(0), OpenPlan::Reset); // a brand new file
    }

    #[test]
    fn prune_removes_oldest_down_to_three_quarters() {
        let rows = [(1, 40), (2, 40), (3, 40), (4, 40)]; // oldest first, 160 total
        assert_eq!(prune_plan(rows, 160, 200), Vec::<i64>::new());
        // budget 100: target 75 -> must drop 85+ bytes: three rows
        assert_eq!(prune_plan(rows, 160, 100), vec![1, 2, 3]);
    }

    #[test]
    fn prune_handles_empty_and_exact() {
        assert!(prune_plan([], 0, 10).is_empty());
        assert!(prune_plan([(1, 10)], 10, 10).is_empty());
        assert_eq!(prune_plan([(1, 100)], 100, 10), vec![1]);
    }
}
