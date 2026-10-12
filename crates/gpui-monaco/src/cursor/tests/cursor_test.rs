//! Port of Monaco's `src/vs/editor/test/browser/controller/cursor.test.ts`
//! (MIT), suite `Editor Controller - Cursor`. The other suites of that file
//! are in `cursor_editing_test.rs`.
//!
//! Skipped in this suite:
//! - `issue #144041: Cursor up/down works`, `issue #140195: Cursor up/down
//!   makes progress`: need word wrap / injected text (view model).
//! - `no move doesn't trigger event`, `move eventing`, `move in selection
//!   mode eventing`, `setSelection / setPosition with source`: test editor
//!   events, which live in the GPUI view.
//! - `saveState & restoreState`: view state serialization is a view concern.
//! - `Independent model edit 1`: carrying cursors through edits made by
//!   others is done by the view (`readSelectionFromMarkers`); it's in
//!   `tests/cursor_movement.rs`, with `sideEditing.test.ts`.

use super::harness::{TestEditor, pos, sel};
use crate::cursor::{ColumnSelectCommand, column_select, column_select_to};

const LINE1: &str = "    \tMy First Line\t ";
const LINE2: &str = "\tMy Second Line";
const LINE3: &str = "    Third Line🐶";
const LINE4: &str = "";
const LINE5: &str = "1";

fn len(s: &str) -> u32 {
    s.encode_utf16().count() as u32
}

fn run_test() -> TestEditor {
    let text = format!("{LINE1}\r\n{LINE2}\n{LINE3}\n{LINE4}\r\n{LINE5}");
    TestEditor::new(&text)
}

#[test]
fn cursor_initialized() {
    let editor = run_test();
    editor.assert_cursor(1, 1);
}

// --------- absolute move

#[test]
fn no_move() {
    let mut editor = run_test();
    editor.move_to(1, 1, false);
    editor.assert_cursor(1, 1);
}

#[test]
fn move_() {
    let mut editor = run_test();
    editor.move_to(1, 2, false);
    editor.assert_cursor(1, 2);
}

#[test]
fn move_in_selection_mode() {
    let mut editor = run_test();
    editor.move_to(1, 2, true);
    editor.assert_selection(sel(1, 1, 1, 2));
}

#[test]
fn move_beyond_line_end() {
    let mut editor = run_test();
    editor.move_to(1, 25, false);
    editor.assert_cursor(1, len(LINE1) + 1);
}

#[test]
fn move_empty_line() {
    let mut editor = run_test();
    editor.move_to(4, 20, false);
    editor.assert_cursor(4, 1);
}

#[test]
fn move_one_char_line() {
    let mut editor = run_test();
    editor.move_to(5, 20, false);
    editor.assert_cursor(5, 2);
}

#[test]
fn selection_down() {
    let mut editor = run_test();
    editor.move_to(2, 1, true);
    editor.assert_selection(sel(1, 1, 2, 1));
}

#[test]
fn move_and_then_select() {
    let mut editor = run_test();
    editor.move_to(2, 3, false);
    editor.assert_cursor(2, 3);
    editor.move_to(2, 15, true);
    editor.assert_selection(sel(2, 3, 2, 15));
    editor.move_to(1, 2, true);
    editor.assert_selection(sel(2, 3, 1, 2));
}

// --------- move left

#[test]
fn move_left_on_top_left_position() {
    let mut editor = run_test();
    editor.move_left(false);
    editor.assert_cursor(1, 1);
}

#[test]
fn move_left() {
    let mut editor = run_test();
    editor.move_to(1, 3, false);
    editor.assert_cursor(1, 3);
    editor.move_left(false);
    editor.assert_cursor(1, 2);
}

#[test]
fn move_left_with_surrogate_pair() {
    let mut editor = run_test();
    editor.move_to(3, 17, false);
    editor.assert_cursor(3, 17);
    editor.move_left(false);
    editor.assert_cursor(3, 15);
}

#[test]
fn move_left_goes_to_previous_row() {
    let mut editor = run_test();
    editor.move_to(2, 1, false);
    editor.assert_cursor(2, 1);
    editor.move_left(false);
    editor.assert_cursor(1, 21);
}

