//! SQLite schema and queries. Only the store thread touches the connection.

use std::{collections::HashMap, path::Path, path::PathBuf};

use rusqlite::{Connection, params};

use crate::model::{Bookmark, Sort, SortDir, SortKey};

use super::Snapshot;

const RECENTS: usize = 30;
const QUERIES: usize = 20;
const KEEP_VISITS: i64 = 500;

pub fn open(path: &Path) -> rusqlite::Result<Connection> {
    let conn = Connection::open(path)?;
    conn.execute_batch(
        "PRAGMA journal_mode = WAL;
         PRAGMA synchronous = NORMAL;
         CREATE TABLE IF NOT EXISTS visits (
             path TEXT PRIMARY KEY, at INTEGER NOT NULL, count INTEGER NOT NULL DEFAULT 1);
         CREATE TABLE IF NOT EXISTS bookmarks (
             path TEXT PRIMARY KEY, label TEXT NOT NULL, pos INTEGER NOT NULL);
         CREATE TABLE IF NOT EXISTS queries (
             text TEXT PRIMARY KEY, at INTEGER NOT NULL);
         CREATE TABLE IF NOT EXISTS folder_prefs (
             path TEXT PRIMARY KEY, sort_key TEXT NOT NULL, sort_dir TEXT NOT NULL);
         CREATE TABLE IF NOT EXISTS prefs (
             key TEXT PRIMARY KEY, value TEXT NOT NULL);",
    )?;
    Ok(conn)
}

pub fn memory() -> rusqlite::Result<Connection> {
    let conn = Connection::open_in_memory()?;
    conn.execute_batch(
        "CREATE TABLE visits (path TEXT PRIMARY KEY, at INTEGER NOT NULL, count INTEGER NOT NULL DEFAULT 1);
         CREATE TABLE bookmarks (path TEXT PRIMARY KEY, label TEXT NOT NULL, pos INTEGER NOT NULL);
         CREATE TABLE queries (text TEXT PRIMARY KEY, at INTEGER NOT NULL);
         CREATE TABLE folder_prefs (path TEXT PRIMARY KEY, sort_key TEXT NOT NULL, sort_dir TEXT NOT NULL);
         CREATE TABLE prefs (key TEXT PRIMARY KEY, value TEXT NOT NULL);",
    )?;
    Ok(conn)
}

fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// Case-insensitive key for paths (Windows).
pub fn norm(p: &Path) -> String {
    p.to_string_lossy().to_lowercase()
}

pub fn visit(c: &Connection, path: &Path) -> rusqlite::Result<()> {
    let p = path.to_string_lossy();
    c.execute(
        "INSERT INTO visits(path, at, count) VALUES(?1, ?2, 1)
         ON CONFLICT(path) DO UPDATE SET at = ?2, count = count + 1",
        params![p, now()],
    )?;
    c.execute(
        "DELETE FROM visits WHERE path NOT IN (SELECT path FROM visits ORDER BY at DESC LIMIT ?1)",
        params![KEEP_VISITS],
    )?;
    Ok(())
}

pub fn clear_visits(c: &Connection) -> rusqlite::Result<()> {
    c.execute("DELETE FROM visits", [])?;
    c.execute("DELETE FROM queries", [])?;
    Ok(())
}

pub fn add_query(c: &Connection, text: &str) -> rusqlite::Result<()> {
    c.execute(
        "INSERT INTO queries(text, at) VALUES(?1, ?2)
         ON CONFLICT(text) DO UPDATE SET at = ?2",
        params![text, now()],
    )?;
    c.execute(
        "DELETE FROM queries WHERE text NOT IN (SELECT text FROM queries ORDER BY at DESC LIMIT 100)",
        [],
    )?;
    Ok(())
}

