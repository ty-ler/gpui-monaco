//! Describing how text changed between two versions, so consumers (anchors,
//! syntax trees, display layers) can update incrementally.

use std::ops::Range;

use sum_tree::Bias;

/// One replaced span: `old` in the previous version's byte offsets, `new` in
/// the next version's byte offsets.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Edit {
    pub old: Range<usize>,
    pub new: Range<usize>,
}

impl Edit {
    pub fn old_len(&self) -> usize {
        self.old.end - self.old.start
    }

    pub fn new_len(&self) -> usize {
        self.new.end - self.new.start
    }

    fn delta(&self) -> isize {
        self.new_len() as isize - self.old_len() as isize
    }
}

/// A sorted list of non-overlapping, non-adjacent [`Edit`]s that transforms
/// one version of the text into another.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Patch(Vec<Edit>);

impl Patch {
    /// Builds a patch from edits sorted by `old.start` that neither overlap
    /// nor touch, with `new` ranges consistent with that order.
    pub fn new(edits: Vec<Edit>) -> Self {
        debug_assert!(
            edits
                .windows(2)
                .all(|w| w[0].old.end < w[1].old.start && w[0].new.end < w[1].new.start),
            "edits must be sorted and disjoint: {edits:?}"
        );
        Self(edits)
    }

