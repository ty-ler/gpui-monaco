//! Moved-code detection (port of Monaco's `computeMovedLines.ts` and the
//! `LineRangeFragment` helper from `utils.ts`).

use std::collections::HashMap;

use super::algorithms::{SequenceDiff, Timeout, myers_diff};
use super::mapping::{DetailedLineRangeMapping, LineRangeMapping};
use super::range::{LineRange, LineRangeSet, MonotonousArray, Range, find_last_monotonous};
use super::sequences::{LinesSliceCharSequence, is_space, trim};

/// Finds blocks of lines that moved, as line range mappings sorted by their
/// original range.
pub(crate) fn compute_moved_lines(
    changes: &[DetailedLineRangeMapping],
    original_lines: &[Vec<u16>],
    modified_lines: &[Vec<u16>],
    hashed_original_lines: &[u32],
    hashed_modified_lines: &[u32],
    timeout: &Timeout,
) -> Vec<LineRangeMapping> {
    let changes: Vec<LineRangeMapping> = changes
        .iter()
        .map(DetailedLineRangeMapping::line_range_mapping)
        .collect();
    let (mut moves, excluded_changes) = compute_moves_from_simple_deletions_to_simple_insertions(
        &changes,
        original_lines,
        modified_lines,
        timeout,
    );
    if !timeout.is_valid() {
        return Vec::new();
    }

    let mut filtered_changes: Vec<LineRangeMapping> = changes
        .iter()
        .enumerate()
        .filter(|(i, _)| !excluded_changes.contains(i))
        .map(|(_, c)| *c)
        .collect();
    let unchanged_moves = compute_unchanged_moves(
        &mut filtered_changes,
        hashed_original_lines,
        hashed_modified_lines,
        original_lines,
        modified_lines,
        timeout,
    );
    moves.extend(unchanged_moves);

    let mut moves = join_close_consecutive_moves(moves);
    // Ignore too short moves.
    moves.retain(|current| {
        let lines: Vec<&[u16]> = original_lines[current.original.start_line_number - 1
            ..current.original.end_line_number_exclusive - 1]
            .iter()
            .map(|l| trim(l))
            .collect();
        let original_text_len =
            lines.iter().map(|l| l.len()).sum::<usize>() + lines.len().saturating_sub(1);
        original_text_len >= 15 && lines.iter().filter(|l| l.len() >= 2).count() >= 2
    });
    remove_moves_in_same_diff(&changes, moves)
}

/// A histogram of the characters (UTF-16 code units, plus one `\n` per line)
/// of a line range (Monaco's `LineRangeFragment`).
struct LineRangeFragment {
    range: LineRange,
    /// Index of the change this fragment comes from.
    source: usize,
    total_count: usize,
    histogram: HashMap<u16, usize>,
}

impl LineRangeFragment {
    fn new(range: LineRange, lines: &[Vec<u16>], source: usize) -> Self {
        let mut total_count = 0;
        let mut histogram = HashMap::new();
        for line in &lines[range.start_line_number - 1..range.end_line_number_exclusive - 1] {
            for &c in line.iter().chain(&[u16::from(b'\n')]) {
                total_count += 1;
                *histogram.entry(c).or_insert(0) += 1;
            }
        }
        Self {
            range,
            source,
            total_count,
            histogram,
        }
    }

    fn compute_similarity(&self, other: &Self) -> f64 {
        let mut sum_differences = 0;
        for (c, &count) in &self.histogram {
            sum_differences += count.abs_diff(other.histogram.get(c).copied().unwrap_or(0));
        }
        for (c, &count) in &other.histogram {
            if !self.histogram.contains_key(c) {
                sum_differences += count;
            }
        }
        1.0 - sum_differences as f64 / (self.total_count + other.total_count) as f64
    }
}

