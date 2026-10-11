//! Port of Monaco's `src/vs/editor/test/browser/controller/cursor.test.ts`
//! (MIT), suites `Editor Controller` and `Undo stops`.
//!
//! Skipped (by reason):
//! - Language configuration (auto-closing pairs and overtyping, surround
//!   selection, onEnter rules, indentation rules, electric characters,
//!   tokenization): ported in `cursor_language_test.rs`, which lists the
//!   ones still skipped there (IME composition, reentrant listeners,
//!   snippets).
//! - Word wrap / view model: `issue #36740`, `issue #110376`, `issue #98320`,
//!   `issue #41573`, `issue #112301`, `move right normalizes the cursor to
//!   the next wrapped line`.
//! - Text model features: `issue #23539` and `can undo typing and EOL change
//!   in one undo stop` (EOL changes), `issue #23983` x2 (`setEOL`/`setValue`),
//!   `issue #90973` (version ids), `issue #46208` (undo of an edit pushed
//!   with empty cursor state), `issue #44805` (read-only editor).
//! - Editor API / multiple editors / events: `issue #3071` (re-entrant edit
//!   from a content listener), `issue #46314`, `issue #15761`, `issue
//!   #42783` (`executeEdits`), `issue #111513` (two editors on one model).
//! - `issue #37967`: needs `multiCursorMergeOverlapping: false`, which is
//!   not configurable (cursors always merge).
//! - `Overtype Mode` suite: overtype (insert key) mode is not ported.

use super::harness::{TestEditor, sel};
use crate::cursor::{CursorConfig, composition_type};

fn config(f: impl FnOnce(&mut CursorConfig)) -> CursorConfig {
    let mut config = CursorConfig::default();
    f(&mut config);
    config
}

#[test]
fn issue_monaco_editor_443_indentation_of_a_single_row_deletes_selected_text() {
    let mut editor = TestEditor::lines_with_config(
        &["Hello world!", "another line"],
        config(|c| c.insert_spaces = false),
    );
    editor.set_selections(&[sel(1, 1, 1, 13)]);
    // Check that indenting maintains the selection start at column 1
    editor.tab();
    editor.assert_selection(sel(1, 1, 1, 14));
}

#[test]
fn bug_9121_auto_indent_undo_redo_is_funky() {
    let mut editor = TestEditor::with_config("", config(|c| c.insert_spaces = false))
        .with_trim_auto_whitespace(false);
    editor.type_keyboard("\n");
    assert_eq!(editor.value(), "\n", "assert1");

    editor.tab();
    assert_eq!(editor.value(), "\n\t", "assert2");

    editor.type_keyboard("\n");
    assert_eq!(editor.value(), "\n\t\n\t", "assert3");

    editor.type_raw("x");
    assert_eq!(editor.value(), "\n\t\n\tx", "assert4");

    editor.move_left(false);
    assert_eq!(editor.value(), "\n\t\n\tx", "assert5");

    editor.delete_left();
    assert_eq!(editor.value(), "\n\t\nx", "assert6");

    editor.delete_left();
    assert_eq!(editor.value(), "\n\tx", "assert7");

    editor.delete_left();
    assert_eq!(editor.value(), "\nx", "assert8");

    editor.delete_left();
    assert_eq!(editor.value(), "x", "assert9");

    editor.undo();
    assert_eq!(editor.value(), "\nx", "assert10");

    editor.undo();
    assert_eq!(editor.value(), "\n\t\nx", "assert11");

    editor.undo();
    assert_eq!(editor.value(), "\n\t\n\tx", "assert12");

    editor.redo();
    assert_eq!(editor.value(), "\n\t\nx", "assert13");

    editor.redo();
    assert_eq!(editor.value(), "\nx", "assert14");

    editor.redo();
    assert_eq!(editor.value(), "x", "assert15");
}

#[test]
fn bug_16815_shift_tab_doesnt_go_back_to_tabstop() {
    let mut editor = TestEditor::lines(&["     function baz() {"]);
    editor.move_to(1, 6, false);
    editor.assert_selection(sel(1, 6, 1, 6));

    editor.outdent();
    assert_eq!(editor.line(1), "    function baz() {");
    editor.assert_selection(sel(1, 5, 1, 5));
}

#[test]
fn bug_18293_cant_outdent_whitespace_line() {
    let mut editor = TestEditor::lines(&["      "]);
    editor.move_to(1, 7, false);
    editor.assert_selection(sel(1, 7, 1, 7));

    editor.outdent();
    assert_eq!(editor.line(1), "    ");
    editor.assert_selection(sel(1, 5, 1, 5));
}

#[test]
fn issue_95591_unindenting_moves_cursor_to_beginning_of_line() {
    let mut editor =
        TestEditor::lines_with_config(&["        "], config(|c| c.use_tab_stops = false));
    editor.move_to(1, 9, false);
    editor.assert_selection(sel(1, 9, 1, 9));

    editor.outdent();
    assert_eq!(editor.line(1), "    ");
    editor.assert_selection(sel(1, 5, 1, 5));
}

#[test]
fn bug_16657_tab_on_empty_line_of_zero_indentation_moves_cursor_to_1_1() {
    let mut editor = TestEditor::lines_with_config(
        &[
            "function baz() {",
            "\tfunction hello() { // something here",
            "\t",
            "",
            "\t}",
            "}",
            "",
        ],
        config(|c| c.insert_spaces = false),
    );
    editor.move_to(7, 1, false);
    editor.assert_selection(sel(7, 1, 7, 1));

    editor.tab();
    assert_eq!(editor.line(7), "\t");
    editor.assert_selection(sel(7, 2, 7, 2));
}

#[test]
fn bug_16740_cut_line_doesnt_quite_cut_the_last_line() {
    // Part 1 => there is text on the last line
    let mut editor = TestEditor::lines(&["asdasd", "qwerty"]);
    editor.move_to(2, 1, false);
    editor.assert_selection(sel(2, 1, 2, 1));
    editor.cut();
    assert_eq!(editor.line_count(), 1);
    assert_eq!(editor.line(1), "asdasd");

    // Part 2 => there is no text on the last line
    let mut editor = TestEditor::lines(&["asdasd", ""]);
    editor.move_to(2, 1, false);
    editor.assert_selection(sel(2, 1, 2, 1));
    editor.cut();
    assert_eq!(editor.line_count(), 1);
    assert_eq!(editor.line(1), "asdasd");

    editor.cut();
    assert_eq!(editor.line_count(), 1);
    assert_eq!(editor.line(1), "");
}

