//! [`Rows`]: [`ViewLines`](super::view_lines::ViewLines)' per-row data (each
//! model row's projection and view line count) in shared chunks.
//!
//! The rows are split into chunks of about [`CHUNK_ROWS`] rows behind `Arc`s,
//! with a prefix sum over each chunk's counts and the running total before
//! each chunk. Cloning copies only the chunk list, and changing a row copies only
//! its chunk if another clone still shares it, so a mapping kept by a view
//! (the minimap's tiles) doesn't make each edit copy the whole document.

use std::ops::Range;
use std::sync::Arc;

use super::injected_text::ModelLineProjectionData;
use super::prefix_sum::PrefixSum;

/// A row's projection; `None` when it shows as is, on one view line.
pub(crate) type Projection = Option<Arc<ModelLineProjectionData>>;

/// About how many rows a chunk holds. Chunks hold up to twice this; edits
/// split larger ones and merge small ones into their neighbor.
const CHUNK_ROWS: usize = 1024;

#[derive(Clone, Default)]
struct Chunk {
    projections: Vec<Projection>,
    counts: PrefixSum,
}

impl Chunk {
    fn new(projections: Vec<Projection>, counts: Vec<u32>) -> Arc<Self> {
        Arc::new(Self {
            projections,
            counts: PrefixSum::new(counts),
        })
    }

    fn len(&self) -> usize {
        self.projections.len()
    }
}

/// Each row's projection and view line count, with `O(log n)` prefix sums
/// of the counts and cheap clones (see the module docs).
#[derive(Clone, Default)]
pub(crate) struct Rows {
    chunks: Vec<Arc<Chunk>>,
    /// The first row of each chunk.
    starts: Vec<usize>,
    /// The total count before each chunk, and after the last.
    totals: Vec<u64>,
    len: usize,
}

/// `projections` and their `counts` in chunks of about [`CHUNK_ROWS`].
fn into_chunks(projections: Vec<Projection>, counts: Vec<u32>) -> Vec<Arc<Chunk>> {
    let len = projections.len();
    if len == 0 {
        return Vec::new();
    }
    if len <= 2 * CHUNK_ROWS {
        return vec![Chunk::new(projections, counts)];
    }
    let size = len.div_ceil(len.div_ceil(CHUNK_ROWS));
    projections
        .chunks(size)
        .zip(counts.chunks(size))
        .map(|(projections, counts)| Chunk::new(projections.to_vec(), counts.to_vec()))
        .collect()
}

impl Rows {
    /// `len` rows shown as is, on one view line each. The full chunks are
    /// one shared chunk until edited.
    pub(crate) fn plain(len: usize) -> Self {
        let full = Chunk::new(vec![None; CHUNK_ROWS], vec![1; CHUNK_ROWS]);
        let mut chunks = vec![full; len / CHUNK_ROWS];
        let rest = len % CHUNK_ROWS;
        if rest > 0 {
            chunks.push(Chunk::new(vec![None; rest], vec![1; rest]));
        }
        let mut this = Self {
            chunks,
            ..Self::default()
        };
        this.reindex();
        this
    }

    /// Rows with these projections and counts.
    pub(crate) fn from_iter(rows: impl IntoIterator<Item = (Projection, u32)>) -> Self {
        let mut chunks = Vec::new();
        let (mut projections, mut counts) = (Vec::new(), Vec::new());
        for (projection, count) in rows {
            if projections.is_empty() {
                projections.reserve(CHUNK_ROWS);
                counts.reserve(CHUNK_ROWS);
            }
            projections.push(projection);
            counts.push(count);
            if projections.len() == CHUNK_ROWS {
                chunks.push(Chunk::new(
                    std::mem::take(&mut projections),
                    std::mem::take(&mut counts),
                ));
            }
        }
        if !projections.is_empty() {
            chunks.push(Chunk::new(projections, counts));
        }
        let mut this = Self {
            chunks,
            ..Self::default()
        };
        this.reindex();
        this
    }

