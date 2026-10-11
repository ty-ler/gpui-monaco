//! Port of Monaco's `diffComputer.test.ts` (the legacy `DiffComputer`), plus
//! a few checks of [`compute_legacy_diff`](super::compute_legacy_diff).

use super::legacy::{CharChange, DiffComputerOptions, LineChange, compute};
use super::{DiffOptions, compute_legacy_diff};

/// Monaco's `Constants.MAX_SAFE_SMALL_INTEGER`, used as "end of line".
const MAX_COLUMN: usize = 1 << 30;

/// `(start_line_number, start_column, end_line_number, end_column)`.
type TextRange = (usize, usize, usize, usize);

/// Replaces a range with UTF-16 text.
type Edit = (TextRange, Vec<u16>);

fn utf16(lines: &[&str]) -> Vec<Vec<u16>> {
    lines.iter().map(|l| l.encode_utf16().collect()).collect()
}

fn ins(
    start_line_number: usize,
    end_line_number: usize,
    original_line_number: usize,
) -> LineChange {
    LineChange {
        original_start_line_number: original_line_number,
        original_end_line_number: 0,
        modified_start_line_number: start_line_number,
        modified_end_line_number: end_line_number,
        char_changes: None,
    }
}

fn del(
    start_line_number: usize,
    end_line_number: usize,
    modified_line_number: usize,
) -> LineChange {
    LineChange {
        original_start_line_number: start_line_number,
        original_end_line_number: end_line_number,
        modified_start_line_number: modified_line_number,
        modified_end_line_number: 0,
        char_changes: None,
    }
}

fn chg(
    original_start_line_number: usize,
    original_end_line_number: usize,
    modified_start_line_number: usize,
    modified_end_line_number: usize,
    char_changes: Option<Vec<CharChange>>,
) -> LineChange {
    LineChange {
        original_start_line_number,
        original_end_line_number,
        modified_start_line_number,
        modified_end_line_number,
        char_changes,
    }
}

#[allow(clippy::too_many_arguments)]
fn cc(
    original_start_line_number: usize,
    original_start_column: usize,
    original_end_line_number: usize,
    original_end_column: usize,
    modified_start_line_number: usize,
    modified_start_column: usize,
    modified_end_line_number: usize,
    modified_end_column: usize,
) -> CharChange {
    CharChange {
        original_start_line_number,
        original_start_column,
        original_end_line_number,
        original_end_column,
        modified_start_line_number,
        modified_start_column,
        modified_end_line_number,
        modified_end_column,
    }
}

/// A minimal text model: the lines and Monaco's position validation.
struct Text {
    lines: Vec<Vec<u16>>,
}

impl Text {
    fn new(lines: &[&str]) -> Self {
        Self {
            lines: utf16(lines),
        }
    }

    /// The offset of a position, clamped into the text like Monaco's
    /// `validatePosition`.
    fn offset(&self, line_number: usize, column: usize) -> usize {
        let (line_number, column) = if line_number < 1 {
            (1, 1)
        } else if line_number > self.lines.len() {
            (self.lines.len(), self.lines.last().map_or(0, Vec::len) + 1)
        } else {
            (
                line_number,
                column.clamp(1, self.lines[line_number - 1].len() + 1),
            )
        };
        self.lines[..line_number - 1]
            .iter()
            .map(|l| l.len() + 1)
            .sum::<usize>()
            + column
            - 1
    }

    fn value(&self) -> Vec<u16> {
        self.lines.join(&u16::from(b'\n'))
    }

    fn value_in_range(&self, range: TextRange) -> Vec<u16> {
        let start = self.offset(range.0, range.1);
        let end = self.offset(range.2, range.3);
        self.value()[start..end.max(start)].to_vec()
    }

    /// Applies non-overlapping `(range, text)` edits.
    fn apply_edits(&self, edits: Vec<Edit>) -> Vec<u16> {
        let mut edits: Vec<_> = edits
            .into_iter()
            .map(|(r, text)| (self.offset(r.0, r.1), self.offset(r.2, r.3), text))
            .collect();
        edits.sort_by_key(|(start, end, _)| (*start, *end));
        let mut value = self.value();
        for (start, end, text) in edits.into_iter().rev() {
            value.splice(start..end, text);
        }
        value
    }
}

