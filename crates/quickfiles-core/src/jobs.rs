//! File operations on a worker thread: copy, move, recycle, delete, rename, new folder.
//! Progress is polled by the UI; every job can be cancelled; name clashes follow a policy.

use std::{
    fs, io,
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    thread,
};

use crate::engine::Wake;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Conflict {
    /// Keep both: the new item gets "name (2)".
    KeepBoth,
    Replace,
    Skip,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    Copy,
    Move,
    Recycle,
    Delete,
}

impl Kind {
    pub fn verb(self) -> &'static str {
        match self {
            Kind::Copy => "Copying",
            Kind::Move => "Moving",
            Kind::Recycle => "Recycling",
            Kind::Delete => "Deleting",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum JobState {
    Running,
    Done,
    Cancelled,
    Failed(String),
}

#[derive(Clone, Debug)]
pub struct JobInfo {
    pub id: u64,
    pub kind: Kind,
    pub state: JobState,
    pub done: u64,
    pub total: u64,
    /// Bytes copied so far / expected in total (copy jobs only; 0 when unknown).
    pub bytes_done: u64,
    pub bytes_total: u64,
    pub current: String,
}

impl JobInfo {
    /// 0..=1 progress: by bytes when known, else by finished items.
    pub fn fraction(&self) -> f32 {
        if self.bytes_total > 0 {
            (self.bytes_done as f64 / self.bytes_total as f64).clamp(0.0, 1.0) as f32
        } else if self.total > 0 {
            (self.done as f64 / self.total as f64).clamp(0.0, 1.0) as f32
        } else {
            0.0
        }
    }
}

struct Job {
    id: u64,
    kind: Kind,
    cancel: AtomicBool,
    done: AtomicU64,
    total: AtomicU64,
    bytes_done: AtomicU64,
    bytes_total: AtomicU64,
    state: Mutex<(JobState, String)>,
}

#[derive(Clone)]
pub struct Jobs {
    list: Arc<Mutex<Vec<Arc<Job>>>>,
    next: Arc<AtomicU64>,
    wake: Wake,
}

impl Jobs {
    pub fn new(wake: Wake) -> Self {
        Self {
            list: Arc::default(),
            next: Arc::new(AtomicU64::new(1)),
            wake,
        }
    }

    /// Start a job over `sources`; `dest` is the target folder for copy and move.
    pub fn start(
        &self,
        kind: Kind,
        sources: Vec<PathBuf>,
        dest: Option<PathBuf>,
        policy: Conflict,
        on_done: impl FnOnce() + Send + 'static,
    ) -> u64 {
        let job = Arc::new(Job {
            id: self.next.fetch_add(1, Ordering::Relaxed),
            kind,
            cancel: AtomicBool::new(false),
            done: AtomicU64::new(0),
            total: AtomicU64::new(sources.len() as u64),
            bytes_done: AtomicU64::new(0),
            bytes_total: AtomicU64::new(0),
            state: Mutex::new((JobState::Running, String::new())),
        });
        self.list
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push(job.clone());
        let wake = self.wake.clone();
        let id = job.id;
        thread::spawn(move || {
            let result = run(&job, &sources, dest.as_deref(), policy, &wake);
            let mut st = job.state.lock().unwrap_or_else(|e| e.into_inner());
            st.0 = if job.cancel.load(Ordering::Relaxed) {
                JobState::Cancelled
            } else {
                match result {
                    Ok(()) => JobState::Done,
                    Err(e) => JobState::Failed(e.to_string()),
                }
            };
            drop(st);
            on_done();
            wake();
        });
        id
    }

    pub fn cancel(&self, id: u64) {
        if let Some(j) = self
            .list
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .iter()
            .find(|j| j.id == id)
        {
            j.cancel.store(true, Ordering::Relaxed);
        }
    }

    pub fn snapshot(&self) -> Vec<JobInfo> {
        self.list
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .iter()
            .map(|j| {
                let st = j.state.lock().unwrap_or_else(|e| e.into_inner());
                JobInfo {
                    id: j.id,
                    kind: j.kind,
                    state: st.0.clone(),
                    done: j.done.load(Ordering::Relaxed),
                    total: j.total.load(Ordering::Relaxed),
                    bytes_done: j.bytes_done.load(Ordering::Relaxed),
                    bytes_total: j.bytes_total.load(Ordering::Relaxed),
                    current: st.1.clone(),
                }
            })
            .collect()
    }

    /// Drop finished jobs the UI has already reported.
    pub fn prune_finished(&self) {
        self.list
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .retain(|j| j.state.lock().unwrap_or_else(|e| e.into_inner()).0 == JobState::Running);
    }
}

fn run(
    job: &Job,
    sources: &[PathBuf],
    dest: Option<&Path>,
    policy: Conflict,
    wake: &Wake,
) -> io::Result<()> {
    if job.kind == Kind::Copy {
        // Total size first, so the progress bar can move by bytes. Links are never followed.
        let total: u64 = sources.iter().map(|s| tree_size(s, &job.cancel)).sum();
        job.bytes_total.store(total, Ordering::Relaxed);
        wake();
    }
    for src in sources {
        if job.cancel.load(Ordering::Relaxed) {
            return Ok(());
        }
        job.state.lock().unwrap_or_else(|e| e.into_inner()).1 = src
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        wake();
        match job.kind {
            Kind::Recycle => {
                crate::sys::recycle(std::slice::from_ref(src)).map_err(io::Error::other)?
            }
            Kind::Delete => {
                if src.is_dir() {
                    fs::remove_dir_all(src)?;
                } else {
                    fs::remove_file(src)?;
                }
            }
            Kind::Copy | Kind::Move => {
                let dest = dest.ok_or_else(|| io::Error::other("no destination"))?;
                let Some(name) = src.file_name() else {
                    continue;
                };
                let Some(target) = resolve(&dest.join(name), src, policy) else {
                    job.done.fetch_add(1, Ordering::Relaxed);
                    continue;
                };
                if job.kind == Kind::Move {
                    move_item(src, &target, job, wake)?;
                } else {
                    copy_item(src, &target, job, wake)?;
                }
            }
        }
        job.done.fetch_add(1, Ordering::Relaxed);
        wake();
    }
    Ok(())
}

/// Pick the final target path; `None` means skip.
fn resolve(target: &Path, src: &Path, policy: Conflict) -> Option<PathBuf> {
    if !target.exists() {
        return Some(target.to_owned());
    }
    // Copying a folder onto itself: always keep both.
    if target == src {
        return Some(unique(target));
    }
    match policy {
        Conflict::Skip => None,
        Conflict::KeepBoth => Some(unique(target)),
        Conflict::Replace => {
            let _ = if target.is_dir() {
                fs::remove_dir_all(target)
            } else {
                fs::remove_file(target)
            };
            Some(target.to_owned())
        }
    }
}

/// "name.ext" -> "name (2).ext", "name (3).ext", ...
pub fn unique(path: &Path) -> PathBuf {
    let dir = path.parent().unwrap_or(Path::new(""));
    let stem = path
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    let ext = path
        .extension()
        .map(|e| format!(".{}", e.to_string_lossy()))
        .unwrap_or_default();
    (2..)
        .map(|n| dir.join(format!("{stem} ({n}){ext}")))
        .find(|p| !p.exists())
        .expect("unbounded range")
}

fn move_item(src: &Path, target: &Path, job: &Job, wake: &Wake) -> io::Result<()> {
    match fs::rename(src, target) {
        Ok(()) => Ok(()),
        // Different volume: copy then delete (only when the copy ran to the end).
        Err(_) => {
            job.bytes_total
                .fetch_add(tree_size(src, &job.cancel), Ordering::Relaxed);
            copy_item(src, target, job, wake)?;
            if job.cancel.load(Ordering::Relaxed) {
                return Ok(());
            }
            if src.is_dir() {
                fs::remove_dir_all(src)
            } else {
                fs::remove_file(src)
            }
        }
    }
}

/// True for symlinks and junctions: they are never followed (no loops, no surprise copies).
fn is_link(path: &Path) -> bool {
    fs::symlink_metadata(path).is_ok_and(|m| m.file_type().is_symlink())
}

/// Total bytes below `path`, not following links.
pub fn tree_size(path: &Path, cancel: &AtomicBool) -> u64 {
    let Ok(meta) = fs::symlink_metadata(path) else {
        return 0;
    };
    if meta.file_type().is_symlink() {
        return 0;
    }
    if !meta.is_dir() {
        return meta.len();
    }
    let Ok(rd) = fs::read_dir(path) else {
        return 0;
    };
    let mut sum = 0;
    for entry in rd.flatten() {
        if cancel.load(Ordering::Relaxed) {
            break;
        }
        sum += tree_size(&entry.path(), cancel);
    }
    sum
}

fn copy_item(src: &Path, target: &Path, job: &Job, wake: &Wake) -> io::Result<()> {
    if is_link(src) && src.is_dir() {
        // A link to a folder is skipped, not followed.
        return Ok(());
    }
    if src.is_dir() {
        if target.starts_with(src) {
            return Err(io::Error::other("cannot copy a folder into itself"));
        }
        fs::create_dir_all(target)?;
        for entry in fs::read_dir(src)? {
            if job.cancel.load(Ordering::Relaxed) {
                return Ok(());
            }
            let entry = entry?;
            copy_item(&entry.path(), &target.join(entry.file_name()), job, wake)?;
        }
        Ok(())
    } else {
        copy_file(src, target, job, wake)
    }
}

/// Chunked file copy: reports bytes as it goes and stops promptly on cancel (the partial
/// target is removed).
fn copy_file(src: &Path, target: &Path, job: &Job, wake: &Wake) -> io::Result<()> {
    use std::io::{Read, Write};
    let mut input = fs::File::open(src)?;
    let mut output = fs::File::create(target)?;
    let mut buf = vec![0u8; 1 << 20];
    let mut last_wake = std::time::Instant::now();
    loop {
        if job.cancel.load(Ordering::Relaxed) {
            drop(output);
            let _ = fs::remove_file(target);
            return Ok(());
        }
        let n = input.read(&mut buf)?;
        if n == 0 {
            break;
        }
        output.write_all(&buf[..n])?;
        job.bytes_done.fetch_add(n as u64, Ordering::Relaxed);
        if last_wake.elapsed() >= std::time::Duration::from_millis(100) {
            last_wake = std::time::Instant::now();
            wake();
        }
    }
    // Keep the modified time, as Explorer does.
    if let Ok(modified) = input.metadata().and_then(|m| m.modified()) {
        let _ = output.set_modified(modified);
    }
    Ok(())
}

/// How many of `sources` would collide with an existing item in `dest` (used to ask the user
/// once per job). Moving an item into its own folder is not a collision.
pub fn conflicts(sources: &[PathBuf], dest: &Path) -> usize {
    sources
        .iter()
        .filter(|s| {
            let Some(name) = s.file_name() else {
                return false;
            };
            let target = dest.join(name);
            target != **s && target.exists()
        })
        .count()
}

pub fn rename(path: &Path, new_name: &str) -> io::Result<PathBuf> {
    let name = new_name.trim();
    if name.is_empty() || name.contains(['\\', '/', ':', '*', '?', '"', '<', '>', '|']) {
        return Err(io::Error::other("invalid name"));
    }
    let target = path.with_file_name(name);
    if target != path
        && target.exists()
        && !name.eq_ignore_ascii_case(&path.file_name().unwrap_or_default().to_string_lossy())
    {
        return Err(io::Error::other("an item with that name already exists"));
    }
    fs::rename(path, &target)?;
    Ok(target)
}

/// Creates "New folder", "New folder (2)", ... inside `dir`.
pub fn new_folder(dir: &Path) -> io::Result<PathBuf> {
    let mut target = dir.join("New folder");
    if target.exists() {
        target = unique(&target);
    }
    fs::create_dir(&target)?;
    Ok(target)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(name: &str) -> PathBuf {
        let p = std::env::temp_dir().join(format!("qf-jobs-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&p);
        fs::create_dir_all(&p).unwrap();
        p
    }

    fn wait(jobs: &Jobs) -> JobState {
        for _ in 0..500 {
            let s = jobs.snapshot();
            if let Some(j) = s.last() {
                if j.state != JobState::Running {
                    return j.state.clone();
                }
            }
            thread::sleep(std::time::Duration::from_millis(10));
        }
        panic!("job timed out");
    }

    #[test]
    fn copy_move_and_conflicts() {
        let root = tmp("a");
        let (src, dst) = (root.join("src"), root.join("dst"));
        fs::create_dir_all(src.join("sub")).unwrap();
        fs::create_dir_all(&dst).unwrap();
        fs::write(src.join("a.txt"), "hello").unwrap();
        fs::write(src.join("sub/b.txt"), "b").unwrap();
        let jobs = Jobs::new(Arc::new(|| {}));

        jobs.start(
            Kind::Copy,
            vec![src.join("a.txt"), src.join("sub")],
            Some(dst.clone()),
            Conflict::KeepBoth,
            || {},
        );
        assert_eq!(wait(&jobs), JobState::Done);
        assert_eq!(fs::read_to_string(dst.join("a.txt")).unwrap(), "hello");
        assert!(dst.join("sub/b.txt").exists());

        jobs.prune_finished();
        jobs.start(
            Kind::Copy,
            vec![src.join("a.txt")],
            Some(dst.clone()),
            Conflict::KeepBoth,
            || {},
        );
        assert_eq!(wait(&jobs), JobState::Done);
        assert!(dst.join("a (2).txt").exists());

        jobs.prune_finished();
        jobs.start(
            Kind::Move,
            vec![src.join("a.txt")],
            Some(dst.clone()),
            Conflict::Skip,
            || {},
        );
        assert_eq!(wait(&jobs), JobState::Done);
        assert!(src.join("a.txt").exists(), "skipped, source untouched");

        jobs.prune_finished();
        jobs.start(
            Kind::Delete,
            vec![dst.join("sub")],
            None,
            Conflict::Skip,
            || {},
        );
        assert_eq!(wait(&jobs), JobState::Done);
        assert!(!dst.join("sub").exists());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn rename_and_new_folder() {
        let root = tmp("b");
        let f = root.join("x.txt");
        fs::write(&f, "").unwrap();
        assert!(rename(&f, "bad/name").is_err());
        let g = rename(&f, "y.txt").unwrap();
        assert!(g.exists() && !f.exists());
        let a = new_folder(&root).unwrap();
        let b = new_folder(&root).unwrap();
        assert_eq!(a.file_name().unwrap(), "New folder");
        assert_eq!(b.file_name().unwrap(), "New folder (2)");
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn conflict_count_and_tree_size() {
        let root = tmp("c");
        let (a, b) = (root.join("a"), root.join("b"));
        fs::create_dir_all(&a).unwrap();
        fs::create_dir_all(&b).unwrap();
        fs::write(a.join("x.txt"), "12345").unwrap();
        fs::write(a.join("y.txt"), "123").unwrap();
        fs::write(b.join("x.txt"), "").unwrap();
        let srcs = vec![a.join("x.txt"), a.join("y.txt")];
        assert_eq!(conflicts(&srcs, &b), 1);
        // Same folder: the item itself is not a collision.
        assert_eq!(conflicts(&srcs, &a), 0);
        assert_eq!(tree_size(&a, &AtomicBool::new(false)), 8);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn copy_reports_bytes() {
        let root = tmp("d");
        let (src, dst) = (root.join("s"), root.join("d"));
        fs::create_dir_all(&src).unwrap();
        fs::create_dir_all(&dst).unwrap();
        fs::write(src.join("big.bin"), vec![7u8; 3 << 20]).unwrap();
        let jobs = Jobs::new(Arc::new(|| {}));
        jobs.start(
            Kind::Copy,
            vec![src.join("big.bin")],
            Some(dst.clone()),
            Conflict::KeepBoth,
            || {},
        );
        assert_eq!(wait(&jobs), JobState::Done);
        let info = jobs.snapshot().pop().unwrap();
        assert_eq!(info.bytes_total, 3 << 20);
        assert_eq!(info.bytes_done, 3 << 20);
        assert!((info.fraction() - 1.0).abs() < f32::EPSILON);
        assert_eq!(fs::metadata(dst.join("big.bin")).unwrap().len(), 3 << 20);
        let _ = fs::remove_dir_all(root);
    }
}
