//! Port of the language-dependent tests of Monaco's
//! `src/vs/editor/test/browser/controller/cursor.test.ts` (MIT), suite
//! `Editor Controller`: auto-closing pairs, overtyping and deleting them,
//! surround selection, electric characters, onEnter rules and indentation
//! rules. The language configurations are in `language_configs.rs`.
//!
//! Tests without a `languageId` in Monaco run in its `plaintext` language
//! ([`plaintext`]).
//!
//! Skipped (by reason):
//! - IME composition (`compositionEndWithInterceptors` is not ported):
//!   `issue #15825`, `issue #90016`, `issue #2773`, `issue #26820`,
//!   `issue #144690`, `issue #144693`, `issue #82701`.
//! - Reentrant model/decoration listeners: `issue #338693: multi-character
//!   autoclose does not insert an orphan close when a listener reentrantly
//!   removes the typed character`, `issue #338693: multi-character autoclose
//!   does not use stale selections after a reentrant edit`, `issue #205598`.
//! - Snippet edits that auto-close (`executeEdits('snippet', ...)`):
//!   `issue #78975`.
//! - Auto-whitespace trimming (a text model feature): `removeAutoWhitespace
//!   on: removes only whitespace the cursor added 2`.

use syntax::{IndentAction, LanguageConfiguration, NotIn};

use super::harness::{TestEditor, pos, sel};
use super::language_configs::*;
use super::tokenizer::Tokenizer;
use crate::cursor::{
    AutoClosingEditStrategy, AutoClosingStrategy, AutoIndent, AutoSurroundStrategy, CursorConfig,
    EditOperation,
};

fn using_cursor(text: &[&str], language: LanguageConfiguration) -> TestEditor {
    TestEditor::lines(text).with_language(language)
}

fn using_cursor_with(
    text: &[&str],
    language: LanguageConfiguration,
    f: impl FnOnce(&mut CursorConfig),
) -> TestEditor {
    let mut config = CursorConfig::default();
    f(&mut config);
    TestEditor::lines_with_config(text, config).with_language(language)
}

fn utf16_split(s: &str, column: u32) -> (String, String) {
    let units: Vec<u16> = s.encode_utf16().collect();
    let at = (column as usize).min(units.len());
    (
        String::from_utf16_lossy(&units[..at]),
        String::from_utf16_lossy(&units[at..]),
    )
}

