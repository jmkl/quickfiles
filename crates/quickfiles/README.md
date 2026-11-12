# quickfiles

Windows-only file manager built on **voidtools Everything** (required) with `egui`/`eframe` 0.37, Rust 2024.

    cargo run --release            # needs Everything installed and running
    cargo run -- --dummy           # fake file system, no Everything (UI work)

Type in the search box to search everything. Double-click (or Enter on) a folder to browse it;
from then on typing searches below that folder. The x button in the box goes back to a global search.

## Layout of the code

| File | Responsibility |
|---|---|
| `src/theme.rs` | **All** colours (`Palette`), sizes (`Metrics`), fonts and egui style. Re-skin here only. |
| `src/hotkeys.rs` | **All** keyboard shortcuts, in one table, overridable in the `[hotkeys]` section of `config.conf`. Shortcuts produce the same `Action`s the buttons do. |
| `src/state.rs` | UI-only state: panels, selection (by stable row key), rename, menu, toast. |
| `src/app.rs` | Glue: owns `quickfiles_core::Core`, turns `Action`s into core calls. |
| `src/ui/layout.rs` | Splits the window into regions (title, sidebar, toolbar, header, list, optional preview). |
| `src/ui/{title_bar,sidebar,toolbar,file_list,preview}.rs` | One module per region. |
| `src/ui/{gate,menu,tex}.rs` | "Everything required" screen, context menu + toast, GPU texture cache. |
| `src/ui/{tabs,grid,dnd}.rs` | Tab strip, thumbnail grid, drag and drop. |

Everything about files lives in `crates/quickfiles-core` (no egui there): Everything IPC engine,
navigation state machine, SQLite store (recents, bookmarks, history, prefs), icon / thumbnail /
preview service, file operations and the folder watcher. See `PLAN.md`.

## Keys

Ctrl+F/L focus search · Ctrl+R regex · Ctrl+H hidden files · Ctrl+B sidebar · Alt+P preview ·
Alt+Left/Right/Up, Backspace navigate · Up/Down/PageUp/PageDown/Shift+arrows move the selection (also from the search box) ·
Enter open · Esc clear text, then selection · F2 rename · Delete recycle · Shift+Delete delete ·
Ctrl+C/X/V (shared with Explorer's clipboard) · Ctrl+Shift+N new folder · Ctrl+Shift+C copy path · F5 refresh ·
Ctrl+T new tab · Ctrl+W close tab · Ctrl+Tab next tab · Ctrl+G thumbnail grid ·
Ctrl + mouse wheel up/down switches to the grid / the list ·
F12 performance overlay (Ctrl+F12 copies a report, Shift+F12 resets).

Drag rows onto a folder (list, grid, sidebar, breadcrumb) to move, hold Ctrl to copy; drop files from Explorer
onto the window to copy them into the current folder.

## Settings

Everything lives in `%LOCALAPPDATA%\quickfiles\`: `quickfiles.db` (history, bookmarks, prefs, column widths),
`window.txt` (window size), `config.conf` (written on first run: `[general]` global hotkey, `[theme]` colours, `[hotkeys]` bindings such as `Ctrl+B = ToggleSidebar` or `F2 = none`; Ctrl+Shift+R reloads it).

## Code style

- Formatting is `rustfmt` with the repo's `rustfmt.toml` (edition 2024, 100 columns). Run
  `cargo fmt --all` before every commit and check with `cargo fmt --all -- --check`.
- Lints: `cargo clippy --workspace --all-targets` should stay clean.
- Format first, then apply changes: when receiving files or patches, run `cargo fmt --all` as its own
  commit beforehand, so a patch never has to match your formatting line by line.
- UI code only draws and returns `Action`s; anything with I/O lives in `quickfiles-core`.

## Measuring performance

`F12` shows an overlay with the time our UI code takes per frame, texture uploads, the image
queue and decode / disk-cache counters. Press `Ctrl+F12` to copy a plain-text report, and
`Shift+F12` to reset the counters before a test. `tools/make-test-folders.ps1` creates the folders
used by `PLAN-PERF.md` (1,000 small JPEGs, 200 x 24 MP JPEGs, a mixed folder, 20,000 files).
