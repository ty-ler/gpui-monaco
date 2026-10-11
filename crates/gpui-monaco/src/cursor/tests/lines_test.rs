//! Whole-line commands. Ports parts of Monaco's
//! `src/vs/editor/contrib/linesOperations/test/browser/linesOperations.test.ts`
//! (MIT): `DeleteAllLeftAction`, `InsertLineBeforeAction`,
//! `InsertLineAfterAction`, `Bug 18276`, `issue #80736`, `Indenting on empty
//! line should move cursor`, `issue #62112` and the delete lines tests. The
//! copy/move lines command tests are generated into `lines_command_cases.rs`;
//! the move lines tests with indentation or onEnter rules (suites `Move
//! Lines Command honors Indentation Rules` and `honors onEnter Rules` of
//! `moveLinesCommand.test.ts`) are ported here.
//!
//! Not ported from that file: sort, delete duplicate lines, join lines,
//! reverse lines, transpose, case transforms, delete all right, duplicate
//! selection (commands not ported).

use super::harness::{TestEditor, sel};
use super::language_configs::{brackets, re, rules};
use crate::cursor::{
    self, Selection, copy_lines_down, copy_lines_up, delete_all_left, delete_lines,
    insert_line_above, insert_line_below, move_lines_down, move_lines_down_with_language,
    move_lines_up, move_lines_up_with_language,
};
use syntax::{IndentationRules, LanguageConfiguration};

#[derive(Clone, Copy, Debug)]
pub enum LinesCommand {
    CopyDown,
    CopyUp,
    MoveDown,
    MoveUp,
}

/// Monaco `testCommand` for a single-cursor lines command.
#[track_caller]
pub fn test_lines_command(
    command: LinesCommand,
    lines: &[&str],
    selection: Selection,
    expected_lines: &[&str],
    expected_selection: Selection,
) {
    let mut editor = TestEditor::lines(lines);
    editor.set_selections(&[selection]);
    editor.run(|c, b, cur| match command {
        LinesCommand::CopyDown => copy_lines_down(c, b, cur),
        LinesCommand::CopyUp => copy_lines_up(c, b, cur),
        LinesCommand::MoveDown => move_lines_down(c, b, cur),
        LinesCommand::MoveUp => move_lines_up(c, b, cur),
    });
    assert_eq!(editor.value(), expected_lines.join("\n"));
    editor.assert_selection(expected_selection);
}

fn delete_all_left_(editor: &mut TestEditor) {
    editor.run(delete_all_left);
}

#[test]
fn delete_all_left_should_delete_to_the_left_of_the_cursor() {
    let mut editor = TestEditor::lines(&["one", "two", "three"]);
    editor.set_selection(sel(1, 2, 1, 2));
    delete_all_left_(&mut editor);
    assert_eq!(editor.line(1), "ne");

    editor.set_selections(&[sel(2, 2, 2, 2), sel(3, 2, 3, 2)]);
    delete_all_left_(&mut editor);
    assert_eq!(editor.line(2), "wo");
    assert_eq!(editor.line(3), "hree");
}

#[test]
fn delete_all_left_should_jump_to_the_previous_line_when_on_first_column() {
    let mut editor = TestEditor::lines(&["one", "two", "three"]);
    editor.set_selection(sel(2, 1, 2, 1));
    delete_all_left_(&mut editor);
    assert_eq!(editor.line(1), "onetwo");

    editor.set_selections(&[sel(1, 1, 1, 1), sel(2, 1, 2, 1)]);
    delete_all_left_(&mut editor);
    assert_eq!(editor.line(1), "onetwothree");
    assert_eq!(editor.line_count(), 1);

    editor.set_selection(sel(1, 1, 1, 1));
    delete_all_left_(&mut editor);
    assert_eq!(editor.line(1), "onetwothree");
}