#[test]
fn move_left_selection() {
    let mut editor = run_test();
    editor.move_to(2, 1, false);
    editor.assert_cursor(2, 1);
    editor.move_left(true);
    editor.assert_selection(sel(2, 1, 1, 21));
}

// --------- move right

#[test]
fn move_right_on_bottom_right_position() {
    let mut editor = run_test();
    editor.move_to(5, 2, false);
    editor.assert_cursor(5, 2);
    editor.move_right(false);
    editor.assert_cursor(5, 2);
}

#[test]
fn move_right() {
    let mut editor = run_test();
    editor.move_to(1, 3, false);
    editor.assert_cursor(1, 3);
    editor.move_right(false);
    editor.assert_cursor(1, 4);
}

#[test]
fn move_right_with_surrogate_pair() {
    let mut editor = run_test();
    editor.move_to(3, 15, false);
    editor.assert_cursor(3, 15);
    editor.move_right(false);
    editor.assert_cursor(3, 17);
}

#[test]
fn move_right_goes_to_next_row() {
    let mut editor = run_test();
    editor.move_to(1, 21, false);
    editor.assert_cursor(1, 21);
    editor.move_right(false);
    editor.assert_cursor(2, 1);
}

#[test]
fn move_right_selection() {
    let mut editor = run_test();
    editor.move_to(1, 21, false);
    editor.assert_cursor(1, 21);
    editor.move_right(true);
    editor.assert_selection(sel(1, 21, 2, 1));
}

// --------- move down

#[test]
fn move_down() {
    let mut editor = run_test();
    editor.move_down(false);
    editor.assert_cursor(2, 1);
    editor.move_down(false);
    editor.assert_cursor(3, 1);
    editor.move_down(false);
    editor.assert_cursor(4, 1);
    editor.move_down(false);
    editor.assert_cursor(5, 1);
    editor.move_down(false);
    editor.assert_cursor(5, 2);
}

#[test]
fn move_down_with_selection() {
    let mut editor = run_test();
    editor.move_down(true);
    editor.assert_selection(sel(1, 1, 2, 1));
    editor.move_down(true);
    editor.assert_selection(sel(1, 1, 3, 1));
    editor.move_down(true);
    editor.assert_selection(sel(1, 1, 4, 1));
    editor.move_down(true);
    editor.assert_selection(sel(1, 1, 5, 1));
    editor.move_down(true);
    editor.assert_selection(sel(1, 1, 5, 2));
}

#[test]
fn move_down_with_tabs() {
    let mut editor = run_test();
    editor.move_to(1, 5, false);
    editor.assert_cursor(1, 5);
    editor.move_down(false);
    editor.assert_cursor(2, 2);
    editor.move_down(false);
    editor.assert_cursor(3, 5);
    editor.move_down(false);
    editor.assert_cursor(4, 1);
    editor.move_down(false);
    editor.assert_cursor(5, 2);
}

// --------- move up

#[test]
fn move_up() {
    let mut editor = run_test();
    editor.move_to(3, 5, false);
    editor.assert_cursor(3, 5);
    editor.move_up(false);
    editor.assert_cursor(2, 2);
    editor.move_up(false);
    editor.assert_cursor(1, 5);
}

#[test]
fn move_up_with_selection() {
    let mut editor = run_test();
    editor.move_to(3, 5, false);
    editor.assert_cursor(3, 5);
    editor.move_up(true);
    editor.assert_selection(sel(3, 5, 2, 2));
    editor.move_up(true);
    editor.assert_selection(sel(3, 5, 1, 5));
}

#[test]
fn move_up_and_down_with_tabs() {
    let mut editor = run_test();
    editor.move_to(1, 5, false);
    editor.assert_cursor(1, 5);
    editor.move_down(false);
    editor.move_down(false);
    editor.move_down(false);
    editor.move_down(false);
    editor.assert_cursor(5, 2);
    editor.move_up(false);
    editor.assert_cursor(4, 1);
    editor.move_up(false);
    editor.assert_cursor(3, 5);
    editor.move_up(false);
    editor.assert_cursor(2, 2);
    editor.move_up(false);
    editor.assert_cursor(1, 5);
}

