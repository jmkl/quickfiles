//! Navigation state machine. Pure logic, no I/O: it decides what the current [`Query`] is.
//!
//! Two modes share one search bar:
//! * **Search mode** (`location == None`): typing searches everything.
//! * **Explore mode** (`location == Some(dir)`): the list is that folder; typing searches below it.
//!
//! The current folder is always the scope of the next query. Every location change clears the
//! search text; Back/Forward restore both the location and the text.

use std::path::PathBuf;

use crate::model::{Query, Sort, SortKey, parent_of};

#[derive(Clone, Debug, PartialEq, Eq)]
struct Snapshot {
    location: Option<PathBuf>,
    text: String,
}

#[derive(Clone, Debug)]
pub struct Nav {
    query: Query,
    back: Vec<Snapshot>,
    forward: Vec<Snapshot>,
}

impl Nav {
    /// `start = None` begins in Search mode with an empty list.
    pub fn new(start: Option<PathBuf>, sort: Sort, show_hidden: bool) -> Self {
        Self {
            query: Query {
                location: start,
                text: String::new(),
                regex: false,
                show_hidden,
                sort,
            },
            back: Vec::new(),
            forward: Vec::new(),
        }
    }

    pub fn query(&self) -> &Query {
        &self.query
    }

    pub fn text(&self) -> &str {
        &self.query.text
    }

    pub fn location(&self) -> Option<&PathBuf> {
        self.query.location.as_ref()
    }

    pub fn is_search_mode(&self) -> bool {
        self.query.location.is_none()
    }

    pub fn can_back(&self) -> bool {
        !self.back.is_empty()
    }

    pub fn can_forward(&self) -> bool {
        !self.forward.is_empty()
    }

    pub fn can_up(&self) -> bool {
        self.query
            .location
            .as_deref()
            .is_some_and(|p| parent_of(p).is_some())
    }

    fn snapshot(&self) -> Snapshot {
        Snapshot {
            location: self.query.location.clone(),
            text: self.query.text.clone(),
        }
    }

    fn restore(&mut self, s: Snapshot) {
        self.query.location = s.location;
        self.query.text = s.text;
    }

    /// Typing in the search box. Does not touch the history. Returns whether anything changed.
    pub fn set_text(&mut self, text: &str) -> bool {
        if self.query.text == text {
            return false;
        }
        self.query.text = text.to_owned();
        true
    }

    /// Enter a folder (an empty path is "This PC"). Clears the text and records history.
    /// Returns false when nothing changed.
    pub fn open_folder(&mut self, path: PathBuf) -> bool {
        if self.query.location.as_ref() == Some(&path) && self.query.text.is_empty() {
            return false;
        }
        let before = self.snapshot();
        self.query.location = Some(path);
        self.query.text.clear();
        self.back.push(before);
        self.forward.clear();
        true
    }

    /// Go to the parent folder (from a drive root: "This PC"). No-op in Search mode.
    pub fn up(&mut self) -> bool {
        match self.query.location.as_deref().and_then(parent_of) {
            Some(parent) => self.open_folder(parent),
            None => false,
        }
    }

    pub fn back(&mut self) -> bool {
        match self.back.pop() {
            Some(prev) => {
                let current = self.snapshot();
                self.forward.push(current);
                self.restore(prev);
                true
            }
            None => false,
        }
    }

    pub fn forward(&mut self) -> bool {
        match self.forward.pop() {
            Some(next) => {
                let current = self.snapshot();
                self.back.push(current);
                self.restore(next);
                true
            }
            None => false,
        }
    }

    /// The x button: clear the text *and* drop the folder scope, so the next query is global.
    pub fn clear(&mut self) -> bool {
        if self.query.location.is_none() && self.query.text.is_empty() {
            return false;
        }
        let before = self.snapshot();
        self.query.location = None;
        self.query.text.clear();
        self.back.push(before);
        self.forward.clear();
        true
    }

    /// Esc: clear the text only, keep the scope.
    pub fn escape(&mut self) -> bool {
        if self.query.text.is_empty() {
            return false;
        }
        self.query.text.clear();
        true
    }

    pub fn set_sort(&mut self, sort: Sort) {
        self.query.sort = sort;
    }

    pub fn toggle_sort(&mut self, key: SortKey) {
        self.query.sort.toggle(key);
    }

    pub fn toggle_regex(&mut self) {
        self.query.regex = !self.query.regex;
    }

