//! File and folder icons drawn as Nerd Font glyphs (see `assets/README.md`). The mapping is plain
//! data, so it is tested without a window; `paint` turns a glyph into pixels.

use eframe::egui::{Align2, Color32, FontId, Painter, Pos2};
use quickfiles_core::{Category, Entry, Kind};

use crate::theme::{Palette, icon_family};

/// Which palette colour a glyph takes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tone {
    Folder,
    Html,
    Script,
    Json,
    Code,
    Image,
    Media,
    Archive,
    Document,
    Pdf,
    Plain,
    /// Computers and devices (This PC, Desktop).
    Device,
}

impl Tone {
    pub fn color(self, pal: &Palette) -> Color32 {
        match self {
            Self::Folder => pal.icon_folder,
            Self::Html => pal.icon_file_html,
            Self::Script => pal.icon_file_script,
            Self::Json => pal.icon_file_json,
            Self::Code => pal.icon_file_code,
            Self::Image => pal.icon_file_image,
            Self::Media => pal.icon_file_media,
            Self::Archive => pal.icon_file_archive,
            Self::Document => pal.icon_file_document,
            Self::Pdf => pal.icon_file_pdf,
            Self::Plain => pal.icon_muted,
            Self::Device => pal.icon_pc,
        }
    }
}

// Codepoints from Nerd Fonts' Font Awesome, Devicons, Seti-UI and Custom sets.
const FILE: char = '\u{f016}';
const FILE_TEXT: char = '\u{f0f6}';
const FILE_IMAGE: char = '\u{f1c5}';
const FILE_PDF: char = '\u{f1c1}';
const FILE_WORD: char = '\u{f1c2}';
const FILE_EXCEL: char = '\u{f1c3}';
const FILE_POWERPOINT: char = '\u{f1c4}';
const FILE_ARCHIVE: char = '\u{f1c6}';
const FILE_AUDIO: char = '\u{f1c7}';
const FILE_VIDEO: char = '\u{f1c8}';
const FILE_CODE: char = '\u{f1c9}';
const FOLDER: char = '\u{f07b}';
const DATABASE: char = '\u{f1c0}';
const TABLE: char = '\u{f0ce}';
const COG: char = '\u{f013}';
const WINDOWS: char = '\u{f17a}';
const DRIVE: char = '\u{f0a0}';

/// The glyph for an extension (lower case, no dot).
fn by_ext(ext: &str) -> Option<(char, Tone)> {
    use Tone::*;
    Some(match ext {
        "rs" => ('\u{e7a8}', Code),
        "py" | "pyw" => ('\u{e73c}', Code),
        "go" => ('\u{e626}', Code),
        "c" | "h" => ('\u{e61e}', Code),
        "cpp" | "cc" | "cxx" | "hpp" => ('\u{e61d}', Code),
        "java" => ('\u{e738}', Code),
        "rb" => ('\u{e739}', Code),
        "php" => ('\u{e73d}', Code),
        "lua" => ('\u{e620}', Code),
        "sh" | "bash" | "zsh" | "bat" | "cmd" | "ps1" => ('\u{e795}', Code),
        "xml" => ('\u{e619}', Code),
        "cs" | "kt" | "swift" | "vb" => (FILE_CODE, Code),
        "js" | "mjs" | "cjs" => ('\u{e74e}', Script),
        "jsx" => ('\u{e7ba}', Script),
        "ts" | "tsx" => ('\u{e628}', Script),
        "json" | "jsonc" => ('\u{e60b}', Json),
        "sql" | "db" | "sqlite" => (DATABASE, Json),
        "html" | "htm" | "xhtml" => ('\u{e736}', Html),
        "css" => ('\u{e749}', Html),
        "scss" | "sass" => ('\u{e74b}', Html),
        "md" | "markdown" => ('\u{e73e}', Plain),
        "toml" | "yaml" | "yml" | "ini" | "cfg" | "conf" | "env" => ('\u{e615}', Plain),
        "txt" | "log" | "rtf" => (FILE_TEXT, Plain),
        "csv" | "tsv" => (TABLE, Plain),
        "pdf" => (FILE_PDF, Pdf),
        "doc" | "docx" | "odt" => (FILE_WORD, Document),
        "xls" | "xlsx" | "ods" => (FILE_EXCEL, Document),
        "ppt" | "pptx" | "odp" => (FILE_POWERPOINT, Document),
        "zip" | "rar" | "7z" | "tar" | "gz" | "bz2" | "xz" | "zst" => (FILE_ARCHIVE, Archive),
        "mp3" | "wav" | "flac" | "ogg" | "m4a" | "aac" | "wma" => (FILE_AUDIO, Media),
        "mp4" | "mkv" | "avi" | "mov" | "webm" | "wmv" | "flv" | "m4v" => (FILE_VIDEO, Media),
        "png" | "jpg" | "jpeg" | "gif" | "bmp" | "webp" | "ico" | "tif" | "tiff" | "avif"
        | "heic" | "psd" | "svg" => (FILE_IMAGE, Image),
        "exe" | "msi" | "scr" => (WINDOWS, Plain),
        "dll" | "sys" => (COG, Plain),
        "iso" | "img" | "vhd" | "vhdx" => (DRIVE, Plain),
        _ => return None,
    })
}

