//! Port of Monaco's
//! `src/vs/editor/contrib/wordOperations/test/browser/wordOperations.test.ts`
//! (MIT).
//!
//! `cursorWordLeft - Recognize words` and `cursorWordRight - Recognize words`
//! need the `word-segmenter` cargo feature (`wordSegmenterLocales`).
//!
//! Skipped:
//! - `deleteWordLeft - issue #91855`: needs language auto-closing pairs.

use super::harness::{TestEditor, pos, sel};
use super::word_test_utils::{
    deserialize_pipe_positions, serialize_pipe_positions,
    test_repeated_action_and_extract_positions,
};
use crate::cursor::{CursorConfig, Position, WordNavigationType, delete_inside_word, delete_word};

use WordNavigationType::*;

fn cursor_word_left(ed: &mut TestEditor, select: bool) {
    ed.move_word(true, WordStartFast, select);
}
fn cursor_word_accessibility_left(ed: &mut TestEditor, select: bool) {
    // Monaco's test helper swaps the select/non-select commands; keep that.
    ed.move_word(true, WordAccessibility, !select);
}
fn cursor_word_accessibility_right(ed: &mut TestEditor, select: bool) {
    ed.move_word(false, WordAccessibility, !select);
}
fn cursor_word_start_left(ed: &mut TestEditor, select: bool) {
    ed.move_word(true, WordStart, select);
}
fn cursor_word_end_left(ed: &mut TestEditor, select: bool) {
    ed.move_word(true, WordEnd, select);
}
fn cursor_word_right(ed: &mut TestEditor, select: bool) {
    ed.move_word(false, WordEnd, select);
}
fn move_word_end_right(ed: &mut TestEditor, select: bool) {
    ed.move_word(false, WordEnd, select);
}
fn move_word_start_right(ed: &mut TestEditor, select: bool) {
    ed.move_word(false, WordStart, select);
}
fn delete_word_with(ed: &mut TestEditor, left: bool, nav: WordNavigationType, heuristics: bool) {
    ed.run(|c, b, cur| delete_word(c, b, cur, left, nav, heuristics));
}
fn delete_word_left(ed: &mut TestEditor) {
    delete_word_with(ed, true, WordStart, true);
}
fn delete_word_start_left(ed: &mut TestEditor) {
    delete_word_with(ed, true, WordStart, false);
}
fn delete_word_end_left(ed: &mut TestEditor) {
    delete_word_with(ed, true, WordEnd, false);
}
fn delete_word_right(ed: &mut TestEditor) {
    delete_word_with(ed, false, WordEnd, true);
}
fn delete_word_start_right(ed: &mut TestEditor) {
    delete_word_with(ed, false, WordStart, false);
}
fn delete_word_end_right(ed: &mut TestEditor) {
    delete_word_with(ed, false, WordEnd, false);
}
fn delete_inside_word_(ed: &mut TestEditor, only_word: bool) {
    ed.run(|c, b, cur| delete_inside_word(c, b, cur, only_word));
}

/// The common `testRepeatedActionAndExtractPositions` + `serializePipePositions`
/// round trip, recording the cursor position until it reaches `stop`.
#[track_caller]
fn assert_stops(
    expected: &str,
    initial: Position,
    stop: Position,
    action: impl FnMut(&mut TestEditor),
) {
    assert_stops_with(
        expected,
        initial,
        CursorConfig::default(),
        action,
        move |ed| ed.position() == stop,
    );
}

#[track_caller]
fn assert_stops_with(
    expected: &str,
    initial: Position,
    config: CursorConfig,
    action: impl FnMut(&mut TestEditor),
    stop: impl Fn(&TestEditor) -> bool,
) {
    let (text, _) = deserialize_pipe_positions(expected);
    let actual_stops = test_repeated_action_and_extract_positions(
        &text,
        initial,
        config,
        action,
        |ed| ed.position(),
        stop,
    );
    assert_eq!(serialize_pipe_positions(&text, actual_stops), expected);
}

/// For left deletions: records the cursor until the text is empty.
#[track_caller]
fn assert_delete_left_stops(
    expected: &str,
    initial: Position,
    config: CursorConfig,
    action: impl FnMut(&mut TestEditor),
) {
    assert_stops_with(expected, initial, config, action, |ed| {
        ed.value().is_empty()
    });
}

