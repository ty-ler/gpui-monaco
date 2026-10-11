//! The legacy line diff computer (port of Monaco's
//! `LegacyLinesDiffComputer` and `DiffComputer` from
//! `vs/editor/common/diff/legacyLinesDiffComputer.ts`), selected in Monaco
//! with `diffAlgorithm: 'legacy'`.
//!
//! Lines are diffed with [`LcsDiff`] on their trimmed contents, then
//! characters within each changed block of fewer than 20 lines. Unlike the
//! default computer it never detects moves.

use std::collections::HashMap;
use std::time::Duration;
use web_time::Instant;

use super::lcs::{DiffChange, LcsDiff, LcsSequence, StringElement};
use super::mapping::{DetailedLineRangeMapping, RangeMapping};
use super::range::{LineRange, Range};
use super::{DiffOptions, LinesDiff};

/// Character changes separated by fewer unchanged characters are merged
/// when post-processing (Monaco's `MINIMUM_MATCHING_CHARACTER_LENGTH`).
const MINIMUM_MATCHING_CHARACTER_LENGTH: isize = 3;

const LINE_FEED: u16 = b'\n' as u16;

/// Options of the legacy [`compute`] (Monaco's `IDiffComputerOpts`).
#[derive(Clone, Copy, Debug)]
pub(crate) struct DiffComputerOptions {
    pub(crate) should_compute_char_changes: bool,
    pub(crate) should_post_process_char_changes: bool,
    pub(crate) should_ignore_trim_whitespace: bool,
    pub(crate) should_make_pretty_diff: bool,
    /// In milliseconds; 0 means no limit.
    pub(crate) max_computation_time_ms: u64,
}

/// A character level change (Monaco's `CharChange`). Lines and columns are
/// 1-based; the end is exclusive.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct CharChange {
    pub(crate) original_start_line_number: usize,
    pub(crate) original_start_column: usize,
    pub(crate) original_end_line_number: usize,
    pub(crate) original_end_column: usize,
    pub(crate) modified_start_line_number: usize,
    pub(crate) modified_start_column: usize,
    pub(crate) modified_end_line_number: usize,
    pub(crate) modified_end_column: usize,
}

/// A line change (Monaco's `LineChange`). Line numbers are 1-based and
/// inclusive; an end line number of 0 marks an insertion (original side) or
/// deletion (modified side), with the start being the line before it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct LineChange {
    pub(crate) original_start_line_number: usize,
    pub(crate) original_end_line_number: usize,
    pub(crate) modified_start_line_number: usize,
    pub(crate) modified_end_line_number: usize,
    /// `None` if character changes were not computed.
    pub(crate) char_changes: Option<Vec<CharChange>>,
}

/// The result of [`compute`] (Monaco's `IDiffComputerResult`).
#[derive(Clone, Debug)]
pub(crate) struct DiffComputerResult {
    /// Whether the line diff ran out of time.
    pub(crate) quit_early: bool,
    pub(crate) changes: Vec<LineChange>,
}

