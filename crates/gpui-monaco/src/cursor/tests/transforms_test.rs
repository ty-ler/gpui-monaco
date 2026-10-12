//! One-shot text transforms. Ports, from Monaco (MIT):
//!
//! - `src/vs/editor/contrib/linesOperations/test/browser/linesOperations.test.ts`:
//!   the `SortLinesAscendingAction`, `SortLinesDescendingAction`,
//!   `DeleteDuplicateLinesAction`, `JoinLinesAction`, `ReverseLinesAction`
//!   and `DeleteAllRightAction` suites and the `transpose` and `toggle case`
//!   tests. The rest of that file (`DeleteAllLeftAction`, `InsertLine*`,
//!   indentation, delete lines) is in `lines_test.rs`.
//! - `src/vs/editor/contrib/linesOperations/test/browser/sortLinesCommand.test.ts`.
//! - `src/vs/editor/contrib/caretOperations/test/browser/moveCarretCommand.test.ts`.
//! - The `Editor Contrib - Duplicate Selection` suite of
//!   `src/vs/editor/contrib/linesOperations/test/browser/copyLinesCommand.test.ts`.
//!
//! `sortLinesCommand.test.ts` and `moveCarretCommand.test.ts` run the bare
//! commands through `testCommand`; here they go through the actions
//! (`sort_lines`, `move_caret`), which run the same command once per
//! selection. The one place that differs is `no op unless at least two lines
//! selected 1`: the action applies a lone single-line selection to the whole
//! document, so that test adds a second single-line cursor to keep the
//! per-selection command semantics it checks.

use super::harness::{TestEditor, fmt_sel, sel};
use crate::cursor::{
    CaseTransform, Selection, delete_all_right, delete_duplicate_lines, duplicate_selection,
    join_lines, move_caret, reverse_lines, sort_lines, transform_case, transpose,
};

#[track_caller]
fn assert_lines(editor: &TestEditor, lines: &[&str]) {
    assert_eq!(editor.value(), lines.join("\n"));
}

/// Monaco `model.getValueInRange(range)` (1-based, EOL `\n`).
fn value_in_range(editor: &TestEditor, s: Selection) -> String {
    let (start, end) = (s.start(), s.end());
    let mut out: Vec<u16> = Vec::new();
    for row in start.row..=end.row {
        let line: Vec<u16> = editor.line(row + 1).encode_utf16().collect();
        let from = if row == start.row {
            start.column as usize
        } else {
            0
        };
        let to = if row == end.row {
            (end.column as usize).min(line.len())
        } else {
            line.len()
        };
        out.extend_from_slice(&line[from.min(to)..to]);
        if row != end.row {
            out.push(u16::from(b'\n'));
        }
    }
    String::from_utf16(&out).unwrap()
}

fn sort(editor: &mut TestEditor, descending: bool) {
    if let Some(op) = sort_lines(&editor.config, &editor.buffer, &editor.cursors, descending) {
        editor.apply(op);
    }
}

fn delete_duplicates(editor: &mut TestEditor) {
    if let Some(op) = delete_duplicate_lines(&editor.config, &editor.buffer, &editor.cursors) {
        editor.apply(op);
    }
}

fn case(editor: &mut TestEditor, selection: Selection, transform: CaseTransform) {
    editor.set_selection(selection);
    editor.run(|c, b, cur| transform_case(c, b, cur, transform));
}

// ---------------------------------------------------------------------------
// SortLinesAscendingAction

/// Monaco: should sort selected lines in ascending order
#[test]
fn sort_ascending_should_sort_selected_lines() {
    let mut editor = TestEditor::lines(&["omicron", "beta", "alpha"]);
    editor.set_selection(sel(1, 1, 3, 5));
    sort(&mut editor, false);
    assert_lines(&editor, &["alpha", "beta", "omicron"]);
    editor.assert_selection(sel(1, 1, 3, 7));
}

/// Monaco: should sort lines in ascending order
#[test]
fn sort_ascending_should_sort_lines() {
    let mut editor = TestEditor::lines(&["omicron", "beta", "alpha"]);
    sort(&mut editor, false);
    assert_lines(&editor, &["alpha", "beta", "omicron"]);
}

/// Monaco: should sort multiple selections in ascending order
#[test]
fn sort_ascending_should_sort_multiple_selections() {
    let mut editor =
        TestEditor::lines(&["omicron", "beta", "alpha", "", "omicron", "beta", "alpha"]);
    editor.set_selections(&[sel(1, 1, 3, 5), sel(5, 1, 7, 5)]);
    sort(&mut editor, false);
    assert_lines(
        &editor,
        &["alpha", "beta", "omicron", "", "alpha", "beta", "omicron"],
    );
    editor.assert_selections(&[sel(1, 1, 3, 7), sel(5, 1, 7, 7)]);
}

/// Monaco: applies to whole document when selection is single line
#[test]
fn sort_ascending_applies_to_whole_document_when_selection_is_single_line() {
    let mut editor = TestEditor::lines(&["omicron", "beta", "alpha"]);
    editor.set_selection(sel(2, 1, 2, 4));
    sort(&mut editor, false);
    assert_lines(&editor, &["alpha", "beta", "omicron"]);
}

// ---------------------------------------------------------------------------
// SortLinesDescendingAction

/// Monaco: should sort selected lines in descending order
#[test]
fn sort_descending_should_sort_selected_lines() {
    let mut editor = TestEditor::lines(&["alpha", "beta", "omicron"]);
    editor.set_selection(sel(1, 1, 3, 7));
    sort(&mut editor, true);
    assert_lines(&editor, &["omicron", "beta", "alpha"]);
    editor.assert_selection(sel(1, 1, 3, 5));
}

/// Monaco: should sort multiple selections in descending order
#[test]
fn sort_descending_should_sort_multiple_selections() {
    let mut editor =
        TestEditor::lines(&["alpha", "beta", "omicron", "", "alpha", "beta", "omicron"]);
    editor.set_selections(&[sel(1, 1, 3, 7), sel(5, 1, 7, 7)]);
    sort(&mut editor, true);
    assert_lines(
        &editor,
        &["omicron", "beta", "alpha", "", "omicron", "beta", "alpha"],
    );
    editor.assert_selections(&[sel(1, 1, 3, 5), sel(5, 1, 7, 5)]);
}