    /// Recomputes the chunk starts and totals after the chunks changed.
    fn reindex(&mut self) {
        self.starts.clear();
        let mut start = 0;
        for chunk in &self.chunks {
            self.starts.push(start);
            start += chunk.len();
        }
        self.len = start;
        self.totals.clear();
        let mut total = 0;
        self.totals.push(0);
        for chunk in &self.chunks {
            total += chunk.counts.total();
            self.totals.push(total);
        }
    }

    /// The chunk holding `row` (`row < len`) and `row`'s index in it.
    fn locate(&self, row: usize) -> (usize, usize) {
        let ix = self.starts.partition_point(|start| *start <= row) - 1;
        (ix, row - self.starts[ix])
    }

    /// Number of rows.
    pub(crate) fn len(&self) -> usize {
        self.len
    }

    /// `row`'s projection, or `None` past the end.
    pub(crate) fn get(&self, row: usize) -> Option<&Projection> {
        if row >= self.len {
            return None;
        }
        let (ix, at) = self.locate(row);
        Some(&self.chunks[ix].projections[at])
    }

    /// `row`'s count. Panics if out of bounds.
    pub(crate) fn count(&self, row: usize) -> u32 {
        assert!(row < self.len, "row {row} out of bounds");
        let (ix, at) = self.locate(row);
        self.chunks[ix].counts.value(at)
    }

    /// [`Rows::prefix`] and [`Rows::count`] of `row`. Panics if out of
    /// bounds.
    pub(crate) fn prefix_and_count(&self, row: usize) -> (u64, u32) {
        let (prefix, count, _) = self.row(row);
        (prefix, count)
    }

    /// [`Rows::prefix`], [`Rows::count`] and the projection of `row`, found
    /// once. Panics if out of bounds.
    pub(crate) fn row(&self, row: usize) -> (u64, u32, &Projection) {
        assert!(row < self.len, "row {row} out of bounds");
        let (ix, at) = self.locate(row);
        let chunk = &self.chunks[ix];
        (
            self.totals[ix] + chunk.counts.prefix(at),
            chunk.counts.value(at),
            &chunk.projections[at],
        )
    }

    /// Sets each `(row, projection, count)`. Panics if a row is out of
    /// bounds.
    pub(crate) fn set_many(&mut self, rows: impl IntoIterator<Item = (usize, Projection, u32)>) {
        let mut first_changed = None;
        for (row, projection, count) in rows {
            assert!(row < self.len, "row {row} out of bounds");
            let (ix, at) = self.locate(row);
            let chunk = Arc::make_mut(&mut self.chunks[ix]);
            chunk.projections[at] = projection;
            if chunk.counts.value(at) != count {
                chunk.counts.set(at, count);
                first_changed = Some(first_changed.map_or(ix, |first: usize| first.min(ix)));
            }
        }
        if let Some(first) = first_changed {
            self.retotal_from(first);
        }
    }

    /// Recomputes the running totals from chunk `first` on.
    fn retotal_from(&mut self, first: usize) {
        let mut total = self.totals[first];
        for (chunk, after) in self.chunks[first..]
            .iter()
            .zip(&mut self.totals[first + 1..])
        {
            total += chunk.counts.total();
            *after = total;
        }
    }

    /// Sum of all counts.
    pub(crate) fn total(&self) -> u64 {
        self.totals.last().copied().unwrap_or(0)
    }

    /// Sum of the counts of the rows before `row` (all of them past the
    /// end).
    pub(crate) fn prefix(&self, row: usize) -> u64 {
        if row >= self.len {
            return self.total();
        }
        let (ix, at) = self.locate(row);
        self.totals[ix] + self.chunks[ix].counts.prefix(at)
    }