#[test]
fn delete_all_left_should_keep_deleting_lines_in_multi_cursor_mode() {
    let mut editor = TestEditor::lines(&[
        "hi my name is Carlos Matos",
        "BCC",
        "waso waso waso",
        "my wife doesnt believe in me",
        "nonononono",
        "bitconneeeect",
    ]);
    let before_second_waso_selection = sel(3, 5, 3, 5);
    let end_of_bcc_selection = sel(2, 4, 2, 4);
    let end_of_nonono = sel(5, 11, 5, 11);
    editor.set_selections(&[
        before_second_waso_selection,
        end_of_bcc_selection,
        end_of_nonono,
    ]);

    delete_all_left_(&mut editor);
    assert_eq!(editor.line(2), "");
    assert_eq!(editor.line(3), " waso waso");
    assert_eq!(editor.line(5), "");
    editor.assert_selections(&[sel(3, 1, 3, 1), sel(2, 1, 2, 1), sel(5, 1, 5, 1)]);

    delete_all_left_(&mut editor);
    assert_eq!(editor.line(1), "hi my name is Carlos Matos waso waso");
    editor.assert_selections(&[sel(1, 27, 1, 27), sel(2, 29, 2, 29)]);
}

#[test]
fn delete_all_left_should_work_in_multi_cursor_mode() {
    let mut editor = TestEditor::lines(&[
        "hello",
        "world",
        "hello world",
        "hello",
        "bonjour",
        "hola",
        "world",
        "hello world",
    ]);
    editor.set_selections(&[sel(1, 2, 1, 2), sel(1, 4, 1, 4)]);
    delete_all_left_(&mut editor);
    assert_eq!(editor.line(1), "lo");

    editor.set_selections(&[sel(2, 2, 2, 2), sel(2, 4, 2, 5)]);
    delete_all_left_(&mut editor);
    assert_eq!(editor.line(2), "d");

    editor.set_selections(&[sel(3, 2, 3, 5), sel(3, 7, 3, 7)]);
    delete_all_left_(&mut editor);
    assert_eq!(editor.line(3), "world");

    editor.set_selections(&[sel(4, 3, 4, 3), sel(4, 5, 5, 4)]);
    delete_all_left_(&mut editor);
    assert_eq!(editor.line(4), "jour");

    editor.set_selections(&[sel(5, 3, 6, 3), sel(6, 5, 7, 5), sel(7, 7, 7, 7)]);
    delete_all_left_(&mut editor);
    assert_eq!(editor.line(5), "world");
}

#[test]
fn delete_all_left_issue_36234_should_push_undo_stop() {
    let mut editor = TestEditor::lines(&["one", "two", "three"]);
    editor.set_selection(sel(1, 1, 1, 1));

    editor.type_raw("Typing some text here on line ");
    assert_eq!(editor.line(1), "Typing some text here on line one");
    editor.assert_selection(sel(1, 31, 1, 31));

    delete_all_left_(&mut editor);
    assert_eq!(editor.line(1), "one");
    editor.assert_selection(sel(1, 1, 1, 1));

    editor.undo();
    assert_eq!(editor.line(1), "Typing some text here on line one");
    editor.assert_selection(sel(1, 31, 1, 31));
}

const INSERT_LINE_TEXT: [&str; 3] = ["First line", "Second line", "Third line"];

#[test]
fn insert_line_before_action() {
    let check = |line: u32, column: u32, expected_selection: Selection, expected: [&str; 4]| {
        let mut editor = TestEditor::lines(&INSERT_LINE_TEXT);
        editor.set_position(line, column);
        editor.run(insert_line_above);
        editor.assert_selection(expected_selection);
        for (i, l) in expected.iter().enumerate() {
            assert_eq!(editor.line(i as u32 + 1), *l);
        }
    };
    check(
        1,
        3,
        sel(1, 1, 1, 1),
        ["", "First line", "Second line", "Third line"],
    );
    check(
        2,
        3,
        sel(2, 1, 2, 1),
        ["First line", "", "Second line", "Third line"],
    );
    check(
        3,
        3,
        sel(3, 1, 3, 1),
        ["First line", "Second line", "", "Third line"],
    );
}

#[test]
fn insert_line_after_action() {
    let check = |line: u32, column: u32, expected_selection: Selection, expected: [&str; 4]| {
        let mut editor = TestEditor::lines(&INSERT_LINE_TEXT);
        editor.set_position(line, column);
        editor.run(insert_line_below);
        editor.assert_selection(expected_selection);
        for (i, l) in expected.iter().enumerate() {
            assert_eq!(editor.line(i as u32 + 1), *l);
        }
    };
    check(
        1,
        3,
        sel(2, 1, 2, 1),
        ["First line", "", "Second line", "Third line"],
    );
    check(
        2,
        3,
        sel(3, 1, 3, 1),
        ["First line", "Second line", "", "Third line"],
    );
    check(
        3,
        3,
        sel(4, 1, 4, 1),
        ["First line", "Second line", "Third line", ""],
    );
}