/// For right deletions: records how much was deleted, until the text is
/// empty.
#[track_caller]
fn assert_delete_right_stops(
    expected: &str,
    config: CursorConfig,
    action: impl FnMut(&mut TestEditor),
) {
    let (text, _) = deserialize_pipe_positions(expected);
    let len = text.encode_utf16().count() as u32;
    let actual_stops = test_repeated_action_and_extract_positions(
        &text,
        pos(1, 1),
        config,
        action,
        |ed| pos(1, len - ed.value().encode_utf16().count() as u32 + 1),
        |ed| ed.value().is_empty(),
    );
    assert_eq!(serialize_pipe_positions(&text, actual_stops), expected);
}

/// The default config with `word_segmenter_locales: [locale]`.
fn segmenter_config(locale: &str) -> CursorConfig {
    CursorConfig {
        word_segmenter_locales: vec![locale.to_string()],
        ..CursorConfig::default()
    }
}

const LINES: [&str; 5] = [
    "    \tMy First Line\t ",
    "\tMy Second Line",
    "    Third Line🐶",
    "",
    "1",
];

#[test]
fn cursor_word_left_simple() {
    let expected = [
        "|    \t|My |First |Line\t ",
        "|\t|My |Second |Line",
        "|    |Third |Line🐶",
        "|",
        "|1",
    ]
    .join("\n");
    assert_stops(&expected, pos(1000, 1000), pos(1, 1), |ed| {
        cursor_word_left(ed, false)
    });
}

#[test]
fn cursor_word_left_with_selection() {
    let mut editor = TestEditor::lines(&LINES);
    editor.set_position(5, 2);
    cursor_word_left(&mut editor, true);
    editor.assert_selection(sel(5, 2, 5, 1));
}

#[test]
fn cursor_word_left_issue_832() {
    let expected = "|   |/* |Just |some   |more   |text |a|+= |3 |+|5-|3 |+ |7 |*/  ";
    assert_stops(expected, pos(1000, 1000), pos(1, 1), |ed| {
        cursor_word_left(ed, false)
    });
}

#[test]
fn cursor_word_left_issue_48046_word_selection_doesnt_work_as_usual() {
    let expected = "|deep.|object.|property";
    assert_stops(expected, pos(1, 21), pos(1, 1), |ed| {
        cursor_word_left(ed, false)
    });
}

#[test]
#[cfg(feature = "word-segmenter")]
fn cursor_word_left_recognize_words() {
    let expected = "|/* |これ|は|テスト|です |/*";
    assert_stops_with(
        expected,
        pos(1000, 1000),
        segmenter_config("ja"),
        |ed| cursor_word_left(ed, true),
        |ed| ed.position() == pos(1, 1),
    );
}

#[test]
fn cursor_word_left_does_not_recognize_words() {
    let expected = "|/* |これはテストです |/*";
    assert_stops_with(
        expected,
        pos(1000, 1000),
        segmenter_config(""),
        |ed| cursor_word_left(ed, true),
        |ed| ed.position() == pos(1, 1),
    );
}

#[test]
fn cursor_word_left_issue_169904_cursors_out_of_sync() {
    let text = [
        ".grid1 {",
        "  display: grid;",
        "  grid-template-columns:",
        "    [full-start] minmax(1em, 1fr)",
        "    [main-start] minmax(0, 40em) [main-end]",
        "    minmax(1em, 1fr) [full-end];",
        "}",
        ".grid2 {",
        "  display: grid;",
        "  grid-template-columns:",
        "    [full-start] minmax(1em, 1fr)",
        "    [main-start] minmax(0, 40em) [main-end] minmax(1em, 1fr) [full-end];",
        "}",
    ];
    let mut editor = TestEditor::lines(&text);
    editor.set_selections(&[
        sel(5, 44, 5, 44),
        sel(6, 32, 6, 32),
        sel(12, 44, 12, 44),
        sel(12, 72, 12, 72),
    ]);
    cursor_word_left(&mut editor, false);
    editor.assert_selections(&[
        sel(5, 43, 5, 43),
        sel(6, 31, 6, 31),
        sel(12, 43, 12, 43),
        sel(12, 71, 12, 71),
    ]);
}

#[test]
fn cursor_word_left_select_issue_74369() {
    let expected = "|this.|is.|a.|test";
    assert_stops(expected, pos(1, 15), pos(1, 1), |ed| {
        cursor_word_left(ed, true)
    });
}

