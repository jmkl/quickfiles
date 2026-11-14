//! `config.toml`: the one user-editable settings file (in the data folder, next to the database).
//! Plain TOML, so any editor highlights and checks it.
//!
//! ```toml
//! [general]
//! global_hotkey = "Ctrl+Alt+Space"   # brings the window to the front from anywhere
//!
//! [theme]
//! active_theme = "tokyo night"       # which [[themes]] block below to start from
//! accent = "#0079a6"
//! font_size = 13
//!
//! [[themes]]
//! name = "tokyo night"
//! background = "#1a1b26"
//!
//! [[opener]]
//! extensions = ["nu", "nuon"]
//! program = 'C:\Tools\editor.exe'    # single quotes: backslashes stay as they are
//! args = ["-o", "$1"]
//!
//! [hotkeys]
//! "Ctrl+B" = "ToggleSidebar"
//! "Space t n" = "NewTab"
//! ```
//!
//! This module parses the file and splits it into sections; each owner (theme, hotkeys, opener,
//! global hotkey) interprets its own section. Nothing here depends on the UI.

use toml::{Table, Value};

/// Name of the file in the data folder.
pub const FILE: &str = "config.toml";

#[derive(Debug, Clone, PartialEq)]
pub struct Entry {
    /// 1-based line in the file (best effort: TOML does not keep positions), for messages.
    pub line: usize,
    pub key: String,
    pub value: String,
}

/// One `[[themes]]` block: a name plus colour lines, picked with `active_theme`.
#[derive(Debug, Clone, PartialEq)]
pub struct NamedTheme {
    pub name: String,
    pub entries: Vec<Entry>,
}

/// One `[[opener]]` block, as written.
#[derive(Debug, Clone, PartialEq)]
pub struct Block {
    /// Line of the block's header, for messages.
    pub line: usize,
    pub table: Table,
}

#[derive(Debug, Default)]
pub struct Sections {
    pub general: Vec<Entry>,
    pub theme: Vec<Entry>,
    pub themes: Vec<NamedTheme>,
    pub openers: Vec<Block>,
    /// The `[hotkeys]` table as `Shortcut = Action` lines, each on the line it has in the file
    /// (the others blank) so messages can name the right line.
    pub hotkeys: String,
    pub errors: Vec<String>,
}

/// Splits the file into sections. A file that is not valid TOML gives no settings and one error
/// (with the line); a bad value inside a valid file only skips that value.
pub fn parse(text: &str) -> Sections {
    let mut out = Sections::default();
    let root: Table = match text.parse() {
        Ok(t) => t,
        Err(e) => {
            let line = e.span().map_or(0, |s| {
                1 + text.as_bytes()[..s.start.min(text.len())]
                    .iter()
                    .filter(|b| **b == b'\n')
                    .count()
            });
            out.errors
                .push(format!("config line {line}: {}", e.message()));
            return out;
        }
    };
    for (name, value) in &root {
        match name.to_ascii_lowercase().as_str() {
            "general" => out.general = section(value, "general", text, &mut out.errors),
            "theme" => out.theme = section(value, "theme", text, &mut out.errors),
            "themes" => {
                for t in tables(value, "themes", &mut out.errors) {
                    let name = t.get("name").and_then(Value::as_str).unwrap_or("");
                    out.themes.push(NamedTheme {
                        name: name.to_owned(),
                        entries: entries_of(t, text, &mut out.errors)
                            .into_iter()
                            .filter(|e| e.key != "name")
                            .collect(),
                    });
                }
            }
            "opener" | "openers" => {
                let headers: Vec<usize> = text
                    .lines()
                    .enumerate()
                    .filter(|(_, l)| l.trim().eq_ignore_ascii_case("[[opener]]"))
                    .map(|(i, _)| i + 1)
                    .collect();
                for (i, t) in tables(value, "opener", &mut out.errors)
                    .into_iter()
                    .enumerate()
                {
                    out.openers.push(Block {
                        line: headers.get(i).copied().unwrap_or(0),
                        table: t.clone(),
                    });
                }
            }
            "hotkeys" => out.hotkeys = hotkey_lines(value, text, &mut out.errors),
            other => out
                .errors
                .push(format!("config: unknown section [{other}]")),
        }
    }
    out
}

