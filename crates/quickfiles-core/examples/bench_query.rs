//! Listing benchmark (PLAN.md "Testing"): time to the first row and page-fetch latency.
//!
//! `cargo run --release -p quickfiles-core --example bench_query -- [--dummy] "<query>" [folder]`
//!
//! With Everything running, try a query that matches about a million files (for example `.`), and
//! a folder with 100k entries. `--dummy` uses the generated file system (20,000-entry folders).

use std::{
    path::PathBuf,
    sync::Arc,
    time::{Duration, Instant},
};

use quickfiles_core::{Config, Core, EngineStatus};

fn wait_for(what: &str, limit: Duration, mut done: impl FnMut() -> bool) -> Option<Duration> {
    let t = Instant::now();
    while t.elapsed() < limit {
        if done() {
            return Some(t.elapsed());
        }
        std::thread::sleep(Duration::from_millis(1));
    }
    eprintln!("timed out waiting for {what}");
    None
}

fn main() {
    let mut dummy = false;
    let mut rest = Vec::new();
    for a in std::env::args().skip(1) {
        if a == "--dummy" {
            dummy = true;
        } else {
            rest.push(a);
        }
    }
    let query = rest.first().cloned().unwrap_or_else(|| ".".to_owned());
    let folder = rest.get(1).map(PathBuf::from);

    let data_dir = std::env::temp_dir().join("quickfiles-bench-data");
    let mut core = Core::start(Config {
        dummy,
        data_dir: Some(data_dir),
        wake: Arc::new(|| {}),
    });
    if wait_for("Everything", Duration::from_secs(30), || {
        matches!(core.status(), EngineStatus::Ready)
    })
    .is_none()
    {
        eprintln!("status: {:?}", core.status());
        return;
    }

    let t = Instant::now();
    match &folder {
        Some(f) => core.open_folder(f.clone()),
        None => core.set_text(&query),
    }
    let first = wait_for("the first row", Duration::from_secs(30), || {
        core.view().want(0..50);
        core.view().get(0).is_some()
    });
    let rows = core.view().len();
    println!(
        "query {:?}: {rows} rows, first row after {:.1} ms (engine reports {:.1} ms)",
        folder.as_ref().map_or(query.as_str(), |_| "<folder>"),
        first.map_or(f64::NAN, |d| d.as_secs_f64() * 1000.0),
        core.view().took().as_secs_f64() * 1000.0
    );
    let _ = t;

    // Jump around like a dragged scrollbar and time how long until the rows there are loaded.
    let mut lat = Vec::new();
    for step in 1..=20usize {
        let at = rows.saturating_sub(60) * step / 20;
        let t = Instant::now();
        let ok = wait_for("a page", Duration::from_secs(10), || {
            core.view().want(at..(at + 40).min(rows));
            core.view().get(at).is_some()
        });
        if ok.is_some() {
            lat.push(t.elapsed().as_secs_f64() * 1000.0);
        }
    }
    lat.sort_by(|a, b| a.total_cmp(b));
    if !lat.is_empty() {
        println!(
            "page fetch after a jump: n {}  p50 {:.1} ms  p99 {:.1} ms  max {:.1} ms",
            lat.len(),
            lat[lat.len() / 2],
            lat[(lat.len() - 1) * 99 / 100],
            lat[lat.len() - 1]
        );
    }
}
