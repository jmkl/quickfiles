//! The real backend: voidtools Everything through `everything-ipc` (`wm` interface).
//! Windows only. One query at a time; a new query cancels the old one inside the crate.

use std::path::Path;
use std::time::Duration;

use everything_ipc::wm::{EverythingClient, RequestFlags, SearchFlags, Sort as ESort};

use super::{Backend, Health, QueryResult};
use crate::{
    compose::{Plan, count_queries},
    fmt::{Builder, filetime_to_unix, flags_from_attrs},
    model::{Kind, SortDir, SortKey},
};

const TIMEOUT: Duration = Duration::from_secs(5);

pub struct Everything {
    client: Option<EverythingClient>,
    tz_offset: i32,
}

impl Everything {
    pub fn new(tz_offset: i32) -> Self {
        Self {
            client: None,
            tz_offset,
        }
    }
}

fn sort_for(key: SortKey, dir: SortDir) -> ESort {
    let asc = dir == SortDir::Asc;
    match key {
        SortKey::Name => {
            if asc {
                ESort::NameAscending
            } else {
                ESort::NameDescending
            }
        }
        SortKey::Modified => {
            if asc {
                ESort::DateModifiedAscending
            } else {
                ESort::DateModifiedDescending
            }
        }
        SortKey::Size => {
            if asc {
                ESort::SizeAscending
            } else {
                ESort::SizeDescending
            }
        }
        SortKey::Type => {
            if asc {
                ESort::ExtensionAscending
            } else {
                ESort::ExtensionDescending
            }
        }
    }
}

impl Backend for Everything {
    fn name(&self) -> &'static str {
        "everything"
    }

    fn connect(&mut self) -> Result<(), String> {
        // Default instance first, then the 1.5 alpha instance.
        let client = match EverythingClient::new() {
            Ok(c) => c,
            Err(_) => EverythingClient::with_instance(Some("1.5a")).map_err(|_| {
                "Everything is not running. Install it from voidtools.com and start it.".to_owned()
            })?,
        };
        if !client.is_ipc_available() {
            return Err("Everything is running but its IPC window is not available.".into());
        }
        self.client = Some(client);
        Ok(())
    }

    fn health(&mut self) -> Health {
        match &self.client {
            Some(c) if !c.is_ipc_available() => {
                self.client = None;
                Health::Missing("Everything was closed.".into())
            }
            Some(c) if c.is_db_loaded() => Health::Ready,
            Some(_) => Health::Loading,
            None => Health::Missing("Everything is not running.".into()),
        }
    }

    fn query(
        &mut self,
        plan: &Plan,
        seg: usize,
        offset: u32,
        count: u32,
        builder: &mut Builder,
    ) -> Result<QueryResult, String> {
        let client = self.client.as_ref().ok_or("Everything is not connected")?;
        let flags = RequestFlags::FileName
            | RequestFlags::Path
            | RequestFlags::Size
            | RequestFlags::DateModified
            | RequestFlags::Attributes;
        // Regex is already in the text (`regex:` prefix, see compose).
        let text = &plan.segs[seg].text;
        let list = client
            .query_wait(text)
            .request_flags(flags)
            .sort(sort_for(plan.sort.key, plan.sort.dir))
            .search_flags(SearchFlags::default())
            .offset(offset)
            .max_results(count)
            .timeout(TIMEOUT)
            .call()
            .map_err(|e| e.to_string())?;

        let total = list.total_len() as u32;
        let mut rows = Vec::with_capacity(list.len() as usize);
        for item in list.iter() {
            let name = item.get_string(RequestFlags::FileName).unwrap_or_default();
            let parent = item.get_string(RequestFlags::Path).unwrap_or_default();
            // 0x10 = FILE_ATTRIBUTE_DIRECTORY; u32::MAX = not indexed.
            let attrs = item.get_u32(RequestFlags::Attributes).unwrap_or(0);
            let valid = attrs != u32::MAX;
            let dir = valid && attrs & 0x10 != 0;
            let size = item
                .get_size(RequestFlags::Size)
                .filter(|&n| n != u64::MAX)
                .unwrap_or(0);
            let modified = item
                .get_time(RequestFlags::DateModified)
                .map(|ft| ((ft.dwHighDateTime as u64) << 32) | ft.dwLowDateTime as u64)
                .filter(|&t| t != 0 && t != u64::MAX)
                .map(filetime_to_unix)
                .unwrap_or(0);
            let kind = if dir { Kind::Folder } else { Kind::File };
            let flags = if valid { flags_from_attrs(attrs) } else { 0 };
            rows.push(builder.entry(&parent, name, kind, size, modified, flags, self.tz_offset));
        }
        Ok(QueryResult { total, rows })
    }

    fn count_children(&mut self, dir: &Path) -> Result<(u32, u32), String> {
        let client = self.client.as_ref().ok_or("Everything is not connected")?;
        let mut out = [0u32; 2];
        for (i, q) in count_queries(dir).iter().enumerate() {
            // max_results(1): we only need the total, never the rows.
            let list = client
                .query_wait(q)
                .request_flags(RequestFlags::FileName)
                .max_results(1)
                .timeout(TIMEOUT)
                .call()
                .map_err(|e| e.to_string())?;
            out[i] = list.total_len() as u32;
        }
        Ok((out[0], out[1]))
    }

    fn dir_has_entries(&mut self, dir: &Path) -> bool {
        std::fs::read_dir(dir).is_ok_and(|mut d| d.next().is_some())
    }
}

