//! Mappings between the original and the modified text (port of Monaco's
//! `rangeMapping.ts`).

use std::fmt;

use super::range::{LineRange, Position, Range};

/// Maps a line range in the original text to a line range in the modified
/// text (Monaco's `LineRangeMapping`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct LineRangeMapping {
    /// The line range in the original text.
    pub original: LineRange,
    /// The line range in the modified text.
    pub modified: LineRange,
}

impl LineRangeMapping {
    /// Creates a mapping.
    pub fn new(original: LineRange, modified: LineRange) -> Self {
        Self { original, modified }
    }

    /// Swaps original and modified.
    pub fn flip(&self) -> Self {
        Self::new(self.modified, self.original)
    }

    /// The smallest mapping containing both mappings.
    pub fn join(&self, other: &Self) -> Self {
        Self::new(
            self.original.join(&other.original),
            self.modified.join(&other.modified),
        )
    }

    /// The number of changed lines: the longer of the two ranges.
    pub fn changed_line_count(&self) -> usize {
        self.original.len().max(self.modified.len())
    }

    /// Converts to a character range mapping covering the full lines
    /// (Monaco's `toRangeMapping2`). Assumes the mapping describes a valid
    /// diff of `original`/`modified`.
    pub(crate) fn to_range_mapping2(
        self,
        original: &[Vec<u16>],
        modified: &[Vec<u16>],
    ) -> RangeMapping {
        let is_valid =
            |line_number: usize, lines: &[Vec<u16>]| line_number >= 1 && line_number <= lines.len();
        if is_valid(self.original.end_line_number_exclusive, original)
            && is_valid(self.modified.end_line_number_exclusive, modified)
        {
            return RangeMapping::new(
                Range::new(
                    self.original.start_line_number,
                    1,
                    self.original.end_line_number_exclusive,
                    1,
                ),
                Range::new(
                    self.modified.start_line_number,
                    1,
                    self.modified.end_line_number_exclusive,
                    1,
                ),
            );
        }

        let end_of = |line_range: &LineRange, lines: &[Vec<u16>]| {
            normalize_position(
                Position::new(
                    line_range.end_line_number_exclusive.wrapping_sub(1),
                    usize::MAX,
                ),
                lines,
            )
        };

        if !self.original.is_empty() && !self.modified.is_empty() {
            return RangeMapping::new(
                Range::from_positions(
                    Position::new(self.original.start_line_number, 1),
                    end_of(&self.original, original),
                ),
                Range::from_positions(
                    Position::new(self.modified.start_line_number, 1),
                    end_of(&self.modified, modified),
                ),
            );
        }

        // Either both start after line 1, or (Monaco considers this
        // impossible and throws) one range is empty, one touches the last line
        // and one starts at line 1; clamping keeps the latter well-defined.
        let start_of = |line_range: &LineRange, lines: &[Vec<u16>]| {
            normalize_position(
                Position::new(line_range.start_line_number.wrapping_sub(1), usize::MAX),
                lines,
            )
        };
        debug_assert!(self.original.start_line_number > 1 && self.modified.start_line_number > 1);
        RangeMapping::new(
            Range::from_positions(
                start_of(&self.original, original),
                end_of(&self.original, original),
            ),
            Range::from_positions(
                start_of(&self.modified, modified),
                end_of(&self.modified, modified),
            ),
        )
    }
}

impl fmt::Display for LineRangeMapping {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{{{}->{}}}", self.original, self.modified)
    }
}

/// Clamps `position` into `lines` (line numbers of 0 or `usize::MAX` mean
/// "before the first line").
fn normalize_position(position: Position, lines: &[Vec<u16>]) -> Position {
    if position.line_number < 1 || position.line_number == usize::MAX {
        return Position::new(1, 1);
    }
    if position.line_number > lines.len() {
        let last = lines.last().map_or(0, Vec::len);
        return Position::new(lines.len(), last + 1);
    }
    let line = &lines[position.line_number - 1];
    if position.column > line.len() + 1 {
        return Position::new(position.line_number, line.len() + 1);
    }
    position
}

/// A [`LineRangeMapping`] with the character-level changes inside it
/// (Monaco's `DetailedLineRangeMapping`).
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct DetailedLineRangeMapping {
    /// The line range in the original text.
    pub original: LineRange,
    /// The line range in the modified text.
    pub modified: LineRange,
    /// The character-level changes, sorted. Each original range lies within
    /// [`Self::original`] (and each modified range within
    /// [`Self::modified`]), except that ranges may extend to the line break
    /// right after them. Never empty for mappings returned by
    /// [`compute_diff`](super::compute_diff); empty where
    /// [`compute_legacy_diff`](super::compute_legacy_diff) didn't compute
    /// them (Monaco's `innerChanges: undefined`, for changes of 20+ lines).
    pub inner_changes: Vec<RangeMapping>,
}

