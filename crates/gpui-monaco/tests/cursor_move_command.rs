//! Port of Monaco's
//! `src/vs/editor/test/browser/controller/cursorMoveCommand.test.ts` (MIT):
//! the `cursorMove` command's character, half-line, line-position,
//! model-line, wrapped-line, blank-line and `foldedLine` moves. The
//! viewport cases are unit tests in `cursor/tests/cursor_move_command_test.rs`
//! and run against a real viewport in `reveal.rs`.

use gpui::{AppContext as _, Entity, Focusable as _, TestAppContext, VisualTestContext};
use gpui_monaco::cursor::Selection;
use gpui_monaco::text::PointUtf16;
use gpui_monaco::{CodeEditor, TextModel};
use serde_json::{Value, json};

const TEXT: &str = "    \tMy First Line\t \n\tMy Second Line\n    Third Line🐶\n\n1";

/// Monaco 1-based (line, column).
fn p(line: u32, column: u32) -> PointUtf16 {
    PointUtf16::from_monaco(line, column)
}

fn setup<'a>(
    text: &str,
    cx: &'a mut TestAppContext,
) -> (Entity<CodeEditor>, &'a mut VisualTestContext) {
    cx.update(gpui_monaco::init);
    let text = text.to_string();
    let (editor, cx) = cx.add_window_view(|window, cx| {
        let model = cx.new(|_| TextModel::new(&text).with_uri("file:///a.txt"));
        CodeEditor::new(model, window, cx)
    });
    editor.update(cx, |e, cx| {
        e.update_options_with(
            |o| {
                o.sticky_scroll.enabled = false;
                o.minimap.enabled = false;
            },
            cx,
        );
    });
    cx.update(|window, cx| window.focus(&editor.focus_handle(cx), cx));
    draw(cx);
    (editor, cx)
}

fn draw(cx: &mut VisualTestContext) {
    cx.update(|window, cx| {
        window.refresh();
        window.draw(cx).clear(cx);
    });
    cx.run_until_parked();
}

fn trigger(editor: &Entity<CodeEditor>, cx: &mut VisualTestContext, id: &str, args: Value) {
    editor
        .update_in(cx, |e, window, cx| {
            e.trigger_with_payload(id, args, window, cx)
        })
        .unwrap_or_else(|err| panic!("{id}: {err}"));
    draw(cx);
}

/// Monaco `CoreNavigationCommands.CursorMove` with `args`.
fn cursor_move(editor: &Entity<CodeEditor>, cx: &mut VisualTestContext, args: Value) {
    trigger(editor, cx, "cursorMove", args);
}

/// Monaco `moveTo`: `CoreNavigationCommands.MoveTo`.
fn move_to(editor: &Entity<CodeEditor>, cx: &mut VisualTestContext, line: u32, column: u32) {
    trigger(
        editor,
        cx,
        "_moveTo",
        json!({ "position": { "lineNumber": line, "column": column } }),
    );
}

/// Monaco `cursorEqual`: one cursor at (`line`, `column`) with its
/// selection starting at `start` (the position by default).
#[track_caller]
fn cursor_equal(
    editor: &Entity<CodeEditor>,
    cx: &mut VisualTestContext,
    (line, column): (u32, u32),
    start: Option<(u32, u32)>,
) {
    let (start_line, start_column) = start.unwrap_or((line, column));
    assert_eq!(
        editor.read_with(cx, |e, _| e.selections()),
        [Selection::new(p(start_line, start_column), p(line, column))]
    );
}

fn left(value: Option<u32>, by: Option<&str>) -> Value {
    json!({ "to": "left", "value": value, "by": by })
}

fn right(value: Option<u32>, by: Option<&str>) -> Value {
    json!({ "to": "right", "value": value, "by": by })
}

/// Monaco `moveUp`/`moveDown`: by wrapped lines.
fn vertical(to: &str, value: u32, select: bool) -> Value {
    json!({ "to": to, "by": "wrappedLine", "value": value, "select": select })
}

/// Monaco `moveUpByModelLine`/`moveDownByModelLine`: no unit.
fn by_model_line(to: &str, value: u32) -> Value {
    json!({ "to": to, "value": value })
}

