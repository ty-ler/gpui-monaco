//! Heuristics that make raw diffs more readable (port of Monaco's
//! `heuristicSequenceOptimizations.ts`).

use std::collections::VecDeque;

use super::algorithms::{OffsetPair, Sequence, SequenceDiff};
use super::range::OffsetRange;
use super::sequences::{LineSequence, LinesSliceCharSequence, trim};

pub(crate) fn optimize_sequence_diffs(
    sequence1: &impl Sequence,
    sequence2: &impl Sequence,
    diffs: Vec<SequenceDiff>,
) -> Vec<SequenceDiff> {
    let mut result = join_sequence_diffs_by_shifting(sequence1, sequence2, diffs);
    // Sometimes, calling this function twice improves the result.
    result = join_sequence_diffs_by_shifting(sequence1, sequence2, result);
    shift_sequence_diffs(sequence1, sequence2, result)
}

/// Fixes issues like this:
/// ```text
/// import { Baz, Bar } from "foo";
/// ```
/// <->
/// ```text
/// import { Baz, Bar, Foo } from "foo";
/// ```
/// Computed diff: [ {Add "," after Bar}, {Add "Foo " after space} ]
/// Improved diff: [ {Add ", Foo" after Bar} ]
fn join_sequence_diffs_by_shifting(
    sequence1: &impl Sequence,
    sequence2: &impl Sequence,
    diffs: Vec<SequenceDiff>,
) -> Vec<SequenceDiff> {
    let Some(&first) = diffs.first() else {
        return diffs;
    };

    // First move them all to the left as much as possible and join them if
    // possible.
    let mut result = vec![first];
    for &diff in &diffs[1..] {
        let prev = *result.last().expect("non-empty");
        let mut cur = diff;
        if cur.seq1_range.is_empty() || cur.seq2_range.is_empty() {
            let length = cur.seq1_range.start - prev.seq1_range.end_exclusive;
            let mut d = 1;
            while d <= length {
                if sequence1.element(cur.seq1_range.start - d)
                    != sequence1.element(cur.seq1_range.end_exclusive - d)
                    || sequence2.element(cur.seq2_range.start - d)
                        != sequence2.element(cur.seq2_range.end_exclusive - d)
                {
                    break;
                }
                d += 1;
            }
            d -= 1;

            if d == length {
                // Merge previous and current diff.
                *result.last_mut().expect("non-empty") = SequenceDiff::new(
                    OffsetRange::new(prev.seq1_range.start, cur.seq1_range.end_exclusive - length),
                    OffsetRange::new(prev.seq2_range.start, cur.seq2_range.end_exclusive - length),
                );
                continue;
            }
            cur = cur.delta(-(d as isize));
        }
        result.push(cur);
    }

    // Then move them all to the right and join them again if possible.
    let mut result2 = Vec::with_capacity(result.len());
    for i in 0..result.len() - 1 {
        let next = result[i + 1];
        let mut cur = result[i];
        if cur.seq1_range.is_empty() || cur.seq2_range.is_empty() {
            let length = next.seq1_range.start - cur.seq1_range.end_exclusive;
            let mut d = 0;
            while d < length {
                if !sequence1
                    .is_strongly_equal(cur.seq1_range.start + d, cur.seq1_range.end_exclusive + d)
                    || !sequence2.is_strongly_equal(
                        cur.seq2_range.start + d,
                        cur.seq2_range.end_exclusive + d,
                    )
                {
                    break;
                }
                d += 1;
            }

            if d == length {
                // Merge previous and current diff, write to result!
                result[i + 1] = SequenceDiff::new(
                    OffsetRange::new(cur.seq1_range.start + length, next.seq1_range.end_exclusive),
                    OffsetRange::new(cur.seq2_range.start + length, next.seq2_range.end_exclusive),
                );
                continue;
            }
            if d > 0 {
                cur = cur.delta(d as isize);
            }
        }
        result2.push(cur);
    }
    if let Some(&last) = result.last() {
        result2.push(last);
    }
    result2
}