#[test]
fn move_up_and_down_with_end_of_lines_starting_from_a_long_one() {
    let mut editor = run_test();
    editor.move_to_end_of_line(false);
    editor.assert_cursor(1, len(LINE1) + 1);
    editor.move_to_end_of_line(false);
    editor.assert_cursor(1, len(LINE1) + 1);
    editor.move_down(false);
    editor.assert_cursor(2, len(LINE2) + 1);
    editor.move_down(false);
    editor.assert_cursor(3, len(LINE3) + 1);
    editor.move_down(false);
    editor.assert_cursor(4, len(LINE4) + 1);
    editor.move_down(false);
    editor.assert_cursor(5, len(LINE5) + 1);
    editor.move_up(false);
    editor.move_up(false);
    editor.move_up(false);
    editor.move_up(false);
    editor.assert_cursor(1, len(LINE1) + 1);
}

#[test]
fn issue_44465_cursor_position_not_correct_when_move() {
    let mut editor = run_test();
    editor.set_selections(&[sel(1, 5, 1, 5)]);
    // going once up on the first line remembers the offset visual columns
    editor.move_up(false);
    editor.assert_cursor(1, 1);
    editor.move_down(false);
    editor.assert_cursor(2, 2);
    editor.move_up(false);
    editor.assert_cursor(1, 5);

    // going twice up on the first line discards the offset visual columns
    editor.move_up(false);
    editor.assert_cursor(1, 1);
    editor.move_up(false);
    editor.assert_cursor(1, 1);
    editor.move_down(false);
    editor.assert_cursor(2, 1);
}

// --------- move to beginning of line

#[test]
fn move_to_beginning_of_line() {
    let mut editor = run_test();
    editor.move_to_beginning_of_line(false);
    editor.assert_cursor(1, 6);
    editor.move_to_beginning_of_line(false);
    editor.assert_cursor(1, 1);
}

#[test]
fn move_to_beginning_of_line_from_within_line() {
    let mut editor = run_test();
    editor.move_to(1, 8, false);
    editor.move_to_beginning_of_line(false);
    editor.assert_cursor(1, 6);
    editor.move_to_beginning_of_line(false);
    editor.assert_cursor(1, 1);
}

#[test]
fn move_to_beginning_of_line_from_whitespace_at_beginning_of_line() {
    let mut editor = run_test();
    editor.move_to(1, 2, false);
    editor.move_to_beginning_of_line(false);
    editor.assert_cursor(1, 6);
    editor.move_to_beginning_of_line(false);
    editor.assert_cursor(1, 1);
}

#[test]
fn move_to_beginning_of_line_from_within_line_selection() {
    let mut editor = run_test();
    editor.move_to(1, 8, false);
    editor.move_to_beginning_of_line(true);
    editor.assert_selection(sel(1, 8, 1, 6));
    editor.move_to_beginning_of_line(true);
    editor.assert_selection(sel(1, 8, 1, 1));
}

#[test]
fn move_to_beginning_of_line_with_selection_multiline_forward() {
    let mut editor = run_test();
    editor.move_to(1, 8, false);
    editor.move_to(3, 9, true);
    editor.move_to_beginning_of_line(false);
    editor.assert_selection(sel(3, 5, 3, 5));
}

#[test]
fn move_to_beginning_of_line_with_selection_multiline_backward() {
    let mut editor = run_test();
    editor.move_to(3, 9, false);
    editor.move_to(1, 8, true);
    editor.move_to_beginning_of_line(false);
    editor.assert_selection(sel(1, 6, 1, 6));
}

#[test]
fn move_to_beginning_of_line_with_selection_single_line_forward() {
    let mut editor = run_test();
    editor.move_to(3, 2, false);
    editor.move_to(3, 9, true);
    editor.move_to_beginning_of_line(false);
    editor.assert_selection(sel(3, 5, 3, 5));
}

#[test]
fn move_to_beginning_of_line_with_selection_single_line_backward() {
    let mut editor = run_test();
    editor.move_to(3, 9, false);
    editor.move_to(3, 2, true);
    editor.move_to_beginning_of_line(false);
    editor.assert_selection(sel(3, 5, 3, 5));
}