#[test]
fn cursor_word_start_left_() {
    // This is the behaviour observed in Visual Studio, please do not touch test
    let expected = "|   |/* |Just |some   |more   |text |a|+= |3 |+|5|-|3 |+ |7 |*/  ";
    assert_stops(expected, pos(1000, 1000), pos(1, 1), |ed| {
        cursor_word_start_left(ed, false)
    });
}

#[test]
fn cursor_word_start_left_issue_51119() {
    // This is the behaviour observed in Visual Studio, please do not touch test
    let expected = "|this|.|is|.|a|.|test";
    assert_stops(expected, pos(1000, 1000), pos(1, 1), |ed| {
        cursor_word_start_left(ed, false)
    });
}

#[test]
fn issue_51275_cursor_word_start_left_does_not_push_undo_redo_stack_element() {
    let mut editor = TestEditor::new("");
    editor.type_keyboard("foo bar baz");
    assert_eq!(editor.value(), "foo bar baz");

    cursor_word_start_left(&mut editor, false);
    cursor_word_start_left(&mut editor, false);
    editor.type_keyboard("q");

    assert_eq!(editor.value(), "foo qbar baz");

    editor.undo();
    assert_eq!(editor.value(), "foo bar baz");
}

#[test]
fn cursor_word_end_left_() {
    let expected = "|   /*| Just| some|   more|   text| a|+=| 3| +|5|-|3| +| 7| */|  ";
    assert_stops(expected, pos(1000, 1000), pos(1, 1), |ed| {
        cursor_word_end_left(ed, false)
    });
}

#[test]
fn cursor_word_right_simple() {
    let expected = [
        "    \tMy| First| Line|\t |",
        "\tMy| Second| Line|",
        "    Third| Line🐶|",
        "|",
        "1|",
    ]
    .join("\n");
    assert_stops(&expected, pos(1, 1), pos(5, 2), |ed| {
        cursor_word_right(ed, false)
    });
}

#[test]
fn cursor_word_right_selection() {
    let mut editor = TestEditor::lines(&LINES);
    editor.set_position(1, 1);
    cursor_word_right(&mut editor, true);
    editor.assert_selection(sel(1, 1, 1, 8));
}

#[test]
fn cursor_word_right_issue_832() {
    let expected = "   /*| Just| some|   more|   text| a|+=| 3| +5|-3| +| 7| */|  |";
    assert_stops(expected, pos(1, 1), pos(1, 50), |ed| {
        cursor_word_right(ed, false)
    });
}

#[test]
fn cursor_word_right_issue_41199() {
    let expected = "console|.log|(err|)|";
    assert_stops(expected, pos(1, 1), pos(1, 17), |ed| {
        cursor_word_right(ed, false)
    });
}

#[test]
#[cfg(feature = "word-segmenter")]
fn cursor_word_right_recognize_words() {
    let expected = "/*| これ|は|テスト|です|/*|";
    assert_stops_with(
        expected,
        pos(1, 1),
        segmenter_config("ja"),
        |ed| cursor_word_right(ed, false),
        |ed| ed.position() == pos(1, 14),
    );
}

#[test]
fn cursor_word_right_does_not_recognize_words() {
    let expected = "/*| これはテストです|/*|";
    assert_stops_with(
        expected,
        pos(1, 1),
        segmenter_config(""),
        |ed| cursor_word_right(ed, false),
        |ed| ed.position() == pos(1, 14),
    );
}

#[test]
fn move_word_end_right_() {
    let expected = "   /*| Just| some|   more|   text| a|+=| 3| +5|-3| +| 7| */|  |";
    assert_stops(expected, pos(1, 1), pos(1, 50), |ed| {
        move_word_end_right(ed, false)
    });
}

#[test]
fn move_word_start_right_() {
    // This is the behaviour observed in Visual Studio, please do not touch test
    let expected = "   |/* |Just |some   |more   |text |a|+= |3 |+|5|-|3 |+ |7 |*/  |";
    assert_stops(expected, pos(1, 1), pos(1, 50), |ed| {
        move_word_start_right(ed, false)
    });
}

#[test]
fn issue_51119_cursor_word_start_right_regression() {
    // This is the behaviour observed in Visual Studio, please do not touch test
    let expected = "this|.|is|.|a|.|test|";
    assert_stops(expected, pos(1, 1), pos(1, 15), |ed| {
        move_word_start_right(ed, false)
    });
}