/// Monaco's `diffFromLineRanges` on `(start_line_number, line_count)` pairs.
fn diff_from_line_ranges(
    original: (usize, usize),
    modified: (usize, usize),
) -> (TextRange, TextRange) {
    if original.0 == 1 || modified.0 == 1 {
        if original.1 != 0 && modified.1 != 0 {
            return (
                (original.0, 1, original.0 + original.1 - 1, MAX_COLUMN),
                (modified.0, 1, modified.0 + modified.1 - 1, MAX_COLUMN),
            );
        }
        return (
            (original.0, 1, original.0 + original.1, 1),
            (modified.0, 1, modified.0 + modified.1, 1),
        );
    }
    (
        (
            original.0 - 1,
            MAX_COLUMN,
            original.0 + original.1 - 1,
            MAX_COLUMN,
        ),
        (
            modified.0 - 1,
            MAX_COLUMN,
            modified.0 + modified.1 - 1,
            MAX_COLUMN,
        ),
    )
}

/// Monaco's `getLineEdit`.
fn line_edit(change: &LineChange, modified: &Text) -> Edit {
    let range = |start: usize, end: usize| {
        if end == 0 {
            (start + 1, 0)
        } else {
            (start, end - start + 1)
        }
    };
    let (r1, r2) = diff_from_line_ranges(
        range(
            change.original_start_line_number,
            change.original_end_line_number,
        ),
        range(
            change.modified_start_line_number,
            change.modified_end_line_number,
        ),
    );
    (r1, modified.value_in_range(r2))
}

/// Monaco's `getCharEdits`.
fn char_edits(change: &LineChange, modified: &Text) -> Vec<Edit> {
    let Some(char_changes) = &change.char_changes else {
        return vec![line_edit(change, modified)];
    };
    char_changes
        .iter()
        .map(|c| {
            (
                (
                    c.original_start_line_number,
                    c.original_start_column,
                    c.original_end_line_number,
                    c.original_end_column,
                ),
                modified.value_in_range((
                    c.modified_start_line_number,
                    c.modified_start_column,
                    c.modified_end_line_number,
                    c.modified_end_column,
                )),
            )
        })
        .collect()
}

fn assert_diff_with(
    original_lines: &[&str],
    modified_lines: &[&str],
    expected: Vec<LineChange>,
    should_compute_char_changes: bool,
    should_post_process_char_changes: bool,
    should_ignore_trim_whitespace: bool,
) {
    let changes = compute(
        &utf16(original_lines),
        &utf16(modified_lines),
        &DiffComputerOptions {
            should_compute_char_changes,
            should_post_process_char_changes,
            should_ignore_trim_whitespace,
            should_make_pretty_diff: true,
            max_computation_time_ms: 0,
        },
    )
    .changes;
    assert_eq!(changes, expected);

    if !should_ignore_trim_whitespace {
        // The diffs must describe how to edit the original into the modified.
        let original = Text::new(original_lines);
        let modified = Text::new(modified_lines);
        let expected_value = modified.value();

        let line_edits = changes.iter().map(|c| line_edit(c, &modified)).collect();
        assert_eq!(
            String::from_utf16_lossy(&original.apply_edits(line_edits)),
            String::from_utf16_lossy(&expected_value),
            "line edits"
        );

        if should_compute_char_changes {
            let edits = changes
                .iter()
                .flat_map(|c| char_edits(c, &modified))
                .collect();
            assert_eq!(
                String::from_utf16_lossy(&original.apply_edits(edits)),
                String::from_utf16_lossy(&expected_value),
                "char edits"
            );
        }
    }
}

fn assert_diff(original_lines: &[&str], modified_lines: &[&str], expected: Vec<LineChange>) {
    assert_diff_with(original_lines, modified_lines, expected, true, false, false);
}

// ---- insertions

#[test]
fn one_inserted_line_below() {
    assert_diff(&["line"], &["line", "new line"], vec![ins(2, 2, 1)]);
}

#[test]
fn two_inserted_lines_below() {
    assert_diff(
        &["line"],
        &["line", "new line", "another new line"],
        vec![ins(2, 3, 1)],
    );
}

#[test]
fn one_inserted_line_above() {
    assert_diff(&["line"], &["new line", "line"], vec![ins(1, 1, 0)]);
}

#[test]
fn two_inserted_lines_above() {
    assert_diff(
        &["line"],
        &["new line", "another new line", "line"],
        vec![ins(1, 2, 0)],
    );
}

