# PLAN-PERF: thumbnails and image preview

Goal: scrolling a folder of thousands of images and arrowing through them stays smooth, and the
preview panel never hitches. This file is the checklist for that work. The main plan is
`PLAN.md`; this one only covers the image pipeline (`quickfiles-core/src/images/*`,
`ui/tex.rs`, `ui/preview.rs`, `ui/grid.rs`, `ui/file_list.rs`).

> Status: phases 0-5 are all written. **None of it has been compiled against the real
> dependencies or measured** (the sandbox had no crates.io access). The pure logic (queue, byte LRU,
> debounce, scroll gate, resize, EXIF parser, cache key / prune / version plans, premultiply,
> preview sizing) was compiled and unit-tested under a stub harness (54 of 55 tests in `quickfiles-core` pass; the one
> failure is a Windows-path test in `fmt.rs` that cannot pass on Linux and is unrelated to this work),
> the whole `images/` module type-checks against stub versions of `image`, `jpeg-decoder` and
> `rusqlite`, and the cache SQL was run against SQLite. The stub check caught one real error that is
> now fixed (`stats.rs` matched `ImageKey::Preview` with the wrong number of fields). Not compiler-checked: everything that touches `jpeg-decoder`, `image`,
> `rusqlite`, egui and the Windows shell. The analysis comes from reading the code, not from a
> profiler: take the baseline (phase 0) before trusting any of the "fixed" claims below.

## Budgets (what "smooth" means)

- [ ] No frame over 16 ms while scrolling the list or the grid (target: p99 under 8 ms of our own work).
- [ ] Arrowing through images: the preview appears in under 100 ms when cached, and never freezes the UI.
- [ ] A burst of 200 thumbnails arriving causes at most one repaint per ~16 ms.
- [ ] Memory: image caches bounded by bytes, not by item count (target under 256 MB in memory).
- [ ] A folder opened for the second time shows every thumbnail within one or two frames (disk cache).

## Where it hurts today (from reading the code)

| # | Cause | Where |
|---|-------|-------|
| 1 | `wake()` after every decoded image = one full UI repaint per thumbnail | `images/mod.rs` `worker` |
| 2 | Preview decoded up to 1024 px, then converted and uploaded on the UI thread | `PREVIEW_PX`, `ui/tex.rs` `get` |
| 3 | Each selection queues a decode; one already running cannot be cancelled | `images/mod.rs` |
| 4 | Whole image decoded to make a 96 px thumbnail | `images/decode.rs` `load` |
| 5 | Per tile per frame: `PathBuf` allocation, full-path hash, mutex lock | `ImageKey::thumb`, `Images::want` |
| 6 | Disk cache hit re-opens the file for writing just to touch its mtime | `images/cache.rs` `get` |
| 7 | One PNG file per thumbnail (slow encode, many small files, antivirus) | `images/cache.rs` |
| 8 | Memory cache counts items (700), a 1024 px preview costs as much as a thumbnail | `MAX_READY` |
| 9 | Icons, thumbnails and previews share 2-4 threads | `Core::new` |
| 10 | GPU uploads are unbounded per frame | `ui/tex.rs` |

## Phase 0: measure first

- [x] Debug overlay (toggle with F12): frame time (last / p99), image queue length, in-flight jobs,
      uploads this frame, texture count, memory cache MB, disk hit / miss counts.
- [x] Timing counters in `Images` (`images/stats.rs`): decode ms, disk read ms, disk write ms, per job kind (icon / thumb / preview).
- [x] Test folders: `tools/make-test-folders.ps1` creates 1,000 small JPEGs, 200 x 24 MP JPEGs, a mixed folder (PNG, GIF, BMP, JPEG, text) and 20,000 files. (No WebP / PDF / video: add some by hand.)
- [x] `Ctrl+F12` copies a text report; `Shift+F12` resets the counters before each test.
- [ ] Record a baseline table (scroll, first open, second open, arrow through 50 images) in this file. **Your turn:** run the script, then follow "How to take a baseline" below.

### How to take a baseline

1. `powershell -ExecutionPolicy Bypass -File tools\make-test-folders.ps1` (let Everything index the new folders).
2. Release build: `cargo run --release -p quickfiles`. Debug builds are far slower and not representative.
3. Open a test folder, press `Shift+F12` (reset), then `F12` (show the overlay).
4. Do one scenario: scroll top to bottom at normal speed; or select the first image and hold Down for 50 images; or open the folder a second time.
5. `Ctrl+F12` and paste the report into the results table below. Compare **ui code p99 / max**, **uploads/frame max**, **worker repaint requests** and the **thumb / preview avg ms**.
6. Delete the `thumbs` folder in `%LOCALAPPDATA%\quickfiles` between "first open" runs, so the disk cache is cold.

