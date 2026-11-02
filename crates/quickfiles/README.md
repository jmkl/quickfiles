# quickfiles

Prototype of a dark file-manager window built with `egui`/`eframe` 0.37 (Rust 2024).
All data is dummy data; the layout was measured pixel-by-pixel from the reference screenshot (1152x760).

    cargo run --release

## Layout of the code

| File | Responsibility |
|---|---|
| `src/theme.rs` | **All** colours (`Palette`), sizes (`Metrics`), fonts and egui style. Re-skin here only. |
| `src/model.rs` | Plain data types (`FileEntry`, `Listing`, `Sort`, ...). No egui. |
| `src/data.rs` | Dummy data + dummy directory generator. Swap for real FS / drive queries later. |
| `src/worker.rs` | Background thread: receives `Command`s, sends `Event`s, wakes the UI with `request_repaint`. |
| `src/hotkeys.rs` | **All** keyboard shortcuts, in one table. Shortcuts produce the same `Action`s the buttons do. |
| `src/state.rs` | All mutable UI state; owns the cached filtered/sorted `view`. |
| `src/app.rs` | Glue: polls the worker, turns `Action`s into state changes. |
| `src/ui/layout.rs` | Splits the window into regions (title, sidebar, toolbar, header, list, optional preview). |
| `src/ui/{title_bar,sidebar,toolbar,file_list,preview}.rs` | One module per region. The title bar holds the search box (filters the current folder; Ctrl+F / Ctrl+L focuses it, Esc clears it). |
| `src/ui/splitter.rs` | Drag handles to resize the left sidebar and the preview panel. |
| `src/ui/resize.rs` | Invisible edge/corner grips that start a native resize (the window is borderless). |
| `src/ui/icons.rs` | Vector icons drawn with the painter (no assets, no icon font). |
| `src/ui/widgets.rs` | Shared helpers: crisp 1px lines, truncated text, icon buttons. |

## Design notes

- **Threading**: the UI thread never touches the file system. Requests are coalesced on the worker
  (only the newest navigation is processed) and stale results are dropped by request id.
  Large dummy directories (up to 20k entries) are generated in parallel with `thread::scope`.
- **Performance**: the file list is virtualised (`ScrollArea::show_rows`); filtering/sorting runs only
  when the filter, sort or listing changes, never per frame; text is laid out once per frame per visible cell.
- **Interaction**: click selects, Ctrl+click toggles, double-click on a folder opens it, header cells sort,
  breadcrumb segments / Up / Back / Forward navigate, drag the title bar to move the window,
  drag any window edge/corner to resize it. Selecting a file opens the preview panel on the right
  (hidden for folders and when the window is too narrow). File-list columns drop out as the window shrinks.

## Panels and hotkeys

Both side panels slide in/out (eased, 0.18 s) and are resized by dragging their inner edge.
The hamburger button toggles the left sidebar; the panel button next to the window buttons toggles the preview.
Every operation is an `Action`, reachable from a button, a hotkey, or both.

| Hotkey | Action |
|---|---|
| Ctrl+B | Toggle left sidebar |
| Alt+P | Toggle preview panel |
| Ctrl+F / Ctrl+L | Focus the search box (Esc clears it) |
| Alt+Left / Alt+Right / Alt+Up, Backspace | Back / Forward / Up |
| Up / Down | Move the active item |
| Enter | Open the active folder |
| Ctrl+A | Select all |
| Esc | Clear search, then selection |
| F5 | Reload folder |

Plain keys (arrows, Enter, Backspace, Esc, Ctrl+A) are ignored while a text box has focus.
