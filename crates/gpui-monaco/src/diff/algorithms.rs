//! Sequence diff algorithms (port of Monaco's `diffAlgorithm.ts`,
//! `myersDiffAlgorithm.ts` and `dynamicProgrammingDiffing.ts`).

use std::cell::Cell;
use std::time::Duration;

use web_time::Instant;

use super::range::OffsetRange;

/// A sequence of comparable elements (Monaco's `ISequence`).
pub(crate) trait Sequence {
    /// The element at `offset` (a line hash or a UTF-16 code unit).
    fn element(&self, offset: usize) -> u32;

    /// The number of elements.
    fn len(&self) -> usize;

    /// How good a split point the position before `length` is (higher is
    /// better, never negative). Used to shift insertions/deletions.
    fn boundary_score(&self, length: usize) -> i64;

    /// Equality on the underlying (e.g. untrimmed) data.
    fn is_strongly_equal(&self, offset1: usize, offset2: usize) -> bool;
}

/// A pair of offsets into the two sequences.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct OffsetPair {
    pub(crate) offset1: usize,
    pub(crate) offset2: usize,
}

impl OffsetPair {
    pub(crate) const ZERO: Self = Self::new(0, 0);
    pub(crate) const MAX: Self = Self::new(usize::MAX, usize::MAX);

    pub(crate) const fn new(offset1: usize, offset2: usize) -> Self {
        Self { offset1, offset2 }
    }

    pub(crate) fn delta(self, offset: isize) -> Self {
        Self::new(
            self.offset1.saturating_add_signed(offset),
            self.offset2.saturating_add_signed(offset),
        )
    }
}

/// A changed region: `seq1_range` in the first sequence is replaced by
/// `seq2_range` in the second (Monaco's `SequenceDiff`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct SequenceDiff {
    pub(crate) seq1_range: OffsetRange,
    pub(crate) seq2_range: OffsetRange,
}

impl SequenceDiff {
    pub(crate) fn new(seq1_range: OffsetRange, seq2_range: OffsetRange) -> Self {
        Self {
            seq1_range,
            seq2_range,
        }
    }

    /// The equal regions between `diffs` (Monaco's `SequenceDiff.invert`).
    pub(crate) fn invert(diffs: &[Self], doc1_length: usize) -> Vec<Self> {
        let mut result = Vec::with_capacity(diffs.len() + 1);
        for i in 0..=diffs.len() {
            let a = i.checked_sub(1).map(|i| diffs[i]);
            let b = diffs.get(i);
            let start = a.map_or(OffsetPair::ZERO, |a| a.end_exclusives());
            let end = match b {
                Some(b) => b.starts(),
                None => {
                    let shift = a.map_or(0, |a| {
                        a.seq2_range.end_exclusive as isize - a.seq1_range.end_exclusive as isize
                    });
                    OffsetPair::new(doc1_length, doc1_length.saturating_add_signed(shift))
                }
            };
            result.push(Self::from_offset_pairs(start, end));
        }
        result
    }

    pub(crate) fn from_offset_pairs(start: OffsetPair, end_exclusive: OffsetPair) -> Self {
        Self::new(
            OffsetRange::new(start.offset1, end_exclusive.offset1),
            OffsetRange::new(start.offset2, end_exclusive.offset2),
        )
    }

    pub(crate) fn swap(self) -> Self {
        Self::new(self.seq2_range, self.seq1_range)
    }

    pub(crate) fn join(&self, other: &Self) -> Self {
        Self::new(
            self.seq1_range.join(&other.seq1_range),
            self.seq2_range.join(&other.seq2_range),
        )
    }

    pub(crate) fn delta(self, offset: isize) -> Self {
        if offset == 0 {
            return self;
        }
        Self::new(self.seq1_range.delta(offset), self.seq2_range.delta(offset))
    }

    pub(crate) fn delta_start(self, offset: isize) -> Self {
        if offset == 0 {
            return self;
        }
        Self::new(
            self.seq1_range.delta_start(offset),
            self.seq2_range.delta_start(offset),
        )
    }

