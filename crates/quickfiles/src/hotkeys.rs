//! Hotkey system: one table that maps shortcuts to [`Action`]s.
//!
//! Every user operation (buttons included) goes through `Action`, so a hotkey is just another
//! way to produce one. To add or change a shortcut, edit [`Hotkeys::default_map`] only.
//!
//! `in_text` says whether a binding still fires while a text box has focus. Plain keys
//! (Backspace, Esc, Delete, Ctrl+A/C/X/V) must not, or they would steal editing keys.
//! Arrow Up/Down and Enter do fire in the search box: that is how you walk the results
//! without leaving the keyboard.
//!
//! A binding can also be a *sequence* of chords separated by spaces (`Space t n`, `g u`), like
//! Vim's leader keys. Sequences never fire while a text box has focus. After the first chord the
//! which-key panel lists what can come next ([`Hotkeys::hints`]); Esc, a wrong key or three idle
//! seconds cancel it. `?` (Shift+/) opens the full list ([`Hotkeys::help_rows`]).

use std::time::{Duration, Instant};

use eframe::egui::{Context, Event, Id, Key, KeyboardShortcut, Modifiers};

use crate::{keyseq, ui::Action};

/// A half-typed sequence is dropped after this long.
const PENDING_TIMEOUT: Duration = Duration::from_secs(3);

struct Binding {
    /// One chord for an ordinary shortcut, several for a sequence.
    seq: Vec<KeyboardShortcut>,
    /// The chords as written (`["Space", "t", "n"]`), for the panels.
    tokens: Vec<String>,
    name: &'static str,
    make: fn() -> Action,
    in_text: bool,
}

/// One line of the which-key panel.
#[derive(Clone, Debug)]
pub struct Hint {
    pub key: String,
    pub label: String,
    /// A prefix that opens a further level (drawn as `+name`).
    pub group: bool,
}

/// What the which-key panel shows while a sequence is half typed.
#[derive(Clone, Debug)]
pub struct Hints {
    /// The chords typed so far, e.g. `Space t`.
    pub prefix: String,
    pub items: Vec<Hint>,
}

/// One line of the `?` help panel: every key bound to one action.
#[derive(Clone, Debug)]
pub struct HelpRow {
    pub keys: String,
    pub label: String,
}

/// Names shown for prefixes in the which-key panel (`+tabs`).
const GROUPS: &[(&str, &str)] = &[
    ("Space", "leader"),
    ("Space t", "tabs"),
    ("Space d", "delete"),
    ("g", "go"),
];

/// "ToggleSidebar" -> "Toggle sidebar".
fn label_of(name: &str) -> String {
    let mut out = String::new();
    for (i, c) in name.chars().enumerate() {
        if i > 0 && c.is_ascii_uppercase() {
            out.push(' ');
            out.push(c.to_ascii_lowercase());
        } else {
            out.push(c);
        }
    }
    out
}

/// Every action a shortcut can trigger: (name used in the `[hotkeys]` section of `config.conf`, constructor, fires while
/// typing in a text box).
const ACTIONS: &[(&str, fn() -> Action, bool)] = &[
    ("ToggleSidebar", || Action::ToggleSidebar, true),
    ("TogglePreview", || Action::TogglePreview, true),
    ("FocusSearch", || Action::FocusSearch, true),
    ("ClearSearch", || Action::ClearSearch, true),
    ("EditPath", || Action::EditPath, true),
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
    // Not while typing: `?` has to reach the search box as a character.
    ("ToggleHelp", || Action::ToggleHelp, false),
    ("ReloadConfig", || Action::ReloadConfig, true),
];

