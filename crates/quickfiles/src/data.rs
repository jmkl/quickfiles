//! Dummy data. Replace these functions with real file-system / drive queries later;
//! nothing else in the app needs to change.

use std::{
    path::{Path, PathBuf},
    thread,
};

use crate::model::{DriveInfo, EntryKind, FileEntry, Listing, SideItem, SideKind};

pub const DEFAULT_PATH: [&str; 6] = [
    "Roaming",
    "Adobe",
    "UXP",
    "Plugins",
    "External",
    "hello.dcsms.quickrestore_1.0.0",
];

/// Where the app starts.
pub fn default_path() -> PathBuf {
    DEFAULT_PATH.iter().collect()
}

const fn folder(label: &'static str) -> SideItem {
    SideItem {
        label,
        kind: SideKind::Folder,
    }
}

pub const RECENTS: [SideItem; 10] = [
    folder("_CODE"),
    folder("ff"),
    folder("rust"),
    folder("crates"),
    SideItem {
        label: "CACHE (E:)",
        kind: SideKind::Drive,
    },
    SideItem {
        label: "This PC",
        kind: SideKind::Pc,
    },
    folder("rembg"),
    folder("compnode"),
    folder("src"),
    folder("UXP"),
];

pub const BOOKMARKS: [SideItem; 1] = [folder("crates")];

pub const PLACES: [SideItem; 2] = [
    SideItem {
        label: "This PC",
        kind: SideKind::Pc,
    },
    folder("Documents"),
];

pub const DRIVES: [DriveInfo; 7] = [
    DriveInfo {
        label: "Windows 10 (C:)",
        used: 0.834,
        system: true,
    },
    DriveInfo {
        label: "DATA DUMP (D:)",
        used: 0.939,
        system: false,
    },
    DriveInfo {
        label: "CACHE (E:)",
        used: 0.534,
        system: false,
    },
    DriveInfo {
        label: "Data (F:)",
        used: 0.975,
        system: false,
    },
    DriveInfo {
        label: "Data 2 (G:)",
        used: 0.933,
        system: false,
    },
    DriveInfo {
        label: "DOWNLOAD - SOFTWARE (H:)",
        used: 0.982,
        system: false,
    },
    DriveInfo {
        label: "RK PRODUCTION (I:)",
        used: 0.920,
        system: false,
    },
];

// --- Directory listings -----------------------------------------------------------------------

fn entry(
    name: &str,
    kind: EntryKind,
    type_label: &'static str,
    modified: &str,
    files: Option<u32>,
    size_bytes: u64,
) -> FileEntry {
    FileEntry {
        name: name.to_owned(),
        key: name.to_lowercase(),
        kind,
        type_label,
        modified: modified.to_owned(),
        folders: None,
        files,
        size_bytes,
    }
}

/// Runs on the worker thread.
pub fn build_listing(path: &Path) -> Listing {
    let parts: Vec<String> = path
        .components()
        .map(|c| c.as_os_str().to_string_lossy().into_owned())
        .collect();
    let is_default = parts.iter().map(String::as_str).eq(DEFAULT_PATH);

    let entries = if path.as_os_str().is_empty() {
        // Virtual root "This PC": one entry per drive.
        DRIVES
            .iter()
            .map(|d| entry(d.label, EntryKind::Folder, "Local Disk", "", None, 0))
            .collect::<Vec<_>>()
    } else if is_default {
        let entries = vec![
            entry(
                "icons",
                EntryKind::Folder,
                "File folder",
                "2026-10-31 15:36",
                Some(1),
                2_560,
            ),
            entry(
                "index.html",
                EntryKind::Html,
                "Helium HTML Document",
                "2026-11-01 13:30",
                None,
                1_024,
            ),
            entry(
                "index.js",
                EntryKind::Script,
                "JavaScript Source File",
                "2026-11-01 13:30",
                None,
                19_354,
            ),
            entry(
                "manifest.json",
                EntryKind::Json,
                "JSON File",
                "2026-11-01 13:30",
                None,
                1_638,
            ),
        ];
        entries
    } else {
        let seed = hash_path(path);
        // 20..=20_000 entries so the virtualised list and the parallel generator get exercised.
        let count = 20 + (mix(seed) % 19_981) as usize;
        generate_entries(seed, count)
    };

    Listing { entries }
}

/// Splits big listings across all cores. `make_entry` is a pure function of (seed, index),
/// so chunks are independent and the result is deterministic.
fn generate_entries(seed: u64, count: usize) -> Vec<FileEntry> {
    let threads = thread::available_parallelism()
        .map_or(1, |n| n.get())
        .min(8);
    if count < 4_096 || threads == 1 {
        return (0..count).map(|i| make_entry(seed, i)).collect();
    }

    let chunk = count.div_ceil(threads);
    thread::scope(|scope| {
        let handles: Vec<_> = (0..count)
            .step_by(chunk)
            .map(|start| {
                let end = (start + chunk).min(count);
                scope.spawn(move || {
                    (start..end)
                        .map(|i| make_entry(seed, i))
                        .collect::<Vec<_>>()
                })
            })
            .collect();
        handles
            .into_iter()
            .flat_map(|h| h.join().expect("generator thread panicked"))
            .collect()
    })
}

fn make_entry(seed: u64, i: usize) -> FileEntry {
    const EXTS: [(&str, EntryKind, &str); 7] = [
        ("html", EntryKind::Html, "Helium HTML Document"),
        ("js", EntryKind::Script, "JavaScript Source File"),
        ("json", EntryKind::Json, "JSON File"),
        ("rs", EntryKind::Other, "Rust Source File"),
        ("toml", EntryKind::Other, "TOML File"),
        ("png", EntryKind::Other, "PNG File"),
        ("txt", EntryKind::Other, "Text Document"),
    ];

    let h = mix(seed ^ (i as u64).wrapping_mul(0x2545_F491_4F6C_DD1D));
    let modified = format!(
        "2026-{:02}-{:02} {:02}:{:02}",
        10 + (h >> 12) % 2,
        1 + (h >> 16) % 28,
        (h >> 24) % 24,
        (h >> 32) % 60
    );

    let name = if h % 5 == 0 {
        format!("folder_{i:05}")
    } else {
        let (ext, ..) = EXTS[((h >> 40) % EXTS.len() as u64) as usize];
        format!("file_{i:05}.{ext}")
    };

    if h % 5 == 0 {
        FileEntry {
            key: name.to_lowercase(),
            name,
            kind: EntryKind::Folder,
            type_label: "File folder",
            modified,
            folders: Some(((h >> 48) % 6) as u32),
            files: Some(((h >> 52) % 40) as u32),
            size_bytes: (h >> 8) % 400_000,
        }
    } else {
        let (_, kind, type_label) = EXTS[((h >> 40) % EXTS.len() as u64) as usize];
        FileEntry {
            key: name.to_lowercase(),
            name,
            kind,
            type_label,
            modified,
            folders: None,
            files: None,
            size_bytes: (h >> 8) % 5_000_000,
        }
    }
}

fn hash_path(path: &Path) -> u64 {
    path.to_string_lossy()
        .bytes()
        .chain(std::iter::once(b'/'))
        .fold(0xcbf2_9ce4_8422_2325, |h, b| {
            (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3)
        })
}

/// SplitMix64.
fn mix(z: u64) -> u64 {
    let mut z = z.wrapping_add(0x9E37_79B9_7F4A_7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}
