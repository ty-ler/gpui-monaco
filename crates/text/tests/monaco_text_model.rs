//! Conformance: ports of Monaco's `validatePosition`, `validateRange` and
//! `modifyPosition` tests from src/vs/editor/test/common/model/textModel.test.ts
//! (MIT). Monaco positions are one-based `(lineNumber, column)` with UTF-16
//! columns; `PointUtf16::{from,to}_monaco` convert. Float/NaN inputs have no
//! Rust equivalent and are skipped.

use text::{Bias, OffsetUtf16, PointUtf16, Rope};

fn validate_position(rope: &Rope, line: u32, column: u32) -> (u32, u32) {
    rope.clip_point_utf16(PointUtf16::from_monaco(line, column), Bias::Left)
        .to_monaco()
}

fn validate_range(rope: &Rope, range: (u32, u32, u32, u32)) -> (u32, u32, u32, u32) {
    let start = PointUtf16::from_monaco(range.0, range.1);
    let end = PointUtf16::from_monaco(range.2, range.3);
    let clipped = rope.clip_range_utf16(start..end);
    let (sl, sc) = clipped.start.to_monaco();
    let (el, ec) = clipped.end.to_monaco();
    (sl, sc, el, ec)
}

fn modify_position(rope: &Rope, line: u32, column: u32, delta: isize) -> (u32, u32) {
    let start = rope.clip_point_utf16(PointUtf16::from_monaco(line, column), Bias::Left);
    let offset = rope.locate(start).offset_utf16.0 as isize + delta;
    let offset = offset.clamp(0, rope.len_utf16().0 as isize) as usize;
    rope.locate(OffsetUtf16(offset)).point_utf16.to_monaco()
}

#[test]
fn validate_position_basic() {
    let m = Rope::from("line one\nline two");
    let cases = [
        ((0, 0), (1, 1)),
        ((0, 1), (1, 1)),
        ((1, 1), (1, 1)),
        ((1, 2), (1, 2)),
        ((1, 30), (1, 9)),
        ((2, 0), (2, 1)),
        ((2, 1), (2, 1)),
        ((2, 2), (2, 2)),
        ((2, 30), (2, 9)),
        ((3, 0), (2, 9)),
        ((3, 1), (2, 9)),
        ((3, 30), (2, 9)),
        ((30, 30), (2, 9)),
        ((u32::MAX, u32::MAX), (2, 9)),
    ];
    for ((line, column), expected) in cases {
        assert_eq!(
            validate_position(&m, line, column),
            expected,
            "({line}, {column})"
        );
    }
}

#[test]
fn validate_position_around_surrogate_pairs_1() {
    let m = Rope::from("a📚b");
    let cases = [
        ((0, 0), (1, 1)),
        ((0, 1), (1, 1)),
        ((0, 7), (1, 1)),
        ((1, 1), (1, 1)),
        ((1, 2), (1, 2)),
        ((1, 3), (1, 2)),
        ((1, 4), (1, 4)),
        ((1, 5), (1, 5)),
        ((1, 30), (1, 5)),
        ((2, 0), (1, 5)),
        ((2, 1), (1, 5)),
        ((2, 2), (1, 5)),
        ((2, 30), (1, 5)),
        ((u32::MAX, u32::MAX), (1, 5)),
    ];
    for ((line, column), expected) in cases {
        assert_eq!(
            validate_position(&m, line, column),
            expected,
            "({line}, {column})"
        );
    }
}

#[test]
fn validate_position_around_surrogate_pairs_2() {
    let m = Rope::from("a📚📚b");
    let cases = [
        ((1, 1), (1, 1)),
        ((1, 2), (1, 2)),
        ((1, 3), (1, 2)),
        ((1, 4), (1, 4)),
        ((1, 5), (1, 4)),
        ((1, 6), (1, 6)),
        ((1, 7), (1, 7)),
    ];
    for ((line, column), expected) in cases {
        assert_eq!(
            validate_position(&m, line, column),
            expected,
            "({line}, {column})"
        );
    }
}

