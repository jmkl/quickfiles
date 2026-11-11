//! Hotkey system: one table that maps shortcuts to [`Action`]s.
//!
//! Every user operation (buttons included) goes through `Action`, so a hotkey is just another
//! way to produce one. To add or change a shortcut, edit [`Hotkeys::default_map`] only.
//!
//! `in_text` says whether a binding still fires while a text box has focus. Plain keys
//! (Backspace, Esc, Delete, Ctrl+A/C/X/V) must not, or they would steal editing keys.
//! Arrow Up/Down and Enter do fire in the search box: that is how you walk the results
//! without leaving the keyboard.

use eframe::egui::{Context, Event, Id, Key, KeyboardShortcut, Modifiers};

use crate::ui::Action;

struct Binding {
    shortcut: KeyboardShortcut,
    make: fn() -> Action,
    in_text: bool,
}

/// Every action a shortcut can trigger: (name used in `hotkeys.conf`, constructor, fires while
/// typing in a text box).
const ACTIONS: &[(&str, fn() -> Action, bool)] = &[
    ("ToggleSidebar", || Action::ToggleSidebar, true),
    ("TogglePreview", || Action::TogglePreview, true),
    ("FocusSearch", || Action::FocusSearch, true),
    ("ToggleRegex", || Action::ToggleRegex, true),
    ("ToggleHidden", || Action::ToggleHidden, true),
    ("ToggleGrid", || Action::ToggleGrid, true),
    ("TogglePerf", || Action::TogglePerf, true),
    ("CopyPerf", || Action::CopyPerf, true),
    ("ResetPerf", || Action::ResetPerf, true),
    ("Back", || Action::Back, false),
    ("Forward", || Action::Forward, false),
    ("Up", || Action::Up, false),
    ("Refresh", || Action::Refresh, true),
    ("NewFolder", || Action::NewFolder, false),
    ("Delete", || Action::Delete { permanent: false }, false),
    (
        "DeletePermanent",
        || Action::Delete { permanent: true },
        false,
    ),
    ("Rename", || Action::Rename, false),
    ("Reveal", || Action::Reveal, false),
    ("Bookmark", || Action::ToggleBookmark, false),
    ("CopyPath", || Action::CopyPath, false),
    ("SelectAll", || Action::SelectAll, false),
    ("ExtendDown", || Action::ExtendFocus(1), false),
    ("ExtendUp", || Action::ExtendFocus(-1), false),
    ("MoveDown", || Action::MoveFocus(1), true),
    ("MoveUp", || Action::MoveFocus(-1), true),
    ("MoveLeft", || Action::MoveSide(-1), false),
    ("MoveRight", || Action::MoveSide(1), false),
    ("PageDown", || Action::MoveFocus(15), false),
    ("PageUp", || Action::MoveFocus(-15), false),
    ("Open", || Action::OpenFocused, true),
    ("Escape", || Action::Escape, false),
    ("NewTab", || Action::NewTab, true),
    ("CloseTab", || Action::CloseCurrentTab, true),
    ("NextTab", || Action::CycleTab(1), true),
    ("PrevTab", || Action::CycleTab(-1), true),
];