#[test]
fn issue_128602_when_cutting_multiple_lines_the_last_line_will_not_be_erased() {
    let mut editor = TestEditor::lines(&["a1", "a2", "a3"]);
    editor.set_selections(&[sel(1, 1, 1, 1), sel(2, 1, 2, 1), sel(3, 1, 3, 1)]);
    editor.cut();
    assert_eq!(editor.line_count(), 1);
    assert_eq!(editor.line(1), "");
}

#[test]
fn issue_1140_backspace_stops_prematurely() {
    let mut editor = TestEditor::lines(&["function baz() {", "  return 1;", "};"]);
    editor.move_to(3, 2, false);
    editor.move_to(1, 14, true);
    editor.assert_selection(sel(3, 2, 1, 14));

    editor.delete_left();
    editor.assert_selection(sel(1, 14, 1, 14));
    assert_eq!(editor.line_count(), 1);
    assert_eq!(editor.line(1), "function baz(;");
}

#[test]
fn issue_10212_pasting_entire_line_does_not_replace_selection() {
    let mut editor = TestEditor::lines(&["line1", "line2"]);
    editor.move_to(2, 1, false);
    editor.move_to(2, 6, true);

    editor.paste("line1\n", true, None);

    assert_eq!(editor.line(1), "line1");
    assert_eq!(editor.line(2), "line1");
    assert_eq!(editor.line(3), "");
}

#[test]
fn issue_74722_pasting_whole_line_does_not_replace_selection() {
    let mut editor = TestEditor::lines(&["line1", "line sel 2", "line3"]);
    editor.set_selections(&[sel(2, 6, 2, 9)]);

    editor.paste("line1\n", true, None);

    assert_eq!(editor.line(1), "line1");
    assert_eq!(editor.line(2), "line line1");
    assert_eq!(editor.line(3), " 2");
    assert_eq!(editor.line(4), "line3");
}

#[test]
fn issue_4996_multiple_cursor_paste_pastes_contents_of_all_cursors() {
    let mut editor = TestEditor::lines(&["line1", "line2", "line3"]);
    editor.set_selections(&[sel(1, 1, 1, 1), sel(2, 1, 2, 1)]);

    editor.paste(
        "a\nb\nc\nd",
        false,
        Some(&["a\nb".to_string(), "c\nd".to_string()]),
    );

    assert_eq!(
        editor.value(),
        ["a", "bline1", "c", "dline2", "line3"].join("\n")
    );
}

#[test]
fn issue_16155_paste_into_multiple_cursors_edge_case() {
    let mut editor = TestEditor::lines(&["test", "test", "test", "test"]);
    editor.set_selections(&[
        sel(1, 1, 1, 5),
        sel(2, 1, 2, 5),
        sel(3, 1, 3, 5),
        sel(4, 1, 4, 5),
    ]);

    editor.paste("aaa\nbbb\nccc\n", false, None);

    assert_eq!(
        editor.value(),
        [
            "aaa", "bbb", "ccc", "", "aaa", "bbb", "ccc", "", "aaa", "bbb", "ccc", "", "aaa",
            "bbb", "ccc", "",
        ]
        .join("\n")
    );
}

#[test]
fn issue_43722_multiline_paste_doesnt_work_anymore() {
    let mut editor = TestEditor::lines(&["test", "test", "test", "test"]);
    editor.set_selections(&[
        sel(1, 1, 1, 5),
        sel(2, 1, 2, 5),
        sel(3, 1, 3, 5),
        sel(4, 1, 4, 5),
    ]);

    editor.paste("aaa\r\nbbb\r\nccc\r\nddd\r\n", false, None);

    assert_eq!(editor.value(), ["aaa", "bbb", "ccc", "ddd"].join("\n"));
}

#[test]
fn issue_46440_1_pasting_a_multi_line_selection_into_every_insertion_point() {
    let mut editor = TestEditor::lines(&["line1", "line2", "line3"]);
    editor.set_selections(&[sel(1, 1, 1, 1), sel(2, 1, 2, 1), sel(3, 1, 3, 1)]);

    editor.paste("a\nb\nc", false, None);

    assert_eq!(editor.value(), ["aline1", "bline2", "cline3"].join("\n"));
}

#[test]
fn issue_46440_2_pasting_a_multi_line_selection_into_every_insertion_point() {
    let mut editor = TestEditor::lines(&["line1", "line2", "line3"]);
    editor.set_selections(&[sel(1, 1, 1, 1), sel(2, 1, 2, 1), sel(3, 1, 3, 1)]);

    editor.paste("a\nb\nc\n", false, None);

    assert_eq!(editor.value(), ["aline1", "bline2", "cline3"].join("\n"));
}

#[test]
fn issue_256039_paste_from_multiple_cursors_with_empty_selections_and_multi_cursor_paste_full() {
    let mut editor = TestEditor::lines_with_config(
        &["line1", "line2", "line3"],
        config(|c| c.multi_cursor_paste = crate::cursor::MultiCursorPaste::Full),
    );
    // 2 cursors on lines 1 and 2
    editor.set_selections(&[sel(1, 1, 1, 1), sel(2, 1, 2, 1)]);

    editor.paste(
        "line1\nline2\n",
        true,
        Some(&["line1\n".to_string(), "line2\n".to_string()]),
    );

    // Each cursor gets its respective line
    assert_eq!(
        editor.value(),
        ["line1", "line1", "line2", "line2", "line3"].join("\n")
    );
}

#[test]
fn issue_12950_cannot_double_click_to_insert_emoji_using_osx_emoji_panel() {
    let mut editor = TestEditor::lines(&["some lines", "and more lines", "just some text"]);
    editor.move_to(3, 1, false);

    editor.type_keyboard("\u{1f60d}");

    assert_eq!(
        editor.value(),
        ["some lines", "and more lines", "\u{1f60d}just some text"].join("\n")
    );
}