#[test]
fn one_inserted_line_in_middle() {
    assert_diff(
        &["line1", "line2", "line3", "line4"],
        &["line1", "line2", "new line", "line3", "line4"],
        vec![ins(3, 3, 2)],
    );
}

#[test]
fn two_inserted_lines_in_middle() {
    assert_diff(
        &["line1", "line2", "line3", "line4"],
        &[
            "line1",
            "line2",
            "new line",
            "another new line",
            "line3",
            "line4",
        ],
        vec![ins(3, 4, 2)],
    );
}

#[test]
fn two_inserted_lines_in_middle_interrupted() {
    assert_diff(
        &["line1", "line2", "line3", "line4"],
        &[
            "line1",
            "line2",
            "new line",
            "line3",
            "another new line",
            "line4",
        ],
        vec![ins(3, 3, 2), ins(5, 5, 3)],
    );
}

// ---- deletions

#[test]
fn one_deleted_line_below() {
    assert_diff(&["line", "new line"], &["line"], vec![del(2, 2, 1)]);
}

#[test]
fn two_deleted_lines_below() {
    assert_diff(
        &["line", "new line", "another new line"],
        &["line"],
        vec![del(2, 3, 1)],
    );
}

#[test]
fn one_deleted_lines_above() {
    assert_diff(&["new line", "line"], &["line"], vec![del(1, 1, 0)]);
}

#[test]
fn two_deleted_lines_above() {
    assert_diff(
        &["new line", "another new line", "line"],
        &["line"],
        vec![del(1, 2, 0)],
    );
}

#[test]
fn one_deleted_line_in_middle() {
    assert_diff(
        &["line1", "line2", "new line", "line3", "line4"],
        &["line1", "line2", "line3", "line4"],
        vec![del(3, 3, 2)],
    );
}

#[test]
fn two_deleted_lines_in_middle() {
    assert_diff(
        &[
            "line1",
            "line2",
            "new line",
            "another new line",
            "line3",
            "line4",
        ],
        &["line1", "line2", "line3", "line4"],
        vec![del(3, 4, 2)],
    );
}

#[test]
fn two_deleted_lines_in_middle_interrupted() {
    assert_diff(
        &[
            "line1",
            "line2",
            "new line",
            "line3",
            "another new line",
            "line4",
        ],
        &["line1", "line2", "line3", "line4"],
        vec![del(3, 3, 2), del(5, 5, 3)],
    );
}

// ---- changes

#[test]
fn one_line_changed_chars_inserted_at_the_end() {
    assert_diff(
        &["line"],
        &["line changed"],
        vec![chg(1, 1, 1, 1, Some(vec![cc(1, 5, 1, 5, 1, 5, 1, 13)]))],
    );
}

#[test]
fn one_line_changed_chars_inserted_at_the_beginning() {
    assert_diff(
        &["line"],
        &["my line"],
        vec![chg(1, 1, 1, 1, Some(vec![cc(1, 1, 1, 1, 1, 1, 1, 4)]))],
    );
}

#[test]
fn one_line_changed_chars_inserted_in_the_middle() {
    assert_diff(
        &["abba"],
        &["abzzba"],
        vec![chg(1, 1, 1, 1, Some(vec![cc(1, 3, 1, 3, 1, 3, 1, 5)]))],
    );
}

#[test]
fn one_line_changed_chars_inserted_in_the_middle_two_spots() {
    assert_diff(
        &["abba"],
        &["abzzbzza"],
        vec![chg(
            1,
            1,
            1,
            1,
            Some(vec![cc(1, 3, 1, 3, 1, 3, 1, 5), cc(1, 4, 1, 4, 1, 6, 1, 8)]),
        )],
    );
}

#[test]
fn one_line_changed_chars_deleted_1() {
    assert_diff(
        &["abcdefg"],
        &["abcfg"],
        vec![chg(1, 1, 1, 1, Some(vec![cc(1, 4, 1, 6, 1, 4, 1, 4)]))],
    );
}

#[test]
fn one_line_changed_chars_deleted_2() {
    assert_diff(
        &["abcdefg"],
        &["acfg"],
        vec![chg(
            1,
            1,
            1,
            1,
            Some(vec![cc(1, 2, 1, 3, 1, 2, 1, 2), cc(1, 4, 1, 6, 1, 3, 1, 3)]),
        )],
    );
}