    pub fn edits(&self) -> &[Edit] {
        &self.0
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    pub fn into_inner(self) -> Vec<Edit> {
        self.0
    }

    /// Maps an offset in the old version to the new version.
    ///
    /// An offset at an insertion point, or inside or at the start of a
    /// replaced span, goes to the start of the new text with [`Bias::Left`]
    /// and to its end with [`Bias::Right`]. An offset at the end of a
    /// replaced span goes to the end of the new text.
    pub fn transform_offset(&self, offset: usize, bias: Bias) -> usize {
        let ix = self.0.partition_point(|edit| edit.old.end < offset);
        let delta: isize = self.0[..ix].iter().map(Edit::delta).sum();
        match self.0.get(ix) {
            Some(edit) if edit.old.start <= offset => {
                let at_end = offset == edit.old.end && edit.old.start < edit.old.end;
                if at_end || bias == Bias::Right {
                    edit.new.end
                } else {
                    edit.new.start
                }
            }
            _ => (offset as isize + delta) as usize,
        }
    }

    /// The patch equivalent to applying `self` and then `next`.
    pub fn compose(&self, next: &Patch) -> Patch {
        compose(&self.0, &next.0)
    }
}

/// Composes `a` (v0→v1) with `b` (v1→v2) into a v0→v2 patch.
///
/// Works in v1 coordinates: `a`'s new ranges and `b`'s old ranges are merged
/// into groups of overlapping or touching spans; each group becomes one edit
/// whose ends are mapped back through `a` and forward through `b`.
fn compose(a: &[Edit], b: &[Edit]) -> Patch {
    let mut result: Vec<Edit> = Vec::new();
    let (mut ia, mut ib) = (0, 0);
    // Net length change of all `a`/`b` edits strictly before the current group.
    let (mut delta_a, mut delta_b) = (0isize, 0isize);

    while ia < a.len() || ib < b.len() {
        let take_a = match (a.get(ia), b.get(ib)) {
            (Some(ea), Some(eb)) => ea.new.start <= eb.old.start,
            (Some(_), None) => true,
            _ => false,
        };
        let lo = if take_a {
            a[ia].new.start
        } else {
            b[ib].old.start
        };
        let mut hi = lo;
        let (a_first, b_first) = (ia, ib);
        loop {
            if let Some(ea) = a.get(ia)
                && ea.new.start <= hi
            {
                hi = hi.max(ea.new.end);
                ia += 1;
                continue;
            }
            if let Some(eb) = b.get(ib)
                && eb.old.start <= hi
            {
                hi = hi.max(eb.old.end);
                ib += 1;
                continue;
            }
            break;
        }
        let group_a = &a[a_first..ia];
        let group_b = &b[b_first..ib];
        let group_delta_a: isize = group_a.iter().map(Edit::delta).sum();
        let group_delta_b: isize = group_b.iter().map(Edit::delta).sum();

        let old_start = match group_a.first() {
            Some(ea) if ea.new.start == lo => ea.old.start,
            _ => (lo as isize - delta_a) as usize,
        };
        let old_end = match group_a.last() {
            Some(ea) if ea.new.end == hi => ea.old.end,
            _ => (hi as isize - delta_a - group_delta_a) as usize,
        };
        let new_start = match group_b.first() {
            Some(eb) if eb.old.start == lo => eb.new.start,
            _ => (lo as isize + delta_b) as usize,
        };
        let new_end = match group_b.last() {
            Some(eb) if eb.old.end == hi => eb.new.end,
            _ => (hi as isize + delta_b + group_delta_b) as usize,
        };
        delta_a += group_delta_a;
        delta_b += group_delta_b;

        let edit = Edit {
            old: old_start..old_end,
            new: new_start..new_end,
        };
        // Groups were merged in v1 space; they can still touch in v0 or v2
        // space (e.g. a deletion next to an insertion). Coalesce those.
        match result.last_mut() {
            Some(last) if last.old.end >= edit.old.start || last.new.end >= edit.new.start => {
                last.old.end = edit.old.end;
                last.new.end = edit.new.end;
            }
            _ => result.push(edit),
        }
    }
    result.retain(|edit| !(edit.old.is_empty() && edit.new.is_empty()));
    Patch(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    /// Applies random edits to `text`, returning the new text and the patch.
    fn random_edit(text: &str, raw: &[(usize, usize, String)]) -> (String, Patch) {
        // Pick disjoint, non-adjacent ranges in the old text.
        let mut ranges: Vec<(Range<usize>, String)> = Vec::new();
        let mut points: Vec<(usize, usize, &String)> = raw
            .iter()
            .map(|(a, b, s)| {
                let a = a % (text.len() + 1);
                let b = b % (text.len() + 1);
                (a.min(b), a.max(b), s)
            })
            .collect();
        points.sort();
        let mut floor = 0;
        for (start, end, insert) in points {
            if start < floor || (start == end && insert.is_empty()) {
                continue;
            }
            ranges.push((start..end, insert.clone()));
            floor = end + 1;
        }
        let mut new_text = String::new();
        let mut edits = Vec::new();
        let mut prev = 0;
        for (range, insert) in ranges {
            new_text.push_str(&text[prev..range.start]);
            let new_start = new_text.len();
            new_text.push_str(&insert);
            edits.push(Edit {
                old: range.clone(),
                new: new_start..new_text.len(),
            });
            prev = range.end;
        }
        new_text.push_str(&text[prev..]);
        (new_text, Patch::new(edits))
    }

    fn apply(old: &str, new: &str, patch: &Patch) -> String {
        let mut out = String::new();
        let mut prev = 0;
        for edit in patch.edits() {
            out.push_str(&old[prev..edit.old.start]);
            out.push_str(&new[edit.new.clone()]);
            prev = edit.old.end;
        }
        out.push_str(&old[prev..]);
        out
    }

    #[test]
    fn transform_offsets() {
        // "abcdef" -> replace 2..4 with "XYZ", insert "!" at 5.
        let patch = Patch::new(vec![
            Edit {
                old: 2..4,
                new: 2..5,
            },
            Edit {
                old: 5..5,
                new: 6..7,
            },
        ]);
        assert_eq!(patch.transform_offset(1, Bias::Left), 1);
        assert_eq!(patch.transform_offset(2, Bias::Left), 2);
        assert_eq!(patch.transform_offset(2, Bias::Right), 5);
        assert_eq!(patch.transform_offset(3, Bias::Left), 2);
        assert_eq!(patch.transform_offset(4, Bias::Left), 5);
        assert_eq!(patch.transform_offset(5, Bias::Left), 6);
        assert_eq!(patch.transform_offset(5, Bias::Right), 7);
        assert_eq!(patch.transform_offset(6, Bias::Left), 8);
    }

    fn arb_edits() -> impl Strategy<Value = Vec<(usize, usize, String)>> {
        proptest::collection::vec((any::<usize>(), any::<usize>(), "[a-z]{0,4}"), 0..6)
    }

    proptest! {
        #[test]
        fn compose_matches_sequential_application(
            text in "[a-z]{0,30}",
            first in arb_edits(),
            second in arb_edits(),
        ) {
            let (v1, a) = random_edit(&text, &first);
            let (v2, b) = random_edit(&v1, &second);
            let composed = a.compose(&b);
            prop_assert_eq!(apply(&text, &v2, &composed), v2.clone());
            for w in composed.edits().windows(2) {
                prop_assert!(w[0].old.end < w[1].old.start, "{:?}", composed);
                prop_assert!(w[0].new.end < w[1].new.start, "{:?}", composed);
            }
            // Offsets outside every edit map the same way either path.
            for offset in 0..=text.len() {
                if composed.edits().iter().all(|e| offset < e.old.start || offset > e.old.end) {
                    let stepwise = b.transform_offset(a.transform_offset(offset, Bias::Left), Bias::Left);
                    prop_assert_eq!(composed.transform_offset(offset, Bias::Left), stepwise);
                }
            }
        }
    }
}
