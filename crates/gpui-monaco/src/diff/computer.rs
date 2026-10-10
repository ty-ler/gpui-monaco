//! The line diff computer (port of Monaco's `DefaultLinesDiffComputer`).

use std::collections::HashMap;

use super::algorithms::{
    DiffAlgorithmResult, SequenceDiff, Timeout, dynamic_programming_diff, myers_diff,
};
use super::heuristics::{
    extend_diffs_to_entire_word_if_appropriate, optimize_sequence_diffs, remove_short_matches,
    remove_very_short_matching_lines_between_diffs,
    remove_very_short_matching_text_between_long_diffs,
};
use super::mapping::{
    DetailedLineRangeMapping, LineRangeMapping, RangeMapping,
    line_range_mapping_from_range_mappings,
};
use super::moves::compute_moved_lines;
use super::range::{LineRange, OffsetRange, Range};
use super::sequences::{LineSequence, LinesSliceCharSequence, trim};
use super::{DiffOptions, LinesDiff, MovedText};

/// See [`compute_diff`](super::compute_diff).
pub(crate) fn compute_diff(
    original_lines: &[Vec<u16>],
    modified_lines: &[Vec<u16>],
    options: &DiffOptions,
) -> LinesDiff {
    if original_lines.len() <= 1 && original_lines == modified_lines {
        return LinesDiff::default();
    }

    if (original_lines.len() == 1 && original_lines[0].is_empty())
        || (modified_lines.len() == 1 && modified_lines[0].is_empty())
    {
        let last_len = |lines: &[Vec<u16>]| lines.last().map_or(0, Vec::len);
        return LinesDiff {
            changes: vec![DetailedLineRangeMapping::new(
                LineRange::new(1, original_lines.len() + 1),
                LineRange::new(1, modified_lines.len() + 1),
                vec![RangeMapping::new(
                    Range::new(1, 1, original_lines.len(), last_len(original_lines) + 1),
                    Range::new(1, 1, modified_lines.len(), last_len(modified_lines) + 1),
                )],
            )],
            moves: Vec::new(),
            hit_timeout: false,
        };
    }

    let timeout = Timeout::new(options.max_computation_time_ms);
    let consider_whitespace_changes = !options.ignore_trim_whitespace;

    let mut perfect_hashes: HashMap<&[u16], u32> = HashMap::new();
    let mut hashes: Vec<u32> = original_lines
        .iter()
        .chain(modified_lines)
        .map(|line| {
            let next = perfect_hashes.len() as u32;
            *perfect_hashes.entry(trim(line)).or_insert(next)
        })
        .collect();
    let modified_hashes = hashes.split_off(original_lines.len());
    let original_hashes = hashes;

    let sequence1 = LineSequence::new(&original_hashes, original_lines);
    let sequence2 = LineSequence::new(&modified_hashes, modified_lines);

    let line_alignment_result = if original_lines.len() + modified_lines.len() < 1700 {
        // Use the improved algorithm for small files.
        let equality_score = |offset1: usize, offset2: usize| {
            if original_lines[offset1] == modified_lines[offset2] {
                let len = modified_lines[offset2].len();
                if len == 0 {
                    0.1
                } else {
                    1.0 + (1.0 + len as f64).ln()
                }
            } else {
                0.99
            }
        };
        dynamic_programming_diff(&sequence1, &sequence2, &timeout, Some(&equality_score))
    } else {
        myers_diff(&sequence1, &sequence2, &timeout)
    };

    let mut hit_timeout = line_alignment_result.hit_timeout;
    let line_alignments =
        optimize_sequence_diffs(&sequence1, &sequence2, line_alignment_result.diffs);
    let line_alignments =
        remove_very_short_matching_lines_between_diffs(&sequence1, line_alignments);

    let mut alignments: Vec<RangeMapping> = Vec::new();

    let scan_for_whitespace_changes =
        |seq1_last_start: usize,
         seq2_last_start: usize,
         equal_lines_count: usize,
         alignments: &mut Vec<RangeMapping>,
         hit_timeout: &mut bool| {
            if !consider_whitespace_changes {
                return;
            }
            for i in 0..equal_lines_count {
                let seq1_offset = seq1_last_start + i;
                let seq2_offset = seq2_last_start + i;
                if original_lines[seq1_offset] != modified_lines[seq2_offset] {
                    // This is because of whitespace changes, diff these lines.
                    let character_diffs = refine_diff(
                        original_lines,
                        modified_lines,
                        SequenceDiff::new(
                            OffsetRange::new(seq1_offset, seq1_offset + 1),
                            OffsetRange::new(seq2_offset, seq2_offset + 1),
                        ),
                        &timeout,
                        consider_whitespace_changes,
                        options,
                    );
                    alignments.extend(character_diffs.diffs);
                    *hit_timeout |= character_diffs.hit_timeout;
                }
            }
        };

    let mut seq1_last_start = 0;
    let mut seq2_last_start = 0;
    for diff in &line_alignments {
        debug_assert_eq!(
            diff.seq1_range.start - seq1_last_start,
            diff.seq2_range.start - seq2_last_start
        );
        let equal_lines_count = diff.seq1_range.start - seq1_last_start;
        scan_for_whitespace_changes(
            seq1_last_start,
            seq2_last_start,
            equal_lines_count,
            &mut alignments,
            &mut hit_timeout,
        );

        seq1_last_start = diff.seq1_range.end_exclusive;
        seq2_last_start = diff.seq2_range.end_exclusive;

        let character_diffs = refine_diff(
            original_lines,
            modified_lines,
            *diff,
            &timeout,
            consider_whitespace_changes,
            options,
        );
        hit_timeout |= character_diffs.hit_timeout;
        alignments.extend(character_diffs.diffs);
    }
    scan_for_whitespace_changes(
        seq1_last_start,
        seq2_last_start,
        original_lines.len() - seq1_last_start,
        &mut alignments,
        &mut hit_timeout,
    );

    let changes =
        line_range_mapping_from_range_mappings(&alignments, original_lines, modified_lines);

    let moves = if options.compute_moves {
        compute_moves(
            &changes,
            original_lines,
            modified_lines,
            &original_hashes,
            &modified_hashes,
            &timeout,
            consider_whitespace_changes,
            options,
        )
    } else {
        Vec::new()
    };

    LinesDiff {
        changes,
        moves,
        hit_timeout,
    }
}

