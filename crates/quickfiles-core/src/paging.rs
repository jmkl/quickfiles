//! Maps view indices onto fixed-size pages of one or two stitched segments
//! (segment 0 = folders, segment 1 = files). Pure arithmetic, unit-tested.

use std::ops::Range;

/// Rows per page: one Everything query fetches this many.
pub const PAGE: u32 = 512;
pub const MAX_SEGS: usize = 2;

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct PageId {
    pub seg: u8,
    pub idx: u32,
}

#[derive(Clone, Copy, Default, PartialEq, Eq, Debug)]
pub struct Layout {
    /// Total rows of each segment.
    pub counts: [u32; MAX_SEGS],
}

impl Layout {
    pub fn new(counts: [u32; MAX_SEGS]) -> Self {
        Self { counts }
    }

    pub fn total(&self) -> usize {
        self.counts.iter().map(|&c| c as usize).sum()
    }

    /// Number of pages of one segment.
    pub fn seg_pages(&self, seg: usize) -> u32 {
        self.counts[seg].div_ceil(PAGE)
    }

    pub fn page_count(&self) -> usize {
        (0..MAX_SEGS).map(|s| self.seg_pages(s) as usize).sum()
    }

    /// View index of the first row of segment `seg`.
    fn seg_base(&self, seg: usize) -> usize {
        self.counts[..seg].iter().map(|&c| c as usize).sum()
    }

    /// Page holding view row `i` and the row's position inside that page.
    pub fn locate(&self, i: usize) -> Option<(PageId, usize)> {
        let mut rest = i;
        for seg in 0..MAX_SEGS {
            let count = self.counts[seg] as usize;
            if rest < count {
                return Some((
                    PageId {
                        seg: seg as u8,
                        idx: (rest / PAGE as usize) as u32,
                    },
                    rest % PAGE as usize,
                ));
            }
            rest -= count;
        }
        None
    }

    /// View index of the first row of `id`.
    pub fn base(&self, id: PageId) -> usize {
        self.seg_base(id.seg as usize) + id.idx as usize * PAGE as usize
    }

    /// Rows in `id` (0 when the page does not exist).
    pub fn page_rows(&self, id: PageId) -> usize {
        let seg = id.seg as usize;
        if seg >= MAX_SEGS {
            return 0;
        }
        let start = id.idx as usize * PAGE as usize;
        (self.counts[seg] as usize)
            .saturating_sub(start)
            .min(PAGE as usize)
    }

    /// Pages overlapping the view range, in view order.
    pub fn pages_in(&self, range: Range<usize>) -> Vec<PageId> {
        let end = range.end.min(self.total());
        let mut out = Vec::new();
        let mut i = range.start;
        while i < end {
            let Some((id, _)) = self.locate(i) else {
                break;
            };
            out.push(id);
            i = self.base(id) + self.page_rows(id);
        }
        out
    }

    pub fn all_pages(&self) -> Vec<PageId> {
        self.pages_in(0..self.total())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(seg: u8, idx: u32) -> PageId {
        PageId { seg, idx }
    }

    #[test]
    fn locate_across_segments() {
        let l = Layout::new([1000, 30]);
        assert_eq!(l.total(), 1030);
        assert_eq!(l.locate(0), Some((id(0, 0), 0)));
        assert_eq!(l.locate(511), Some((id(0, 0), 511)));
        assert_eq!(l.locate(512), Some((id(0, 1), 0)));
        assert_eq!(l.locate(999), Some((id(0, 1), 487)));
        assert_eq!(l.locate(1000), Some((id(1, 0), 0)));
        assert_eq!(l.locate(1029), Some((id(1, 0), 29)));
        assert_eq!(l.locate(1030), None);
    }

    #[test]
    fn page_geometry() {
        let l = Layout::new([1000, 30]);
        assert_eq!(l.page_count(), 3);
        assert_eq!(l.base(id(0, 1)), 512);
        assert_eq!(l.base(id(1, 0)), 1000);
        assert_eq!(l.page_rows(id(0, 0)), 512);
        assert_eq!(l.page_rows(id(0, 1)), 488);
        assert_eq!(l.page_rows(id(1, 0)), 30);
        assert_eq!(l.page_rows(id(1, 1)), 0);
        assert_eq!(l.page_rows(id(2, 0)), 0);
    }

    #[test]
    fn pages_in_range() {
        let l = Layout::new([1000, 30]);
        assert_eq!(l.pages_in(0..10), vec![id(0, 0)]);
        assert_eq!(l.pages_in(500..520), vec![id(0, 0), id(0, 1)]);
        assert_eq!(l.pages_in(990..1010), vec![id(0, 1), id(1, 0)]);
        assert_eq!(l.pages_in(1020..5000), vec![id(1, 0)]);
        assert_eq!(l.pages_in(2000..3000), vec![]);
        assert_eq!(l.all_pages(), vec![id(0, 0), id(0, 1), id(1, 0)]);
    }

    #[test]
    fn empty_first_segment() {
        let l = Layout::new([0, 700]);
        assert_eq!(l.locate(0), Some((id(1, 0), 0)));
        assert_eq!(l.base(id(1, 1)), 512);
        assert_eq!(l.pages_in(0..700), vec![id(1, 0), id(1, 1)]);
    }

    #[test]
    fn empty_layout() {
        let l = Layout::default();
        assert_eq!(l.total(), 0);
        assert_eq!(l.locate(0), None);
        assert!(l.all_pages().is_empty());
    }

    #[test]
    fn exact_page_boundary() {
        let l = Layout::new([512, 0]);
        assert_eq!(l.page_count(), 1);
        assert_eq!(l.locate(511), Some((id(0, 0), 511)));
        assert_eq!(l.locate(512), None);
    }
}