#[test]
fn issue_3463_pressing_tab_adds_spaces_but_not_as_many_as_for_a_tab() {
    let mut editor = TestEditor::lines(&["function a() {", "\tvar a = {", "\t\tx: 3", "\t};", "}"]);
    editor.move_to(3, 2, false);
    editor.tab();
    assert_eq!(editor.line(3), "\t    \tx: 3");
}

#[test]
fn issue_4312_trying_to_type_a_tab_character_over_a_sequence_of_spaces() {
    let mut editor = TestEditor::lines_with_config(
        &[
            "var foo = 123;       // this is a comment",
            "var bar = 4;       // another comment",
        ],
        config(|c| c.insert_spaces = false),
    );
    editor.move_to(1, 15, false);
    editor.move_to(1, 22, true);
    editor.tab();
    assert_eq!(editor.line(1), "var foo = 123;\t// this is a comment");
}

#[test]
fn issue_832_word_right() {
    let mut editor = TestEditor::lines(&["   /* Just some   more   text a+= 3 +5-3 + 7 */  "]);
    editor.move_to(1, 1, false);

    let mut assert_word_right = |col: u32, expected: &str| {
        // WordSelect at column 1, then WordSelectDrag.
        editor.word_select(1, col, col != 1);
        let selection = editor.selections()[0];
        assert_eq!(selection.start().column, 0, "TEST FOR {col}");
        assert_eq!(
            selection.end().column,
            expected.encode_utf16().count() as u32,
            "TEST FOR {col}"
        );
    };

    assert_word_right(1, "   ");
    assert_word_right(2, "   ");
    assert_word_right(3, "   ");
    assert_word_right(4, "   ");
    assert_word_right(5, "   /");
    assert_word_right(6, "   /*");
    assert_word_right(7, "   /* ");
    assert_word_right(8, "   /* Just");
    assert_word_right(9, "   /* Just");
    assert_word_right(10, "   /* Just");
    assert_word_right(11, "   /* Just");
    assert_word_right(12, "   /* Just ");
    assert_word_right(13, "   /* Just some");
    assert_word_right(14, "   /* Just some");
    assert_word_right(15, "   /* Just some");
    assert_word_right(16, "   /* Just some");
    assert_word_right(17, "   /* Just some ");
    assert_word_right(18, "   /* Just some  ");
    assert_word_right(19, "   /* Just some   ");
    assert_word_right(20, "   /* Just some   more");
    assert_word_right(21, "   /* Just some   more");
    assert_word_right(22, "   /* Just some   more");
    assert_word_right(23, "   /* Just some   more");
    assert_word_right(24, "   /* Just some   more ");
    assert_word_right(25, "   /* Just some   more  ");
    assert_word_right(26, "   /* Just some   more   ");
    assert_word_right(27, "   /* Just some   more   text");
    assert_word_right(28, "   /* Just some   more   text");
    assert_word_right(29, "   /* Just some   more   text");
    assert_word_right(30, "   /* Just some   more   text");
    assert_word_right(31, "   /* Just some   more   text ");
    assert_word_right(32, "   /* Just some   more   text a");
    assert_word_right(33, "   /* Just some   more   text a+");
    assert_word_right(34, "   /* Just some   more   text a+=");
    assert_word_right(35, "   /* Just some   more   text a+= ");
    assert_word_right(36, "   /* Just some   more   text a+= 3");
    assert_word_right(37, "   /* Just some   more   text a+= 3 ");
    assert_word_right(38, "   /* Just some   more   text a+= 3 +");
    assert_word_right(39, "   /* Just some   more   text a+= 3 +5");
    assert_word_right(40, "   /* Just some   more   text a+= 3 +5-");
    assert_word_right(41, "   /* Just some   more   text a+= 3 +5-3");
    assert_word_right(42, "   /* Just some   more   text a+= 3 +5-3 ");
    assert_word_right(43, "   /* Just some   more   text a+= 3 +5-3 +");
    assert_word_right(44, "   /* Just some   more   text a+= 3 +5-3 + ");
    assert_word_right(45, "   /* Just some   more   text a+= 3 +5-3 + 7");
    assert_word_right(46, "   /* Just some   more   text a+= 3 +5-3 + 7 ");
    assert_word_right(47, "   /* Just some   more   text a+= 3 +5-3 + 7 *");
    assert_word_right(48, "   /* Just some   more   text a+= 3 +5-3 + 7 */");
    assert_word_right(49, "   /* Just some   more   text a+= 3 +5-3 + 7 */ ");
    assert_word_right(50, "   /* Just some   more   text a+= 3 +5-3 + 7 */  ");
}

#[test]
fn issue_33788_wrong_cursor_position_when_double_click_to_select_a_word() {
    let mut editor = TestEditor::lines(&["Just some text"]);
    editor.word_select(1, 8, false);
    editor.assert_selection(sel(1, 6, 1, 10));

    editor.word_select(1, 8, true);
    editor.assert_selection(sel(1, 6, 1, 10));
}

#[test]
fn issue_12887_double_click_highlighting_separating_white_space() {
    let mut editor = TestEditor::lines(&["abc def"]);
    editor.word_select(1, 5, false);
    editor.assert_selection(sel(1, 5, 1, 8));
}

#[test]
fn double_click_on_punctuation_should_select_the_character_not_adjacent_space() {
    let mut editor = TestEditor::lines(&["// a b c 1 2 3 ~ ! @ # $ % ^ & * ( ) _ + \\ /"]);
    // Test double-click on '@' at position 20
    editor.word_select(1, 20, false);
    editor.assert_selection(sel(1, 20, 1, 21));
    // Test double-click on '#' at position 22
    editor.word_select(1, 22, false);
    editor.assert_selection(sel(1, 22, 1, 23));
    // Test double-click on '!' at position 18
    editor.word_select(1, 18, false);
    editor.assert_selection(sel(1, 18, 1, 19));
    // Test double-click on first '/' in '//' at position 1
    editor.word_select(1, 1, false);
    editor.assert_selection(sel(1, 1, 1, 3));
    // Test double-click on second '/' in '//' at position 2
    editor.word_select(1, 2, false);
    editor.assert_selection(sel(1, 1, 1, 3));
    // Test double-click on '\' at position 42
    editor.word_select(1, 42, false);
    editor.assert_selection(sel(1, 42, 1, 43));
}