/// Monaco sorts with the default `Intl.Collator`: punctuation, then
/// symbols, digits and letters, with case and accents only breaking ties.
#[test]
fn sort_follows_the_default_collator() {
    let mut editor = TestEditor::lines(&["b", "_a", "1", "}", "é", "f", "e", "+", "B", "a"]);
    editor.set_selection(sel(1, 1, 10, 2));
    sort(&mut editor, false);
    assert_lines(
        &editor,
        &["_a", "}", "+", "1", "a", "b", "B", "e", "é", "f"],
    );
    sort(&mut editor, true);
    assert_lines(
        &editor,
        &["f", "é", "e", "B", "b", "a", "1", "+", "}", "_a"],
    );
}

// ---------------------------------------------------------------------------
// sortLinesCommand.test.ts

const SORT_TEXT: &[&str] = &["first", "second line", "third line", "fourth line", "fifth"];

#[track_caller]
fn test_sort_command(
    descending: bool,
    selections: &[Selection],
    expected_lines: &[&str],
    expected_selections: &[Selection],
) {
    let mut editor = TestEditor::lines(SORT_TEXT);
    editor.set_selections(selections);
    sort(&mut editor, descending);
    assert_lines(&editor, expected_lines);
    editor.assert_selections(expected_selections);
}

/// Monaco: no op unless at least two lines selected 1
#[test]
fn sort_command_no_op_unless_at_least_two_lines_selected_1() {
    // A second single-line cursor keeps the action from applying the
    // selection to the whole document (see the module docs).
    test_sort_command(
        false,
        &[sel(1, 3, 1, 1), sel(5, 1, 5, 1)],
        SORT_TEXT,
        &[sel(1, 3, 1, 1), sel(5, 1, 5, 1)],
    );
}

/// Monaco: no op unless at least two lines selected 2
#[test]
fn sort_command_no_op_unless_at_least_two_lines_selected_2() {
    test_sort_command(false, &[sel(1, 3, 2, 1)], SORT_TEXT, &[sel(1, 3, 2, 1)]);
}

/// Monaco: sorting two lines ascending
#[test]
fn sort_command_sorting_two_lines_ascending() {
    test_sort_command(
        false,
        &[sel(3, 3, 4, 2)],
        &["first", "second line", "fourth line", "third line", "fifth"],
        &[sel(3, 3, 4, 1)],
    );
}

/// Monaco: sorting first 4 lines ascending
#[test]
fn sort_command_sorting_first_4_lines_ascending() {
    test_sort_command(
        false,
        &[sel(1, 1, 5, 1)],
        &["first", "fourth line", "second line", "third line", "fifth"],
        &[sel(1, 1, 5, 1)],
    );
}

/// Monaco: sorting all lines ascending
#[test]
fn sort_command_sorting_all_lines_ascending() {
    test_sort_command(
        false,
        &[sel(1, 1, 5, 6)],
        &["fifth", "first", "fourth line", "second line", "third line"],
        &[sel(1, 1, 5, 11)],
    );
}

/// Monaco: sorting first 4 lines descending
#[test]
fn sort_command_sorting_first_4_lines_descending() {
    test_sort_command(
        true,
        &[sel(1, 1, 5, 1)],
        &["third line", "second line", "fourth line", "first", "fifth"],
        &[sel(1, 1, 5, 1)],
    );
}

/// Monaco: sorting all lines descending
#[test]
fn sort_command_sorting_all_lines_descending() {
    test_sort_command(
        true,
        &[sel(1, 1, 5, 6)],
        &["third line", "second line", "fourth line", "first", "fifth"],
        &[sel(1, 1, 5, 6)],
    );
}

// ---------------------------------------------------------------------------
// DeleteDuplicateLinesAction

/// Monaco: should remove duplicate lines within selection
#[test]
fn delete_duplicate_lines_should_remove_duplicate_lines_within_selection() {
    let mut editor = TestEditor::lines(&["alpha", "beta", "beta", "beta", "alpha", "omicron"]);
    editor.set_selection(sel(1, 3, 6, 4));
    delete_duplicates(&mut editor);
    assert_lines(&editor, &["alpha", "beta", "omicron"]);
    editor.assert_selection(sel(1, 1, 3, 8));
}

/// Monaco: should remove duplicate lines
#[test]
fn delete_duplicate_lines_should_remove_duplicate_lines() {
    let mut editor = TestEditor::lines(&["alpha", "beta", "beta", "beta", "alpha", "omicron"]);
    delete_duplicates(&mut editor);
    assert_lines(&editor, &["alpha", "beta", "omicron"]);
    assert!(editor.selections()[0].is_empty());
}

/// Monaco: should remove duplicate lines in multiple selections
#[test]
fn delete_duplicate_lines_should_remove_duplicate_lines_in_multiple_selections() {
    let mut editor = TestEditor::lines(&[
        "alpha", "beta", "beta", "omicron", "", "alpha", "alpha", "beta",
    ]);
    editor.set_selections(&[sel(1, 2, 4, 3), sel(6, 2, 8, 3)]);
    delete_duplicates(&mut editor);
    assert_lines(&editor, &["alpha", "beta", "omicron", "", "alpha", "beta"]);
    editor.assert_selections(&[sel(1, 1, 3, 8), sel(5, 1, 6, 5)]);
}

/// Monaco: applies to whole document when selection is single line
#[test]
fn delete_duplicate_lines_applies_to_whole_document_when_selection_is_single_line() {
    let mut editor = TestEditor::lines(&["alpha", "beta", "alpha", "omicron"]);
    editor.set_selection(sel(2, 1, 2, 2));
    delete_duplicates(&mut editor);
    assert_lines(&editor, &["alpha", "beta", "omicron"]);
}

// ---------------------------------------------------------------------------
// JoinLinesAction

const JOIN_TEXT: &[&str] = &[
    "hello",
    "world",
    "hello ",
    "world",
    "hello\t\t",
    "\tworld",
    "hello   ",
    "\tworld",
    "",
    "",
    "hello world",
];