#[test]
fn bug_18276_indentation_broken_when_selection_is_empty() {
    let mut editor = TestEditor::lines(&["function baz() {"]);
    editor.config.insert_spaces = false;
    editor.set_position(1, 2);

    editor.indent_lines();
    assert_eq!(editor.line(1), "\tfunction baz() {");
    editor.assert_selection(sel(1, 3, 1, 3));

    editor.tab();
    assert_eq!(editor.line(1), "\tf\tunction baz() {");
}

#[test]
fn issue_80736_indenting_at_the_start_of_a_line_does_not_select_the_added_indent() {
    let mut editor = TestEditor::lines(&["Some text"]);
    editor.config.insert_spaces = false;
    editor.set_position(1, 1);

    editor.indent_lines();
    assert_eq!(editor.line(1), "\tSome text");
    editor.assert_selection(sel(1, 2, 1, 2));
}

#[test]
fn indenting_on_empty_line_should_move_cursor() {
    let mut editor = TestEditor::lines(&[""]);
    editor.config.use_tab_stops = false;
    editor.set_position(1, 1);

    editor.indent_lines();
    assert_eq!(editor.line(1), "    ");
    editor.assert_selection(sel(1, 5, 1, 5));
}

#[test]
fn issue_62112_delete_line_does_not_work_properly_when_multiple_cursors_are_on_line() {
    let mut editor = TestEditor::lines(&["a", "foo boo", "too", "c"]);
    editor.set_selections(&[sel(2, 4, 2, 4), sel(2, 8, 2, 8), sel(3, 4, 3, 4)]);
    editor.run(delete_lines);
    assert_eq!(editor.value(), "a\nc");
}

#[track_caller]
fn test_delete_lines_command(
    initial: &[&str],
    selections: &[Selection],
    resulting: &[&str],
    resulting_selections: &[Selection],
) {
    let mut editor = TestEditor::lines(initial);
    editor.set_selections(selections);
    editor.run(delete_lines);
    assert_eq!(editor.value(), resulting.join("\n"));
    editor.assert_selections(resulting_selections);
}

const FIVE_LINES: [&str; 5] = ["first", "second line", "third line", "fourth line", "fifth"];

#[test]
fn delete_lines_empty_selection_in_middle_of_lines() {
    test_delete_lines_command(
        &FIVE_LINES,
        &[sel(2, 3, 2, 3)],
        &["first", "third line", "fourth line", "fifth"],
        &[sel(2, 3, 2, 3)],
    );
}

#[test]
fn delete_lines_empty_selection_at_top_of_lines() {
    test_delete_lines_command(
        &FIVE_LINES,
        &[sel(1, 5, 1, 5)],
        &["second line", "third line", "fourth line", "fifth"],
        &[sel(1, 5, 1, 5)],
    );
}

#[test]
fn delete_lines_empty_selection_at_end_of_lines() {
    test_delete_lines_command(
        &FIVE_LINES,
        &[sel(5, 2, 5, 2)],
        &["first", "second line", "third line", "fourth line"],
        &[sel(4, 2, 4, 2)],
    );
}

#[test]
fn delete_lines_with_selection_in_middle_of_lines() {
    test_delete_lines_command(
        &FIVE_LINES,
        &[sel(3, 3, 2, 2)],
        &["first", "fourth line", "fifth"],
        &[sel(2, 2, 2, 2)],
    );
}

#[test]
fn delete_lines_with_selection_at_top_of_lines() {
    test_delete_lines_command(
        &FIVE_LINES,
        &[sel(1, 4, 1, 5)],
        &["second line", "third line", "fourth line", "fifth"],
        &[sel(1, 5, 1, 5)],
    );
}

#[test]
fn delete_lines_with_selection_at_end_of_lines() {
    test_delete_lines_command(
        &FIVE_LINES,
        &[sel(5, 1, 5, 2)],
        &["first", "second line", "third line", "fourth line"],
        &[sel(4, 2, 4, 2)],
    );
}

#[test]
fn delete_lines_with_full_line_selection_in_middle_of_lines() {
    test_delete_lines_command(
        &FIVE_LINES,
        &[sel(4, 1, 2, 1)],
        &["first", "fourth line", "fifth"],
        &[sel(2, 1, 2, 1)],
    );
}