/// See [`compute_legacy_diff`](super::compute_legacy_diff) (Monaco's
/// `LegacyLinesDiffComputer.computeDiff`).
pub(crate) fn compute_legacy_diff(
    original_lines: &[Vec<u16>],
    modified_lines: &[Vec<u16>],
    options: &DiffOptions,
) -> LinesDiff {
    let result = compute(
        original_lines,
        modified_lines,
        &DiffComputerOptions {
            should_compute_char_changes: true,
            should_post_process_char_changes: true,
            should_ignore_trim_whitespace: options.ignore_trim_whitespace,
            should_make_pretty_diff: true,
            max_computation_time_ms: options.max_computation_time_ms,
        },
    );

    // `None` inner changes are Monaco's `undefined`.
    let mut changes: Vec<(DetailedLineRangeMapping, Option<Vec<RangeMapping>>)> = Vec::new();
    for c in result.changes {
        let original = if c.original_end_line_number == 0 {
            // Insertion.
            LineRange::new(
                c.original_start_line_number + 1,
                c.original_start_line_number + 1,
            )
        } else {
            LineRange::new(c.original_start_line_number, c.original_end_line_number + 1)
        };
        let modified = if c.modified_end_line_number == 0 {
            // Deletion.
            LineRange::new(
                c.modified_start_line_number + 1,
                c.modified_start_line_number + 1,
            )
        } else {
            LineRange::new(c.modified_start_line_number, c.modified_end_line_number + 1)
        };
        let mut inner_changes = c.char_changes.map(|char_changes| {
            char_changes
                .iter()
                .map(|c| {
                    RangeMapping::new(
                        Range::new(
                            c.original_start_line_number,
                            c.original_start_column,
                            c.original_end_line_number,
                            c.original_end_column,
                        ),
                        Range::new(
                            c.modified_start_line_number,
                            c.modified_start_column,
                            c.modified_end_line_number,
                            c.modified_end_column,
                        ),
                    )
                })
                .collect::<Vec<_>>()
        });
        let mut change = DetailedLineRangeMapping::new(original, modified, Vec::new());

        if let Some((last, last_inner)) = changes.last()
            && (last.modified.end_line_number_exclusive == change.modified.start_line_number
                || last.original.end_line_number_exclusive == change.original.start_line_number)
        {
            // Join touching diffs (moving diffs up/down in the algorithm
            // probably causes them).
            change.original = last.original.join(&change.original);
            change.modified = last.modified.join(&change.modified);
            inner_changes = match (last_inner, inner_changes) {
                (Some(last_inner), Some(inner)) => Some([last_inner.as_slice(), &inner].concat()),
                _ => None,
            };
            changes.pop();
        }
        changes.push((change, inner_changes));
    }

    debug_assert!(changes.windows(2).all(|pair| {
        let (m1, m2) = (&pair[0].0, &pair[1].0);
        m2.original.start_line_number as isize - m1.original.end_line_number_exclusive as isize
            == m2.modified.start_line_number as isize
                - m1.modified.end_line_number_exclusive as isize
            // There has to be an unchanged line in between.
            && m1.original.end_line_number_exclusive < m2.original.start_line_number
            && m1.modified.end_line_number_exclusive < m2.modified.start_line_number
    }));

    LinesDiff {
        changes: changes
            .into_iter()
            .map(|(mut change, inner_changes)| {
                change.inner_changes = inner_changes.unwrap_or_default();
                change
            })
            .collect(),
        moves: Vec::new(),
        hit_timeout: result.quit_early,
    }
}

/// Monaco's `createContinueProcessingPredicate`.
fn continue_processing_predicate(maximum_runtime_ms: u64) -> impl Fn() -> bool {
    let start_time = Instant::now();
    move || {
        maximum_runtime_ms == 0 || start_time.elapsed() < Duration::from_millis(maximum_runtime_ms)
    }
}

/// Monaco's `getFirstNonBlankColumn`: only spaces and tabs are blank.
fn first_non_blank_column(line: &[u16], default_value: usize) -> usize {
    line.iter()
        .position(|&c| c != b' ' as u16 && c != b'\t' as u16)
        .map_or(default_value, |i| i + 1)
}

/// Monaco's `getLastNonBlankColumn`: the column after the last non-blank
/// character.
fn last_non_blank_column(line: &[u16], default_value: usize) -> usize {
    line.iter()
        .rposition(|&c| c != b' ' as u16 && c != b'\t' as u16)
        .map_or(default_value, |i| i + 2)
}

/// Whether a UTF-16 code unit matches JavaScript's `\s`.
fn is_js_whitespace(c: u16) -> bool {
    matches!(
        c,
        0x09..=0x0d
            | 0x20
            | 0xa0
            | 0x1680
            | 0x2000..=0x200a
            | 0x2028
            | 0x2029
            | 0x202f
            | 0x205f
            | 0x3000
            | 0xfeff
    )
}

/// The lines of a text, compared by their trimmed contents (Monaco's
/// `LineSequence`).
struct LineSequence<'a> {
    lines: &'a [Vec<u16>],
    start_columns: Vec<usize>,
    end_columns: Vec<usize>,
}

impl<'a> LineSequence<'a> {
    fn new(lines: &'a [Vec<u16>]) -> Self {
        Self {
            lines,
            start_columns: lines.iter().map(|l| first_non_blank_column(l, 1)).collect(),
            end_columns: lines.iter().map(|l| last_non_blank_column(l, 1)).collect(),
        }
    }

