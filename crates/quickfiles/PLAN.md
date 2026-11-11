# Implementation status (read this first)

All phases 0-7 are written. **Nothing here has been compiled against its real dependencies**
(the build sandbox had no Rust toolchain and no access to crates.io). What was verified earlier:
the dependency-free core (model, fmt, nav, compose, paging, view, engine + dummy backend, jobs,
core.rs, the image service logic) compiled and passed 55 tests under a stub harness. Everything
added in the latest pass (below) was only read through and bracket-checked, not compiled.
Not compiler-checked: the whole UI crate (egui calls), `engine/everything.rs`, `sys.rs` (including
the new Windows clipboard code), `store/`, `images/{shell,decode,cache}.rs`, `watch.rs`, and the
new tests. Expect a handful of API-signature fixes on first `cargo check`; the likeliest spots are
the `windows` 0.62 clipboard calls in `sys.rs` (`GlobalAlloc`, `SetClipboardData`, `DragQueryFileW`
signatures) and `Key::from_name` in `hotkeys.rs`.

Phase checkboxes below mean "written", not "verified on Windows".

**Latest pass:** the whole of `PLAN-PERF.md` (phases 2-5: preview pipeline, faster decoding, SQLite
thumbnail cache, scroll gating) plus the gaps listed under "Added in the latest pass". Its pure logic
passed 36 unit tests in a stub harness; the new code that touches `jpeg-decoder`, `image`, `rusqlite`,
egui and Windows was not compiled. New dependency: `jpeg-decoder` 0.3 (`default-features = false`).

Run `cargo run -p quickfiles -- --dummy` first (fake file system, no Everything needed), then
`cargo test --workspace`.

## Conventions

`rustfmt.toml` + `.editorconfig` at the repo root. Run `cargo fmt --all` before committing. The files
in the latest passes were written without a rustfmt available, so run it once on the whole tree.

## Added in the latest pass (Phase 5-7 gaps)

- **Image pipeline** (details in `PLAN-PERF.md`): three work lanes (icon / thumbnail / preview), job
  cancellation, prefetch priority, byte-bounded memory and texture caches, panel-sized debounced
  preview with a progressive thumbnail, JPEG scaled decode and EXIF thumbnails, `thumbs.db` write-behind
  cache with versioning and LRU pruning, scroll-speed gating, "Clear thumbnail cache" in the kebab menu.
- **Grid sort controls:** "Sort by" chips (Name / Type / Date Modified / Size) in the grid's header row;
  clicking the active one flips the direction.
- **Smoke test against a running Everything:** `engine/everything.rs` `mod smoke` (Windows only; prints
  "skipped" and passes when Everything is not running). Covers `parent:` children only, folder / file
  segments, scoped recursion, the hidden filter, `max_results(1)` still reporting the total, and child
  counts matching the listing.
- **Benchmarks:** `cargo run --release -p quickfiles-core --example bench_query -- [--dummy] "<query>" [folder]`
  (time to first row, page fetch after a jump) and `--example bench_images -- <folder>`.

- **Window size** persists in `window.txt` (read before the window opens, so not in SQLite);
  maximised state too.
- **Column resize:** drag a header divider (anchored right, so dragging left grows the column),
  double-click resets; widths saved in prefs (`col_w`).
- **Scroll position per state:** Back/Forward/tab switch restore the scroll offset of that
  (location, text); fresh navigation starts at the top.
- **OS clipboard:** copy/cut put `CF_HDROP` + `Preferred DropEffect` on the clipboard, paste prefers
  files someone else put there (Explorer interop). Ctrl+Shift+C copies paths as text.
- **Conflict prompt:** paste and drops ask once (Replace / Keep both / Skip) when names clash.
- **Byte-level job progress** (copy shows % by bytes, chunked copy, cancel removes the partial
  file), links and junctions are never followed, modified time is preserved.
- **Drag and drop:** rows/tiles onto folders in the list, grid, sidebar and breadcrumb (Ctrl = copy);
  files dropped from Explorer are copied into the current folder.