/// A `[name]` table of plain values.
fn section(value: &Value, name: &str, text: &str, errors: &mut Vec<String>) -> Vec<Entry> {
    match value.as_table() {
        Some(t) => entries_of(t, text, errors),
        None => {
            errors.push(format!("config: [{name}] must be a table"));
            Vec::new()
        }
    }
}

/// The blocks of a `[[name]]` array.
fn tables<'a>(value: &'a Value, name: &str, errors: &mut Vec<String>) -> Vec<&'a Table> {
    let Some(array) = value.as_array() else {
        errors.push(format!("config: use [[{name}]] (double brackets) for this"));
        return Vec::new();
    };
    array
        .iter()
        .filter_map(|v| {
            let t = v.as_table();
            if t.is_none() {
                errors.push(format!(
                    "config: [[{name}]] has an entry that is not a table"
                ));
            }
            t
        })
        .collect()
}

fn entries_of(table: &Table, text: &str, errors: &mut Vec<String>) -> Vec<Entry> {
    let mut out = Vec::new();
    for (key, value) in table {
        let line = find_line(text, key);
        match scalar_text(value) {
            Some(value) => out.push(Entry {
                line,
                key: key.to_ascii_lowercase(),
                value,
            }),
            None => errors.push(format!(
                "config line {line}: \"{key}\" must be text or a number"
            )),
        }
    }
    out
}

/// Strings, numbers and booleans as the text the owners parse.
fn scalar_text(value: &Value) -> Option<String> {
    match value {
        Value::String(s) => Some(s.clone()),
        Value::Integer(i) => Some(i.to_string()),
        Value::Float(f) => Some(f.to_string()),
        Value::Boolean(b) => Some(b.to_string()),
        _ => None,
    }
}

/// `Shortcut = Action` lines for `Hotkeys::from_config`, each placed on its own line of the file.
fn hotkey_lines(value: &Value, text: &str, errors: &mut Vec<String>) -> String {
    let Some(table) = value.as_table() else {
        errors.push("config: [hotkeys] must be a table".to_owned());
        return String::new();
    };
    let mut lines: Vec<String> = Vec::new();
    for (shortcut, action) in table {
        let n = find_line(text, shortcut);
        let Some(action) = action.as_str() else {
            errors.push(format!(
                "config line {n}: \"{shortcut}\" needs an action name in quotes"
            ));
            continue;
        };
        let row = format!("{shortcut} = {action}");
        if n > lines.len() {
            lines.resize(n, String::new());
            lines[n - 1] = row;
        } else if n > 0 && lines[n - 1].is_empty() {
            lines[n - 1] = row;
        } else {
            lines.push(row);
        }
    }
    lines.join("\n")
}

/// 1-based line of `key = ...` (the key may be quoted), or 0 when it is not found.
fn find_line(text: &str, key: &str) -> usize {
    text.lines()
        .position(|l| {
            let l = l.trim_start();
            let l = l.strip_prefix(['"', '\'']).unwrap_or(l);
            l.strip_prefix(key).is_some_and(|r| {
                r.trim_start_matches(['"', '\'', ' ', '\t'])
                    .starts_with('=')
            })
        })
        .map_or(0, |i| i + 1)
}

/// `#rgb`, `#rrggbb` or `#rrggbbaa` (the `#` is optional) -> RGBA.
pub fn parse_color(text: &str) -> Option<[u8; 4]> {
    let h = text.trim().trim_start_matches('#');
    if !h.is_ascii() {
        return None;
    }
    let byte = |s: &str| u8::from_str_radix(s, 16).ok();
    match h.len() {
        3 => {
            let d = |i: usize| byte(&h[i..=i]).map(|v| v * 17);
            Some([d(0)?, d(1)?, d(2)?, 255])
        }
        6 => Some([byte(&h[0..2])?, byte(&h[2..4])?, byte(&h[4..6])?, 255]),
        8 => Some([
            byte(&h[0..2])?,
            byte(&h[2..4])?,
            byte(&h[4..6])?,
            byte(&h[6..8])?,
        ]),
        _ => None,
    }
}