// Align character level diffs at whitespace characters:
//
// import { IBar } from "foo";
// import { I[Arr, I]Bar } from "foo";
// ->
// import { [IArr, ]IBar } from "foo";
//
// import { ITransaction, observableValue, transaction } from 'vs/base/common/observable';
// import { ITransaction, observable[FromEvent, observable]Value, transaction } from 'vs/base/common/observable';
// ->
// import { ITransaction, [observableFromEvent, ]observableValue, transaction } from 'vs/base/common/observable';
//
// collectBrackets(level + 1, levelPerBracketType);
// collectBrackets(level + 1, levelPerBracket[ + 1, levelPerBracket]Type);
// ->
// collectBrackets(level + 1, [levelPerBracket + 1, ]levelPerBracketType);
fn shift_sequence_diffs(
    sequence1: &impl Sequence,
    sequence2: &impl Sequence,
    mut diffs: Vec<SequenceDiff>,
) -> Vec<SequenceDiff> {
    for i in 0..diffs.len() {
        let prev = i.checked_sub(1).map(|i| diffs[i]);
        let diff = diffs[i];
        let next = diffs.get(i + 1).copied();

        let seq1_valid_range = OffsetRange::new(
            prev.map_or(0, |p| p.seq1_range.end_exclusive + 1),
            next.map_or(sequence1.len(), |n| n.seq1_range.start - 1),
        );
        let seq2_valid_range = OffsetRange::new(
            prev.map_or(0, |p| p.seq2_range.end_exclusive + 1),
            next.map_or(sequence2.len(), |n| n.seq2_range.start - 1),
        );

        if diff.seq1_range.is_empty() {
            diffs[i] = shift_diff_to_better_position(
                diff,
                sequence1,
                sequence2,
                seq1_valid_range,
                seq2_valid_range,
            );
        } else if diff.seq2_range.is_empty() {
            diffs[i] = shift_diff_to_better_position(
                diff.swap(),
                sequence2,
                sequence1,
                seq2_valid_range,
                seq1_valid_range,
            )
            .swap();
        }
    }
    diffs
}

fn shift_diff_to_better_position(
    diff: SequenceDiff,
    sequence1: &impl Sequence,
    sequence2: &impl Sequence,
    seq1_valid_range: OffsetRange,
    seq2_valid_range: OffsetRange,
) -> SequenceDiff {
    // To prevent performance issues.
    const MAX_SHIFT_LIMIT: usize = 100;

    // Don't touch previous or next!
    let mut delta_before = 1;
    while diff.seq1_range.start >= seq1_valid_range.start + delta_before
        && diff.seq2_range.start >= seq2_valid_range.start + delta_before
        && sequence2.is_strongly_equal(
            diff.seq2_range.start - delta_before,
            diff.seq2_range.end_exclusive - delta_before,
        )
        && delta_before < MAX_SHIFT_LIMIT
    {
        delta_before += 1;
    }
    delta_before -= 1;

    let mut delta_after = 0;
    while diff.seq1_range.start + delta_after < seq1_valid_range.end_exclusive
        && diff.seq2_range.end_exclusive + delta_after < seq2_valid_range.end_exclusive
        && sequence2.is_strongly_equal(
            diff.seq2_range.start + delta_after,
            diff.seq2_range.end_exclusive + delta_after,
        )
        && delta_after < MAX_SHIFT_LIMIT
    {
        delta_after += 1;
    }

    if delta_before == 0 && delta_after == 0 {
        return diff;
    }

    // Visualize `[sequence1.text, diff.seq1Range.start + deltaAfter]`
    // and `[sequence2.text, diff.seq2Range.start + deltaAfter, diff.seq2Range.endExclusive + deltaAfter]`
    let mut best_delta = 0isize;
    let mut best_score = -1;
    for delta in -(delta_before as isize)..=delta_after as isize {
        let seq2_start = diff.seq2_range.start.saturating_add_signed(delta);
        let seq2_end = diff.seq2_range.end_exclusive.saturating_add_signed(delta);
        let seq1_offset = diff.seq1_range.start.saturating_add_signed(delta);
        let score = sequence1.boundary_score(seq1_offset)
            + sequence2.boundary_score(seq2_start)
            + sequence2.boundary_score(seq2_end);
        if score > best_score {
            best_score = score;
            best_delta = delta;
        }
    }
    diff.delta(best_delta)
}

pub(crate) fn remove_short_matches(diffs: Vec<SequenceDiff>) -> Vec<SequenceDiff> {
    let mut result: Vec<SequenceDiff> = Vec::with_capacity(diffs.len());
    for s in diffs {
        if let Some(last) = result.last_mut()
            && (s.seq1_range.start - last.seq1_range.end_exclusive <= 2
                || s.seq2_range.start - last.seq2_range.end_exclusive <= 2)
        {
            *last = last.join(&s);
        } else {
            result.push(s);
        }
    }
    result
}