    /// The element compared by the line diff: the line without leading and
    /// trailing blanks.
    fn element(&self, index: usize) -> &'a [u16] {
        &self.lines[index][self.start_columns[index] - 1..self.end_columns[index] - 1]
    }

    fn start_line_number(&self, i: isize) -> usize {
        (i + 1) as usize
    }

    fn end_line_number(&self, i: isize) -> usize {
        (i + 1) as usize
    }

    /// The characters of lines `start_index..=end_index` (Monaco's
    /// `createCharSequence`).
    fn create_char_sequence(
        &self,
        should_ignore_trim_whitespace: bool,
        start_index: usize,
        end_index: usize,
    ) -> CharSequence {
        let mut char_codes = Vec::new();
        let mut line_numbers = Vec::new();
        let mut columns = Vec::new();
        for index in start_index..=end_index {
            let line_content = &self.lines[index];
            let (start_column, end_column) = if should_ignore_trim_whitespace {
                (self.start_columns[index], self.end_columns[index])
            } else {
                (1, line_content.len() + 1)
            };
            for col in start_column..end_column {
                char_codes.push(line_content[col - 1]);
                line_numbers.push(index + 1);
                columns.push(col);
            }
            if !should_ignore_trim_whitespace && index < end_index {
                // Add `\n` if trim whitespace is not ignored.
                char_codes.push(LINE_FEED);
                line_numbers.push(index + 1);
                columns.push(line_content.len() + 1);
            }
        }
        CharSequence {
            char_codes,
            line_numbers,
            columns,
        }
    }
}

/// Builds the [`LcsSequence`]s of two line sequences: trimmed lines as
/// strings, full lines as strict elements.
fn line_lcs_sequences<'a>(
    original: &LineSequence<'a>,
    modified: &LineSequence<'a>,
) -> (LcsSequence, LcsSequence) {
    let mut ids: HashMap<&'a [u16], u32> = HashMap::new();
    let mut strict_ids: HashMap<&'a [u16], u32> = HashMap::new();
    let mut build = |sequence: &LineSequence<'a>| {
        let lines: &'a [Vec<u16>] = sequence.lines;
        let mut elements = Vec::with_capacity(lines.len());
        let mut strings = Vec::with_capacity(lines.len());
        let mut strict_elements = Vec::with_capacity(lines.len());
        for (index, line) in lines.iter().enumerate() {
            let element = sequence.element(index);
            let next = ids.len() as u32;
            elements.push(*ids.entry(element).or_insert(next));
            strings.push(StringElement {
                len: element.len(),
                is_whitespace: element.iter().all(|&c| is_js_whitespace(c)),
            });
            let next = strict_ids.len() as u32;
            strict_elements.push(*strict_ids.entry(line.as_slice()).or_insert(next));
        }
        LcsSequence {
            elements,
            strings: Some(strings),
            strict_elements: Some(strict_elements),
        }
    };
    let original = build(original);
    let modified = build(modified);
    (original, modified)
}

/// Characters with their positions (Monaco's `CharSequence`). Indices are
/// `isize` because Monaco asks for the positions just outside the sequence.
struct CharSequence {
    char_codes: Vec<u16>,
    line_numbers: Vec<usize>,
    columns: Vec<usize>,
}

impl CharSequence {
    fn lcs_sequence(&self) -> LcsSequence {
        LcsSequence {
            elements: self.char_codes.iter().map(|&c| u32::from(c)).collect(),
            strings: None,
            strict_elements: None,
        }
    }

    fn index(&self, i: isize) -> usize {
        assert!(
            i >= 0 && (i as usize) < self.line_numbers.len(),
            "Illegal index"
        );
        i as usize
    }

    fn start_line_number(&self, i: isize) -> usize {
        if i > 0 && i as usize == self.line_numbers.len() {
            // The start of the element after the last is the end of the last.
            return self.end_line_number(i - 1);
        }
        self.line_numbers[self.index(i)]
    }

    fn end_line_number(&self, i: isize) -> usize {
        if i == -1 {
            // The end of the element before the first is the start of the
            // first.
            return self.start_line_number(i + 1);
        }
        let i = self.index(i);
        if self.char_codes[i] == LINE_FEED {
            return self.line_numbers[i] + 1;
        }
        self.line_numbers[i]
    }

    fn start_column(&self, i: isize) -> usize {
        if i > 0 && i as usize == self.columns.len() {
            return self.end_column(i - 1);
        }
        self.columns[self.index(i)]
    }

    fn end_column(&self, i: isize) -> usize {
        if i == -1 {
            return self.start_column(i + 1);
        }
        let i = self.index(i);
        if self.char_codes[i] == LINE_FEED {
            return 1;
        }
        self.columns[i] + 1
    }
}