pub fn color_text(c: [u8; 4]) -> String {
    if c[3] == 255 {
        format!("#{:02x}{:02x}{:02x}", c[0], c[1], c[2])
    } else {
        format!("#{:02x}{:02x}{:02x}{:02x}", c[0], c[1], c[2], c[3])
    }
}

/// Text of a fresh `config.toml`. `legacy_hotkeys` are active `Shortcut = Action` lines copied
/// from an old `hotkeys.conf`, so a first run after the upgrade keeps the user's bindings.
pub fn template(
    global_default: &str,
    theme: &[(&str, String)],
    hotkey_defaults: &[(&str, &str)],
    actions: &[&str],
    legacy_hotkeys: &[String],
) -> String {
    let mut out = String::from(
        "# quickfiles settings (TOML). Edit, save, then press Ctrl+Shift+R in the app to reload.\n\
         # Text goes in \"double quotes\". For Windows paths use 'single quotes' so backslashes stay.\n\n\
         [general]\n\
         # Works from any program: shows the window, even if minimized or hidden. `\"none\"` turns it off.\n\
         # The app has to be running. Needs at least one of Ctrl, Alt, Shift, Win.\n",
    );
    out.push_str(&format!("# global_hotkey = \"{global_default}\"\n\n"));
    out.push_str(
        "[theme]\n\
         # Start from one of the [[themes]] blocks below (by name). Lines in this section\n\
         # are applied on top of it, so they work as personal tweaks.\n\
         # active_theme = \"tokyo night\"\n\
         # Colours as \"#rrggbb\" (or \"#rrggbbaa\"). Uncomment a line to change it.\n\
         # font_size = 13\n",
    );
    for (name, value) in theme {
        out.push_str(&format!("# {name} = \"{value}\"\n"));
    }
    out.push_str(
        "\n# Saved themes: copy a block, rename it, change colours. Any colour left out keeps the\n\
         # default above. Pick one with active_theme in [theme]. Remove the leading `# `.\n\
         #\n\
         # [[themes]]\n\
         # name = \"tokyo night\"\n\
         # surface = \"#16161e\"\n\
         # background = \"#1a1b26\"\n\
         # panel = \"#16161e\"\n\
         # row_alt = \"#1c1d2b\"\n\
         # hover = \"#292e42\"\n\
         # border = \"#292e42\"\n\
         # accent = \"#7aa2f7\"\n\
         # danger = \"#f7768e\"\n\
         # control = \"#292e42\"\n\
         # control_border = \"#3b4261\"\n\
         # storage_fill = \"#7aa2f7\"\n\
         # selection_band = \"#7aa2f7\"\n\
         # list_selected = \"#2e3c64\"\n\
         # list_hover = \"#292e42\"\n\
         # input_background = \"#1a1b26\"\n\
         # input_border = \"#292e42\"\n\
         # input_border_focus = \"#7aa2f7\"\n\
         # input_text = \"#c0caf5\"\n\
         # storage_fill_full = \"#f7768e\"\n\
         # text = \"#c0caf5\"\n\
         # text_muted = \"#565f89\"\n\
         # text_secondary = \"#a9b1d6\"\n\
         # scrollbar_track = \"#16161e\"\n\
         # scrollbar_thumb = \"#3b4261\"\n\
         # scrollbar_thumb_hover = \"#565f89\"\n\
         # scrollbar_thumb_active = \"#7aa2f7\"\n\
         # icon_folder = \"#e0af68\"\n\
         #\n\
         # [[themes]]\n\
         # name = \"catppuccin\"\n\
         # surface = \"#181825\"\n\
         # background = \"#1e1e2e\"\n\
         # panel = \"#181825\"\n\
         # row_alt = \"#202032\"\n\
         # hover = \"#313244\"\n\
         # border = \"#313244\"\n\
         # accent = \"#89b4fa\"\n\
         # danger = \"#f38ba8\"\n\
         # control = \"#313244\"\n\
         # control_border = \"#45475a\"\n\
         # storage_fill = \"#89b4fa\"\n\
         # selection_band = \"#89b4fa\"\n\
         # list_selected = \"#3b4a6b\"\n\
         # list_hover = \"#313244\"\n\
         # input_background = \"#1e1e2e\"\n\
         # input_border = \"#313244\"\n\
         # input_border_focus = \"#89b4fa\"\n\
         # input_text = \"#cdd6f4\"\n\
         # storage_fill_full = \"#f38ba8\"\n\
         # text = \"#cdd6f4\"\n\
         # text_muted = \"#6c7086\"\n\
         # text_secondary = \"#a6adc8\"\n\
         # scrollbar_track = \"#181825\"\n\
         # scrollbar_thumb = \"#45475a\"\n\
         # scrollbar_thumb_hover = \"#6c7086\"\n\
         # scrollbar_thumb_active = \"#89b4fa\"\n\
         # icon_folder = \"#f9e2af\"\n\
         \n# Open files with a program of your choice (instead of the Windows default). One block per\n\
         # program; the first block that lists the extension wins. `$1` is the file's path (it is\n\
         # one argument, spaces are fine); leave `args` out to just pass the path.\n\
         #\n\
         # [[opener]]\n\
         # extensions = [\"nu\", \"nuon\"]\n\
         # program = 'C:\\Tools\\editor\\editor.exe'\n\
         # args = [\"-o\", \"$1\"]\n\
         \n[hotkeys]\n\
         # \"Shortcut\" = \"Action\", one per line. A line replaces the built-in binding of the same\n\
         # shortcut; \"none\" as the action removes it. Several keys in a row make a sequence\n\
         # (\"Space t n\" = \"NewTab\"): press them one after the other, a panel shows what comes next.\n\
         # Shortcuts: Ctrl, Shift, Alt + a key (A-Z, 0-9, F1-F12, Enter, Delete, Tab, Left, Up, ...).\n\
         # Ctrl+C / Ctrl+X / Ctrl+V always copy / cut / paste. `?` shows every binding.\n\
         # Actions: ",
    );
    out.push_str(&actions.join(", "));
    out.push_str("\n#\n# Defaults:\n");
    for (shortcut, action) in hotkey_defaults {
        out.push_str(&format!("# \"{shortcut}\" = \"{action}\"\n"));
    }
    // Old `Shortcut = Action` lines become TOML pairs; a repeated shortcut keeps its last action.
    let mut legacy: Vec<(String, String)> = Vec::new();
    for l in legacy_hotkeys {
        let Some((key, action)) = l.split_once('=') else {
            continue;
        };
        let key = key.trim().to_owned();
        let action = action.split('#').next().unwrap_or("").trim().to_owned();
        if key.is_empty() || action.is_empty() || key.contains('"') || action.contains('"') {
            continue;
        }
        match legacy.iter_mut().find(|(k, _)| *k == key) {
            Some(slot) => slot.1 = action,
            None => legacy.push((key, action)),
        }
    }
    if !legacy.is_empty() {
        out.push_str("\n# Imported from your old hotkeys.conf:\n");
        for (key, action) in legacy {
            out.push_str(&format!("\"{key}\" = \"{action}\"\n"));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "# top comment\n\
        [general]\n\
        Global_Hotkey = \"Ctrl+Alt+Space\"\n\
        [theme]\n\
        accent = \"#0079a6\"\n\
        font_size = 13\n\
        \n\
        [hotkeys]\n\
        \"Ctrl+B\" = \"ToggleSidebar\"\n\
        \"Space t n\" = \"NewTab\"\n\
        [bogus]\n\
        x = 1\n";

    #[test]
    fn splits_sections_and_keeps_line_numbers() {
        let s = parse(SAMPLE);
        assert_eq!(s.general.len(), 1);
        assert_eq!(
            (s.general[0].key.as_str(), s.general[0].line),
            ("global_hotkey", 3)
        );
        let accent = s.theme.iter().find(|e| e.key == "accent").unwrap();
        assert_eq!((accent.value.as_str(), accent.line), ("#0079a6", 5));
        let size = s.theme.iter().find(|e| e.key == "font_size").unwrap();
        assert_eq!(size.value, "13");
        let lines: Vec<&str> = s.hotkeys.lines().collect();
        assert_eq!(
            lines[8], "Ctrl+B = ToggleSidebar",
            "same line number as in the file"
        );
        assert_eq!(lines[9], "Space t n = NewTab");
        assert_eq!(s.errors.len(), 1, "{:?}", s.errors);
        assert!(s.errors[0].contains("[bogus]"));
    }

    #[test]
    fn invalid_toml_is_one_error_with_a_line() {
        let s = parse("[theme]\naccent = #0079a6\n");
        assert_eq!(s.errors.len(), 1, "{:?}", s.errors);
        assert!(s.errors[0].starts_with("config line 2"), "{:?}", s.errors);
        assert!(s.theme.is_empty());
    }

    #[test]
    fn wrong_kinds_are_reported_not_fatal() {
        let s = parse("[theme]\naccent = [1]\nfont_size = 12\n[hotkeys]\n\"Ctrl+B\" = 4\n");
        assert_eq!(s.errors.len(), 2, "{:?}", s.errors);
        assert_eq!(s.theme.len(), 1);
    }

    #[test]
    fn colours() {
        assert_eq!(parse_color("#0079a6"), Some([0, 0x79, 0xa6, 255]));
        assert_eq!(parse_color("fff"), Some([255, 255, 255, 255]));
        assert_eq!(parse_color("#11223344"), Some([0x11, 0x22, 0x33, 0x44]));
        assert_eq!(parse_color("#12"), None);
        assert_eq!(parse_color("#gggggg"), None);
        assert_eq!(parse_color("é12345"), None);
        assert_eq!(color_text([0, 0x79, 0xa6, 255]), "#0079a6");
        assert_eq!(color_text([1, 2, 3, 4]), "#01020304");
    }

    #[test]
    fn a_fresh_file_parses_cleanly_and_keeps_legacy_lines() {
        let t = template(
            "Ctrl+Alt+Space",
            &[("accent", "#0079a6".to_owned())],
            &[("Ctrl+B", "ToggleSidebar")],
            &["ToggleSidebar", "NewTab"],
            &[
                "Ctrl+B = ToggleSidebar".to_owned(),
                "Ctrl+B = NewTab  # mine".to_owned(),
                "Space t n = NewTab".to_owned(),
            ],
        );
        let s = parse(&t);
        assert!(s.errors.is_empty(), "{:?}\n{t}", s.errors);
        assert!(
            s.general.is_empty() && s.theme.is_empty() && s.themes.is_empty(),
            "everything is commented out"
        );
        assert!(s.openers.is_empty());
        assert!(s.hotkeys.contains("Ctrl+B = NewTab"));
        assert!(
            !s.hotkeys.contains("Ctrl+B = ToggleSidebar"),
            "the last one wins"
        );
        assert!(s.hotkeys.contains("Space t n = NewTab"));
    }

    #[test]
    fn named_themes_and_openers() {
        let s = parse(
            "[theme]\nactive_theme = \"tokyo night\"\n\
             [[themes]]\nname = \"tokyo night\"\nbackground = \"#1a1b26\"\n\
             [[themes]]\nname = \"catppuccin\"\naccent = \"#89b4fa\"\nhover = \"#313244\"\n\
             [[opener]]\nextensions = [\"nu\"]\nprogram = 'C:\\ed.exe'\n\
             [hotkeys]\n\"Ctrl+B\" = \"NewTab\"\n",
        );
        assert!(s.errors.is_empty(), "{:?}", s.errors);
        assert_eq!(s.theme[0].value, "tokyo night");
        assert_eq!(s.themes.len(), 2);
        assert_eq!(s.themes[0].name, "tokyo night");
        assert_eq!(s.themes[0].entries.len(), 1);
        assert_eq!(s.themes[1].name, "catppuccin");
        assert_eq!(s.themes[1].entries.len(), 2);
        assert_eq!(s.openers.len(), 1);
        assert_eq!(s.openers[0].line, 6);
        assert!(s.hotkeys.contains("Ctrl+B = NewTab"));
    }
}