#[test]
fn issue_9675_undo_redo_adds_a_stop_in_between_chn_characters() {
    let mut editor = TestEditor::new("");
    editor.assert_cursor(1, 1);

    let composition = |editor: &mut TestEditor, text: &str, replace_prev: u32| {
        let buffer = &editor.buffer;
        let op = composition_type(
            buffer,
            &editor.cursors,
            text,
            replace_prev,
            0,
            0,
            editor.prev,
        );
        editor.apply(op);
    };

    // Typing sennsei in Japanese - Hiragana
    editor.type_keyboard("\u{ff53}");
    composition(&mut editor, "\u{305b}", 1);
    composition(&mut editor, "\u{305b}\u{ff4e}", 1);
    composition(&mut editor, "\u{305b}\u{3093}", 2);
    composition(&mut editor, "\u{305b}\u{3093}\u{ff53}", 2);
    composition(&mut editor, "\u{305b}\u{3093}\u{305b}", 3);
    composition(&mut editor, "\u{305b}\u{3093}\u{305b}", 3);
    composition(&mut editor, "\u{305b}\u{3093}\u{305b}\u{3044}", 3);
    composition(&mut editor, "\u{305b}\u{3093}\u{305b}\u{3044}", 4);
    composition(&mut editor, "\u{305b}\u{3093}\u{305b}\u{3044}", 4);
    composition(&mut editor, "\u{305b}\u{3093}\u{305b}\u{3044}", 4);

    assert_eq!(editor.line(1), "\u{305b}\u{3093}\u{305b}\u{3044}");
    editor.assert_cursor(1, 5);

    editor.undo();
    assert_eq!(editor.line(1), "");
    editor.assert_cursor(1, 1);
}

#[test]
fn issue_22717_moving_text_cursor_cause_an_incorrect_position_in_chinese() {
    let mut editor = TestEditor::lines(&[
        "\u{4e00}\u{4e8c}\u{4e09}\u{56db}\u{4e94}\u{516d}\u{4e03}\u{516b}\u{4e5d}\u{5341}",
        "12345678901234567890",
    ]);
    editor.set_selections(&[sel(1, 5, 1, 5)]);

    editor.move_down(false);
    editor.assert_selection(sel(2, 9, 2, 9));

    editor.move_right(false);
    editor.assert_selection(sel(2, 10, 2, 10));

    editor.move_right(false);
    editor.assert_selection(sel(2, 11, 2, 11));

    editor.move_up(false);
    editor.assert_selection(sel(1, 6, 1, 6));
}

#[test]
fn issue_85712_paste_line_moves_cursor_to_start_of_current_line_rather_than_next_line() {
    let mut editor = TestEditor::lines(&["abc123", ""]);
    editor.set_selections(&[sel(2, 1, 2, 1)]);
    editor.paste("something\n", true, None);
    assert_eq!(editor.value(), ["abc123", "something", ""].join("\n"));
    editor.assert_cursor(3, 1);
}

const THAI: &str = "\u{e2a}\u{e27}\u{e31}\u{e2a}\u{e14}\u{e35}";

#[track_caller]
fn assert_thai_left_delete(config: CursorConfig) {
    let mut editor = TestEditor::lines_with_config(&[THAI], config);
    editor.set_selections(&[sel(1, 7, 1, 7)]);
    for expected in [
        "\u{e2a}\u{e27}\u{e31}\u{e2a}\u{e14}",
        "\u{e2a}\u{e27}\u{e31}\u{e2a}",
        "\u{e2a}\u{e27}\u{e31}",
        "\u{e2a}\u{e27}",
        "\u{e2a}",
        "",
    ] {
        editor.delete_left();
        assert_eq!(editor.value(), expected);
    }
}

#[test]
fn issue_84897_left_delete_behavior_in_some_languages_is_changed() {
    assert_thai_left_delete(CursorConfig::default());
}

#[test]
fn issue_122914_left_delete_behavior_in_some_languages_is_changed_use_tab_stops_false() {
    assert_thai_left_delete(config(|c| c.use_tab_stops = false));
}

#[test]
fn issue_99629_emoji_modifiers_in_text_treated_separately_when_using_backspace() {
    let text = "\u{1f476}\u{1f3fe}";
    let mut editor = TestEditor::lines_with_config(&[text], config(|c| c.use_tab_stops = false));
    let len = text.encode_utf16().count() as u32;
    editor.set_selections(&[sel(1, 1 + len, 1, 1 + len)]);

    editor.delete_left();
    assert_eq!(editor.value(), "");
}

#[test]
fn issue_99629_emoji_modifiers_in_text_treated_separately_when_using_backspace_zwj_sequence() {
    let text = "\u{1f468}\u{200d}\u{1f469}\u{1f3fd}\u{200d}\u{1f467}\u{200d}\u{1f466}";
    let mut editor = TestEditor::lines_with_config(&[text], config(|c| c.use_tab_stops = false));
    let len = text.encode_utf16().count() as u32;
    editor.set_selections(&[sel(1, 1 + len, 1, 1 + len)]);

    editor.delete_left();
    assert_eq!(
        editor.value(),
        "\u{1f468}\u{200d}\u{1f469}\u{1f3fd}\u{200d}\u{1f467}"
    );

    editor.delete_left();
    assert_eq!(editor.value(), "\u{1f468}\u{200d}\u{1f469}\u{1f3fd}");

    editor.delete_left();
    assert_eq!(editor.value(), "\u{1f468}");

    editor.delete_left();
    assert_eq!(editor.value(), "");
}