/// Monaco: should join lines and insert space if necessary
#[test]
fn join_lines_should_join_lines_and_insert_space_if_necessary() {
    let mut editor = TestEditor::lines(JOIN_TEXT);

    editor.set_selection(sel(1, 2, 1, 2));
    editor.run(join_lines);
    assert_eq!(editor.line(1), "hello world");
    editor.assert_selection(sel(1, 6, 1, 6));

    editor.set_selection(sel(2, 2, 2, 2));
    editor.run(join_lines);
    assert_eq!(editor.line(2), "hello world");
    editor.assert_selection(sel(2, 7, 2, 7));

    editor.set_selection(sel(3, 2, 3, 2));
    editor.run(join_lines);
    assert_eq!(editor.line(3), "hello world");
    editor.assert_selection(sel(3, 7, 3, 7));

    editor.set_selection(sel(4, 2, 5, 3));
    editor.run(join_lines);
    assert_eq!(editor.line(4), "hello world");
    editor.assert_selection(sel(4, 2, 4, 8));

    editor.set_selection(sel(5, 1, 7, 3));
    editor.run(join_lines);
    assert_eq!(editor.line(5), "hello world");
    editor.assert_selection(sel(5, 1, 5, 3));
}

/// Monaco: #50471 Join lines at the end of document
#[test]
fn join_lines_at_the_end_of_document_50471() {
    let mut editor = TestEditor::lines(&["hello", "world"]);
    editor.set_selection(sel(2, 1, 2, 1));
    editor.run(join_lines);
    assert_eq!(editor.line(1), "hello");
    assert_eq!(editor.line(2), "world");
    editor.assert_selection(sel(2, 6, 2, 6));
}

/// Monaco: should work in multi cursor mode
#[test]
fn join_lines_should_work_in_multi_cursor_mode() {
    let mut editor = TestEditor::lines(JOIN_TEXT);
    editor.set_selections(&[
        // primary cursor
        sel(5, 2, 5, 2),
        sel(1, 2, 1, 2),
        sel(3, 2, 4, 2),
        sel(5, 4, 6, 3),
        sel(7, 5, 8, 4),
        sel(10, 1, 10, 1),
    ]);
    editor.run(join_lines);
    assert_eq!(
        editor.value(),
        "hello world\nhello world\nhello world\nhello world\n\nhello world"
    );
    editor.assert_selections(&[
        // primary cursor
        sel(3, 4, 3, 8),
        sel(1, 6, 1, 6),
        sel(2, 2, 2, 8),
        sel(4, 5, 4, 9),
        sel(6, 1, 6, 1),
    ]);
}

/// Monaco: should push undo stop
#[test]
fn join_lines_should_push_undo_stop() {
    let mut editor = TestEditor::lines(&["hello", "world"]);
    editor.set_selection(sel(1, 6, 1, 6));

    editor.type_keyboard(" my dear");
    assert_eq!(editor.line(1), "hello my dear");
    assert_eq!(
        fmt_sel(&editor.selections()[0]),
        fmt_sel(&sel(1, 14, 1, 14))
    );

    editor.run(join_lines);
    assert_eq!(editor.line(1), "hello my dear world");
    assert_eq!(
        fmt_sel(&editor.selections()[0]),
        fmt_sel(&sel(1, 14, 1, 14))
    );

    editor.undo();
    assert_eq!(editor.line(1), "hello my dear");
    assert_eq!(
        fmt_sel(&editor.selections()[0]),
        fmt_sel(&sel(1, 14, 1, 14))
    );
}

// ---------------------------------------------------------------------------
// ReverseLinesAction

/// Monaco: reverses lines
#[test]
fn reverse_lines_reverses_lines() {
    let mut editor = TestEditor::lines(&["alice", "bob", "charlie"]);
    editor.run(reverse_lines);
    assert_lines(&editor, &["charlie", "bob", "alice"]);
}

/// Monaco: excludes empty last line
#[test]
fn reverse_lines_excludes_empty_last_line() {
    let mut editor = TestEditor::lines(&["alice", "bob", "charlie", ""]);
    editor.run(reverse_lines);
    assert_lines(&editor, &["charlie", "bob", "alice", ""]);
}

/// Monaco: updates cursor
#[test]
fn reverse_lines_updates_cursor() {
    let mut editor = TestEditor::lines(&["alice", "bob", "charlie"]);
    // Cursor at the third column of the third line, 'charlie'.
    editor.set_position(3, 3);
    editor.run(reverse_lines);
    // Cursor at the third column of the *first* line, 'charlie'.
    assert_eq!(editor.position().to_monaco(), (1, 3));
}

/// Monaco: preserves cursor on empty last line
#[test]
fn reverse_lines_preserves_cursor_on_empty_last_line() {
    let mut editor = TestEditor::lines(&["alice", "bob", "charlie", ""]);
    editor.set_position(4, 1);
    editor.run(reverse_lines);
    assert_eq!(editor.position().to_monaco(), (4, 1));
}

/// Monaco: preserves selected text when selections do not span lines
#[test]
fn reverse_lines_preserves_selected_text_when_selections_do_not_span_lines() {
    let mut editor = TestEditor::lines(&["alice", "bob", "charlie", ""]);
    editor.set_selections(&[sel(1, 1, 1, 3), sel(2, 1, 2, 4), sel(3, 1, 3, 5)]);
    let selected = |editor: &TestEditor| -> Vec<String> {
        editor
            .selections()
            .into_iter()
            .map(|s| value_in_range(editor, s))
            .collect()
    };
    let expected = ["al", "bob", "char"];
    assert_eq!(selected(&editor), expected);

    editor.run(reverse_lines);
    assert_eq!(selected(&editor), expected);
}

/// Monaco: reverses lines within selection
#[test]
fn reverse_lines_reverses_lines_within_selection() {
    let mut editor = TestEditor::lines(&["line1", "line2", "line3", "line4", "line5"]);
    // Select lines 2-4.
    editor.set_selection(sel(2, 1, 4, 6));
    editor.run(reverse_lines);
    assert_lines(&editor, &["line1", "line4", "line3", "line2", "line5"]);
}

/// Monaco: reverses lines within partial selection
#[test]
fn reverse_lines_reverses_lines_within_partial_selection() {
    let mut editor = TestEditor::lines(&["line1", "line2", "line3", "line4", "line5"]);
    // From the middle of line 2 to the middle of line 4.
    editor.set_selection(sel(2, 3, 4, 3));
    editor.run(reverse_lines);
    assert_lines(&editor, &["line1", "line4", "line3", "line2", "line5"]);
}

/// Monaco: reverses lines with multiple selections
#[test]
fn reverse_lines_reverses_lines_with_multiple_selections() {
    let mut editor = TestEditor::lines(&["line1", "line2", "line3", "line4", "line5", "line6"]);
    // Select lines 1-2 and lines 4-5.
    editor.set_selections(&[sel(1, 1, 2, 6), sel(4, 1, 5, 6)]);
    editor.run(reverse_lines);
    assert_lines(
        &editor,
        &["line2", "line1", "line3", "line5", "line4", "line6"],
    );
}

