//! Plain data types shared by the core and the UI. Std only, no I/O.

use std::{
    borrow::Cow,
    path::{Component, Path, PathBuf},
    sync::Arc,
};

/// Label of the virtual root (the drive list). Its path is the empty path.
pub const ROOT_LABEL: &str = "This PC";

// --- Sorting --------------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum SortKey {
    Name,
    Type,
    Modified,
    Size,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum SortDir {
    Asc,
    Desc,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct Sort {
    pub key: SortKey,
    pub dir: SortDir,
}

impl Sort {
    pub const DEFAULT: Sort = Sort {
        key: SortKey::Name,
        dir: SortDir::Asc,
    };

    /// Clicking the active column flips the direction; a new column starts ascending.
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

impl Default for Sort {
    fn default() -> Self {
        Self::DEFAULT
    }
}

// --- Entries --------------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Kind {
    File,
    Folder,
    /// A drive in the "This PC" listing. `Entry::parent` holds its root path (`C:\`).
    Drive,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Category {
    Folder,
    Drive,
    Html,
    Script,
    Json,
    Image,
    Video,
    Audio,
    Archive,
    Code,
    Text,
    Pdf,
    Document,
    Other,
}

pub const FLAG_HIDDEN: u8 = 1;
pub const FLAG_SYSTEM: u8 = 2;
pub const FLAG_READONLY: u8 = 4;

/// One row of a listing. Raw values for sorting/logic, display strings built once on the
/// worker thread so that drawing a frame never formats or allocates.
#[derive(Clone, Debug)]
pub struct Entry {
    /// Stable identity (hash of the lower-cased full path). Selection and focus use this.
    pub key: u64,
    pub name: Box<str>,
    /// Containing folder (shared between rows of the same folder). For drives: the root.
    pub parent: Arc<str>,
    pub kind: Kind,
    pub category: Category,
    pub flags: u8,
    pub size: u64,
    /// Unix seconds, 0 = unknown.
    pub modified: i64,
    pub type_label: Arc<str>,
    pub modified_text: Box<str>,
    pub size_text: Box<str>,
}

impl Entry {
    pub fn is_dir(&self) -> bool {
        matches!(self.kind, Kind::Folder | Kind::Drive)
    }

    pub fn is_hidden(&self) -> bool {
        self.flags & FLAG_HIDDEN != 0
    }

    pub fn is_system(&self) -> bool {
        self.flags & FLAG_SYSTEM != 0
    }

    /// Full path of the item.
    pub fn path(&self) -> PathBuf {
        match self.kind {
            Kind::Drive => PathBuf::from(&*self.parent),
            _ if self.parent.is_empty() => PathBuf::from(&*self.name),
            _ => Path::new(&*self.parent).join(&*self.name),
        }
    }

    /// Lower-cased extension without the dot (files only).
    pub fn ext(&self) -> Option<String> {
        if self.kind != Kind::File {
            return None;
        }
        let (stem, ext) = self.name.rsplit_once('.')?;
        (!stem.is_empty() && !ext.is_empty()).then(|| ext.to_ascii_lowercase())
    }
}

/// FNV-1a over the lower-cased path `parent\name`, ignoring a trailing backslash on `parent`.
pub fn key_of(parent: &str, name: &str) -> u64 {
    const BASIS: u64 = 0xcbf2_9ce4_8422_2325;
    const PRIME: u64 = 0x0100_0000_01b3;
    let mut h = BASIS;
    let mut feed = |c: char| {
        for l in c.to_lowercase() {
            h ^= u64::from(l as u32);
            h = h.wrapping_mul(PRIME);
        }
    };
    for c in parent.trim_end_matches(['\\', '/']).chars() {
        feed(if c == '/' { '\\' } else { c });
    }
    feed('\\');
    for c in name.chars() {
        feed(c);
    }
    h
}

/// Key of an existing path (same hash as `Entry::key` for the same item).
pub fn key_of_path(path: &Path) -> u64 {
    match (path.parent(), path.file_name()) {
        (Some(parent), Some(name)) => key_of(&parent.to_string_lossy(), &name.to_string_lossy()),
        _ => key_of(&path.to_string_lossy(), ""),
    }
}

// --- Queries --------------------------------------------------------------------------------

/// What the user is looking at. `location` is the scope of every search:
/// `None` = global search, `Some(dir)` = that folder (browse when `text` is empty, otherwise a
/// search below it), `Some("")` = the virtual root "This PC".
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Query {
    pub location: Option<PathBuf>,
    pub text: String,
    pub regex: bool,
    pub show_hidden: bool,
    pub sort: Sort,
}

impl Query {
    /// Folders are listed before files whenever a folder is open (the list assumes it).
    pub fn folders_first(&self) -> bool {
        self.location.is_some()
    }

    pub fn is_browse(&self) -> bool {
        self.location.is_some() && self.text.trim().is_empty()
    }
}

// --- Sidebar data ---------------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq)]
pub struct DriveInfo {
    /// "Windows 10 (C:)"
    pub label: String,
    /// "C:\"
    pub root: PathBuf,
    pub total: u64,
    pub free: u64,
    /// 0.0..=1.0
    pub used: f32,
    pub system: bool,
    /// "NTFS", "exFAT", ... (empty when unknown).
    pub fs: String,
}

impl DriveInfo {
    pub fn nearly_full(&self) -> bool {
        self.used >= 0.9
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Place {
    pub label: String,
    pub path: PathBuf,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Bookmark {
    pub path: PathBuf,
    pub label: String,
}

// --- Formatting -----------------------------------------------------------------------------

pub fn format_size(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
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

/// 12345 -> "12,345"
pub fn group_digits(n: u64) -> String {
    let s = n.to_string();
    let mut out = String::with_capacity(s.len() + s.len() / 3);
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(c);
    }
    out
}

// --- Paths ----------------------------------------------------------------------------------
//
// Location model: a `PathBuf`. The empty path is the virtual root "This PC" (the drive list).

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

/// `C:\` (or `C:`): a path with nothing but a drive prefix.
pub fn is_drive_root(path: &Path) -> bool {
    let s = path.to_string_lossy();
    let t = s.trim_end_matches(['\\', '/']);
    t.len() == 2 && t.ends_with(':')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_path_is_this_pc() {
        assert_eq!(
            breadcrumbs(Path::new("")),
            vec![(ROOT_LABEL.to_owned(), PathBuf::new())]
        );
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

    #[test]
    fn sort_toggle() {
        let mut s = Sort::DEFAULT;
        s.toggle(SortKey::Name);
        assert_eq!(s.dir, SortDir::Desc);
        s.toggle(SortKey::Size);
        assert_eq!((s.key, s.dir), (SortKey::Size, SortDir::Asc));
    }

    #[test]
    fn keys_ignore_case_and_trailing_backslash() {
        assert_eq!(key_of(r"C:\Users", "Bob"), key_of(r"c:\users\", "BOB"));
        assert_ne!(key_of(r"C:\Users", "Bob"), key_of(r"C:\Users", "Rob"));
    }

    #[cfg(windows)]
    #[test]
    fn key_of_path_matches_key_of() {
        assert_eq!(
            key_of_path(Path::new(r"C:\Users\Bob")),
            key_of(r"C:\Users", "Bob")
        );
    }

    #[test]
    fn sizes_and_digits() {
        assert_eq!(format_size(0), "0 B");
        assert_eq!(format_size(1536), "1.5 KB");
        assert_eq!(format_size(1024 * 1024), "1 MB");
        assert_eq!(group_digits(0), "0");
        assert_eq!(group_digits(999), "999");
        assert_eq!(group_digits(1000), "1,000");
        assert_eq!(group_digits(1234567), "1,234,567");
    }

    #[test]
    fn query_modes() {
        let mut q = Query {
            location: None,
            text: String::new(),
            regex: false,
            show_hidden: false,
            sort: Sort::DEFAULT,
        };
        assert!(!q.folders_first() && !q.is_browse());
        q.location = Some(PathBuf::from("a"));
        assert!(q.folders_first() && q.is_browse());
        q.text = "x".into();
        assert!(!q.is_browse());
    }

    #[test]
    fn drive_roots() {
        assert!(is_drive_root(Path::new(r"C:\")));
        assert!(is_drive_root(Path::new("C:")));
        assert!(!is_drive_root(Path::new(r"C:\x")));
    }
}
