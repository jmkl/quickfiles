//! Hotkey system: one table that maps shortcuts to [`Action`]s.
//!
//! Every user operation (buttons included) goes through `Action`, so a hotkey is just another
//! way to produce one. To add or change a shortcut, edit [`Hotkeys::default_map`] only.
//!
//! `in_text` says whether a binding still fires while a text box has focus. Plain keys
//! (arrows, Enter, Backspace, Esc, Ctrl+A) must not, or they would steal editing keys.

use eframe::egui::{Context, Id, Key, KeyboardShortcut, Modifiers};

use crate::ui::Action;

struct Binding {
    shortcut: KeyboardShortcut,
    make: fn() -> Action,
    in_text: bool,
}

pub struct Hotkeys {
    bindings: Vec<Binding>,
}

impl Hotkeys {
    pub fn default_map() -> Self {
        let b = |mods: Modifiers, key: Key, in_text: bool, make: fn() -> Action| Binding {
            shortcut: KeyboardShortcut::new(mods, key),
            make,
            in_text,
        };
        let none = Modifiers::NONE;
        let cmd = Modifiers::COMMAND;
        let alt = Modifiers::ALT;

        // Bindings with modifiers first, plain keys last.
        Self {
            bindings: vec![
                // Panels
                b(cmd, Key::B, true, || Action::ToggleSidebar), // left sidebar
                b(alt, Key::P, true, || Action::TogglePreview), // preview panel
                // Search
                b(cmd, Key::F, true, || Action::FocusSearch),
                b(cmd, Key::L, true, || Action::FocusSearch),
                // Navigation
                b(alt, Key::ArrowLeft, false, || Action::Back),
                b(alt, Key::ArrowRight, false, || Action::Forward),
                b(alt, Key::ArrowUp, false, || Action::Up),
                b(none, Key::Backspace, false, || Action::Up),
                b(none, Key::F5, true, || Action::Refresh),
                // List
                b(cmd, Key::A, false, || Action::SelectAll),
                b(none, Key::ArrowDown, false, || Action::MoveFocus(1)),
                b(none, Key::ArrowUp, false, || Action::MoveFocus(-1)),
                b(none, Key::Enter, false, || Action::OpenFocused),
                b(none, Key::Escape, false, || Action::Escape),
            ],
        }
    }

    /// Consumes the shortcuts pressed this frame and pushes their actions.
    pub fn poll(&self, ctx: &Context, out: &mut Vec<Action>) {
        // Only the search box counts as "typing". `wants_keyboard_input()` is also true when a
        // clicked button holds focus, which would silently disable every plain-key hotkey.
        let typing = ctx.memory(|m| m.focused() == Some(Id::new("title_search")));
        ctx.input_mut(|input| {
            for binding in &self.bindings {
                if typing && !binding.in_text {
                    continue;
                }
                if input.consume_shortcut(&binding.shortcut) {
                    out.push((binding.make)());
                }
            }
        });
    }
}