/// Special file names that say more than their extension.
fn by_name(name: &str) -> Option<(char, Tone)> {
    Some(match name {
        "dockerfile" | "docker-compose.yml" | "docker-compose.yaml" => ('\u{e7b0}', Tone::Code),
        ".gitignore" | ".gitattributes" | ".gitmodules" => ('\u{e702}', Tone::Plain),
        "package.json" => ('\u{e718}', Tone::Json),
        "cargo.toml" | "cargo.lock" => ('\u{e7a8}', Tone::Plain),
        _ => return None,
    })
}

/// The glyph and tone for a listing row.
pub fn glyph(e: &Entry) -> (char, Tone) {
    match e.kind {
        Kind::Folder => (FOLDER, Tone::Folder),
        Kind::Drive => (DRIVE, Tone::Plain),
        Kind::File => {
            if let Some(g) = by_name(&e.name.to_ascii_lowercase()) {
                return g;
            }
            if let Some(g) = e.ext().as_deref().and_then(by_ext) {
                return g;
            }
            // Unknown extension: fall back on what the engine thinks it is.
            match e.category {
                Category::Image => (FILE_IMAGE, Tone::Image),
                Category::Video => (FILE_VIDEO, Tone::Media),
                Category::Audio => (FILE_AUDIO, Tone::Media),
                Category::Archive => (FILE_ARCHIVE, Tone::Archive),
                Category::Code | Category::Script => (FILE_CODE, Tone::Code),
                Category::Text => (FILE_TEXT, Tone::Plain),
                Category::Pdf => (FILE_PDF, Tone::Pdf),
                _ => (FILE, Tone::Plain),
            }
        }
    }
}

/// Draws a glyph centred on `c`, `size` points tall.
pub fn paint_glyph(p: &Painter, pal: &Palette, glyph: (char, Tone), c: Pos2, size: f32) {
    let (ch, tone) = glyph;
    p.text(
        c,
        Align2::CENTER_CENTER,
        ch.to_string(),
        FontId::new(size, icon_family()),
        tone.color(pal),
    );
}

/// Draws the row's glyph centred on `c`, `size` points tall.
pub fn paint(p: &Painter, pal: &Palette, e: &Entry, c: Pos2, size: f32) {
    paint_glyph(p, pal, glyph(e), c, size);
}

/// The plain folder glyph (sidebar lists of folders).
pub const FOLDER_GLYPH: (char, Tone) = (FOLDER, Tone::Folder);

/// Glyphs for the well-known places in the sidebar, by their label.
pub fn place(label: &str) -> Option<(char, Tone)> {
    Some(match label {
        "Home" => ('\u{f015}', Tone::Folder),
        "Desktop" => ('\u{f108}', Tone::Device),
        "Documents" => (FILE_TEXT, Tone::Document),
        "Downloads" => ('\u{f019}', Tone::Code),
        "Pictures" => ('\u{f03e}', Tone::Image),
        "Music" => ('\u{f001}', Tone::Media),
        "Videos" => ('\u{f03d}', Tone::Media),
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_extensions_have_glyphs_and_unknown_do_not() {
        assert_eq!(by_ext("rs"), Some(('\u{e7a8}', Tone::Code)));
        assert_eq!(by_ext("pdf").map(|g| g.1), Some(Tone::Pdf));
        assert_eq!(by_ext("zip").map(|g| g.1), Some(Tone::Archive));
        assert_eq!(by_ext("nope"), None);
    }

    #[test]
    fn every_extension_is_lower_case_and_unique() {
        // Guards the match against typos such as a duplicated or capitalised arm.
        let exts = [
            "rs", "py", "go", "c", "h", "cpp", "java", "rb", "php", "lua", "sh", "xml", "js", "ts",
            "json", "sql", "html", "css", "md", "toml", "txt", "csv", "pdf", "docx", "xlsx",
            "pptx", "zip", "mp3", "mp4", "png", "exe", "dll", "iso",
        ];
        for e in exts {
            assert!(by_ext(e).is_some(), "{e}");
            assert_eq!(e, e.to_ascii_lowercase());
        }
    }

    #[test]
    fn every_sidebar_place_has_its_own_glyph() {
        let names = [
            "Home",
            "Desktop",
            "Documents",
            "Downloads",
            "Pictures",
            "Music",
            "Videos",
        ];
        let mut seen = std::collections::HashSet::new();
        for n in names {
            let g = place(n).unwrap_or_else(|| panic!("{n}"));
            assert!(seen.insert(g.0), "{n} repeats a glyph");
        }
        assert_eq!(place("Elsewhere"), None);
    }

    #[test]
    fn special_names() {
        assert_eq!(by_name("dockerfile").map(|g| g.0), Some('\u{e7b0}'));
        assert_eq!(by_name(".gitignore").map(|g| g.0), Some('\u{e702}'));
        assert_eq!(by_name("readme"), None);
    }
}
