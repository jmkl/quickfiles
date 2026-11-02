//! Glue: owns the theme, the state and the worker; turns UI `Action`s into state changes.

use std::path::PathBuf;

use eframe::egui;

use crate::{
    hotkeys::Hotkeys,
    model,
    state::State,
    theme::Theme,
    ui::{self, Action, Cx},
    worker::{Command, Event, Worker},
};

pub struct App {
    theme: Theme,
    state: State,
    worker: Worker,
    hotkeys: Hotkeys,
}

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        let theme = Theme::dark();
        theme.apply(&cc.egui_ctx);

        let mut app = Self {
            theme,
            state: State::new(),
            worker: Worker::spawn(cc.egui_ctx.clone()),
            hotkeys: Hotkeys::default_map(),
        };
        app.load_current();
        app
    }

    fn load_current(&mut self) {
        self.state.request_id += 1;
        self.state.loading = true;
        // The search box filters the folder you are in; a new folder starts unfiltered.
        self.state.filter.clear();
        self.worker.send(Command::Load {
            id: self.state.request_id,
            path: self.state.path.clone(),
        });
    }

    fn pump_worker(&mut self) {
        for event in self.worker.events() {
            match event {
                // Ignore results of requests the user has already navigated away from.
                Event::Loaded { id, listing } if id == self.state.request_id => {
                    self.state.set_listing(listing);
                }
                Event::Loaded { .. } => {}
            }
        }
    }

    fn navigate(&mut self, path: PathBuf) {
        if path == self.state.path {
            return;
        }
        let old = std::mem::replace(&mut self.state.path, path);
        self.state.back.push(old);
        self.state.forward.clear();
        self.load_current();
    }

    fn apply(&mut self, action: Action) {
        #[cfg(debug_assertions)]
        eprintln!("action: {action:?}");
        match action {
            Action::Navigate(path) => self.navigate(path),
            Action::OpenFolder(name) => {
                let path = self.state.path.join(name);
                self.navigate(path);
            }
            Action::Up => {
                if let Some(parent) = model::parent_of(&self.state.path) {
                    self.navigate(parent);
                }
            }
            Action::Back => {
                if let Some(prev) = self.state.back.pop() {
                    let current = std::mem::replace(&mut self.state.path, prev);
                    self.state.forward.push(current);
                    self.load_current();
                }
            }
            Action::ToggleSidebar => self.state.left_open = !self.state.left_open,
            Action::TogglePreview => self.state.preview_open = !self.state.preview_open,
            Action::FocusSearch => self.state.focus_search = true,
            Action::Refresh => self.load_current(),
            Action::SelectAll => {
                let view = self.state.view.clone();
                self.state.selected.extend(view);
            }
            Action::Escape => {
                // First Esc clears the search, the next one the selection.
                if self.state.filter.is_empty() {
                    self.state.selected.clear();
                } else {
                    self.state.filter.clear();
                    self.state.rebuild_view();
                }
            }
            Action::MoveFocus(delta) => self.state.move_focus(delta),
            Action::OpenFocused => {
                let folder = self.state.focused.and_then(|f| {
                    let listing = self.state.listing.as_ref()?;
                    let entry = listing.entries.get(f as usize)?;
                    entry.is_dir().then(|| entry.name.clone())
                });
                if let Some(name) = folder {
                    self.apply(Action::OpenFolder(name));
                }
            }
            Action::Forward => {
                if let Some(next) = self.state.forward.pop() {
                    let current = std::mem::replace(&mut self.state.path, next);
                    self.state.back.push(current);
                    self.load_current();
                }
            }
        }
    }
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.pump_worker();

        // Hotkeys are applied before drawing, so their effect shows in this very frame.
        let mut hotkey_actions = Vec::new();
        self.hotkeys.poll(ui.ctx(), &mut hotkey_actions);
        for action in hotkey_actions {
            self.apply(action);
        }

        let mut actions = Vec::new();
        egui::CentralPanel::default()
            .frame(egui::Frame::NONE)
            .show(ui, |panel_ui| {
                let mut cx = Cx {
                    theme: &self.theme,
                    state: &mut self.state,
                    actions: &mut actions,
                };
                ui::draw(panel_ui, &mut cx);
            });

        if !actions.is_empty() {
            // Actions change state after this frame was drawn; draw the result right away.
            ui.ctx().request_repaint();
        }
        for action in actions {
            self.apply(action);
        }
    }
}