macro_rules! cursor_move_test {
    ($name:ident, from: ($line:expr, $column:expr), $args:expr, to: ($to_line:expr, $to_column:expr)) => {
        #[gpui::test]
        fn $name(cx: &mut TestAppContext) {
            let (editor, cx) = setup(TEXT, cx);
            move_to(&editor, cx, $line, $column);
            cursor_move(&editor, cx, $args);
            cursor_equal(&editor, cx, ($to_line, $to_column), None);
        }
    };
}

// ----- Cursor move command test ----------------------------------------------

cursor_move_test!(move_left_should_move_to_left_character,
    from: (1, 8), left(None, None), to: (1, 7));
cursor_move_test!(move_left_should_move_to_left_by_n_characters,
    from: (1, 8), left(Some(3), None), to: (1, 5));
cursor_move_test!(move_left_should_move_to_left_by_half_line,
    from: (1, 8), left(Some(1), Some("halfLine")), to: (1, 1));
cursor_move_test!(move_left_moves_to_previous_line,
    from: (2, 3), left(Some(10), None), to: (1, 21));
cursor_move_test!(move_right_should_move_to_right_character,
    from: (1, 5), right(None, None), to: (1, 6));
cursor_move_test!(move_right_should_move_to_right_by_n_characters,
    from: (1, 2), right(Some(6), None), to: (1, 8));
cursor_move_test!(move_right_should_move_to_right_by_half_line,
    from: (1, 4), right(Some(1), Some("halfLine")), to: (1, 14));
cursor_move_test!(move_right_moves_to_next_line,
    from: (1, 8), right(Some(100), None), to: (2, 1));

cursor_move_test!(move_to_first_character_of_line_from_middle,
    from: (1, 8), json!({ "to": "wrappedLineStart" }), to: (1, 1));
cursor_move_test!(move_to_first_character_of_line_from_first_non_white_space_character,
    from: (1, 6), json!({ "to": "wrappedLineStart" }), to: (1, 1));
cursor_move_test!(move_to_first_character_of_line_from_first_character,
    from: (1, 1), json!({ "to": "wrappedLineStart" }), to: (1, 1));

cursor_move_test!(move_to_first_non_white_space_character_of_line_from_middle,
    from: (1, 8), json!({ "to": "wrappedLineFirstNonWhitespaceCharacter" }), to: (1, 6));
cursor_move_test!(move_to_first_non_white_space_character_of_line_from_first_non_white_space_character,
    from: (1, 6), json!({ "to": "wrappedLineFirstNonWhitespaceCharacter" }), to: (1, 6));
cursor_move_test!(move_to_first_non_white_space_character_of_line_from_first_character,
    from: (1, 1), json!({ "to": "wrappedLineFirstNonWhitespaceCharacter" }), to: (1, 6));

cursor_move_test!(move_to_end_of_line_from_middle,
    from: (1, 8), json!({ "to": "wrappedLineEnd" }), to: (1, 21));
cursor_move_test!(move_to_end_of_line_from_last_non_white_space_character,
    from: (1, 19), json!({ "to": "wrappedLineEnd" }), to: (1, 21));
cursor_move_test!(move_to_end_of_line_from_line_end,
    from: (1, 21), json!({ "to": "wrappedLineEnd" }), to: (1, 21));

cursor_move_test!(move_to_last_non_white_space_character_from_middle,
    from: (1, 8), json!({ "to": "wrappedLineLastNonWhitespaceCharacter" }), to: (1, 19));
cursor_move_test!(move_to_last_non_white_space_character_from_last_non_white_space_character,
    from: (1, 19), json!({ "to": "wrappedLineLastNonWhitespaceCharacter" }), to: (1, 19));
cursor_move_test!(move_to_last_non_white_space_character_from_line_end,
    from: (1, 21), json!({ "to": "wrappedLineLastNonWhitespaceCharacter" }), to: (1, 19));

cursor_move_test!(move_to_center_of_line_not_from_center,
    from: (1, 8), json!({ "to": "wrappedLineColumnCenter" }), to: (1, 11));
