//! `config.conf`: the one user-editable settings file (in the data folder, next to the database).
//!
//! ```text
//! # Full-line comments only: `#` also starts a colour.
//! [general]
//! global_hotkey = Ctrl+Alt+Space      # brings the window to the front from anywhere
//!
//! [theme]
//! accent = #0079a6
//! font_size = 13
//!
//! active_theme = "tokyo night"      # which [[themes]] block below to start from
//!
//! [[themes]]
//! name = "tokyo night"
//! background = #1a1b26
//!
//! [hotkeys]
//! Ctrl+B = ToggleSidebar
//! Space t n = NewTab
//! ```
//!
//! This module only splits the file into sections; each owner (theme, hotkeys, global hotkey)
//! interprets its own section. Nothing here depends on the UI.

/// Name of the file in the data folder.
pub const FILE: &str = "config.conf";

#[derive(Debug, Clone, PartialEq)]
pub struct Entry {
    /// 1-based line in the file, for error messages.
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

#[derive(Debug, Default)]
pub struct Sections {
    pub general: Vec<Entry>,
    pub theme: Vec<Entry>,
    pub themes: Vec<NamedTheme>,
    /// The `[hotkeys]` lines verbatim, every other line blanked so line numbers stay true.
    pub hotkeys: String,
    pub errors: Vec<String>,
}

#[derive(Clone, Copy, PartialEq)]
enum Sec {
    None,
    General,
    Theme,
    Themes,
    Hotkeys,
    Unknown,
}

pub fn parse(text: &str) -> Sections {
    let mut out = Sections::default();
    let mut sec = Sec::None;
    for (i, raw) in text.lines().enumerate() {
        let n = i + 1;
        let line = raw.trim();
        let blank_line = |out: &mut Sections| out.hotkeys.push('\n');
        if line.is_empty() || line.starts_with('#') || line.starts_with(';') {
            blank_line(&mut out);
            continue;
        }
        if let Some(name) = line.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
            sec = match name.trim().to_ascii_lowercase().as_str() {
                "general" => Sec::General,
                "theme" => Sec::Theme,
                "[themes]" => {
                    out.themes.push(NamedTheme {
                        name: String::new(),
                        entries: Vec::new(),
                    });
                    Sec::Themes
                }
                "hotkeys" => Sec::Hotkeys,
                other => {
                    out.errors
                        .push(format!("config line {n}: unknown section [{other}]"));
                    Sec::Unknown
                }
            };
            blank_line(&mut out);
            continue;
        }
        match sec {
            Sec::Hotkeys => {
                out.hotkeys.push_str(line);
                out.hotkeys.push('\n');
            }
            Sec::General | Sec::Theme | Sec::Themes => {
                blank_line(&mut out);
                match line.split_once('=') {
                    Some((k, v)) => {
                        let e = Entry {
                            line: n,
                            key: k.trim().to_ascii_lowercase(),
                            value: unquote(v),
                        };
                        match sec {
                            Sec::General => out.general.push(e),
                            Sec::Theme => out.theme.push(e),
                            _ => {
                                if let Some(t) = out.themes.last_mut() {
                                    if e.key == "name" {
                                        t.name = e.value;
                                    } else {
                                        t.entries.push(e);
                                    }
                                }
                            }
                        }
                    }
                    None => out
                        .errors
                        .push(format!("config line {n}: expected key = value")),
                }
            }
            Sec::None => {
                blank_line(&mut out);
                out.errors
                    .push(format!("config line {n}: put this under a [section]"));
            }
            Sec::Unknown => blank_line(&mut out),
        }
    }
    out
}