#[allow(clippy::too_many_arguments)]
fn compute_moves(
    changes: &[DetailedLineRangeMapping],
    original_lines: &[Vec<u16>],
    modified_lines: &[Vec<u16>],
    hashed_original_lines: &[u32],
    hashed_modified_lines: &[u32],
    timeout: &Timeout,
    consider_whitespace_changes: bool,
    options: &DiffOptions,
) -> Vec<MovedText> {
    let moves = compute_moved_lines(
        changes,
        original_lines,
        modified_lines,
        hashed_original_lines,
        hashed_modified_lines,
        timeout,
    );
    moves
        .into_iter()
        .map(|m| {
            let move_changes = refine_diff(
                original_lines,
                modified_lines,
                SequenceDiff::new(m.original.to_offset_range(), m.modified.to_offset_range()),
                timeout,
                consider_whitespace_changes,
                options,
            );
            let mappings = line_range_mapping_from_range_mappings(
                &move_changes.diffs,
                original_lines,
                modified_lines,
            );
            MovedText {
                line_range_mapping: m,
                changes: mappings,
            }
        })
        .collect()
}

/// Character-level changes of a line-level diff.
struct RefineResult {
    diffs: Vec<RangeMapping>,
    hit_timeout: bool,
}

fn refine_diff(
    original_lines: &[Vec<u16>],
    modified_lines: &[Vec<u16>],
    diff: SequenceDiff,
    timeout: &Timeout,
    consider_whitespace_changes: bool,
    options: &DiffOptions,
) -> RefineResult {
    let line_range_mapping = LineRangeMapping::new(
        LineRange::new(diff.seq1_range.start + 1, diff.seq1_range.end_exclusive + 1),
        LineRange::new(diff.seq2_range.start + 1, diff.seq2_range.end_exclusive + 1),
    );
    let range_mapping = line_range_mapping.to_range_mapping2(original_lines, modified_lines);

    let slice1 = LinesSliceCharSequence::new(
        original_lines,
        range_mapping.original_range,
        consider_whitespace_changes,
    );
    let slice2 = LinesSliceCharSequence::new(
        modified_lines,
        range_mapping.modified_range,
        consider_whitespace_changes,
    );

    use super::algorithms::Sequence as _;
    let DiffAlgorithmResult { diffs, hit_timeout } = if slice1.len() + slice2.len() < 500 {
        dynamic_programming_diff(&slice1, &slice2, timeout, None)
    } else {
        myers_diff(&slice1, &slice2, timeout)
    };

    let mut diffs = optimize_sequence_diffs(&slice1, &slice2, diffs);
    diffs = extend_diffs_to_entire_word_if_appropriate(
        &slice1,
        &slice2,
        diffs,
        LinesSliceCharSequence::find_word_containing,
        false,
    );
    if options.extend_to_subwords {
        diffs = extend_diffs_to_entire_word_if_appropriate(
            &slice1,
            &slice2,
            diffs,
            LinesSliceCharSequence::find_sub_word_containing,
            true,
        );
    }
    diffs = remove_short_matches(diffs);
    diffs = remove_very_short_matching_text_between_long_diffs(&slice1, &slice2, diffs);

    RefineResult {
        diffs: diffs
            .iter()
            .map(|d| {
                RangeMapping::new(
                    slice1.translate_range(d.seq1_range),
                    slice2.translate_range(d.seq2_range),
                )
            })
            .collect(),
        hit_timeout,
    }
}