#[test]
fn two_lines_changed_1() {
    assert_diff(
        &["abcd", "efgh"],
        &["abcz"],
        vec![chg(1, 2, 1, 1, Some(vec![cc(1, 4, 2, 5, 1, 4, 1, 5)]))],
    );
}

#[test]
fn two_lines_changed_2() {
    assert_diff(
        &["foo", "abcd", "efgh", "BAR"],
        &["foo", "abcz", "BAR"],
        vec![chg(2, 3, 2, 2, Some(vec![cc(2, 4, 3, 5, 2, 4, 2, 5)]))],
    );
}

#[test]
fn two_lines_changed_3() {
    assert_diff(
        &["foo", "abcd", "efgh", "BAR"],
        &["foo", "abcz", "zzzzefgh", "BAR"],
        vec![chg(
            2,
            3,
            2,
            3,
            Some(vec![cc(2, 4, 2, 5, 2, 4, 2, 5), cc(3, 1, 3, 1, 3, 1, 3, 5)]),
        )],
    );
}

#[test]
fn two_lines_changed_4() {
    assert_diff(
        &["abc"],
        &["", "", "axc", ""],
        vec![chg(
            1,
            1,
            1,
            4,
            Some(vec![
                cc(1, 1, 1, 1, 1, 1, 3, 1),
                cc(1, 2, 1, 3, 3, 2, 3, 3),
                cc(1, 4, 1, 4, 3, 4, 4, 1),
            ]),
        )],
    );
}

#[test]
fn empty_original_sequence_in_char_diff() {
    assert_diff(
        &["abc", "", "xyz"],
        &["abc", "qwe", "rty", "xyz"],
        vec![chg(2, 2, 2, 3, None)],
    );
}

#[test]
fn three_lines_changed() {
    assert_diff(
        &["foo", "abcd", "efgh", "BAR"],
        &["foo", "zzzefgh", "xxx", "BAR"],
        vec![chg(
            2,
            3,
            2,
            3,
            Some(vec![cc(2, 1, 3, 1, 2, 1, 2, 4), cc(3, 5, 3, 5, 2, 8, 3, 4)]),
        )],
    );
}

#[test]
fn big_change_part_1() {
    assert_diff(
        &["foo", "abcd", "efgh", "BAR"],
        &["hello", "foo", "zzzefgh", "xxx", "BAR"],
        vec![
            ins(1, 1, 0),
            chg(
                2,
                3,
                3,
                4,
                Some(vec![cc(2, 1, 3, 1, 3, 1, 3, 4), cc(3, 5, 3, 5, 3, 8, 4, 4)]),
            ),
        ],
    );
}

#[test]
fn big_change_part_2() {
    assert_diff(
        &["foo", "abcd", "efgh", "BAR", "RAB"],
        &["hello", "foo", "zzzefgh", "xxx", "BAR"],
        vec![
            ins(1, 1, 0),
            chg(
                2,
                3,
                3,
                4,
                Some(vec![cc(2, 1, 3, 1, 3, 1, 3, 4), cc(3, 5, 3, 5, 3, 8, 4, 4)]),
            ),
            del(5, 5, 5),
        ],
    );
}

#[test]
fn char_change_postprocessing_merges() {
    assert_diff_with(
        &["abba"],
        &["azzzbzzzbzzza"],
        vec![chg(1, 1, 1, 1, Some(vec![cc(1, 2, 1, 4, 1, 2, 1, 13)]))],
        true,
        true,
        false,
    );
}

#[test]
fn ignore_trim_whitespace() {
    assert_diff_with(
        &["\t\t foo ", "abcd", "efgh", "\t\t BAR\t\t"],
        &["  hello\t", "\t foo   \t", "zzzefgh", "xxx", "   BAR   \t"],
        vec![
            ins(1, 1, 0),
            chg(
                2,
                3,
                3,
                4,
                Some(vec![cc(2, 1, 2, 5, 3, 1, 3, 4), cc(3, 5, 3, 5, 4, 1, 4, 4)]),
            ),
        ],
        true,
        false,
        true,
    );
}

#[test]
fn issue_12122_has_own_property_is_not_a_function() {
    assert_diff(
        &["hasOwnProperty"],
        &["hasOwnProperty", "and another line"],
        vec![ins(2, 2, 1)],
    );
}

#[test]
fn empty_diff_1() {
    assert_diff_with(
        &[""],
        &["something"],
        vec![chg(1, 1, 1, 1, None)],
        true,
        false,
        true,
    );
}

