//! Positions and ranges used by the diff computer.
//!
//! Ports of Monaco's `Position`, `Range`, `LineRange`, `LineRangeSet` and
//! `OffsetRange` (`vs/editor/common/core`), keeping Monaco's 1-based
//! coordinates so diff results match Monaco exactly.

use std::fmt;

use text::PointUtf16;

/// Saturating `usize` to `u32` conversion for [`PointUtf16`] coordinates.
fn to_u32(value: usize) -> u32 {
    u32::try_from(value).unwrap_or(u32::MAX)
}

/// A position in a text, like Monaco's `Position`.
///
/// Both coordinates are 1-based: `line_number` 1 is the first line and
/// `column` 1 is before the first character. Columns count UTF-16 code units.
/// Use [`Position::to_point_utf16`] to convert to the 0-based [`PointUtf16`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Position {
    /// The 1-based line number.
    pub line_number: usize,
    /// The 1-based column, in UTF-16 code units.
    pub column: usize,
}

impl Position {
    /// Creates a position from a 1-based line number and 1-based UTF-16 column.
    pub const fn new(line_number: usize, column: usize) -> Self {
        Self {
            line_number,
            column,
        }
    }

    /// Whether `self` is strictly before `other`.
    pub fn is_before(self, other: Self) -> bool {
        self < other
    }

    /// Whether `self` is before or equal to `other`.
    pub fn is_before_or_equal(self, other: Self) -> bool {
        self <= other
    }

    /// Converts to a 0-based [`PointUtf16`] (saturating at `u32::MAX`; a
    /// coordinate of 0 maps to 0).
    pub fn to_point_utf16(self) -> PointUtf16 {
        PointUtf16::new(
            to_u32(self.line_number.saturating_sub(1)),
            to_u32(self.column.saturating_sub(1)),
        )
    }

    /// Converts from a 0-based [`PointUtf16`].
    pub fn from_point_utf16(point: PointUtf16) -> Self {
        Self::new(point.row as usize + 1, point.column as usize + 1)
    }
}

impl fmt::Display for Position {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "({},{})", self.line_number, self.column)
    }
}

/// A range in a text, like Monaco's `Range`: from a start [`Position`] to an
/// end [`Position`] (exclusive), with 1-based line numbers and 1-based UTF-16
/// columns.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Range {
    /// The 1-based line number of the start.
    pub start_line_number: usize,
    /// The 1-based UTF-16 column of the start.
    pub start_column: usize,
    /// The 1-based line number of the end.
    pub end_line_number: usize,
    /// The 1-based UTF-16 column of the end.
    pub end_column: usize,
}

impl Range {
    /// Creates a range. Like Monaco, swaps the endpoints if the start is after
    /// the end.
    pub fn new(
        start_line_number: usize,
        start_column: usize,
        end_line_number: usize,
        end_column: usize,
    ) -> Self {
        if start_line_number > end_line_number
            || (start_line_number == end_line_number && start_column > end_column)
        {
            Self {
                start_line_number: end_line_number,
                start_column: end_column,
                end_line_number: start_line_number,
                end_column: start_column,
            }
        } else {
            Self {
                start_line_number,
                start_column,
                end_line_number,
                end_column,
            }
        }
    }

    /// Creates a range between two positions (swapped if out of order).
    pub fn from_positions(start: Position, end: Position) -> Self {
        Self::new(start.line_number, start.column, end.line_number, end.column)
    }

    /// The start position.
    pub fn start(&self) -> Position {
        Position::new(self.start_line_number, self.start_column)
    }

    /// The end position.
    pub fn end(&self) -> Position {
        Position::new(self.end_line_number, self.end_column)
    }

    /// Whether the range is empty (start equals end).
    pub fn is_empty(&self) -> bool {
        self.start() == self.end()
    }