    pub(crate) fn delta_end(self, offset: isize) -> Self {
        if offset == 0 {
            return self;
        }
        Self::new(
            self.seq1_range.delta_end(offset),
            self.seq2_range.delta_end(offset),
        )
    }

    pub(crate) fn intersect(&self, other: &Self) -> Option<Self> {
        let i1 = self.seq1_range.intersect(&other.seq1_range)?;
        let i2 = self.seq2_range.intersect(&other.seq2_range)?;
        Some(Self::new(i1, i2))
    }

    pub(crate) fn starts(&self) -> OffsetPair {
        OffsetPair::new(self.seq1_range.start, self.seq2_range.start)
    }

    pub(crate) fn end_exclusives(&self) -> OffsetPair {
        OffsetPair::new(self.seq1_range.end_exclusive, self.seq2_range.end_exclusive)
    }
}

/// A computation deadline (Monaco's `ITimeout`: `InfiniteTimeout` or
/// `DateTimeout`). Once it has expired it stays expired.
pub(crate) struct Timeout {
    deadline: Option<Instant>,
    valid: Cell<bool>,
    /// Calls since the clock was last read: reading it is comparatively
    /// expensive (especially on wasm) and some loops check per element.
    calls: Cell<u32>,
}

impl Timeout {
    /// How many `is_valid` calls share one clock reading.
    const CALLS_PER_CLOCK_READ: u32 = 64;

    /// A timeout of `ms` milliseconds from now; 0 means no limit.
    pub(crate) fn new(ms: u64) -> Self {
        let deadline = if ms == 0 {
            None
        } else {
            // A deadline too far in the future to represent is no limit.
            Instant::now().checked_add(Duration::from_millis(ms))
        };
        Self {
            deadline,
            valid: Cell::new(true),
            calls: Cell::new(0),
        }
    }

    /// A timeout that never expires.
    #[cfg(test)]
    pub(crate) fn infinite() -> Self {
        Self::new(0)
    }

    /// Whether there is still time left.
    pub(crate) fn is_valid(&self) -> bool {
        let Some(deadline) = self.deadline else {
            return true;
        };
        if !self.valid.get() {
            return false;
        }
        let calls = self.calls.get();
        self.calls.set(calls.wrapping_add(1));
        if calls.is_multiple_of(Self::CALLS_PER_CLOCK_READ) && Instant::now() >= deadline {
            self.valid.set(false);
        }
        self.valid.get()
    }
}

/// The result of a diff algorithm.
pub(crate) struct DiffAlgorithmResult {
    pub(crate) diffs: Vec<SequenceDiff>,
    /// Whether the timeout was reached; the diffs are then an approximation.
    pub(crate) hit_timeout: bool,
}

impl DiffAlgorithmResult {
    fn trivial(seq1: &impl Sequence, seq2: &impl Sequence, hit_timeout: bool) -> Self {
        Self {
            diffs: vec![SequenceDiff::new(
                OffsetRange::of_length(seq1.len()),
                OffsetRange::of_length(seq2.len()),
            )],
            hit_timeout,
        }
    }
}

/// A vector indexable by negative indices (Monaco's `FastInt32Array` /
/// `FastArrayNegativeIndices`), reading `default` for unset slots.
struct NegVec<T: Copy> {
    positive: Vec<T>,
    negative: Vec<T>,
    default: T,
}

impl<T: Copy> NegVec<T> {
    fn new(default: T) -> Self {
        Self {
            positive: Vec::new(),
            negative: Vec::new(),
            default,
        }
    }

    fn slot(&mut self, idx: isize) -> &mut T {
        let (vec, i) = if idx < 0 {
            (&mut self.negative, (-idx - 1) as usize)
        } else {
            (&mut self.positive, idx as usize)
        };
        if i >= vec.len() {
            vec.resize((i + 1).max(vec.len() * 2), self.default);
        }
        &mut vec[i]
    }

    fn get(&self, idx: isize) -> T {
        let (vec, i) = if idx < 0 {
            (&self.negative, (-idx - 1) as usize)
        } else {
            (&self.positive, idx as usize)
        };
        vec.get(i).copied().unwrap_or(self.default)
    }

