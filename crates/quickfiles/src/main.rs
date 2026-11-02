// Hide the console window in release builds on Windows.
#![cfg_attr(
    all(not(debug_assertions), target_os = "windows"),
    windows_subsystem = "windows"
)]

mod app;
mod data;
mod hotkeys;
mod model;
mod state;
mod theme;
mod ui;
mod worker;

use eframe::egui;

fn main() -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("quickfiles")
            .with_inner_size([1152.0, 760.0])
            .with_min_inner_size([760.0, 440.0])
            // The UI draws its own title bar / window buttons and resize grips (ui/resize.rs).
            .with_decorations(false)
            .with_resizable(true),
        ..Default::default()
    };

    eframe::run_native(
        "quickfiles",
        options,
        Box::new(|cc| Ok(Box::new(app::App::new(cc)))),
    )
}
