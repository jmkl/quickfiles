//! Display strings and file classification. Std only; run on the engine thread when a page
//! arrives so the UI never formats anything per frame.

use std::{collections::HashMap, sync::Arc};

use crate::model::{Category, Entry, FLAG_HIDDEN, Kind, format_size, key_of};

/// "2026-11-01 13:30". `offset_secs` is the local UTC offset at that moment. 0 = unknown -> "".
pub fn time_text(unix: i64, offset_secs: i32) -> String {
    if unix == 0 {
        return String::new();
    }
    let t = unix + i64::from(offset_secs);
    let days = t.div_euclid(86_400);
    let sod = t.rem_euclid(86_400);
    let (y, m, d) = civil_from_days(days);
    format!(
        "{y:04}-{m:02}-{d:02} {:02}:{:02}",
        sod / 3600,
        (sod % 3600) / 60
    )
}

/// Days since 1970-01-01 -> (year, month, day). Howard Hinnant's algorithm.
pub fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (y + i64::from(m <= 2), m, d)
}

/// Windows FILETIME (100 ns ticks since 1601) -> unix seconds. 0 and `u64::MAX` mean unknown.
pub fn filetime_to_unix(ft: u64) -> i64 {
    if ft == 0 || ft == u64::MAX {
        return 0;
    }
    (ft / 10_000_000) as i64 - 11_644_473_600
}

/// Size column text: folders show nothing when their size is unknown (0).
pub fn size_text(size: u64, is_dir: bool) -> String {
    if is_dir && size == 0 {
        String::new()
    } else {
        format_size(size)
    }
}

/// Category and a human type label for a lower-cased extension.
pub fn classify(ext: &str) -> (Category, Option<&'static str>) {
    use Category::*;
    match ext {
        "html" | "htm" | "xhtml" => (Html, Some("HTML Document")),
        "js" | "mjs" | "cjs" => (Script, Some("JavaScript Source File")),
        "ts" | "tsx" => (Script, Some("TypeScript Source File")),
        "jsx" => (Script, Some("JavaScript JSX File")),
        "json" | "jsonc" => (Json, Some("JSON File")),
        "rs" => (Code, Some("Rust Source File")),
        "toml" => (Code, Some("TOML File")),
        "yaml" | "yml" => (Code, Some("YAML File")),
        "py" => (Code, Some("Python Source File")),
        "c" | "h" => (Code, Some("C Source File")),
        "cpp" | "cc" | "hpp" => (Code, Some("C++ Source File")),
        "cs" => (Code, Some("C# Source File")),
        "java" => (Code, Some("Java Source File")),
        "go" => (Code, Some("Go Source File")),
        "kt" => (Code, Some("Kotlin Source File")),
        "php" => (Code, Some("PHP Source File")),
        "rb" => (Code, Some("Ruby Source File")),
        "lua" => (Code, Some("Lua Source File")),
        "sh" | "bat" | "cmd" | "ps1" => (Code, Some("Script File")),
        "css" | "scss" => (Code, Some("Stylesheet")),
        "sql" => (Code, Some("SQL File")),
        "xml" => (Code, Some("XML File")),
        "png" | "jpg" | "jpeg" | "gif" | "bmp" | "webp" | "ico" | "tif" | "tiff" | "avif"
        | "heic" | "psd" => (Image, None),
        "svg" => (Image, Some("SVG Image")),
        "mp4" | "mkv" | "avi" | "mov" | "webm" | "wmv" | "flv" | "m4v" => (Video, None),
        "mp3" | "wav" | "flac" | "ogg" | "m4a" | "aac" | "wma" => (Audio, None),
        "zip" | "rar" | "7z" | "tar" | "gz" | "bz2" | "xz" | "zst" => (Archive, None),
        "pdf" => (Pdf, Some("PDF Document")),
        "txt" | "log" => (Text, Some("Text Document")),
        "md" | "markdown" => (Text, Some("Markdown File")),
        "ini" | "cfg" | "conf" | "env" => (Text, Some("Configuration File")),
        "csv" => (Text, Some("CSV File")),
        "doc" | "docx" => (Document, Some("Word Document")),
        "xls" | "xlsx" => (Document, Some("Excel Workbook")),
        "ppt" | "pptx" => (Document, Some("PowerPoint Presentation")),
        "exe" => (Other, Some("Application")),
        "dll" => (Other, Some("Application Extension")),
        "msi" => (Other, Some("Windows Installer Package")),
        "lnk" => (Other, Some("Shortcut")),
        _ => (Other, None),
    }
}