    /// The row whose span of counts contains `position` (0-based within the
    /// total), and the remainder within it. Positions past the end clamp to
    /// the last row with a count ([`PrefixSum::find`]).
    pub(crate) fn find(&self, position: u64) -> (usize, u32) {
        let total = self.total();
        if total == 0 {
            return (0, 0);
        }
        let position = position.min(total - 1);
        // The first chunk ending after it (so not an empty one).
        let ix = self.totals[1..].partition_point(|end| *end <= position);
        let (at, rest) = self.chunks[ix].counts.find(position - self.totals[ix]);
        (self.starts[ix] + at, rest)
    }

    /// The projections of `rows` (clamped to the end), in order.
    pub(crate) fn projections(&self, rows: Range<usize>) -> impl Iterator<Item = &Projection> {
        let end = rows.end.min(self.len);
        let start = rows.start.min(end);
        let first = if start < end {
            self.locate(start).0
        } else {
            self.chunks.len()
        };
        self.chunks[first..]
            .iter()
            .zip(&self.starts[first..])
            .take_while(move |(_, chunk_start)| **chunk_start < end)
            .flat_map(move |(chunk, chunk_start)| {
                let from = start.saturating_sub(*chunk_start);
                let to = (end - chunk_start).min(chunk.len());
                &chunk.projections[from..to]
            })
    }

    /// Sets the count of each of `rows` (clamped to the end) to `count` of
    /// it and its projection, copying only the chunks they're in.
    pub(crate) fn recount(
        &mut self,
        rows: Range<usize>,
        mut count: impl FnMut(usize, &Projection) -> u32,
    ) {
        let end = rows.end.min(self.len);
        if rows.start >= end {
            return;
        }
        let (first, _) = self.locate(rows.start);
        let mut ix = first;
        while ix < self.chunks.len() && self.starts[ix] < end {
            let start = self.starts[ix];
            let chunk = Arc::make_mut(&mut self.chunks[ix]);
            let from = rows.start.max(start) - start;
            let to = (end - start).min(chunk.len());
            if to - from <= 16 {
                for at in from..to {
                    let count = count(start + at, &chunk.projections[at]);
                    if chunk.counts.value(at) != count {
                        chunk.counts.set(at, count);
                    }
                }
            } else {
                let counts = (0..chunk.len())
                    .map(|at| {
                        if (from..to).contains(&at) {
                            count(start + at, &chunk.projections[at])
                        } else {
                            chunk.counts.value(at)
                        }
                    })
                    .collect();
                chunk.counts = PrefixSum::new(counts);
            }
            ix += 1;
        }
        self.retotal_from(first);
    }

    /// Replaces `range` (clamped to the end) with `rows`, copying only the
    /// chunks at its ends.
    pub(crate) fn splice(&mut self, range: Range<usize>, rows: Vec<(Projection, u32)>) {
        let end = range.end.min(self.len);
        let start = range.start.min(end);
        if self.chunks.is_empty() {
            *self = Self::from_iter(rows);
            return;
        }
        let first = if start == self.len {
            self.chunks.len() - 1
        } else {
            self.locate(start).0
        };
        let last = if end > start {
            self.locate(end - 1).0
        } else {
            first
        };
        let (head, tail) = (&self.chunks[first], &self.chunks[last]);
        let head_len = start - self.starts[first];
        let tail_from = (end - self.starts[last]).max(if last == first { head_len } else { 0 });
        let mut projections =
            Vec::with_capacity(head_len + rows.len() + tail.len() - tail_from + CHUNK_ROWS);
        let mut counts = Vec::with_capacity(projections.capacity());
        projections.extend_from_slice(&head.projections[..head_len]);
        counts.extend((0..head_len).map(|at| head.counts.value(at)));
        for (projection, count) in rows {
            projections.push(projection);
            counts.push(count);
        }
        projections.extend_from_slice(&tail.projections[tail_from..]);
        counts.extend((tail_from..tail.len()).map(|at| tail.counts.value(at)));
        let mut replaced = first..last + 1;
        // Keep chunks from getting small: merge with the next one.
        if projections.len() < CHUNK_ROWS / 2 && replaced.end < self.chunks.len() {
            let next = &self.chunks[replaced.end];
            projections.extend_from_slice(&next.projections);
            counts.extend((0..next.len()).map(|at| next.counts.value(at)));
            replaced.end += 1;
        }
        self.chunks
            .splice(replaced, into_chunks(projections, counts));
        self.reindex();
    }