/// Extends diffs to whole (sub)words if a large part of the word changed.
/// `find_parent` returns the (sub)word containing an offset.
pub(crate) fn extend_diffs_to_entire_word_if_appropriate(
    sequence1: &LinesSliceCharSequence,
    sequence2: &LinesSliceCharSequence,
    diffs: Vec<SequenceDiff>,
    find_parent: impl Fn(&LinesSliceCharSequence, usize) -> Option<OffsetRange>,
    force: bool,
) -> Vec<SequenceDiff> {
    let mut equal_mappings: VecDeque<SequenceDiff> =
        SequenceDiff::invert(&diffs, sequence1.len()).into();
    let mut additional = Vec::new();
    let mut last_point = OffsetPair::ZERO;

    let mut scan_word = |pair: OffsetPair,
                         equal_mapping: SequenceDiff,
                         equal_mappings: &mut VecDeque<SequenceDiff>| {
        if pair.offset1 < last_point.offset1 || pair.offset2 < last_point.offset2 {
            return;
        }
        let (Some(w1), Some(w2)) = (
            find_parent(sequence1, pair.offset1),
            find_parent(sequence2, pair.offset2),
        ) else {
            return;
        };
        let mut w = SequenceDiff::new(w1, w2);
        let Some(equal_part) = w.intersect(&equal_mapping) else {
            return;
        };
        let mut equal_chars1 = equal_part.seq1_range.len();
        let mut equal_chars2 = equal_part.seq2_range.len();

        // The words do not touch previous equals mappings, as we would have
        // processed them already. But they might touch the next ones.
        while let Some(&next) = equal_mappings.front() {
            let intersects = next.seq1_range.intersects(&w.seq1_range)
                || next.seq2_range.intersects(&w.seq2_range);
            if !intersects {
                break;
            }
            // Because there is an intersection, the words are not empty.
            let (Some(v1), Some(v2)) = (
                find_parent(sequence1, next.seq1_range.start),
                find_parent(sequence2, next.seq2_range.start),
            ) else {
                break;
            };
            let v = SequenceDiff::new(v1, v2);
            let Some(equal_part) = v.intersect(&next) else {
                break;
            };
            equal_chars1 += equal_part.seq1_range.len();
            equal_chars2 += equal_part.seq2_range.len();
            w = w.join(&v);

            if w.seq1_range.end_exclusive >= next.seq1_range.end_exclusive {
                // The word extends beyond the next equal mapping.
                equal_mappings.pop_front();
            } else {
                break;
            }
        }

        let equal_chars = (equal_chars1 + equal_chars2) as f64;
        let word_chars = (w.seq1_range.len() + w.seq2_range.len()) as f64;
        if (force && equal_chars < word_chars) || equal_chars < word_chars * 2.0 / 3.0 {
            additional.push(w);
        }
        last_point = w.end_exclusives();
    };

    while let Some(next) = equal_mappings.pop_front() {
        if next.seq1_range.is_empty() {
            continue;
        }
        scan_word(next.starts(), next, &mut equal_mappings);
        // The equal parts are not empty, so -1 gives us a character that is
        // equal in both parts.
        scan_word(next.end_exclusives().delta(-1), next, &mut equal_mappings);
    }

    merge_sequence_diffs(diffs, additional)
}

fn merge_sequence_diffs(diffs1: Vec<SequenceDiff>, diffs2: Vec<SequenceDiff>) -> Vec<SequenceDiff> {
    let mut result: Vec<SequenceDiff> = Vec::with_capacity(diffs1.len() + diffs2.len());
    let mut iter1 = diffs1.into_iter().peekable();
    let mut iter2 = diffs2.into_iter().peekable();
    loop {
        let take_first = match (iter1.peek(), iter2.peek()) {
            (None, None) => break,
            (Some(sd1), Some(sd2)) => sd1.seq1_range.start < sd2.seq1_range.start,
            (Some(_), None) => true,
            (None, Some(_)) => false,
        };
        let next = if take_first {
            iter1.next()
        } else {
            iter2.next()
        }
        .expect("peeked");
        if let Some(last) = result.last_mut()
            && last.seq1_range.end_exclusive >= next.seq1_range.start
        {
            *last = last.join(&next);
        } else {
            result.push(next);
        }
    }
    result
}

/// Repeatedly (at most 11 times) joins neighboring diffs for which
/// `should_join(before, after)` holds.
fn join_repeatedly(
    mut diffs: Vec<SequenceDiff>,
    should_join: impl Fn(&SequenceDiff, &SequenceDiff) -> bool,
) -> Vec<SequenceDiff> {
    if diffs.is_empty() {
        return diffs;
    }
    let mut counter = 0;
    loop {
        let mut should_repeat = false;
        let mut result = vec![diffs[0]];
        for cur in &diffs[1..] {
            let last = result.last_mut().expect("non-empty");
            if should_join(last, cur) {
                should_repeat = true;
                *last = last.join(cur);
            } else {
                result.push(*cur);
            }
        }
        diffs = result;
        let again = counter < 10 && should_repeat;
        counter += 1;
        if !again {
            return diffs;
        }
    }
}