/// Whether the extension is worth a shell/decoder thumbnail request.
pub fn wants_thumbnail(cat: Category) -> bool {
    matches!(
        cat,
        Category::Image | Category::Video | Category::Pdf | Category::Document
    )
}

/// Whether the file is plain text we can preview by reading its first bytes.
pub fn is_text_like(cat: Category, ext: &str) -> bool {
    matches!(
        cat,
        Category::Text | Category::Code | Category::Json | Category::Html | Category::Script
    ) || matches!(ext, "svg")
}

/// Builds `Entry` values: interns type labels and shares the parent `Arc` between rows.
pub struct Builder {
    labels: HashMap<String, Arc<str>>,
    folder: Arc<str>,
    drive: Arc<str>,
    file: Arc<str>,
    last_parent: Arc<str>,
}

impl Default for Builder {
    fn default() -> Self {
        Self::new()
    }
}

impl Builder {
    pub fn new() -> Self {
        Self {
            labels: HashMap::new(),
            folder: Arc::from("File folder"),
            drive: Arc::from("Local Disk"),
            file: Arc::from("File"),
            last_parent: Arc::from(""),
        }
    }

    fn parent_arc(&mut self, parent: &str) -> Arc<str> {
        // A bare drive ("E:") would make `Path::join` produce the drive-relative "E:name".
        let fixed;
        let parent = if parent.len() == 2 && parent.ends_with(':') {
            fixed = format!("{parent}\\");
            fixed.as_str()
        } else {
            parent
        };
        if &*self.last_parent != parent {
            self.last_parent = Arc::from(parent);
        }
        self.last_parent.clone()
    }

    fn label_for(&mut self, ext: &str, fixed: Option<&'static str>) -> Arc<str> {
        if let Some(l) = self.labels.get(ext) {
            return l.clone();
        }
        let text = match fixed {
            Some(l) => l.to_owned(),
            None => format!("{} File", ext.to_ascii_uppercase()),
        };
        let arc: Arc<str> = Arc::from(text);
        self.labels.insert(ext.to_owned(), arc.clone());
        arc
    }

    /// `parent` is the containing folder path; `name` the item name.
    /// `tz_offset` is the local UTC offset (seconds) for `modified`.
    #[allow(clippy::too_many_arguments)]
    pub fn entry(
        &mut self,
        parent: &str,
        name: String,
        kind: Kind,
        size: u64,
        modified: i64,
        flags: u8,
        tz_offset: i32,
    ) -> Entry {
        let (category, type_label) = match kind {
            Kind::Folder => (Category::Folder, self.folder.clone()),
            Kind::Drive => (Category::Drive, self.drive.clone()),
            Kind::File => match name.rsplit_once('.') {
                Some((stem, ext)) if !stem.is_empty() && !ext.is_empty() => {
                    let ext = ext.to_ascii_lowercase();
                    let (cat, fixed) = classify(&ext);
                    let label = self.label_for(&ext, fixed);
                    (cat, label)
                }
                _ => (Category::Other, self.file.clone()),
            },
        };
        let is_dir = kind != Kind::File;
        let parent_arc = self.parent_arc(parent);
        Entry {
            key: key_of(&parent_arc, &name),
            parent: parent_arc,
            kind,
            category,
            flags,
            size,
            modified,
            type_label,
            modified_text: time_text(modified, tz_offset).into_boxed_str(),
            size_text: size_text(size, is_dir).into_boxed_str(),
            name: name.into_boxed_str(),
        }
    }

    /// A drive row for the "This PC" listing.
    pub fn drive(&mut self, label: &str, root: &str, system: bool) -> Entry {
        let mut e = self.entry(root, label.to_owned(), Kind::Drive, 0, 0, 0, 0);
        // Drives are identified by their root path, not by label + parent.
        e.key = key_of(root, "");
        if system {
            e.flags |= crate::model::FLAG_SYSTEM;
        }
        e
    }
}