#[test]
fn issue_64810_cursor_word_start_right_skips_first_word_after_newline() {
    // This is the behaviour observed in Visual Studio, please do not touch test
    let expected = ["Hello |World|", "|Hei |mailman|"].join("\n");
    assert_stops(&expected, pos(1, 1), pos(2, 12), |ed| {
        move_word_start_right(ed, false)
    });
}

#[test]
fn cursor_word_accessibility_left_() {
    let expected = "|   /* |Just |some   |more   |text |a+= |3 +|5-|3 + |7 */  ";
    assert_stops(expected, pos(1000, 1000), pos(1, 1), |ed| {
        cursor_word_accessibility_left(ed, false)
    });
}

#[test]
fn cursor_word_accessibility_right_() {
    let expected = "   /* |Just |some   |more   |text |a+= |3 +|5-|3 + |7 */  |";
    assert_stops(expected, pos(1, 1), pos(1, 50), |ed| {
        cursor_word_accessibility_right(ed, false)
    });
}

#[test]
fn delete_word_left_for_non_empty_selection() {
    let mut editor = TestEditor::lines(&LINES);
    editor.set_selection(sel(3, 7, 3, 9));
    delete_word_left(&mut editor);
    assert_eq!(editor.line(3), "    Thd Line🐶");
    assert_eq!(editor.position(), pos(3, 7));
}

#[test]
fn delete_word_left_for_cursor_at_beginning_of_document() {
    let mut editor = TestEditor::lines(&LINES);
    editor.set_position(1, 1);
    delete_word_left(&mut editor);
    assert_eq!(editor.line(1), "    \tMy First Line\t ");
    assert_eq!(editor.position(), pos(1, 1));
}

#[test]
fn delete_word_left_for_cursor_at_end_of_whitespace() {
    let mut editor = TestEditor::lines(&LINES);
    editor.set_position(3, 11);
    delete_word_left(&mut editor);
    assert_eq!(editor.line(3), "    Line🐶");
    assert_eq!(editor.position(), pos(3, 5));
}

#[test]
fn delete_word_left_for_cursor_just_behind_a_word() {
    let mut editor = TestEditor::lines(&LINES);
    editor.set_position(2, 11);
    delete_word_left(&mut editor);
    assert_eq!(editor.line(2), "\tMy  Line");
    assert_eq!(editor.position(), pos(2, 5));
}

#[test]
fn delete_word_left_for_cursor_inside_of_a_word() {
    let mut editor = TestEditor::lines(&LINES);
    editor.set_position(1, 12);
    delete_word_left(&mut editor);
    assert_eq!(editor.line(1), "    \tMy st Line\t ");
    assert_eq!(editor.position(), pos(1, 9));
}

#[test]
fn delete_word_right_for_non_empty_selection() {
    let mut editor = TestEditor::lines(&LINES);
    editor.set_selection(sel(3, 7, 3, 9));
    delete_word_right(&mut editor);
    assert_eq!(editor.line(3), "    Thd Line🐶");
    assert_eq!(editor.position(), pos(3, 7));
}

#[test]
fn delete_word_right_for_cursor_at_end_of_document() {
    let mut editor = TestEditor::lines(&LINES);
    editor.set_position(5, 3);
    delete_word_right(&mut editor);
    assert_eq!(editor.line(5), "1");
    assert_eq!(editor.position(), pos(5, 2));
}

#[test]
fn delete_word_right_for_cursor_at_beggining_of_whitespace() {
    let mut editor = TestEditor::lines(&LINES);
    editor.set_position(3, 1);
    delete_word_right(&mut editor);
    assert_eq!(editor.line(3), "Third Line🐶");
    assert_eq!(editor.position(), pos(3, 1));
}

#[test]
fn delete_word_right_for_cursor_just_before_a_word() {
    let mut editor = TestEditor::lines(&LINES);
    editor.set_position(2, 5);
    delete_word_right(&mut editor);
    assert_eq!(editor.line(2), "\tMy  Line");
    assert_eq!(editor.position(), pos(2, 5));
}

#[test]
fn delete_word_right_for_cursor_inside_of_a_word() {
    let mut editor = TestEditor::lines(&LINES);
    editor.set_position(1, 11);
    delete_word_right(&mut editor);
    assert_eq!(editor.line(1), "    \tMy Fi Line\t ");
    assert_eq!(editor.position(), pos(1, 11));
}