#[test]
fn empty_diff_2() {
    assert_diff_with(
        &[""],
        &["something", "something else"],
        vec![chg(1, 1, 1, 2, None)],
        true,
        false,
        true,
    );
}

#[test]
fn empty_diff_3() {
    assert_diff_with(
        &["something", "something else"],
        &[""],
        vec![chg(1, 2, 1, 1, None)],
        true,
        false,
        true,
    );
}

#[test]
fn empty_diff_4() {
    assert_diff_with(
        &["something"],
        &[""],
        vec![chg(1, 1, 1, 1, None)],
        true,
        false,
        true,
    );
}

#[test]
fn empty_diff_5() {
    assert_diff_with(&[""], &[""], vec![], true, false, true);
}

#[test]
fn pretty_diff_1() {
    assert_diff_with(
        &[
            "suite(function () {",
            "\ttest1() {",
            "\t\tassert.ok(true);",
            "\t}",
            "",
            "\ttest2() {",
            "\t\tassert.ok(true);",
            "\t}",
            "});",
            "",
        ],
        &[
            "// An insertion",
            "suite(function () {",
            "\ttest1() {",
            "\t\tassert.ok(true);",
            "\t}",
            "",
            "\ttest2() {",
            "\t\tassert.ok(true);",
            "\t}",
            "",
            "\ttest3() {",
            "\t\tassert.ok(true);",
            "\t}",
            "});",
            "",
        ],
        vec![ins(1, 1, 0), ins(10, 13, 8)],
        true,
        false,
        true,
    );
}

#[test]
fn pretty_diff_2() {
    assert_diff_with(
        &[
            "// Just a comment",
            "",
            "function compute(a, b, c, d) {",
            "\tif (a) {",
            "\t\tif (b) {",
            "\t\t\tif (c) {",
            "\t\t\t\treturn 5;",
            "\t\t\t}",
            "\t\t}",
            "\t\t// These next lines will be deleted",
            "\t\tif (d) {",
            "\t\t\treturn -1;",
            "\t\t}",
            "\t\treturn 0;",
            "\t}",
            "}",
        ],
        &[
            "// Here is an inserted line",
            "// and another inserted line",
            "// and another one",
            "// Just a comment",
            "",
            "function compute(a, b, c, d) {",
            "\tif (a) {",
            "\t\tif (b) {",
            "\t\t\tif (c) {",
            "\t\t\t\treturn 5;",
            "\t\t\t}",
            "\t\t}",
            "\t\treturn 0;",
            "\t}",
            "}",
        ],
        vec![ins(1, 3, 0), del(10, 13, 12)],
        true,
        false,
        true,
    );
}

#[test]
fn pretty_diff_3() {
    assert_diff_with(
        &[
            "class A {",
            "\t/**",
            "\t * m1",
            "\t */",
            "\tmethod1() {}",
            "",
            "\t/**",
            "\t * m3",
            "\t */",
            "\tmethod3() {}",
            "}",
        ],
        &[
            "class A {",
            "\t/**",
            "\t * m1",
            "\t */",
            "\tmethod1() {}",
            "",
            "\t/**",
            "\t * m2",
            "\t */",
            "\tmethod2() {}",
            "",
            "\t/**",
            "\t * m3",
            "\t */",
            "\tmethod3() {}",
            "}",
        ],
        vec![ins(7, 11, 6)],
        true,
        false,
        true,
    );
}