/// Monaco runs this with word wrap at column 24; the expectations are the
/// same without wrapping.
#[test]
fn issue_105730_move_left_behaves_differently_for_multiple_cursors() {
    let mut editor = TestEditor::new("asdfghjkl, asdfghjkl, asdfghjkl, ");
    let selections = [sel(1, 10, 1, 12), sel(1, 21, 1, 23), sel(1, 32, 1, 34)];
    editor.set_selections(&selections);
    editor.move_left(false);
    editor.assert_selections(&[sel(1, 10, 1, 10), sel(1, 21, 1, 21), sel(1, 32, 1, 32)]);

    editor.set_selections(&selections);
    editor.move_left(true);
    editor.assert_selections(&[sel(1, 10, 1, 11), sel(1, 21, 1, 22), sel(1, 32, 1, 33)]);
}

/// Monaco runs this with word wrap at column 24; the expectations are the
/// same without wrapping.
#[test]
fn issue_105730_move_right_should_always_skip_wrap_point() {
    let mut editor = TestEditor::new("asdfghjkl, asdfghjkl, asdfghjkl, \nasdfghjkl,");
    editor.set_selections(&[sel(1, 22, 1, 22)]);
    editor.move_right(false);
    editor.move_right(false);
    editor.assert_selections(&[sel(1, 24, 1, 24)]);

    editor.set_selections(&[sel(1, 22, 1, 22)]);
    editor.move_right(true);
    editor.move_right(true);
    editor.assert_selections(&[sel(1, 22, 1, 24)]);
}

/// Monaco runs this with word wrap at column 8; the expectations are the
/// same without wrapping.
#[test]
fn issue_123178_sticky_tab_in_consecutive_wrapped_lines() {
    let mut editor = TestEditor::with_config(
        "    aaaa        aaaa",
        config(|c| {
            c.tab_size = 4;
            c.sticky_tab_stops = true;
        }),
    );
    editor.set_selections(&[sel(1, 9, 1, 9)]);
    editor.move_right(false);
    editor.assert_selections(&[sel(1, 10, 1, 10)]);

    editor.move_left(false);
    editor.assert_selections(&[sel(1, 9, 1, 9)]);
}

#[test]
fn cursor_honors_insert_spaces_configuration_on_new_line() {
    let mut editor = TestEditor::lines(&[
        "    \tMy First Line\t ",
        "\tMy Second Line",
        "    Third Line",
        "",
        "1",
    ]);
    editor.move_to(1, 21, false);
    editor.type_keyboard("\n");
    assert_eq!(editor.line(1), "    \tMy First Line\t ");
    assert_eq!(editor.line(2), "        ");
}

#[test]
fn cursor_honors_insert_spaces_configuration_on_tab() {
    let mut editor = TestEditor::lines_with_config(
        &[
            "    \tMy First Line\t ",
            "My Second Line123",
            "    Third Line",
            "",
            "1",
        ],
        config(|c| {
            c.tab_size = 13;
            c.indent_size = 13;
        }),
    );
    let mut check = |column: u32, expected: &str, undo: bool| {
        assert_eq!(editor.line(2), "My Second Line123");
        editor.move_to(2, column, false);
        editor.tab();
        assert_eq!(editor.line(2), expected);
        if undo {
            editor.undo();
        }
    };
    // Tab on column 1
    check(1, "             My Second Line123", true);
    // Tab on column 2
    check(2, "M            y Second Line123", true);
    // Tab on column 3
    check(3, "My            Second Line123", true);
    // Tab on column 4
    check(4, "My           Second Line123", true);
    // Tab on column 5
    check(5, "My S         econd Line123", true);
    // Tab on column 5
    check(5, "My S         econd Line123", true);
    // Tab on column 13
    check(13, "My Second Li ne123", true);
    // Tab on column 14
    check(14, "My Second Lin             e123", false);
}

#[test]
fn issue_148256_pressing_enter_creates_line_with_bad_indent_with_insert_spaces_true() {
    let mut editor = TestEditor::lines(&["  \t"]);
    editor.move_to(1, 4, false);
    editor.type_keyboard("\n");
    assert_eq!(editor.value(), "  \t\n    ");
}

#[test]
fn issue_148256_pressing_enter_creates_line_with_bad_indent_with_insert_spaces_false() {
    let mut editor = TestEditor::lines(&["  \t"]);
    editor.config.insert_spaces = false;
    editor.move_to(1, 4, false);
    editor.type_keyboard("\n");
    assert_eq!(editor.value(), "  \t\n\t");
}

#[test]
fn remove_auto_whitespace_off() {
    let mut editor = TestEditor::lines(&["    some  line abc  "]).with_trim_auto_whitespace(false);

    // Move cursor to the end, verify that we do not trim whitespaces if line has values
    let len = editor.line(1).encode_utf16().count() as u32;
    editor.move_to(1, len + 1, false);
    editor.type_keyboard("\n");
    assert_eq!(editor.line(1), "    some  line abc  ");
    assert_eq!(editor.line(2), "    ");

    // Try to enter again, we should trimmed previous line
    editor.type_keyboard("\n");
    assert_eq!(editor.line(1), "    some  line abc  ");
    assert_eq!(editor.line(2), "    ");
    assert_eq!(editor.line(3), "    ");
}

#[test]
fn remove_auto_whitespace_on_removes_only_whitespace_the_cursor_added_1() {
    let mut editor = TestEditor::lines(&["    "]);
    let len = editor.line(1).encode_utf16().count() as u32;
    editor.move_to(1, len + 1, false);
    editor.type_keyboard("\n");
    assert_eq!(editor.line(1), "    ");
    assert_eq!(editor.line(2), "    ");

    editor.type_keyboard("\n");
    assert_eq!(editor.line(1), "    ");
    assert_eq!(editor.line(2), "");
    assert_eq!(editor.line(3), "    ");
}