#[test]
fn delete_lines_with_full_line_selection_at_top_of_lines() {
    test_delete_lines_command(
        &FIVE_LINES,
        &[sel(2, 1, 1, 5)],
        &["second line", "third line", "fourth line", "fifth"],
        &[sel(1, 5, 1, 5)],
    );
}

#[test]
fn delete_lines_with_full_line_selection_at_end_of_lines() {
    test_delete_lines_command(
        &FIVE_LINES,
        &[sel(4, 1, 5, 2)],
        &["first", "second line", "third line"],
        &[sel(3, 2, 3, 2)],
    );
}

#[test]
fn delete_lines_multicursor_1() {
    test_delete_lines_command(
        &[
            "class P {",
            "",
            "    getA() {",
            "        if (true) {",
            "            return \"a\";",
            "        }",
            "    }",
            "",
            "    getB() {",
            "        if (true) {",
            "            return \"b\";",
            "        }",
            "    }",
            "",
            "    getC() {",
            "        if (true) {",
            "            return \"c\";",
            "        }",
            "    }",
            "}",
        ],
        &[sel(4, 1, 5, 1), sel(10, 1, 11, 1), sel(16, 1, 17, 1)],
        &[
            "class P {",
            "",
            "    getA() {",
            "            return \"a\";",
            "        }",
            "    }",
            "",
            "    getB() {",
            "            return \"b\";",
            "        }",
            "    }",
            "",
            "    getC() {",
            "            return \"c\";",
            "        }",
            "    }",
            "}",
        ],
        &[sel(4, 1, 4, 1), sel(9, 1, 9, 1), sel(14, 1, 14, 1)],
    );
}

// ---------------------------------------------------------------------------
// moveLinesCommand.test.ts: suites `Move Lines Command honors Indentation
// Rules` and `Move Lines Command honors onEnter Rules` (`autoIndent: full`).

/// Monaco `testMoveLinesUpOrDownWithIndentCommand`.
#[track_caller]
fn test_move_lines_with_indent(
    down: bool,
    language: Option<LanguageConfiguration>,
    lines: &[&str],
    selection: Selection,
    expected_lines: &[&str],
    expected_selection: Selection,
) {
    let mut editor = TestEditor::lines(lines);
    if let Some(language) = language {
        editor = editor.with_language(language);
    }
    editor.set_selections(&[selection]);
    let op = editor.with_lang(|lang| {
        if down {
            move_lines_down_with_language(&editor.config, &editor.buffer, &editor.cursors, lang)
        } else {
            move_lines_up_with_language(&editor.config, &editor.buffer, &editor.cursors, lang)
        }
    });
    editor.apply(op);
    assert_eq!(editor.value(), expected_lines.join("\n"));
    editor.assert_selection(expected_selection);
}

/// The `IndentRulesMode` of `moveLinesCommand.test.ts`.
fn move_lines_indent_rules_mode() -> LanguageConfiguration {
    LanguageConfiguration {
        indentation_rules: Some(IndentationRules {
            decrease_indent_pattern: re(
                r"^\s*((?!\S.*\/[*]).*[*]\/\s*)?[})\]]|^\s*(case\b.*|default):\s*(\/\/.*|\/[*].*[*]\/\s*)?$",
            ),
            increase_indent_pattern: re(
                r#"(\{[^}"'`]*|\([^)"']*|\[[^\]"']*|^\s*(\{\}|\(\)|\[\]|(case\b.*|default):))\s*(\/\/.*|\/[*].*[*]\/\s*)?$"#,
            ),
            indent_next_line_pattern: Some(re(
                r"^\s*(for|while|if|else)\b(?!.*[;{}]\s*(\/\/.*|\/[*].*[*]\/\s*)?$)",
            )),
            un_indented_line_pattern: Some(re(
                r#"^(?!.*([;{}]|\S:)\s*(\/\/.*|\/[*].*[*]\/\s*)?$)(?!.*(\{[^}"']*|\([^)"']*|\[[^\]"']*|^\s*(\{\}|\(\)|\[\]|(case\b.*|default):))\s*(\/\/.*|\/[*].*[*]\/\s*)?$)(?!^\s*((?!\S.*\/[*]).*[*]\/\s*)?[})\]]|^\s*(case\b.*|default):\s*(\/\/.*|\/[*].*[*]\/\s*)?$)(?!^\s*(for|while|if|else)\b(?!.*[;{}]\s*(\/\/.*|\/[*].*[*]\/\s*)?$))"#,
            )),
        }),
        ..Default::default()
    }
}