#[test]
fn issue_23636() {
    let original = [
        "if(!TextDrawLoad[playerid])",
        "{",
        "",
        "\tTextDrawHideForPlayer(playerid,TD_AppleJob[3]);",
        "\tTextDrawHideForPlayer(playerid,TD_AppleJob[4]);",
        "\tif(!AppleJobTreesType[AppleJobTreesPlayerNum[playerid]])",
        "\t{",
        "\t\tfor(new i=0;i<10;i++) if(StatusTD_AppleJobApples[playerid][i]) TextDrawHideForPlayer(playerid,TD_AppleJob[5+i]);",
        "\t}",
        "\telse",
        "\t{",
        "\t\tfor(new i=0;i<10;i++) if(StatusTD_AppleJobApples[playerid][i]) TextDrawHideForPlayer(playerid,TD_AppleJob[15+i]);",
        "\t}",
        "}",
        "else",
        "{",
        "\tTextDrawHideForPlayer(playerid,TD_AppleJob[3]);",
        "\tTextDrawHideForPlayer(playerid,TD_AppleJob[27]);",
        "\tif(!AppleJobTreesType[AppleJobTreesPlayerNum[playerid]])",
        "\t{",
        "\t\tfor(new i=0;i<10;i++) if(StatusTD_AppleJobApples[playerid][i]) TextDrawHideForPlayer(playerid,TD_AppleJob[28+i]);",
        "\t}",
        "\telse",
        "\t{",
        "\t\tfor(new i=0;i<10;i++) if(StatusTD_AppleJobApples[playerid][i]) TextDrawHideForPlayer(playerid,TD_AppleJob[38+i]);",
        "\t}",
        "}",
    ];
    // The same lines, indented by one more tab.
    let modified: Vec<String> = original.iter().map(|l| format!("\t{l}")).collect();
    let modified: Vec<&str> = modified.iter().map(String::as_str).collect();
    let expected = vec![chg(
        1,
        27,
        1,
        27,
        Some((1..=27).map(|l| cc(l, 1, l, 1, l, 1, l, 2)).collect()),
    )];
    assert_diff_with(&original, &modified, expected, true, true, false);
}

#[test]
fn issue_43922() {
    assert_diff_with(
        &[
            " * `yarn [install]` -- Install project NPM dependencies. This is automatically done when you first create the project. You should only need to run this if you add dependencies in `package.json`.",
        ],
        &[
            " * `yarn` -- Install project NPM dependencies. You should only need to run this if you add dependencies in `package.json`.",
        ],
        vec![chg(
            1,
            1,
            1,
            1,
            Some(vec![
                cc(1, 9, 1, 19, 1, 9, 1, 9),
                cc(1, 58, 1, 120, 1, 48, 1, 48),
            ]),
        )],
        true,
        true,
        false,
    );
}

#[test]
fn issue_42751() {
    assert_diff_with(
        &["    1", "  2"],
        &["    1", "   3"],
        vec![chg(2, 2, 2, 2, Some(vec![cc(2, 3, 2, 4, 2, 3, 2, 5)]))],
        true,
        true,
        false,
    );
}

#[test]
fn does_not_give_character_changes() {
    assert_diff_with(
        &["    1", "  2", "A"],
        &["    1", "   3", " A"],
        vec![chg(2, 3, 2, 3, None)],
        false,
        false,
        false,
    );
}

#[test]
fn issue_44422_less_than_ideal_diff_results() {
    let m1_body: Vec<String> = (2..=18).map(|i| format!("\t\t//{i}")).collect();
    let m1_body: Vec<&str> = m1_body.iter().map(String::as_str).collect();
    let mut original = vec!["export class C {", "", "\tpublic m1(): void {", "\t\t{"];
    original.extend(&m1_body);
    original.extend([
        "\t\t}",
        "\t}",
        "",
        "\tpublic m2(): void {",
        "\t\tif (a) {",
        "\t\t\tif (b) {",
        "\t\t\t\t//A1",
        "\t\t\t\t//A2",
        "\t\t\t\t//A3",
        "\t\t\t\t//A4",
        "\t\t\t\t//A5",
        "\t\t\t\t//A6",
        "\t\t\t\t//A7",
        "\t\t\t\t//A8",
        "\t\t\t}",
        "\t\t}",
        "",
        "\t\t//A9",
        "\t\t//A10",
        "\t\t//A11",
        "\t\t//A12",
        "\t\t//A13",
        "\t\t//A14",
        "\t\t//A15",
        "\t}",
        "",
        "\tpublic m3(): void {",
        "\t\tif (a) {",
        "\t\t\t//B1",
        "\t\t}",
        "\t\t//B2",
        "\t\t//B3",
        "\t}",
        "",
        "\tpublic m4(): boolean {",
        "\t\t//1",
        "\t\t//2",
        "\t\t//3",
        "\t\t//4",
        "\t}",
        "",
        "}",
    ]);
    let mut modified = vec![
        "export class C {",
        "",
        "\tconstructor() {",
        "",
        "",
        "",
        "",
        "\t}",
        "",
        "\tpublic m1(): void {",
        "\t\t{",
    ];
    modified.extend(&m1_body);
    modified.extend([
        "\t\t}",
        "\t}",
        "",
        "\tpublic m4(): boolean {",
        "\t\t//1",
        "\t\t//2",
        "\t\t//3",
        "\t\t//4",
        "\t}",
        "",
        "}",
    ]);
    assert_eq!(original.len(), 63);
    assert_eq!(modified.len(), 39);
    assert_diff_with(
        &original,
        &modified,
        vec![chg(2, 0, 3, 9, None), chg(25, 55, 31, 0, None)],
        false,
        false,
        false,
    );
}

