//! Turns a [`Query`] into what the engine has to ask for. Pure, unit-tested.
//!
//! | location            | text  | result                                             |
//! |---------------------|-------|----------------------------------------------------|
//! | none                | empty | `Empty` (nothing to show)                          |
//! | none                | text  | Everything: `text`                                 |
//! | folder              | empty | Everything: `parent:"dir"` (children)              |
//! | folder              | text  | Everything: `"dir\" text` (recursive below `dir`)  |
//! | This PC (empty)     | empty | `Drives` (Windows API, not Everything)             |
//! | This PC (empty)     | text  | Everything: `text` (global)                        |
//!
//! A folder is listed as two segments (folders, then files) so folders always come first while
//! sorting and paging stay Everything's job.

use std::path::{Path, PathBuf};

use crate::model::{Query, Sort};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Only {
    Both,
    Folders,
    Files,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Seg {
    /// Final Everything search text of this segment.
    pub text: String,
    pub only: Only,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Plan {
    pub segs: Vec<Seg>,
    pub sort: Sort,
    pub show_hidden: bool,
    /// Browse: the folder whose children are listed.
    pub children_of: Option<PathBuf>,
    /// Scoped search: the folder searched below.
    pub scope: Option<PathBuf>,
    /// The user's raw text (used by the dummy backend).
    pub needle: String,
    pub regex: bool,
    /// Unfiltered browse text, used to tell "only hidden items" from "not indexed".
    pub probe: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Compose {
    Empty,
    Drives,
    Search(Plan),
}

pub fn compose(q: &Query) -> Compose {
    let text = q.text.trim();
    let hidden = if q.show_hidden { "" } else { " !attrib:H" };

    let dir = match &q.location {
        None => {
            if text.is_empty() {
                return Compose::Empty;
            }
            None
        }
        Some(p) if p.as_os_str().is_empty() => {
            if text.is_empty() {
                return Compose::Drives;
            }
            None // searching from "This PC" is a global search
        }
        Some(p) => Some(p.clone()),
    };

    let (base, children_of, scope, probe) = match (&dir, text.is_empty()) {
        (Some(d), true) => {
            let b = format!("parent:\"{}\"", dir_arg(d));
            (b.clone(), Some(d.clone()), None, Some(b))
        }
        (Some(d), false) => (
            format!("\"{}\\\" {}", dir_arg_no_slash(d), term(text, q.regex)),
            None,
            Some(d.clone()),
            None,
        ),
        (None, _) => (term(text, q.regex), None, None, None),
    };
    let base = format!("{base}{hidden}");

    let segs = if q.folders_first() && dir.is_some() {
        vec![
            Seg {
                text: format!("{base} folder:"),
                only: Only::Folders,
            },
            Seg {
                text: format!("{base} file:"),
                only: Only::Files,
            },
        ]
    } else {
        vec![Seg {
            text: base,
            only: Only::Both,
        }]
    };

    Compose::Search(Plan {
        segs,
        sort: q.sort,
        show_hidden: q.show_hidden,
        children_of,
        scope,
        needle: text.to_owned(),
        regex: q.regex,
        probe,
    })
}

/// Everything search texts that count the children of `dir` (folders, files), hidden included.
pub fn count_queries(dir: &Path) -> [String; 2] {
    let base = format!("parent:\"{}\"", dir_arg(dir));
    [format!("{base} folder:"), format!("{base} file:")]
}

/// Directory argument for `parent:`. A drive root keeps its backslash (`C:\`), any other
/// folder loses a trailing one. Windows paths cannot contain `"`, so quoting is always safe.
fn dir_arg(p: &Path) -> String {
    let s = p.to_string_lossy();
    let t = s.trim_end_matches(['\\', '/']);
    if t.len() == 2 && t.ends_with(':') {
        format!("{t}\\")
    } else {
        t.to_owned()
    }
}

/// Same as [`dir_arg`] without a trailing backslash (the caller appends it).
fn dir_arg_no_slash(p: &Path) -> String {
    let s = dir_arg(p);
    s.trim_end_matches('\\').to_owned()
}

fn term(text: &str, regex: bool) -> String {
    if !regex {
        text.to_owned()
    } else if text.chars().any(char::is_whitespace) {
        format!("regex:\"{text}\"")
    } else {
        format!("regex:{text}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn q(location: Option<&str>, text: &str) -> Query {
        Query {
            location: location.map(PathBuf::from),
            text: text.to_owned(),
            regex: false,
            show_hidden: true,
            sort: Sort::DEFAULT,
        }
    }

    fn texts(c: &Compose) -> Vec<String> {
        match c {
            Compose::Search(p) => p.segs.iter().map(|s| s.text.clone()).collect(),
            _ => vec![],
        }
    }

    #[test]
    fn empty_global_is_empty() {
        assert_eq!(compose(&q(None, "")), Compose::Empty);
        assert_eq!(compose(&q(None, "   ")), Compose::Empty);
    }

    #[test]
    fn global_search_is_the_text() {
        let c = compose(&q(None, "foo ext:png"));
        assert_eq!(texts(&c), ["foo ext:png"]);
        let Compose::Search(p) = c else { panic!() };
        assert_eq!(p.segs[0].only, Only::Both);
        assert!(p.children_of.is_none() && p.scope.is_none());
    }

    #[test]
    fn browse_is_parent_children_folders_then_files() {
        let c = compose(&q(Some(r"E:\_CODE\rust"), ""));
        assert_eq!(
            texts(&c),
            [
                r#"parent:"E:\_CODE\rust" folder:"#,
                r#"parent:"E:\_CODE\rust" file:"#
            ]
        );
        let Compose::Search(p) = c else { panic!() };
        assert_eq!(p.segs[0].only, Only::Folders);
        assert_eq!(p.segs[1].only, Only::Files);
        assert_eq!(p.children_of, Some(PathBuf::from(r"E:\_CODE\rust")));
        assert_eq!(p.probe.as_deref(), Some(r#"parent:"E:\_CODE\rust""#));
    }

    #[test]
    fn trailing_backslash_is_normalised() {
        let c = compose(&q(Some(r"E:\_CODE\"), ""));
        assert_eq!(texts(&c)[0], r#"parent:"E:\_CODE" folder:"#);
    }

    #[test]
    fn drive_root_keeps_its_backslash() {
        assert_eq!(
            texts(&compose(&q(Some(r"C:\"), "")))[0],
            r#"parent:"C:\" folder:"#
        );
        assert_eq!(
            texts(&compose(&q(Some("C:"), "")))[0],
            r#"parent:"C:\" folder:"#
        );
    }

    #[test]
    fn scoped_search_is_recursive_below_the_folder() {
        let c = compose(&q(Some(r"E:\_CODE\rust"), "foo"));
        assert_eq!(
            texts(&c),
            [
                r#""E:\_CODE\rust\" foo folder:"#,
                r#""E:\_CODE\rust\" foo file:"#
            ]
        );
        let Compose::Search(p) = c else { panic!() };
        assert_eq!(p.scope, Some(PathBuf::from(r"E:\_CODE\rust")));
        assert!(p.children_of.is_none());
        assert_eq!(p.needle, "foo");
    }

    #[test]
    fn scoped_search_on_a_drive_root() {
        assert_eq!(
            texts(&compose(&q(Some(r"C:\"), "x")))[0],
            r#""C:\" x folder:"#
        );
    }

    #[test]
    fn this_pc() {
        assert_eq!(compose(&q(Some(""), "")), Compose::Drives);
        let c = compose(&q(Some(""), "foo"));
        assert_eq!(texts(&c), ["foo"], "global search from This PC");
    }

    #[test]
    fn hidden_filter_and_regex() {
        let mut query = q(Some(r"C:\a"), "");
        query.show_hidden = false;
        assert_eq!(
            texts(&compose(&query))[1],
            r#"parent:"C:\a" !attrib:H file:"#
        );

        let mut query = q(None, "a.*b");
        query.regex = true;
        query.show_hidden = false;
        assert_eq!(texts(&compose(&query)), ["regex:a.*b !attrib:H"]);
        query.text = "a b".into();
        assert_eq!(texts(&compose(&query)), [r#"regex:"a b" !attrib:H"#]);
    }

    #[test]
    fn count_query_texts() {
        let [f, g] = count_queries(Path::new(r"C:\a\"));
        assert_eq!(f, r#"parent:"C:\a" folder:"#);
        assert_eq!(g, r#"parent:"C:\a" file:"#);
    }
}