    /// The smallest range containing both `self` and `other` (Monaco's
    /// `plusRange`).
    pub fn plus_range(&self, other: &Self) -> Self {
        let (start_line_number, start_column) = if other.start_line_number < self.start_line_number
        {
            (other.start_line_number, other.start_column)
        } else if other.start_line_number == self.start_line_number {
            (
                other.start_line_number,
                other.start_column.min(self.start_column),
            )
        } else {
            (self.start_line_number, self.start_column)
        };
        let (end_line_number, end_column) = if other.end_line_number > self.end_line_number {
            (other.end_line_number, other.end_column)
        } else if other.end_line_number == self.end_line_number {
            (other.end_line_number, other.end_column.max(self.end_column))
        } else {
            (self.end_line_number, self.end_column)
        };
        Self {
            start_line_number,
            start_column,
            end_line_number,
            end_column,
        }
    }

    /// Converts to a range of 0-based [`PointUtf16`]s.
    pub fn to_point_utf16_range(&self) -> std::ops::Range<PointUtf16> {
        self.start().to_point_utf16()..self.end().to_point_utf16()
    }

    /// Converts from a range of 0-based [`PointUtf16`]s.
    pub fn from_point_utf16_range(range: std::ops::Range<PointUtf16>) -> Self {
        Self::from_positions(
            Position::from_point_utf16(range.start),
            Position::from_point_utf16(range.end),
        )
    }
}

impl fmt::Display for Range {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "[{},{} -> {},{}]",
            self.start_line_number, self.start_column, self.end_line_number, self.end_column
        )
    }
}

/// A range of whole lines, like Monaco's `LineRange`: 1-based
/// `start_line_number` up to `end_line_number_exclusive`. An empty range
/// (`start == end`) denotes the position between two lines.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct LineRange {
    /// The first line (1-based).
    pub start_line_number: usize,
    /// The line after the last line (1-based, exclusive).
    pub end_line_number_exclusive: usize,
}

impl LineRange {
    /// Creates a line range. `start_line_number` must not exceed
    /// `end_line_number_exclusive`.
    pub fn new(start_line_number: usize, end_line_number_exclusive: usize) -> Self {
        debug_assert!(
            start_line_number <= end_line_number_exclusive,
            "startLineNumber {start_line_number} cannot be after endLineNumberExclusive {end_line_number_exclusive}"
        );
        Self {
            start_line_number,
            end_line_number_exclusive,
        }
    }

    /// The lines touched by `range` (start line through end line).
    pub fn from_range_inclusive(range: &Range) -> Self {
        Self::new(range.start_line_number, range.end_line_number + 1)
    }

    /// Whether the range contains no lines.
    pub fn is_empty(&self) -> bool {
        self.start_line_number == self.end_line_number_exclusive
    }

    /// The number of lines in the range.
    pub fn len(&self) -> usize {
        self.end_line_number_exclusive - self.start_line_number
    }

    /// Whether `line_number` is in the range.
    pub fn contains(&self, line_number: usize) -> bool {
        self.start_line_number <= line_number && line_number < self.end_line_number_exclusive
    }

    /// Moves the range by `offset` lines.
    pub fn delta(&self, offset: isize) -> Self {
        Self::new(
            self.start_line_number.saturating_add_signed(offset),
            self.end_line_number_exclusive.saturating_add_signed(offset),
        )
    }

    /// The smallest line range containing both ranges.
    pub fn join(&self, other: &Self) -> Self {
        Self::new(
            self.start_line_number.min(other.start_line_number),
            self.end_line_number_exclusive
                .max(other.end_line_number_exclusive),
        )
    }

    /// The intersection; empty if the ranges only touch, `None` if they are
    /// disjoint.
    pub fn intersect(&self, other: &Self) -> Option<Self> {
        let start = self.start_line_number.max(other.start_line_number);
        let end = self
            .end_line_number_exclusive
            .min(other.end_line_number_exclusive);
        (start <= end).then(|| Self::new(start, end))
    }

    /// Whether the ranges intersect or touch.
    pub fn intersects_or_touches(&self, other: &Self) -> bool {
        self.start_line_number <= other.end_line_number_exclusive
            && other.start_line_number <= self.end_line_number_exclusive
    }