/// The `EnterRulesMode` of `moveLinesCommand.test.ts`.
fn move_lines_enter_rules_mode() -> LanguageConfiguration {
    LanguageConfiguration {
        indentation_rules: Some(rules(re(r"^\s*\]$"), re(r"^\s*\[$"))),
        brackets: Some(brackets(&[("{", "}")])),
        ..Default::default()
    }
}

#[test]
fn move_lines_first_line_indentation_adjust_to_0() {
    test_move_lines_with_indent(
        false,
        Some(move_lines_indent_rules_mode()),
        &["class X {", "\tz = 2", "}"],
        sel(2, 1, 2, 1),
        &["z = 2", "class X {", "}"],
        sel(1, 1, 1, 1),
    );
}

#[test]
fn move_lines_across_block() {
    test_move_lines_with_indent(
        true,
        Some(move_lines_indent_rules_mode()),
        &[
            "const value = 2;",
            "const standardLanguageDescriptions = [",
            "    {",
            "        diagnosticSource: 'js',",
            "    }",
            "];",
        ],
        sel(1, 1, 1, 1),
        &[
            "const standardLanguageDescriptions = [",
            "    const value = 2;",
            "    {",
            "        diagnosticSource: 'js',",
            "    }",
            "];",
        ],
        sel(2, 5, 2, 5),
    );
}

#[test]
fn move_line_should_still_work_as_before_if_there_is_no_indentation_rules() {
    test_move_lines_with_indent(
        false,
        None,
        &[
            "if (true) {",
            "    var task = new Task(() => {",
            "        var work = 1234;",
            "    });",
            "}",
        ],
        sel(3, 1, 3, 1),
        &[
            "if (true) {",
            "        var work = 1234;",
            "    var task = new Task(() => {",
            "    });",
            "}",
        ],
        sel(2, 1, 2, 1),
    );
}

#[test]
fn issue_54829_move_block_across_block() {
    test_move_lines_with_indent(
        true,
        Some(move_lines_enter_rules_mode()),
        &[
            "if (true) {",
            "    if (false) {",
            "        if (1) {",
            "            console.log('b');",
            "        }",
            "        console.log('a');",
            "    }",
            "}",
        ],
        sel(3, 9, 5, 10),
        &[
            "if (true) {",
            "    if (false) {",
            "        console.log('a');",
            "        if (1) {",
            "            console.log('b');",
            "        }",
            "    }",
            "}",
        ],
        sel(4, 9, 6, 10),
    );
}

// Not in Monaco's tests: `autoIndent` below `full` only moves the text.
#[test]
fn move_lines_keeps_indentation_below_auto_indent_full() {
    let mut editor = TestEditor::lines(&["class X {", "\tz = 2", "}"])
        .with_language(move_lines_indent_rules_mode());
    editor.config.auto_indent = cursor::AutoIndent::Advanced;
    editor.set_selections(&[sel(2, 1, 2, 1)]);
    let op = editor.with_lang(|lang| {
        move_lines_up_with_language(&editor.config, &editor.buffer, &editor.cursors, lang)
    });
    editor.apply(op);
    assert_eq!(editor.value(), "\tz = 2\nclass X {\n}");
}

// ---------------------------------------------------------------------------
// Not ported from Monaco: small checks of the remaining public API.

#[test]
fn expand_line_selection_grows_by_lines() {
    let mut editor = TestEditor::lines(&["one", "two", "three"]);
    editor.set_position(1, 2);
    let buffer = &editor.buffer;
    cursor::expand_line_selection(buffer, &mut editor.cursors);
    editor.assert_selection(sel(1, 1, 2, 1));
    let buffer = &editor.buffer;
    cursor::expand_line_selection(buffer, &mut editor.cursors);
    editor.assert_selection(sel(1, 1, 3, 1));
    let buffer = &editor.buffer;
    cursor::expand_line_selection(buffer, &mut editor.cursors);
    editor.assert_selection(sel(1, 1, 3, 6));
}

#[test]
fn add_cursor_above_and_below() {
    let mut editor = TestEditor::lines(&["\tabc", "abcdef", "ab"]);
    editor.set_position(2, 6);
    editor.add_cursor_above();
    editor.assert_selections(&[sel(2, 6, 2, 6), sel(1, 3, 1, 3)]);
    editor.set_position(2, 6);
    editor.add_cursor_below();
    editor.assert_selections(&[sel(2, 6, 2, 6), sel(3, 3, 3, 3)]);
}