#[test]
fn delete_word_left_issue_832() {
    let expected = "|   |/* |Just |some |text |a|+= |3 |+|5 |*/|  ";
    assert_delete_left_stops(
        expected,
        pos(1000, 10000),
        CursorConfig::default(),
        delete_word_left,
    );
}

#[test]
fn delete_word_start_left_() {
    let expected = "|   |/* |Just |some |text |a|+= |3 |+|5 |*/  ";
    assert_delete_left_stops(
        expected,
        pos(1000, 10000),
        CursorConfig::default(),
        delete_word_start_left,
    );
}

#[test]
fn delete_word_end_left_() {
    let expected = "|   /*| Just| some| text| a|+=| 3| +|5| */|  ";
    assert_delete_left_stops(
        expected,
        pos(1000, 10000),
        CursorConfig::default(),
        delete_word_end_left,
    );
}

#[test]
fn delete_word_left_issue_24947() {
    for action in [
        delete_word_left,
        delete_word_start_left,
        delete_word_end_left,
    ] {
        let mut editor = TestEditor::lines(&["{", "}"]);
        editor.set_position(2, 1);
        action(&mut editor);
        assert_eq!(editor.line(1), "{}");
    }
}

#[test]
fn delete_word_right_issue_832() {
    let expected = "   |/*| |Just| |some| |text| |a|+=| |3| |+|5|-|3| |*/|  |";
    assert_delete_right_stops(expected, CursorConfig::default(), delete_word_right);
}

#[test]
fn delete_word_right_issue_3882() {
    for action in [
        delete_word_right,
        delete_word_start_right,
        delete_word_end_right,
    ] {
        let mut editor =
            TestEditor::lines(&["public void Add( int x,", "                 int y )"]);
        editor.set_position(1, 24);
        action(&mut editor);
        assert_eq!(editor.line(1), "public void Add( int x,int y )", "001");
    }
}

#[test]
fn delete_word_start_right_() {
    let expected = "   |/* |Just |some |text |a|+= |3 |+|5|-|3 |*/  |";
    assert_delete_right_stops(expected, CursorConfig::default(), delete_word_start_right);
}

#[test]
fn delete_word_end_right_() {
    let expected = "   /*| Just| some| text| a|+=| 3| +|5|-|3| */|  |";
    assert_delete_right_stops(expected, CursorConfig::default(), delete_word_end_right);
}

#[test]
fn delete_word_right_issue_3882_1_ctrl_delete_at_end_of_line() {
    let mut editor = TestEditor::lines(&["A line with text.", "   And another one"]);
    editor.set_position(1, 18);
    delete_word_right(&mut editor);
    assert_eq!(editor.line(1), "A line with text.And another one", "001");
}

#[test]
fn delete_word_left_issue_3882_2_ctrl_delete_at_end_of_line() {
    let mut editor = TestEditor::lines(&["A line with text.", "   And another one"]);
    editor.set_position(2, 1);
    delete_word_left(&mut editor);
    assert_eq!(editor.line(1), "A line with text.   And another one", "001");
}

#[test]
fn delete_inside_word_empty_line() {
    let mut editor = TestEditor::lines(&["Line1", "", "Line2"]);
    editor.set_position(2, 1);
    delete_inside_word_(&mut editor, false);
    assert_eq!(editor.value(), "Line1\nLine2");
}

#[test]
fn delete_inside_word_in_whitespace_1() {
    let mut editor = TestEditor::lines(&["Just  some text."]);
    editor.set_position(1, 6);
    delete_inside_word_(&mut editor, false);
    assert_eq!(editor.value(), "Justsome text.");
}

#[test]
fn delete_inside_word_in_whitespace_2() {
    let mut editor = TestEditor::lines(&["Just     some text."]);
    editor.set_position(1, 6);
    delete_inside_word_(&mut editor, false);
    assert_eq!(editor.value(), "Justsome text.");
}

#[track_caller]
fn assert_delete_inside_word_sequence(text: &str, column: u32, expected: &[&str]) {
    let mut editor = TestEditor::new(text);
    editor.set_position(1, column);
    for value in expected {
        delete_inside_word_(&mut editor, false);
        assert_eq!(editor.value(), *value);
    }
}

#[test]
fn delete_inside_word_in_whitespace_3() {
    assert_delete_inside_word_sequence(
        "Just     \"some text.",
        6,
        &[
            "Just\"some text.",
            "\"some text.",
            "some text.",
            "text.",
            ".",
            "",
            "",
        ],
    );
}