impl DetailedLineRangeMapping {
    /// Creates a mapping.
    pub fn new(original: LineRange, modified: LineRange, inner_changes: Vec<RangeMapping>) -> Self {
        Self {
            original,
            modified,
            inner_changes,
        }
    }

    /// The line ranges without the inner changes.
    pub fn line_range_mapping(&self) -> LineRangeMapping {
        LineRangeMapping::new(self.original, self.modified)
    }

    /// Swaps original and modified (including the inner changes).
    pub fn flip(&self) -> Self {
        Self::new(
            self.modified,
            self.original,
            self.inner_changes.iter().map(RangeMapping::flip).collect(),
        )
    }
}

impl fmt::Display for DetailedLineRangeMapping {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{{{}->{}}}", self.original, self.modified)
    }
}

/// Maps a character range in the original text to a character range in the
/// modified text (Monaco's `RangeMapping`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct RangeMapping {
    /// The range in the original text.
    pub original_range: Range,
    /// The range in the modified text.
    pub modified_range: Range,
}

impl RangeMapping {
    /// Creates a mapping.
    pub fn new(original_range: Range, modified_range: Range) -> Self {
        Self {
            original_range,
            modified_range,
        }
    }

    /// Swaps original and modified.
    pub fn flip(&self) -> Self {
        Self::new(self.modified_range, self.original_range)
    }

    /// The smallest mapping containing both mappings.
    pub fn join(&self, other: &Self) -> Self {
        Self::new(
            self.original_range.plus_range(&other.original_range),
            self.modified_range.plus_range(&other.modified_range),
        )
    }
}

impl fmt::Display for RangeMapping {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{{{}->{}}}", self.original_range, self.modified_range)
    }
}

/// Groups character-level mappings into line-level mappings (Monaco's
/// `lineRangeMappingFromRangeMappings`).
pub(crate) fn line_range_mapping_from_range_mappings(
    alignments: &[RangeMapping],
    original: &[Vec<u16>],
    modified: &[Vec<u16>],
) -> Vec<DetailedLineRangeMapping> {
    let mut changes: Vec<DetailedLineRangeMapping> = Vec::new();
    // The previous mapping and the first mapping of the current group.
    let mut last: Option<(LineRangeMapping, LineRangeMapping)> = None;
    for alignment in alignments {
        let mapping = get_line_range_mapping(alignment, original, modified);
        let group_first = last.and_then(|(last, first)| {
            (last.original.intersects_or_touches(&mapping.original)
                || last.modified.intersects_or_touches(&mapping.modified))
            .then_some(first)
        });
        if let Some(first) = group_first
            && let Some(current) = changes.last_mut()
        {
            // Like Monaco: the group spans its first and last mapping.
            current.original = first.original.join(&mapping.original);
            current.modified = first.modified.join(&mapping.modified);
            current.inner_changes.push(*alignment);
            last = Some((mapping, first));
            continue;
        } else {
            changes.push(DetailedLineRangeMapping::new(
                mapping.original,
                mapping.modified,
                vec![*alignment],
            ));
        }
        last = Some((mapping, mapping));
    }
    changes
}

/// The line range mapping touched by a character-level mapping (Monaco's
/// `getLineRangeMapping`).
pub(crate) fn get_line_range_mapping(
    range_mapping: &RangeMapping,
    original: &[Vec<u16>],
    modified: &[Vec<u16>],
) -> LineRangeMapping {
    let o = &range_mapping.original_range;
    let m = &range_mapping.modified_range;
    let line_len = |lines: &[Vec<u16>], line_number: usize| {
        lines.get(line_number.wrapping_sub(1)).map_or(0, Vec::len)
    };

    let mut line_start_delta = 0;
    let mut line_end_delta = 0isize;

    // original: ]xxx \n <- this line is not modified
    // modified: ]xx  \n
    if m.end_column == 1
        && o.end_column == 1
        && o.start_line_number + line_start_delta <= o.end_line_number
        && m.start_line_number + line_start_delta <= m.end_line_number
    {
        // We can only do this if the range is not empty yet.
        line_end_delta = -1;
    }

    // original: xxx[ \n <- this line is not modified
    // modified: xxx[ \n
    if m.start_column > line_len(modified, m.start_line_number)
        && o.start_column > line_len(original, o.start_line_number)
        && o.start_line_number as isize <= o.end_line_number as isize + line_end_delta
        && m.start_line_number as isize <= m.end_line_number as isize + line_end_delta
    {
        // We can only do this if the range is not empty yet.
        line_start_delta = 1;
    }

    let line_range = |r: &Range| {
        LineRange::new(
            r.start_line_number + line_start_delta,
            (r.end_line_number + 1).saturating_add_signed(line_end_delta),
        )
    };
    LineRangeMapping::new(line_range(o), line_range(m))
}