    /// How many chunks `self` shares with `other` (for tests).
    #[cfg(test)]
    pub(crate) fn shared_chunks(&self, other: &Self) -> (usize, usize) {
        let shared = self
            .chunks
            .iter()
            .filter(|chunk| other.chunks.iter().any(|o| Arc::ptr_eq(chunk, o)))
            .count();
        (shared, self.chunks.len())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rows(counts: &[u32]) -> Rows {
        Rows::from_iter(counts.iter().map(|count| (None, *count)))
    }

    fn check(rows: &Rows, expected: &[u32]) {
        let flat = PrefixSum::new(expected.to_vec());
        assert_eq!(rows.len(), expected.len());
        assert_eq!(rows.total(), flat.total());
        for (row, count) in expected.iter().enumerate() {
            assert_eq!(rows.count(row), *count);
            assert_eq!(rows.prefix(row), flat.prefix(row));
            assert_eq!(rows.prefix_and_count(row), (flat.prefix(row), *count));
        }
        assert_eq!(rows.prefix(expected.len() + 3), flat.total());
        for position in 0..flat.total() + 2 {
            assert_eq!(rows.find(position), flat.find(position), "at {position}");
        }
        assert_eq!(
            rows.projections(0..expected.len() + 5).count(),
            expected.len()
        );
    }

    fn pattern(len: usize, seed: usize) -> Vec<u32> {
        (0..len).map(|i| ((i * 7 + seed) % 5) as u32).collect()
    }

    #[test]
    fn matches_a_flat_prefix_sum_through_splices() {
        let mut expected = pattern(5000, 0);
        let mut table = rows(&expected);
        check(&table, &expected);
        let splices: [(Range<usize>, usize); 8] = [
            (10..11, 3),
            (1023..1025, 1),
            (0..4000, 2),
            (2..3, 3000),
            (0..1, 0),
            (100..100, 7),
            (expected.len()..expected.len(), 4),
            (500..1500, 0),
        ];
        for (seed, (range, new_len)) in splices.into_iter().enumerate() {
            let range = range.start.min(expected.len())..range.end.min(expected.len());
            let new = pattern(new_len, seed + 1);
            expected.splice(range.clone(), new.iter().copied());
            table.splice(range, new.iter().map(|count| (None, *count)).collect());
            check(&table, &expected);
        }
        expected[7] = 9;
        table.set_many([(7, None, 9)]);
        table.recount(1000..1200, |row, _| row as u32 % 3);
        for (row, count) in expected.iter_mut().enumerate().take(1200).skip(1000) {
            *count = row as u32 % 3;
        }
        check(&table, &expected);
    }

    #[test]
    fn plain_rows() {
        for len in [1, 1024, 1025, 3000] {
            let table = Rows::plain(len);
            check(&table, &vec![1; len]);
            assert!(table.projections(0..len).all(Option::is_none));
        }
    }

    #[test]
    fn all_zero_counts() {
        let table = rows(&[0, 0, 0]);
        assert_eq!(table.find(0), (0, 0));
        assert_eq!(table.total(), 0);
    }

    #[test]
    fn edits_copy_only_their_chunk() {
        let table = rows(&vec![1; 100_000]);
        let mut edited = table.clone();
        edited.set_many([(50_000, None, 2)]);
        edited.splice(70_000..70_001, vec![(None, 1), (None, 1)]);
        let (shared, chunks) = edited.shared_chunks(&table);
        assert!(chunks > 50);
        assert!(shared >= chunks - 2, "{shared} of {chunks} shared");
    }
}