/// Monaco: updates selection positions after reversal
#[test]
fn reverse_lines_updates_selection_positions_after_reversal() {
    let mut editor = TestEditor::lines(&["line1", "line2", "line3", "line4"]);
    // Select lines 1-3.
    editor.set_selection(sel(1, 2, 3, 3));
    editor.run(reverse_lines);
    // Line 1 col 2 -> line 3 col 3 becomes line 3 col 2 -> line 1 col 3; the
    // range is normalized.
    let selection = editor.selections()[0];
    assert_eq!(selection.start().to_monaco(), (1, 3));
    assert_eq!(selection.end().to_monaco(), (3, 2));
}

/// Monaco: applies to whole document when selection is single line (the
/// first of two tests with this name)
#[test]
fn reverse_lines_applies_to_whole_document_when_selection_is_single_line() {
    let mut editor = TestEditor::lines(&["line1", "line2", "line3"]);
    // Select only line 2.
    editor.set_selection(sel(2, 1, 2, 6));
    editor.run(reverse_lines);
    assert_lines(&editor, &["line3", "line2", "line1"]);
}

/// Monaco: excludes end line when selection ends at column 1
#[test]
fn reverse_lines_excludes_end_line_when_selection_ends_at_column_1() {
    let mut editor = TestEditor::lines(&["line1", "line2", "line3", "line4", "line5"]);
    // From line 2 to line 4 column 1 (line 4 is excluded).
    editor.set_selection(sel(2, 1, 4, 1));
    editor.run(reverse_lines);
    assert_lines(&editor, &["line1", "line3", "line2", "line4", "line5"]);
}

/// Monaco: applies to whole document when selection is single line (the
/// second of two tests with this name)
#[test]
fn reverse_lines_applies_to_whole_document_when_selection_is_single_line_2() {
    let mut editor = TestEditor::lines(&["omicron", "beta", "alpha"]);
    editor.set_selection(sel(2, 1, 2, 4));
    editor.run(reverse_lines);
    assert_lines(&editor, &["alpha", "beta", "omicron"]);
}

// ---------------------------------------------------------------------------
// transpose

/// Monaco: transpose
#[test]
fn transpose_() {
    let mut editor = TestEditor::lines(&["hello world", "", "", "   "]);

    editor.set_selection(sel(1, 1, 1, 1));
    editor.run(transpose);
    assert_eq!(editor.line(1), "hello world");
    editor.assert_selection(sel(1, 2, 1, 2));

    editor.set_selection(sel(1, 6, 1, 6));
    editor.run(transpose);
    assert_eq!(editor.line(1), "hell oworld");
    editor.assert_selection(sel(1, 7, 1, 7));

    editor.set_selection(sel(1, 12, 1, 12));
    editor.run(transpose);
    assert_eq!(editor.line(1), "hell oworl");
    editor.assert_selection(sel(2, 2, 2, 2));

    editor.set_selection(sel(3, 1, 3, 1));
    editor.run(transpose);
    assert_eq!(editor.line(3), "");
    editor.assert_selection(sel(4, 1, 4, 1));

    editor.set_selection(sel(4, 2, 4, 2));
    editor.run(transpose);
    assert_eq!(editor.line(4), "   ");
    editor.assert_selection(sel(4, 3, 4, 3));

    // fix #16633
    let mut editor = TestEditor::lines(&[
        "",
        "",
        "hello",
        "world",
        "",
        "hello world",
        "",
        "hello world",
    ]);

    editor.set_selection(sel(1, 1, 1, 1));
    editor.run(transpose);
    assert_eq!(editor.line(2), "");
    editor.assert_selection(sel(2, 1, 2, 1));

    editor.set_selection(sel(3, 6, 3, 6));
    editor.run(transpose);
    assert_eq!(editor.line(4), "oworld");
    editor.assert_selection(sel(4, 2, 4, 2));

    editor.set_selection(sel(6, 12, 6, 12));
    editor.run(transpose);
    assert_eq!(editor.line(7), "d");
    editor.assert_selection(sel(7, 2, 7, 2));

    editor.set_selection(sel(8, 12, 8, 12));
    editor.run(transpose);
    assert_eq!(editor.line(8), "hello world");
    editor.assert_selection(sel(8, 12, 8, 12));
}

// ---------------------------------------------------------------------------
// toggle case