    fn set(&mut self, idx: isize, value: T) {
        *self.slot(idx) = value;
    }
}

struct SnakePath {
    prev: Option<usize>,
    x: isize,
    y: isize,
    length: isize,
}

/// An O(ND) diff (Myers) with quadratic worst-case space (Monaco's
/// `MyersDiffAlgorithm`).
pub(crate) fn myers_diff(
    seq_x: &impl Sequence,
    seq_y: &impl Sequence,
    timeout: &Timeout,
) -> DiffAlgorithmResult {
    // These are common special cases; the early return improves performance
    // dramatically.
    if seq_x.len() == 0 || seq_y.len() == 0 {
        return DiffAlgorithmResult::trivial(seq_x, seq_y, false);
    }
    let len_x = seq_x.len() as isize;
    let len_y = seq_y.len() as isize;

    let x_after_snake = |mut x: isize, mut y: isize| -> isize {
        // A negative `y` never matches (Monaco reads `undefined`).
        if y < 0 {
            return x;
        }
        while x < len_x && y < len_y && seq_x.element(x as usize) == seq_y.element(y as usize) {
            x += 1;
            y += 1;
        }
        x
    };

    // v[k]: X value of the longest d-line that ends in diagonal k.
    let mut v = NegVec::new(0isize);
    v.set(0, x_after_snake(0, 0));

    let mut snakes: Vec<SnakePath> = Vec::new();
    let mut paths = NegVec::<Option<usize>>::new(None);
    if v.get(0) != 0 {
        snakes.push(SnakePath {
            prev: None,
            x: 0,
            y: 0,
            length: v.get(0),
        });
        paths.set(0, Some(0));
    }

    let mut d: isize = 0;
    let mut k: isize;
    'outer: loop {
        d += 1;
        if !timeout.is_valid() {
            return DiffAlgorithmResult::trivial(seq_x, seq_y, true);
        }
        // The paper has `for (k = -d; k <= d; k += 2)`, but diagonals that
        // cannot influence the result can be ignored.
        let lower_bound = -d.min(len_y + d % 2);
        let upper_bound = d.min(len_x + d % 2);
        k = lower_bound;
        while k <= upper_bound {
            // Vertical non-diagonal (add a symbol in seq_x) / horizontal (delete).
            let max_x_top = if k == upper_bound { -1 } else { v.get(k + 1) };
            let max_x_left = if k == lower_bound {
                -1
            } else {
                v.get(k - 1) + 1
            };
            let x = max_x_top.max(max_x_left).min(len_x);
            let y = x - k;
            if x > len_x || y > len_y {
                // This diagonal is irrelevant for the result.
                k += 2;
                continue;
            }
            let new_max_x = x_after_snake(x, y);
            v.set(k, new_max_x);
            let last_path = if x == max_x_top {
                paths.get(k + 1)
            } else {
                paths.get(k - 1)
            };
            let path = if new_max_x != x {
                snakes.push(SnakePath {
                    prev: last_path,
                    x,
                    y,
                    length: new_max_x - x,
                });
                Some(snakes.len() - 1)
            } else {
                last_path
            };
            paths.set(k, path);

            if new_max_x == len_x && new_max_x - k == len_y {
                break 'outer;
            }
            k += 2;
        }
    }

    let mut path = paths.get(k);
    let mut result = Vec::new();
    let mut last_pos1 = len_x;
    let mut last_pos2 = len_y;
    loop {
        let (end_x, end_y) = path.map_or((0, 0), |p| {
            let p = &snakes[p];
            (p.x + p.length, p.y + p.length)
        });
        if end_x != last_pos1 || end_y != last_pos2 {
            result.push(SequenceDiff::new(
                OffsetRange::new(end_x as usize, last_pos1 as usize),
                OffsetRange::new(end_y as usize, last_pos2 as usize),
            ));
        }
        let Some(p) = path else { break };
        let p = &snakes[p];
        last_pos1 = p.x;
        last_pos2 = p.y;
        path = p.prev;
    }
    result.reverse();
    DiffAlgorithmResult {
        diffs: result,
        hit_timeout: false,
    }
}