#[test]
fn delete_inside_word_in_non_words() {
    assert_delete_inside_word_sequence(
        "x=3+4+5+6",
        7,
        &["x=3+45+6", "x=3++6", "x=36", "x=", "x", "", ""],
    );
}

#[test]
fn delete_inside_word_in_words_1() {
    assert_delete_inside_word_sequence(
        "This is interesting",
        7,
        &["This interesting", "This", "", ""],
    );
}

#[test]
fn delete_inside_word_in_words_2() {
    assert_delete_inside_word_sequence(
        "This  is  interesting",
        7,
        &["This  interesting", "This", "", ""],
    );
}

#[test]
fn delete_inside_word_only_word_does_not_delete_whitespace_before_last_word() {
    let mut editor = TestEditor::new("hello world");
    editor.set_position(1, 9);
    delete_inside_word_(&mut editor, true);
    assert_eq!(editor.value(), "hello ");
}

#[test]
fn delete_inside_word_only_word_deletes_just_the_word() {
    let mut editor = TestEditor::new("This is interesting");
    editor.set_position(1, 7);
    delete_inside_word_(&mut editor, true);
    assert_eq!(editor.value(), "This  interesting");
}

// Not in Monaco: `wordSegmenterLocales` with the other word operations.

#[test]
#[cfg(feature = "word-segmenter")]
fn delete_word_left_recognize_words() {
    let expected = "|これ|は|テスト|です";
    assert_delete_left_stops(
        expected,
        pos(1000, 1000),
        segmenter_config("ja"),
        delete_word_left,
    );
}

#[test]
#[cfg(feature = "word-segmenter")]
fn delete_word_right_recognize_words() {
    let expected = "これ|は|テスト|です|";
    assert_delete_right_stops(expected, segmenter_config("ja"), delete_word_right);
}

#[test]
#[cfg(feature = "word-segmenter")]
fn delete_inside_word_recognize_words() {
    let mut editor = TestEditor::with_config("これはテストです", segmenter_config("ja"));
    editor.set_position(1, 5);
    delete_inside_word_(&mut editor, false);
    assert_eq!(editor.value(), "これはです");
}

#[test]
#[cfg(feature = "word-segmenter")]
fn cursor_word_accessibility_right_recognize_words() {
    // Accessibility navigation uses the default separators but keeps the
    // locales.
    let config = CursorConfig {
        word_separators: String::new(),
        ..segmenter_config("ja")
    };
    let expected = "/* |これ|は|テスト|です /*|";
    assert_stops_with(
        expected,
        pos(1, 1),
        config,
        |ed| cursor_word_accessibility_right(ed, false),
        |ed| ed.position() == pos(1, 15),
    );
}

#[test]
#[cfg(feature = "word-segmenter")]
fn cursor_word_left_recognize_chinese_and_thai_words() {
    assert_stops_with(
        "|我们|是|学生",
        pos(1000, 1000),
        segmenter_config("zh-CN"),
        |ed| cursor_word_left(ed, false),
        |ed| ed.position() == pos(1, 1),
    );
    assert_stops_with(
        "|ทุก|สอง|สัปดาห์",
        pos(1000, 1000),
        segmenter_config("th"),
        |ed| cursor_word_left(ed, false),
        |ed| ed.position() == pos(1, 1),
    );
}

#[test]
#[cfg(feature = "word-segmenter")]
fn word_at_recognize_words() {
    let editor = TestEditor::with_config("/* これはテストです */", segmenter_config("ja"));
    let word_at = |column| {
        crate::cursor::word_at(&editor.config, &editor.buffer, pos(1, column))
            .map(|r| (r.start.column + 1, r.end.column + 1))
    };
    assert_eq!(word_at(4), Some((4, 6)));
    assert_eq!(word_at(7), Some((6, 7)));
    assert_eq!(word_at(9), Some((7, 10)));
    assert_eq!(word_at(2), Some((1, 3)));
}

#[test]
#[cfg(not(feature = "word-segmenter"))]
fn word_segmenter_locales_have_no_effect_without_the_feature() {
    let expected = "|/* |これはテストです |/*";
    assert_stops_with(
        expected,
        pos(1000, 1000),
        segmenter_config("ja"),
        |ed| cursor_word_left(ed, true),
        |ed| ed.position() == pos(1, 1),
    );
}
