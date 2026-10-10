//! Port of Monaco's `defaultLinesDiffComputer.test.ts`, plus a few checks of
//! the public API.

use super::algorithms::{Timeout, dynamic_programming_diff, myers_diff};
use super::mapping::get_line_range_mapping;
use super::range::OffsetRange;
use super::sequences::LinesSliceCharSequence;
use super::*;

fn utf16(lines: &[&str]) -> Vec<Vec<u16>> {
    lines.iter().map(|l| l.encode_utf16().collect()).collect()
}

#[test]
fn myers_1() {
    let lines1 = utf16(&["hello world"]);
    let lines2 = utf16(&["hallo welt"]);
    let s1 = LinesSliceCharSequence::new(&lines1, Range::new(1, 1, 1, usize::MAX), true);
    let s2 = LinesSliceCharSequence::new(&lines2, Range::new(1, 1, 1, usize::MAX), true);
    let timeout = Timeout::infinite();
    let myers = myers_diff(&s1, &s2, &timeout);
    let dp = dynamic_programming_diff(&s1, &s2, &timeout, None);
    assert!(!myers.hit_timeout && !dp.hit_timeout);
    assert!(!myers.diffs.is_empty() && !dp.diffs.is_empty());
}

#[test]
fn line_range_mapping_simple() {
    let mapping = get_line_range_mapping(
        &RangeMapping::new(Range::new(2, 1, 3, 1), Range::new(2, 1, 2, 1)),
        &utf16(&["const abc = \"helloworld\".split(\"\");", "", ""]),
        &utf16(&["const asciiLower = \"helloworld\".split(\"\");", ""]),
    );
    assert_eq!(mapping.to_string(), "{[2,3)->[2,2)}");
}

#[test]
fn line_range_mapping_empty_lines() {
    let mapping = get_line_range_mapping(
        &RangeMapping::new(Range::new(2, 1, 2, 1), Range::new(2, 1, 4, 1)),
        &utf16(&["", ""]),
        &utf16(&["", "", "", ""]),
    );
    assert_eq!(mapping.to_string(), "{[2,2)->[2,4)}");
}

fn slice_sequence() -> LinesSliceCharSequence {
    LinesSliceCharSequence::new(
        &utf16(&[
            "line1: foo",
            "line2: fizzbuzz",
            "line3: barr",
            "line4: hello world",
            "line5: bazz",
        ]),
        Range::new(2, 1, 5, 1),
        true,
    )
}

#[test]
fn lines_slice_char_sequence_translate_offset() {
    use super::algorithms::Sequence as _;
    let sequence = slice_sequence();
    let actual: Vec<String> = (0..sequence.len())
        .map(|offset| sequence.translate_offset(offset, false).to_string())
        .collect();
    let mut expected = Vec::new();
    for (line, columns) in [(2, 16), (3, 12), (4, 19)] {
        for column in 1..=columns {
            expected.push(format!("({line},{column})"));
        }
    }
    assert_eq!(actual, expected);
}

#[test]
fn lines_slice_char_sequence_extend_to_full_lines() {
    let sequence = slice_sequence();
    let text =
        |range| String::from_utf16(sequence.text(sequence.extend_to_full_lines(range))).unwrap();
    assert_eq!(text(OffsetRange::new(20, 25)), "line3: barr\n");
    assert_eq!(
        text(OffsetRange::new(20, 45)),
        "line3: barr\nline4: hello world\n"
    );
}

#[test]
fn identical_and_trivial_inputs() {
    let options = DiffOptions::default();
    assert_eq!(compute_diff(&["a"], &["a"], &options), LinesDiff::default());
    let empty: [&str; 0] = [];
    assert_eq!(compute_diff(&empty, &[""], &options), LinesDiff::default());

    let diff = compute_diff(&[""], &["a", "bc"], &options);
    assert_eq!(diff.changes.len(), 1);
    assert_eq!(diff.changes[0].to_string(), "{[1,2)->[1,3)}");
    assert_eq!(
        diff.changes[0].inner_changes[0].to_string(),
        "{[1,1 -> 1,1]->[1,1 -> 2,3]}"
    );
}

#[test]
fn utf16_columns_and_point_conversion() {
    let options = DiffOptions::default();
    let diff = compute_diff(&["😀 a", "x"], &["😀 b", "x"], &options);
    assert_eq!(diff.changes.len(), 1);
    let inner = diff.changes[0].inner_changes[0];
    // The emoji is two UTF-16 code units.
    assert_eq!(inner.original_range.to_string(), "[1,4 -> 1,5]");
    let points = inner.original_range.to_point_utf16_range();
    assert_eq!(points.start, text::PointUtf16::new(0, 3));
    assert_eq!(points.end, text::PointUtf16::new(0, 4));
    assert_eq!(Range::from_point_utf16_range(points), inner.original_range);
    assert_eq!(diff.changes[0].original.to_rows(), 0..1);
}

#[test]
fn ignore_trim_whitespace() {
    let original = ["fn a() {", "x", "}"];
    let modified = ["fn a() {", "    x", "}"];
    let ignoring = compute_diff(&original, &modified, &DiffOptions::default());
    assert!(ignoring.changes.is_empty());
    let considering = compute_diff(
        &original,
        &modified,
        &DiffOptions {
            ignore_trim_whitespace: false,
            ..DiffOptions::default()
        },
    );
    assert_eq!(considering.changes.len(), 1);
    assert_eq!(considering.changes[0].to_string(), "{[2,3)->[2,3)}");
}

#[test]
fn extend_to_subwords() {
    let options = DiffOptions {
        extend_to_subwords: true,
        ..DiffOptions::default()
    };
    let diff = compute_diff(&["fooBarBaz"], &["fooQuxBaz"], &options);
    assert_eq!(
        diff.changes[0].inner_changes[0].to_string(),
        "{[1,4 -> 1,7]->[1,4 -> 1,7]}"
    );
}

#[test]
fn flip() {
    let diff = compute_diff(&["a", "b", "c"], &["a", "x", "c"], &DiffOptions::default());
    let change = &diff.changes[0];
    assert_eq!(change.flip().flip(), *change);
    assert_eq!(change.flip().original, change.modified);
}
