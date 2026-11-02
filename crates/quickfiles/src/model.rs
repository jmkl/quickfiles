//! Plain data types shared between the UI and the worker thread. No egui here.

use std::{
    borrow::Cow,
    path::{Component, Path, PathBuf},
};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum EntryKind {
    Folder,
    Html,
    Script,
    Json,
    Other,
}

#[derive(Debug)]
pub struct FileEntry {
    pub name: String,
    /// Lower-cased name, computed once on the worker thread (cheap sorting + filtering).
    pub key: String,
    pub kind: EntryKind,
    pub type_label: &'static str,
    pub modified: String, // ISO-like, so plain string comparison sorts chronologically
    pub folders: Option<u32>,
    pub files: Option<u32>,
    pub size_bytes: u64,
}

impl FileEntry {
    pub fn is_dir(&self) -> bool {
        self.kind == EntryKind::Folder
    }
}

/// Result of scanning one directory.
#[derive(Debug)]
pub struct Listing {
    pub entries: Vec<FileEntry>,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SortKey {
    Name,
    Type,
    Modified,
    Size,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SortDir {
    Asc,
    Desc,
}

#[derive(Clone, Copy, Debug)]
pub struct Sort {
    pub key: SortKey,
    pub dir: SortDir,
}

impl Sort {
    pub fn toggle(&mut self, key: SortKey) {
        if self.key == key {
            self.dir = match self.dir {
                SortDir::Asc => SortDir::Desc,
                SortDir::Desc => SortDir::Asc,
            };
        } else {
            *self = Self {
                key,
                dir: SortDir::Asc,
            };
        }
    }
}

// --- Sidebar --------------------------------------------------------------------------------

#[derive(Clone, Copy)]
pub enum SideKind {
    Folder,
    Drive,
    Pc,
}

#[derive(Clone, Copy)]
pub struct SideItem {
    pub label: &'static str,
    pub kind: SideKind,
}

#[derive(Clone, Copy)]
pub struct DriveInfo {
    pub label: &'static str,
    /// 0.0..=1.0
    pub used: f32,
    pub system: bool,
}

impl DriveInfo {
    pub fn nearly_full(&self) -> bool {
        self.used >= 0.9
    }
}

// --- Formatting -----------------------------------------------------------------------------

pub fn format_size(bytes: u64) -> String {
    const UNITS: [&str; 4] = ["B", "KB", "MB", "GB"];
    if bytes < 1024 {
        return format!("{bytes} B");
    }
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    let text = format!("{value:.1}");
    let text = text.strip_suffix(".0").unwrap_or(&text);
    format!("{text} {}", UNITS[unit])
}

pub fn count_text(n: Option<u32>) -> Cow<'static, str> {
    match n {
        Some(n) => Cow::Owned(n.to_string()),
        None => Cow::Borrowed("--"),
    }
}

// --- Paths ----------------------------------------------------------------------------------
//
// Location model: a `PathBuf`. The empty path is the virtual root "This PC" (the drive list).

/// Label of the virtual root.
pub const ROOT_LABEL: &str = "This PC";

/// `(label, target)` for every segment of `path`, from the first to the current one.
/// A Windows prefix and its root (`C:` + `\`) form one segment: label `C:`, target `C:\`.
/// The empty path yields the single segment "This PC".
pub fn breadcrumbs(path: &Path) -> Vec<(String, PathBuf)> {
    let mut crumbs: Vec<(String, PathBuf)> = Vec::new();
    let mut acc = PathBuf::new();
    for component in path.components() {
        acc.push(component.as_os_str());
        match component {
            Component::RootDir => match crumbs.last_mut() {
                Some(prefix) => prefix.1 = acc.clone(), // `C:` becomes `C:\`
                None => crumbs.push(("/".to_owned(), acc.clone())),
            },
            other => crumbs.push((
                other.as_os_str().to_string_lossy().into_owned(),
                acc.clone(),
            )),
        }
    }
    if crumbs.is_empty() {
        crumbs.push((ROOT_LABEL.to_owned(), PathBuf::new()));
    }
    crumbs
}

/// Short name of the folder at `path` (last segment).
pub fn display_name(path: &Path) -> String {
    breadcrumbs(path)
        .pop()
        .map_or_else(|| ROOT_LABEL.to_owned(), |(label, _)| label)
}

/// The folder above `path`; `None` at the virtual root. A drive root goes up to "This PC".
pub fn parent_of(path: &Path) -> Option<PathBuf> {
    if path.as_os_str().is_empty() {
        return None;
    }
    Some(path.parent().map(Path::to_path_buf).unwrap_or_default())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_path_is_this_pc() {
        assert_eq!(breadcrumbs(Path::new("")), vec![(ROOT_LABEL.to_owned(), PathBuf::new())]);
        assert_eq!(display_name(Path::new("")), ROOT_LABEL);
        assert_eq!(parent_of(Path::new("")), None);
    }

    #[test]
    fn breadcrumbs_accumulate_targets() {
        let crumbs = breadcrumbs(Path::new("a/b/c"));
        let labels: Vec<_> = crumbs.iter().map(|(l, _)| l.as_str()).collect();
        assert_eq!(labels, ["a", "b", "c"]);
        assert_eq!(crumbs[1].1, PathBuf::from("a/b"));
        assert_eq!(display_name(Path::new("a/b/c")), "c");
    }

    #[test]
    fn parent_of_top_level_is_this_pc() {
        assert_eq!(parent_of(Path::new("a/b")), Some(PathBuf::from("a")));
        assert_eq!(parent_of(Path::new("a")), Some(PathBuf::new()));
    }
}