    pub fn toggle_hidden(&mut self) {
        self.query.show_hidden = !self.query.show_hidden;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::SortDir;

    fn nav() -> Nav {
        Nav::new(None, Sort::DEFAULT, false)
    }

    fn p(s: &str) -> PathBuf {
        PathBuf::from(s)
    }

    #[test]
    fn starts_in_search_mode() {
        let n = nav();
        assert!(n.is_search_mode());
        assert!(!n.can_back() && !n.can_forward() && !n.can_up());
        assert_eq!(n.text(), "");
    }

    #[test]
    fn typing_does_not_make_history() {
        let mut n = nav();
        assert!(n.set_text("foo"));
        assert!(!n.set_text("foo"));
        assert!(!n.can_back());
        assert!(n.is_search_mode(), "typing alone never leaves search mode");
    }

    #[test]
    fn open_folder_enters_explore_and_clears_text() {
        let mut n = nav();
        n.set_text("report");
        assert!(n.open_folder(p(r"C:\docs")));
        assert_eq!(n.location(), Some(&p(r"C:\docs")));
        assert_eq!(n.text(), "");
        assert!(n.can_back());
        assert!(!n.is_search_mode());
    }

    #[test]
    fn typing_in_explore_keeps_scope() {
        let mut n = nav();
        n.open_folder(p(r"C:\docs"));
        n.set_text("foo");
        assert_eq!(n.query().location, Some(p(r"C:\docs")));
        assert_eq!(n.query().text, "foo");
        assert!(!n.query().is_browse());
    }

    #[test]
    fn back_restores_location_and_text() {
        let mut n = nav();
        n.set_text("report");
        n.open_folder(p(r"C:\docs"));
        assert!(n.back());
        assert!(n.is_search_mode());
        assert_eq!(n.text(), "report");
        assert!(n.can_forward());
        assert!(n.forward());
        assert_eq!(n.location(), Some(&p(r"C:\docs")));
        assert_eq!(n.text(), "");
    }

    #[test]
    fn x_button_resets_to_global() {
        let mut n = nav();
        n.open_folder(p(r"C:\docs"));
        n.set_text("foo");
        assert!(n.clear());
        assert!(n.is_search_mode());
        assert_eq!(n.text(), "");
        // Back returns to where we were, with the text.
        assert!(n.back());
        assert_eq!(n.location(), Some(&p(r"C:\docs")));
        assert_eq!(n.text(), "foo");
        // Nothing to clear in the empty global state.
        let mut m = nav();
        assert!(!m.clear());
    }

    #[test]
    fn escape_clears_text_only() {
        let mut n = nav();
        n.open_folder(p(r"C:\docs"));
        n.set_text("foo");
        assert!(n.escape());
        assert_eq!(n.location(), Some(&p(r"C:\docs")));
        assert!(n.query().is_browse());
        assert!(!n.escape());
    }

    #[test]
    fn up_goes_to_parent_then_this_pc() {
        let mut n = nav();
        assert!(!n.up(), "no-op in search mode");
        n.open_folder(p("a/b"));
        assert!(n.up());
        assert_eq!(n.location(), Some(&p("a")));
        assert!(n.up());
        assert_eq!(n.location(), Some(&PathBuf::new()), "This PC");
        assert!(!n.up());
        assert!(!n.can_up());
    }

    #[cfg(windows)]
    #[test]
    fn up_from_a_drive_root_is_this_pc() {
        let mut n = nav();
        n.open_folder(p(r"C:\a"));
        assert!(n.up());
        assert_eq!(n.location(), Some(&p(r"C:\")));
        assert!(n.up());
        assert_eq!(n.location(), Some(&PathBuf::new()));
    }

    #[test]
    fn navigation_clears_forward_and_dedups() {
        let mut n = nav();
        n.open_folder(p("a"));
        n.open_folder(p("b"));
        n.back();
        assert!(n.can_forward());
        assert!(!n.open_folder(p("a")), "same folder, empty text: no-op");
        n.open_folder(p("c"));
        assert!(!n.can_forward());
    }

    #[test]
    fn same_folder_with_text_clears_text() {
        let mut n = nav();
        n.open_folder(p("a"));
        n.set_text("x");
        assert!(n.open_folder(p("a")));
        assert_eq!(n.text(), "");
    }

    #[test]
    fn sort_and_toggles() {
        let mut n = nav();
        n.toggle_sort(SortKey::Name);
        assert_eq!(n.query().sort.dir, SortDir::Desc);
        n.toggle_regex();
        n.toggle_hidden();
        assert!(n.query().regex && n.query().show_hidden);
    }
}
