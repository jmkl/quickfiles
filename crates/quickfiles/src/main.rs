// Hide the console window in release builds on Windows.
#![cfg_attr(
    all(not(debug_assertions), target_os = "windows"),
    windows_subsystem = "windows"
)]

mod app;
mod config;
mod hotkeys;
mod ids;
mod keyseq;
mod opener;
mod state;
mod theme;
mod ui;

use eframe::egui;

/// The folder to open from the command line: `qf.exe .`, `qf.exe C:\Users`, `qf.exe "..\x"`.
/// The first argument that is not a `--flag`; relative paths resolve against the current
/// directory, and a file opens its parent folder. `None` when nothing usable was given.
fn start_dir() -> Option<std::path::PathBuf> {
    let arg = std::env::args_os()
        .skip(1)
        .find(|a| !a.to_string_lossy().starts_with("--"))?;
    // `qf.exe "C:\Some Dir\"` reaches us with a stray quote on some shells; trim it.
    let arg = std::path::PathBuf::from(arg.to_string_lossy().trim_matches('"'));
    let abs = std::path::absolute(&arg).ok()?;
    // Resolve `.` / `..` and symlinks; keep the plain form if that fails (e.g. a dead drive).
    let path = std::fs::canonicalize(&abs).unwrap_or(abs);
    // canonicalize returns `\\?\C:\...` on Windows; the rest of the app uses plain paths.
    let plain = path
        .to_str()
        .and_then(|s| s.strip_prefix(r"\\?\"))
        .filter(|s| !s.starts_with("UNC\\"))
        .map_or(path.clone(), std::path::PathBuf::from);
    if plain.is_dir() {
        Some(plain)
    } else if plain.is_file() {
        plain.parent().map(std::path::Path::to_path_buf)
    } else {
        None
    }
}

fn main() -> eframe::Result {
    // `--dummy`: fake file system, no Everything needed (UI work).
    let dummy = std::env::args().any(|a| a == "--dummy");
    let start = start_dir();

    // A second launch shows the running window (which may be hidden in the tray) and hands it the
    // folder, then quits. `--dummy` is for UI work and may run next to the real one.
    if !dummy && !quickfiles_core::single_instance::claim_or_forward("quickfiles", start.as_deref())
    {
        return Ok(());
    }

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
        Box::new(move |cc| Ok(Box::new(app::App::new(cc, dummy, start)))),
    )
}