/// Matches pure deletions to similar pure insertions. Returns the moves and
/// the indices of the changes they consumed.
fn compute_moves_from_simple_deletions_to_simple_insertions(
    changes: &[LineRangeMapping],
    original_lines: &[Vec<u16>],
    modified_lines: &[Vec<u16>],
    timeout: &Timeout,
) -> (Vec<LineRangeMapping>, Vec<usize>) {
    let mut moves = Vec::new();

    let deletions: Vec<LineRangeFragment> = changes
        .iter()
        .enumerate()
        .filter(|(_, c)| c.modified.is_empty() && c.original.len() >= 3)
        .map(|(i, c)| LineRangeFragment::new(c.original, original_lines, i))
        .collect();
    let mut insertions: Vec<LineRangeFragment> = changes
        .iter()
        .enumerate()
        .filter(|(_, c)| c.original.is_empty() && c.modified.len() >= 3)
        .map(|(i, c)| LineRangeFragment::new(c.modified, modified_lines, i))
        .collect();

    let mut excluded_changes = Vec::new();

    for deletion in &deletions {
        let mut highest_similarity = -1.0;
        let mut best = None;
        for (i, insertion) in insertions.iter().enumerate() {
            let similarity = deletion.compute_similarity(insertion);
            if similarity > highest_similarity {
                highest_similarity = similarity;
                best = Some(i);
            }
        }

        if highest_similarity > 0.90
            && let Some(best) = best
        {
            let best = insertions.remove(best);
            moves.push(LineRangeMapping::new(deletion.range, best.range));
            excluded_changes.push(deletion.source);
            excluded_changes.push(best.source);
        }

        if !timeout.is_valid() {
            break;
        }
    }

    (moves, excluded_changes)
}

struct PossibleMapping {
    modified_line_range: LineRange,
    original_line_range: LineRange,
}

