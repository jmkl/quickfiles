//! Icons, thumbnails and previews. Decoded on a small thread pool, cached in memory (bounded)
//! and, for thumbnails, on disk. No egui here: the UI turns [`Pixels`] into textures.
//!
//! Interest model: the UI calls [`Images::want`] for everything it draws; the call returns the
//! pixels if ready and otherwise records interest. Workers only run jobs that were wanted in
//! the current or previous frame (call [`Images::begin_frame`] once per painted frame), so
//! scrolling past thousands of rows never queues thousands of decodes.
//!
//! Three lanes with their own threads and queues (icons, thumbnails, previews), so a slow
//! preview never delays thumbnails and the other way round. Speculative work
//! ([`Images::prefetch`]) only runs when nothing on screen is waiting. A running job is
//! abandoned between its decode steps once the UI stops asking for it.

pub mod cache;
pub mod debounce;
pub mod decode;
pub mod lru;
pub mod queue;
pub mod scroll;
pub mod shell;
pub mod stats;

use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::{Arc, Condvar, Mutex},
    thread,
};

pub use debounce::Debounce;
pub use queue::Prio;
pub use scroll::ScrollGate;
pub use stats::{ImageStats, JobKind, KindStats};

use crate::{
    engine::Wake,
    fmt::wants_thumbnail,
    model::{Category, Entry, Kind},
};
use decode::{Decoded, Fail, Route};

pub const THUMB_PX: i32 = 96;
/// Preview sizes are rounded up to a multiple of this, so resizing the panel does not decode
/// the same picture again for every pixel of width.
const PREVIEW_STEP: u32 = 256;
const PREVIEW_MIN: u32 = 256;
pub const PREVIEW_MAX: u32 = 2048;
const MAX_READY: usize = 700;
/// Decoded pixels kept in memory (a 1024 px preview is ~4 MB, a thumbnail ~37 KB).
const MAX_READY_BYTES: usize = 128 * 1024 * 1024;
/// Workers ask the UI to repaint at most this often (a burst of results costs one frame).
const WAKE_INTERVAL: std::time::Duration = std::time::Duration::from_millis(16);
/// Most requests one lane keeps waiting; older ones are dropped (they are scrolled past).
const QUEUE_CAP: [usize; 3] = [256, 256, 8];

/// The preview size to ask for: about 2x the panel's longest side in physical pixels (so it is
/// sharp on high-DPI screens and when zoomed), in steps of 256, within 256..=2048.
pub fn preview_px(panel_longest_side_px: f32) -> u32 {
    let want = (panel_longest_side_px.max(0.0) * 2.0).ceil() as u32;
    (want.div_ceil(PREVIEW_STEP) * PREVIEW_STEP).clamp(PREVIEW_MIN, PREVIEW_MAX)
}

/// RGBA pixels. Straight alpha when freshly decoded; the image service hands out premultiplied
/// pixels (see [`Pixels::premultiplied`]) so the UI thread only has to upload them.
#[derive(Debug)]
pub struct Pixels {
    pub w: u32,
    pub h: u32,
    pub rgba: Vec<u8>,
    pub premultiplied: bool,
}

impl Pixels {
    /// Straight-alpha pixels.
    pub fn new(w: u32, h: u32, rgba: Vec<u8>) -> Self {
        Self {
            w,
            h,
            rgba,
            premultiplied: false,
        }
    }

    /// Converts to premultiplied alpha in place (a no-op when already done, or fully opaque).
    pub fn into_premultiplied(mut self) -> Self {
        if !self.premultiplied {
            for p in self.rgba.chunks_exact_mut(4) {
                let a = p[3] as u32;
                if a != 255 {
                    for v in &mut p[..3] {
                        *v = ((*v as u32 * a + 127) / 255) as u8;
                    }
                }
            }
            self.premultiplied = true;
        }
        self
    }
}

#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct Stamp {
    pub path: PathBuf,
    pub modified: i64,
    pub size: u64,
}

#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub enum ImageKey {
    /// Shared per file type (resolved from the extension, no disk access).
    Icon {
        ext: Box<str>,
        folder: bool,
    },
    /// Per-file icon: executables, shortcuts, drives.
    PathIcon(PathBuf),
    Thumb(Stamp),
    /// A picture of at most `px` on the longest side.
    Preview(Stamp, u32),
}