/// Monaco: toggle case
#[test]
fn toggle_case() {
    use CaseTransform::*;

    let mut editor = TestEditor::lines(&[
        "hello world",
        "öçşğü",
        "parseHTMLString",
        "getElementById",
        "insertHTML",
        "PascalCase",
        "CSSSelectorsList",
        "iD",
        "tEST",
        "öçşÖÇŞğüĞÜ",
        "audioConverter.convertM4AToMP3();",
        "snake_case",
        "Capital_Snake_Case",
        "function helloWorld() {",
        "return someGlobalObject.printHelloWorld(\"en\", \"utf-8\");",
        "}",
        "helloWorld();",
        "'JavaScript'",
        "parseHTML4String",
        "_accessor: ServicesAccessor",
    ]);

    case(&mut editor, sel(1, 1, 1, 12), Upper);
    assert_eq!(editor.line(1), "HELLO WORLD");
    editor.assert_selection(sel(1, 1, 1, 12));

    case(&mut editor, sel(1, 1, 1, 12), Lower);
    assert_eq!(editor.line(1), "hello world");
    editor.assert_selection(sel(1, 1, 1, 12));

    case(&mut editor, sel(1, 3, 1, 3), Upper);
    assert_eq!(editor.line(1), "HELLO world");
    editor.assert_selection(sel(1, 3, 1, 3));

    case(&mut editor, sel(1, 4, 1, 4), Lower);
    assert_eq!(editor.line(1), "hello world");
    editor.assert_selection(sel(1, 4, 1, 4));

    case(&mut editor, sel(1, 1, 1, 12), Title);
    assert_eq!(editor.line(1), "Hello World");
    editor.assert_selection(sel(1, 1, 1, 12));

    case(&mut editor, sel(2, 1, 2, 6), Upper);
    assert_eq!(editor.line(2), "ÖÇŞĞÜ");
    editor.assert_selection(sel(2, 1, 2, 6));

    case(&mut editor, sel(2, 1, 2, 6), Lower);
    assert_eq!(editor.line(2), "öçşğü");
    editor.assert_selection(sel(2, 1, 2, 6));

    case(&mut editor, sel(2, 1, 2, 6), Title);
    assert_eq!(editor.line(2), "Öçşğü");
    editor.assert_selection(sel(2, 1, 2, 6));

    case(&mut editor, sel(3, 1, 3, 16), Snake);
    assert_eq!(editor.line(3), "parse_html_string");
    editor.assert_selection(sel(3, 1, 3, 18));

    case(&mut editor, sel(4, 1, 4, 15), Snake);
    assert_eq!(editor.line(4), "get_element_by_id");
    editor.assert_selection(sel(4, 1, 4, 18));

    case(&mut editor, sel(5, 1, 5, 11), Snake);
    assert_eq!(editor.line(5), "insert_html");
    editor.assert_selection(sel(5, 1, 5, 12));

    case(&mut editor, sel(6, 1, 6, 11), Snake);
    assert_eq!(editor.line(6), "pascal_case");
    editor.assert_selection(sel(6, 1, 6, 12));

    case(&mut editor, sel(7, 1, 7, 17), Snake);
    assert_eq!(editor.line(7), "css_selectors_list");
    editor.assert_selection(sel(7, 1, 7, 19));

    case(&mut editor, sel(8, 1, 8, 3), Snake);
    assert_eq!(editor.line(8), "i_d");
    editor.assert_selection(sel(8, 1, 8, 4));

    case(&mut editor, sel(9, 1, 9, 5), Snake);
    assert_eq!(editor.line(9), "t_est");
    editor.assert_selection(sel(9, 1, 9, 6));

    case(&mut editor, sel(10, 1, 10, 11), Snake);
    assert_eq!(editor.line(10), "öçş_öç_şğü_ğü");
    editor.assert_selection(sel(10, 1, 10, 14));

    case(&mut editor, sel(11, 1, 11, 34), Snake);
    assert_eq!(editor.line(11), "audio_converter.convert_m4a_to_mp3();");
    editor.assert_selection(sel(11, 1, 11, 38));

    case(&mut editor, sel(12, 1, 12, 11), Snake);
    assert_eq!(editor.line(12), "snake_case");
    editor.assert_selection(sel(12, 1, 12, 11));

    case(&mut editor, sel(13, 1, 13, 19), Snake);
    assert_eq!(editor.line(13), "capital_snake_case");
    editor.assert_selection(sel(13, 1, 13, 19));

    case(&mut editor, sel(14, 1, 17, 14), Snake);
    assert_eq!(
        value_in_range(&editor, sel(14, 1, 17, 15)),
        "function hello_world() {\n\
         return some_global_object.print_hello_world(\"en\", \"utf-8\");\n\
         }\n\
         hello_world();"
    );
    editor.assert_selection(sel(14, 1, 17, 15));

    case(&mut editor, sel(18, 1, 18, 13), Snake);
    assert_eq!(editor.line(18), "'java_script'");
    editor.assert_selection(sel(18, 1, 18, 14));

    case(&mut editor, sel(19, 1, 19, 17), Snake);
    assert_eq!(editor.line(19), "parse_html4_string");
    editor.assert_selection(sel(19, 1, 19, 19));

    case(&mut editor, sel(20, 1, 20, 28), Snake);
    assert_eq!(editor.line(20), "_accessor: services_accessor");
    editor.assert_selection(sel(20, 1, 20, 29));

    let mut editor = TestEditor::lines(&[
        "foO baR BaZ",
        "foO'baR'BaZ",
        "foO[baR]BaZ",
        "foO`baR~BaZ",
        "foO^baR%BaZ",
        "foO$baR!BaZ",
        "'physician's assistant'",
    ]);

    case(&mut editor, sel(1, 1, 1, 12), Title);
    assert_eq!(editor.line(1), "Foo Bar Baz");

    case(&mut editor, sel(2, 1, 2, 12), Title);
    assert_eq!(editor.line(2), "Foo'bar'baz");

    case(&mut editor, sel(3, 1, 3, 12), Title);
    assert_eq!(editor.line(3), "Foo[Bar]Baz");

    case(&mut editor, sel(4, 1, 4, 12), Title);
    assert_eq!(editor.line(4), "Foo`Bar~Baz");

    case(&mut editor, sel(5, 1, 5, 12), Title);
    assert_eq!(editor.line(5), "Foo^Bar%Baz");

    case(&mut editor, sel(6, 1, 6, 12), Title);
    assert_eq!(editor.line(6), "Foo$Bar!Baz");

    case(&mut editor, sel(7, 1, 7, 23), Title);
    assert_eq!(editor.line(7), "'Physician's Assistant'");

    let mut editor = TestEditor::lines(&[
        "camel from words",
        "from_snake_case",
        "from-kebab-case",
        "alreadyCamel",
        "ReTain_some_CAPitalization",
        "my_var.test_function()",
        "öçş_öç_şğü_ğü",
        "XMLHttpRequest",
        "\tfunction hello_world() {",
        "\t\treturn some_global_object;",
        "\t}",
    ]);

    case(&mut editor, sel(1, 1, 1, 18), Camel);
    assert_eq!(editor.line(1), "camelFromWords");

    case(&mut editor, sel(2, 1, 2, 15), Camel);
    assert_eq!(editor.line(2), "fromSnakeCase");

    case(&mut editor, sel(3, 1, 3, 15), Camel);
    assert_eq!(editor.line(3), "fromKebabCase");

    case(&mut editor, sel(4, 1, 4, 12), Camel);
    assert_eq!(editor.line(4), "alreadyCamel");

    case(&mut editor, sel(5, 1, 5, 26), Camel);
    assert_eq!(editor.line(5), "reTainSomeCAPitalization");

    case(&mut editor, sel(6, 1, 6, 23), Camel);
    assert_eq!(editor.line(6), "myVar.testFunction()");

    case(&mut editor, sel(7, 1, 7, 14), Camel);
    assert_eq!(editor.line(7), "öçşÖçŞğüĞü");

    case(&mut editor, sel(8, 1, 8, 14), Camel);
    assert_eq!(editor.line(8), "XMLHttpRequest");

    case(&mut editor, sel(9, 1, 11, 2), Camel);
    assert_eq!(
        value_in_range(&editor, sel(9, 1, 11, 3)),
        "\tfunction helloWorld() {\n\t\treturn someGlobalObject;\n\t}"
    );

    let mut editor = TestEditor::lines(&["", "   "]);

    case(&mut editor, sel(1, 1, 1, 1), Upper);
    assert_eq!(editor.line(1), "");
    editor.assert_selection(sel(1, 1, 1, 1));

    case(&mut editor, sel(1, 1, 1, 1), Lower);
    assert_eq!(editor.line(1), "");
    editor.assert_selection(sel(1, 1, 1, 1));

    case(&mut editor, sel(2, 2, 2, 2), Upper);
    assert_eq!(editor.line(2), "   ");
    editor.assert_selection(sel(2, 2, 2, 2));

    case(&mut editor, sel(2, 2, 2, 2), Lower);
    assert_eq!(editor.line(2), "   ");
    editor.assert_selection(sel(2, 2, 2, 2));

    let mut editor = TestEditor::lines(&[
        "hello world",
        "öçşğü",
        "parseHTMLString",
        "getElementById",
        "PascalCase",
        "öçşÖÇŞğüĞÜ",
        "audioConverter.convertM4AToMP3();",
        "Capital_Snake_Case",
        "parseHTML4String",
        "_accessor: ServicesAccessor",
        "Kebab-Case",
    ]);

    case(&mut editor, sel(1, 1, 1, 12), Kebab);
    assert_eq!(editor.line(1), "hello world");
    editor.assert_selection(sel(1, 1, 1, 12));

    case(&mut editor, sel(2, 1, 2, 6), Kebab);
    assert_eq!(editor.line(2), "öçşğü");
    editor.assert_selection(sel(2, 1, 2, 6));

    case(&mut editor, sel(3, 1, 3, 16), Kebab);
    assert_eq!(editor.line(3), "parse-html-string");
    editor.assert_selection(sel(3, 1, 3, 18));

    case(&mut editor, sel(4, 1, 4, 15), Kebab);
    assert_eq!(editor.line(4), "get-element-by-id");
    editor.assert_selection(sel(4, 1, 4, 18));

    case(&mut editor, sel(5, 1, 5, 11), Kebab);
    assert_eq!(editor.line(5), "pascal-case");
    editor.assert_selection(sel(5, 1, 5, 12));

    case(&mut editor, sel(6, 1, 6, 11), Kebab);
    assert_eq!(editor.line(6), "öçş-öç-şğü-ğü");
    editor.assert_selection(sel(6, 1, 6, 14));

    case(&mut editor, sel(7, 1, 7, 34), Kebab);
    assert_eq!(editor.line(7), "audio-converter.convert-m4a-to-mp3();");
    editor.assert_selection(sel(7, 1, 7, 38));

    case(&mut editor, sel(8, 1, 8, 19), Kebab);
    assert_eq!(editor.line(8), "capital-snake-case");
    editor.assert_selection(sel(8, 1, 8, 19));

    case(&mut editor, sel(9, 1, 9, 17), Kebab);
    assert_eq!(editor.line(9), "parse-html4-string");
    editor.assert_selection(sel(9, 1, 9, 19));

    case(&mut editor, sel(10, 1, 10, 28), Kebab);
    assert_eq!(editor.line(10), "_accessor: services-accessor");
    editor.assert_selection(sel(10, 1, 10, 29));

    case(&mut editor, sel(11, 1, 11, 11), Kebab);
    assert_eq!(editor.line(11), "kebab-case");
    editor.assert_selection(sel(11, 1, 11, 11));

    let mut editor = TestEditor::lines(&[
        "hello world",
        "öçşğü",
        "parseHTMLString",
        "getElementById",
        "PascalCase",
        "öçşÖÇŞğüĞÜ",
        "audioConverter.convertM4AToMP3();",
        "Capital_Snake_Case",
        "parseHTML4String",
        "Kebab-Case",
        "FOO_BAR",
        "FOO BAR A",
        "xML_HTTP-reQUEsT",
        "ÉCOLE",
        "ΩMEGA_CASE",
        "ДОМ_ТЕСТ",
    ]);

    case(&mut editor, sel(1, 1, 1, 12), Pascal);
    assert_eq!(editor.line(1), "HelloWorld");
    editor.assert_selection(sel(1, 1, 1, 11));

    case(&mut editor, sel(2, 1, 2, 6), Pascal);
    assert_eq!(editor.line(2), "Öçşğü");
    editor.assert_selection(sel(2, 1, 2, 6));

    case(&mut editor, sel(3, 1, 3, 16), Pascal);
    assert_eq!(editor.line(3), "ParseHTMLString");
    editor.assert_selection(sel(3, 1, 3, 16));

    case(&mut editor, sel(4, 1, 4, 15), Pascal);
    assert_eq!(editor.line(4), "GetElementById");
    editor.assert_selection(sel(4, 1, 4, 15));

    case(&mut editor, sel(5, 1, 5, 11), Pascal);
    assert_eq!(editor.line(5), "PascalCase");
    editor.assert_selection(sel(5, 1, 5, 11));

    case(&mut editor, sel(6, 1, 6, 11), Pascal);
    assert_eq!(editor.line(6), "ÖçşÖÇŞğüĞÜ");
    editor.assert_selection(sel(6, 1, 6, 11));

    case(&mut editor, sel(7, 1, 7, 34), Pascal);
    assert_eq!(editor.line(7), "AudioConverter.ConvertM4AToMP3();");
    editor.assert_selection(sel(7, 1, 7, 34));

    case(&mut editor, sel(8, 1, 8, 19), Pascal);
    assert_eq!(editor.line(8), "CapitalSnakeCase");
    editor.assert_selection(sel(8, 1, 8, 17));

    case(&mut editor, sel(9, 1, 9, 17), Pascal);
    assert_eq!(editor.line(9), "ParseHTML4String");
    editor.assert_selection(sel(9, 1, 9, 17));

    case(&mut editor, sel(10, 1, 10, 11), Pascal);
    assert_eq!(editor.line(10), "KebabCase");
    editor.assert_selection(sel(10, 1, 10, 10));

    case(&mut editor, sel(9, 1, 10, 11), Pascal);
    assert_eq!(
        value_in_range(&editor, sel(9, 1, 10, 11)),
        "ParseHTML4String\nKebabCase"
    );
    editor.assert_selection(sel(9, 1, 10, 10));

    case(&mut editor, sel(11, 1, 11, 8), Pascal);
    assert_eq!(editor.line(11), "FooBar");
    editor.assert_selection(sel(11, 1, 11, 7));

    case(&mut editor, sel(12, 1, 12, 10), Pascal);
    assert_eq!(editor.line(12), "FooBarA");
    editor.assert_selection(sel(12, 1, 12, 8));

    case(&mut editor, sel(13, 1, 13, 17), Pascal);
    assert_eq!(editor.line(13), "XmlHttpReQUEsT");
    editor.assert_selection(sel(13, 1, 13, 15));

    case(&mut editor, sel(14, 1, 14, 6), Pascal);
    assert_eq!(editor.line(14), "École");
    editor.assert_selection(sel(14, 1, 14, 6));

    case(&mut editor, sel(15, 1, 15, 11), Pascal);
    assert_eq!(editor.line(15), "ΩmegaCase");
    editor.assert_selection(sel(15, 1, 15, 10));

    case(&mut editor, sel(16, 1, 16, 9), Pascal);
    assert_eq!(editor.line(16), "ДомТест");
    editor.assert_selection(sel(16, 1, 16, 8));
}

