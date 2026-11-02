# PLAN: from prototype to real file manager

Goal: replace the dummy data with the real file system **without touching the UI design**.
The architecture already has the hard parts: a worker thread, request ids that drop stale results,
a virtualised list, and a single `Action` pipeline that buttons and hotkeys share.

## Principles

1. **The UI thread never touches the file system.** Every FS operation is a `Command` to the worker.
2. **Stale results are dropped** by request id (already implemented). Reuse it for every new job type.
3. **Stream, don't block.** Big folders show their first rows immediately.
4. **Paths are `PathBuf`, not `String`.** Convert to `String` only for display.
5. **One change per PR**, each runnable on its own. Dummy data stays as a fallback until Phase 1 lands.

## Where things change

| File | Change |
|---|---|
| `model.rs` | `FileEntry` gains `path: PathBuf`, `hidden`, `is_symlink`; `modified` becomes a sortable value plus display string. |
| `data.rs` | Replace `build_listing` with a real scanner. Keep the dummy generator behind a `--dummy` flag for UI work. |
| `worker.rs` | More `Command`/`Event` variants; chunked results; cancellation tokens. |
| `state.rs` | ~~`path: Vec<String>` becomes `PathBuf`~~ (done in Phase 0). |
| `app.rs` | New `Action` handlers; watcher wiring; job progress. |
| `hotkeys.rs` | New bindings for file operations. |
| `ui/toolbar.rs` | Breadcrumb built from `PathBuf::components()`; editable path box (Ctrl+L style) optional. |
| `ui/sidebar.rs` | Real drives, places, bookmarks, recents. |
| `ui/preview.rs` | Real text/image preview. |

## Phase 0: groundwork (DONE)