/// Adds the bookmark, or removes it when it exists. Returns whether it is bookmarked now.
pub fn toggle_bookmark(c: &Connection, path: &Path, label: &str) -> rusqlite::Result<bool> {
    let p = path.to_string_lossy();
    let removed = c.execute(
        "DELETE FROM bookmarks WHERE lower(path) = lower(?1)",
        params![p],
    )?;
    if removed > 0 {
        return Ok(false);
    }
    let pos: i64 = c.query_row("SELECT COALESCE(MAX(pos), 0) + 1 FROM bookmarks", [], |r| {
        r.get(0)
    })?;
    c.execute(
        "INSERT INTO bookmarks(path, label, pos) VALUES(?1, ?2, ?3)",
        params![p, label, pos],
    )?;
    Ok(true)
}

pub fn remove_bookmark(c: &Connection, path: &Path) -> rusqlite::Result<()> {
    c.execute(
        "DELETE FROM bookmarks WHERE lower(path) = lower(?1)",
        params![path.to_string_lossy()],
    )?;
    Ok(())
}

pub fn set_pref(c: &Connection, key: &str, value: &str) -> rusqlite::Result<()> {
    c.execute(
        "INSERT INTO prefs(key, value) VALUES(?1, ?2)
         ON CONFLICT(key) DO UPDATE SET value = ?2",
        params![key, value],
    )?;
    Ok(())
}

pub fn set_folder_sort(c: &Connection, path: &Path, sort: Sort) -> rusqlite::Result<()> {
    c.execute(
        "INSERT INTO folder_prefs(path, sort_key, sort_dir) VALUES(?1, ?2, ?3)
         ON CONFLICT(path) DO UPDATE SET sort_key = ?2, sort_dir = ?3",
        params![norm(path), key_str(sort.key), dir_str(sort.dir)],
    )?;
    Ok(())
}

fn key_str(k: SortKey) -> &'static str {
    match k {
        SortKey::Name => "name",
        SortKey::Type => "type",
        SortKey::Modified => "modified",
        SortKey::Size => "size",
    }
}

fn dir_str(d: SortDir) -> &'static str {
    match d {
        SortDir::Asc => "asc",
        SortDir::Desc => "desc",
    }
}

pub fn parse_sort(key: &str, dir: &str) -> Option<Sort> {
    let key = match key {
        "name" => SortKey::Name,
        "type" => SortKey::Type,
        "modified" => SortKey::Modified,
        "size" => SortKey::Size,
        _ => return None,
    };
    let dir = if dir == "desc" {
        SortDir::Desc
    } else {
        SortDir::Asc
    };
    Some(Sort { key, dir })
}

pub fn snapshot(c: &Connection) -> rusqlite::Result<Snapshot> {
    let mut recents = Vec::new();
    {
        let mut st = c.prepare("SELECT path FROM visits ORDER BY at DESC LIMIT ?1")?;
        let rows = st.query_map(params![RECENTS as i64], |r| r.get::<_, String>(0))?;
        for p in rows.flatten() {
            recents.push(PathBuf::from(p));
        }
    }
    let mut bookmarks = Vec::new();
    {
        let mut st = c.prepare("SELECT path, label FROM bookmarks ORDER BY pos")?;
        let rows = st.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?;
        for (p, l) in rows.flatten() {
            bookmarks.push(Bookmark {
                path: PathBuf::from(p),
                label: l,
            });
        }
    }
    let mut queries = Vec::new();
    {
        let mut st = c.prepare("SELECT text FROM queries ORDER BY at DESC LIMIT ?1")?;
        let rows = st.query_map(params![QUERIES as i64], |r| r.get::<_, String>(0))?;
        queries.extend(rows.flatten());
    }
    let mut prefs = HashMap::new();
    {
        let mut st = c.prepare("SELECT key, value FROM prefs")?;
        let rows = st.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?;
        prefs.extend(rows.flatten());
    }
    let mut folder_sorts = HashMap::new();
    {
        let mut st = c.prepare("SELECT path, sort_key, sort_dir FROM folder_prefs")?;
        let rows = st.query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
            ))
        })?;
        for (p, k, d) in rows.flatten() {
            if let Some(s) = parse_sort(&k, &d) {
                folder_sorts.insert(p, s);
            }
        }
    }
    Ok(Snapshot {
        recents,
        bookmarks,
        queries,
        prefs,
        folder_sorts,
    })
}