#[test]
fn remove_auto_whitespace_on_removes_only_whitespace_the_cursor_added_2() {
    let mut editor = TestEditor::lines(&["    if (a) {", "        ", "", "", "    }"]);

    editor.move_to(3, 1, false);
    editor.tab();
    assert_eq!(editor.line(1), "    if (a) {");
    assert_eq!(editor.line(2), "        ");
    assert_eq!(editor.line(3), "    ");
    assert_eq!(editor.line(4), "");
    assert_eq!(editor.line(5), "    }");

    editor.move_to(4, 1, false);
    editor.tab();
    assert_eq!(editor.line(1), "    if (a) {");
    assert_eq!(editor.line(2), "        ");
    assert_eq!(editor.line(3), "");
    assert_eq!(editor.line(4), "    ");
    assert_eq!(editor.line(5), "    }");

    let len = editor.line(5).encode_utf16().count() as u32;
    editor.move_to(5, len + 1, false);
    editor.type_keyboard("something");
    assert_eq!(editor.line(1), "    if (a) {");
    assert_eq!(editor.line(2), "        ");
    assert_eq!(editor.line(3), "");
    assert_eq!(editor.line(4), "");
    assert_eq!(editor.line(5), "    }something");
}

#[test]
fn remove_auto_whitespace_on_test_1() {
    let mut editor = TestEditor::lines(&["    some  line abc  "]);

    // Move cursor to the end, verify that we do not trim whitespaces if line has values
    let len = editor.line(1).encode_utf16().count() as u32;
    editor.move_to(1, len + 1, false);
    editor.type_keyboard("\n");
    assert_eq!(editor.line(1), "    some  line abc  ");
    assert_eq!(editor.line(2), "    ");

    // Try to enter again, we should trimmed previous line
    editor.type_keyboard("\n");
    assert_eq!(editor.line(1), "    some  line abc  ");
    assert_eq!(editor.line(2), "");
    assert_eq!(editor.line(3), "    ");

    // More whitespaces
    editor.tab();
    assert_eq!(editor.line(1), "    some  line abc  ");
    assert_eq!(editor.line(2), "");
    assert_eq!(editor.line(3), "        ");

    // Enter and verify that trimmed again
    editor.type_keyboard("\n");
    assert_eq!(editor.line(1), "    some  line abc  ");
    assert_eq!(editor.line(2), "");
    assert_eq!(editor.line(3), "");
    assert_eq!(editor.line(4), "        ");

    // Trimmed if we will keep only text
    editor.move_to(1, 5, false);
    editor.type_keyboard("\n");
    assert_eq!(editor.line(1), "    ");
    assert_eq!(editor.line(2), "    some  line abc  ");
    assert_eq!(editor.line(3), "");
    assert_eq!(editor.line(4), "");
    assert_eq!(editor.line(5), "");

    // Trimmed if we will keep only text by selection
    editor.move_to(2, 5, false);
    editor.move_to(3, 1, true);
    editor.type_keyboard("\n");
    assert_eq!(editor.line(1), "    ");
    assert_eq!(editor.line(2), "    ");
    assert_eq!(editor.line(3), "    ");
    assert_eq!(editor.line(4), "");
    assert_eq!(editor.line(5), "");
}

#[test]
fn issue_15118_remove_auto_whitespace_when_pasting_entire_line() {
    let mut editor = TestEditor::lines(&[
        "    function f() {",
        "        // I'm gonna copy this line",
        "        return 3;",
        "    }",
    ]);

    let len = editor.line(3).encode_utf16().count() as u32;
    editor.move_to(3, len + 1, false);
    editor.type_keyboard("\n");

    assert_eq!(
        editor.value(),
        [
            "    function f() {",
            "        // I'm gonna copy this line",
            "        return 3;",
            "        ",
            "    }",
        ]
        .join("\n")
    );
    let len = editor.line(4).encode_utf16().count() as u32;
    editor.assert_cursor(4, len + 1);

    editor.paste("        // I'm gonna copy this line\n", true, None);
    assert_eq!(
        editor.value(),
        [
            "    function f() {",
            "        // I'm gonna copy this line",
            "        return 3;",
            "        // I'm gonna copy this line",
            "",
            "    }",
        ]
        .join("\n")
    );
    editor.assert_cursor(5, 1);
}

#[test]
fn issue_40695_maintain_cursor_position_when_copying_lines_using_ctrl_c_ctrl_v() {
    let mut editor = TestEditor::lines(&[
        "    function f() {",
        "        // I'm gonna copy this line",
        "        // Another line",
        "        return 3;",
        "    }",
    ]);
    editor.set_selections(&[sel(4, 10, 4, 10)]);
    editor.paste("        // I'm gonna copy this line\n", true, None);

    assert_eq!(
        editor.value(),
        [
            "    function f() {",
            "        // I'm gonna copy this line",
            "        // Another line",
            "        // I'm gonna copy this line",
            "        return 3;",
            "    }",
        ]
        .join("\n")
    );
    editor.assert_cursor(5, 10);
}

#[test]
fn use_tab_stops_is_off() {
    let mut editor = TestEditor::lines_with_config(
        &["    x", "        a    ", "    "],
        config(|c| c.use_tab_stops = false),
    );
    // DeleteLeft removes just one whitespace
    editor.move_to(2, 9, false);
    editor.delete_left();
    assert_eq!(editor.line(2), "       a    ");
}

#[test]
fn backspace_removes_whitespaces_with_tab_size() {
    let mut editor = TestEditor::lines_with_config(
        &[" \t \t     x", "        a    ", "    "],
        config(|c| c.use_tab_stops = true),
    );
    // DeleteLeft does not remove tab size, because some text exists before
    let len = editor.line(2).encode_utf16().count() as u32;
    editor.move_to(2, len + 1, false);
    editor.delete_left();
    assert_eq!(editor.line(2), "        a   ");

    // DeleteLeft removes tab size = 4
    editor.move_to(2, 9, false);
    editor.delete_left();
    assert_eq!(editor.line(2), "    a   ");

    // DeleteLeft removes tab size = 4
    editor.delete_left();
    assert_eq!(editor.line(2), "a   ");

    // Undo DeleteLeft - get us back to original indentation
    editor.undo();
    assert_eq!(editor.line(2), "        a   ");

    // Nothing is broken when cursor is in (1,1)
    editor.move_to(1, 1, false);
    editor.delete_left();
    assert_eq!(editor.line(1), " \t \t     x");

    // DeleteLeft stops at tab stops even in mixed whitespace case
    editor.move_to(1, 10, false);
    editor.delete_left();
    assert_eq!(editor.line(1), " \t \t    x");

    editor.delete_left();
    assert_eq!(editor.line(1), " \t \tx");

    editor.delete_left();
    assert_eq!(editor.line(1), " \tx");

    editor.delete_left();
    assert_eq!(editor.line(1), "x");

    // DeleteLeft on last line
    let len = editor.line(3).encode_utf16().count() as u32;
    editor.move_to(3, len + 1, false);
    editor.delete_left();
    assert_eq!(editor.line(3), "");

    // DeleteLeft with removing new line symbol
    editor.delete_left();
    assert_eq!(editor.value(), "x\n        a   ");

    // In case of selection DeleteLeft only deletes selected text
    editor.move_to(2, 3, false);
    editor.move_to(2, 4, true);
    editor.delete_left();
    assert_eq!(editor.line(2), "       a   ");
}