- **Tabs:** `Core` keeps a tab slot per `Nav` (Ctrl+T / Ctrl+W / Ctrl+Tab); strip appears with 2+ tabs.
- **Hotkeys file:** `hotkeys.conf` overrides the built-in table (template written on first run).
- **Thumbnail grid** (Ctrl+G, or the kebab menu) on the Thumb tier.
- **This PC view:** drives show File System / Free Space / Size / Percent Full in the list and a
  usage-bar tile in the grid (`ui/drives.rs`); `DriveInfo` gained `fs`.
- Not-indexed folders fall back to a live `read_dir` listing (`engine::list_dir`); drive-root
  parents are normalised to `E:\`.
- Removed orphan files that were no longer part of the build: UI `model.rs`, `data.rs`, `worker.rs`,
  `ui/status_bar.rs`.

## Still not done

- Drag **out** to Explorer (needs OLE `DoDragDrop` with an `IDropSource` / `IDataObject`). Dropping in
  works. Left out on purpose: it cannot be checked without a Windows build, and a wrong COM
  implementation fails at runtime, not at compile time.
- The smoke test and the benchmarks exist now but have **not been run** against a real Everything or a
  million-row index; the `PLAN-PERF.md` baseline table is still empty.
- Tabs keep only their `Nav`; a tab's scroll is restored through the per-query scroll memory, not
  stored in the tab.
- Texture atlas for small thumbnails (optional item in `PLAN-PERF.md` phase 5).
- The status-bar type filter (open decision 1) is moot: the status bar was removed.

Deviations from the plan below:
- Thumbnail disk cache is now `thumbs.db` (SQLite, 512 MB by default, pref `thumb_cache_mb`) as planned;
  the interim one-PNG-per-file `thumbs/` folder is deleted on first start.
- No `crossbeam` / `thiserror`; std mpsc and a small hand-written error type.
- `open` crate instead of `opener`; Everything version string not shown.
- Result count, timing and Everything state live at the right end of the toolbar row.
- Window size lives in `window.txt` and hotkeys in `hotkeys.conf`, not in the database.

---

# PLAN: from prototype to real file manager

Goal: replace the dummy data with the real file system **without touching the UI design**.
The UI already has the hard parts: a worker thread, request ids that drop stale results,
a virtualised list, and a single `Action` pipeline that buttons and hotkeys share.

quickfiles is a **companion to voidtools Everything**. Everything is a hard requirement: all searching *and*
all folder listing go through `everything-ipc`. If Everything is not installed or not running, the app does not
degrade to some other mode; it shows a "Everything is required" screen and waits.

Everything that touches the OS lives in a second crate, **`quickfiles-core`** (Everything engine, database,
thumbnails, drives). The UI crate is a thin client of one small API. Updated 2026-11-01.

## How the app works

One search bar, two modes. The *current folder* is always the scope of the next query.

1. **Search mode (global).** Nothing is entered yet. Typing queries the whole Everything index; rows show the name plus the muted parent path. With an empty box the list is an empty state ("Search everything...").
2. **Explore mode.** Click or Enter on a folder (from search results, the sidebar, or a breadcrumb) makes it the current location and **clears the search box**, whose hint reads "Search in <folder>". The list shows that folder's children. Arrows, Enter, Backspace, Alt+Left/Right/Up, breadcrumb and sidebar only change the location.
3. **Scoped search.** Typing while in Explore mode searches *under the current folder* (recursive). Clicking a folder in those results walks into it, and the scope follows.
4. **x button beside the input** (visible when there is text or a folder scope): clears the text **and drops the folder scope**, so the next query searches everything again. Esc clears only the text and keeps the scope. "This PC" is Explore mode at the empty path; typing there searches globally.
5. **Back/Forward** restore the exact previous state (location *and* text), so Back from a folder returns you to the search results you came from.

| location | text | what core asks |
|---|---|---|
| none (Search mode) | empty | nothing: empty state |
| none | text | Everything: `text` |
| folder | empty | Everything: `parent:"<dir>"` (children) |
| folder | text | Everything: `"<dir>\" text` (recursive under the folder) |
| This PC (empty path) | empty | drive list from the Windows API (not Everything) |
| This PC | text | Everything: `text` (global) |