cursor_move_test!(move_to_center_of_line_from_center,
    from: (1, 11), json!({ "to": "wrappedLineColumnCenter" }), to: (1, 11));

#[gpui::test]
fn move_to_center_of_line_from_start(cx: &mut TestAppContext) {
    let (editor, cx) = setup(TEXT, cx);
    cursor_move(&editor, cx, json!({ "to": "wrappedLineStart" }));
    cursor_move(&editor, cx, json!({ "to": "wrappedLineColumnCenter" }));
    cursor_equal(&editor, cx, (1, 11), None);
}

#[gpui::test]
fn move_to_center_of_line_from_end(cx: &mut TestAppContext) {
    let (editor, cx) = setup(TEXT, cx);
    cursor_move(&editor, cx, json!({ "to": "wrappedLineEnd" }));
    cursor_move(&editor, cx, json!({ "to": "wrappedLineColumnCenter" }));
    cursor_equal(&editor, cx, (1, 11), None);
}

#[gpui::test]
fn move_up_by_cursor_move_command(cx: &mut TestAppContext) {
    let (editor, cx) = setup(TEXT, cx);
    move_to(&editor, cx, 3, 5);
    cursor_equal(&editor, cx, (3, 5), None);
    cursor_move(&editor, cx, vertical("up", 2, false));
    cursor_equal(&editor, cx, (1, 5), None);
    cursor_move(&editor, cx, vertical("up", 1, false));
    cursor_equal(&editor, cx, (1, 1), None);
}

#[gpui::test]
fn move_up_by_model_line_cursor_move_command(cx: &mut TestAppContext) {
    let (editor, cx) = setup(TEXT, cx);
    move_to(&editor, cx, 3, 5);
    cursor_equal(&editor, cx, (3, 5), None);
    cursor_move(&editor, cx, by_model_line("up", 2));
    cursor_equal(&editor, cx, (1, 5), None);
    cursor_move(&editor, cx, by_model_line("up", 1));
    cursor_equal(&editor, cx, (1, 1), None);
}

#[gpui::test]
fn move_down_by_model_line_cursor_move_command(cx: &mut TestAppContext) {
    let (editor, cx) = setup(TEXT, cx);
    move_to(&editor, cx, 3, 5);
    cursor_equal(&editor, cx, (3, 5), None);
    cursor_move(&editor, cx, by_model_line("down", 2));
    cursor_equal(&editor, cx, (5, 2), None);
    cursor_move(&editor, cx, by_model_line("down", 1));
    cursor_equal(&editor, cx, (5, 2), None);
}

#[gpui::test]
fn move_up_with_selection_by_cursor_move_command(cx: &mut TestAppContext) {
    let (editor, cx) = setup(TEXT, cx);
    move_to(&editor, cx, 3, 5);
    cursor_equal(&editor, cx, (3, 5), None);
    cursor_move(&editor, cx, vertical("up", 1, true));
    cursor_equal(&editor, cx, (2, 2), Some((3, 5)));
    cursor_move(&editor, cx, vertical("up", 1, true));
    cursor_equal(&editor, cx, (1, 5), Some((3, 5)));
}

#[gpui::test]
fn move_up_and_down_with_tabs_by_cursor_move_command(cx: &mut TestAppContext) {
    let (editor, cx) = setup(TEXT, cx);
    move_to(&editor, cx, 1, 5);
    cursor_equal(&editor, cx, (1, 5), None);
    cursor_move(&editor, cx, vertical("down", 4, false));
    cursor_equal(&editor, cx, (5, 2), None);
    cursor_move(&editor, cx, vertical("up", 1, false));
    cursor_equal(&editor, cx, (4, 1), None);
    cursor_move(&editor, cx, vertical("up", 1, false));
    cursor_equal(&editor, cx, (3, 5), None);
    cursor_move(&editor, cx, vertical("up", 1, false));
    cursor_equal(&editor, cx, (2, 2), None);
    cursor_move(&editor, cx, vertical("up", 1, false));
    cursor_equal(&editor, cx, (1, 5), None);
}