#[test]
fn pr_5423_auto_indent_undo_redo_is_funky() {
    let mut editor = TestEditor::with_config("", config(|c| c.insert_spaces = false));
    editor.type_keyboard("\n");
    assert_eq!(editor.value(), "\n", "assert1");

    editor.tab();
    assert_eq!(editor.value(), "\n\t", "assert2");

    editor.type_keyboard("y");
    assert_eq!(editor.value(), "\n\ty", "assert2");

    editor.type_keyboard("\n");
    assert_eq!(editor.value(), "\n\ty\n\t", "assert3");

    editor.type_raw("x");
    assert_eq!(editor.value(), "\n\ty\n\tx", "assert4");

    editor.move_left(false);
    assert_eq!(editor.value(), "\n\ty\n\tx", "assert5");

    editor.delete_left();
    assert_eq!(editor.value(), "\n\ty\nx", "assert6");

    editor.delete_left();
    assert_eq!(editor.value(), "\n\tyx", "assert7");

    editor.delete_left();
    assert_eq!(editor.value(), "\n\tx", "assert8");

    editor.delete_left();
    assert_eq!(editor.value(), "\nx", "assert9");

    editor.delete_left();
    assert_eq!(editor.value(), "x", "assert10");

    editor.undo();
    assert_eq!(editor.value(), "\nx", "assert11");

    editor.undo();
    assert_eq!(editor.value(), "\n\ty\nx", "assert12");

    editor.undo();
    assert_eq!(editor.value(), "\n\ty\n\tx", "assert13");

    editor.redo();
    assert_eq!(editor.value(), "\n\ty\nx", "assert14");

    editor.redo();
    assert_eq!(editor.value(), "\nx", "assert15");

    editor.redo();
    assert_eq!(editor.value(), "x", "assert16");
}

#[test]
fn on_enter_works_if_there_are_no_indentation_rules() {
    let mut editor = TestEditor::lines_with_config(
        &[
            "<?",
            "\tif (true) {",
            "\t\techo $hi;",
            "\t\techo $bye;",
            "\t}",
            "?>",
        ],
        config(|c| c.insert_spaces = false),
    );
    editor.move_to(5, 3, false);
    editor.assert_selection(sel(5, 3, 5, 3));

    editor.type_keyboard("\n");
    assert_eq!(editor.line(6), "\t");
    editor.assert_selection(sel(6, 2, 6, 2));
    assert_eq!(editor.line(5), "\t}");
}

#[test]
fn on_enter_works_if_there_are_no_indentation_rules_2() {
    let mut editor = TestEditor::lines_with_config(
        &["\tif (5)", "\t\treturn 5;", "\t"],
        config(|c| c.insert_spaces = false),
    );
    editor.move_to(3, 2, false);
    editor.assert_selection(sel(3, 2, 3, 2));

    editor.type_keyboard("\n");
    editor.assert_selection(sel(4, 2, 4, 2));
    assert_eq!(editor.line(4), "\t");
}

#[test]
fn issue_7100_mouse_word_selection_is_strange_when_non_word_character_is_at_the_end_of_line() {
    let mut editor = TestEditor::lines(&[
        "before.a",
        "before",
        "hello:",
        "there:",
        "this is strange:",
        "here",
        "it",
        "is",
    ]);
    editor.word_select(3, 7, false);
    editor.assert_selection(sel(3, 7, 3, 7));

    editor.word_select(4, 7, true);
    editor.assert_selection(sel(3, 7, 4, 7));
}

#[test]
fn issue_112039_shift_continuing_a_double_click_and_drag_selection_remembers_its_starting_mode() {
    let mut editor = TestEditor::lines(&["just some text", "and another line", "and another one"]);
    editor.word_select(2, 6, false);
    editor.move_to(1, 8, true);
    editor.assert_selection(sel(2, 12, 1, 6));
}

#[test]
fn issue_158236_shift_click_selection_does_not_work_on_line_number_indicator() {
    let mut editor = TestEditor::lines(&["just some text", "and another line", "and another one"]);
    editor.move_to(3, 5, false);
    editor.line_select(2, 1, true);
    editor.assert_selection(sel(3, 5, 2, 1));
}

// ---------------------------------------------------------------------------
// suite('Undo stops')

fn undo_stops_editor() -> TestEditor {
    TestEditor::lines(&["A  line", "Another line"])
}

#[test]
fn there_is_an_undo_stop_between_typing_and_deleting_left() {
    let mut editor = undo_stops_editor();
    editor.set_selections(&[sel(1, 3, 1, 3)]);
    editor.type_keyboard("first");
    assert_eq!(editor.line(1), "A first line");
    editor.assert_selection(sel(1, 8, 1, 8));

    editor.delete_left();
    editor.delete_left();
    assert_eq!(editor.line(1), "A fir line");
    editor.assert_selection(sel(1, 6, 1, 6));

    editor.undo();
    assert_eq!(editor.line(1), "A first line");
    editor.assert_selection(sel(1, 8, 1, 8));

    editor.undo();
    assert_eq!(editor.line(1), "A  line");
    editor.assert_selection(sel(1, 3, 1, 3));
}