#[test]
fn issue_15401_end_key_is_behaving_weird_when_text_is_selected_part_1() {
    let mut editor = run_test();
    editor.move_to(1, 8, false);
    editor.move_to(3, 9, true);
    editor.move_to_beginning_of_line(false);
    editor.assert_selection(sel(3, 5, 3, 5));
}

#[test]
fn issue_17011_shift_home_end_go_to_the_end_of_the_selection_starts_line() {
    let mut editor = run_test();
    editor.move_to(1, 8, false);
    editor.move_to(3, 9, true);
    editor.move_to_beginning_of_line(true);
    editor.assert_selection(sel(1, 8, 3, 5));
}

// --------- move to end of line

#[test]
fn move_to_end_of_line() {
    let mut editor = run_test();
    editor.move_to_end_of_line(false);
    editor.assert_cursor(1, len(LINE1) + 1);
    editor.move_to_end_of_line(false);
    editor.assert_cursor(1, len(LINE1) + 1);
}

#[test]
fn move_to_end_of_line_from_within_line() {
    let mut editor = run_test();
    editor.move_to(1, 6, false);
    editor.move_to_end_of_line(false);
    editor.assert_cursor(1, len(LINE1) + 1);
    editor.move_to_end_of_line(false);
    editor.assert_cursor(1, len(LINE1) + 1);
}

#[test]
fn move_to_end_of_line_from_whitespace_at_end_of_line() {
    let mut editor = run_test();
    editor.move_to(1, 20, false);
    editor.move_to_end_of_line(false);
    editor.assert_cursor(1, len(LINE1) + 1);
    editor.move_to_end_of_line(false);
    editor.assert_cursor(1, len(LINE1) + 1);
}

#[test]
fn move_to_end_of_line_from_within_line_selection() {
    let mut editor = run_test();
    editor.move_to(1, 6, false);
    editor.move_to_end_of_line(true);
    editor.assert_selection(sel(1, 6, 1, len(LINE1) + 1));
    editor.move_to_end_of_line(true);
    editor.assert_selection(sel(1, 6, 1, len(LINE1) + 1));
}

#[test]
fn move_to_end_of_line_with_selection_multiline_forward() {
    let mut editor = run_test();
    editor.move_to(1, 1, false);
    editor.move_to(3, 9, true);
    editor.move_to_end_of_line(false);
    editor.assert_selection(sel(3, 17, 3, 17));
}

#[test]
fn move_to_end_of_line_with_selection_multiline_backward() {
    let mut editor = run_test();
    editor.move_to(3, 9, false);
    editor.move_to(1, 1, true);
    editor.move_to_end_of_line(false);
    editor.assert_selection(sel(1, 21, 1, 21));
}

#[test]
fn move_to_end_of_line_with_selection_single_line_forward() {
    let mut editor = run_test();
    editor.move_to(3, 1, false);
    editor.move_to(3, 9, true);
    editor.move_to_end_of_line(false);
    editor.assert_selection(sel(3, 17, 3, 17));
}

#[test]
fn move_to_end_of_line_with_selection_single_line_backward() {
    let mut editor = run_test();
    editor.move_to(3, 9, false);
    editor.move_to(3, 1, true);
    editor.move_to_end_of_line(false);
    editor.assert_selection(sel(3, 17, 3, 17));
}

#[test]
fn issue_15401_end_key_is_behaving_weird_when_text_is_selected_part_2() {
    let mut editor = run_test();
    editor.move_to(1, 1, false);
    editor.move_to(3, 9, true);
    editor.move_to_end_of_line(false);
    editor.assert_selection(sel(3, 17, 3, 17));
}

// --------- move to beginning of buffer

#[test]
fn move_to_beginning_of_buffer() {
    let mut editor = run_test();
    editor.move_to_beginning_of_buffer(false);
    editor.assert_cursor(1, 1);
}

#[test]
fn move_to_beginning_of_buffer_from_within_first_line() {
    let mut editor = run_test();
    editor.move_to(1, 3, false);
    editor.move_to_beginning_of_buffer(false);
    editor.assert_cursor(1, 1);
}