#[gpui::test]
fn move_up_and_down_with_end_of_lines_starting_from_a_long_one_by_cursor_move_command(
    cx: &mut TestAppContext,
) {
    let (editor, cx) = setup(TEXT, cx);
    trigger(&editor, cx, "cursorEnd", json!({}));
    cursor_equal(&editor, cx, (1, 21), None);
    trigger(&editor, cx, "cursorEnd", json!({}));
    cursor_equal(&editor, cx, (1, 21), None);
    cursor_move(&editor, cx, vertical("down", 2, false));
    cursor_equal(&editor, cx, (3, 17), None);
    cursor_move(&editor, cx, vertical("down", 1, false));
    cursor_equal(&editor, cx, (4, 1), None);
    cursor_move(&editor, cx, vertical("down", 1, false));
    cursor_equal(&editor, cx, (5, 2), None);
    cursor_move(&editor, cx, vertical("up", 4, false));
    cursor_equal(&editor, cx, (1, 21), None);
}

// ----- Cursor move by blankline test -----------------------------------------

const BLANK_LINE_TEXT: &str =
    "    \tMy First Line\t \n\tMy Second Line\n    Third Line🐶\n\n1\n2\n3\n\n         \na\nb";

/// Monaco `moveUpByBlankLine`/`moveDownByBlankLine`.
fn blank_line(to: &str, select: bool) -> Value {
    json!({ "to": to, "by": "wrappedLine", "select": select })
}

macro_rules! blank_line_test {
    ($name:ident, from: ($line:expr, $column:expr), $to:expr, select: $select:expr,
        expect: ($pos_line:expr, $pos_column:expr), start: $start:expr) => {
        #[gpui::test]
        fn $name(cx: &mut TestAppContext) {
            let (editor, cx) = setup(BLANK_LINE_TEXT, cx);
            move_to(&editor, cx, $line, $column);
            cursor_move(&editor, cx, blank_line($to, $select));
            cursor_equal(&editor, cx, ($pos_line, $pos_column), $start);
        }
    };
}

blank_line_test!(move_down_should_move_to_start_of_next_blank_line,
    from: (1, 1), "nextBlankLine", select: false, expect: (4, 1), start: None);
blank_line_test!(move_up_should_move_to_start_of_previous_blank_line,
    from: (7, 1), "prevBlankLine", select: false, expect: (4, 1), start: None);
blank_line_test!(move_down_should_skip_over_whitespace_if_already_on_blank_line,
    from: (8, 1), "nextBlankLine", select: false, expect: (11, 1), start: None);
blank_line_test!(move_up_should_skip_over_whitespace_if_already_on_blank_line,
    from: (9, 1), "prevBlankLine", select: false, expect: (4, 1), start: None);
blank_line_test!(move_up_should_go_to_first_column_of_first_line_if_not_empty,
    from: (2, 1), "prevBlankLine", select: false, expect: (1, 1), start: None);
blank_line_test!(move_down_should_go_to_first_column_of_last_line_if_not_empty,
    from: (10, 1), "nextBlankLine", select: false, expect: (11, 1), start: None);
blank_line_test!(select_down_should_select_to_start_of_next_blank_line,
    from: (1, 1), "nextBlankLine", select: true, expect: (4, 1), start: Some((1, 1)));
blank_line_test!(select_up_should_select_to_start_of_previous_blank_line,
    from: (7, 1), "prevBlankLine", select: true, expect: (4, 1), start: Some((7, 1)));

// ----- Cursor move command - foldedLine unit ---------------------------------

const FOLD_TEXT: &str = "line1\nline2\nline3\nline4\nline5";

/// Monaco `moveDownByFoldedLine`/`moveUpByFoldedLine`.
fn folded_line(to: &str, value: u32) -> Value {
    json!({ "to": to, "by": "foldedLine", "value": value })
}

/// Monaco `viewModel.setHiddenAreas([new Range(start, 1, end, 1)])`,
/// 1-based lines, inclusive.
fn hide(editor: &Entity<CodeEditor>, cx: &mut VisualTestContext, start: u32, end: u32) {
    editor.update(cx, |e, cx| {
        e.set_hidden_areas(std::iter::once(start - 1..end).collect(), cx)
    });
    draw(cx);
}