impl CharChange {
    /// Monaco's `CharChange.createFromDiffChange`.
    fn from_diff_change(
        diff_change: &DiffChange,
        original: &CharSequence,
        modified: &CharSequence,
    ) -> Self {
        let original_last = diff_change.original_start + diff_change.original_length - 1;
        let modified_last = diff_change.modified_start + diff_change.modified_length - 1;
        Self {
            original_start_line_number: original.start_line_number(diff_change.original_start),
            original_start_column: original.start_column(diff_change.original_start),
            original_end_line_number: original.end_line_number(original_last),
            original_end_column: original.end_column(original_last),
            modified_start_line_number: modified.start_line_number(diff_change.modified_start),
            modified_start_column: modified.start_column(diff_change.modified_start),
            modified_end_line_number: modified.end_line_number(modified_last),
            modified_end_column: modified.end_column(modified_last),
        }
    }

    /// A change within one line.
    fn on_lines(
        original_line_number: usize,
        original_start_column: usize,
        original_end_column: usize,
        modified_line_number: usize,
        modified_start_column: usize,
        modified_end_column: usize,
    ) -> Self {
        Self {
            original_start_line_number: original_line_number,
            original_start_column,
            original_end_line_number: original_line_number,
            original_end_column,
            modified_start_line_number: modified_line_number,
            modified_start_column,
            modified_end_line_number: modified_line_number,
            modified_end_column,
        }
    }
}

/// Merges character changes separated by fewer than
/// [`MINIMUM_MATCHING_CHARACTER_LENGTH`] unchanged characters (Monaco's
/// `postProcessCharChanges`).
fn post_process_char_changes(raw_changes: Vec<DiffChange>) -> Vec<DiffChange> {
    let mut result: Vec<DiffChange> = Vec::with_capacity(raw_changes.len());
    for curr_change in raw_changes {
        let Some(prev_change) = result.last_mut() else {
            result.push(curr_change);
            continue;
        };
        let original_matching_length =
            curr_change.original_start - (prev_change.original_start + prev_change.original_length);
        let modified_matching_length =
            curr_change.modified_start - (prev_change.modified_start + prev_change.modified_length);
        // Both should be equal, but the continue predicate may prevent that.
        let matching_length = original_matching_length.min(modified_matching_length);

        if matching_length < MINIMUM_MATCHING_CHARACTER_LENGTH {
            // Merge the current change into the previous one.
            prev_change.original_length = (curr_change.original_start
                + curr_change.original_length)
                - prev_change.original_start;
            prev_change.modified_length = (curr_change.modified_start
                + curr_change.modified_length)
                - prev_change.modified_start;
        } else {
            result.push(curr_change);
        }
    }
    result
}

impl LineChange {
    /// Monaco's `LineChange.createFromDiffResult`.
    fn from_diff_result(
        diff_change: &DiffChange,
        original: &LineSequence<'_>,
        modified: &LineSequence<'_>,
        continue_char_diff: &dyn Fn() -> bool,
        options: &DiffComputerOptions,
    ) -> Self {
        let (original_start_line_number, original_end_line_number) = if diff_change.original_length
            == 0
        {
            (
                original.start_line_number(diff_change.original_start) - 1,
                0,
            )
        } else {
            (
                original.start_line_number(diff_change.original_start),
                original
                    .end_line_number(diff_change.original_start + diff_change.original_length - 1),
            )
        };
        let (modified_start_line_number, modified_end_line_number) = if diff_change.modified_length
            == 0
        {
            (
                modified.start_line_number(diff_change.modified_start) - 1,
                0,
            )
        } else {
            (
                modified.start_line_number(diff_change.modified_start),
                modified
                    .end_line_number(diff_change.modified_start + diff_change.modified_length - 1),
            )
        };

        let mut char_changes = None;
        if options.should_compute_char_changes
            && diff_change.original_length > 0
            && diff_change.original_length < 20
            && diff_change.modified_length > 0
            && diff_change.modified_length < 20
            && continue_char_diff()
        {
            // Compute character changes for diff chunks of at most 20 lines.
            let original_start = diff_change.original_start as usize;
            let modified_start = diff_change.modified_start as usize;
            let original_chars = original.create_char_sequence(
                options.should_ignore_trim_whitespace,
                original_start,
                original_start + diff_change.original_length as usize - 1,
            );
            let modified_chars = modified.create_char_sequence(
                options.should_ignore_trim_whitespace,
                modified_start,
                modified_start + diff_change.modified_length as usize - 1,
            );

            if !original_chars.char_codes.is_empty() && !modified_chars.char_codes.is_empty() {
                let mut raw_changes = LcsDiff::new(
                    original_chars.lcs_sequence(),
                    modified_chars.lcs_sequence(),
                    continue_char_diff,
                )
                .compute_diff(true)
                .changes;

                if options.should_post_process_char_changes {
                    raw_changes = post_process_char_changes(raw_changes);
                }

                char_changes = Some(
                    raw_changes
                        .iter()
                        .map(|c| CharChange::from_diff_change(c, &original_chars, &modified_chars))
                        .collect(),
                );
            }
        }

        Self {
            original_start_line_number,
            original_end_line_number,
            modified_start_line_number,
            modified_end_line_number,
            char_changes,
        }
    }
}