#[test]
fn move_to_beginning_of_buffer_from_within_another_line() {
    let mut editor = run_test();
    editor.move_to(3, 3, false);
    editor.move_to_beginning_of_buffer(false);
    editor.assert_cursor(1, 1);
}

#[test]
fn move_to_beginning_of_buffer_from_within_first_line_selection() {
    let mut editor = run_test();
    editor.move_to(1, 3, false);
    editor.move_to_beginning_of_buffer(true);
    editor.assert_selection(sel(1, 3, 1, 1));
}

#[test]
fn move_to_beginning_of_buffer_from_within_another_line_selection() {
    let mut editor = run_test();
    editor.move_to(3, 3, false);
    editor.move_to_beginning_of_buffer(true);
    editor.assert_selection(sel(3, 3, 1, 1));
}

// --------- move to end of buffer

#[test]
fn move_to_end_of_buffer() {
    let mut editor = run_test();
    editor.move_to_end_of_buffer(false);
    editor.assert_cursor(5, len(LINE5) + 1);
}

#[test]
fn move_to_end_of_buffer_from_within_last_line() {
    let mut editor = run_test();
    editor.move_to(5, 1, false);
    editor.move_to_end_of_buffer(false);
    editor.assert_cursor(5, len(LINE5) + 1);
}

#[test]
fn move_to_end_of_buffer_from_within_another_line() {
    let mut editor = run_test();
    editor.move_to(3, 3, false);
    editor.move_to_end_of_buffer(false);
    editor.assert_cursor(5, len(LINE5) + 1);
}

#[test]
fn move_to_end_of_buffer_from_within_last_line_selection() {
    let mut editor = run_test();
    editor.move_to(5, 1, false);
    editor.move_to_end_of_buffer(true);
    editor.assert_selection(sel(5, 1, 5, len(LINE5) + 1));
}

#[test]
fn move_to_end_of_buffer_from_within_another_line_selection() {
    let mut editor = run_test();
    editor.move_to(3, 3, false);
    editor.move_to_end_of_buffer(true);
    editor.assert_selection(sel(3, 3, 5, len(LINE5) + 1));
}

// --------- misc

#[test]
fn select_all() {
    let mut editor = run_test();
    editor.select_all();
    editor.assert_selection(sel(1, 1, 5, len(LINE5) + 1));
}

// --------- column select

fn column_select_keyboard(editor: &mut TestEditor, command: ColumnSelectCommand) {
    let buffer = &editor.buffer;
    column_select(&editor.config, buffer, &mut editor.cursors, command, 10);
}

/// Monaco `CoreNavigationCommands.ColumnSelect` with `position`,
/// `viewPosition` and `mouseColumn`.
fn column_select_mouse(
    editor: &mut TestEditor,
    line: u32,
    column: u32,
    mouse_column: u32,
    do_column_select: bool,
) {
    let buffer = &editor.buffer;
    column_select_to(
        &editor.config,
        buffer,
        &mut editor.cursors,
        pos(line, column),
        mouse_column - 1,
        do_column_select,
    );
}

#[test]
fn column_select_1() {
    let mut editor = TestEditor::lines(&[
        "\tprivate compute(a:number): boolean {",
        "\t\tif (a + 3 === 0 || a + 5 === 0) {",
        "\t\t\treturn false;",
        "\t\t}",
        "\t}",
    ]);
    editor.move_to(1, 7, false);
    editor.assert_cursor(1, 7);

    column_select_mouse(&mut editor, 4, 4, 15, true);

    editor.assert_selections(&[
        sel(1, 7, 1, 12),
        sel(2, 4, 2, 9),
        sel(3, 3, 3, 6),
        sel(4, 4, 4, 4),
    ]);
}

