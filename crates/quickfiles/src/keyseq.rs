//! Key-sequence matching (`Space t n`, `g g`) without any UI types, so it can be unit-tested.
//!
//! A binding is a list of chords. While the user is part-way through one, the *pending* prefix
//! decides which chords may come next; [`options`] answers that, and the which-key panel is
//! drawn from the same answer.

/// One chord that may follow the pending prefix.
#[derive(Clone, Debug, PartialEq)]
pub struct Opt<C> {
    pub chord: C,
    /// Index of the first sequence that goes through this chord (for its label and action).
    pub first: usize,
    /// `true` when a sequence ends exactly here.
    pub leaf: bool,
    /// Sequences that continue past this chord (0 for a plain leaf).
    pub more: usize,
}

/// The distinct chords that can follow `pending`, in table order.
pub fn options<C: Copy + PartialEq>(seqs: &[Vec<C>], pending: &[C]) -> Vec<Opt<C>> {
    let mut out: Vec<Opt<C>> = Vec::new();
    for (i, s) in seqs.iter().enumerate() {
        if s.len() <= pending.len() || s[..pending.len()] != *pending {
            continue;
        }
        let chord = s[pending.len()];
        let ends_here = s.len() == pending.len() + 1;
        match out.iter_mut().find(|o| o.chord == chord) {
            Some(o) => {
                o.leaf |= ends_here;
                o.more += usize::from(!ends_here);
            }
            None => out.push(Opt {
                chord,
                first: i,
                leaf: ends_here,
                more: usize::from(!ends_here),
            }),
        }
    }
    out
}

/// Pairs `(short, long)` where `short` is a whole prefix of `long`: such a pair cannot work,
/// because the short one would fire before the long one could be typed.
pub fn conflicts<C: PartialEq>(seqs: &[Vec<C>]) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    for (i, a) in seqs.iter().enumerate() {
        for (j, b) in seqs.iter().enumerate() {
            if a.len() < b.len() && b[..a.len()] == a[..] {
                out.push((i, j));
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t() -> Vec<Vec<char>> {
        ["ab", "ac", "acd", "x", "yz"]
            .iter()
            .map(|s| s.chars().collect())
            .collect()
    }

    #[test]
    fn root_lists_every_first_chord_once() {
        let o = options(&t(), &[]);
        let chords: Vec<char> = o.iter().map(|o| o.chord).collect();
        assert_eq!(chords, ['a', 'x', 'y']);
        assert!(o.iter().find(|o| o.chord == 'x').unwrap().leaf);
        assert_eq!(o.iter().find(|o| o.chord == 'a').unwrap().more, 3);
    }

    #[test]
    fn a_prefix_narrows_the_options() {
        let o = options(&t(), &['a']);
        assert_eq!(o.iter().map(|o| o.chord).collect::<Vec<_>>(), ['b', 'c']);
        let c = o.iter().find(|o| o.chord == 'c').unwrap();
        assert!(
            c.leaf && c.more == 1,
            "'ac' is bound and also a prefix of 'acd'"
        );
        assert_eq!(c.first, 1);
        assert!(options(&t(), &['q']).is_empty());
        assert!(options(&t(), &['a', 'c', 'd']).is_empty());
    }

    #[test]
    fn finds_prefix_conflicts() {
        assert_eq!(conflicts(&t()), vec![(1, 2)]);
        assert!(conflicts(&[vec![1, 2], vec![1, 3], vec![2]]).is_empty());
    }
}