    /// The 0-based row range (`start_line_number - 1..end_line_number_exclusive - 1`).
    pub fn to_rows(&self) -> std::ops::Range<u32> {
        to_u32(self.start_line_number.saturating_sub(1))
            ..to_u32(self.end_line_number_exclusive.saturating_sub(1))
    }

    /// The equivalent 0-based offset range.
    pub(crate) fn to_offset_range(self) -> OffsetRange {
        OffsetRange::new(
            self.start_line_number - 1,
            self.end_line_number_exclusive - 1,
        )
    }
}

impl fmt::Display for LineRange {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "[{},{})",
            self.start_line_number, self.end_line_number_exclusive
        )
    }
}

/// A 0-based range of sequence offsets (Monaco's `OffsetRange`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) struct OffsetRange {
    pub(crate) start: usize,
    pub(crate) end_exclusive: usize,
}

impl OffsetRange {
    pub(crate) fn new(start: usize, end_exclusive: usize) -> Self {
        debug_assert!(
            start <= end_exclusive,
            "Invalid range: [{start}, {end_exclusive})"
        );
        Self {
            start,
            end_exclusive,
        }
    }

    pub(crate) fn of_length(length: usize) -> Self {
        Self::new(0, length)
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.start == self.end_exclusive
    }

    pub(crate) fn len(&self) -> usize {
        self.end_exclusive - self.start
    }

    pub(crate) fn delta(&self, offset: isize) -> Self {
        Self::new(
            self.start.saturating_add_signed(offset),
            self.end_exclusive.saturating_add_signed(offset),
        )
    }

    pub(crate) fn delta_start(&self, offset: isize) -> Self {
        Self::new(self.start.saturating_add_signed(offset), self.end_exclusive)
    }

    pub(crate) fn delta_end(&self, offset: isize) -> Self {
        Self::new(self.start, self.end_exclusive.saturating_add_signed(offset))
    }

    pub(crate) fn join(&self, other: &Self) -> Self {
        Self::new(
            self.start.min(other.start),
            self.end_exclusive.max(other.end_exclusive),
        )
    }

    pub(crate) fn intersect(&self, other: &Self) -> Option<Self> {
        let start = self.start.max(other.start);
        let end = self.end_exclusive.min(other.end_exclusive);
        (start <= end).then(|| Self::new(start, end))
    }

    /// Whether some offset is in both ranges (always false if one is empty).
    pub(crate) fn intersects(&self, other: &Self) -> bool {
        self.start.max(other.start) < self.end_exclusive.min(other.end_exclusive)
    }

    pub(crate) fn iter(&self) -> std::ops::Range<usize> {
        self.start..self.end_exclusive
    }
}

/// Index of the last item for which `predicate` holds, searching
/// `array[start..]`; `predicate` must be `[true, …, true, false, …, false]`.
/// Returns `start - 1` (as `isize`) if there is none, like Monaco's
/// `findLastIdxMonotonous`.
pub(crate) fn find_last_idx_monotonous<T>(
    array: &[T],
    start: usize,
    predicate: impl Fn(&T) -> bool,
) -> isize {
    let mut i = start;
    let mut j = array.len();
    while i < j {
        let k = (i + j) / 2;
        if predicate(&array[k]) {
            i = k + 1;
        } else {
            j = k;
        }
    }
    i as isize - 1
}

/// Index of the first item for which `predicate` holds, or `array.len()`;
/// `predicate` must be `[false, …, false, true, …, true]`.
pub(crate) fn find_first_idx_monotonous_or_len<T>(
    array: &[T],
    predicate: impl Fn(&T) -> bool,
) -> usize {
    array.partition_point(|item| !predicate(item))
}

/// The last item for which `predicate` holds (see [`find_last_idx_monotonous`]).
pub(crate) fn find_last_monotonous<T>(
    array: &[T],
    predicate: impl Fn(&T) -> bool,
) -> Option<usize> {
    usize::try_from(find_last_idx_monotonous(array, 0, predicate)).ok()
}