/// The built-in map. Edit `hotkeys.conf` (same `shortcut = Action` lines) to change it.
/// Bindings with more modifiers come first, plain keys last.
pub const DEFAULTS: &[(&str, &str)] = &[
    ("Ctrl+B", "ToggleSidebar"),
    ("Alt+P", "TogglePreview"),
    ("Ctrl+F", "FocusSearch"),
    ("Ctrl+L", "FocusSearch"),
    ("Ctrl+R", "ToggleRegex"),
    ("Ctrl+H", "ToggleHidden"),
    ("Ctrl+G", "ToggleGrid"),
    ("F12", "TogglePerf"),
    ("Ctrl+F12", "CopyPerf"),
    ("Shift+F12", "ResetPerf"),
    ("Ctrl+T", "NewTab"),
    ("Ctrl+W", "CloseTab"),
    ("Ctrl+Shift+Tab", "PrevTab"),
    ("Ctrl+Tab", "NextTab"),
    ("Alt+Left", "Back"),
    ("Alt+Right", "Forward"),
    ("Alt+Up", "Up"),
    ("Backspace", "Up"),
    ("F5", "Refresh"),
    ("Ctrl+Shift+N", "NewFolder"),
    ("Ctrl+Shift+C", "CopyPath"),
    ("Shift+Delete", "DeletePermanent"),
    ("Delete", "Delete"),
    ("F2", "Rename"),
    ("Ctrl+A", "SelectAll"),
    ("Shift+Down", "ExtendDown"),
    ("Shift+Up", "ExtendUp"),
    ("Down", "MoveDown"),
    ("Up", "MoveUp"),
    ("Left", "MoveLeft"),
    ("Right", "MoveRight"),
    ("PageDown", "PageDown"),
    ("PageUp", "PageUp"),
    ("Enter", "Open"),
    ("Escape", "Escape"),
];

/// "Ctrl+Shift+N" -> shortcut. Ctrl and Cmd both mean the platform command key.
pub fn parse_shortcut(text: &str) -> Result<KeyboardShortcut, String> {
    let mut mods = Modifiers::NONE;
    let mut key = None;
    for part in text.split('+').map(str::trim).filter(|p| !p.is_empty()) {
        match part.to_ascii_lowercase().as_str() {
            "ctrl" | "control" | "cmd" | "command" => mods = mods | Modifiers::COMMAND,
            "shift" => mods = mods | Modifiers::SHIFT,
            "alt" | "option" => mods = mods | Modifiers::ALT,
            _ => {
                // Letters are upper-case in egui's key names ("N"), the rest as written.
                let name = if part.chars().count() == 1 {
                    part.to_ascii_uppercase()
                } else {
                    part.to_owned()
                };
                if key
                    .replace(
                        Key::from_name(&name).ok_or_else(|| format!("unknown key \"{part}\""))?,
                    )
                    .is_some()
                {
                    return Err("more than one key".to_owned());
                }
            }
        }
    }
    key.map(|k| KeyboardShortcut::new(mods, k))
        .ok_or_else(|| "no key".to_owned())
}

fn action_by_name(name: &str) -> Option<&'static (&'static str, fn() -> Action, bool)> {
    ACTIONS
        .iter()
        .find(|(n, _, _)| n.eq_ignore_ascii_case(name))
}

/// Text of a fresh `hotkeys.conf`: explains the format and lists the defaults as comments.
pub fn config_template() -> String {
    let mut out = String::from(
        "# quickfiles hotkeys. One binding per line:  Shortcut = Action\n\
         # A line replaces the built-in binding of the same shortcut; `Shortcut = none` removes it.\n\
         # Shortcuts: Ctrl, Shift, Alt + a key (A-Z, 0-9, F1-F12, Enter, Delete, Tab, Left, Up, ...).\n\
         # Ctrl+C / Ctrl+X / Ctrl+V always copy / cut / paste.\n\
         # Actions: ",
    );
    out.push_str(&ACTIONS.iter().map(|a| a.0).collect::<Vec<_>>().join(", "));
    out.push_str("\n#\n# Defaults:\n");
    for (shortcut, action) in DEFAULTS {
        out.push_str(&format!("# {shortcut} = {action}\n"));
    }
    out
}

pub struct Hotkeys {
    bindings: Vec<Binding>,
}

impl Hotkeys {
    pub fn default_map() -> Self {
        Self::from_config("").0
    }