#[gpui::test]
fn move_down_by_folded_line_skips_a_fold_below_the_cursor(cx: &mut TestAppContext) {
    let (editor, cx) = setup(FOLD_TEXT, cx);
    // Line 4 is hidden (folded under line 3 as header).
    hide(&editor, cx, 4, 4);
    move_to(&editor, cx, 2, 1);
    cursor_move(&editor, cx, folded_line("down", 1));
    cursor_equal(&editor, cx, (3, 1), None);
    // Line 4 is hidden: lands on line 5.
    cursor_move(&editor, cx, folded_line("down", 1));
    cursor_equal(&editor, cx, (5, 1), None);
}

#[gpui::test]
fn move_up_by_folded_line_skips_a_fold_above_the_cursor(cx: &mut TestAppContext) {
    let (editor, cx) = setup(FOLD_TEXT, cx);
    // Line 3 is hidden (folded under line 2 as header).
    hide(&editor, cx, 3, 3);
    move_to(&editor, cx, 4, 1);
    cursor_move(&editor, cx, folded_line("up", 1));
    cursor_equal(&editor, cx, (2, 1), None);
    cursor_move(&editor, cx, folded_line("up", 1));
    cursor_equal(&editor, cx, (1, 1), None);
}

#[gpui::test]
fn move_down_by_folded_line_with_count_treats_each_fold_as_one_step(cx: &mut TestAppContext) {
    let (editor, cx) = setup(FOLD_TEXT, cx);
    hide(&editor, cx, 3, 3);
    move_to(&editor, cx, 1, 1);
    // 3j from line 1: step 1 → 2, step 2 → 3 (hidden) → 4, step 3 → 5.
    cursor_move(&editor, cx, folded_line("down", 3));
    cursor_equal(&editor, cx, (5, 1), None);
}

#[gpui::test]
fn move_down_by_folded_line_skips_a_multi_line_fold_as_one_step(cx: &mut TestAppContext) {
    let (editor, cx) = setup(FOLD_TEXT, cx);
    // Lines 2-4 are hidden (folded under line 1 as header).
    hide(&editor, cx, 2, 4);
    move_to(&editor, cx, 1, 1);
    cursor_move(&editor, cx, folded_line("down", 1));
    cursor_equal(&editor, cx, (5, 1), None);
}

#[gpui::test]
fn move_down_by_folded_line_at_last_line_stays_at_last_line(cx: &mut TestAppContext) {
    let (editor, cx) = setup(FOLD_TEXT, cx);
    move_to(&editor, cx, 5, 1);
    cursor_move(&editor, cx, folded_line("down", 1));
    cursor_equal(&editor, cx, (5, 1), None);
}

#[gpui::test]
fn move_up_by_folded_line_at_first_line_stays_at_first_line(cx: &mut TestAppContext) {
    let (editor, cx) = setup(FOLD_TEXT, cx);
    move_to(&editor, cx, 1, 1);
    cursor_move(&editor, cx, folded_line("up", 1));
    cursor_equal(&editor, cx, (1, 1), None);
}

#[gpui::test]
fn move_down_by_folded_line_with_count_clamps_to_last_visible_line_after_fold(
    cx: &mut TestAppContext,
) {
    let (editor, cx) = setup(FOLD_TEXT, cx);
    // Lines 2-4 are hidden. Visible lines are 1 and 5.
    hide(&editor, cx, 2, 4);
    move_to(&editor, cx, 1, 1);
    cursor_move(&editor, cx, folded_line("down", 2));
    cursor_equal(&editor, cx, (5, 1), None);
}

#[gpui::test]
fn move_up_by_folded_line_with_count_clamps_to_first_visible_line_before_fold(
    cx: &mut TestAppContext,
) {
    let (editor, cx) = setup(FOLD_TEXT, cx);
    // Lines 2-4 are hidden. Visible lines are 1 and 5.
    hide(&editor, cx, 2, 4);
    move_to(&editor, cx, 5, 1);
    cursor_move(&editor, cx, folded_line("up", 2));
    cursor_equal(&editor, cx, (1, 1), None);
}