/// Identity of an entry's image that needs no allocation, for the per-frame texture lookup
/// (building an [`ImageKey`] clones the path). Changes when the file's mtime or size change.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum QuickKey {
    Thumb(u64, i64, u64),
    Preview(u64, i64, u64, u32),
    PathIcon(u64),
}

const OWN_ICON: [&str; 6] = ["exe", "lnk", "ico", "msi", "url", "scr"];

impl ImageKey {
    pub fn icon(e: &Entry) -> Self {
        match e.kind {
            Kind::Folder => Self::Icon {
                ext: "".into(),
                folder: true,
            },
            Kind::Drive => Self::PathIcon(e.path()),
            Kind::File => {
                let ext = e.ext().unwrap_or_default();
                if OWN_ICON.contains(&ext.as_str()) {
                    Self::PathIcon(e.path())
                } else {
                    Self::Icon {
                        ext: ext.into(),
                        folder: false,
                    }
                }
            }
        }
    }

    /// Allocation-free twin of [`ImageKey::thumb`].
    pub fn quick_thumb(e: &Entry) -> Option<QuickKey> {
        (e.kind == Kind::File && wants_thumbnail(e.category))
            .then(|| QuickKey::Thumb(e.key, e.modified, e.size))
    }

    /// Allocation-free twin of [`ImageKey::preview`].
    pub fn quick_preview(e: &Entry, px: u32) -> Option<QuickKey> {
        if e.kind != Kind::File {
            return None;
        }
        if e.category == Category::Image {
            Some(QuickKey::Preview(e.key, e.modified, e.size, px))
        } else {
            Self::quick_thumb(e)
        }
    }

    /// Allocation-free: does this entry get its own shell icon (executables, shortcuts, drives)?
    pub fn quick_path_icon(e: &Entry) -> Option<QuickKey> {
        match e.kind {
            Kind::Folder => None,
            Kind::Drive => Some(QuickKey::PathIcon(e.key)),
            Kind::File => {
                let (stem, ext) = e.name.rsplit_once('.')?;
                let own = !stem.is_empty() && OWN_ICON.iter().any(|o| ext.eq_ignore_ascii_case(o));
                own.then_some(QuickKey::PathIcon(e.key))
            }
        }
    }

    fn stamp(e: &Entry) -> Stamp {
        Stamp {
            path: e.path(),
            modified: e.modified,
            size: e.size,
        }
    }

    /// Thumbnail key, for entries whose type can have one.
    pub fn thumb(e: &Entry) -> Option<Self> {
        (e.kind == Kind::File && wants_thumbnail(e.category)).then(|| Self::Thumb(Self::stamp(e)))
    }

    /// Preview key (images only; other types reuse their thumbnail).
    pub fn preview(e: &Entry, px: u32) -> Option<Self> {
        if e.kind != Kind::File {
            return None;
        }
        if e.category == Category::Image {
            Some(Self::Preview(Self::stamp(e), px))
        } else {
            Self::thumb(e)
        }
    }
}

/// The work lanes; each has its own queue and threads.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Lane {
    Icon = 0,
    Thumb = 1,
    Preview = 2,
}

impl Lane {
    fn of(key: &ImageKey) -> Self {
        match key {
            ImageKey::Icon { .. } | ImageKey::PathIcon(_) => Self::Icon,
            ImageKey::Thumb(_) => Self::Thumb,
            ImageKey::Preview(..) => Self::Preview,
        }
    }
}

struct State {
    frame: u64,
    lanes: [queue::Queue<ImageKey>; 3],
    /// Running jobs, with the last frame the UI asked for them in (drives cancellation).
    in_flight: HashMap<ImageKey, u64>,
    ready: lru::ByteLru<ImageKey>,
    stop: bool,
}

impl State {
    fn new() -> Self {
        Self {
            frame: 0,
            lanes: QUEUE_CAP.map(queue::Queue::new),
            in_flight: HashMap::new(),
            ready: lru::ByteLru::new(MAX_READY_BYTES, MAX_READY),
            stop: false,
        }
    }
}

/// Coalesces repaint requests from the workers into at most one per [`WAKE_INTERVAL`].
#[derive(Default)]
struct Waker {
    state: Mutex<WakeState>,
    cv: Condvar,
}

#[derive(Default)]
struct WakeState {
    dirty: bool,
    stop: bool,
}

