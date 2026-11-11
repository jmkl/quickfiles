//! The work queue of one image lane: what the UI wants, newest first, with a depth cap.
//!
//! Pure logic (no threads, no I/O) so it is unit-tested. Entries carry the frame they were last
//! wanted in; an entry that was not asked for in the current or previous frame is dropped
//! instead of run, so fast scrolling never leaves junk behind.

use std::{
    collections::{HashMap, VecDeque},
    hash::Hash,
};

/// How urgent a request is.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Prio {
    /// On screen right now.
    Visible,
    /// Speculative (neighbours, rows ahead of the scroll). Only runs when nothing visible waits.
    Prefetch,
}

/// A queue of keys. `push`/`pop` are O(1) amortised; stale entries are skipped lazily.
pub struct Queue<K> {
    wanted: HashMap<K, (u64, Prio)>,
    visible: VecDeque<K>,
    prefetch: VecDeque<K>,
    cap: usize,
    dropped: u64,
}

impl<K: Clone + Eq + Hash> Queue<K> {
    pub fn new(cap: usize) -> Self {
        Self {
            wanted: HashMap::new(),
            visible: VecDeque::new(),
            prefetch: VecDeque::new(),
            cap: cap.max(1),
            dropped: 0,
        }
    }

    /// Records interest. Returns `true` when there is new work for a worker (the key was not
    /// queued yet, or a prefetch became visible); `false` when it only refreshed an entry.
    pub fn want(&mut self, key: &K, prio: Prio, frame: u64) -> bool {
        if let Some((f, p)) = self.wanted.get_mut(key) {
            *f = frame;
            if *p == Prio::Prefetch && prio == Prio::Visible {
                *p = Prio::Visible;
                self.visible.push_back(key.clone());
                return true;
            }
            return false;
        }
        self.wanted.insert(key.clone(), (frame, prio));
        match prio {
            Prio::Visible => self.visible.push_back(key.clone()),
            Prio::Prefetch => self.prefetch.push_back(key.clone()),
        }
        self.enforce_cap();
        true
    }

    /// Past the cap the *oldest* wanted keys go (the newest interest is what is on screen).
    fn enforce_cap(&mut self) {
        while self.wanted.len() > self.cap {
            // Prefetch work is the first to go, then the oldest visible request.
            let victim = loop {
                let v = match self.prefetch.pop_front() {
                    Some(k) => k,
                    None => match self.visible.pop_front() {
                        Some(k) => k,
                        None => return,
                    },
                };
                // Skip entries that were already removed or promoted to the other list.
                if self.wanted.contains_key(&v) {
                    break v;
                }
            };
            self.wanted.remove(&victim);
            self.dropped += 1;
        }
    }

    /// The next key to run at `frame`: newest visible interest first, then prefetch.
    /// Entries not wanted in this or the previous frame are discarded on the way.
    pub fn pop(&mut self, frame: u64) -> Option<K> {
        for list in [Prio::Visible, Prio::Prefetch] {
            loop {
                let k = match list {
                    Prio::Visible => self.visible.pop_back(),
                    Prio::Prefetch => self.prefetch.pop_back(),
                };
                let Some(k) = k else { break };
                match self.wanted.get(&k).copied() {
                    // Removed, or promoted: the copy in the other list handles it.
                    None => continue,
                    Some((_, p)) if p != list => continue,
                    Some((f, _)) if f.saturating_add(1) >= frame => {
                        self.wanted.remove(&k);
                        return Some(k);
                    }
                    Some(_) => {
                        self.wanted.remove(&k);
                    }
                }
            }
        }
        None
    }

    pub fn len(&self) -> usize {
        self.wanted.len()
    }

    pub fn is_empty(&self) -> bool {
        self.wanted.is_empty()
    }

    /// Requests dropped because the queue was over its cap.
    pub fn dropped(&self) -> u64 {
        self.dropped
    }

    pub fn clear(&mut self) {
        self.wanted.clear();
        self.visible.clear();
        self.prefetch.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn newest_visible_first_then_prefetch() {
        let mut q = Queue::new(100);
        q.want(&"a", Prio::Visible, 1);
        q.want(&"p", Prio::Prefetch, 1);
        q.want(&"b", Prio::Visible, 1);
        assert_eq!(q.pop(1), Some("b"));
        assert_eq!(q.pop(1), Some("a"));
        assert_eq!(q.pop(1), Some("p"));
        assert_eq!(q.pop(1), None);
        assert!(q.is_empty());
    }

    #[test]
    fn entries_not_wanted_recently_are_dropped() {
        let mut q = Queue::new(100);
        q.want(&1, Prio::Visible, 1);
        q.want(&2, Prio::Visible, 5);
        assert_eq!(q.pop(6), Some(2)); // wanted in the previous frame: fine
        assert_eq!(q.pop(6), None); // 1 was last wanted five frames ago: discarded
        assert!(q.is_empty());
    }

    #[test]
    fn asking_again_refreshes_the_entry() {
        let mut q = Queue::new(100);
        assert!(q.want(&1, Prio::Visible, 1));
        assert!(!q.want(&1, Prio::Visible, 9));
        assert_eq!(q.pop(9), Some(1));
    }

    #[test]
    fn prefetch_is_promoted_when_it_becomes_visible() {
        let mut q = Queue::new(100);
        q.want(&"x", Prio::Prefetch, 1);
        q.want(&"y", Prio::Visible, 1);
        q.want(&"x", Prio::Visible, 1);
        assert_eq!(q.pop(1), Some("x"));
        assert_eq!(q.pop(1), Some("y"));
        assert_eq!(q.pop(1), None); // the stale prefetch copy of x is skipped
    }

    #[test]
    fn depth_cap_drops_prefetch_first_then_the_oldest() {
        let mut q = Queue::new(3);
        q.want(&"p", Prio::Prefetch, 1);
        q.want(&"a", Prio::Visible, 1);
        q.want(&"b", Prio::Visible, 1);
        q.want(&"c", Prio::Visible, 1); // over the cap: the prefetch goes
        assert_eq!(q.len(), 3);
        assert_eq!(q.dropped(), 1);
        q.want(&"d", Prio::Visible, 1); // now the oldest visible (a) goes
        assert_eq!(q.dropped(), 2);
        assert_eq!(q.pop(1), Some("d"));
        assert_eq!(q.pop(1), Some("c"));
        assert_eq!(q.pop(1), Some("b"));
        assert_eq!(q.pop(1), None);
    }

    #[test]
    fn a_run_key_can_be_wanted_again() {
        let mut q = Queue::new(10);
        q.want(&1, Prio::Visible, 1);
        assert_eq!(q.pop(1), Some(1));
        assert!(q.want(&1, Prio::Visible, 2));
        assert_eq!(q.pop(2), Some(1));
    }
}