/// The built-in map. Edit `[hotkeys]` in `config.conf` (same `shortcut = Action` lines) to change it.
/// Bindings with more modifiers come first, plain keys last.
pub const DEFAULTS: &[(&str, &str)] = &[
    ("Ctrl+B", "ToggleSidebar"),
    ("Alt+P", "TogglePreview"),
    ("Ctrl+F", "FocusSearch"),
    ("Ctrl+L", "FocusSearch"),
    ("Ctrl+Shift+F", "ClearSearch"),
    ("Alt+D", "EditPath"),
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
    // Which-key help. `?` is Shift+/ on a US keyboard; layouts report it either way.
    ("Ctrl+Shift+R", "ReloadConfig"),
    ("Shift+/", "ToggleHelp"),
    ("?", "ToggleHelp"),
    // Leader sequences (not while typing in a text box). Press Space and wait: the panel lists
    // what comes next.
    ("Space b", "ToggleSidebar"),
    ("Space p", "TogglePreview"),
    ("Space f", "FocusSearch"),
    ("Space c", "ClearSearch"),
    ("Space l", "EditPath"),
    ("Space r", "ToggleRegex"),
    ("Space h", "ToggleHidden"),
    ("Space g", "ToggleGrid"),
    ("Space n", "NewFolder"),
    ("Space e", "Rename"),
    ("Space o", "Reveal"),
    ("Space m", "Bookmark"),
    ("Space y", "CopyPath"),
    ("Space t n", "NewTab"),
    ("Space t c", "CloseTab"),
    ("Space t l", "NextTab"),
    ("Space t h", "PrevTab"),
    ("Space d d", "Delete"),
    ("Space d p", "DeletePermanent"),
    ("g u", "Up"),
    ("g b", "Back"),
    ("g f", "Forward"),
];

/// "Space t n" -> chords. A single chord is an ordinary shortcut.
pub fn parse_sequence(text: &str) -> Result<(Vec<KeyboardShortcut>, Vec<String>), String> {
    let tokens: Vec<String> = text.split_whitespace().map(str::to_owned).collect();
    if tokens.is_empty() {
        return Err("no key".to_owned());
    }
    let seq = tokens
        .iter()
        .map(|t| parse_shortcut(t))
        .collect::<Result<Vec<_>, _>>()?;
    Ok((seq, tokens))
}

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

/// Names of every action, for the comment in a fresh `config.conf`.
pub fn action_names() -> Vec<&'static str> {
    ACTIONS.iter().map(|a| a.0).collect()
}

pub struct Hotkeys {
    bindings: Vec<Binding>,
    /// Chords typed so far of a sequence; empty when none is in progress.
    pending: Vec<KeyboardShortcut>,
    since: Instant,
}

/// Number of modifier keys in a chord: more modifiers are tried first.
fn weight(s: &KeyboardShortcut) -> std::cmp::Reverse<u8> {
    let m = s.modifiers;
    std::cmp::Reverse(u8::from(m.command) + u8::from(m.shift) + u8::from(m.alt))
}

impl Hotkeys {
    pub fn default_map() -> Self {
        Self::from_config("").0
    }

    /// Defaults plus the user's overrides. Bad lines are skipped and reported.
    pub fn from_config(config: &str) -> (Self, Vec<String>) {
        type Row = (Vec<KeyboardShortcut>, Vec<String>, Option<&'static str>);
        let mut errors = Vec::new();
        let mut table: Vec<Row> = DEFAULTS
            .iter()
            .filter_map(|(s, a)| {
                parse_sequence(s)
                    .ok()
                    .map(|(seq, tokens)| (seq, tokens, Some(*a)))
            })
            .collect();

        for (n, line) in config.lines().enumerate() {
            let line = line.split('#').next().unwrap_or("").trim();
            if line.is_empty() {
                continue;
            }
            let Some((left, right)) = line.split_once('=') else {
                errors.push(format!("config line {}: expected Shortcut = Action", n + 1));
                continue;
            };
            let (seq, tokens) = match parse_sequence(left) {
                Ok(s) => s,
                Err(e) => {
                    errors.push(format!("config line {}: {e}", n + 1));
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
                    "config line {}: unknown action \"{target}\"",
                    n + 1
                ));
                continue;
            };
            table.retain(|(s, _, _)| *s != seq);
            table.push((seq, tokens, action));
        }

        // `g` and `g g` cannot both work (the short one fires first): keep the short one.
        let live: Vec<usize> = (0..table.len()).filter(|&i| table[i].2.is_some()).collect();
        let seqs: Vec<Vec<KeyboardShortcut>> = live.iter().map(|&i| table[i].0.clone()).collect();
        let mut dropped = std::collections::HashSet::new();
        for (short, long) in keyseq::conflicts(&seqs) {
            let (short, long) = (live[short], live[long]);
            if dropped.insert(long) {
                errors.push(format!(
                    "hotkeys: \"{}\" is a prefix of \"{}\"; the longer one is ignored",
                    table[short].1.join(" "),
                    table[long].1.join(" ")
                ));
            }
        }

        let mut bindings: Vec<Binding> = table
            .into_iter()
            .enumerate()
            .filter(|(i, _)| !dropped.contains(i))
            .filter_map(|(_, (seq, tokens, name))| {
                let (name, make, in_text) = action_by_name(name?)?;
                Some(Binding {
                    seq,
                    tokens,
                    name,
                    make: *make,
                    in_text: *in_text,
                })
            })
            .collect();
        // More modifiers first, so Ctrl+Shift+N is tried before Ctrl+N.
        bindings.sort_by_key(|b| weight(&b.seq[0]));
        (
            Self {
                bindings,
                pending: Vec::new(),
                since: Instant::now(),
            },
            errors,
        )
    }