/// Hidden attribute bit as reported by Windows.
pub fn flags_from_attrs(attrs: u32) -> u8 {
    if attrs == u32::MAX {
        return 0;
    }
    let mut f = 0;
    if attrs & 0x2 != 0 {
        f |= FLAG_HIDDEN;
    }
    if attrs & 0x4 != 0 {
        f |= crate::model::FLAG_SYSTEM;
    }
    if attrs & 0x1 != 0 {
        f |= crate::model::FLAG_READONLY;
    }
    f
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn civil_dates() {
        assert_eq!(civil_from_days(0), (1970, 1, 1));
        assert_eq!(civil_from_days(19_723), (2024, 1, 1));
        assert_eq!(civil_from_days(-1), (1969, 12, 31));
        assert_eq!(civil_from_days(11_016), (2000, 2, 29));
    }

    #[test]
    fn time_text_applies_offset() {
        // 2026-11-01 06:30:00 UTC
        let unix = 1_793_514_600;
        assert_eq!(time_text(unix, 0), "2026-11-01 06:30");
        assert_eq!(time_text(unix, 7 * 3600), "2026-11-01 13:30");
        assert_eq!(time_text(0, 3600), "");
    }

    #[test]
    fn filetime() {
        assert_eq!(filetime_to_unix(0), 0);
        assert_eq!(filetime_to_unix(u64::MAX), 0);
        assert_eq!(filetime_to_unix(116_444_736_000_000_000), 0 + 0); // 1970-01-01 -> 0
        assert_eq!(filetime_to_unix(116_444_736_000_000_000 + 10_000_000), 1);
    }

    #[test]
    fn builder_shares_parent_and_labels() {
        let mut b = Builder::new();
        let a = b.entry(r"C:\x", "a.rs".into(), Kind::File, 10, 0, 0, 0);
        let c = b.entry(r"C:\x", "b.rs".into(), Kind::File, 20, 0, 0, 0);
        assert!(Arc::ptr_eq(&a.parent, &c.parent));
        assert!(Arc::ptr_eq(&a.type_label, &c.type_label));
        assert_eq!(&*a.type_label, "Rust Source File");
        assert_eq!(a.category, Category::Code);
        let d = b.entry(r"C:\x", "pic.PNG".into(), Kind::File, 5, 0, 0, 0);
        assert_eq!(&*d.type_label, "PNG File");
        assert_eq!(d.category, Category::Image);
        let f = b.entry(r"C:\x", "dir".into(), Kind::Folder, 0, 0, 0, 0);
        assert_eq!(&*f.size_text, "");
        assert_eq!(&*f.type_label, "File folder");
        let n = b.entry(r"C:\x", "noext".into(), Kind::File, 1, 0, 0, 0);
        assert_eq!(&*n.type_label, "File");
        let dot = b.entry(r"C:\x", ".gitignore".into(), Kind::File, 1, 0, 0, 0);
        assert_eq!(dot.category, Category::Other);
    }

    #[test]
    fn entry_key_matches_path_key() {
        let mut b = Builder::new();
        let e = b.entry(r"C:\x", "a.rs".into(), Kind::File, 1, 0, 0, 0);
        assert_eq!(e.key, key_of(r"C:\x", "a.rs"));
        assert_eq!(e.path(), std::path::PathBuf::from(r"C:\x").join("a.rs"));
    }

    #[test]
    fn attrs() {
        assert_eq!(flags_from_attrs(u32::MAX), 0);
        assert_eq!(flags_from_attrs(0x2) & FLAG_HIDDEN, FLAG_HIDDEN);
    }

    #[test]
    fn bare_drive_parent_gets_a_backslash() {
        let mut b = Builder::new();
        let e = b.entry("E:", "_CODE".to_owned(), Kind::Folder, 0, 0, 0, 0);
        assert_eq!(&*e.parent, "E:\\");
        assert_eq!(
            e.key,
            crate::model::key_of_path(std::path::Path::new("E:\\_CODE"))
        );
    }
}