/// Computes the line changes from `original_lines` to `modified_lines`
/// (Monaco's `DiffComputer.computeDiff`).
pub(crate) fn compute(
    original_lines: &[Vec<u16>],
    modified_lines: &[Vec<u16>],
    options: &DiffComputerOptions,
) -> DiffComputerResult {
    let continue_line_diff = continue_processing_predicate(options.max_computation_time_ms);
    // Never run character changes for more than 5 s.
    let continue_char_diff =
        continue_processing_predicate(if options.max_computation_time_ms == 0 {
            0
        } else {
            options.max_computation_time_ms.min(5000)
        });

    let is_empty = |lines: &[Vec<u16>]| lines.len() == 1 && lines[0].is_empty();
    if is_empty(original_lines) {
        // Empty original: fast path.
        if is_empty(modified_lines) {
            return DiffComputerResult {
                quit_early: false,
                changes: Vec::new(),
            };
        }
        return DiffComputerResult {
            quit_early: false,
            changes: vec![LineChange {
                original_start_line_number: 1,
                original_end_line_number: 1,
                modified_start_line_number: 1,
                modified_end_line_number: modified_lines.len(),
                char_changes: None,
            }],
        };
    }
    if is_empty(modified_lines) {
        // Empty modified: fast path.
        return DiffComputerResult {
            quit_early: false,
            changes: vec![LineChange {
                original_start_line_number: 1,
                original_end_line_number: original_lines.len(),
                modified_start_line_number: 1,
                modified_end_line_number: 1,
                char_changes: None,
            }],
        };
    }

    let original = LineSequence::new(original_lines);
    let modified = LineSequence::new(modified_lines);
    let (original_lcs, modified_lcs) = line_lcs_sequences(&original, &modified);
    let diff_result = LcsDiff::new(original_lcs, modified_lcs, &continue_line_diff)
        .compute_diff(options.should_make_pretty_diff);
    let raw_changes = diff_result.changes;
    let quit_early = diff_result.quit_early;

    // The diff is always computed ignoring trim whitespace, which gives the
    // prettiest diff.
    let line_change = |diff_change: &DiffChange| {
        LineChange::from_diff_result(
            diff_change,
            &original,
            &modified,
            &continue_char_diff,
            options,
        )
    };

    if options.should_ignore_trim_whitespace {
        return DiffComputerResult {
            quit_early,
            changes: raw_changes.iter().map(line_change).collect(),
        };
    }

    // Introduce changes where the trim whitespace differs, starting before
    // the first change.
    let mut result: Vec<LineChange> = Vec::new();
    let mut original_line_index = 0;
    let mut modified_line_index = 0;
    for i in 0..=raw_changes.len() {
        let next_change = raw_changes.get(i);
        let (original_stop, modified_stop) = match next_change {
            Some(c) => (c.original_start as usize, c.modified_start as usize),
            None => (original_lines.len(), modified_lines.len()),
        };

        while original_line_index < original_stop && modified_line_index < modified_stop {
            let original_line = &original_lines[original_line_index];
            let modified_line = &modified_lines[modified_line_index];

            if original_line != modified_line {
                // The lines differ only in trim whitespace.

                // Check the leading whitespace.
                {
                    let mut original_start_column = first_non_blank_column(original_line, 1);
                    let mut modified_start_column = first_non_blank_column(modified_line, 1);
                    while original_start_column > 1 && modified_start_column > 1 {
                        let original_char = original_line[original_start_column - 2];
                        let modified_char = modified_line[modified_start_column - 2];
                        if original_char != modified_char {
                            break;
                        }
                        original_start_column -= 1;
                        modified_start_column -= 1;
                    }

                    if original_start_column > 1 || modified_start_column > 1 {
                        push_trim_whitespace_char_change(
                            &mut result,
                            options,
                            original_line_index + 1,
                            1,
                            original_start_column,
                            modified_line_index + 1,
                            1,
                            modified_start_column,
                        );
                    }
                }

                // Check the trailing whitespace.
                {
                    let mut original_end_column = last_non_blank_column(original_line, 1);
                    let mut modified_end_column = last_non_blank_column(modified_line, 1);
                    let original_max_column = original_line.len() + 1;
                    let modified_max_column = modified_line.len() + 1;
                    while original_end_column < original_max_column
                        && modified_end_column < modified_max_column
                    {
                        let original_char = original_line.get(original_end_column - 1);
                        // Like Monaco, this reads the *original* line (a bug
                        // there); out of range reads are NaN, equal to nothing.
                        let modified_char = original_line.get(modified_end_column - 1);
                        match (original_char, modified_char) {
                            (Some(a), Some(b)) if a == b => {}
                            _ => break,
                        }
                        original_end_column += 1;
                        modified_end_column += 1;
                    }

                    if original_end_column < original_max_column
                        || modified_end_column < modified_max_column
                    {
                        push_trim_whitespace_char_change(
                            &mut result,
                            options,
                            original_line_index + 1,
                            original_end_column,
                            original_max_column,
                            modified_line_index + 1,
                            modified_end_column,
                            modified_max_column,
                        );
                    }
                }
            }
            original_line_index += 1;
            modified_line_index += 1;
        }

        if let Some(next_change) = next_change {
            // Emit the actual change.
            result.push(line_change(next_change));
            original_line_index += next_change.original_length as usize;
            modified_line_index += next_change.modified_length as usize;
        }
    }

    DiffComputerResult {
        quit_early,
        changes: result,
    }
}

