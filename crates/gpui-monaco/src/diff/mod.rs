//! Line diffing: a port of Monaco's default ("advanced") lines diff computer
//! (`vs/editor/common/diff`, MIT licensed), as used by Monaco's diff editor.
//!
//! [`compute_diff`] diffs two texts given as lines and returns the changed
//! line ranges, the character-level changes inside them and, optionally,
//! blocks of moved code. The results match Monaco's exactly (verified
//! against Monaco's diffing fixtures).
//!
//! # Coordinates
//!
//! Everything uses Monaco's coordinates so results compare 1:1 with Monaco:
//!
//! - Line numbers are **1-based**. A [`LineRange`] is
//!   `[start_line_number, end_line_number_exclusive)`.
//! - A [`Position`]/[`Range`] column is **1-based** and counts **UTF-16 code
//!   units** (like JavaScript string offsets).
//!
//! Use [`Position::to_point_utf16`], [`Range::to_point_utf16_range`] and
//! [`LineRange::to_rows`] to convert to the editor's 0-based
//! [`text::PointUtf16`] coordinates.
//!
//! The input lines must not contain line breaks.

mod algorithms;
mod computer;
mod heuristics;
mod mapping;
mod moves;
pub(crate) mod range;
mod sequences;
#[cfg(test)]
mod tests;

pub use mapping::{DetailedLineRangeMapping, LineRangeMapping, RangeMapping};
pub use range::{LineRange, Position, Range};

/// Options for [`compute_diff`] (Monaco's `ILinesDiffComputerOptions`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DiffOptions {
    /// Treat lines that differ only in leading/trailing whitespace as equal
    /// (no character-level changes are reported for such differences).
    pub ignore_trim_whitespace: bool,
    /// The time budget in milliseconds; 0 means no limit. When it runs out
    /// the remaining work falls back to coarse results and
    /// [`LinesDiff::hit_timeout`] is set.
    pub max_computation_time_ms: u64,
    /// Whether to detect moved blocks of code ([`LinesDiff::moves`]).
    pub compute_moves: bool,
    /// Extend character-level changes to whole sub-words (`fooBar` has the
    /// sub-words `foo` and `Bar`) when much of the sub-word changed.
    pub extend_to_subwords: bool,
}

impl Default for DiffOptions {
    /// Monaco's diff editor defaults: ignore trim whitespace, 5 s time
    /// budget, no moves, no sub-word extension.
    fn default() -> Self {
        Self {
            ignore_trim_whitespace: true,
            max_computation_time_ms: 5000,
            compute_moves: false,
            extend_to_subwords: false,
        }
    }
}

/// The result of [`compute_diff`] (Monaco's `LinesDiff`).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LinesDiff {
    /// The changes, sorted, with at least one unchanged line between any two.
    pub changes: Vec<DetailedLineRangeMapping>,
    /// Moved blocks, sorted by original line range. The original ranges (and
    /// the modified ranges) are disjoint but may touch.
    pub moves: Vec<MovedText>,
    /// Whether the time budget ran out; the diff may then be an
    /// approximation and the user could be offered a rerun with more time.
    pub hit_timeout: bool,
}

/// A block of lines that moved (Monaco's `MovedText`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MovedText {
    /// Where the block was in the original and where it is in the modified
    /// text.
    pub line_range_mapping: LineRangeMapping,
    /// The diff from the original block to the moved block, contained in
    /// [`Self::line_range_mapping`]; empty if the text only moved.
    pub changes: Vec<DetailedLineRangeMapping>,
}

impl MovedText {
    /// Swaps original and modified.
    pub fn flip(&self) -> Self {
        Self {
            line_range_mapping: self.line_range_mapping.flip(),
            changes: self
                .changes
                .iter()
                .map(DetailedLineRangeMapping::flip)
                .collect(),
        }
    }
}

/// Encodes lines as UTF-16; an empty slice becomes a single empty line (a
/// text always has at least one line).
fn to_utf16_lines<S: AsRef<str>>(lines: &[S]) -> Vec<Vec<u16>> {
    if lines.is_empty() {
        return vec![Vec::new()];
    }
    lines
        .iter()
        .map(|line| line.as_ref().encode_utf16().collect())
        .collect()
}

/// Computes the diff from `original` to `modified`, both given as lines
/// without line breaks (Monaco's `DefaultLinesDiffComputer.computeDiff`).
///
/// An empty slice is treated like a single empty line. See the [module
/// docs](self) for the coordinate conventions of the result.
///
/// ```
/// use gpui_monaco::diff::{DiffOptions, compute_diff};
///
/// let diff = compute_diff(&["let a = 1;"], &["let b = 1;"], &DiffOptions::default());
/// assert_eq!(diff.changes.len(), 1);
/// let inner = diff.changes[0].inner_changes[0];
/// assert_eq!(inner.original_range.to_string(), "[1,5 -> 1,6]");
/// ```
pub fn compute_diff<S: AsRef<str>>(
    original: &[S],
    modified: &[S],
    options: &DiffOptions,
) -> LinesDiff {
    computer::compute_diff(
        &to_utf16_lines(original),
        &to_utf16_lines(modified),
        options,
    )
}