#[test]
fn validate_range_around_surrogate_pairs_1() {
    let m = Rope::from("a📚b");
    let cases = [
        ((0, 0, 0, 1), (1, 1, 1, 1)),
        ((0, 0, 0, 7), (1, 1, 1, 1)),
        ((1, 1, 1, 1), (1, 1, 1, 1)),
        ((1, 1, 1, 2), (1, 1, 1, 2)),
        ((1, 1, 1, 3), (1, 1, 1, 4)),
        ((1, 1, 1, 4), (1, 1, 1, 4)),
        ((1, 1, 1, 5), (1, 1, 1, 5)),
        ((1, 2, 1, 2), (1, 2, 1, 2)),
        ((1, 2, 1, 3), (1, 2, 1, 4)),
        ((1, 2, 1, 4), (1, 2, 1, 4)),
        ((1, 2, 1, 5), (1, 2, 1, 5)),
        ((1, 3, 1, 3), (1, 2, 1, 2)),
        ((1, 3, 1, 4), (1, 2, 1, 4)),
        ((1, 3, 1, 5), (1, 2, 1, 5)),
        ((1, 4, 1, 4), (1, 4, 1, 4)),
        ((1, 4, 1, 5), (1, 4, 1, 5)),
        ((1, 5, 1, 5), (1, 5, 1, 5)),
    ];
    for (range, expected) in cases {
        assert_eq!(validate_range(&m, range), expected, "{range:?}");
    }
}

#[test]
fn validate_range_around_surrogate_pairs_2() {
    let m = Rope::from("a📚📚b");
    let cases = [
        ((0, 0, 0, 1), (1, 1, 1, 1)),
        ((0, 0, 0, 7), (1, 1, 1, 1)),
        ((1, 1, 1, 1), (1, 1, 1, 1)),
        ((1, 1, 1, 2), (1, 1, 1, 2)),
        ((1, 1, 1, 3), (1, 1, 1, 4)),
        ((1, 1, 1, 4), (1, 1, 1, 4)),
        ((1, 1, 1, 5), (1, 1, 1, 6)),
        ((1, 1, 1, 6), (1, 1, 1, 6)),
        ((1, 1, 1, 7), (1, 1, 1, 7)),
        ((1, 2, 1, 2), (1, 2, 1, 2)),
        ((1, 2, 1, 3), (1, 2, 1, 4)),
        ((1, 2, 1, 4), (1, 2, 1, 4)),
        ((1, 2, 1, 5), (1, 2, 1, 6)),
        ((1, 2, 1, 6), (1, 2, 1, 6)),
        ((1, 2, 1, 7), (1, 2, 1, 7)),
        ((1, 3, 1, 3), (1, 2, 1, 2)),
        ((1, 3, 1, 4), (1, 2, 1, 4)),
        ((1, 3, 1, 5), (1, 2, 1, 6)),
        ((1, 3, 1, 6), (1, 2, 1, 6)),
        ((1, 3, 1, 7), (1, 2, 1, 7)),
        ((1, 4, 1, 4), (1, 4, 1, 4)),
        ((1, 4, 1, 5), (1, 4, 1, 6)),
        ((1, 4, 1, 6), (1, 4, 1, 6)),
        ((1, 4, 1, 7), (1, 4, 1, 7)),
        ((1, 5, 1, 5), (1, 4, 1, 4)),
        ((1, 5, 1, 6), (1, 4, 1, 6)),
        ((1, 5, 1, 7), (1, 4, 1, 7)),
        ((1, 6, 1, 6), (1, 6, 1, 6)),
        ((1, 6, 1, 7), (1, 6, 1, 7)),
        ((1, 7, 1, 7), (1, 7, 1, 7)),
    ];
    for (range, expected) in cases {
        assert_eq!(validate_range(&m, range), expected, "{range:?}");
    }
}

#[test]
fn modify_position_cases() {
    let m = Rope::from("line one\nline two");
    let cases = [
        ((1, 1, 0), (1, 1)),
        ((0, 0, 0), (1, 1)),
        ((30, 1, 0), (2, 9)),
        ((1, 1, 17), (2, 9)),
        ((1, 1, 1), (1, 2)),
        ((1, 1, 3), (1, 4)),
        ((1, 2, 10), (2, 3)),
        ((1, 5, 13), (2, 9)),
        ((1, 2, 16), (2, 9)),
        ((2, 9, -17), (1, 1)),
        ((1, 2, -1), (1, 1)),
        ((1, 4, -3), (1, 1)),
        ((2, 3, -10), (1, 2)),
        ((2, 9, -13), (1, 5)),
        ((2, 9, -16), (1, 2)),
        ((1, 2, 17), (2, 9)),
        ((1, 2, 100), (2, 9)),
        ((1, 2, -2), (1, 1)),
        ((1, 2, -100), (1, 1)),
        ((2, 2, -100), (1, 1)),
        ((2, 9, -18), (1, 1)),
    ];
    for ((line, column, delta), expected) in cases {
        assert_eq!(
            modify_position(&m, line, column, delta),
            expected,
            "({line}, {column}) {delta:+}"
        );
    }
}