#[test]
fn gives_preference_to_matching_longer_lines() {
    assert_diff_with(
        &["A", "A", "BB", "C"],
        &["A", "BB", "A", "D", "E", "A", "C"],
        vec![chg(2, 2, 1, 0, None), chg(3, 0, 3, 6, None)],
        false,
        false,
        false,
    );
}

#[test]
fn issue_119051_gives_preference_to_fewer_diff_hunks() {
    assert_diff_with(
        &["1", "", "", "2", ""],
        &["1", "", "1.5", "", "", "2", "", "3", ""],
        vec![chg(2, 0, 3, 4, None), chg(5, 0, 8, 9, None)],
        false,
        false,
        false,
    );
}

#[test]
fn issue_121436_diff_chunk_contains_an_unchanged_line_part_1() {
    assert_diff_with(
        &["if (cond) {", "    cmd", "}"],
        &[
            "if (cond) {",
            "    if (other_cond) {",
            "        cmd",
            "    }",
            "}",
        ],
        vec![chg(1, 0, 2, 2, None), chg(2, 0, 4, 4, None)],
        false,
        false,
        true,
    );
}

#[test]
fn issue_121436_diff_chunk_contains_an_unchanged_line_part_2() {
    assert_diff_with(
        &["if (cond) {", "    cmd", "}"],
        &[
            "if (cond) {",
            "    if (other_cond) {",
            "        cmd",
            "    }",
            "}",
        ],
        vec![
            chg(1, 0, 2, 2, None),
            chg(2, 2, 3, 3, None),
            chg(2, 0, 4, 4, None),
        ],
        false,
        false,
        false,
    );
}

#[test]
fn issue_169552_both_leading_and_trailing_whitespace_diffs() {
    assert_diff_with(
        &["if True:", "    print(2)"],
        &["if True:", "\tprint(2) "],
        vec![chg(
            2,
            2,
            2,
            2,
            Some(vec![
                cc(2, 1, 2, 5, 2, 1, 2, 2),
                cc(2, 13, 2, 13, 2, 10, 2, 11),
            ]),
        )],
        true,
        false,
        false,
    );
}

// ---- `compute_legacy_diff`

#[test]
fn legacy_lines_diff_converts_line_changes() {
    let options = DiffOptions {
        ignore_trim_whitespace: false,
        max_computation_time_ms: 0,
        compute_moves: true,
        extend_to_subwords: false,
    };
    let diff = compute_legacy_diff(
        &["foo", "abcd", "efgh", "BAR", "RAB"],
        &["hello", "foo", "zzzefgh", "xxx", "BAR"],
        &options,
    );
    assert!(diff.moves.is_empty());
    assert!(!diff.hit_timeout);
    let summary: Vec<String> = diff
        .changes
        .iter()
        .map(|c| {
            let inner: Vec<String> = c.inner_changes.iter().map(|i| i.to_string()).collect();
            format!("{} {}", c.line_range_mapping(), inner.join(" "))
        })
        .collect();
    assert_eq!(
        summary,
        [
            "{[1,1)->[1,2)} ",
            "{[2,4)->[3,5)} {[2,1 -> 3,1]->[3,1 -> 3,4]} {[3,5 -> 3,5]->[3,8 -> 4,4]}",
            "{[5,6)->[6,6)} ",
        ]
    );
}

#[test]
fn legacy_lines_diff_identical_and_empty() {
    let options = DiffOptions::default();
    let diff = compute_legacy_diff(&["a", "b"], &["a", "b"], &options);
    assert!(diff.changes.is_empty());
    let empty: [&str; 0] = [];
    let diff = compute_legacy_diff(&empty, &["x", "y"], &options);
    assert_eq!(diff.changes.len(), 1);
    assert_eq!(
        diff.changes[0].line_range_mapping().to_string(),
        "{[1,2)->[1,3)}"
    );
    assert!(diff.changes[0].inner_changes.is_empty());
}