/// An O(MN) diff supporting a score function for equal elements (Monaco's
/// `DynamicProgrammingDiffing`).
pub(crate) fn dynamic_programming_diff(
    seq1: &impl Sequence,
    seq2: &impl Sequence,
    timeout: &Timeout,
    equality_score: Option<&dyn Fn(usize, usize) -> f64>,
) -> DiffAlgorithmResult {
    const DIAGONAL: u8 = 3;
    const HORIZONTAL: u8 = 1;
    const VERTICAL: u8 = 2;

    let len1 = seq1.len();
    let len2 = seq2.len();
    if len1 == 0 || len2 == 0 {
        return DiffAlgorithmResult::trivial(seq1, seq2, false);
    }

    // lcs_lengths[(s1, s2)]: score of the best alignment of
    // seq1[..=s1] and seq2[..=s2].
    let idx = |s1: usize, s2: usize| s1 + s2 * len1;
    let mut lcs_lengths = vec![0f64; len1 * len2];
    let mut directions = vec![0u8; len1 * len2];
    let mut lengths = vec![0u32; len1 * len2];

    for s1 in 0..len1 {
        for s2 in 0..len2 {
            if !timeout.is_valid() {
                return DiffAlgorithmResult::trivial(seq1, seq2, true);
            }

            let horizontal_len = if s1 == 0 {
                0.0
            } else {
                lcs_lengths[idx(s1 - 1, s2)]
            };
            let vertical_len = if s2 == 0 {
                0.0
            } else {
                lcs_lengths[idx(s1, s2 - 1)]
            };

            let extended_seq_score = if seq1.element(s1) == seq2.element(s2) {
                let mut score = if s1 == 0 || s2 == 0 {
                    0.0
                } else {
                    lcs_lengths[idx(s1 - 1, s2 - 1)]
                };
                if s1 > 0 && s2 > 0 && directions[idx(s1 - 1, s2 - 1)] == DIAGONAL {
                    // Prefer consecutive diagonals.
                    score += f64::from(lengths[idx(s1 - 1, s2 - 1)]);
                }
                score + equality_score.map_or(1.0, |f| f(s1, s2))
            } else {
                -1.0
            };

            let new_value = horizontal_len.max(vertical_len).max(extended_seq_score);
            let i = idx(s1, s2);
            if new_value == extended_seq_score {
                // Prefer diagonals.
                let prev_len = if s1 > 0 && s2 > 0 {
                    lengths[idx(s1 - 1, s2 - 1)]
                } else {
                    0
                };
                lengths[i] = prev_len + 1;
                directions[i] = DIAGONAL;
            } else if new_value == horizontal_len {
                lengths[i] = 0;
                directions[i] = HORIZONTAL;
            } else {
                lengths[i] = 0;
                directions[i] = VERTICAL;
            }
            lcs_lengths[i] = new_value;
        }
    }

    // Backtracking.
    let mut result = Vec::new();
    let mut last_pos1 = len1;
    let mut last_pos2 = len2;
    // Reports the aligned pair *before* (s1, s2), i.e. (s1 - 1, s2 - 1) in
    // Monaco's terms, so the offsets stay unsigned.
    let mut report = |s1: usize, s2: usize, result: &mut Vec<SequenceDiff>| {
        if s1 != last_pos1 || s2 != last_pos2 {
            result.push(SequenceDiff::new(
                OffsetRange::new(s1, last_pos1),
                OffsetRange::new(s2, last_pos2),
            ));
        }
        (last_pos1, last_pos2) = (s1.saturating_sub(1), s2.saturating_sub(1));
    };

    let (mut s1, mut s2) = (len1, len2);
    while s1 > 0 && s2 > 0 {
        match directions[idx(s1 - 1, s2 - 1)] {
            DIAGONAL => {
                report(s1, s2, &mut result);
                s1 -= 1;
                s2 -= 1;
            }
            HORIZONTAL => s1 -= 1,
            _ => s2 -= 1,
        }
    }
    report(0, 0, &mut result);
    result.reverse();
    DiffAlgorithmResult {
        diffs: result,
        hit_timeout: false,
    }
}