- [x] `path` is a `PathBuf` across `state.rs`, `app.rs`, `toolbar.rs`, `sidebar.rs`, `worker.rs`, `data.rs`. The empty path is the virtual root "This PC".
- [x] Breadcrumbs built from path components (`model::breadcrumbs`); a Windows prefix + root (`C:` + `\`) is one segment. Helpers `display_name` and `parent_of` live in `model.rs` with unit tests.
- [x] Up goes from a top-level folder to "This PC"; the dummy "This PC" lists the drives.
- [x] Dependencies and the `--dummy` flag are **deferred to Phase 1**: nothing uses them yet, and the dummy generator is still the only data source, so a flag now would do nothing. Add each crate in the phase that first needs it.

**Result:** app behaves as before on dummy data, now on `PathBuf` paths.

## Phase 1: real listing (1 day)

- [ ] `data::scan(path, cancel)` using `std::fs::read_dir`.
  - On Windows `DirEntry::metadata()` is cheap (comes from the directory scan), so size and date need no extra syscalls.
- [ ] Per-entry error handling (permission denied, broken symlinks, junctions, locked files). One bad entry must never fail the listing.
- [ ] Kind from extension, `type_label` from an extension table (cache per extension, not per entry).
- [ ] Hidden/system files: read attributes; hide by default, toggle with Ctrl+H.
- [ ] **Streaming:** worker emits `Event::Chunk { id, entries }` every ~2,000 entries, then `Event::Done { id }`. `state.rs` appends and re-sorts incrementally.
- [ ] Natural sort (`natord`) for the Name column. Folders still first.
- [ ] Virtual root "This PC" lists drives (see Phase 2).

**Done when:** you can browse `C:\Windows\System32` (≈5k entries) and the first rows appear instantly while the rest stream in.

## Phase 2: sidebar data (0.5 day)

- [ ] Drives: label, used/total, system flag from `sysinfo` (or `GetLogicalDrives` + `GetDiskFreeSpaceEx`). Refresh on a timer and on device-change.
- [ ] Places: Desktop, Documents, Downloads, etc. from `dirs`.
- [ ] Bookmarks and recents: JSON in the config dir (`dirs::config_dir()/quickfiles/state.json`). Load at start, save on change (debounced).
- [ ] Bookmark toggle on the toolbar bookmark button.

**Done when:** the sidebar shows your real machine and survives a restart.

## Phase 3: open + file operations (1-2 days)

- [ ] Open: Enter / double-click a file calls `opener::open`; folders navigate (already works).
- [ ] New `Action`s: `Rename`, `NewFolder`, `Delete`, `DeletePermanent`, `Copy`, `Cut`, `Paste`, `CopyPath`.
- [ ] Delete goes to the recycle bin via `trash`; Shift+Del is permanent with a confirm.
- [ ] Copy/move/delete run as **worker jobs** with:
  - progress events (`bytes_done / bytes_total`, current item),
  - cancellation through `Arc<AtomicBool>`,
  - conflict policy (skip / replace / keep both) asked once per job.
- [ ] Small progress/toast area (the old status bar space is free if you want it back).
- [ ] Inline rename editor in the list row; context menu on right-click.
- [ ] Hotkeys: Ctrl+C / Ctrl+X / Ctrl+V, F2, Del, Shift+Del, Ctrl+Shift+N.

**Done when:** copying a 5 GB folder shows progress, can be cancelled, and the UI never stutters.

## Phase 4: stay in sync and stay fast (1 day)

- [ ] `notify` watcher on the current folder; on change send a debounced (~200 ms) refresh. Keep selection and scroll position across refresh by matching on path.
- [ ] Folder item counts and sizes computed **lazily on the worker**, only for visible rows, cached by path + mtime. Never part of the main scan.
- [ ] Cap work per frame; keep `show_rows` virtualisation (already in place).
- [ ] Back/forward remember scroll position per path.

**Done when:** creating a file in Explorer makes it appear within a fraction of a second, with no flicker.

## Phase 5: search and preview (2-3 days)

- [ ] Folder filter (search box) already works on the loaded listing; keep it for the current folder.
- [ ] Recursive search mode: `ignore` or `jwalk` (parallel), streamed as `Event::SearchChunk`, cancelled whenever the query changes.
- [ ] Preview panel:
  - text/code/json: first 64 KB, read on the worker, monospace view;
  - images: decode with `image` on the worker, upload an egui texture, fit to panel;
  - everything else: metadata only.
- [ ] Optional later: whole-disk instant search by reading the NTFS MFT / USN journal (separate project; skip at first).

**Done when:** typing in the search box with "search subfolders" on streams matches live, and selecting a PNG shows it in the preview.

## Phase 6: polish (as needed)

- [ ] Drag and drop (in-app and from/to Explorer).
- [ ] Multiple tabs again (state is already isolated in `State`; would become `Vec<State>`).
- [ ] Persist window size, panel widths, sort order, column visibility.
- [ ] Real file icons (shell icons per extension, cached as textures).
- [ ] Settings file for hotkeys (the table in `hotkeys.rs` is already data-driven).

## Pitfalls to design around

- **Non-UTF-8 and long paths:** keep `OsString`/`PathBuf`; use the `\\?\` prefix for paths over 260 characters.
- **Junctions and symlink loops:** never follow links when recursing; detect cycles in search/size jobs.
- **Huge folders (100k+ entries):** streaming plus the virtualised list covers it; re-sorting on every chunk must be incremental or throttled (e.g. at most every 100 ms).
- **Network and sleeping drives:** every worker call can be slow, so show a loading state and allow cancel. Never assume a drive is ready.
- **Races:** a file can disappear between listing and use. Treat "not found" as a normal result, not a crash.
- **Destructive actions:** confirm permanent deletes; never overwrite silently.

## Testing

- Unit tests for the scanner against a temp directory tree (permissions, symlinks, unicode names, 50k files).
- A scripted benchmark: time-to-first-row and total scan time on `C:\Windows\System32` and a 100k-file folder.
- Manual checklist for each phase in the "Done when" lines above.

## Suggested order and rough effort

| Phase | Effort | Why this order |
|---|---|---|
| 0 Groundwork | done | Unblocks everything, no visible change. |
| 1 Real listing | 1 d | Biggest jump in usefulness. |
| 2 Sidebar data | 0.5 d | Makes navigation real. |
| 3 Operations | 1-2 d | Turns the browser into a file manager. |
| 4 Live sync | 1 d | Makes it trustworthy. |
| 5 Search/preview | 2-3 d | Highest-value extras. |
| 6 Polish | ongoing | Nice-to-haves. |

**Total to a usable daily driver (Phases 0-4): about 4-5 days.**