#[test]
fn grapheme_breaking() {
    let mut editor = TestEditor::lines(&[
        "abcabc",
        "a\u{303}a\u{303}a\u{303}a\u{303}a\u{303}a\u{303}",
        "\u{8fbb}\u{e0100}\u{8fbb}\u{e0100}\u{8fbb}\u{e0100}",
        "\u{baa}\u{bc1}",
    ]);

    editor.set_selections(&[sel(2, 1, 2, 1)]);
    editor.move_right(false);
    editor.assert_cursor(2, 3);
    editor.move_left(false);
    editor.assert_cursor(2, 1);

    editor.set_selections(&[sel(3, 1, 3, 1)]);
    editor.move_right(false);
    editor.assert_cursor(3, 4);
    editor.move_left(false);
    editor.assert_cursor(3, 1);

    editor.set_selections(&[sel(4, 1, 4, 1)]);
    editor.move_right(false);
    editor.assert_cursor(4, 3);
    editor.move_left(false);
    editor.assert_cursor(4, 1);

    editor.set_selections(&[sel(1, 3, 1, 3)]);
    editor.move_down(false);
    editor.assert_cursor(2, 5);
    editor.move_down(false);
    editor.assert_cursor(3, 4);
    editor.move_up(false);
    editor.assert_cursor(2, 5);
    editor.move_up(false);
    editor.assert_cursor(1, 3);
}

#[test]
fn issue_4905_column_select_is_biased_to_the_right() {
    let mut editor = TestEditor::lines(&[
        "var gulp = require(\"gulp\");",
        "var path = require(\"path\");",
        "var rimraf = require(\"rimraf\");",
        "var isarray = require(\"isarray\");",
        "var merge = require(\"merge-stream\");",
        "var concat = require(\"gulp-concat\");",
        "var newer = require(\"gulp-newer\");",
    ]);
    editor.move_to(1, 4, false);
    editor.assert_cursor(1, 4);

    column_select_mouse(&mut editor, 4, 1, 1, true);

    editor.assert_selections(&[
        sel(1, 4, 1, 1),
        sel(2, 4, 2, 1),
        sel(3, 4, 3, 1),
        sel(4, 4, 4, 1),
    ]);
}

const PROPERTY_LINES: [&str; 10] = [
    "<property id=\"SomeThing\" key=\"SomeKey\" value=\"000\"/>",
    "<property id=\"SomeThing\" key=\"SomeKey\" value=\"000\"/>",
    "<property id=\"SomeThing\" Key=\"SomeKey\" value=\"000\"/>",
    "<property id=\"SomeThing\" key=\"SomeKey\" value=\"000\"/>",
    "<property id=\"SomeThing\" key=\"SoMEKEy\" value=\"000\"/>",
    "<property id=\"SomeThing\" key=\"SomeKey\" value=\"000\"/>",
    "<property id=\"SomeThing\" key=\"SomeKey\" value=\"000\"/>",
    "<property id=\"SomeThing\" key=\"SomeKey\" valuE=\"000\"/>",
    "<property id=\"SomeThing\" key=\"SomeKey\" value=\"000\"/>",
    "<property id=\"SomeThing\" key=\"SomeKey\" value=\"00X\"/>",
];

#[test]
fn issue_20087_column_select_with_mouse() {
    let mut editor = TestEditor::lines(&PROPERTY_LINES);
    editor.move_to(10, 10, false);
    editor.assert_cursor(10, 10);

    let expected = [
        sel(10, 10, 10, 1),
        sel(9, 10, 9, 1),
        sel(8, 10, 8, 1),
        sel(7, 10, 7, 1),
        sel(6, 10, 6, 1),
        sel(5, 10, 5, 1),
        sel(4, 10, 4, 1),
        sel(3, 10, 3, 1),
        sel(2, 10, 2, 1),
        sel(1, 10, 1, 1),
    ];
    column_select_mouse(&mut editor, 1, 1, 1, true);
    editor.assert_selections(&expected);

    column_select_mouse(&mut editor, 1, 1, 1, true);
    editor.assert_selections(&expected);
}

#[test]
fn issue_20087_column_select_with_keyboard() {
    let mut editor = TestEditor::lines(&PROPERTY_LINES);
    editor.move_to(10, 10, false);
    editor.assert_cursor(10, 10);

    column_select_keyboard(&mut editor, ColumnSelectCommand::Left);
    editor.assert_selections(&[sel(10, 10, 10, 9)]);

    column_select_keyboard(&mut editor, ColumnSelectCommand::Left);
    editor.assert_selections(&[sel(10, 10, 10, 8)]);

    column_select_keyboard(&mut editor, ColumnSelectCommand::Right);
    editor.assert_selections(&[sel(10, 10, 10, 9)]);

    column_select_keyboard(&mut editor, ColumnSelectCommand::Up);
    editor.assert_selections(&[sel(10, 10, 10, 9), sel(9, 10, 9, 9)]);

    column_select_keyboard(&mut editor, ColumnSelectCommand::Down);
    editor.assert_selections(&[sel(10, 10, 10, 9)]);
}