// ---------------------------------------------------------------------------
// DeleteAllRightAction

/// Monaco: should be noop on empty
#[test]
fn delete_all_right_should_be_noop_on_empty() {
    let mut editor = TestEditor::lines(&[""]);

    editor.run(delete_all_right);
    assert_lines(&editor, &[""]);
    editor.assert_selections(&[sel(1, 1, 1, 1)]);

    editor.set_selection(sel(1, 1, 1, 1));
    editor.run(delete_all_right);
    assert_lines(&editor, &[""]);
    editor.assert_selections(&[sel(1, 1, 1, 1)]);

    editor.set_selections(&[sel(1, 1, 1, 1), sel(1, 1, 1, 1), sel(1, 1, 1, 1)]);
    editor.run(delete_all_right);
    assert_lines(&editor, &[""]);
    editor.assert_selections(&[sel(1, 1, 1, 1)]);
}

/// Monaco: should delete selected range
#[test]
fn delete_all_right_should_delete_selected_range() {
    let mut editor = TestEditor::lines(&["hello", "world"]);

    editor.set_selection(sel(1, 2, 1, 5));
    editor.run(delete_all_right);
    assert_lines(&editor, &["ho", "world"]);
    editor.assert_selections(&[sel(1, 2, 1, 2)]);

    editor.set_selection(sel(1, 1, 2, 4));
    editor.run(delete_all_right);
    assert_lines(&editor, &["ld"]);
    editor.assert_selections(&[sel(1, 1, 1, 1)]);

    editor.set_selection(sel(1, 1, 1, 3));
    editor.run(delete_all_right);
    assert_lines(&editor, &[""]);
    editor.assert_selections(&[sel(1, 1, 1, 1)]);
}