struct Shared {
    state: Mutex<State>,
    /// One condvar per lane, so a wake-up always reaches a worker of the right lane.
    cvs: [Condvar; 3],
    wake: Wake,
    disk: cache::DiskCache,
    stats: Arc<stats::Stats>,
    waker: Waker,
}

#[derive(Clone)]
pub struct Images {
    shared: Arc<Shared>,
}

impl Images {
    /// `thumb_threads` workers decode thumbnails; icons and previews get one thread each.
    pub fn new(data_dir: &Path, wake: Wake, thumb_threads: usize, disk_budget_mb: u64) -> Self {
        let stats = Arc::new(stats::Stats::default());
        let shared = Arc::new(Shared {
            state: Mutex::new(State::new()),
            cvs: [Condvar::new(), Condvar::new(), Condvar::new()],
            wake,
            disk: cache::DiskCache::open(data_dir, disk_budget_mb, stats.clone()),
            stats,
            waker: Waker::default(),
        });
        {
            let s = shared.clone();
            thread::Builder::new()
                .name("qf-img-wake".to_owned())
                .spawn(move || waker_loop(&s))
                .expect("failed to spawn the repaint thread");
        }
        for (lane, n) in [
            (Lane::Icon, 1),
            (Lane::Thumb, thumb_threads.max(1)),
            (Lane::Preview, 1),
        ] {
            for i in 0..n {
                let s = shared.clone();
                thread::Builder::new()
                    .name(format!("qf-img-{lane:?}-{i}").to_lowercase())
                    .spawn(move || worker(s, lane))
                    .expect("failed to spawn an image thread");
            }
        }
        Self { shared }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, State> {
        self.shared.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Call once per painted frame, before any [`want`](Self::want).
    pub fn begin_frame(&self) {
        self.lock().frame += 1;
    }

    /// The pixels if decoded; otherwise records interest and returns `None`. A key that failed
    /// (no thumbnail handler, unreadable file) stays `None` and is not retried.
    pub fn want(&self, key: &ImageKey) -> Option<Arc<Pixels>> {
        self.ask(key, Prio::Visible)
    }

    /// Like [`want`](Self::want) for speculative work (the next image, rows ahead of the scroll):
    /// it runs only when nothing visible is waiting and is dropped first when a queue is full.
    pub fn prefetch(&self, key: &ImageKey) -> Option<Arc<Pixels>> {
        self.ask(key, Prio::Prefetch)
    }

    fn ask(&self, key: &ImageKey, prio: Prio) -> Option<Arc<Pixels>> {
        let mut g = self.lock();
        if let Some(r) = g.ready.get(key) {
            return r.clone();
        }
        let frame = g.frame;
        if let Some(f) = g.in_flight.get_mut(key) {
            *f = frame; // still wanted: keeps the running job alive
            return None;
        }
        let lane = Lane::of(key);
        if g.lanes[lane as usize].want(key, prio, frame) {
            drop(g);
            self.shared.cvs[lane as usize].notify_one();
        }
        None
    }

    /// Counters and queue state for the F12 overlay.
    pub fn stats(&self) -> ImageStats {
        let mut out = self.shared.stats.snapshot();
        let g = self.lock();
        out.queued = g.lanes.iter().map(|q| q.len()).sum();
        out.dropped = g.lanes.iter().map(|q| q.dropped()).sum();
        out.in_flight = g.in_flight.len();
        out.cached = g.ready.len();
        out.cached_mb = g.ready.bytes() as f64 / (1024.0 * 1024.0);
        drop(g);
        out.disk_mb = self.shared.disk.bytes() as f64 / (1024.0 * 1024.0);
        out.disk_pending = self.shared.disk.pending();
        out
    }

    pub fn reset_stats(&self) {
        self.shared.stats.reset();
    }

    /// Forget everything for a path (the file changed).
    pub fn invalidate(&self, path: &Path) {
        self.lock().ready.remove_where(|k| match k {
            ImageKey::Thumb(s) | ImageKey::Preview(s, _) => s.path == path,
            ImageKey::PathIcon(p) => p == path,
            ImageKey::Icon { .. } => false,
        });
    }

    /// "Clear thumbnail cache": drops the in-memory images and every thumbnail on disk.
    pub fn clear_cache(&self) {
        let mut g = self.lock();
        g.ready.clear();
        for q in &mut g.lanes {
            q.clear();
        }
        drop(g);
        self.shared.disk.clear();
    }

    /// Size limit of the on-disk thumbnail cache.
    pub fn set_disk_budget_mb(&self, mb: u64) {
        self.shared.disk.set_budget_mb(mb);
    }

    pub fn stop(&self) {
        self.lock().stop = true;
        for cv in &self.shared.cvs {
            cv.notify_all();
        }
        self.shared.disk.stop();
        self.shared
            .waker
            .state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .stop = true;
        self.shared.waker.cv.notify_all();
    }
}

fn worker(s: Arc<Shared>, lane: Lane) {
    let _com = shell::ComGuard::new();
    loop {
        let key = {
            let mut g = s.state.lock().unwrap_or_else(|e| e.into_inner());
            loop {
                if g.stop {
                    return;
                }
                let frame = g.frame;
                if let Some(k) = g.lanes[lane as usize].pop(frame) {
                    g.in_flight.insert(k.clone(), frame);
                    break k;
                }
                g = s.cvs[lane as usize]
                    .wait(g)
                    .unwrap_or_else(|e| e.into_inner());
            }
        };
        let started = std::time::Instant::now();
        // Polled between decode steps: stop once the UI has not asked for this for a frame.
        let cancel = || {
            let g = s.state.lock().unwrap_or_else(|e| e.into_inner());
            g.stop
                || g.in_flight
                    .get(&key)
                    .is_none_or(|f| f.saturating_add(1) < g.frame)
        };
        let result = produce(&s, &key, &cancel);
        let cancelled = matches!(result, Err(Fail::Cancelled));
        let ok = result.is_ok();
        if cancelled {
            s.stats.cancelled();
        } else {
            s.stats.job(JobKind::of(&key), started.elapsed(), ok);
        }
        {
            let mut g = s.state.lock().unwrap_or_else(|e| e.into_inner());
            g.in_flight.remove(&key);
            match result {
                // Premultiplied here, off the UI thread.
                Ok(px) => g.ready.insert(key, Some(Arc::new(px.into_premultiplied()))),
                Err(Fail::Failed) => g.ready.insert(key, None),
                // Not a property of the file: ask again if it is wanted again.
                Err(Fail::Cancelled) => {}
            }
        }
        if cancelled {
            continue;
        }
        s.stats.wake_request();
        let mut w = s.waker.state.lock().unwrap_or_else(|e| e.into_inner());
        w.dirty = true;
        drop(w);
        s.waker.cv.notify_one();
    }
}

/// Sends the coalesced repaint requests: the first one after a quiet period goes out
/// immediately, further ones at most every [`WAKE_INTERVAL`].
fn waker_loop(s: &Shared) {
    let mut last: Option<std::time::Instant> = None;
    loop {
        {
            let mut g = s.waker.state.lock().unwrap_or_else(|e| e.into_inner());
            while !g.dirty && !g.stop {
                g = s.waker.cv.wait(g).unwrap_or_else(|e| e.into_inner());
            }
            if g.stop {
                return;
            }
        }
        if let Some(l) = last {
            let since = l.elapsed();
            if since < WAKE_INTERVAL {
                thread::sleep(WAKE_INTERVAL - since);
            }
        }
        // Everything finished before this point is covered by the repaint below.
        s.waker
            .state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .dirty = false;
        last = Some(std::time::Instant::now());
        s.stats.wake();
        (s.wake)();
    }
}

fn ext_of(path: &Path) -> String {
    path.extension()
        .map(|e| e.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default()
}

fn or_shell(r: Decoded, shell: impl FnOnce() -> Option<Pixels>) -> Decoded {
    match r {
        Err(Fail::Failed) => shell().ok_or(Fail::Failed),
        other => other,
    }
}

fn produce(s: &Shared, key: &ImageKey, cancel: &dyn Fn() -> bool) -> Decoded {
    let some = |p: Option<Pixels>| p.ok_or(Fail::Failed);
    match key {
        ImageKey::Icon { ext, folder } => {
            let name = if *folder {
                "folder".to_owned()
            } else {
                format!("file.{ext}")
            };
            some(shell::icon(&name, *folder, true))
        }
        ImageKey::PathIcon(p) => some(shell::icon(&p.to_string_lossy(), false, false)),
        ImageKey::Thumb(st) => {
            let path = st.path.to_string_lossy();
            let ck = cache::key(&path, st.modified, st.size, THUMB_PX as u32);
            let read = std::time::Instant::now();
            let cached = s.disk.get(ck);
            s.stats.disk_read(cached.is_some(), read.elapsed());
            if let Some(px) = cached {
                return Ok(px);
            }
            let ext = ext_of(&st.path);
            let decoded = match decode::route(&ext, st.size) {
                // Types the shell handles well (RAW, HEIC, PDF, video, Office) and anything
                // too large for in-process decoding: Explorer's own (warm) thumbnail cache.
                Route::Shell => Err(Fail::Failed),
                _ => decode::thumbnail(&st.path, &ext, st.size, THUMB_PX as u32, cancel),
            };
            let px = or_shell(decoded, || shell::thumb(&path, THUMB_PX))?;
            s.disk.put(ck, &px);
            Ok(px)
        }
        ImageKey::Preview(st, px) => {
            let ext = ext_of(&st.path);
            let decoded = decode::preview(&st.path, &ext, st.size, *px, cancel);
            if matches!(decoded, Err(Fail::Failed)) && cancel() {
                return Err(Fail::Cancelled);
            }
            or_shell(decoded, || {
                shell::thumb(&st.path.to_string_lossy(), *px as i32)
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{fmt::Builder, model::Kind};

    fn file(name: &str) -> Entry {
        Builder::new().entry("C:\\dir", name.to_owned(), Kind::File, 5, 7, 0, 0)
    }

    #[test]
    fn quick_keys_agree_with_the_full_keys() {
        for name in [
            "a.jpg", "b.PNG", "c.exe", "d.LNK", "e.txt", "noext", ".hidden", "f.mp4", "g.pdf",
        ] {
            let e = file(name);
            assert_eq!(
                ImageKey::quick_thumb(&e).is_some(),
                ImageKey::thumb(&e).is_some(),
                "{name} thumb"
            );
            assert_eq!(
                ImageKey::quick_preview(&e, 512).is_some(),
                ImageKey::preview(&e, 512).is_some(),
                "{name} preview"
            );
            let own = matches!(ImageKey::icon(&e), ImageKey::PathIcon(_));
            assert_eq!(
                ImageKey::quick_path_icon(&e).is_some(),
                own,
                "{name} own icon"
            );
        }
    }

    #[test]
    fn quick_key_changes_with_the_file() {
        let mut a = file("a.jpg");
        let before = ImageKey::quick_thumb(&a);
        a.modified += 1;
        assert_ne!(before, ImageKey::quick_thumb(&a));
    }

    #[test]
    fn preview_size_follows_the_panel_in_steps() {
        assert_eq!(preview_px(0.0), 256);
        assert_eq!(preview_px(100.0), 256); // 200 -> 256
        assert_eq!(preview_px(300.0), 768); // 600 -> 768
        assert_eq!(preview_px(301.0), 768);
        assert_eq!(preview_px(400.0), 1024); // 800 -> 1024
        assert_eq!(preview_px(5000.0), PREVIEW_MAX);
        // Resizing the panel by a few pixels must not change the size asked for.
        assert_eq!(preview_px(330.0), preview_px(340.0));
    }

    #[test]
    fn different_preview_sizes_are_different_keys() {
        let e = file("a.jpg");
        assert_ne!(
            ImageKey::quick_preview(&e, 512),
            ImageKey::quick_preview(&e, 1024)
        );
        // A non-image reuses its thumbnail whatever size is asked for.
        let p = file("a.pdf");
        assert_eq!(
            ImageKey::quick_preview(&p, 512),
            ImageKey::quick_preview(&p, 1024)
        );
    }

    #[test]
    fn premultiplying_scales_the_colour_by_alpha() {
        let p = Pixels::new(2, 1, vec![200, 100, 50, 128, 10, 20, 30, 255]).into_premultiplied();
        assert!(p.premultiplied);
        assert_eq!(p.rgba, vec![100, 50, 25, 128, 10, 20, 30, 255]);
        // Idempotent: asking twice must not darken the picture again.
        let again = p.into_premultiplied();
        assert_eq!(again.rgba, vec![100, 50, 25, 128, 10, 20, 30, 255]);
    }

    #[test]
    fn zero_alpha_becomes_transparent_black() {
        let p = Pixels::new(1, 1, vec![255, 255, 255, 0]).into_premultiplied();
        assert_eq!(p.rgba, vec![0, 0, 0, 0]);
    }
}