#[test]
fn issue_118062_column_selection_cannot_select_first_position_of_a_line() {
    let mut editor = TestEditor::lines(&["hello world"]);
    editor.move_to(1, 2, false);
    editor.assert_cursor(1, 2);

    column_select_keyboard(&mut editor, ColumnSelectCommand::Left);
    editor.assert_selections(&[sel(1, 2, 1, 1)]);
}

#[test]
fn column_select_with_keyboard() {
    let mut editor = TestEditor::lines(&[
        "var gulp = require(\"gulp\");",
        "var path = require(\"path\");",
        "var rimraf = require(\"rimraf\");",
        "var isarray = require(\"isarray\");",
        "var merge = require(\"merge-stream\");",
        "var concat = require(\"gulp-concat\");",
        "var newer = require(\"gulp-newer\");",
    ]);
    use ColumnSelectCommand::{Down, Left, Right};
    let rows = |ends: [u32; 7]| -> Vec<_> {
        (1..=7)
            .map(|l| sel(l, 4, l, ends[l as usize - 1]))
            .collect()
    };

    editor.move_to(1, 4, false);
    editor.assert_cursor(1, 4);

    column_select_keyboard(&mut editor, Right);
    editor.assert_selections(&[sel(1, 4, 1, 5)]);

    column_select_keyboard(&mut editor, Down);
    editor.assert_selections(&[sel(1, 4, 1, 5), sel(2, 4, 2, 5)]);

    column_select_keyboard(&mut editor, Down);
    editor.assert_selections(&[sel(1, 4, 1, 5), sel(2, 4, 2, 5), sel(3, 4, 3, 5)]);

    for _ in 0..4 {
        column_select_keyboard(&mut editor, Down);
    }
    editor.assert_selections(&rows([5; 7]));

    column_select_keyboard(&mut editor, Right);
    editor.assert_selections(&rows([6; 7]));

    // 10 times
    for _ in 0..10 {
        column_select_keyboard(&mut editor, Right);
    }
    editor.assert_selections(&rows([16; 7]));

    // 10 times
    for _ in 0..10 {
        column_select_keyboard(&mut editor, Right);
    }
    editor.assert_selections(&rows([26; 7]));

    // 2 times => reaching the ending of lines 1 and 2
    for _ in 0..2 {
        column_select_keyboard(&mut editor, Right);
    }
    editor.assert_selections(&rows([28; 7]));

    // 4 times => reaching the ending of line 3
    for _ in 0..4 {
        column_select_keyboard(&mut editor, Right);
    }
    editor.assert_selections(&rows([28, 28, 32, 32, 32, 32, 32]));

    // 2 times => reaching the ending of line 4
    for _ in 0..2 {
        column_select_keyboard(&mut editor, Right);
    }
    editor.assert_selections(&rows([28, 28, 32, 34, 34, 34, 34]));

    // 1 time => reaching the ending of line 7
    column_select_keyboard(&mut editor, Right);
    editor.assert_selections(&rows([28, 28, 32, 34, 35, 35, 35]));

    // 3 times => reaching the ending of lines 5 & 6
    for _ in 0..3 {
        column_select_keyboard(&mut editor, Right);
    }
    editor.assert_selections(&rows([28, 28, 32, 34, 37, 37, 35]));

    // cannot go anywhere anymore
    column_select_keyboard(&mut editor, Right);
    editor.assert_selections(&rows([28, 28, 32, 34, 37, 37, 35]));

    // cannot go anywhere anymore even if we insist
    for _ in 0..4 {
        column_select_keyboard(&mut editor, Right);
    }
    editor.assert_selections(&rows([28, 28, 32, 34, 37, 37, 35]));

    // can easily go back
    column_select_keyboard(&mut editor, Left);
    editor.assert_selections(&rows([28, 28, 32, 34, 36, 36, 35]));
}