/// Monaco: should delete to the right of the cursor
#[test]
fn delete_all_right_should_delete_to_the_right_of_the_cursor() {
    let mut editor = TestEditor::lines(&["hello", "world"]);

    editor.set_selection(sel(1, 3, 1, 3));
    editor.run(delete_all_right);
    assert_lines(&editor, &["he", "world"]);
    editor.assert_selections(&[sel(1, 3, 1, 3)]);

    editor.set_selection(sel(2, 1, 2, 1));
    editor.run(delete_all_right);
    assert_lines(&editor, &["he", ""]);
    editor.assert_selections(&[sel(2, 1, 2, 1)]);
}

/// Monaco: should join two lines, if at the end of the line
#[test]
fn delete_all_right_should_join_two_lines_if_at_the_end_of_the_line() {
    let mut editor = TestEditor::lines(&["hello", "world"]);

    editor.set_selection(sel(1, 6, 1, 6));
    editor.run(delete_all_right);
    assert_lines(&editor, &["helloworld"]);
    editor.assert_selections(&[sel(1, 6, 1, 6)]);

    editor.set_selection(sel(1, 6, 1, 6));
    editor.run(delete_all_right);
    assert_lines(&editor, &["hello"]);
    editor.assert_selections(&[sel(1, 6, 1, 6)]);

    editor.set_selection(sel(1, 6, 1, 6));
    editor.run(delete_all_right);
    assert_lines(&editor, &["hello"]);
    editor.assert_selections(&[sel(1, 6, 1, 6)]);
}

/// Monaco: should work with multiple cursors
#[test]
fn delete_all_right_should_work_with_multiple_cursors() {
    let mut editor = TestEditor::lines(&["hello", "there", "world"]);
    editor.set_selections(&[sel(1, 3, 1, 3), sel(1, 6, 1, 6), sel(3, 4, 3, 4)]);

    editor.run(delete_all_right);
    assert_lines(&editor, &["hethere", "wor"]);
    editor.assert_selections(&[sel(1, 3, 1, 3), sel(2, 4, 2, 4)]);

    editor.run(delete_all_right);
    assert_lines(&editor, &["he", "wor"]);
    editor.assert_selections(&[sel(1, 3, 1, 3), sel(2, 4, 2, 4)]);

    editor.run(delete_all_right);
    assert_lines(&editor, &["hewor"]);
    editor.assert_selections(&[sel(1, 3, 1, 3), sel(1, 6, 1, 6)]);

    editor.run(delete_all_right);
    assert_lines(&editor, &["he"]);
    editor.assert_selections(&[sel(1, 3, 1, 3)]);

    editor.run(delete_all_right);
    assert_lines(&editor, &["he"]);
    editor.assert_selections(&[sel(1, 3, 1, 3)]);
}

/// Monaco: should work with undo/redo
#[test]
fn delete_all_right_should_work_with_undo_redo() {
    let mut editor = TestEditor::lines(&["hello", "there", "world"]);
    editor.set_selections(&[sel(1, 3, 1, 3), sel(1, 6, 1, 6), sel(3, 4, 3, 4)]);

    editor.run(delete_all_right);
    assert_lines(&editor, &["hethere", "wor"]);
    editor.assert_selections(&[sel(1, 3, 1, 3), sel(2, 4, 2, 4)]);

    editor.undo();
    editor.assert_selections(&[sel(1, 3, 1, 3), sel(1, 6, 1, 6), sel(3, 4, 3, 4)]);

    editor.redo();
    editor.assert_selections(&[sel(1, 3, 1, 3), sel(2, 4, 2, 4)]);
}