#[test]
fn copy_and_cut_empty_selection_round_trip() {
    let mut editor = TestEditor::lines(&["one", "two"]);
    editor.set_position(1, 2);
    let (text, from_empty, pieces) = editor.copy();
    assert_eq!(
        (text.as_str(), from_empty, pieces.len()),
        ("one\n", true, 0)
    );
    editor.cut();
    assert_eq!(editor.value(), "two");
    editor.paste(&text, from_empty, None);
    assert_eq!(editor.value(), "one\ntwo");

    editor.set_selections(&[sel(1, 1, 1, 2), sel(2, 1, 2, 3)]);
    let (text, from_empty, pieces) = editor.copy();
    assert_eq!(text, "o\ntw");
    assert!(!from_empty);
    assert_eq!(pieces, vec!["o".to_string(), "tw".to_string()]);
}

#[test]
fn word_at_is_the_double_click_range() {
    let editor = TestEditor::lines(&["foo.bar  baz", ""]);
    let word_at = |line, column| {
        cursor::word_at(
            &editor.config,
            &editor.buffer,
            super::harness::pos(line, column),
        )
        .map(|r| (r.start.to_monaco(), r.end.to_monaco()))
    };
    assert_eq!(word_at(1, 2), Some(((1, 1), (1, 4))));
    assert_eq!(word_at(1, 4), Some(((1, 1), (1, 4))));
    assert_eq!(word_at(1, 9), Some(((1, 8), (1, 10))));
    assert_eq!(word_at(2, 1), None);
}

#[test]
fn page_down_and_up_move_by_page_lines() {
    let lines: Vec<String> = (0..30).map(|i| format!("line {i}")).collect();
    let lines: Vec<&str> = lines.iter().map(String::as_str).collect();
    let mut editor = TestEditor::lines(&lines);
    editor.page_lines = 10;
    editor.move_cursors(cursor::MoveCommand::PageDown, false);
    editor.assert_cursor(11, 1);
    editor.move_cursors(cursor::MoveCommand::PageDown, true);
    editor.assert_selection(sel(11, 1, 21, 1));
    // Like Monaco, moving up without extending starts from the selection start.
    editor.move_cursors(cursor::MoveCommand::PageUp, false);
    editor.assert_cursor(1, 1);
}

#[test]
fn blank_line_navigation() {
    let mut editor = TestEditor::lines(&["a", "b", "", "c", "  ", "d"]);
    editor.move_cursors(cursor::MoveCommand::NextBlankLine, false);
    editor.assert_cursor(3, 1);
    editor.move_cursors(cursor::MoveCommand::NextBlankLine, false);
    editor.assert_cursor(5, 1);
    editor.move_cursors(cursor::MoveCommand::NextBlankLine, false);
    editor.assert_cursor(6, 1);
    editor.move_cursors(cursor::MoveCommand::PrevBlankLine, false);
    editor.assert_cursor(5, 1);
    editor.move_cursors(cursor::MoveCommand::PrevBlankLine, false);
    editor.assert_cursor(3, 1);
}

#[test]
fn cursors_merge_when_they_meet() {
    let mut editor = TestEditor::lines(&["abc"]);
    editor.set_selections(&[sel(1, 2, 1, 2), sel(1, 3, 1, 3)]);
    editor.move_left(false);
    editor.assert_selections(&[sel(1, 1, 1, 1), sel(1, 2, 1, 2)]);
    editor.move_left(false);
    editor.assert_selections(&[sel(1, 1, 1, 1)]);
}

#[test]
fn external_edits_clamp_cursors() {
    let mut editor = TestEditor::lines(&["hello world", "two"]);
    editor.set_selections(&[sel(1, 12, 1, 12), sel(2, 4, 2, 4)]);
    editor.apply_external_edit((1, 6, 2, 4), "");
    editor.assert_selections(&[sel(1, 6, 1, 6)]);
}

#[test]
fn line_break_keeps_indentation_up_to_the_cursor() {
    let mut editor = TestEditor::lines(&["    abc"]);
    editor.set_position(1, 3);
    editor.line_break();
    assert_eq!(editor.value(), "  \n    abc");
    editor.assert_cursor(2, 3);
}
