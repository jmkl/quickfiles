// Hide the console window in release builds on Windows.
#![cfg_attr(
    all(not(debug_assertions), target_os = "windows"),
    windows_subsystem = "windows"
)]

mod app;
mod hotkeys;
mod state;
mod theme;
mod ui;

use eframe::egui;

fn main() -> eframe::Result {
    // `--dummy`: fake file system, no Everything needed (UI work).
    let dummy = std::env::args().any(|a| a == "--dummy");

    // Reopen at the size the window had last time.
    let saved = quickfiles_core::sys::load_window();
    let size = saved.map_or([1152.0, 760.0], |w| [w.w, w.h]);
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("quickfiles")
            .with_inner_size(size)
            .with_maximized(saved.is_some_and(|w| w.maximized))
            .with_min_inner_size([760.0, 440.0])
            // The UI draws its own title bar / window buttons and resize grips (ui/resize.rs).
            .with_decorations(false)
            .with_resizable(true),
        ..Default::default()
    };

    eframe::run_native(
        "quickfiles",
        options,
        Box::new(move |cc| Ok(Box::new(app::App::new(cc, dummy)))),
    )
}
