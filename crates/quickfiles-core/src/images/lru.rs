//! Decoded-pixel memory cache bounded by bytes (and by count), oldest out first.

use std::{
    collections::{HashMap, VecDeque},
    hash::Hash,
    sync::Arc,
};

use super::Pixels;

pub struct ByteLru<K> {
    map: HashMap<K, Option<Arc<Pixels>>>,
    order: VecDeque<K>,
    bytes: usize,
    max_bytes: usize,
    max_items: usize,
}

fn size_of(px: &Option<Arc<Pixels>>) -> usize {
    px.as_ref().map_or(0, |p| p.rgba.len())
}

impl<K: Clone + Eq + Hash> ByteLru<K> {
    pub fn new(max_bytes: usize, max_items: usize) -> Self {
        Self {
            map: HashMap::new(),
            order: VecDeque::new(),
            bytes: 0,
            max_bytes,
            max_items: max_items.max(1),
        }
    }

    /// `Some(None)` = a known failure (not retried); `None` = never produced.
    pub fn get(&self, key: &K) -> Option<&Option<Arc<Pixels>>> {
        self.map.get(key)
    }

    /// Inserts and evicts the oldest entries until both limits hold. The newest always stays.
    pub fn insert(&mut self, key: K, px: Option<Arc<Pixels>>) {
        self.bytes += size_of(&px);
        if let Some(old) = self.map.insert(key.clone(), px) {
            self.bytes = self.bytes.saturating_sub(size_of(&old));
        }
        self.order.push_back(key);
        while (self.map.len() > self.max_items || self.bytes > self.max_bytes) && self.map.len() > 1
        {
            let Some(old) = self.order.pop_front() else {
                break;
            };
            // A re-inserted key appears twice in `order`; only its newest slot may evict it.
            if self.order.contains(&old) {
                continue;
            }
            if let Some(r) = self.map.remove(&old) {
                self.bytes = self.bytes.saturating_sub(size_of(&r));
            }
        }
        // Keep `order` from growing with duplicates of hot keys.
        if self.order.len() > self.map.len() * 2 + 16 {
            let mut seen = std::collections::HashSet::new();
            let mut fresh: VecDeque<K> = VecDeque::new();
            for k in self.order.drain(..).rev() {
                if self.map.contains_key(&k) && seen.insert(k.clone()) {
                    fresh.push_front(k);
                }
            }
            self.order = fresh;
        }
    }

    pub fn remove_where(&mut self, mut hit: impl FnMut(&K) -> bool) {
        let gone: Vec<K> = self.map.keys().filter(|k| hit(k)).cloned().collect();
        for k in gone {
            if let Some(r) = self.map.remove(&k) {
                self.bytes = self.bytes.saturating_sub(size_of(&r));
            }
        }
        self.order.retain(|k| self.map.contains_key(k));
    }

    pub fn clear(&mut self) {
        self.map.clear();
        self.order.clear();
        self.bytes = 0;
    }

    pub fn len(&self) -> usize {
        self.map.len()
    }

    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }

    pub fn bytes(&self) -> usize {
        self.bytes
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn px(n: usize) -> Option<Arc<Pixels>> {
        Some(Arc::new(Pixels {
            w: 1,
            h: 1,
            rgba: vec![0; n],
            premultiplied: false,
        }))
    }

    #[test]
    fn evicts_oldest_by_bytes() {
        let mut c = ByteLru::new(100, 1000);
        c.insert(1, px(40));
        c.insert(2, px(40));
        c.insert(3, px(40)); // 120 > 100: key 1 goes
        assert!(c.get(&1).is_none());
        assert!(c.get(&2).is_some() && c.get(&3).is_some());
        assert_eq!(c.bytes(), 80);
    }

    #[test]
    fn a_big_item_costs_what_it_weighs() {
        let mut c = ByteLru::new(100, 1000);
        for i in 0..5 {
            c.insert(i, px(10));
        }
        c.insert(99, px(95)); // evicts the small ones until it fits
        assert!(c.bytes() <= 100 + 95); // the newest always stays
        assert!(c.get(&99).is_some());
        assert!(c.get(&0).is_none());
    }

    #[test]
    fn evicts_by_count_too() {
        let mut c = ByteLru::new(usize::MAX, 2);
        c.insert(1, px(1));
        c.insert(2, px(1));
        c.insert(3, px(1));
        assert_eq!(c.len(), 2);
        assert!(c.get(&1).is_none());
    }

    #[test]
    fn replacing_a_key_keeps_the_byte_count_right() {
        let mut c = ByteLru::new(1000, 10);
        c.insert(1, px(100));
        c.insert(1, px(30));
        assert_eq!(c.bytes(), 30);
        assert_eq!(c.len(), 1);
    }

    #[test]
    fn a_refreshed_key_survives_its_old_slot() {
        let mut c = ByteLru::new(usize::MAX, 2);
        c.insert(1, px(1));
        c.insert(2, px(1));
        c.insert(1, px(1)); // 1 is now newer than 2
        c.insert(3, px(1)); // over by count: 2 is the oldest
        assert!(c.get(&2).is_none());
        assert!(c.get(&1).is_some() && c.get(&3).is_some());
    }

    #[test]
    fn failures_are_remembered_and_free() {
        let mut c: ByteLru<u8> = ByteLru::new(10, 10);
        c.insert(1, None);
        assert!(matches!(c.get(&1), Some(None)));
        assert_eq!(c.bytes(), 0);
    }

    #[test]
    fn remove_where_and_clear() {
        let mut c = ByteLru::new(1000, 10);
        c.insert(1, px(10));
        c.insert(2, px(10));
        c.remove_where(|k| *k == 1);
        assert_eq!((c.len(), c.bytes()), (1, 10));
        c.clear();
        assert!(c.is_empty());
    }
}