pub(crate) fn remove_very_short_matching_lines_between_diffs(
    sequence1: &LineSequence<'_>,
    diffs: Vec<SequenceDiff>,
) -> Vec<SequenceDiff> {
    join_repeatedly(diffs, |before, after| {
        let unchanged_range =
            OffsetRange::new(before.seq1_range.end_exclusive, after.seq1_range.start);
        sequence1.non_whitespace_len(unchanged_range) <= 4
            && (before.seq1_range.len() + before.seq2_range.len() > 5
                || after.seq1_range.len() + after.seq2_range.len() > 5)
    })
}

pub(crate) fn remove_very_short_matching_text_between_long_diffs(
    sequence1: &LinesSliceCharSequence,
    sequence2: &LinesSliceCharSequence,
    diffs: Vec<SequenceDiff>,
) -> Vec<SequenceDiff> {
    let diffs = join_repeatedly(diffs, |before, after| {
        let unchanged_range =
            OffsetRange::new(before.seq1_range.end_exclusive, after.seq1_range.start);
        let unchanged_line_count = sequence1.count_lines_in(unchanged_range);
        if unchanged_line_count > 5 || unchanged_range.len() > 500 {
            return false;
        }

        let unchanged_text = trim(sequence1.text(unchanged_range));
        if unchanged_text.len() > 20
            || unchanged_text
                .iter()
                .any(|&c| c == u16::from(b'\n') || c == u16::from(b'\r'))
        {
            return false;
        }

        let before_line_count1 = sequence1.count_lines_in(before.seq1_range);
        let before_seq1_length = before.seq1_range.len();
        let before_line_count2 = sequence2.count_lines_in(before.seq2_range);
        let before_seq2_length = before.seq2_range.len();

        let after_line_count1 = sequence1.count_lines_in(after.seq1_range);
        let after_seq1_length = after.seq1_range.len();
        let after_line_count2 = sequence2.count_lines_in(after.seq2_range);
        let after_seq2_length = after.seq2_range.len();

        // TODO: Maybe a neural net can be used to derive the result from these numbers.
        const MAX: f64 = 2.0 * 40.0 + 50.0;
        let cap = |lines: usize, length: usize| ((lines * 40 + length) as f64).min(MAX);

        (cap(before_line_count1, before_seq1_length).powf(1.5)
            + cap(before_line_count2, before_seq2_length).powf(1.5))
        .powf(1.5)
            + (cap(after_line_count1, after_seq1_length).powf(1.5)
                + cap(after_line_count2, after_seq2_length).powf(1.5))
            .powf(1.5)
            > MAX.powf(1.5).powf(1.5) * 1.3
    });

    // Remove short suffixes/prefixes.
    let mut new_diffs: Vec<SequenceDiff> = Vec::with_capacity(diffs.len());
    for (i, &cur) in diffs.iter().enumerate() {
        let prev = i.checked_sub(1).map(|i| diffs[i]);
        let next = diffs.get(i + 1);
        let mut new_diff = cur;

        let should_mark_as_changed = |text: &[u16]| {
            !text.is_empty()
                && trim(text).len() <= 3
                && cur.seq1_range.len() + cur.seq2_range.len() > 100
        };

        let full_range1 = sequence1.extend_to_full_lines(cur.seq1_range);
        let prefix = sequence1.text(OffsetRange::new(full_range1.start, cur.seq1_range.start));
        if should_mark_as_changed(prefix) {
            new_diff = new_diff.delta_start(-(prefix.len() as isize));
        }
        let suffix = sequence1.text(OffsetRange::new(
            cur.seq1_range.end_exclusive,
            full_range1.end_exclusive,
        ));
        if should_mark_as_changed(suffix) {
            new_diff = new_diff.delta_end(suffix.len() as isize);
        }

        let available_space = SequenceDiff::from_offset_pairs(
            prev.map_or(OffsetPair::ZERO, |p| p.end_exclusives()),
            next.map_or(OffsetPair::MAX, SequenceDiff::starts),
        );
        let Some(result) = new_diff.intersect(&available_space) else {
            new_diffs.push(cur);
            continue;
        };
        if let Some(last) = new_diffs.last_mut()
            && result.starts() == last.end_exclusives()
        {
            *last = last.join(&result);
        } else {
            new_diffs.push(result);
        }
    }
    new_diffs
}