    /// Defaults plus the user's overrides. Bad lines are skipped and reported.
    pub fn from_config(config: &str) -> (Self, Vec<String>) {
        let mut errors = Vec::new();
        let mut table: Vec<(KeyboardShortcut, Option<&'static str>)> = DEFAULTS
            .iter()
            .filter_map(|(s, a)| parse_shortcut(s).ok().map(|s| (s, Some(*a))))
            .collect();

        for (n, line) in config.lines().enumerate() {
            let line = line.split('#').next().unwrap_or("").trim();
            if line.is_empty() {
                continue;
            }
            let Some((left, right)) = line.split_once('=') else {
                errors.push(format!(
                    "hotkeys.conf line {}: expected Shortcut = Action",
                    n + 1
                ));
                continue;
            };
            let shortcut = match parse_shortcut(left) {
                Ok(s) => s,
                Err(e) => {
                    errors.push(format!("hotkeys.conf line {}: {e}", n + 1));
                    continue;
                }
            };
            let target = right.trim();
            let action = if target.eq_ignore_ascii_case("none") {
                None
            } else if let Some(a) = action_by_name(target) {
                Some(a.0)
            } else {
                errors.push(format!(
                    "hotkeys.conf line {}: unknown action \"{target}\"",
                    n + 1
                ));
                continue;
            };
            table.retain(|(s, _)| *s != shortcut);
            table.push((shortcut, action));
        }

        let mut bindings: Vec<Binding> = table
            .into_iter()
            .filter_map(|(shortcut, name)| {
                let (_, make, in_text) = action_by_name(name?)?;
                Some(Binding {
                    shortcut,
                    make: *make,
                    in_text: *in_text,
                })
            })
            .collect();
        // More modifiers first, so Ctrl+Shift+N is tried before Ctrl+N.
        bindings.sort_by_key(|b| {
            let m = b.shortcut.modifiers;
            std::cmp::Reverse(u8::from(m.command) + u8::from(m.shift) + u8::from(m.alt))
        });
        (Self { bindings }, errors)
    }

    /// Consumes the shortcuts pressed this frame and pushes their actions.
    /// `suppress` is true while an overlay owns the keyboard (menu open).
    pub fn poll(&self, ctx: &Context, suppress: bool, out: &mut Vec<Action>) {
        if suppress {
            return;
        }
        // Only the search box and the rename box count as "typing". `wants_keyboard_input()` is
        // also true when a clicked button holds focus, which would disable every plain hotkey.
        let typing = ctx.memory(|m| {
            matches!(m.focused(), Some(f) if f == Id::new("title_search") || f == Id::new("rename_edit"))
        });
        ctx.input_mut(|input| {
            for binding in &self.bindings {
                if typing && !binding.in_text {
                    continue;
                }
                if input.consume_shortcut(&binding.shortcut) {
                    out.push((binding.make)());
                }
            }
            // egui turns Ctrl+C / X / V into dedicated events instead of key presses.
            if !typing {
                for event in &input.events {
                    match event {
                        Event::Copy => out.push(Action::Copy),
                        Event::Cut => out.push(Action::Cut),
                        Event::Paste(_) => out.push(Action::Paste),
                        _ => {}
                    }
                }
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_shortcuts() {
        let s = parse_shortcut("Ctrl+Shift+n").unwrap();
        assert_eq!(s.logical_key, Key::N);
        assert!(s.modifiers.command && s.modifiers.shift && !s.modifiers.alt);
        assert!(parse_shortcut("Ctrl+").is_err());
        assert!(parse_shortcut("Ctrl+Banana").is_err());
    }

    #[test]
    fn every_default_is_valid() {
        for (shortcut, action) in DEFAULTS {
            assert!(parse_shortcut(shortcut).is_ok(), "{shortcut}");
            assert!(action_by_name(action).is_some(), "{action}");
        }
    }

    #[test]
    fn config_overrides_and_unbinds() {
        let (map, errors) =
            Hotkeys::from_config("Ctrl+B = ToggleHidden\nF2 = none\nbogus\nF9 = Nope\n");
        assert_eq!(errors.len(), 2);
        let ctrl_b = parse_shortcut("Ctrl+B").unwrap();
        let b = map.bindings.iter().find(|b| b.shortcut == ctrl_b).unwrap();
        assert!(matches!((b.make)(), Action::ToggleHidden));
        let f2 = parse_shortcut("F2").unwrap();
        assert!(map.bindings.iter().all(|b| b.shortcut != f2));
    }
}