/// Finds moved blocks of at least three identical (modulo surrounding
/// whitespace) lines, extended by similar lines around them.
fn compute_unchanged_moves(
    changes: &mut [LineRangeMapping],
    hashed_original_lines: &[u32],
    hashed_modified_lines: &[u32],
    original_lines: &[Vec<u16>],
    modified_lines: &[Vec<u16>],
    timeout: &Timeout,
) -> Vec<LineRangeMapping> {
    let mut moves = Vec::new();

    let key = |hashes: &[u32], i: usize| (hashes[i - 1], hashes[i], hashes[i + 1]);

    let mut original_3_line_hashes: HashMap<(u32, u32, u32), Vec<LineRange>> = HashMap::new();
    for change in changes.iter() {
        for i in change.original.start_line_number
            ..change.original.end_line_number_exclusive.saturating_sub(2)
        {
            original_3_line_hashes
                .entry(key(hashed_original_lines, i))
                .or_default()
                .push(LineRange::new(i, i + 3));
        }
    }

    let mut possible_mappings: Vec<PossibleMapping> = Vec::new();

    changes.sort_by_key(|c| c.modified.start_line_number);

    for change in changes.iter() {
        let mut last_mappings: Vec<usize> = Vec::new();
        for i in change.modified.start_line_number
            ..change.modified.end_line_number_exclusive.saturating_sub(2)
        {
            let current_modified_range = LineRange::new(i, i + 3);
            let mut next_mappings = Vec::new();
            if let Some(ranges) = original_3_line_hashes.get(&key(hashed_modified_lines, i)) {
                'ranges: for range in ranges {
                    for &last in &last_mappings {
                        let last_mapping: &mut PossibleMapping = &mut possible_mappings[last];
                        // Does this match extend some last match?
                        if last_mapping.original_line_range.end_line_number_exclusive + 1
                            == range.end_line_number_exclusive
                            && last_mapping.modified_line_range.end_line_number_exclusive + 1
                                == current_modified_range.end_line_number_exclusive
                        {
                            last_mapping.original_line_range = LineRange::new(
                                last_mapping.original_line_range.start_line_number,
                                range.end_line_number_exclusive,
                            );
                            last_mapping.modified_line_range = LineRange::new(
                                last_mapping.modified_line_range.start_line_number,
                                current_modified_range.end_line_number_exclusive,
                            );
                            next_mappings.push(last);
                            continue 'ranges;
                        }
                    }
                    possible_mappings.push(PossibleMapping {
                        modified_line_range: current_modified_range,
                        original_line_range: *range,
                    });
                    next_mappings.push(possible_mappings.len() - 1);
                }
            }
            last_mappings = next_mappings;
        }

        if !timeout.is_valid() {
            return Vec::new();
        }
    }

    possible_mappings.sort_by(|a, b| {
        b.modified_line_range
            .len()
            .cmp(&a.modified_line_range.len())
    });

    let mut modified_set = LineRangeSet::default();
    let mut original_set = LineRangeSet::default();

    for mapping in &possible_mappings {
        let diff_orig_to_mod = mapping.modified_line_range.start_line_number as isize
            - mapping.original_line_range.start_line_number as isize;
        let modified_sections = modified_set.subtract_from(mapping.modified_line_range);
        let original_translated_sections = original_set
            .subtract_from(mapping.original_line_range)
            .with_delta(diff_orig_to_mod);

        let modified_intersected_sections =
            modified_sections.intersection(&original_translated_sections);

        for s in modified_intersected_sections.ranges() {
            if s.len() < 3 {
                continue;
            }
            let modified_line_range = *s;
            let original_line_range = s.delta(-diff_orig_to_mod);

            moves.push(LineRangeMapping::new(
                original_line_range,
                modified_line_range,
            ));

            modified_set.add_range(modified_line_range);
            original_set.add_range(original_line_range);
        }
    }

    moves.sort_by_key(|m| m.original.start_line_number);

    let changes: &[LineRangeMapping] = changes;
    let mut monotonous_changes = MonotonousArray::new(changes);
    for move_slot in &mut moves {
        let mv = *move_slot;
        let first_touching_change_orig = monotonous_changes.find_last_monotonous(|c| {
            c.original.start_line_number <= mv.original.start_line_number
        });
        let first_touching_change_mod = find_last_monotonous(changes, |c| {
            c.modified.start_line_number <= mv.modified.start_line_number
        });
        let last_touching_change_orig = monotonous_changes.find_last_monotonous(|c| {
            c.original.start_line_number < mv.original.end_line_number_exclusive
        });
        let last_touching_change_mod = find_last_monotonous(changes, |c| {
            c.modified.start_line_number < mv.modified.end_line_number_exclusive
        });
        // Monaco asserts these exist.
        let (
            Some(first_touching_change_orig),
            Some(first_touching_change_mod),
            Some(last_touching_change_orig),
            Some(last_touching_change_mod),
        ) = (
            first_touching_change_orig,
            first_touching_change_mod,
            last_touching_change_orig,
            last_touching_change_mod,
        )
        else {
            continue;
        };

        let lines_above = (mv.original.start_line_number as isize
            - changes[first_touching_change_orig]
                .original
                .start_line_number as isize)
            .max(
                mv.modified.start_line_number as isize
                    - changes[first_touching_change_mod]
                        .modified
                        .start_line_number as isize,
            );
        let lines_below = (changes[last_touching_change_orig]
            .original
            .end_line_number_exclusive as isize
            - mv.original.end_line_number_exclusive as isize)
            .max(
                changes[last_touching_change_mod]
                    .modified
                    .end_line_number_exclusive as isize
                    - mv.modified.end_line_number_exclusive as isize,
            );

        let is_similar_pair =
            |orig_line: isize, mod_line: isize, sets: (&LineRangeSet, &LineRangeSet)| {
                if orig_line < 1
                    || mod_line < 1
                    || orig_line as usize > original_lines.len()
                    || mod_line as usize > modified_lines.len()
                {
                    return false;
                }
                let (orig_line, mod_line) = (orig_line as usize, mod_line as usize);
                if sets.1.contains(mod_line) || sets.0.contains(orig_line) {
                    return false;
                }
                are_lines_similar(
                    &original_lines[orig_line - 1],
                    &modified_lines[mod_line - 1],
                    timeout,
                )
            };

        let mut extend_to_top = 0;
        while extend_to_top < lines_above {
            let orig_line = mv.original.start_line_number as isize - extend_to_top - 1;
            let mod_line = mv.modified.start_line_number as isize - extend_to_top - 1;
            if !is_similar_pair(orig_line, mod_line, (&original_set, &modified_set)) {
                break;
            }
            extend_to_top += 1;
        }
        let extend_to_top = extend_to_top as usize;

        if extend_to_top > 0 {
            original_set.add_range(LineRange::new(
                mv.original.start_line_number - extend_to_top,
                mv.original.start_line_number,
            ));
            modified_set.add_range(LineRange::new(
                mv.modified.start_line_number - extend_to_top,
                mv.modified.start_line_number,
            ));
        }

        let mut extend_to_bottom = 0;
        while extend_to_bottom < lines_below {
            let orig_line = mv.original.end_line_number_exclusive as isize + extend_to_bottom;
            let mod_line = mv.modified.end_line_number_exclusive as isize + extend_to_bottom;
            if !is_similar_pair(orig_line, mod_line, (&original_set, &modified_set)) {
                break;
            }
            extend_to_bottom += 1;
        }
        let extend_to_bottom = extend_to_bottom as usize;

        if extend_to_bottom > 0 {
            original_set.add_range(LineRange::new(
                mv.original.end_line_number_exclusive,
                mv.original.end_line_number_exclusive + extend_to_bottom,
            ));
            modified_set.add_range(LineRange::new(
                mv.modified.end_line_number_exclusive,
                mv.modified.end_line_number_exclusive + extend_to_bottom,
            ));
        }

        if extend_to_top > 0 || extend_to_bottom > 0 {
            *move_slot = LineRangeMapping::new(
                LineRange::new(
                    mv.original.start_line_number - extend_to_top,
                    mv.original.end_line_number_exclusive + extend_to_bottom,
                ),
                LineRange::new(
                    mv.modified.start_line_number - extend_to_top,
                    mv.modified.end_line_number_exclusive + extend_to_bottom,
                ),
            );
        }
    }

    moves
}