What the numbers mean: *ui code ms* is time inside our own drawing code (not the GPU); a frame
needs to stay well under 16 ms. *worker repaint requests* counts how often decoding asked for a
redraw (phase 1 should make this drop). *upload ms* is the cost of converting and uploading
textures on the UI thread (phase 2 should make this drop).

## Phase 1: quick wins (small, low risk)

- [x] **Coalesce repaints:** workers mark "dirty" and a small waker thread sends at most one repaint per 16 ms; the first one after a quiet period goes out immediately. The overlay shows requested vs sent. (#1)
- [x] **Upload budget:** at most 12 texture uploads per frame (`MAX_UPLOADS_PER_FRAME` in `ui/tex.rs`); the rest wait for the next frame and a repaint is requested. The large preview is exempt. The overlay counts deferred uploads. (#10)
- [x] **Cheap cache key:** textures are looked up by `QuickKey` (entry key, modified, size): a thumbnail that already has a texture costs one hash lookup per frame, with no `PathBuf`, no lock. The full `ImageKey` is built only on a miss. The disk-cache hash no longer allocates a lowercase path. (#5)
- [x] **No write-open on cache hits:** one open per hit; the timestamp is refreshed only when it is over a day old. (#6)
- [x] **Byte budget for the memory cache:** 128 MB (`MAX_READY_BYTES`), previews count their real size; the 700-item cap stays as a second limit. (#8)

Done when: the debug overlay shows repaints sent far below repaints requested during a thumbnail
burst, `uploads/frame max` stays at or below 12, and scrolling shows no new spikes. **Re-run the
baseline scenarios and fill the "After phase 1" column.**

## Phase 2: preview panel

- [x] **Decode at panel size:** ask for about 2x the panel's physical pixels instead of a fixed 1024. (#2)
- [x] **Premultiply off the UI thread:** workers return pixels that are already premultiplied, so the UI thread only uploads. (#2)
- [x] **Debounce:** request the preview only after the selection has been steady for 100-150 ms. (#3)
- [x] **Progressive display:** show the cached thumbnail scaled up immediately, swap to the full preview when ready.
- [x] **Prefetch neighbours:** low-priority preview of the previous and next image while idle.
- [x] **Dedicated preview thread** (three lanes: icons 1 thread, thumbnails 2-4, previews 1; each with its own queue) (one worker that only takes preview jobs) so thumbnails never wait behind it and vice versa. (#9)
- [x] **Cancellation flag** checked between decode steps where the decoder allows it, so a stale preview stops early. (#3)

Done when: holding the Down arrow across 50 large images never drops a frame, and the preview
catches up within 150 ms after you stop.

## Phase 3: faster decoding

- [x] **JPEG scaled decode** (1/2, 1/4, 1/8 via DCT scaling) with the `jpeg-decoder` crate (`default-features = false`, so no rayon), then a small final resize. (#4) CMYK / 16-bit grey JPEGs fall back to the `image` crate.
- [~] **SIMD resize** (`fast_image_resize`) instead of `DynamicImage::thumbnail`. *Deviation:* no new crate; JPEGs use an exact area-average resize (`decode::area_resize`, two separable passes) on the already DCT-scaled image, which is small (about 750 x 500 for a 24 MP photo at 96 px). Other formats still use `DynamicImage::thumbnail`. Swap in `fast_image_resize` if the baseline shows it matters.
- [x] **EXIF embedded thumbnail** for JPEGs when it is large enough.
- [x] **Shell thumbnail first** for types the shell handles well (RAW, HEIC, PDF, video), using Explorer's own warm cache.
- [x] Keep the 96 MB in-process size limit; larger files go to the shell path only.

Done when: a 24 MP JPEG thumbnail takes well under 30 ms and well under 20 MB of working memory.

## Phase 4: disk cache

- [x] **Single store:** `thumbs.db` (SQLite, WAL, one reader connection plus a writer thread) instead of one PNG per thumbnail. (#7) The old `thumbs/` folder is deleted on first start.
- [x] **Cheap encoding:** raw RGBA blobs (about 37 KB per 96 px thumbnail), no encoder on the hot path. No lz4 (no new crate); at 512 MB that is roughly 14,000 thumbnails.
- [x] **Write-behind queue:** encode and write on a background thread, never inside the decode job.
- [x] **Real LRU** via a `last_used` column; prune in the background; keep the 512 MB budget configurable.
- [x] **Versioned keys:** a cache version number, so a format change cleans up old entries.
- [x] Menu item "Clear thumbnail cache" (kebab menu).

Done when: reopening a 5,000-image folder shows all thumbnails from cache without visible loading.

## Phase 5: grid and list scrolling

- [x] **Fast-scroll gating:** while scroll velocity is high, draw icons only and queue nothing; load when it slows.
- [x] **Prefetch in the scroll direction:** one or two rows ahead, with lower priority than what is visible.
- [x] **Cap decode queue depth** (drop the oldest wanted keys beyond a limit).
- [x] **Smaller work per frame in the grid:** skip hashing and lookups for tiles that already have a texture handle this frame.
- [ ] Optional (not done): pack small thumbnails into a texture atlas to cut the number of GPU textures.

Done when: dragging the scrollbar through 20,000 files stays smooth and thumbnails fill in right after you stop.

## Verification

- [x] Unit tests: byte-budget eviction, debounce timer, cache key stability, cache version bump (also queue, scroll gate, resize, EXIF, prune plan, premultiply, preview sizing).
- [x] Benchmark: `cargo run --release -p quickfiles-core --example bench_images -- <folder>` (decode, preview, cache write and read). `--example bench_query` does the listing side.
- [ ] Before / after table filled in (Phase 0 vs end of each phase). **Your turn**, needs a Windows run.
- [ ] `cargo fmt --all -- --check` and `cargo clippy --workspace --all-targets` clean. (All touched files pass `rustfmt --check`; clippy was not run: it needs the real dependencies.)

## How phases 2-5 work (for review)

- **Lanes** (`images/mod.rs`): `Lane::{Icon, Thumb, Preview}`, each with its own `Queue` (`images/queue.rs`),
  condvar and threads. Queue depth caps are 256 / 256 / 8; past the cap prefetch work goes first, then the
  oldest request. `want()` is visible priority, `prefetch()` only runs when no visible request waits.
- **Cancellation:** a running job asks "is this still wanted?" between read, decode and resize. It is
  wanted while the UI re-asks for it every painted frame (`in_flight` carries the last frame). A cancelled
  job is not remembered as failed; it is simply asked for again if it becomes visible again.
- **Preview size:** `preview_px(panel_longest_side_px)` = 2 x the panel in physical pixels, in steps of
  256, clamped to 256..2048, so dragging the splitter does not re-decode at every width. It is part of
  `ImageKey::Preview` and `QuickKey::Preview`.
- **Debounce** (`images/debounce.rs`, 120 ms, `state.preview_debounce`): while the selection moves, the
  preview shows the cached thumbnail scaled up and requests nothing.
- **Scroll gate** (`images/scroll.rs`): above 2,400 px/s the textures stop asking for images (icons only),
  below 900 px/s they resume; `Tex::set_gate`. The overlay shows "scroll gate CLOSED".
- **Memory:** decoded pixels 128 MB (`ByteLru`, by bytes and by count), GPU textures 192 MB; both evict
  oldest first.
- **Disk cache:** `images/cache.rs`; `PRAGMA user_version` is the format version (`CACHE_VERSION`), the
  budget is the `thumb_cache_mb` pref (default 512), pruning deletes the oldest rows down to 75 %.

## Baseline and results

| Scenario | Baseline | After phase 1 | After phase 2 | After phase 3-5 |
|----------|----------|---------------|---------------|-----------------|
| Scroll 1,000 JPEGs (p99 frame ms) | | | | |
| Open 24 MP JPEG preview (ms to shown) | | | | |
| Arrow through 50 images (dropped frames) | | | | |
| Reopen folder (ms to all thumbnails) | | | | |
| Memory after 5 min (MB) | | | | |

## Order of work (all written; next is measuring)

1. First `cargo check --workspace` and fix the signature slips this sandbox could not catch. Likeliest
   spots: `jpeg_decoder::Decoder::scale` / `info`, `rusqlite` `prepare_cached` / `OpenFlags`, and
   `ColorImage::from_rgba_premultiplied`.
2. Take the baseline (phase 0) and fill the "After phase 2 / After phase 3-5" columns.
3. Tune the constants if the numbers ask for it: `FAST` / `SLOW` in `scroll.rs`, `PREVIEW_DEBOUNCE_MS`,
   `QUEUE_CAP`, `MAX_READY_BYTES`, `MAX_TEXTURE_BYTES`.