/// Monaco's `MonotonousArray`: repeated `find_last_monotonous` queries with
/// weakening predicates, resuming the search where the previous one ended.
pub(crate) struct MonotonousArray<'a, T> {
    array: &'a [T],
    last_idx: usize,
}

impl<'a, T> MonotonousArray<'a, T> {
    pub(crate) fn new(array: &'a [T]) -> Self {
        Self { array, last_idx: 0 }
    }

    /// The index of the last item matching `predicate`, searching from the
    /// end of the previous match (exactly like Monaco, including returning
    /// the item before the search start if nothing after it matches).
    pub(crate) fn find_last_monotonous(&mut self, predicate: impl Fn(&T) -> bool) -> Option<usize> {
        let idx = find_last_idx_monotonous(self.array, self.last_idx, predicate);
        self.last_idx = (idx + 1) as usize;
        usize::try_from(idx).ok()
    }
}

/// A normalized set of line ranges (Monaco's `LineRangeSet`): sorted, with
/// no two ranges touching or intersecting.
#[derive(Clone, Debug, Default)]
pub(crate) struct LineRangeSet {
    ranges: Vec<LineRange>,
}

impl LineRangeSet {
    fn from_ranges(ranges: Vec<LineRange>) -> Self {
        Self { ranges }
    }

    pub(crate) fn ranges(&self) -> &[LineRange] {
        &self.ranges
    }

    /// The `[first, last)` indices of the ranges touching `range`.
    fn touching(&self, range: &LineRange) -> (usize, usize) {
        let start = find_first_idx_monotonous_or_len(&self.ranges, |r| {
            r.end_line_number_exclusive >= range.start_line_number
        });
        let end = (find_last_idx_monotonous(&self.ranges, 0, |r| {
            r.start_line_number <= range.end_line_number_exclusive
        }) + 1) as usize;
        (start, end)
    }

    pub(crate) fn add_range(&mut self, range: LineRange) {
        if range.is_empty() {
            return;
        }
        let (start, end) = self.touching(&range);
        if start == end {
            self.ranges.insert(start, range);
        } else if start == end - 1 {
            self.ranges[start] = self.ranges[start].join(&range);
        } else {
            let joined = self.ranges[start].join(&self.ranges[end - 1]).join(&range);
            self.ranges.splice(start..end, [joined]);
        }
    }

    pub(crate) fn contains(&self, line_number: usize) -> bool {
        find_last_monotonous(&self.ranges, |r| r.start_line_number <= line_number)
            .is_some_and(|idx| self.ranges[idx].end_line_number_exclusive > line_number)
    }

    /// `range` minus all ranges in this set.
    pub(crate) fn subtract_from(&self, range: LineRange) -> Self {
        let (start, end) = self.touching(&range);
        if start == end {
            return Self::from_ranges(vec![range]);
        }
        let mut result = Vec::new();
        let mut start_line_number = range.start_line_number;
        for r in &self.ranges[start..end] {
            if r.start_line_number > start_line_number {
                result.push(LineRange::new(start_line_number, r.start_line_number));
            }
            start_line_number = r.end_line_number_exclusive;
        }
        if start_line_number < range.end_line_number_exclusive {
            result.push(LineRange::new(
                start_line_number,
                range.end_line_number_exclusive,
            ));
        }
        Self::from_ranges(result)
    }

    pub(crate) fn intersection(&self, other: &Self) -> Self {
        let mut result = Vec::new();
        let (mut i1, mut i2) = (0, 0);
        while i1 < self.ranges.len() && i2 < other.ranges.len() {
            let r1 = self.ranges[i1];
            let r2 = other.ranges[i2];
            if let Some(i) = r1.intersect(&r2)
                && !i.is_empty()
            {
                result.push(i);
            }
            if r1.end_line_number_exclusive < r2.end_line_number_exclusive {
                i1 += 1;
            } else {
                i2 += 1;
            }
        }
        Self::from_ranges(result)
    }

    pub(crate) fn with_delta(&self, delta: isize) -> Self {
        Self::from_ranges(self.ranges.iter().map(|r| r.delta(delta)).collect())
    }
}