/// Monaco's `assertType`.
#[track_caller]
fn assert_type(
    ed: &mut TestEditor,
    line: u32,
    column: u32,
    chr: &str,
    expected_insert: &str,
    message: &str,
) {
    let content = ed.line(line);
    let (before, after) = utf16_split(&content, column - 1);
    let expected = format!("{before}{expected_insert}{after}");
    ed.move_to(line, column, false);
    ed.type_keyboard(chr);
    assert_eq!(ed.line(line), expected, "{message}");
    ed.undo();
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ColumnType {
    Normal,
    Special1,
    Special2,
}

/// Monaco's `extractAutoClosingSpecialColumns`: index 1..=max_column.
fn extract_auto_closing_special_columns(max_column: u32, annotated_line: &str) -> Vec<ColumnType> {
    let mut result = vec![ColumnType::Normal; max_column as usize + 1];
    let mut column = 1;
    for c in annotated_line.chars() {
        match c {
            '|' => result[column] = ColumnType::Special1,
            '!' => result[column] = ColumnType::Special2,
            _ => column += 1,
        }
    }
    result
}

fn max_column(ed: &TestEditor, line: u32) -> u32 {
    ed.line(line).encode_utf16().count() as u32 + 1
}

const AUTO_CLOSING_TEXT: [&str; 8] = [
    "var a = [];",
    "var b = `asd`;",
    "var c = 'asd';",
    "var d = \"asd\";",
    "var e = /*3*/\t3;",
    "var f = /** 3 */3;",
    "var g = (3+5);",
    "var h = { a: 'value' };",
];

/// Types `(` at every column, expecting it to auto-close exactly at the
/// columns marked `|`.
#[track_caller]
fn check_open_paren_positions(ed: &mut TestEditor, auto_close_positions: &[&str]) {
    for (i, annotated) in auto_close_positions.iter().enumerate() {
        let line_number = i as u32 + 1;
        let columns = extract_auto_closing_special_columns(max_column(ed, line_number), annotated);
        for column in 1..columns.len() as u32 {
            if columns[column as usize] == ColumnType::Special1 {
                assert_type(
                    ed,
                    line_number,
                    column,
                    "(",
                    "()",
                    &format!("auto closes @ ({line_number}, {column})"),
                );
            } else {
                assert_type(
                    ed,
                    line_number,
                    column,
                    "(",
                    "(",
                    &format!("does not auto close @ ({line_number}, {column})"),
                );
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Surround selection

#[test]
fn issue_47733_undo_mangles_unicode_characters() {
    let mut ed = using_cursor(
        &["'👁'"],
        LanguageConfiguration {
            surrounding_pairs: vec![surround("%", "%")],
            ..Default::default()
        },
    );
    ed.set_selection(sel(1, 1, 1, 2));
    ed.type_keyboard("%");
    assert_eq!(ed.value(), "%'%👁'", "assert1");
    ed.undo();
    assert_eq!(ed.value(), "'👁'", "assert2");
}

#[test]
fn bug_11476_double_bracket_surrounding_undo_is_broken() {
    let mut ed = using_cursor(&["hello"], surrounding_language());
    ed.move_to(1, 3, false);
    ed.move_to(1, 5, true);
    ed.assert_selection(sel(1, 3, 1, 5));

    ed.type_keyboard("(");
    ed.assert_selection(sel(1, 4, 1, 6));

    ed.type_keyboard("(");
    ed.assert_selection(sel(1, 5, 1, 7));
}

#[test]
fn issue_206774_surround_selection_command_with_empty_char_after_selection_should_not_throw() {
    let mut ed = using_cursor(&["hello world"], empty_closing_surround_language());
    ed.move_to(1, 1, false);
    ed.move_to(1, 6, true);
    ed.assert_selection(sel(1, 1, 1, 6));
    ed.type_keyboard("<");
    assert_eq!(ed.value(), "<hello world");
}

// ---------------------------------------------------------------------------
// onEnter rules

#[test]
fn enter_auto_indents_with_insert_spaces_setting_1() {
    let mut ed = using_cursor(&["\thello"], on_enter_language(IndentAction::Indent));
    ed.move_to(1, 7, false);
    ed.assert_selection(sel(1, 7, 1, 7));
    ed.type_keyboard("\n");
    assert_eq!(ed.value(), "\thello\n        ");
}

#[test]
fn enter_auto_indents_with_insert_spaces_setting_2() {
    let mut ed = using_cursor(&["\thello"], on_enter_language(IndentAction::None));
    ed.move_to(1, 7, false);
    ed.assert_selection(sel(1, 7, 1, 7));
    ed.type_keyboard("\n");
    assert_eq!(ed.value(), "\thello\n    ");
}

#[test]
fn enter_auto_indents_with_insert_spaces_setting_3() {
    let mut ed = using_cursor(
        &["\thell()"],
        on_enter_language(IndentAction::IndentOutdent),
    );
    ed.move_to(1, 7, false);
    ed.assert_selection(sel(1, 7, 1, 7));
    ed.type_keyboard("\n");
    assert_eq!(ed.value(), "\thell(\n        \n    )");
}

#[test]
fn issue_115033_indent_and_append_text() {
    let mut ed = using_cursor(
        &["text"],
        LanguageConfiguration {
            on_enter_rules: vec![on_enter(
                re(".*"),
                None,
                None,
                IndentAction::Indent,
                Some("x"),
                None,
            )],
            ..Default::default()
        },
    );
    ed.move_to(1, 5, false);
    ed.type_keyboard("\n");
    assert_eq!(ed.line(1), "text");
    assert_eq!(ed.line(2), "    x");
    ed.assert_cursor(2, 6);
}

#[test]
fn issue_6862_editor_removes_auto_inserted_indentation_when_formatting_on_type() {
    let mut ed = using_cursor(
        &["function foo (params: string) {}"],
        on_enter_language(IndentAction::IndentOutdent),
    );
    ed.move_to(1, 32, false);
    ed.type_keyboard("\n");
    assert_eq!(ed.line(1), "function foo (params: string) {");
    assert_eq!(ed.line(2), "    ");
    assert_eq!(ed.line(3), "}");

    // Monaco runs a custom command deleting (1,13)-(1,14), away from the
    // cursor, so the auto-inserted indentation stays.
    ed.run(|_, buffer, cursors| {
        EditOperation::from_edits(
            buffer,
            &cursors.selections(),
            vec![(pos(1, 13)..pos(1, 14), String::new())],
        )
    });
    assert_eq!(ed.line(1), "function foo(params: string) {");
    assert_eq!(ed.line(2), "    ");
    assert_eq!(ed.line(3), "}");
}

#[test]
fn on_enter_works_if_there_are_no_indentation_rules() {
    let mut ed = using_cursor_with(
        &[
            "<?",
            "\tif (true) {",
            "\t\techo $hi;",
            "\t\techo $bye;",
            "\t}",
            "?>",
        ],
        plaintext(),
        |c| c.insert_spaces = false,
    );
    ed.move_to(5, 3, false);
    ed.assert_selection(sel(5, 3, 5, 3));
    ed.type_keyboard("\n");
    assert_eq!(ed.line(6), "\t");
    ed.assert_selection(sel(6, 2, 6, 2));
    assert_eq!(ed.line(5), "\t}");
}

#[test]
fn on_enter_works_if_there_are_no_indentation_rules_2() {
    let mut ed = using_cursor_with(&["\tif (5)", "\t\treturn 5;", "\t"], plaintext(), |c| {
        c.insert_spaces = false
    });
    ed.move_to(3, 2, false);
    ed.assert_selection(sel(3, 2, 3, 2));
    ed.type_keyboard("\n");
    ed.assert_selection(sel(4, 2, 4, 2));
    assert_eq!(ed.line(4), "\t");
}

// ---------------------------------------------------------------------------
// Indentation rules

fn no_tabs(c: &mut CursorConfig) {
    c.insert_spaces = false;
}

#[test]
fn enter_honors_increase_indent_pattern() {
    let mut ed = using_cursor_with(
        &["if (true) {", "\tif (true) {"],
        indent_rules_language(),
        no_tabs,
    );
    ed.move_to(1, 12, false);
    ed.assert_selection(sel(1, 12, 1, 12));
    ed.type_keyboard("\n");
    ed.assert_selection(sel(2, 2, 2, 2));

    ed.move_to(3, 13, false);
    ed.assert_selection(sel(3, 13, 3, 13));
    ed.type_keyboard("\n");
    ed.assert_selection(sel(4, 3, 4, 3));
}

#[test]
fn type_honors_decrease_indent_pattern() {
    let mut ed = using_cursor(&["if (true) {", "\t"], indent_rules_language());
    ed.move_to(2, 2, false);
    ed.assert_selection(sel(2, 2, 2, 2));
    ed.type_keyboard("}");
    ed.assert_selection(sel(2, 2, 2, 2));
    assert_eq!(ed.line(2), "}", "001");
}

#[test]
fn enter_honors_un_indented_line_pattern() {
    let mut ed = using_cursor_with(
        &["if (true) {", "\t\t\treturn true"],
        indent_rules_language(),
        no_tabs,
    );
    ed.move_to(2, 15, false);
    ed.assert_selection(sel(2, 15, 2, 15));
    ed.type_keyboard("\n");
    ed.assert_selection(sel(3, 2, 3, 2));
}

#[test]
fn enter_honors_indent_next_line_pattern() {
    let mut ed = using_cursor_with(
        &[
            "if (true)",
            "\treturn true;",
            "if (true)",
            "\t\t\t\treturn true",
        ],
        indent_rules_language(),
        no_tabs,
    );
    ed.move_to(2, 14, false);
    ed.assert_selection(sel(2, 14, 2, 14));
    ed.type_keyboard("\n");
    ed.assert_selection(sel(3, 1, 3, 1));

    ed.move_to(5, 16, false);
    ed.assert_selection(sel(5, 16, 5, 16));
    ed.type_keyboard("\n");
    ed.assert_selection(sel(6, 2, 6, 2));
}

#[test]
fn enter_honors_indent_next_line_pattern_2() {
    let mut ed = using_cursor_with(
        &["if (true)", "\tif (true)"],
        indent_rules_language(),
        no_tabs,
    );
    ed.move_to(2, 11, false);
    ed.assert_selection(sel(2, 11, 2, 11));
    ed.type_keyboard("\n");
    ed.assert_selection(sel(3, 3, 3, 3));

    ed.type_keyboard("console.log();");
    ed.type_keyboard("\n");
    ed.assert_selection(sel(4, 1, 4, 1));
}

#[test]
fn enter_honors_intential_indent() {
    let mut ed = using_cursor(
        &["if (true) {", "\tif (true) {", "return true;", "}}"],
        indent_rules_language(),
    );
    ed.move_to(3, 13, false);
    ed.assert_selection(sel(3, 13, 3, 13));
    ed.type_keyboard("\n");
    ed.assert_selection(sel(4, 1, 4, 1));
    assert_eq!(ed.line(3), "return true;", "001");
}

#[test]
fn enter_supports_selection_1() {
    let mut ed = using_cursor_with(
        &["if (true) {", "\tif (true) {", "\t\treturn true;", "\t}a}"],
        indent_rules_language(),
        no_tabs,
    );
    ed.move_to(4, 3, false);
    ed.move_to(4, 4, true);
    ed.assert_selection(sel(4, 3, 4, 4));
    ed.type_keyboard("\n");
    ed.assert_selection(sel(5, 1, 5, 1));
    assert_eq!(ed.line(4), "\t}", "001");
}

#[test]
fn enter_supports_selection_2() {
    let mut ed = using_cursor_with(
        &["if (true) {", "\tif (true) {"],
        indent_rules_language(),
        no_tabs,
    );
    ed.move_to(2, 12, false);
    ed.move_to(2, 13, true);
    ed.assert_selection(sel(2, 12, 2, 13));
    ed.type_keyboard("\n");
    ed.assert_selection(sel(3, 3, 3, 3));
    ed.type_keyboard("\n");
    ed.assert_selection(sel(4, 3, 4, 3));
}

#[test]
fn enter_honors_tab_size_and_insert_spaces_1() {
    let mut ed = using_cursor(&["if (true) {", "\tif (true) {"], indent_rules_language());
    ed.move_to(1, 12, false);
    ed.assert_selection(sel(1, 12, 1, 12));
    ed.type_keyboard("\n");
    ed.assert_selection(sel(2, 5, 2, 5));

    ed.move_to(3, 13, false);
    ed.assert_selection(sel(3, 13, 3, 13));
    ed.type_keyboard("\n");
    ed.assert_selection(sel(4, 9, 4, 9));
}

#[test]
fn enter_honors_tab_size_and_insert_spaces_2() {
    let mut ed = using_cursor(&["if (true) {", "    if (true) {"], indent_rules_language());
    ed.move_to(1, 12, false);
    ed.assert_selection(sel(1, 12, 1, 12));
    ed.type_keyboard("\n");
    ed.assert_selection(sel(2, 5, 2, 5));

    ed.move_to(3, 16, false);
    ed.assert_selection(sel(3, 16, 3, 16));
    ed.type_keyboard("\n");
    assert_eq!(ed.line(3), "    if (true) {");
    ed.assert_selection(sel(4, 9, 4, 9));
}

#[test]
fn enter_honors_tab_size_and_insert_spaces_3() {
    let mut ed = using_cursor_with(
        &["if (true) {", "    if (true) {"],
        indent_rules_language(),
        no_tabs,
    );
    ed.move_to(1, 12, false);
    ed.assert_selection(sel(1, 12, 1, 12));
    ed.type_keyboard("\n");
    ed.assert_selection(sel(2, 2, 2, 2));

    ed.move_to(3, 16, false);
    ed.assert_selection(sel(3, 16, 3, 16));
    ed.type_keyboard("\n");
    assert_eq!(ed.line(3), "    if (true) {");
    ed.assert_selection(sel(4, 3, 4, 3));
}

#[test]
fn enter_supports_intentional_indentation() {
    let mut ed = using_cursor_with(
        &[
            "\tif (true) {",
            "\t\tswitch(true) {",
            "\t\t\tcase true:",
            "\t\t\t\tbreak;",
            "\t\t}",
            "\t}",
        ],
        indent_rules_language(),
        no_tabs,
    );
    ed.move_to(5, 4, false);
    ed.assert_selection(sel(5, 4, 5, 4));
    ed.type_keyboard("\n");
    assert_eq!(ed.line(5), "\t\t}");
    ed.assert_selection(sel(6, 3, 6, 3));
}

#[test]
fn enter_should_not_adjust_cursor_position_when_press_enter_in_the_middle_of_a_line_1() {
    let mut ed = using_cursor_with(
        &["if (true) {", "\tif (true) {", "\t\treturn true;", "\t}a}"],
        indent_rules_language(),
        no_tabs,
    );
    ed.move_to(3, 9, false);
    ed.assert_selection(sel(3, 9, 3, 9));
    ed.type_keyboard("\n");
    ed.assert_selection(sel(4, 3, 4, 3));
    assert_eq!(ed.line(4), "\t\t true;", "001");
}

#[test]
fn enter_should_not_adjust_cursor_position_when_press_enter_in_the_middle_of_a_line_2() {
    let mut ed = using_cursor_with(
        &["if (true) {", "\tif (true) {", "\t\treturn true;", "\t}a}"],
        indent_rules_language(),
        no_tabs,
    );
    ed.move_to(3, 3, false);
    ed.assert_selection(sel(3, 3, 3, 3));
    ed.type_keyboard("\n");
    ed.assert_selection(sel(4, 3, 4, 3));
    assert_eq!(ed.line(4), "\t\treturn true;", "001");
}

#[test]
fn enter_should_not_adjust_cursor_position_when_press_enter_in_the_middle_of_a_line_3() {
    let mut ed = using_cursor(
        &["if (true) {", "  if (true) {", "    return true;", "  }a}"],
        indent_rules_language(),
    );
    ed.move_to(3, 11, false);
    ed.assert_selection(sel(3, 11, 3, 11));
    ed.type_keyboard("\n");
    ed.assert_selection(sel(4, 5, 4, 5));
    assert_eq!(ed.line(4), "     true;", "001");
}

#[test]
fn enter_should_adjust_cursor_position_when_press_enter_in_the_middle_of_leading_whitespaces_1() {
    let mut ed = using_cursor_with(
        &["if (true) {", "\tif (true) {", "\t\treturn true;", "\t}a}"],
        indent_rules_language(),
        no_tabs,
    );
    ed.move_to(3, 2, false);
    ed.assert_selection(sel(3, 2, 3, 2));
    ed.type_keyboard("\n");
    ed.assert_selection(sel(4, 2, 4, 2));
    assert_eq!(ed.line(4), "\t\treturn true;", "001");

    ed.move_to(4, 1, false);
    ed.assert_selection(sel(4, 1, 4, 1));
    ed.type_keyboard("\n");
    ed.assert_selection(sel(5, 1, 5, 1));
    assert_eq!(ed.line(5), "\t\treturn true;", "002");
}

#[test]
fn enter_should_adjust_cursor_position_when_press_enter_in_the_middle_of_leading_whitespaces_2() {
    let mut ed = using_cursor_with(
        &[
            "\tif (true) {",
            "\t\tif (true) {",
            "\t    \treturn true;",
            "\t\t}a}",
        ],
        indent_rules_language(),
        no_tabs,
    );
    ed.move_to(3, 4, false);
    ed.assert_selection(sel(3, 4, 3, 4));
    ed.type_keyboard("\n");
    ed.assert_selection(sel(4, 3, 4, 3));
    assert_eq!(ed.line(4), "\t\t\treturn true;", "001");

    ed.move_to(4, 1, false);
    ed.assert_selection(sel(4, 1, 4, 1));
    ed.type_keyboard("\n");
    ed.assert_selection(sel(5, 1, 5, 1));
    assert_eq!(ed.line(5), "\t\t\treturn true;", "002");
}

#[test]
fn enter_should_adjust_cursor_position_when_press_enter_in_the_middle_of_leading_whitespaces_3() {
    let mut ed = using_cursor(
        &["if (true) {", "  if (true) {", "    return true;", "}a}"],
        indent_rules_language(),
    );
    ed.move_to(3, 2, false);
    ed.assert_selection(sel(3, 2, 3, 2));
    ed.type_keyboard("\n");
    ed.assert_selection(sel(4, 2, 4, 2));
    assert_eq!(ed.line(4), "    return true;", "001");

    ed.move_to(4, 3, false);
    ed.type_keyboard("\n");
    ed.assert_selection(sel(5, 3, 5, 3));
    assert_eq!(ed.line(5), "    return true;", "002");
}

#[test]
fn enter_should_adjust_cursor_position_when_press_enter_in_the_middle_of_leading_whitespaces_4() {
    let mut ed = using_cursor_with(
        &[
            "if (true) {",
            "  if (true) {",
            "\t  return true;",
            "}a}",
            "",
            "if (true) {",
            "  if (true) {",
            "\t  return true;",
            "}a}",
        ],
        indent_rules_language(),
        |c| {
            c.tab_size = 2;
            c.indent_size = 2;
        },
    );
    ed.move_to(3, 3, false);
    ed.assert_selection(sel(3, 3, 3, 3));
    ed.type_keyboard("\n");
    ed.assert_selection(sel(4, 4, 4, 4));
    assert_eq!(ed.line(4), "    return true;", "001");

    ed.move_to(9, 4, false);
    ed.type_keyboard("\n");
    ed.assert_selection(sel(10, 5, 10, 5));
    assert_eq!(ed.line(10), "    return true;", "001");
}

#[test]
fn enter_should_adjust_cursor_position_when_press_enter_in_the_middle_of_leading_whitespaces_5() {
    let mut ed = using_cursor_with(
        &[
            "if (true) {",
            "  if (true) {",
            "    return true;",
            "    return true;",
            "",
        ],
        indent_rules_language(),
        // Monaco's `tabSize: 2` here leaves `indentSize` at 4.
        |c| c.tab_size = 2,
    );
    ed.move_to(3, 5, false);
    ed.move_to(4, 3, true);
    ed.assert_selection(sel(3, 5, 4, 3));
    ed.type_keyboard("\n");
    ed.assert_selection(sel(4, 3, 4, 3));
    assert_eq!(ed.line(4), "    return true;", "001");
}

#[test]
fn issue_microsoft_monaco_editor_108_part_1_auto_indentation_on_enter_with_selection_is_half_broken()
 {
    let mut ed = using_cursor_with(
        &[
            "function baz() {",
            "\tvar x = 1;",
            "\t\t\t\t\t\t\treturn x;",
            "}",
        ],
        indent_rules_language(),
        no_tabs,
    );
    ed.move_to(3, 8, false);
    ed.move_to(2, 12, true);
    ed.assert_selection(sel(3, 8, 2, 12));
    ed.type_keyboard("\n");
    assert_eq!(ed.line(3), "\treturn x;");
    ed.assert_cursor(3, 2);
}

#[test]
fn issue_microsoft_monaco_editor_108_part_2_auto_indentation_on_enter_with_selection_is_half_broken()
 {
    let mut ed = using_cursor_with(
        &[
            "function baz() {",
            "\tvar x = 1;",
            "\t\t\t\t\t\t\treturn x;",
            "}",
        ],
        indent_rules_language(),
        no_tabs,
    );
    ed.move_to(2, 12, false);
    ed.move_to(3, 8, true);
    ed.assert_selection(sel(2, 12, 3, 8));
    ed.type_keyboard("\n");
    assert_eq!(ed.line(3), "\treturn x;");
    ed.assert_cursor(3, 2);
}

#[test]
fn bug_16543_tab_should_indent_to_correct_indentation_spot_immediately() {
    let mut ed = using_cursor_with(
        &[
            "function baz() {",
            "\tfunction hello() { // something here",
            "\t",
            "",
            "\t}",
            "}",
        ],
        indent_rules_language(),
        no_tabs,
    );
    ed.move_to(4, 1, false);
    ed.assert_selection(sel(4, 1, 4, 1));
    ed.tab();
    assert_eq!(ed.line(4), "\t\t");
}

fn bug_2938(line4: &str, column: u32, expected: &str) {
    let mut ed = using_cursor_with(
        &[
            "\tfunction baz() {",
            "\t\tfunction hello() { // something here",
            "\t\t",
            line4,
            "\t\t}",
            "\t}",
        ],
        indent_rules_language(),
        no_tabs,
    );
    ed.move_to(4, column, false);
    ed.assert_selection(sel(4, column, 4, column));
    ed.tab();
    assert_eq!(ed.line(4), expected);
}

#[test]
fn bug_2938_1_when_pressing_tab_on_white_space_only_lines_indent_straight_to_the_right_spot() {
    bug_2938("\t", 2, "\t\t\t");
}

#[test]
fn bug_2938_2_when_pressing_tab_on_white_space_only_lines_indent_straight_to_the_right_spot() {
    bug_2938("    ", 1, "\t\t\t");
}

#[test]
fn bug_2938_3_when_pressing_tab_on_white_space_only_lines_indent_straight_to_the_right_spot() {
    bug_2938("\t\t\t", 3, "\t\t\t\t");
}

#[test]
fn bug_2938_4_when_pressing_tab_on_white_space_only_lines_indent_straight_to_the_right_spot() {
    bug_2938("\t\t\t\t", 4, "\t\t\t\t\t");
}

#[test]
fn bug_31015_when_pressing_tab_on_lines_and_enter_rules_are_avail_indent_straight_to_the_right_spot()
 {
    let mut ed = using_cursor(
        &["    if (a) {", "        ", "", "", "    }"],
        on_enter_language(IndentAction::Indent),
    );
    ed.move_to(3, 1, false);
    ed.tab();
    assert_eq!(ed.line(1), "    if (a) {");
    assert_eq!(ed.line(2), "        ");
    assert_eq!(ed.line(3), "        ");
    assert_eq!(ed.line(4), "");
    assert_eq!(ed.line(5), "    }");
}

#[test]
fn type_honors_indentation_rules_ruby_keywords() {
    let mut ed = using_cursor(
        &[
            "class Greeter",
            "  def initialize(name)",
            "    @name = name",
            "    en",
        ],
        indent_rules_only(
            re(
                r"^\s*((begin|class|def|else|elsif|ensure|for|if|module|rescue|unless|until|when|while)|(.*\sdo\b))\b[^\{;]*$",
            ),
            re(
                r"^\s*([}\]]([,)]?\s*(#|$)|\.[a-zA-Z_]\w*\b)|(end|rescue|ensure|else|elsif|when)\b)",
            ),
        ),
    );
    ed.move_to(4, 7, false);
    ed.assert_selection(sel(4, 7, 4, 7));
    ed.type_keyboard("d");
    assert_eq!(ed.line(4), "  end");
}

#[test]
fn auto_indent_on_type_increase_indent_pattern_has_higher_priority_than_decrease_indent_when_inheriting()
 {
    let mut ed = using_cursor(
        &[
            "\tif (true) {",
            "\t\tconsole.log();",
            "\t} else if {",
            "\t\tconsole.log()",
            "\t}",
        ],
        indent_rules_language(),
    );
    ed.move_to(5, 3, false);
    ed.assert_selection(sel(5, 3, 5, 3));
    ed.type_keyboard("e");
    ed.assert_selection(sel(5, 4, 5, 4));
    assert_eq!(ed.line(5), "\t}e", "This line should not decrease indent");
}

#[test]
fn type_honors_users_indentation_adjustment() {
    let mut ed = using_cursor(
        &["\tif (true ||", "\t ) {", "\t}", "if (true ||", ") {", "}"],
        indent_rules_language(),
    );
    ed.move_to(2, 3, false);
    ed.assert_selection(sel(2, 3, 2, 3));
    ed.type_keyboard(" ");
    ed.assert_selection(sel(2, 4, 2, 4));
    assert_eq!(
        ed.line(2),
        "\t  ) {",
        "This line should not decrease indent"
    );
}

#[test]
fn bug_29972_if_a_line_is_line_comment_open_bracket_should_not_indent_next_line() {
    let mut ed = using_cursor(&["if (true) {", "\t// {", "\t\t"], indent_rules_language());
    ed.move_to(3, 3, false);
    ed.assert_selection(sel(3, 3, 3, 3));
    ed.type_keyboard("}");
    ed.assert_selection(sel(3, 2, 3, 2));
    assert_eq!(ed.line(3), "}");
}

#[test]
fn issue_38261_tab_key_results_in_bizarre_indentation_in_cpp_mode() {
    let mut ed = using_cursor_with(
        &[
            "int main() {",
            "  return 0;",
            "}",
            "",
            "bool Foo::bar(const string &a,",
            "              const string &b) {",
            "  foo();",
            "",
            ")",
        ],
        LanguageConfiguration {
            brackets: standard_brackets(),
            indentation_rules: Some(rules(re(r"(^.*\{[^}]*$)"), re(r"^\s*\}"))),
            ..Default::default()
        },
        |c| {
            c.tab_size = 2;
            c.indent_size = 2;
            c.auto_indent = AutoIndent::Advanced;
        },
    );
    ed.move_to(8, 1, false);
    ed.assert_selection(sel(8, 1, 8, 1));
    ed.tab();
    assert_eq!(
        ed.value(),
        [
            "int main() {",
            "  return 0;",
            "}",
            "",
            "bool Foo::bar(const string &a,",
            "              const string &b) {",
            "  foo();",
            "  ",
            ")",
        ]
        .join("\n")
    );
    ed.assert_selection(sel(8, 3, 8, 3));
}

#[test]
fn issue_57197_indent_rules_regex_should_be_stateless() {
    let language = LanguageConfiguration {
        indentation_rules: Some(rules(
            syntax::Pattern::new(
                r"^(?![^\S\n]*(?!--|––|——)(?:[-❍❑■⬜□☐▪▫–—≡→›✘xX✔✓☑+]|\[[ xX+-]?\])\s[^\n]*)[^\S\n]*(.+:)[^\S\n]*(?:(?=@[^\s*~(]+(?::\/\/[^\s*~(:]+)?(?:\([^)]*\))?)|$)",
                "gm",
            ),
            syntax::Pattern::new(r"^\s*}$", "gm"),
        )),
        ..Default::default()
    };
    let mut ed = using_cursor_with(&["Project:"], language, no_tabs);
    ed.move_to(1, 9, false);
    ed.assert_selection(sel(1, 9, 1, 9));
    ed.type_keyboard("\n");
    ed.assert_selection(sel(2, 2, 2, 2));

    ed.move_to(1, 9, false);
    ed.assert_selection(sel(1, 9, 1, 9));
    ed.type_keyboard("\n");
    ed.assert_selection(sel(2, 2, 2, 2));
}

#[test]
fn typing_in_json() {
    let mut ed = using_cursor_with(
        &[
            "{",
            "  \"scripts: {\"",
            "    \"watch\": \"a {\"",
            "    \"build{\": \"b\"",
            "    \"tasks\": []",
            "    \"tasks\": [\"a\"]",
            "  \"}\"",
            "\"}\"",
        ],
        LanguageConfiguration {
            brackets: standard_brackets(),
            indentation_rules: Some(rules(
                re(r#"(\{+(?=([^"]*"[^"]*")*[^"}]*$))|(\[+(?=([^"]*"[^"]*")*[^"\]]*$))"#),
                re(r"^\s*[}\]],?\s*$"),
            )),
            ..Default::default()
        },
        |c| {
            c.tab_size = 2;
            c.indent_size = 2;
        },
    );
    ed.move_to(3, 19, false);
    ed.assert_selection(sel(3, 19, 3, 19));
    ed.type_keyboard("\n");
    assert_eq!(ed.line(4), "    ");

    ed.move_to(5, 18, false);
    ed.assert_selection(sel(5, 18, 5, 18));
    ed.type_keyboard("\n");
    assert_eq!(ed.line(6), "    ");

    ed.move_to(7, 15, false);
    ed.assert_selection(sel(7, 15, 7, 15));
    ed.type_keyboard("\n");
    assert_eq!(ed.line(8), "      ");
    assert_eq!(ed.line(9), "    ]");

    ed.move_to(10, 18, false);
    ed.assert_selection(sel(10, 18, 10, 18));
    ed.type_keyboard("\n");
    assert_eq!(ed.line(11), "    ]");
}

#[test]
fn issue_111128_multicursor_enter_issue_with_indentation() {
    let mut ed = using_cursor_with(&["    let a, b, c;"], indent_rules_language(), no_tabs);
    ed.set_selections(&[sel(1, 11, 1, 11), sel(1, 14, 1, 14)]);
    ed.type_keyboard("\n");
    assert_eq!(ed.value(), "    let a,\n\t b,\n\t c;");
}

#[test]
fn issue_122714_tab_size_1_prevent_typing_a_string_matching_decrease_indent_pattern_in_an_empty_file()
 {
    let mut ed = using_cursor_with(
        &["\\end"],
        indent_rules_only(
            re(r"\\begin\{(?!document)([^}]*)\}(?!.*\\end\{\1\})"),
            re(r"^\s*\\end\{(?!document)"),
        ),
        |c| c.tab_size = 1,
    );
    ed.move_to(1, 5, false);
    ed.assert_selection(sel(1, 5, 1, 5));
    ed.type_keyboard("{");
    // Monaco expects `\\end{}` here, but this language defines no
    // auto-closing pairs (nor brackets to default them from), so nothing can
    // auto-close `{`; what the issue is about is that the `{` is typed.
    assert_eq!(ed.line(1), "\\end{");
}

// ---------------------------------------------------------------------------
// Electric characters

#[test]
fn electric_character_does_nothing_if_no_electric_char() {
    let mut ed = using_cursor(&["  if (a) {", ""], electric_char_language());
    ed.move_to(2, 1, false);
    ed.type_keyboard("*");
    assert_eq!(ed.line(2), "*");
}

#[test]
fn electric_character_indents_in_order_to_match_bracket() {
    let mut ed = using_cursor(&["  if (a) {", ""], electric_char_language());
    ed.move_to(2, 1, false);
    ed.type_keyboard("}");
    assert_eq!(ed.line(2), "  }");
}

#[test]
fn electric_character_unindents_in_order_to_match_bracket() {
    let mut ed = using_cursor(&["  if (a) {", "    "], electric_char_language());
    ed.move_to(2, 5, false);
    ed.type_keyboard("}");
    assert_eq!(ed.line(2), "  }");
}

#[test]
fn electric_character_matches_with_correct_bracket() {
    let mut ed = using_cursor(
        &["  if (a) {", "    if (b) {", "    }", "    "],
        electric_char_language(),
    );
    ed.move_to(4, 1, false);
    ed.type_keyboard("}");
    assert_eq!(ed.line(4), "  }    ");
}

#[test]
fn electric_character_does_nothing_if_bracket_does_not_match() {
    let mut ed = using_cursor(
        &["  if (a) {", "    if (b) {", "    }", "  }  "],
        electric_char_language(),
    );
    ed.move_to(4, 6, false);
    ed.type_keyboard("}");
    assert_eq!(ed.line(4), "  }  }");
}

#[test]
fn electric_character_matches_bracket_even_in_line_with_content() {
    let mut ed = using_cursor(&["  if (a) {", "// hello"], electric_char_language());
    ed.move_to(2, 1, false);
    ed.type_keyboard("}");
    assert_eq!(ed.line(2), "  }// hello");
}

#[test]
fn electric_character_is_no_op_if_bracket_is_lined_up() {
    let mut ed = using_cursor(&["  if (a) {", "  "], electric_char_language());
    ed.move_to(2, 3, false);
    ed.type_keyboard("}");
    assert_eq!(ed.line(2), "  }");
}

#[test]
fn electric_character_is_no_op_if_there_is_non_whitespace_text_before() {
    let mut ed = using_cursor(&["  if (a) {", "a"], electric_char_language());
    ed.move_to(2, 2, false);
    ed.type_keyboard("}");
    assert_eq!(ed.line(2), "a}");
}

#[test]
fn electric_character_is_no_op_if_pairs_are_all_matched_before() {
    let mut ed = using_cursor(
        &["foo(() => {", "  ( 1 + 2 ) ", "})"],
        electric_char_language(),
    );
    ed.move_to(2, 13, false);
    ed.type_keyboard("*");
    assert_eq!(ed.line(2), "  ( 1 + 2 ) *");
}

#[test]
fn electric_character_is_no_op_if_matching_bracket_is_on_the_same_line() {
    let mut ed = using_cursor(&["(div"], electric_char_language());
    ed.move_to(1, 5, false);
    let before = ed.value();
    ed.type_keyboard(")");
    assert_eq!(ed.line(1), "(div)");
    // Monaco checks the change was just `)`.
    assert_eq!(format!("{before})"), ed.value());
}

#[test]
fn electric_character_is_no_op_if_the_line_has_other_content() {
    let mut ed = using_cursor(&["Math.max(", "\t2", "\t3"], electric_char_language());
    ed.move_to(3, 3, false);
    ed.type_keyboard(")");
    assert_eq!(ed.line(3), "\t3)");
}

#[test]
fn electric_character_appends_text() {
    let mut ed = using_cursor(&["  if (a) {", "/*"], electric_char_language());
    ed.move_to(2, 3, false);
    ed.type_keyboard("*");
    assert_eq!(ed.line(2), "/** */");
}

#[test]
fn electric_character_appends_text_2() {
    let mut ed = using_cursor(&["  if (a) {", "  /*"], electric_char_language());
    ed.move_to(2, 5, false);
    ed.type_keyboard("*");
    assert_eq!(ed.line(2), "  /** */");
}

#[test]
fn electric_character_issue_23711_replacing_selected_text_with_brackets_fails_to_delete_old_text_with_backwards_dragged_selection()
 {
    let mut ed = using_cursor(&["{", "word"], electric_char_language());
    ed.move_to(2, 5, false);
    ed.move_to(2, 1, true);
    ed.type_keyboard("}");
    assert_eq!(ed.line(2), "}");
}

// ---------------------------------------------------------------------------
// Auto-closing pairs

#[test]
fn issue_61070_backtick_should_auto_close_after_a_word_character() {
    let mut ed = using_cursor(&["const markup = highlight"], auto_closing_language());
    assert_type(&mut ed, 1, 25, "`", "``", "auto closes ` @ (1, 25)");
}

#[test]
fn issue_132912_quotes_should_not_auto_close_if_they_are_closing_a_string() {
    let mut ed = using_cursor(&["const t2 = `something ${t1}"], auto_closing_language())
        .with_tokenizer(Tokenizer::AutoClosing);
    assert_type(&mut ed, 1, 28, "`", "`", "does not auto close ` @ (1, 28)");
}

#[test]
fn auto_closing_pairs_open_parens_default() {
    let mut ed = using_cursor(&AUTO_CLOSING_TEXT, auto_closing_language());
    check_open_paren_positions(
        &mut ed,
        &[
            "var| a| |=| [|]|;|",
            "var| b| |=| |`asd|`|;|",
            "var| c| |=| |'asd|'|;|",
            "var| d| |=| |\"asd|\"|;|",
            "var| e| |=| /*3*/|\t3|;|",
            "var| f| |=| /**| 3| */3|;|",
            "var| g| |=| (3+5|)|;|",
            "var| h| |=| {| a|:| |'value|'| |}|;|",
        ],
    );
}

#[test]
fn auto_closing_pairs_open_parens_whitespace() {
    let mut ed = using_cursor_with(&AUTO_CLOSING_TEXT, auto_closing_language(), |c| {
        c.auto_closing_brackets = AutoClosingStrategy::BeforeWhitespace;
    });
    check_open_paren_positions(
        &mut ed,
        &[
            "var| a| =| [|];|",
            "var| b| =| `asd`;|",
            "var| c| =| 'asd';|",
            "var| d| =| \"asd\";|",
            "var| e| =| /*3*/|\t3;|",
            "var| f| =| /**| 3| */3;|",
            "var| g| =| (3+5|);|",
            "var| h| =| {| a:| 'value'| |};|",
        ],
    );
}

#[test]
fn auto_closing_pairs_open_parens_disabled_enabled_open_quotes_enabled_disabled() {
    let mut ed = using_cursor_with(&["var a = [];"], auto_closing_language(), |c| {
        c.auto_closing_brackets = AutoClosingStrategy::BeforeWhitespace;
        c.auto_closing_quotes = AutoClosingStrategy::Never;
    });
    let columns = extract_auto_closing_special_columns(max_column(&ed, 1), "var| a| =| [|];|");
    for column in 1..columns.len() as u32 {
        if columns[column as usize] == ColumnType::Special1 {
            assert_type(
                &mut ed,
                1,
                column,
                "(",
                "()",
                &format!("auto closes @ (1, {column})"),
            );
        } else {
            assert_type(
                &mut ed,
                1,
                column,
                "(",
                "(",
                &format!("does not auto close @ (1, {column})"),
            );
        }
        assert_type(
            &mut ed,
            1,
            column,
            "'",
            "'",
            &format!("does not auto close @ (1, {column})"),
        );
    }

    let mut ed = using_cursor_with(&["var b = [];"], auto_closing_language(), |c| {
        c.auto_closing_brackets = AutoClosingStrategy::Never;
        c.auto_closing_quotes = AutoClosingStrategy::BeforeWhitespace;
    });
    let columns = extract_auto_closing_special_columns(max_column(&ed, 1), "var b =| [|];|");
    for column in 1..columns.len() as u32 {
        if columns[column as usize] == ColumnType::Special1 {
            assert_type(
                &mut ed,
                1,
                column,
                "'",
                "''",
                &format!("auto closes @ (1, {column})"),
            );
        } else {
            assert_type(
                &mut ed,
                1,
                column,
                "'",
                "'",
                &format!("does not auto close @ (1, {column})"),
            );
        }
        assert_type(
            &mut ed,
            1,
            column,
            "(",
            "(",
            &format!("does not auto close @ (1, {column})"),
        );
    }
}

#[test]
fn auto_closing_pairs_configurable_open_parens() {
    let mut ed = using_cursor(
        &AUTO_CLOSING_TEXT,
        auto_closing_language_with_auto_close_before("abc"),
    );
    check_open_paren_positions(
        &mut ed,
        &[
            "v|ar |a = [|];|",
            "v|ar |b = `|asd`;|",
            "v|ar |c = '|asd';|",
            "v|ar d = \"|asd\";|",
            "v|ar e = /*3*/\t3;|",
            "v|ar f = /** 3| */3;|",
            "v|ar g = (3+5|);|",
            "v|ar h = { |a: 'v|alue' |};|",
        ],
    );
}

#[test]
fn auto_closing_pairs_auto_pairing_can_be_disabled() {
    let mut ed = using_cursor_with(&AUTO_CLOSING_TEXT, auto_closing_language(), |c| {
        c.auto_closing_brackets = AutoClosingStrategy::Never;
        c.auto_closing_quotes = AutoClosingStrategy::Never;
    });
    for (i, line) in AUTO_CLOSING_TEXT.iter().enumerate() {
        let line_number = i as u32 + 1;
        let columns = extract_auto_closing_special_columns(max_column(&ed, line_number), line);
        for column in 1..columns.len() as u32 {
            assert_eq!(columns[column as usize], ColumnType::Normal);
            let message = format!("does not auto close @ ({line_number}, {column})");
            assert_type(&mut ed, line_number, column, "(", "(", &message);
            assert_type(&mut ed, line_number, column, "\"", "\"", &message);
        }
    }
}

#[test]
fn auto_closing_pairs_auto_wrapping_is_configurable() {
    let mut ed = using_cursor(&["var a = asd"], auto_closing_language());
    ed.set_selections(&[sel(1, 1, 1, 4), sel(1, 9, 1, 12)]);
    ed.type_keyboard("`");
    assert_eq!(ed.value(), "`var` a = `asd`");
    ed.type_keyboard("(");
    assert_eq!(ed.value(), "`(var)` a = `(asd)`");

    let mut ed = using_cursor_with(&["var a = asd"], auto_closing_language(), |c| {
        c.auto_surround = AutoSurroundStrategy::Never;
    });
    ed.set_selections(&[sel(1, 1, 1, 4)]);
    ed.type_keyboard("`");
    assert_eq!(ed.value(), "` a = asd");

    let mut ed = using_cursor_with(&["var a = asd"], auto_closing_language(), |c| {
        c.auto_surround = AutoSurroundStrategy::Quotes;
    });
    ed.set_selections(&[sel(1, 1, 1, 4)]);
    ed.type_keyboard("`");
    assert_eq!(ed.value(), "`var` a = asd");
    ed.type_keyboard("(");
    assert_eq!(ed.value(), "`(` a = asd");

    let mut ed = using_cursor_with(&["var a = asd"], auto_closing_language(), |c| {
        c.auto_surround = AutoSurroundStrategy::Brackets;
    });
    ed.set_selections(&[sel(1, 1, 1, 4)]);
    ed.type_keyboard("(");
    assert_eq!(ed.value(), "(var) a = asd");
    ed.type_keyboard("`");
    assert_eq!(ed.value(), "(`) a = asd");
}

#[test]
fn auto_closing_pairs_quote() {
    let mut ed = using_cursor(&AUTO_CLOSING_TEXT, auto_closing_language());
    let auto_close_positions = [
        "var a |=| [|]|;|",
        "var b |=| `asd`|;|",
        "var c |=| 'asd'|;|",
        "var d |=| \"asd\"|;|",
        "var e |=| /*3*/|\t3;|",
        "var f |=| /**| 3 */3;|",
        "var g |=| (3+5)|;|",
        "var h |=| {| a:| 'value'| |}|;|",
    ];
    for (i, annotated) in auto_close_positions.iter().enumerate() {
        let line_number = i as u32 + 1;
        let columns = extract_auto_closing_special_columns(max_column(&ed, line_number), annotated);
        for column in 1..columns.len() as u32 {
            match columns[column as usize] {
                ColumnType::Special1 => assert_type(
                    &mut ed,
                    line_number,
                    column,
                    "'",
                    "''",
                    &format!("auto closes @ ({line_number}, {column})"),
                ),
                ColumnType::Special2 => assert_type(
                    &mut ed,
                    line_number,
                    column,
                    "'",
                    "",
                    &format!("over types @ ({line_number}, {column})"),
                ),
                ColumnType::Normal => assert_type(
                    &mut ed,
                    line_number,
                    column,
                    "'",
                    "'",
                    &format!("does not auto close @ ({line_number}, {column})"),
                ),
            }
        }
    }
}

#[test]
fn auto_closing_pairs_multi_character_autoclose() {
    let mut ed = using_cursor(&[""], auto_closing_language());
    ed.set_value("begi");
    ed.set_selections(&[sel(1, 5, 1, 5)]);
    ed.type_keyboard("n");
    assert_eq!(ed.line(1), "beginend");

    ed.set_value("/*");
    ed.set_selections(&[sel(1, 3, 1, 3)]);
    ed.type_keyboard("*");
    assert_eq!(ed.line(1), "/** */");
}

#[test]
fn issue_338693_undo_after_multi_character_autoclose_first_removes_only_the_auto_closed_characters()
{
    let mut ed = using_cursor(&["begi begi"], auto_closing_language());
    let mut states: Vec<(String, Vec<String>)> = Vec::new();
    let record = |ed: &TestEditor, states: &mut Vec<(String, Vec<String>)>| {
        states.push((
            ed.line(1),
            ed.selections()
                .iter()
                .map(super::harness::fmt_sel)
                .collect(),
        ));
    };
    ed.set_selections(&[sel(1, 5, 1, 5), sel(1, 10, 1, 10)]);
    ed.type_keyboard("n");
    record(&ed, &mut states);
    ed.undo();
    record(&ed, &mut states);
    ed.undo();
    record(&ed, &mut states);

    let expected: Vec<(String, Vec<String>)> = vec![
        (
            "beginend beginend".into(),
            vec!["[1,6 -> 1,6]".into(), "[1,15 -> 1,15]".into()],
        ),
        (
            "begin begin".into(),
            vec!["[1,6 -> 1,6]".into(), "[1,12 -> 1,12]".into()],
        ),
        (
            "begi begi".into(),
            vec!["[1,5 -> 1,5]".into(), "[1,10 -> 1,10]".into()],
        ),
    ];
    assert_eq!(states, expected);
}

#[test]
fn issue_338693_undo_after_multi_character_autoclose_with_auto_indentation_first_removes_only_the_auto_closed_characters()
 {
    let language = LanguageConfiguration {
        auto_closing_pairs: vec![pair("begin", "end")],
        indentation_rules: Some(rules(re(r"^\s*procedure\b"), re(r"^\s*begin\b"))),
        ..Default::default()
    };
    let mut ed = using_cursor(&["\tfoo", "\tbegi"], language);
    let mut states: Vec<(String, Vec<String>)> = Vec::new();
    let record = |ed: &TestEditor, states: &mut Vec<(String, Vec<String>)>| {
        states.push((
            ed.line(2),
            ed.selections()
                .iter()
                .map(super::harness::fmt_sel)
                .collect(),
        ));
    };
    ed.set_selections(&[sel(2, 6, 2, 6)]);
    ed.type_keyboard("n");
    record(&ed, &mut states);
    ed.undo();
    record(&ed, &mut states);
    ed.undo();
    record(&ed, &mut states);

    let expected: Vec<(String, Vec<String>)> = vec![
        ("beginend".into(), vec!["[2,6 -> 2,6]".into()]),
        ("begin".into(), vec!["[2,6 -> 2,6]".into()]),
        ("\tbegi".into(), vec!["[2,6 -> 2,6]".into()]),
    ];
    assert_eq!(states, expected);
}

#[test]
fn auto_closing_pairs_doc_comments_can_be_turned_off() {
    let mut ed = using_cursor_with(&[""], auto_closing_language(), |c| {
        c.auto_closing_comments = AutoClosingStrategy::Never;
    });
    ed.set_value("/*");
    ed.set_selections(&[sel(1, 3, 1, 3)]);
    ed.type_keyboard("*");
    assert_eq!(ed.line(1), "/**");
}

#[test]
fn issue_72177_multi_character_autoclose_with_conflicting_patterns() {
    let language = LanguageConfiguration {
        auto_closing_pairs: vec![
            pair("(", ")"),
            pair("(*", "*)"),
            pair("<@", "@>"),
            pair("<@@", "@@>"),
        ],
        ..Default::default()
    };
    let mut ed = using_cursor(&[""], language);
    ed.type_keyboard("(");
    assert_eq!(ed.line(1), "()");
    ed.type_keyboard("*");
    assert_eq!(
        ed.line(1),
        "(**)",
        "doesn't add entire close when already closed substring is there"
    );

    ed.set_value("(");
    ed.set_selections(&[sel(1, 2, 1, 2)]);
    ed.type_keyboard("*");
    assert_eq!(
        ed.line(1),
        "(**)",
        "does add entire close if not already there"
    );

    ed.set_value("");
    ed.type_keyboard("<@");
    assert_eq!(ed.line(1), "<@@>");
    ed.type_keyboard("@");
    assert_eq!(
        ed.line(1),
        "<@@@@>",
        "autocloses when before multi-character closing brace"
    );
    ed.type_keyboard("(");
    assert_eq!(
        ed.line(1),
        "<@@()@@>",
        "autocloses when before multi-character closing brace"
    );
}

#[test]
fn issue_55314_do_not_auto_close_when_ending_with_open() {
    let language = LanguageConfiguration {
        auto_closing_pairs: vec![
            pair("{", "}"),
            pair("[", "]"),
            pair("(", ")"),
            pair_not_in("'", "'", &[NotIn::String, NotIn::Comment]),
            pair_not_in("\"", "\"", &[NotIn::String]),
            pair_not_in("B\"", "\"", &[NotIn::String, NotIn::Comment]),
            pair_not_in("`", "`", &[NotIn::String, NotIn::Comment]),
            pair_not_in("/**", " */", &[NotIn::String]),
        ],
        ..Default::default()
    };
    let mut ed = using_cursor(
        &["little goat", "little LAMB", "little sheep", "Big LAMB"],
        language,
    );
    let message = "does not double quote when ending with open";
    assert_type(&mut ed, 1, 4, "\"", "\"", message);
    assert_type(&mut ed, 2, 4, "\"", "\"", message);
    assert_type(&mut ed, 3, 4, "\"", "\"", message);
    assert_type(&mut ed, 4, 2, "\"", "\"", message);
    assert_type(&mut ed, 4, 3, "\"", "\"", message);
}

#[test]
fn issue_27937_trying_to_add_an_item_to_the_front_of_a_list_is_cumbersome() {
    let mut ed = using_cursor(&["var arr = [\"b\", \"c\"];"], auto_closing_language());
    assert_type(
        &mut ed,
        1,
        12,
        "\"",
        "\"",
        "does not over type and will not auto close",
    );
}

#[test]
fn issue_6841_auto_closing_brackets_should_balance_brackets() {
    let language = LanguageConfiguration {
        brackets: standard_brackets(),
        auto_closing_pairs: vec![pair("{", "}"), pair("[", "]"), pair("(", ")")],
        ..Default::default()
    };
    let cases: [(&str, (u32, u32), &str, &str); 10] = [
        ("\n}", (1, 1), "{", "{\n}"),
        ("", (1, 1), "{", "{}"),
        ("{}", (1, 2), "{", "{{}}"),
        ("{\n", (2, 1), "{", "{\n{}"),
        ("\n]", (1, 1), "{", "{}\n]"),
        (")\n", (2, 1), "(", ")\n()"),
        (
            "function foo() {\n\n}",
            (2, 1),
            "{",
            "function foo() {\n{}\n}",
        ),
        ("someFunction);", (1, 13), "(", "someFunction();"),
        ("someFunction;", (1, 13), "(", "someFunction();"),
        (
            "someFunctionsomeParam);",
            (1, 13),
            "(",
            "someFunction(someParam);",
        ),
    ];
    let mut ed = using_cursor(&[""], language.clone());
    let actual: Vec<String> = cases
        .iter()
        .map(|(text, (line, column), typed, _)| {
            ed.set_value(text);
            ed.set_selections(&[sel(*line, *column, *line, *column)]);
            ed.type_keyboard(typed);
            ed.value()
        })
        .collect();
    let expected: Vec<String> = cases.iter().map(|c| c.3.to_string()).collect();
    assert_eq!(actual, expected);

    let mut ed = using_cursor(&["", "}", ""], language.clone());
    ed.set_selections(&[sel(1, 1, 1, 1), sel(3, 1, 3, 1)]);
    ed.type_keyboard("{");
    assert_eq!(ed.value(), "{\n}\n{");

    let mut ed = using_cursor_with(&["", ")"], language, |c| {
        c.auto_closing_brackets = AutoClosingStrategy::Always;
    });
    ed.type_keyboard("(");
    assert_eq!(ed.value(), "()\n)");

    let language = LanguageConfiguration {
        brackets: brackets(&[("{", "}")]),
        ..auto_closing_language()
    };
    let mut ed =
        using_cursor(&["", "\"}\""], language.clone()).with_tokenizer(Tokenizer::AutoClosing);
    ed.type_keyboard("{");
    assert_eq!(ed.value(), "{}\n\"}\"");

    let mut ed =
        using_cursor(&["", "\"}\"", "// }"], language).with_tokenizer(Tokenizer::AutoClosing);
    ed.type_keyboard("{");
    assert_eq!(ed.value(), "{}\n\"}\"\n// }");
}

#[test]
fn issue_25658_do_not_auto_close_single_double_quotes_after_word_characters() {
    let mut ed = using_cursor(&[""], auto_closing_language());
    let type_characters = |ed: &mut TestEditor, chars: &str| {
        for c in chars.chars() {
            ed.type_keyboard(&c.to_string());
        }
    };

    // First gif
    type_characters(&mut ed, "teste1 = teste' ok");
    assert_eq!(ed.line(1), "teste1 = teste' ok");

    ed.set_selections(&[sel(1, 1000, 1, 1000)]);
    type_characters(&mut ed, "\n");
    type_characters(&mut ed, "teste2 = teste 'ok");
    assert_eq!(ed.line(2), "teste2 = teste 'ok'");

    ed.set_selections(&[sel(2, 1000, 2, 1000)]);
    type_characters(&mut ed, "\n");
    type_characters(&mut ed, "teste3 = teste\" ok");
    assert_eq!(ed.line(3), "teste3 = teste\" ok");

    ed.set_selections(&[sel(3, 1000, 3, 1000)]);
    type_characters(&mut ed, "\n");
    type_characters(&mut ed, "teste4 = teste \"ok");
    assert_eq!(ed.line(4), "teste4 = teste \"ok\"");

    // Second gif
    ed.set_selections(&[sel(4, 1000, 4, 1000)]);
    type_characters(&mut ed, "\n");
    type_characters(&mut ed, "teste '");
    assert_eq!(ed.line(5), "teste ''");

    ed.set_selections(&[sel(5, 1000, 5, 1000)]);
    type_characters(&mut ed, "\n");
    type_characters(&mut ed, "teste \"");
    assert_eq!(ed.line(6), "teste \"\"");

    ed.set_selections(&[sel(6, 1000, 6, 1000)]);
    type_characters(&mut ed, "\n");
    type_characters(&mut ed, "teste'");
    assert_eq!(ed.line(7), "teste'");

    ed.set_selections(&[sel(7, 1000, 7, 1000)]);
    type_characters(&mut ed, "\n");
    type_characters(&mut ed, "teste\"");
    assert_eq!(ed.line(8), "teste\"");
}

#[test]
fn issue_37315_overtypes_only_those_characters_that_it_inserted() {
    let mut ed = using_cursor(&["", "y=();"], auto_closing_language());
    ed.assert_cursor(1, 1);

    ed.type_keyboard("x=(");
    assert_eq!(ed.line(1), "x=()");

    ed.type_keyboard("asd");
    assert_eq!(ed.line(1), "x=(asd)");

    // overtype!
    ed.type_keyboard(")");
    assert_eq!(ed.line(1), "x=(asd)");

    // do not overtype!
    ed.set_selections(&[sel(2, 4, 2, 4)]);
    ed.type_keyboard(")");
    assert_eq!(ed.line(2), "y=());");
}

#[test]
fn issue_37315_stops_overtyping_once_cursor_leaves_area() {
    let mut ed = using_cursor(&["", "y=();"], auto_closing_language());
    ed.assert_cursor(1, 1);

    ed.type_keyboard("x=(");
    assert_eq!(ed.line(1), "x=()");

    ed.set_selections(&[sel(1, 5, 1, 5)]);
    ed.type_keyboard(")");
    assert_eq!(ed.line(1), "x=())");
}

#[test]
fn issue_37315_it_overtypes_only_once() {
    let mut ed = using_cursor(&["", "y=();"], auto_closing_language());
    ed.assert_cursor(1, 1);

    ed.type_keyboard("x=(");
    assert_eq!(ed.line(1), "x=()");

    ed.type_keyboard(")");
    assert_eq!(ed.line(1), "x=()");

    ed.set_selections(&[sel(1, 4, 1, 4)]);
    ed.type_keyboard(")");
    assert_eq!(ed.line(1), "x=())");
}

#[test]
fn issue_37315_it_can_remember_multiple_auto_closed_instances() {
    let mut ed = using_cursor(&["", "y=();"], auto_closing_language());
    ed.assert_cursor(1, 1);

    ed.type_keyboard("x=(");
    assert_eq!(ed.line(1), "x=()");

    ed.type_keyboard("(");
    assert_eq!(ed.line(1), "x=(())");

    ed.type_keyboard(")");
    assert_eq!(ed.line(1), "x=(())");

    ed.type_keyboard(")");
    assert_eq!(ed.line(1), "x=(())");
}

#[test]
fn issue_118270_auto_closing_deletes_only_those_characters_that_it_inserted() {
    let mut ed = using_cursor(&["", "y=();"], auto_closing_language());
    ed.assert_cursor(1, 1);

    ed.type_keyboard("x=(");
    assert_eq!(ed.line(1), "x=()");

    ed.type_keyboard("asd");
    assert_eq!(ed.line(1), "x=(asd)");

    ed.delete_left();
    ed.delete_left();
    ed.delete_left();
    assert_eq!(ed.line(1), "x=()");

    // delete closing char!
    ed.delete_left();
    assert_eq!(ed.line(1), "x=");

    // do not delete closing char!
    ed.set_selections(&[sel(2, 4, 2, 4)]);
    ed.delete_left();
    assert_eq!(ed.line(2), "y=);");
}

#[test]
fn issue_78527_does_not_close_quote_on_odd_count() {
    let mut ed = using_cursor(&["std::cout << '\"' << entryMap"], auto_closing_language());
    ed.set_selections(&[sel(1, 29, 1, 29)]);

    ed.type_keyboard("[");
    assert_eq!(ed.line(1), "std::cout << '\"' << entryMap[]");

    ed.type_keyboard("\"");
    assert_eq!(ed.line(1), "std::cout << '\"' << entryMap[\"\"]");

    ed.type_keyboard("a");
    assert_eq!(ed.line(1), "std::cout << '\"' << entryMap[\"a\"]");

    ed.type_keyboard("\"");
    assert_eq!(ed.line(1), "std::cout << '\"' << entryMap[\"a\"]");

    ed.type_keyboard("]");
    assert_eq!(ed.line(1), "std::cout << '\"' << entryMap[\"a\"]");
}

#[test]
fn issue_85983_editor_auto_closing_brackets_before_whitespace_is_incorrect_for_python() {
    let mut pairs = vec![
        pair("{", "}"),
        pair("[", "]"),
        pair("(", ")"),
        pair_not_in("\"", "\"", &[NotIn::String]),
    ];
    for prefix in ["r", "R", "u", "U", "f", "F", "b", "B"] {
        pairs.push(pair_not_in(
            &format!("{prefix}\""),
            "\"",
            &[NotIn::String, NotIn::Comment],
        ));
    }
    pairs.push(pair_not_in("'", "'", &[NotIn::String, NotIn::Comment]));
    for prefix in ["r", "R", "u", "U", "f", "F", "b", "B"] {
        pairs.push(pair_not_in(
            &format!("{prefix}'"),
            "'",
            &[NotIn::String, NotIn::Comment],
        ));
    }
    pairs.push(pair_not_in("`", "`", &[NotIn::String]));
    let language = LanguageConfiguration {
        auto_closing_pairs: pairs,
        ..Default::default()
    };
    let mut ed = using_cursor_with(&["foo'hello'"], language, |c| {
        c.auto_closing_brackets = AutoClosingStrategy::BeforeWhitespace;
    });
    assert_type(&mut ed, 1, 4, "(", "(", "does not auto close @ (1, 4)");
}

#[test]
fn issue_78833_add_config_to_use_old_brackets_quotes_overtyping() {
    let mut ed = using_cursor_with(&["", "y=();"], auto_closing_language(), |c| {
        c.auto_closing_overtype = AutoClosingEditStrategy::Always;
    });
    ed.assert_cursor(1, 1);

    ed.type_keyboard("x=(");
    assert_eq!(ed.line(1), "x=()");

    ed.type_keyboard(")");
    assert_eq!(ed.line(1), "x=()");

    ed.set_selections(&[sel(1, 4, 1, 4)]);
    ed.type_keyboard(")");
    assert_eq!(ed.line(1), "x=()");

    ed.set_selections(&[sel(2, 4, 2, 4)]);
    ed.type_keyboard(")");
    assert_eq!(ed.line(2), "y=();");
}

#[test]
fn issue_53357_over_typing_ignores_characters_after_backslash() {
    let mut ed = using_cursor(&["console.log();"], auto_closing_language());
    ed.set_selections(&[sel(1, 13, 1, 13)]);

    ed.type_keyboard("'");
    assert_eq!(ed.value(), "console.log('');");

    ed.type_keyboard("it");
    assert_eq!(ed.value(), "console.log('it');");

    ed.type_keyboard("\\");
    assert_eq!(ed.value(), "console.log('it\\');");

    ed.type_keyboard("'");
    assert_eq!(ed.value(), "console.log('it\\'');");
}

#[test]
fn issue_84998_overtyping_brackets_doesnt_work_after_backslash() {
    let mut ed = using_cursor(&[""], auto_closing_language());
    ed.set_selections(&[sel(1, 1, 1, 1)]);

    ed.type_keyboard("\\");
    assert_eq!(ed.value(), "\\");

    ed.type_keyboard("(");
    assert_eq!(ed.value(), "\\()");

    ed.type_keyboard("abc");
    assert_eq!(ed.value(), "\\(abc)");

    ed.type_keyboard("\\");
    assert_eq!(ed.value(), "\\(abc\\)");

    ed.type_keyboard(")");
    assert_eq!(ed.value(), "\\(abc\\)");
}

#[test]
fn issue_20891_all_cursors_should_do_the_same_thing() {
    let mut ed = using_cursor(&["var a = asd"], auto_closing_language());
    ed.set_selections(&[sel(1, 9, 1, 9), sel(1, 12, 1, 12)]);
    ed.type_keyboard("`");
    assert_eq!(ed.value(), "var a = `asd`");
}

#[test]
fn issue_41825_special_handling_of_quotes_in_surrounding_pairs() {
    let language = LanguageConfiguration {
        surrounding_pairs: vec![surround("\"", "\""), surround("'", "'")],
        ..Default::default()
    };
    let mut ed = using_cursor(&["var x = 'hi';"], language);
    ed.set_selections(&[sel(1, 9, 1, 10), sel(1, 12, 1, 13)]);
    ed.type_keyboard("\"");
    assert_eq!(ed.value(), "var x = \"hi\";", "assert1");

    ed.set_selections(&[sel(1, 9, 1, 10), sel(1, 12, 1, 13)]);
    ed.type_keyboard("'");
    assert_eq!(ed.value(), "var x = 'hi';", "assert2");
}

#[test]
fn all_cursors_should_do_the_same_thing_when_deleting_left() {
    let mut ed = using_cursor(&["var a = ()"], auto_closing_language());
    ed.set_selections(&[sel(1, 4, 1, 4), sel(1, 10, 1, 10)]);
    ed.delete_left();
    assert_eq!(ed.value(), "va a = )");
}

#[test]
fn auto_closed_ranges_follow_edits_before_them() {
    // Not in Monaco: typing on an earlier line keeps the auto-closed
    // character tracked.
    let mut ed = using_cursor(&["", "x"], auto_closing_language());
    ed.set_selections(&[sel(2, 2, 2, 2)]);
    ed.type_keyboard("(");
    assert_eq!(ed.line(2), "x()");
    let ranges = ed.cursors.auto_closed_ranges().closing_ranges();
    assert_eq!(ranges, vec![pos(2, 3)..pos(2, 4)]);
    ed.type_keyboard("a");
    assert_eq!(
        ed.cursors.auto_closed_ranges().closing_ranges(),
        vec![pos(2, 4)..pos(2, 5)]
    );
    ed.type_keyboard(")");
    assert_eq!(ed.line(2), "x(a)");
}