/// `"tokyo night"` -> `tokyo night`; unquoted text is only trimmed.
pub fn unquote(value: &str) -> String {
    let v = value.trim();
    v.strip_prefix('"')
        .and_then(|r| r.strip_suffix('"'))
        .unwrap_or(v)
        .to_owned()
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

/// Text of a fresh `config.conf`. `legacy_hotkeys` are active lines copied from an old
/// `hotkeys.conf`, so a first run after the upgrade keeps the user's bindings.
pub fn template(
    global_default: &str,
    theme: &[(&str, String)],
    hotkey_defaults: &[(&str, &str)],
    actions: &[&str],
    legacy_hotkeys: &[String],
) -> String {
    let mut out = String::from(
        "# quickfiles settings. Edit, save, then press Ctrl+Shift+R in the app to reload.\n\
         # Comments must be on their own line (a `#` inside a value is a colour).\n\n\
         [general]\n\
         # Works from any program: shows the window, even if minimized or hidden. `none` turns it off.\n\
         # The app has to be running. Needs at least one of Ctrl, Alt, Shift, Win.\n",
    );
    out.push_str(&format!("# global_hotkey = {global_default}\n\n"));
    out.push_str(
        "[theme]\n\
         # Start from one of the [[themes]] blocks below (by name). Lines in this section\n\
         # are applied on top of it, so they work as personal tweaks.\n\
         # active_theme = \"tokyo night\"\n\
         # Colours as #rrggbb (or #rrggbbaa). Uncomment a line to change it.\n\
         # font_size = 13\n",
    );
    for (name, value) in theme {
        out.push_str(&format!("# {name} = {value}\n"));
    }
    out.push_str(
        "\n# Saved themes: copy a block, rename it, change colours. Any colour left out keeps the\n\
         # default above. Pick one with active_theme in [theme]. Remove the leading `# `.\n\
         #\n\
         # [[themes]]\n\
         # name = \"tokyo night\"\n\
         # surface = #16161e\n\
         # background = #1a1b26\n\
         # panel = #16161e\n\
         # row_alt = #1c1d2b\n\
         # hover = #292e42\n\
         # border = #292e42\n\
         # accent = #7aa2f7\n\
         # danger = #f7768e\n\
         # control = #292e42\n\
         # control_border = #3b4261\n\
         # storage_fill = #7aa2f7\n\
         # selection_band = #7aa2f7\n\
         # list_selected = #2e3c64\n\
         # list_hover = #292e42\n\
         # input_background = #1a1b26\n\
         # input_border = #292e42\n\
         # input_border_focus = #7aa2f7\n\
         # input_text = #c0caf5\n\
         # storage_fill_full = #f7768e\n\
         # text = #c0caf5\n\
         # text_muted = #565f89\n\
         # text_secondary = #a9b1d6\n\
         # scrollbar_track = #16161e\n\
         # scrollbar_thumb = #3b4261\n\
         # scrollbar_thumb_hover = #565f89\n\
         # scrollbar_thumb_active = #7aa2f7\n\
         # icon_folder = #e0af68\n\
         #\n\
         # [[themes]]\n\
         # name = \"catppuccin\"\n\
         # surface = #181825\n\
         # background = #1e1e2e\n\
         # panel = #181825\n\
         # row_alt = #202032\n\
         # hover = #313244\n\
         # border = #313244\n\
         # accent = #89b4fa\n\
         # danger = #f38ba8\n\
         # control = #313244\n\
         # control_border = #45475a\n\
         # storage_fill = #89b4fa\n\
         # selection_band = #89b4fa\n\
         # list_selected = #3b4a6b\n\
         # list_hover = #313244\n\
         # input_background = #1e1e2e\n\
         # input_border = #313244\n\
         # input_border_focus = #89b4fa\n\
         # input_text = #cdd6f4\n\
         # storage_fill_full = #f38ba8\n\
         # text = #cdd6f4\n\
         # text_muted = #6c7086\n\
         # text_secondary = #a6adc8\n\
         # scrollbar_track = #181825\n\
         # scrollbar_thumb = #45475a\n\
         # scrollbar_thumb_hover = #6c7086\n\
         # scrollbar_thumb_active = #89b4fa\n\
         # icon_folder = #f9e2af\n\
         \n[hotkeys]\n\
         # Shortcut = Action, one per line. A line replaces the built-in binding of the same\n\
         # shortcut; `Shortcut = none` removes it. Several keys in a row make a sequence\n\
         # (`Space t n = NewTab`): press them one after the other, a panel shows what comes next.\n\
         # Shortcuts: Ctrl, Shift, Alt + a key (A-Z, 0-9, F1-F12, Enter, Delete, Tab, Left, Up, ...).\n\
         # Ctrl+C / Ctrl+X / Ctrl+V always copy / cut / paste. `?` shows every binding.\n\
         # Actions: ",
    );
    out.push_str(&actions.join(", "));
    out.push_str("\n#\n# Defaults:\n");
    for (shortcut, action) in hotkey_defaults {
        out.push_str(&format!("# {shortcut} = {action}\n"));
    }
    if !legacy_hotkeys.is_empty() {
        out.push_str("\n# Imported from your old hotkeys.conf:\n");
        for l in legacy_hotkeys {
            out.push_str(l);
            out.push('\n');
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "# top comment\n\
        [general]\n\
        Global_Hotkey = Ctrl+Alt+Space\n\
        [theme]\n\
        accent = #0079a6\n\
        \n\
        [hotkeys]\n\
        Ctrl+B = ToggleSidebar\n\
        Space t n = NewTab\n\
        [bogus]\n\
        x = y\n";

    #[test]
    fn splits_sections_and_keeps_line_numbers() {
        let s = parse(SAMPLE);
        assert_eq!(s.general.len(), 1);
        assert_eq!(
            (s.general[0].key.as_str(), s.general[0].line),
            ("global_hotkey", 3)
        );
        assert_eq!(s.theme[0].value, "#0079a6");
        let lines: Vec<&str> = s.hotkeys.lines().collect();
        assert_eq!(
            lines[7], "Ctrl+B = ToggleSidebar",
            "same line number as in the file"
        );
        assert_eq!(lines[8], "Space t n = NewTab");
        assert!(lines[2].is_empty());
        assert_eq!(s.errors.len(), 1, "{:?}", s.errors);
        assert!(s.errors[0].contains("[bogus]"));
    }

    #[test]
    fn reports_stray_lines() {
        let s = parse("oops\n[theme]\nno equals here\n");
        assert_eq!(s.errors.len(), 2);
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
            &["Ctrl+B = NewTab".to_owned()],
        );
        let s = parse(&t);
        assert!(s.errors.is_empty(), "{:?}", s.errors);
        assert!(
            s.general.is_empty() && s.theme.is_empty(),
            "everything is commented out"
        );
        assert!(s.hotkeys.contains("Ctrl+B = NewTab"));
    }

    #[test]
    fn named_themes_and_quotes() {
        let s = parse(
            "[theme]\nactive_theme = \"tokyo night\"\n[[themes]]\nname = \"tokyo night\"\nbackground = #1a1b26\n\
             [[themes]]\nname = catppuccin\naccent = #89b4fa\nhover = #313244\n[hotkeys]\nCtrl+B = NewTab\n",
        );
        assert!(s.errors.is_empty(), "{:?}", s.errors);
        assert_eq!(s.theme[0].value, "tokyo night");
        assert_eq!(s.themes.len(), 2);
        assert_eq!(s.themes[0].name, "tokyo night");
        assert_eq!(s.themes[0].entries.len(), 1);
        assert_eq!(s.themes[1].name, "catppuccin");
        assert_eq!(s.themes[1].entries.len(), 2);
        assert!(s.hotkeys.contains("Ctrl+B = NewTab"));
    }
}