/// Whether two lines share more than 60% of their non-whitespace characters.
///
/// Ported with Monaco's quirks: the compared sequences exclude each line's
/// last character, and the counts index the untrimmed first line.
fn are_lines_similar(line1: &[u16], line2: &[u16], timeout: &Timeout) -> bool {
    if trim(line1) == trim(line2) {
        return true;
    }
    if line1.len() > 300 && line2.len() > 300 {
        return false;
    }

    let line1_lines = [line1.to_vec()];
    let line2_lines = [line2.to_vec()];
    let result = myers_diff(
        &LinesSliceCharSequence::new(&line1_lines, Range::new(1, 1, 1, line1.len()), false),
        &LinesSliceCharSequence::new(&line2_lines, Range::new(1, 1, 1, line2.len()), false),
        timeout,
    );
    let mut common_non_space_char_count = 0;
    for seq in SequenceDiff::invert(&result.diffs, line1.len()) {
        for idx in seq.seq1_range.iter() {
            if !line1.get(idx).is_some_and(|&c| is_space(c)) {
                common_non_space_char_count += 1;
            }
        }
    }

    let count_non_ws_chars = |s: &[u16]| {
        // Monaco iterates up to `line1.length` for both lines; reading past
        // the end of a JS string yields NaN, which counts as non-space.
        (0..line1.len())
            .filter(|&i| !s.get(i).is_some_and(|&c| is_space(c)))
            .count()
    };

    let longer_line_length = count_non_ws_chars(if line1.len() > line2.len() {
        line1
    } else {
        line2
    });
    longer_line_length > 10 && common_non_space_char_count as f64 / longer_line_length as f64 > 0.6
}

fn join_close_consecutive_moves(mut moves: Vec<LineRangeMapping>) -> Vec<LineRangeMapping> {
    if moves.is_empty() {
        return moves;
    }
    moves.sort_by_key(|m| m.original.start_line_number);

    let mut result = vec![moves[0]];
    for current in &moves[1..] {
        let last = result.last_mut().expect("non-empty");
        let original_dist = current.original.start_line_number as isize
            - last.original.end_line_number_exclusive as isize;
        let modified_dist = current.modified.start_line_number as isize
            - last.modified.end_line_number_exclusive as isize;
        let current_move_after_last = original_dist >= 0 && modified_dist >= 0;

        if current_move_after_last && original_dist + modified_dist <= 2 {
            *last = last.join(current);
            continue;
        }
        result.push(*current);
    }
    result
}

/// Removes moves whose ends fall into the same change on both sides.
fn remove_moves_in_same_diff(
    changes: &[LineRangeMapping],
    mut moves: Vec<LineRangeMapping>,
) -> Vec<LineRangeMapping> {
    let mut changes_monotonous = MonotonousArray::new(changes);
    moves.retain(|m| {
        let diff_before_end_of_move_original = changes_monotonous.find_last_monotonous(|c| {
            c.original.start_line_number < m.original.end_line_number_exclusive
        });
        let diff_before_end_of_move_modified = find_last_monotonous(changes, |c| {
            c.modified.start_line_number < m.modified.end_line_number_exclusive
        });
        // Monaco compares object identity, with a fresh placeholder (never
        // equal) when no original change is found.
        diff_before_end_of_move_original.is_none()
            || diff_before_end_of_move_original != diff_before_end_of_move_modified
    });
    moves
}