/// Monaco's `_pushTrimWhitespaceCharChange`.
#[allow(clippy::too_many_arguments)]
fn push_trim_whitespace_char_change(
    result: &mut Vec<LineChange>,
    options: &DiffComputerOptions,
    original_line_number: usize,
    original_start_column: usize,
    original_end_column: usize,
    modified_line_number: usize,
    modified_start_column: usize,
    modified_end_column: usize,
) {
    let char_change = CharChange::on_lines(
        original_line_number,
        original_start_column,
        original_end_column,
        modified_line_number,
        modified_start_column,
        modified_end_column,
    );
    if merge_trim_whitespace_char_change(result, options, char_change) {
        // Merged into the previous change.
        return;
    }
    result.push(LineChange {
        original_start_line_number: original_line_number,
        original_end_line_number: original_line_number,
        modified_start_line_number: modified_line_number,
        modified_end_line_number: modified_line_number,
        char_changes: options
            .should_compute_char_changes
            .then(|| vec![char_change]),
    });
}

/// Monaco's `_mergeTrimWhitespaceCharChange`.
fn merge_trim_whitespace_char_change(
    result: &mut [LineChange],
    options: &DiffComputerOptions,
    char_change: CharChange,
) -> bool {
    let Some(prev_change) = result.last_mut() else {
        return false;
    };
    if prev_change.original_end_line_number == 0 || prev_change.modified_end_line_number == 0 {
        // Don't merge with inserts/deletes.
        return false;
    }

    let original_line_number = char_change.original_start_line_number;
    let modified_line_number = char_change.modified_start_line_number;
    let same_lines = prev_change.original_end_line_number == original_line_number
        && prev_change.modified_end_line_number == modified_line_number;
    let next_lines = prev_change.original_end_line_number + 1 == original_line_number
        && prev_change.modified_end_line_number + 1 == modified_line_number;
    if !same_lines && !next_lines {
        return false;
    }
    if next_lines {
        prev_change.original_end_line_number = original_line_number;
        prev_change.modified_end_line_number = modified_line_number;
    }
    if options.should_compute_char_changes
        && let Some(char_changes) = &mut prev_change.char_changes
    {
        char_changes.push(char_change);
    }
    true
}
