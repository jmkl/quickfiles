//! Live sync: watch the current folder and report changes (debounced) so the UI can refresh.

use std::{
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::{Duration, Instant},
};

use notify::{RecommendedWatcher, RecursiveMode, Watcher};

use crate::engine::Wake;

const DEBOUNCE: Duration = Duration::from_millis(200);

/// Calls `on_change` (from a helper thread) after a quiet period following file changes.
pub struct Watch {
    watcher: Mutex<Option<RecommendedWatcher>>,
    dirty: Arc<Mutex<Option<Instant>>>,
    alive: Arc<AtomicBool>,
}

impl Watch {
    pub fn new(wake: Wake, on_change: impl Fn() + Send + 'static) -> Self {
        let dirty: Arc<Mutex<Option<Instant>>> = Arc::default();
        let alive = Arc::new(AtomicBool::new(true));
        let (d, a) = (dirty.clone(), alive.clone());
        thread::spawn(move || {
            while a.load(Ordering::Relaxed) {
                thread::sleep(Duration::from_millis(80));
                let fire = {
                    let mut g = d.lock().unwrap_or_else(|e| e.into_inner());
                    match *g {
                        Some(t) if t.elapsed() >= DEBOUNCE => {
                            *g = None;
                            true
                        }
                        _ => false,
                    }
                };
                if fire {
                    on_change();
                    wake();
                }
            }
        });
        Self {
            watcher: Mutex::new(None),
            dirty,
            alive,
        }
    }

    /// Watch `dir` (non-recursive); `None` or "This PC" stops watching.
    pub fn set(&self, dir: Option<&Path>) {
        let mut slot = self.watcher.lock().unwrap_or_else(|e| e.into_inner());
        *slot = None;
        let Some(dir) = dir.filter(|d| !d.as_os_str().is_empty()) else {
            return;
        };
        let dirty = self.dirty.clone();
        let w = notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
            if res.is_ok() {
                *dirty.lock().unwrap_or_else(|e| e.into_inner()) = Some(Instant::now());
            }
        });
        if let Ok(mut w) = w {
            if w.watch(&PathBuf::from(dir), RecursiveMode::NonRecursive)
                .is_ok()
            {
                *slot = Some(w);
            }
        }
    }
}

impl Drop for Watch {
    fn drop(&mut self) {
        self.alive.store(false, Ordering::Relaxed);
    }
}