// ---------------------------------------------------------------------------
// moveCarretCommand.test.ts

#[track_caller]
fn test_move_caret(
    left: bool,
    lines: &[&str],
    selection: Selection,
    expected_lines: &[&str],
    expected_selection: Selection,
) {
    let mut editor = TestEditor::lines(lines);
    editor.set_selection(selection);
    editor.run(|c, b, cur| move_caret(c, b, cur, left));
    assert_lines(&editor, expected_lines);
    editor.assert_selection(expected_selection);
}

/// Monaco: move selection to left
#[test]
fn move_caret_move_selection_to_left() {
    test_move_caret(
        true,
        &["012345"],
        sel(1, 3, 1, 5),
        &["023145"],
        sel(1, 2, 1, 4),
    );
}

/// Monaco: move selection to right
#[test]
fn move_caret_move_selection_to_right() {
    test_move_caret(
        false,
        &["012345"],
        sel(1, 3, 1, 5),
        &["014235"],
        sel(1, 4, 1, 6),
    );
}

/// Monaco: move selection to left - from first column - no change
#[test]
fn move_caret_move_selection_to_left_from_first_column_no_change() {
    test_move_caret(
        true,
        &["012345"],
        sel(1, 1, 1, 1),
        &["012345"],
        sel(1, 1, 1, 1),
    );
}

/// Monaco: move selection to right - from last column - no change
#[test]
fn move_caret_move_selection_to_right_from_last_column_no_change() {
    test_move_caret(
        false,
        &["012345"],
        sel(1, 5, 1, 7),
        &["012345"],
        sel(1, 5, 1, 7),
    );
}

// ---------------------------------------------------------------------------
// Duplicate Selection (copyLinesCommand.test.ts)

#[track_caller]
fn test_duplicate_selection(
    lines: &[&str],
    selections: &[Selection],
    expected_lines: &[&str],
    expected_selections: &[Selection],
) {
    let mut editor = TestEditor::lines(lines);
    editor.set_selections(selections);
    editor.run(duplicate_selection);
    assert_lines(&editor, expected_lines);
    editor.assert_selections(expected_selections);
}

/// Monaco: empty selection
#[test]
fn duplicate_selection_empty_selection() {
    test_duplicate_selection(
        &["first", "second line", "third line", "fourth line", "fifth"],
        &[sel(2, 2, 2, 2), sel(3, 2, 3, 2)],
        &[
            "first",
            "second line",
            "second line",
            "third line",
            "third line",
            "fourth line",
            "fifth",
        ],
        &[sel(3, 2, 3, 2), sel(5, 2, 5, 2)],
    );
}

/// Monaco: with selection
#[test]
fn duplicate_selection_with_selection() {
    test_duplicate_selection(
        &["first", "second line", "third line", "fourth line", "fifth"],
        &[sel(2, 1, 2, 4), sel(3, 1, 3, 4)],
        &[
            "first",
            "secsecond line",
            "thithird line",
            "fourth line",
            "fifth",
        ],
        &[sel(2, 4, 2, 7), sel(3, 4, 3, 7)],
    );
}

/// Monaco `trimTrailingWhitespaceCommand.test.ts`: remove trailing
/// whitespace (the `TrimTrailingWhitespaceCommand` cases; the cursor-aware
/// variant is VS Code's auto-save path, which isn't exposed).
#[test]
fn trim_trailing_whitespace_command() {
    #[track_caller]
    fn check(lines: &[&str], expected: &[(u32, u32, u32, u32)]) {
        let editor = TestEditor::lines(lines);
        let op = crate::cursor::trim_trailing_whitespace(
            &editor.config,
            &editor.buffer,
            &editor.cursors,
        );
        let actual: Vec<(u32, u32, u32, u32)> = op
            .edits
            .iter()
            .map(|(range, text)| {
                assert!(text.is_empty());
                let (a, b) = (range.start.to_monaco(), range.end.to_monaco());
                (a.0, a.1, b.0, b.1)
            })
            .collect();
        assert_eq!(actual, expected);
    }
    check(&[""], &[]);
    check(&["text"], &[]);
    check(&["text   "], &[(1, 5, 1, 8)]);
    check(&["text\t   "], &[(1, 5, 1, 9)]);
    check(&["\t   "], &[(1, 1, 1, 5)]);
    check(&["text\t"], &[(1, 5, 1, 6)]);
    check(
        &[
            "some text\t",
            "some more text",
            "\t  ",
            "even more text  ",
            "and some mixed\t   \t",
        ],
        &[(1, 10, 1, 11), (3, 1, 3, 4), (4, 15, 4, 17), (5, 15, 5, 20)],
    );
}

/// Monaco `trimTrailingWhitespaceCommand.test.ts`: skips strings and regex
/// if configured. Its tokenizer's tokens, as [`LanguageContext`] token
/// types (Monaco's `trimInRegexesAndStrings: false`, as standalone runs).
///
/// [`LanguageContext`]: crate::cursor::LanguageContext
#[test]
fn trim_trailing_whitespace_skips_strings_and_regex() {
    use crate::cursor::{LanguageContext, Position, StandardTokenType};
    let editor = TestEditor::lines(&["const a = `  ", "  a string  ", "`;  "]);
    // The token of the character before `p` (0-based rows and columns).
    let token_type_at = |p: Position| {
        let offset = p.column.saturating_sub(1);
        let string = match p.row {
            // `0, otherMetadata, 10, stringMetadata`
            0 => offset >= 10,
            // `0, stringMetadata`
            1 => true,
            // `0, stringMetadata, 1, otherMetadata`
            _ => offset < 1,
        };
        if string {
            StandardTokenType::String
        } else {
            StandardTokenType::Other
        }
    };
    let config = syntax::LanguageConfiguration::default();
    let language = LanguageContext::new(&config, &token_type_at);
    let op = crate::cursor::trim_trailing_whitespace_with_language(
        &editor.config,
        &editor.buffer,
        &editor.cursors,
        Some(&language),
    );
    let actual: Vec<_> = op
        .edits
        .iter()
        .map(|(range, text)| {
            (
                range.start.to_monaco(),
                range.end.to_monaco(),
                text.as_str(),
            )
        })
        .collect();
    assert_eq!(actual, vec![((3, 3), (3, 5), "")]);
}
