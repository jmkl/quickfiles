//! Deterministic fake file system for UI work and tests (`--dummy`). No Everything needed.
//!
//! Every folder has a stable, hash-derived number of sub-folders and files, so browsing is
//! repeatable; a few folders are huge on purpose to exercise paging.

use std::path::Path;
use std::sync::Arc;

use super::{Backend, Health, QueryResult};
use crate::{
    compose::{Only, Plan},
    fmt::Builder,
    model::{DriveInfo, Kind, SortDir},
    paging::PAGE,
};

const WORDS: [&str; 12] = [
    "alpha", "bravo", "cargo", "delta", "echo", "forge", "garnet", "harbor", "ivory", "jade",
    "kilo", "lumen",
];
const EXTS: [&str; 14] = [
    "txt", "rs", "json", "png", "jpg", "pdf", "docx", "zip", "mp4", "mp3", "html", "js", "toml",
    "md",
];
/// Size of every search result list.
const SEARCH_TOTAL: u32 = 3_000;
const BASE_TIME: i64 = 1_700_000_000;

fn hash(s: &str) -> u64 {
    let mut h = 0xcbf29ce484222325u64;
    for b in s.to_ascii_lowercase().bytes() {
        h ^= b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    h
}

/// (folders, files) inside `dir`.
fn shape(dir: &Path) -> (u32, u32) {
    let s = dir.to_string_lossy();
    let h = hash(&s);
    if h % 53 == 0 {
        return (30, 6_000); // a huge folder
    }
    if h % 17 == 0 {
        return (0, 0); // an empty one
    }
    ((h >> 8) as u32 % 25, (h >> 20) as u32 % 180)
}

fn folder_name(i: u32) -> String {
    format!("{}-{}", WORDS[i as usize % WORDS.len()], i)
}

fn file_name(i: u32) -> String {
    format!(
        "{}_{:04}.{}",
        WORDS[(i as usize * 7) % WORDS.len()],
        i,
        EXTS[(i as usize * 5) % EXTS.len()]
    )
}

pub struct Dummy {
    /// Pretend Everything is still loading its index for this many `health` calls.
    pub warmup: u32,
}

impl Dummy {
    pub fn new() -> Self {
        Self { warmup: 0 }
    }

    fn list(
        plan: &Plan,
        seg: usize,
        offset: u32,
        count: u32,
        b: &mut Builder,
    ) -> Result<QueryResult, String> {
        let only = plan.segs[seg].only;
        let desc = plan.sort.dir == SortDir::Desc;
        let (parent, nf, nfi): (String, u32, u32) = match &plan.children_of {
            Some(d) => {
                let (f, i) = shape(d);
                (d.to_string_lossy().into_owned(), f, i)
            }
            None => {
                let parent = plan
                    .scope
                    .as_ref()
                    .map(|p| p.to_string_lossy().into_owned())
                    .unwrap_or_else(|| r"C:\Users\dummy\work".to_owned());
                (parent, SEARCH_TOTAL / 10, SEARCH_TOTAL - SEARCH_TOTAL / 10)
            }
        };
        let browse = plan.children_of.is_some();
        let (kind, total) = match only {
            Only::Folders => (Kind::Folder, nf),
            Only::Files => (Kind::File, nfi),
            Only::Both => (Kind::File, nf + nfi), // search: mixed, folders are every 10th row
        };
        let needle = if browse {
            String::new()
        } else {
            plan.needle.to_lowercase()
        };
        let mut rows = Vec::new();
        let end = (offset + count).min(total);
        for pos in offset..end {
            let i = if desc { total - 1 - pos } else { pos };
            let (name, k) = match only {
                Only::Both => {
                    if i % 10 == 0 {
                        (folder_name(i), Kind::Folder)
                    } else {
                        (file_name(i), Kind::File)
                    }
                }
                _ if kind == Kind::Folder => (folder_name(i), Kind::Folder),
                _ => (file_name(i), Kind::File),
            };
            let name = if needle.is_empty() || browse {
                name
            } else {
                let stem = needle.trim_matches('"');
                match k {
                    Kind::Folder => format!("{stem}-{i}"),
                    _ => format!("{stem}_{i:04}.{}", EXTS[(i as usize * 5) % EXTS.len()]),
                }
            };
            let size = if k == Kind::Folder {
                0
            } else {
                100 + (i as u64 * 7919) % 90_000_000
            };
            let modified = BASE_TIME - (i as i64) * 3_600;
            let sub = if browse || plan.scope.is_some() {
                parent.clone()
            } else {
                format!(r"{parent}\proj{}", i % 40)
            };
            let flags = if i % 41 == 0 {
                crate::model::FLAG_HIDDEN
            } else {
                0
            };
            rows.push(b.entry(&sub, name, k, size, modified, flags, 0));
        }
        Ok(QueryResult { total, rows })
    }
}

impl Default for Dummy {
    fn default() -> Self {
        Self::new()
    }
}

impl Backend for Dummy {
    fn name(&self) -> &'static str {
        "dummy"
    }

    fn connect(&mut self) -> Result<(), String> {
        Ok(())
    }

    fn health(&mut self) -> Health {
        if self.warmup > 0 {
            self.warmup -= 1;
            Health::Loading
        } else {
            Health::Ready
        }
    }

    fn query(
        &mut self,
        plan: &Plan,
        seg: usize,
        offset: u32,
        count: u32,
        builder: &mut Builder,
    ) -> Result<QueryResult, String> {
        debug_assert!(count <= PAGE);
        Self::list(plan, seg, offset, count, builder)
    }

    fn count_children(&mut self, dir: &Path) -> Result<(u32, u32), String> {
        Ok(shape(dir))
    }
}

pub fn dummy_drives() -> Arc<[DriveInfo]> {
    const GB: u64 = 1 << 30;
    let d = |label: &str, root: &str, total: u64, free: u64, system: bool| DriveInfo {
        label: label.into(),
        root: root.into(),
        total: total * GB,
        free: free * GB,
        used: (total - free) as f32 / total as f32,
        system,
        fs: "NTFS".into(),
    };
    Arc::from(vec![
        d("Windows (C:)", r"C:\", 476, 112, true),
        d("Data (D:)", r"D:\", 1863, 1210, false),
        d("Backup (E:)", r"E:\", 931, 44, false),
    ])
}