    fn seqs(&self) -> Vec<Vec<KeyboardShortcut>> {
        self.bindings.iter().map(|b| b.seq.clone()).collect()
    }

    /// Consumes the shortcuts pressed this frame and pushes their actions.
    /// `suppress` is true while an overlay owns the keyboard (menu open).
    pub fn poll(&mut self, ctx: &Context, suppress: bool, out: &mut Vec<Action>) {
        if suppress {
            self.pending.clear();
            return;
        }
        // Only the search box and the rename box count as "typing". `wants_keyboard_input()` is
        // also true when a clicked button holds focus, which would disable every plain hotkey.
        let typing = ctx.memory(|m| {
            matches!(m.focused(), Some(f) if f == Id::new("title_search") || f == Id::new("rename_edit") || f == Id::new("path_edit"))
        });
        // The address bar owns every key (Up / Down / Tab move in its suggestion list), unlike the
        // search box, where Up / Down still move the file selection.
        if ctx.memory(|m| matches!(m.focused(), Some(f) if f == Id::new("path_edit"))) {
            self.pending.clear();
            return;
        }
        if typing || self.since.elapsed() > PENDING_TIMEOUT {
            self.pending.clear();
        }
        let seqs = self.seqs();
        let mut pending = std::mem::take(&mut self.pending);
        ctx.input_mut(|input| {
            if !pending.is_empty() {
                if input.consume_key(Modifiers::NONE, Key::Escape) {
                    pending.clear();
                    return;
                }
                let mut next = keyseq::options(&seqs, &pending);
                next.sort_by_key(|o| weight(&o.chord));
                for o in next {
                    if !input.consume_shortcut(&o.chord) {
                        continue;
                    }
                    if o.leaf && o.more == 0 {
                        out.push((self.bindings[o.first].make)());
                        pending.clear();
                    } else {
                        pending.push(o.chord);
                        self.since = Instant::now();
                    }
                    return;
                }
                // A key that continues nothing cancels the sequence (and is then handled as a
                // normal key); with no key pressed this frame we keep waiting.
                if input
                    .events
                    .iter()
                    .any(|e| matches!(e, Event::Key { pressed: true, .. }))
                {
                    pending.clear();
                } else {
                    return;
                }
            }
            for binding in self.bindings.iter().filter(|b| b.seq.len() == 1) {
                if typing && !binding.in_text {
                    continue;
                }
                if input.consume_shortcut(&binding.seq[0]) {
                    out.push((binding.make)());
                }
            }
            if !typing {
                let mut first = keyseq::options(&seqs, &[]);
                first.retain(|o| !o.leaf);
                first.sort_by_key(|o| weight(&o.chord));
                for o in first {
                    if input.consume_shortcut(&o.chord) {
                        pending.push(o.chord);
                        self.since = Instant::now();
                        break;
                    }
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
        self.pending = pending;
        if !self.pending.is_empty() {
            // Wake up once more to hide the panel if the user walks away.
            ctx.request_repaint_after(PENDING_TIMEOUT + Duration::from_millis(50));
        }
    }

    /// The which-key panel's content, or `None` when no sequence is in progress.
    pub fn hints(&self) -> Option<Hints> {
        if self.pending.is_empty() {
            return None;
        }
        let depth = self.pending.len();
        let opts = keyseq::options(&self.seqs(), &self.pending);
        let items = opts
            .iter()
            .map(|o| {
                let b = &self.bindings[o.first];
                let key = b.tokens[depth].clone();
                if o.leaf && o.more == 0 {
                    Hint {
                        key,
                        label: label_of(b.name),
                        group: false,
                    }
                } else {
                    let path = b.tokens[..=depth].join(" ");
                    let name = GROUPS
                        .iter()
                        .find(|(p, _)| *p == path)
                        .map_or_else(|| format!("{} more", o.more), |(_, n)| (*n).to_owned());
                    Hint {
                        key,
                        label: name,
                        group: true,
                    }
                }
            })
            .collect::<Vec<_>>();
        let prefix = self.bindings[opts.first()?.first].tokens[..depth].join(" ");
        Some(Hints { prefix, items })
    }

    /// Every bound action with all of its keys, for the `?` panel (in [`ACTIONS`] order).
    pub fn help_rows(&self) -> Vec<HelpRow> {
        ACTIONS
            .iter()
            .filter_map(|(name, _, _)| {
                let keys: Vec<String> = self
                    .bindings
                    .iter()
                    .filter(|b| b.name == *name)
                    .map(|b| b.tokens.join(" "))
                    .collect();
                (!keys.is_empty()).then(|| HelpRow {
                    keys: keys.join(" / "),
                    label: label_of(name),
                })
            })
            .collect()
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
            assert!(parse_sequence(shortcut).is_ok(), "{shortcut}");
            assert!(action_by_name(action).is_some(), "{action}");
        }
    }

    #[test]
    fn config_overrides_and_unbinds() {
        let (map, errors) =
            Hotkeys::from_config("Ctrl+B = ToggleHidden\nF2 = none\nbogus\nF9 = Nope\n");
        assert_eq!(errors.len(), 2);
        let ctrl_b = parse_shortcut("Ctrl+B").unwrap();
        let b = map.bindings.iter().find(|b| b.seq == [ctrl_b]).unwrap();
        assert!(matches!((b.make)(), Action::ToggleHidden));
        let f2 = parse_shortcut("F2").unwrap();
        assert!(map.bindings.iter().all(|b| b.seq != [f2]));
    }

    #[test]
    fn parses_sequences() {
        let (seq, tokens) = parse_sequence("Space t n").unwrap();
        assert_eq!(seq.len(), 3);
        assert_eq!(tokens, ["Space", "t", "n"]);
        assert_eq!(seq[0].logical_key, Key::Space);
        assert!(parse_sequence("   ").is_err());
        assert!(parse_sequence("Space Banana").is_err());
    }

    #[test]
    fn help_key_is_bound_both_ways() {
        let map = Hotkeys::default_map();
        for text in ["Shift+/", "?"] {
            let want = parse_shortcut(text).unwrap();
            assert!(
                map.bindings
                    .iter()
                    .any(|b| b.seq == [want] && b.name == "ToggleHelp")
            );
        }
    }

    #[test]
    fn a_prefix_conflict_drops_the_longer_binding() {
        let (map, errors) = Hotkeys::from_config("g = Refresh\n");
        assert_eq!(
            errors.len(),
            3,
            "g u, g b and g f are all shadowed: {errors:?}"
        );
        assert!(map.bindings.iter().all(|b| b.tokens != ["g", "u"]));
        assert!(map.bindings.iter().any(|b| b.tokens == ["g"]));
    }

    #[test]
    fn hints_follow_the_pending_prefix() {
        let mut map = Hotkeys::default_map();
        assert!(map.hints().is_none());
        map.pending = vec![parse_shortcut("Space").unwrap()];
        let h = map.hints().unwrap();
        assert_eq!(h.prefix, "Space");
        let t = h.items.iter().find(|i| i.key == "t").unwrap();
        assert!(t.group && t.label == "tabs");
        let b = h.items.iter().find(|i| i.key == "b").unwrap();
        assert!(!b.group && b.label == "Toggle sidebar");
        map.pending.push(parse_shortcut("t").unwrap());
        let h = map.hints().unwrap();
        assert_eq!(h.prefix, "Space t");
        assert_eq!(h.items.len(), 4);
    }

    #[test]
    fn help_rows_join_every_key_of_an_action() {
        let rows = Hotkeys::default_map().help_rows();
        let sidebar = rows.iter().find(|r| r.label == "Toggle sidebar").unwrap();
        assert_eq!(sidebar.keys, "Ctrl+B / Space b");
        let help = rows.iter().find(|r| r.label == "Toggle help").unwrap();
        assert!(help.keys.contains('?'));
    }
}