#[test]
fn there_is_an_undo_stop_between_typing_and_deleting_right() {
    let mut editor = undo_stops_editor();
    editor.set_selections(&[sel(1, 3, 1, 3)]);
    editor.type_keyboard("first");
    assert_eq!(editor.line(1), "A first line");
    editor.assert_selection(sel(1, 8, 1, 8));

    editor.delete_right();
    editor.delete_right();
    assert_eq!(editor.line(1), "A firstine");
    editor.assert_selection(sel(1, 8, 1, 8));

    editor.undo();
    assert_eq!(editor.line(1), "A first line");
    editor.assert_selection(sel(1, 8, 1, 8));

    editor.undo();
    assert_eq!(editor.line(1), "A  line");
    editor.assert_selection(sel(1, 3, 1, 3));
}

#[test]
fn there_is_an_undo_stop_between_deleting_left_and_typing() {
    let mut editor = undo_stops_editor();
    editor.set_selections(&[sel(2, 8, 2, 8)]);
    for _ in 0..7 {
        editor.delete_left();
    }
    assert_eq!(editor.line(2), " line");
    editor.assert_selection(sel(2, 1, 2, 1));

    editor.type_keyboard("Second");
    assert_eq!(editor.line(2), "Second line");
    editor.assert_selection(sel(2, 7, 2, 7));

    editor.undo();
    assert_eq!(editor.line(2), " line");
    editor.assert_selection(sel(2, 1, 2, 1));

    editor.undo();
    assert_eq!(editor.line(2), "Another line");
    editor.assert_selection(sel(2, 8, 2, 8));
}

#[test]
fn there_is_an_undo_stop_between_deleting_left_and_deleting_right() {
    let mut editor = undo_stops_editor();
    editor.set_selections(&[sel(2, 8, 2, 8)]);
    for _ in 0..7 {
        editor.delete_left();
    }
    assert_eq!(editor.line(2), " line");
    editor.assert_selection(sel(2, 1, 2, 1));

    for _ in 0..5 {
        editor.delete_right();
    }
    assert_eq!(editor.line(2), "");
    editor.assert_selection(sel(2, 1, 2, 1));

    editor.undo();
    assert_eq!(editor.line(2), " line");
    editor.assert_selection(sel(2, 1, 2, 1));

    editor.undo();
    assert_eq!(editor.line(2), "Another line");
    editor.assert_selection(sel(2, 8, 2, 8));
}

#[test]
fn there_is_an_undo_stop_between_deleting_right_and_typing() {
    let mut editor = undo_stops_editor();
    editor.set_selections(&[sel(2, 9, 2, 9)]);
    for _ in 0..4 {
        editor.delete_right();
    }
    assert_eq!(editor.line(2), "Another ");
    editor.assert_selection(sel(2, 9, 2, 9));

    editor.type_keyboard("text");
    assert_eq!(editor.line(2), "Another text");
    editor.assert_selection(sel(2, 13, 2, 13));

    editor.undo();
    assert_eq!(editor.line(2), "Another ");
    editor.assert_selection(sel(2, 9, 2, 9));

    editor.undo();
    assert_eq!(editor.line(2), "Another line");
    editor.assert_selection(sel(2, 9, 2, 9));
}

#[test]
fn there_is_an_undo_stop_between_deleting_right_and_deleting_left() {
    let mut editor = undo_stops_editor();
    editor.set_selections(&[sel(2, 9, 2, 9)]);
    for _ in 0..4 {
        editor.delete_right();
    }
    assert_eq!(editor.line(2), "Another ");
    editor.assert_selection(sel(2, 9, 2, 9));

    for _ in 0..6 {
        editor.delete_left();
    }
    assert_eq!(editor.line(2), "An");
    editor.assert_selection(sel(2, 3, 2, 3));

    editor.undo();
    assert_eq!(editor.line(2), "Another ");
    editor.assert_selection(sel(2, 9, 2, 9));

    editor.undo();
    assert_eq!(editor.line(2), "Another line");
    editor.assert_selection(sel(2, 9, 2, 9));
}

#[test]
fn inserts_undo_stop_when_typing_space() {
    let mut editor = undo_stops_editor();
    editor.set_selections(&[sel(1, 3, 1, 3)]);
    editor.type_keyboard("first and interesting");
    assert_eq!(editor.line(1), "A first and interesting line");
    editor.assert_selection(sel(1, 24, 1, 24));

    editor.undo();
    assert_eq!(editor.line(1), "A first and line");
    editor.assert_selection(sel(1, 12, 1, 12));

    editor.undo();
    assert_eq!(editor.line(1), "A first line");
    editor.assert_selection(sel(1, 8, 1, 8));

    editor.undo();
    assert_eq!(editor.line(1), "A  line");
    editor.assert_selection(sel(1, 3, 1, 3));
}

#[test]
fn issue_93585_undo_multi_cursor_edit_corrupts_document() {
    let mut editor = TestEditor::lines(&["hello world", "hello world"]);
    editor.set_selections(&[sel(2, 7, 2, 12), sel(1, 7, 1, 12)]);
    editor.type_keyboard("no");
    assert_eq!(editor.value(), "hello no\nhello no");

    editor.undo();
    assert_eq!(editor.value(), "hello world\nhello world");
}

#[test]
fn there_is_a_single_undo_stop_for_consecutive_whitespaces() {
    let mut editor = TestEditor::with_config("", config(|c| c.insert_spaces = false));
    for s in ["a", "b", " ", " ", "c", "d"] {
        editor.type_keyboard(s);
    }
    assert_eq!(editor.value(), "ab  cd", "assert1");

    editor.undo();
    assert_eq!(editor.value(), "ab  ", "assert2");

    editor.undo();
    assert_eq!(editor.value(), "ab", "assert3");

    editor.undo();
    assert_eq!(editor.value(), "", "assert4");
}

#[test]
fn there_is_no_undo_stop_after_a_single_whitespace() {
    let mut editor = TestEditor::with_config("", config(|c| c.insert_spaces = false));
    for s in ["a", "b", " ", "c", "d"] {
        editor.type_keyboard(s);
    }
    assert_eq!(editor.value(), "ab cd", "assert1");

    editor.undo();
    assert_eq!(editor.value(), "ab", "assert3");

    editor.undo();
    assert_eq!(editor.value(), "", "assert4");
}