/// Smoke tests against a running Everything (PLAN.md Phase 2 "Verify against a running
/// Everything"). They need a live Everything with `C:\Windows` indexed and print "skipped"
/// otherwise, so `cargo test` stays green on machines without it.
#[cfg(test)]
mod smoke {
    use std::path::PathBuf;

    use super::*;
    use crate::{
        compose::{Compose, Plan, compose},
        engine::Backend,
        fmt::Builder,
        model::{Query, Sort},
    };

    fn connected() -> Option<Everything> {
        let mut e = Everything::new(0);
        if e.connect().is_err() || !matches!(e.health(), Health::Ready) {
            eprintln!("skipped: Everything is not running with a loaded index");
            return None;
        }
        Some(e)
    }

    fn plan(location: Option<&str>, text: &str, hidden: bool) -> Plan {
        let q = Query {
            location: location.map(PathBuf::from),
            text: text.to_owned(),
            regex: false,
            show_hidden: hidden,
            sort: Sort::DEFAULT,
        };
        match compose(&q) {
            Compose::Search(p) => p,
            other => panic!("expected a search plan, got {other:?}"),
        }
    }

    #[test]
    fn parent_lists_children_only() {
        let Some(mut e) = connected() else { return };
        let p = plan(Some(r"C:\Windows"), "", true);
        let mut b = Builder::new();
        for seg in 0..p.segs.len() {
            let r = e.query(&p, seg, 0, 200, &mut b).unwrap();
            assert!(!r.rows.is_empty(), "C:\\Windows should have children");
            for row in &r.rows {
                assert!(
                    row.parent.eq_ignore_ascii_case(r"C:\Windows"),
                    "{} is not a direct child",
                    row.path().display()
                );
            }
        }
    }

    #[test]
    fn folders_segment_has_only_folders_and_files_segment_only_files() {
        let Some(mut e) = connected() else { return };
        let p = plan(Some(r"C:\Windows"), "", true);
        let mut b = Builder::new();
        let folders = e.query(&p, 0, 0, 100, &mut b).unwrap();
        let files = e.query(&p, 1, 0, 100, &mut b).unwrap();
        assert!(folders.rows.iter().all(|r| r.is_dir()));
        assert!(files.rows.iter().all(|r| !r.is_dir()));
    }

    #[test]
    fn scoped_search_recurses_below_the_folder() {
        let Some(mut e) = connected() else { return };
        let p = plan(Some(r"C:\Windows"), "notepad.exe", true);
        let mut b = Builder::new();
        let r = e.query(&p, 0, 0, 50, &mut b).unwrap();
        assert!(r.total > 0, "notepad.exe should exist below C:\\Windows");
        let below = r
            .rows
            .iter()
            .all(|row| row.parent.to_ascii_lowercase().starts_with(r"c:\windows"));
        assert!(below, "a result lies outside the scope");
    }

    #[test]
    fn hidden_filter_removes_hidden_entries() {
        let Some(mut e) = connected() else { return };
        let p = plan(Some(r"C:\Windows"), "", false);
        let mut b = Builder::new();
        for seg in 0..p.segs.len() {
            let r = e.query(&p, seg, 0, 500, &mut b).unwrap();
            assert!(r.rows.iter().all(|row| !row.is_hidden()));
        }
    }

    #[test]
    fn zero_or_one_row_still_reports_the_total() {
        let Some(mut e) = connected() else { return };
        let p = plan(None, "ext:dll", true);
        let mut b = Builder::new();
        let one = e.query(&p, 0, 0, 1, &mut b).unwrap();
        assert!(one.rows.len() <= 1);
        assert!(
            one.total >= 100,
            "total_len must not be capped by max_results"
        );
    }

    #[test]
    fn child_counts_match_the_listing() {
        let Some(mut e) = connected() else { return };
        let (folders, files) = e.count_children(Path::new(r"C:\Windows")).unwrap();
        let p = plan(Some(r"C:\Windows"), "", true);
        let mut b = Builder::new();
        assert_eq!(e.query(&p, 0, 0, 1, &mut b).unwrap().total, folders);
        assert_eq!(e.query(&p, 1, 0, 1, &mut b).unwrap().total, files);
    }
}