This logic is a small pure state machine (`Nav`, in core), so it is unit-tested and later reusable per tab.

## Workspace

```
crates/
  quickfiles/        egui app: drawing, hotkeys, UI-only state. Depends on eframe + quickfiles-core.
  quickfiles-core/   Everything engine, database, thumbnails, drives, Nav. No egui.
```

Root `Cargo.toml`: add `"crates/quickfiles-core"` to `members` and `quickfiles-core = { path = "crates/quickfiles-core" }`
to `[workspace.dependencies]`. Do not name the crate `core` (it collides with Rust's built-in `core`).

`eframe` stays as it is in the workspace (`default-features = false`, `default_fonts`, `wgpu`). There is no
`persistence` feature on purpose: preferences live in the core database, not in eframe storage.

### Dependencies (add each in the phase that first needs it)

| Crate | Used for | Phase |
|---|---|---|
| `everything-ipc` 0.1.4 | Everything search (`wm` interface: sync, Everything 1.4 and 1.5) | 2 |
| `windows` 0.62 | drives, volume info, shell thumbnails/icons. Same major as `everything-ipc`, so only one copy is built | 3 |
| `crossbeam-channel` | channels between threads | 1 |
| `chrono` (feature `clock` only) | date strings from Everything's FILETIME | 2 |
| `opener` | open a file with its default app | 2 |
| `thiserror` | core error type | 1 |
| `mimalloc` (app crate) | global allocator, faster for many small strings | 2 |
| `rusqlite` (feature `bundled`) | history, bookmarks, prefs, thumbnail index | 3 |
| `dirs` | Desktop, Documents, Downloads, data dir | 3 |
| `image` | decode/resize fallback for thumbnails and previews | 4 |
| `trash` | recycle bin | 5 |
| `notify` | folder watcher | 6 |

Deliberately **not** used: `tokio` (the Everything IPC is blocking and handles one query per client; plain threads fit)
and `rayon` (no data-parallel work worth a pool; thumbnails use a small dedicated pool).

## Principles

1. **The UI thread never touches the file system or Everything.** Every operation is a request to core.
2. **Stale results are dropped** by request id. Reuse it for every job type.
3. **Stream, don't block.** Pages arrive as they are ready; the first rows show immediately.
4. **Paths are `PathBuf`, not `String`.** Convert to `String` only for display. The empty path is the virtual root "This PC".
5. **The UI crate has no OS dependencies**: no `std::fs`, `windows`, `everything-ipc`, `rusqlite`. The dependency list enforces it.
6. **Core knows nothing about egui.** It gets a `wake` callback (`move || ctx.request_repaint()`) and returns plain data (RGBA bytes, not textures).
7. **The UI never holds a full result set.** It gets a *windowed view*: total count plus the rows near the viewport. A global search can have millions of rows.
8. **Rows are identified by a stable key, not by index.** Indices change when results refresh.
9. **Everything is required, with no fallback listing.** Missing or stopped Everything is a first-class screen, not an error toast.
10. **Reads are lock-free snapshots, writes are asynchronous.** The sidebar reads `Arc` snapshots every frame; saving never blocks a frame.
11. **One change per PR**, each runnable on its own. The dummy backend stays (behind `--dummy`) so UI work never needs Everything.

## Core API (sketch)

```rust
let core = Core::start(Config { data_dir, wake })?;

// Everything gate
core.status();            // Connecting | Missing | LoadingIndex | Ready { version }
core.retry();             // reconnect now (F5, "Retry" button)
core.start_everything();  // launches Everything.exe when it can be found

// navigation state: pure logic, no I/O; one per tab later; lives in the UI State
let mut nav = Nav::new();
nav.type_text("foo");     // Search mode: global. Explore mode: scoped to the location
nav.open_folder(path);    // enters Explore mode, clears the text
nav.up(); nav.back(); nav.forward();
nav.clear();              // the x button: clears text AND drops the scope -> Search mode, empty
nav.escape();             // clears the text only
nav.query();              // -> &Query

// browse and search: one call
let view = core.open(nav.query());   // Query { location: Option<PathBuf>, text, regex, show_hidden, sort, folders_first }
view.len();               // total rows
view.state();             // Loading | Ready | Empty { not_indexed: bool } | Error(msg)
view.get(i);              // Option<EntryRef>: cheap handle, None = page not loaded yet
view.want(first..last);   // visible range; core fetches and prefetches pages
view.took();              // last query time, for the status bar

core.poll();              // once per frame: drains events, returns whether anything changed

// sidebar and history (Arc snapshots, no locks held by the UI)
core.drives(); core.places(); core.recents(); core.bookmarks();
core.visit(&path); core.toggle_bookmark(&path); core.is_bookmarked(&path);

// preferences
core.pref("sort", ..); core.set_pref(..);

// images
core.thumb(&entry, Tier::Thumb);   // Ready(Arc<Pixels>) | Pending | None
core.preview(&entry);              // Text(first 64 KB) | Image(Pixels) | Meta | Pending
```

`Entry` carries raw values (`size: u64`, `modified: i64`, flags) **and** display strings (`type_label`, date text, size text)
built on the engine thread when a page arrives, so a UI frame does no formatting or allocation.
`Entry::key() -> u64` is a hash of the lower-cased full path and is what selection and focus use.
`Entry::category()` (`Folder, Html, Script, Json, Image, Video, Audio, Archive, Code, Text, Pdf, Other`) replaces the
five-value `EntryKind` so more files get a proper icon.

### Threads

```
UI thread --Query--> engine thread --> Everything (or dummy) --pages--> page cache --> view.get()
UI thread --writes--> db thread (SQLite) --> snapshots (Arc) read by the UI
UI thread --thumb(entry)--> thumb pool (2-4 threads, below-normal priority) --> memory + disk cache
sys thread: drives and free space, refreshed every ~5 s while the sidebar is visible
```

Every thread calls `wake()` after producing something the UI can show.

### Layout of `quickfiles-core`

```
src/
  lib.rs           Core handle, Config, Event, re-exports (the only public entry point)
  model.rs         Entry, Category, Query, Sort; breadcrumbs / display_name / parent_of / format_size (moved from the UI, tests included)
  nav.rs           Nav: location + text + back/forward, the mode transitions above (pure, unit-tested)
  view.rs          View, page cache (LRU), windowing, folders-then-files stitching
  engine/          mod.rs (thread, coalescing, request ids, gate/status), compose.rs (query text builder), everything.rs, dummy.rs
  store/           mod.rs (db thread, snapshots), schema.rs (migrations via PRAGMA user_version)
  sys.rs           drives, volume labels, places, reveal in Explorer, locate Everything.exe
  images/          mod.rs (tiers, API), shell.rs (IShellItemImageFactory), cache.rs (disk), pool.rs (workers)
  jobs.rs          copy / move / delete with progress (Phase 5)
```

## Where things change

| File | Change |
|---|---|
| **core** `model.rs` | `Entry` gains full path, flags (hidden, system, symlink), raw `modified` + display text, `category`. `Sort`, path helpers and `format_size` move here from the UI. |
| **core** `nav.rs`, `engine/` | New. `ev.rs` from the earlier prototype (connect async, `run_search`, heartbeat, directory attribute check, FILETIME conversion) is ported into `everything.rs`. |
| **core** `store/`, `sys.rs`, `images/` | New. |
| `state.rs` | Loses `listing`, `view: Vec<u32>`, `request_id`, `loading`, `rebuild_view`, `path`, `back`, `forward`, sort/filter code. Holds `nav: Nav`, `view: Option<View>`, `selected: HashSet<u64>`, `focused: Option<u64>`. |
| `app.rs` | `Worker` replaced by `core.poll()`. Actions go through `Nav`. New actions: `Open(PathBuf)` (replaces `OpenFolder(String)`), `ClearSearch` (x button), `SetSort`, `ToggleRegex`, `ToggleHidden`, `ToggleBookmark`, `Retry`. |
| `worker.rs`, `data.rs` | Deleted. The dummy generator moves into core as the `--dummy` backend. |
| `model.rs` (UI) | Reduced to re-exports, then removed. |
| `hotkeys.rs` | Ctrl+H hidden files, Ctrl+R regex, F5 refresh (and reconnect when the gate is showing). Later: file operations. |
| `ui/title_bar.rs` | Search box edits `nav` text; hint "Search in <folder>" (already there) or "Search everything"; **x button** (`ClearSearch`) shown when there is text or a folder scope; regex toggle. |
| `ui/gate.rs` | New: full-window "Everything is required" screen (install link, Retry, "Start Everything"), and a "Everything is loading its index..." state. |
| `ui/file_list.rs` | `show_rows` over `view.len()`; rows read `view.get(i)`; a missing page draws a placeholder row; one `view.want()` call per frame. In Search mode the parent path is shown muted after the name. Empty states: "Search everything...", "No items", "Not indexed by Everything". Folders/Files columns show `--` until lazy counts arrive (Phase 6). |
| `ui/toolbar.rs` | Breadcrumb shows "Search results" in Search mode. Bookmark button toggles and lights up; history dropdown lists recents. |
| `ui/sidebar.rs` | Reads `core.recents()`, `bookmarks()`, `drives()`, `places()` instead of `data::*`. |
| `ui/status_bar.rs` | Shows `view.len()`, query time, Everything status and version, later job progress. |
| `ui/preview.rs` | `dummy_lines` replaced by `core.preview()`; images become textures held in a UI-side LRU. |
| `ui/icons.rs` | `Icon::for_kind(EntryKind)` becomes `Icon::for_category(Category)`. Painted vector icons stay as the instant placeholder. |

## Phase 0: groundwork (DONE)

- [x] `path` is a `PathBuf` across `state.rs`, `app.rs`, `toolbar.rs`, `sidebar.rs`, `worker.rs`, `data.rs`. The empty path is the virtual root "This PC".
- [x] Breadcrumbs built from path components (`model::breadcrumbs`); a Windows prefix + root (`C:` + `\`) is one segment. Helpers `display_name` and `parent_of` live in `model.rs` with unit tests.
- [x] Up goes from a top-level folder to "This PC"; the dummy "This PC" lists the drives.

**Result:** app behaves as before on dummy data, now on `PathBuf` paths.

## Phase 1: core skeleton, modes and UI adapted (1 day)

Runs entirely on the dummy backend. This proves the API, the windowed list and the mode logic before any real I/O exists.

- [x] Create `crates/quickfiles-core`; add it to the workspace.
- [x] Move the pure logic (`Sort`, `breadcrumbs`, `display_name`, `parent_of`, `format_size`) and its tests into core.
- [x] Define `Entry`, `Category`, `Query`, `Nav`, `View`, `Core`, `Event`, `CoreError`.
- [x] `Nav` with all transitions from "How the app works" (type, open folder, up, back, forward, clear, escape) and unit tests.
- [x] Dummy backend serves pages through the real view/page-cache code (so the windowing is exercised now, not later). It answers `parent:`-style browse and text queries.
- [x] Engine thread with coalescing and request ids (port of `worker.rs`).
- [x] UI: selection and focus by `u64` key; `Action::Open(PathBuf)`; `file_list` on `view.get(i)` + `view.want(..)`; x button and the two hints; breadcrumb label in Search mode; `ui/gate.rs` with a fake status for visual work; delete `worker.rs` and `data.rs`.
- [x] Sorting and "folders first" are part of the query, not UI code.

**Done when:** you can type (global results), click a folder (Explore, box cleared), walk with the keyboard, type again (scoped), press x (global again), and Back returns to the earlier search, all on dummy data, with the UI crate free of listing/sorting code and the 20,000-entry dummy folder still scrolling smoothly.

## Phase 2: Everything engine (2 days)

- [x] **Gate.** Connect on a background thread so the window opens instantly; try the default instance, then `Some("1.5a")`; check `is_ipc_available()`. Status `Missing` shows the gate screen (download link, Retry, and "Start Everything" when `Everything.exe` can be found in the registry or default install paths). `is_db_loaded()` false is `LoadingIndex`: a waiting state, not a bail. A heartbeat (as in the earlier prototype) detects Everything closing mid-session, shows the gate again, and reconnects automatically.
- [x] **One thread owns the client.** Everything runs one query per client and a newer query cancels the older one, so the engine coalesces to the newest request and drops results whose generation is old.
- [x] `compose.rs` builds the query text from `Query` using the table in "How the app works" (pure function, unit-tested). Windows paths cannot contain `"`, so quoting a path is always safe. Filters appended as Everything modifiers (`file:`, `folder:`, `ext:`, `size:`, hidden/system attributes).
- [x] Sort mapping to `Sort::{Name,Extension,DateModified,Size}{Ascending,Descending}`. Sorting and filtering are Everything's job, never Rust's.
- [x] **Paging:** page size 512, an LRU of about 16 pages around the viewport, prefetch of the neighbours only when idle. `total_len()` gives the scrollbar its size from the first reply.
- [x] **Folders first** (the list assumes it) by stitching a `folder:` segment and a `file:` segment into one view; the page cache maps a view index to (segment, offset). `Query::folders_first` is on in Explore mode and off in global search.
- [x] Regex through `SearchFlags::Regex`. Hidden files off by default (Ctrl+H).
- [x] **Unindexed folders:** when a browse returns no rows, one `read_dir(dir).next()` check tells an empty folder from one Everything does not index; the latter shows "Not indexed by Everything" with a hint to add it in Everything's index settings. This check never lists anything.
- [x] Open: Enter / double-click a file calls `opener::open`. Reveal in Explorer.
- [x] Status bar shows Everything state and version; F5 re-queries (or retries when gated).
- [x] Safe to be wrong: Everything's index can lag. Rows that no longer exist are a normal "not found", not an error.

**Verify against a running Everything (smoke test, `cfg(windows)`, needs Everything running):**
`parent:` vs `infolder:` (children only, no subfolders); `"dir\" text` recursion; negated attribute filters for hidden/system;
whether `max_results(0)` still returns the total (else fetch one row for counts); the `1.5a` instance name; `Attributes == u32::MAX` for unindexed attributes.

**Done when:** typing streams global results in a few milliseconds, scrolling a 1,000,000-row result is smooth, folders open instantly as `parent:` listings, stopping Everything shows the gate screen, and restarting it brings the app back without a restart.

## Phase 3: database and sidebar data (1 day)

- [x] SQLite in `%LOCALAPPDATA%\quickfiles\state.db` (WAL mode). One db thread owns the connection; the UI sends writes and reads `Arc` snapshots that are rebuilt after each write. Writes are batched and debounced.
- [x] Schema (migrations through `PRAGMA user_version`):
  - `visits(path_key PRIMARY KEY, path, last, count)`: recents and frecency;
  - `bookmarks(path_key PRIMARY KEY, path, label, pos)`;
  - `queries(text PRIMARY KEY, last, count)`: search history;
  - `folder_prefs(path_key PRIMARY KEY, sort_key, sort_dir, ...)`: per-folder sort;
  - `prefs(key PRIMARY KEY, value)`: window size, panel widths and open state, `show_hidden`, start state.
  `path_key` is the lower-cased path (Windows paths are case-insensitive); `path` keeps the original casing for display.
- [x] Recents: `core.visit()` on every folder entered; the sidebar shows the newest N.
- [x] Bookmarks: toolbar bookmark button toggles; the sidebar lists them in order.
- [x] Drives (`sys.rs`): `GetLogicalDrives` + `GetDiskFreeSpaceExW` + volume label and system-drive flag. They feed "This PC" and the sidebar, since Everything does not list drives. Refreshed on a timer and with F5 on the sys thread; a sleeping or network drive that hangs keeps its last value and never blocks anything else.
- [x] Places (Desktop, Documents, Downloads, ...) from `dirs`.
- [x] A corrupt or newer-schema database is renamed aside and recreated; the app still starts.

**Done when:** the sidebar shows your real machine and survives a restart, including panel sizes and sort order.

## Phase 4: icons, thumbnails, preview (2 days)

Three tiers behind one API (`Tier::Icon | Thumb | Preview`):

| Tier | Size | Source | Cache |
|---|---|---|---|
| Icon | 16-32 px | Shell icon per extension (per file only for `exe`, `lnk`, `ico`, `msi`, `scr`, `cpl`, `url`) | memory, keyed by extension |
| Thumb | 96 px | Windows Shell thumbnail (`IShellItemImageFactory::GetImage`, the same source as Explorer: images, video, PDF, Office); `image` crate as fallback | memory + disk |
| Preview | up to panel size | `image` decode with a size cap, or the first 64 KB of text | memory only |

- [x] Core returns RGBA `Pixels`; the UI uploads them and keeps an LRU of textures with a byte budget.
- [x] Disk cache in its own file (`thumbs.db`) so it can be deleted safely: key = hash(path, mtime, size, px), value = encoded image, total size capped (default 512 MB) with LRU eviction by `last_used`. `mtime` and `size` come with the entry from Everything, so validation costs nothing.
- [x] Worker pool of 2-4 threads at below-normal priority. **Interest model:** the UI calls `thumb()` for the visible rows every frame; a request not repeated within a short time is dropped, so fast scrolling never queues junk. Newest request first.
- [x] Each pool thread initialises COM (`CoInitializeEx`) and releases it on exit.
- [x] Decode limits: maximum pixel count and file size before decoding, so a 200 MP image cannot exhaust memory.
- [x] Preview text/code/json: first 64 KB, read on a worker, monospace view. Images fit to the panel. Everything else: metadata only.
- [x] Painted vector icons stay as the placeholder while a thumbnail loads.

**Done when:** selecting a PNG shows it in the preview, a folder of 2,000 photos scrolls without stutter and fills with thumbnails, and a second run shows them instantly from the disk cache.

## Phase 5: file operations (1-2 days)

- [x] New `Action`s: `Rename`, `NewFolder`, `Delete`, `DeletePermanent`, `Copy`, `Cut`, `Paste`, `CopyPath`.
- [x] Delete goes to the recycle bin via `trash`; Shift+Del is permanent with a confirm.
- [x] `jobs.rs`: copy/move/delete run as jobs on the same event pipe with progress (`bytes_done / bytes_total`, current item), cancellation through `Arc<AtomicBool>`, and a conflict policy (skip / replace / keep both) asked once per job.
- [x] Small progress/toast area (the old status bar space is free if you want it back).
- [x] Inline rename editor in the list row; context menu on right-click.
- [x] Hotkeys: Ctrl+C / Ctrl+X / Ctrl+V, F2, Del, Shift+Del, Ctrl+Shift+N.
- [x] After a job finishes, re-query so the list reflects it even if Everything is a moment behind.

**Done when:** copying a 5 GB folder shows progress, can be cancelled, and the UI never stutters.

## Phase 6: stay in sync and stay fast (1 day)

- [x] `notify` watcher on the current folder (Everything IPC has no push notifications); a change triggers a debounced (~200 ms) re-query. Selection and scroll position are kept across the refresh by matching the entry key.
- [x] Folder item counts (Folders / Files columns) are computed lazily, only for visible rows, as Everything count queries (`parent:"dir" folder:` and `file:`), cached by path. They never run as part of the main listing and never touch the file system.
- [x] Back/forward remember scroll position per state.
- [x] Per-frame work stays capped; `show_rows` virtualisation stays.

**Done when:** creating a file in Explorer makes it appear within a fraction of a second, with no flicker.

## Phase 7: polish (as needed)

- [x] Drag and drop in-app and from Explorer (to Explorer: not done, needs OLE).
- [x] Multiple tabs (`Vec<Nav>` slots in core; the view is re-opened on switch).
- [x] Settings file for hotkeys (`hotkeys.conf`).
- [x] Thumbnail grid view using the Thumb tier (Ctrl+G).

## Pitfalls to design around

- **No fallback means Everything's quirks are the app's quirks.** Not running, still indexing, unindexed folder and a lagging index are normal states with their own screen or message, never a blank list.
- **Everything specifics:** one query per client at a time and a new query cancels the old one; do not fire page fetches in parallel from one client. Attributes and dates can be missing for unindexed properties. Network and removable volumes only appear if the user adds them in Everything's index settings.
- **Non-UTF-8 and long paths:** keep `OsString`/`PathBuf`; use the `\\?\` prefix for paths over 260 characters in file operations.
- **Junctions and symlink loops:** never follow links in copy/size jobs; detect cycles.
- **Huge folders (100k+ entries):** paging plus the virtualised list covers it; sorting is Everything's.
- **Races:** a file can disappear between listing and use. Treat "not found" as a normal result, not a crash.
- **Stable identity:** all selection, focus and "keep position after refresh" logic uses `Entry::key()`, never an index. History entries store `(location, text)`, not row indices.
- **Case-insensitive paths:** keys, bookmarks and recents compare lower-cased paths.
- **Sleeping and network drives:** drive info and `read_dir` diagnostics run off the UI thread and never block other work.
- **COM threading:** shell thumbnails need COM initialised on the calling thread.
- **Cache growth:** the thumbnail disk cache and the texture LRU both have hard byte caps.
- **Destructive actions:** confirm permanent deletes; never overwrite silently.

## Testing

- Core has no egui, so it is unit-testable: the **`Nav` state machine** (every transition in "How the app works", including Back restoring location + text), the query composer (the table above, quoting, filters), the stitched page mapping (view index to segment and offset), page-cache eviction, breadcrumbs/`parent_of` (moved with their tests), database migrations and queries on in-memory SQLite, and the thumbnail cache key.
- A `cfg(windows)` smoke test that needs a running Everything (the verify list in Phase 2), skipped when it is not available.
- A scripted benchmark: time-to-first-row and page-fetch latency on a million-row Everything result, and a folder of 100k entries.
- Manual checklist for each phase in the "Done when" lines above.

## Suggested order and rough effort

| Phase | Effort | Why this order |
|---|---|---|
| 0 Groundwork | done | Unblocks everything, no visible change. |
| 1 Core skeleton, modes, UI adapted | 1 d | Proves the API, windowed list and `Nav` on dummy data before any real I/O. |
| 2 Everything engine | 2 d | The heart of the app: search, browse, gate screen. |
| 3 Database + sidebar | 1 d | Makes navigation real and persistent. |
| 4 Icons, thumbnails, preview | 2 d | Highest-value extras. |
| 5 Operations | 1-2 d | Turns the browser into a file manager. |
| 6 Live sync | 1 d | Makes it trustworthy. |
| 7 Polish | ongoing | Nice-to-haves. |

**Total to a usable daily driver (Phases 0-3): about 4 days.**

## Decisions

Settled:

- Everything is mandatory; no `read_dir` fallback, no browse-source preference, no scope toggle (the scope is always the current folder).
- The x button resets to global search and shows an empty "Search everything..." state; Esc clears only the text.
- SQLite instead of JSON for storage (frecency, query history, per-folder preferences).
- Folder counts come from lazy Everything count queries.

Open (defaults in brackets):

1. Both search boxes (title bar and status bar) still edit the same string today. [Title bar is the query; the status-bar box becomes a type filter (all / files / folders / extension) in a later phase.]
2. Start state. [Search mode with an empty list; a pref can restore the last folder.]
3. "Start Everything" button on the gate screen. [Included when `Everything.exe` can be found; otherwise only the download link and Retry.]
