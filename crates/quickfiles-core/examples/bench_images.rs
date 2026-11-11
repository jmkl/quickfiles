//! Image pipeline benchmark (PLAN-PERF.md "Verification").
//!
//! `cargo run --release -p quickfiles-core --example bench_images -- <folder> [thumb_px] [preview_px]`
//!
//! Times, for every image directly inside `<folder>`: the thumbnail decode, the preview decode,
//! and the disk cache (batched write-behind, then cold reads). Use the folders that
//! `tools/make-test-folders.ps1` creates. Always run with `--release`.

use std::{
    path::PathBuf,
    sync::Arc,
    time::{Duration, Instant},
};

use quickfiles_core::images::{
    THUMB_PX,
    cache::{self, DiskCache},
    decode::{self, Route},
    stats::Stats,
};

fn report(name: &str, mut ms: Vec<f64>) {
    if ms.is_empty() {
        println!("{name:<22} (nothing)");
        return;
    }
    ms.sort_by(|a, b| a.total_cmp(b));
    let at = |q: f64| ms[((ms.len() - 1) as f64 * q) as usize];
    let avg = ms.iter().sum::<f64>() / ms.len() as f64;
    println!(
        "{name:<22} n {:>5}  avg {avg:>8.2} ms  p50 {:>8.2}  p99 {:>8.2}  max {:>8.2}",
        ms.len(),
        at(0.5),
        at(0.99),
        ms[ms.len() - 1]
    );
}

fn main() {
    let mut args = std::env::args().skip(1);
    let Some(folder) = args.next().map(PathBuf::from) else {
        eprintln!("usage: bench_images <folder> [thumb_px] [preview_px]");
        return;
    };
    let thumb_px: u32 = args
        .next()
        .and_then(|a| a.parse().ok())
        .unwrap_or(THUMB_PX as u32);
    let preview_px: u32 = args.next().and_then(|a| a.parse().ok()).unwrap_or(1024);

    let mut files = Vec::new();
    for e in std::fs::read_dir(&folder).into_iter().flatten().flatten() {
        let path = e.path();
        let ext = path
            .extension()
            .map(|x| x.to_string_lossy().to_ascii_lowercase())
            .unwrap_or_default();
        let Ok(meta) = e.metadata() else { continue };
        if decode::route(&ext, meta.len()) != Route::Shell {
            files.push((path, ext, meta.len()));
        }
    }
    println!("{} decodable files in {}", files.len(), folder.display());
    let never = || false;

    let mut thumbs = Vec::new();
    let mut previews = Vec::new();
    let mut pixels = Vec::new();
    for (path, ext, len) in &files {
        let t = Instant::now();
        if let Ok(px) = decode::thumbnail(path, ext, *len, thumb_px, &never) {
            thumbs.push(t.elapsed().as_secs_f64() * 1000.0);
            pixels.push((path.to_string_lossy().into_owned(), *len, px));
        }
        let t = Instant::now();
        if decode::preview(path, ext, *len, preview_px, &never).is_ok() {
            previews.push(t.elapsed().as_secs_f64() * 1000.0);
        }
    }
    report("thumbnail decode", thumbs);
    report(&format!("preview decode {preview_px}"), previews);

    // Disk cache: queue every thumbnail, wait for the write-behind thread, then read them back.
    let dir = std::env::temp_dir().join("quickfiles-bench-cache");
    let _ = std::fs::remove_dir_all(&dir);
    let disk = DiskCache::open(&dir, 512, Arc::new(Stats::default()));
    if !disk.enabled() {
        println!("disk cache unavailable");
        return;
    }
    let keys: Vec<i64> = pixels
        .iter()
        .map(|(p, len, _)| cache::key(p, 0, *len, thumb_px))
        .collect();
    let t = Instant::now();
    for (k, (_, _, px)) in keys.iter().zip(&pixels) {
        disk.put(*k, px);
    }
    let queued = t.elapsed();
    while disk.pending() > 0 && t.elapsed() < Duration::from_secs(60) {
        std::thread::sleep(Duration::from_millis(5));
    }
    println!(
        "cache write            {} thumbnails queued in {:.1} ms, committed after {:.1} ms ({:.1} MB)",
        keys.len(),
        queued.as_secs_f64() * 1000.0,
        t.elapsed().as_secs_f64() * 1000.0,
        disk.bytes() as f64 / (1024.0 * 1024.0)
    );
    let mut reads = Vec::new();
    let all = Instant::now();
    for k in &keys {
        let t = Instant::now();
        let hit = disk.get(*k).is_some();
        reads.push(t.elapsed().as_secs_f64() * 1000.0);
        assert!(hit, "a committed thumbnail was not found");
    }
    report("cache read", reads);
    println!(
        "reopen of the folder   all {} thumbnails from cache in {:.1} ms",
        keys.len(),
        all.elapsed().as_secs_f64() * 1000.0
    );
    disk.stop();
    let _ = std::fs::remove_dir_all(&dir);
}
