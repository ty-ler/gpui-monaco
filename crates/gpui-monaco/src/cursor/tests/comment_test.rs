//! Ported from Monaco's
//! `src/vs/editor/contrib/comment/test/browser/lineCommentCommand.test.ts` and
//! `blockCommentCommand.test.ts` (MIT). The `testCommand` cases are generated
//! from the Monaco files; `_analyzeLines` and `_normalizeInsertionPoint` are
//! ported by hand.

use syntax::Comments;

use super::harness::{TestEditor, fmt_sel, sel};
use crate::cursor::comment::{LinePreflight, analyze_lines, normalize_insertion_points};
use crate::cursor::{
    CommentOptions, CursorModel, LineCommentType, Position, Selection, block_comment, line_comment,
};

#[track_caller]
fn check(
    before: &[&str],
    selection: Selection,
    after: &[&str],
    expected: Selection,
    run: impl FnOnce(&TestEditor) -> crate::cursor::EditOperation,
) {
    let mut editor = TestEditor::lines(before);
    editor.set_selection(selection);
    let op = run(&editor);
    editor.apply(op);
    assert_eq!(editor.value(), after.join("\n"));
    assert_eq!(fmt_sel(&editor.selections()[0]), fmt_sel(&expected));
}

/// Monaco `createTestCommandHelper` with a `LineCommentCommand`.
#[track_caller]
#[allow(clippy::too_many_arguments)]
fn check_line(
    comments: Comments,
    kind: LineCommentType,
    insert_space: bool,
    ignore_empty_lines: bool,
    before: &[&str],
    selection: Selection,
    after: &[&str],
    expected: Selection,
) {
    let options = CommentOptions {
        insert_space,
        ignore_empty_lines,
    };
    check(before, selection, after, expected, |e| {
        line_comment(
            &e.config,
            &e.buffer,
            &e.cursors,
            &|_| Some(comments.clone()),
            kind,
            options,
        )
    });
}

/// Monaco's block comment `_testCommentCommand` (`<0` `0>` tokens).
#[track_caller]
fn check_block(
    insert_space: bool,
    before: &[&str],
    selection: Selection,
    after: &[&str],
    expected: Selection,
) {
    let comments = Comments {
        line_comment: Some("!@#".into()),
        line_comment_no_indent: false,
        block_comment: Some(("<0".into(), "0>".into())),
    };
    let options = CommentOptions {
        insert_space,
        ignore_empty_lines: true,
    };
    check(before, selection, after, expected, |e| {
        block_comment(
            &e.config,
            &e.buffer,
            &e.cursors,
            &|_| Some(comments.clone()),
            options,
        )
    });
}

/// Monaco's "mixed modes" suite: lines starting with two spaces are in an
/// inner language with only `{/* */}` block comments.
#[track_caller]
fn check_mixed(before: &[&str], selection: Selection, after: &[&str], expected: Selection) {
    let outer = Comments {
        line_comment: Some("//".into()),
        line_comment_no_indent: false,
        block_comment: Some(("/*".into(), "*/".into())),
    };
    let inner = Comments {
        line_comment: None,
        line_comment_no_indent: false,
        block_comment: Some(("{/*".into(), "*/}".into())),
    };
    check(before, selection, after, expected, |e| {
        let comments_at = |p: Position| {
            Some(if e.buffer.line_content(p.row).starts_with("  ") {
                inner.clone()
            } else {
                outer.clone()
            })
        };
        line_comment(
            &e.config,
            &e.buffer,
            &e.cursors,
            &comments_at,
            LineCommentType::Toggle,
            CommentOptions::default(),
        )
    });
}

fn preflight(tokens: &[&str]) -> Vec<LinePreflight> {
    tokens
        .iter()
        .map(|t| LinePreflight {
            ignore: false,
            comment_str: t.to_string(),
            comment_str_offset: 0,
            comment_str_length: t.len() as u32,
        })
        .collect()
}

/// Monaco: _analyzeLines
#[test]
fn line_analyze_lines() {
    let editor = TestEditor::lines(&["\t\t", "    ", "    c", "\t\td"]);
    let mut lines = preflight(&["//", "rem", "!@#", "!@#"]);
    let remove = analyze_lines(
        LineCommentType::Toggle,
        true,
        &editor.buffer,
        &mut lines,
        0,
        true,
        false,
        false,
    );
    assert!(!remove);
    let tokens: Vec<_> = lines.iter().map(|l| l.comment_str.as_str()).collect();
    assert_eq!(tokens, ["//", "rem", "!@#", "!@#"]);
    let ignore: Vec<_> = lines.iter().map(|l| l.ignore).collect();
    assert_eq!(ignore, [true, true, false, false]);
    let offsets: Vec<_> = lines.iter().map(|l| l.comment_str_offset).collect();
    assert_eq!(offsets, [2, 4, 4, 2]);

    let editor = TestEditor::lines(&["\t\t", "    rem ", "    !@# c", "\t\t!@#d"]);
    let mut lines = preflight(&["//", "rem", "!@#", "!@#"]);
    let remove = analyze_lines(
        LineCommentType::Toggle,
        true,
        &editor.buffer,
        &mut lines,
        0,
        true,
        false,
        false,
    );
    assert!(remove);
    let ignore: Vec<_> = lines.iter().map(|l| l.ignore).collect();
    assert_eq!(ignore, [true, false, false, false]);
    let offsets: Vec<_> = lines.iter().map(|l| l.comment_str_offset).collect();
    assert_eq!(offsets, [2, 4, 4, 2]);
    let lengths: Vec<_> = lines.iter().map(|l| l.comment_str_length).collect();
    assert_eq!(lengths, [2, 4, 4, 3]);
}

/// Monaco: _normalizeInsertionPoint
#[test]
fn line_normalize_insertion_point() {
    #[track_caller]
    fn run(mixed: &[(&str, u32)], tab_size: u32, expected: &[u32]) {
        let text: Vec<&str> = mixed.iter().map(|(l, _)| *l).collect();
        let editor = TestEditor::lines(&text);
        let mut lines: Vec<LinePreflight> = mixed
            .iter()
            .map(|(_, offset)| LinePreflight {
                ignore: false,
                comment_str: String::new(),
                comment_str_offset: *offset,
                comment_str_length: 0,
            })
            .collect();
        normalize_insertion_points(&editor.buffer, &mut lines, 0, tab_size);
        let actual: Vec<u32> = lines.iter().map(|l| l.comment_str_offset).collect();
        assert_eq!(actual, expected);
    }
    // Bug 16696: comments not aligned in this case.
    run(&[("  XX", 2), ("    YY", 4)], 4, &[0, 0]);
    run(
        &[
            ("\t\t\tXX", 3),
            ("    \tYY", 5),
            ("        ZZ", 8),
            ("\t\tTT", 2),
        ],
        4,
        &[2, 5, 8, 2],
    );
    run(
        &[
            ("\t\t\t   XX", 6),
            ("    \t\t\t\tYY", 8),
            ("        ZZ", 8),
            ("\t\t    TT", 6),
        ],
        4,
        &[2, 5, 8, 2],
    );
    run(
        &[("\t\t", 2), ("\t\t\t", 3), ("\t\t\t\t", 4), ("\t\t\t", 3)],
        4,
        &[2, 2, 2, 2],
    );
    run(
        &[
            ("\t\t", 2),
            ("\t\t\t", 3),
            ("\t\t\t\t", 4),
            ("\t\t\t", 3),
            ("    ", 4),
        ],
        2,
        &[2, 2, 2, 2, 4],
    );
    run(
        &[
            ("\t\t", 2),
            ("\t\t\t", 3),
            ("\t\t\t\t", 4),
            ("\t\t\t", 3),
            ("    ", 4),
        ],
        4,
        &[1, 1, 1, 1, 4],
    );
    run(
        &[
            (" \t", 2),
            ("  \t", 3),
            ("   \t", 4),
            ("    ", 4),
            ("\t", 1),
        ],
        4,
        &[2, 3, 4, 4, 1],
    );
    run(
        &[
            (" \t\t", 3),
            ("  \t\t", 4),
            ("   \t\t", 5),
            ("    \t", 5),
            ("\t", 1),
        ],
        4,
        &[2, 3, 4, 4, 1],
    );
    run(&[("\t", 1), ("    ", 4)], 4, &[1, 4]);
    run(&[("\t", 1), ("   ", 3)], 4, &[0, 0]);
    run(&[("\t", 1), ("  ", 2)], 4, &[0, 0]);
    run(&[("\t", 1), (" ", 1)], 4, &[0, 0]);
    run(&[("\t", 1), ("", 0)], 4, &[0, 0]);
}

/// Monaco: comment single line
#[test]
fn line_comment_single_line() {
    check_line(
        Comments {
            line_comment: Some("!@#".into()),
            line_comment_no_indent: false,
            block_comment: Some(("<!@#".into(), "#@!>".into())),
        },
        LineCommentType::Toggle,
        true,
        true,
        &["some text", "\tsome more text"],
        sel(1, 1, 1, 1),
        &["!@# some text", "\tsome more text"],
        sel(1, 5, 1, 5),
    );
}

/// Monaco: case insensitive
#[test]
fn line_case_insensitive() {
    check_line(
        Comments {
            line_comment: Some("rem".into()),
            line_comment_no_indent: false,
            block_comment: None,
        },
        LineCommentType::Toggle,
        true,
        true,
        &["REM some text"],
        sel(1, 1, 1, 1),
        &["some text"],
        sel(1, 1, 1, 1),
    );
}

/// Monaco: comment with token column fixed
#[test]
fn line_comment_with_token_column_fixed() {
    check_line(
        Comments {
            line_comment: Some("!@#".into()),
            line_comment_no_indent: true,
            block_comment: Some(("<!@#".into(), "#@!>".into())),
        },
        LineCommentType::Toggle,
        true,
        true,
        &["some text", "\tsome more text"],
        sel(2, 1, 2, 1),
        &["some text", "!@# \tsome more text"],
        sel(2, 5, 2, 5),
    );
}

/// Monaco: detects indentation
#[test]
fn line_detects_indentation() {
    check_line(
        Comments {
            line_comment: Some("!@#".into()),
            line_comment_no_indent: false,
            block_comment: Some(("<!@#".into(), "#@!>".into())),
        },
        LineCommentType::Toggle,
        true,
        true,
        &["\tsome text", "\tsome more text"],
        sel(2, 2, 1, 1),
        &["\t!@# some text", "\t!@# some more text"],
        sel(2, 2, 1, 1),
    );
}

/// Monaco: detects mixed indentation
#[test]
fn line_detects_mixed_indentation() {
    check_line(
        Comments {
            line_comment: Some("!@#".into()),
            line_comment_no_indent: false,
            block_comment: Some(("<!@#".into(), "#@!>".into())),
        },
        LineCommentType::Toggle,
        true,
        true,
        &["\tsome text", "    some more text"],
        sel(2, 2, 1, 1),
        &["\t!@# some text", "    !@# some more text"],
        sel(2, 2, 1, 1),
    );
}

/// Monaco: ignores whitespace lines
#[test]
fn line_ignores_whitespace_lines() {
    check_line(
        Comments {
            line_comment: Some("!@#".into()),
            line_comment_no_indent: false,
            block_comment: Some(("<!@#".into(), "#@!>".into())),
        },
        LineCommentType::Toggle,
        true,
        true,
        &["\tsome text", "\t   ", "", "\tsome more text"],
        sel(4, 2, 1, 1),
        &["\t!@# some text", "\t   ", "", "\t!@# some more text"],
        sel(4, 2, 1, 1),
    );
}

/// Monaco: removes its own
#[test]
fn line_removes_its_own() {
    check_line(
        Comments {
            line_comment: Some("!@#".into()),
            line_comment_no_indent: false,
            block_comment: Some(("<!@#".into(), "#@!>".into())),
        },
        LineCommentType::Toggle,
        true,
        true,
        &["\t!@# some text", "\t   ", "\t\t!@# some more text"],
        sel(3, 2, 1, 1),
        &["\tsome text", "\t   ", "\t\tsome more text"],
        sel(3, 2, 1, 1),
    );
}

/// Monaco: works in only whitespace
#[test]
fn line_works_in_only_whitespace() {
    check_line(
        Comments {
            line_comment: Some("!@#".into()),
            line_comment_no_indent: false,
            block_comment: Some(("<!@#".into(), "#@!>".into())),
        },
        LineCommentType::Toggle,
        true,
        true,
        &["\t    ", "\t", "\t\tsome more text"],
        sel(3, 1, 1, 1),
        &["\t!@#     ", "\t!@# ", "\t\tsome more text"],
        sel(3, 1, 1, 1),
    );
}

/// Monaco: bug 9697 - whitespace before comment token
#[test]
fn line_bug_9697_whitespace_before_comment_token() {
    check_line(
        Comments {
            line_comment: Some("!@#".into()),
            line_comment_no_indent: false,
            block_comment: Some(("<!@#".into(), "#@!>".into())),
        },
        LineCommentType::Toggle,
        true,
        true,
        &["\t !@#first", "\tsecond line"],
        sel(1, 1, 1, 1),
        &["\t first", "\tsecond line"],
        sel(1, 1, 1, 1),
    );
}

/// Monaco: bug 10162 - line comment before caret
#[test]
fn line_bug_10162_line_comment_before_caret() {
    check_line(
        Comments {
            line_comment: Some("!@#".into()),
            line_comment_no_indent: false,
            block_comment: Some(("<!@#".into(), "#@!>".into())),
        },
        LineCommentType::Toggle,
        true,
        true,
        &["first!@#", "\tsecond line"],
        sel(1, 1, 1, 1),
        &["!@# first!@#", "\tsecond line"],
        sel(1, 5, 1, 5),
    );
}

/// Monaco: comment single line - leading whitespace
#[test]
fn line_comment_single_line_leading_whitespace() {
    check_line(
        Comments {
            line_comment: Some("!@#".into()),
            line_comment_no_indent: false,
            block_comment: Some(("<!@#".into(), "#@!>".into())),
        },
        LineCommentType::Toggle,
        true,
        true,
        &["first!@#", "\tsecond line"],
        sel(2, 3, 2, 1),
        &["first!@#", "\t!@# second line"],
        sel(2, 7, 2, 1),
    );
}

/// Monaco: ignores invisible selection
#[test]
fn line_ignores_invisible_selection() {
    check_line(
        Comments {
            line_comment: Some("!@#".into()),
            line_comment_no_indent: false,
            block_comment: Some(("<!@#".into(), "#@!>".into())),
        },
        LineCommentType::Toggle,
        true,
        true,
        &[
            "first",
            "\tsecond line",
            "third line",
            "fourth line",
            "fifth",
        ],
        sel(2, 1, 1, 1),
        &[
            "!@# first",
            "\tsecond line",
            "third line",
            "fourth line",
            "fifth",
        ],
        sel(2, 1, 1, 5),
    );
}

/// Monaco: multiple lines
#[test]
fn line_multiple_lines() {
    check_line(
        Comments {
            line_comment: Some("!@#".into()),
            line_comment_no_indent: false,
            block_comment: Some(("<!@#".into(), "#@!>".into())),
        },
        LineCommentType::Toggle,
        true,
        true,
        &[
            "first",
            "\tsecond line",
            "third line",
            "fourth line",
            "fifth",
        ],
        sel(2, 4, 1, 1),
        &[
            "!@# first",
            "!@# \tsecond line",
            "third line",
            "fourth line",
            "fifth",
        ],
        sel(2, 8, 1, 5),
    );
}

/// Monaco: multiple modes on multiple lines
#[test]
fn line_multiple_modes_on_multiple_lines() {
    check_line(
        Comments {
            line_comment: Some("!@#".into()),
            line_comment_no_indent: false,
            block_comment: Some(("<!@#".into(), "#@!>".into())),
        },
        LineCommentType::Toggle,
        true,
        true,
        &[
            "first",
            "\tsecond line",
            "third line",
            "fourth line",
            "fifth",
        ],
        sel(4, 4, 3, 1),
        &[
            "first",
            "\tsecond line",
            "!@# third line",
            "!@# fourth line",
            "fifth",
        ],
        sel(4, 8, 3, 5),
    );
}

/// Monaco: toggle single line
#[test]
fn line_toggle_single_line() {
    check_line(
        Comments {
            line_comment: Some("!@#".into()),
            line_comment_no_indent: false,
            block_comment: Some(("<!@#".into(), "#@!>".into())),
        },
        LineCommentType::Toggle,
        true,
        true,
        &[
            "first",
            "\tsecond line",
            "third line",
            "fourth line",
            "fifth",
        ],
        sel(1, 1, 1, 1),
        &[
            "!@# first",
            "\tsecond line",
            "third line",
            "fourth line",
            "fifth",
        ],
        sel(1, 5, 1, 5),
    );
    check_line(
        Comments {
            line_comment: Some("!@#".into()),
            line_comment_no_indent: false,
            block_comment: Some(("<!@#".into(), "#@!>".into())),
        },
        LineCommentType::Toggle,
        true,
        true,
        &[
            "!@# first",
            "\tsecond line",
            "third line",
            "fourth line",
            "fifth",
        ],
        sel(1, 4, 1, 4),
        &[
            "first",
            "\tsecond line",
            "third line",
            "fourth line",
            "fifth",
        ],
        sel(1, 1, 1, 1),
    );
}

/// Monaco: toggle multiple lines
#[test]
fn line_toggle_multiple_lines() {
    check_line(
        Comments {
            line_comment: Some("!@#".into()),
            line_comment_no_indent: false,
            block_comment: Some(("<!@#".into(), "#@!>".into())),
        },
        LineCommentType::Toggle,
        true,
        true,
        &[
            "first",
            "\tsecond line",
            "third line",
            "fourth line",
            "fifth",
        ],
        sel(2, 4, 1, 1),
        &[
            "!@# first",
            "!@# \tsecond line",
            "third line",
            "fourth line",
            "fifth",
        ],
        sel(2, 8, 1, 5),
    );
    check_line(
        Comments {
            line_comment: Some("!@#".into()),
            line_comment_no_indent: false,
            block_comment: Some(("<!@#".into(), "#@!>".into())),
        },
        LineCommentType::Toggle,
        true,
        true,
        &[
            "!@# first",
            "!@# \tsecond line",
            "third line",
            "fourth line",
            "fifth",
        ],
        sel(2, 7, 1, 4),
        &[
            "first",
            "\tsecond line",
            "third line",
            "fourth line",
            "fifth",
        ],
        sel(2, 3, 1, 1),
    );
}

/// Monaco: issue #5964: Ctrl+/ to create comment when cursor is at the beginning of the line puts the cursor in a strange position
#[test]
fn line_issue_5964_ctrl_to_create_comment_when_cursor_is_at_the_beginning_of_the_line_puts_the_cursor_in_a_strange_position()
 {
    check_line(
        Comments {
            line_comment: Some("!@#".into()),
            line_comment_no_indent: false,
            block_comment: Some(("<!@#".into(), "#@!>".into())),
        },
        LineCommentType::Toggle,
        true,
        true,
        &[
            "first",
            "\tsecond line",
            "third line",
            "fourth line",
            "fifth",
        ],
        sel(1, 1, 1, 1),
        &[
            "!@# first",
            "\tsecond line",
            "third line",
            "fourth line",
            "fifth",
        ],
        sel(1, 5, 1, 5),
    );
}

/// Monaco: issue #35673: Comment hotkeys throws the cursor before the comment
#[test]
fn line_issue_35673_comment_hotkeys_throws_the_cursor_before_the_comment() {
    check_line(
        Comments {
            line_comment: Some("!@#".into()),
            line_comment_no_indent: false,
            block_comment: Some(("<!@#".into(), "#@!>".into())),
        },
        LineCommentType::Toggle,
        true,
        true,
        &[
            "first",
            "",
            "\tsecond line",
            "third line",
            "fourth line",
            "fifth",
        ],
        sel(2, 1, 2, 1),
        &[
            "first",
            "!@# ",
            "\tsecond line",
            "third line",
            "fourth line",
            "fifth",
        ],
        sel(2, 5, 2, 5),
    );
    check_line(
        Comments {
            line_comment: Some("!@#".into()),
            line_comment_no_indent: false,
            block_comment: Some(("<!@#".into(), "#@!>".into())),
        },
        LineCommentType::Toggle,
        true,
        true,
        &[
            "first",
            "\t",
            "\tsecond line",
            "third line",
            "fourth line",
            "fifth",
        ],
        sel(2, 2, 2, 2),
        &[
            "first",
            "\t!@# ",
            "\tsecond line",
            "third line",
            "fourth line",
            "fifth",
        ],
        sel(2, 6, 2, 6),
    );
}

/// Monaco: issue #2837 "Add Line Comment" fault when blank lines involved
#[test]
fn line_issue_2837_add_line_comment_fault_when_blank_lines_involved() {
    check_line(
        Comments {
            line_comment: Some("!@#".into()),
            line_comment_no_indent: false,
            block_comment: Some(("<!@#".into(), "#@!>".into())),
        },
        LineCommentType::ForceAdd,
        true,
        true,
        &[
            "    if displayName == \"\":",
            "        displayName = groupName",
            "    description = getAttr(attributes, \"description\")",
            "    mailAddress = getAttr(attributes, \"mail\")",
            "",
            "    print \"||Group name|%s|\" % displayName",
            "    print \"||Description|%s|\" % description",
            "    print \"||Email address|[mailto:%s]|\" % mailAddress`",
        ],
        sel(1, 1, 8, 56),
        &[
            "    !@# if displayName == \"\":",
            "    !@#     displayName = groupName",
            "    !@# description = getAttr(attributes, \"description\")",
            "    !@# mailAddress = getAttr(attributes, \"mail\")",
            "",
            "    !@# print \"||Group name|%s|\" % displayName",
            "    !@# print \"||Description|%s|\" % description",
            "    !@# print \"||Email address|[mailto:%s]|\" % mailAddress`",
        ],
        sel(1, 1, 8, 60),
    );
}

/// Monaco: issue #47004: Toggle comments shouldn't move cursor
#[test]
fn line_issue_47004_toggle_comments_shouldn_t_move_cursor() {
    check_line(
        Comments {
            line_comment: Some("!@#".into()),
            line_comment_no_indent: false,
            block_comment: Some(("<!@#".into(), "#@!>".into())),
        },
        LineCommentType::ForceAdd,
        true,
        true,
        &["    A line", "    Another line"],
        sel(2, 7, 1, 1),
        &["    !@# A line", "    !@# Another line"],
        sel(2, 11, 1, 1),
    );
}

/// Monaco: insertSpace false
#[test]
fn line_insertspace_false() {
    check_line(
        Comments {
            line_comment: Some("!@#".into()),
            line_comment_no_indent: false,
            block_comment: None,
        },
        LineCommentType::Toggle,
        false,
        true,
        &["some text"],
        sel(1, 1, 1, 1),
        &["!@#some text"],
        sel(1, 4, 1, 4),
    );
}

/// Monaco: insertSpace false does not remove space
#[test]
fn line_insertspace_false_does_not_remove_space() {
    check_line(
        Comments {
            line_comment: Some("!@#".into()),
            line_comment_no_indent: false,
            block_comment: None,
        },
        LineCommentType::Toggle,
        false,
        true,
        &["!@#    some text"],
        sel(1, 1, 1, 1),
        &["    some text"],
        sel(1, 1, 1, 1),
    );
}

/// Monaco: does not ignore whitespace lines
#[test]
fn line_does_not_ignore_whitespace_lines() {
    check_line(
        Comments {
            line_comment: Some("!@#".into()),
            line_comment_no_indent: false,
            block_comment: Some(("<!@#".into(), "#@!>".into())),
        },
        LineCommentType::Toggle,
        true,
        false,
        &["\tsome text", "\t   ", "", "\tsome more text"],
        sel(4, 2, 1, 1),
        &[
            "!@# \tsome text",
            "!@# \t   ",
            "!@# ",
            "!@# \tsome more text",
        ],
        sel(4, 6, 1, 5),
    );
}

/// Monaco: removes its own
#[test]
fn line_removes_its_own_2() {
    check_line(
        Comments {
            line_comment: Some("!@#".into()),
            line_comment_no_indent: false,
            block_comment: Some(("<!@#".into(), "#@!>".into())),
        },
        LineCommentType::Toggle,
        true,
        false,
        &["\t!@# some text", "\t   ", "\t\t!@# some more text"],
        sel(3, 2, 1, 1),
        &["\tsome text", "\t   ", "\t\tsome more text"],
        sel(3, 2, 1, 1),
    );
}

/// Monaco: works in only whitespace
#[test]
fn line_works_in_only_whitespace_2() {
    check_line(
        Comments {
            line_comment: Some("!@#".into()),
            line_comment_no_indent: false,
            block_comment: Some(("<!@#".into(), "#@!>".into())),
        },
        LineCommentType::Toggle,
        true,
        false,
        &["\t    ", "\t", "\t\tsome more text"],
        sel(3, 1, 1, 1),
        &["\t!@#     ", "\t!@# ", "\t\tsome more text"],
        sel(3, 1, 1, 1),
    );
}

/// Monaco: comments single line
#[test]
fn line_comments_single_line() {
    check_line(
        Comments {
            line_comment: Some("!@#".into()),
            line_comment_no_indent: false,
            block_comment: Some(("<!@#".into(), "#@!>".into())),
        },
        LineCommentType::Toggle,
        true,
        false,
        &["some text", "\tsome more text"],
        sel(1, 1, 1, 1),
        &["!@# some text", "\tsome more text"],
        sel(1, 5, 1, 5),
    );
}

/// Monaco: detects indentation
#[test]
fn line_detects_indentation_2() {
    check_line(
        Comments {
            line_comment: Some("!@#".into()),
            line_comment_no_indent: false,
            block_comment: Some(("<!@#".into(), "#@!>".into())),
        },
        LineCommentType::Toggle,
        true,
        false,
        &["\tsome text", "\tsome more text"],
        sel(2, 2, 1, 1),
        &["\t!@# some text", "\t!@# some more text"],
        sel(2, 2, 1, 1),
    );
}

/// Monaco: fall back to block comment command
#[test]
fn line_fall_back_to_block_comment_command() {
    check_line(
        Comments {
            line_comment: Some("".into()),
            line_comment_no_indent: false,
            block_comment: Some(("(".into(), ")".into())),
        },
        LineCommentType::Toggle,
        true,
        true,
        &[
            "first",
            "\tsecond line",
            "third line",
            "fourth line",
            "fifth",
        ],
        sel(1, 1, 1, 1),
        &[
            "( first )",
            "\tsecond line",
            "third line",
            "fourth line",
            "fifth",
        ],
        sel(1, 3, 1, 3),
    );
}

/// Monaco: fall back to block comment command - toggle
#[test]
fn line_fall_back_to_block_comment_command_toggle() {
    check_line(
        Comments {
            line_comment: Some("".into()),
            line_comment_no_indent: false,
            block_comment: Some(("(".into(), ")".into())),
        },
        LineCommentType::Toggle,
        true,
        true,
        &[
            "(first)",
            "\tsecond line",
            "third line",
            "fourth line",
            "fifth",
        ],
        sel(1, 7, 1, 2),
        &[
            "first",
            "\tsecond line",
            "third line",
            "fourth line",
            "fifth",
        ],
        sel(1, 6, 1, 1),
    );
}

/// Monaco: bug 9513 - expand single line to uncomment auto block
#[test]
fn line_bug_9513_expand_single_line_to_uncomment_auto_block() {
    check_line(
        Comments {
            line_comment: Some("".into()),
            line_comment_no_indent: false,
            block_comment: Some(("(".into(), ")".into())),
        },
        LineCommentType::Toggle,
        true,
        true,
        &[
            "first",
            "\tsecond line",
            "third line",
            "fourth line",
            "fifth",
        ],
        sel(1, 1, 1, 1),
        &[
            "( first )",
            "\tsecond line",
            "third line",
            "fourth line",
            "fifth",
        ],
        sel(1, 3, 1, 3),
    );
}

/// Monaco: bug 9691 - always expand selection to line boundaries
#[test]
fn line_bug_9691_always_expand_selection_to_line_boundaries() {
    check_line(
        Comments {
            line_comment: Some("".into()),
            line_comment_no_indent: false,
            block_comment: Some(("(".into(), ")".into())),
        },
        LineCommentType::Toggle,
        true,
        true,
        &[
            "first",
            "\tsecond line",
            "third line",
            "fourth line",
            "fifth",
        ],
        sel(3, 2, 1, 3),
        &[
            "( first",
            "\tsecond line",
            "third line )",
            "fourth line",
            "fifth",
        ],
        sel(3, 2, 1, 5),
    );
    check_line(
        Comments {
            line_comment: Some("".into()),
            line_comment_no_indent: false,
            block_comment: Some(("(".into(), ")".into())),
        },
        LineCommentType::Toggle,
        true,
        true,
        &[
            "(first",
            "\tsecond line",
            "third line)",
            "fourth line",
            "fifth",
        ],
        sel(3, 11, 1, 2),
        &[
            "first",
            "\tsecond line",
            "third line",
            "fourth line",
            "fifth",
        ],
        sel(3, 11, 1, 1),
    );
}

/// Monaco: no selection => uses indentation
#[test]
fn line_no_selection_uses_indentation() {
    check_line(
        Comments {
            line_comment: None,
            line_comment_no_indent: false,
            block_comment: Some(("<!@#".into(), "#@!>".into())),
        },
        LineCommentType::Toggle,
        true,
        true,
        &[
            "\t\tfirst\t    ",
            "\t\tsecond line",
            "\tthird line",
            "fourth line",
            "\t\t<!@#fifth#@!>\t\t",
        ],
        sel(1, 1, 1, 1),
        &[
            "\t\t<!@# first\t     #@!>",
            "\t\tsecond line",
            "\tthird line",
            "fourth line",
            "\t\t<!@#fifth#@!>\t\t",
        ],
        sel(1, 1, 1, 1),
    );
    check_line(
        Comments {
            line_comment: None,
            line_comment_no_indent: false,
            block_comment: Some(("<!@#".into(), "#@!>".into())),
        },
        LineCommentType::Toggle,
        true,
        true,
        &[
            "\t\t<!@#first\t    #@!>",
            "\t\tsecond line",
            "\tthird line",
            "fourth line",
            "\t\t<!@#fifth#@!>\t\t",
        ],
        sel(1, 1, 1, 1),
        &[
            "\t\tfirst\t   ",
            "\t\tsecond line",
            "\tthird line",
            "fourth line",
            "\t\t<!@#fifth#@!>\t\t",
        ],
        sel(1, 1, 1, 1),
    );
}

/// Monaco: can remove
#[test]
fn line_can_remove() {
    check_line(
        Comments {
            line_comment: None,
            line_comment_no_indent: false,
            block_comment: Some(("<!@#".into(), "#@!>".into())),
        },
        LineCommentType::Toggle,
        true,
        true,
        &[
            "\t\tfirst\t    ",
            "\t\tsecond line",
            "\tthird line",
            "fourth line",
            "\t\t<!@#fifth#@!>\t\t",
        ],
        sel(5, 1, 5, 1),
        &[
            "\t\tfirst\t    ",
            "\t\tsecond line",
            "\tthird line",
            "fourth line",
            "\t\tfifth\t\t",
        ],
        sel(5, 1, 5, 1),
    );
    check_line(
        Comments {
            line_comment: None,
            line_comment_no_indent: false,
            block_comment: Some(("<!@#".into(), "#@!>".into())),
        },
        LineCommentType::Toggle,
        true,
        true,
        &[
            "\t\tfirst\t    ",
            "\t\tsecond line",
            "\tthird line",
            "fourth line",
            "\t\t<!@#fifth#@!>\t\t",
        ],
        sel(5, 3, 5, 3),
        &[
            "\t\tfirst\t    ",
            "\t\tsecond line",
            "\tthird line",
            "fourth line",
            "\t\tfifth\t\t",
        ],
        sel(5, 3, 5, 3),
    );
    check_line(
        Comments {
            line_comment: None,
            line_comment_no_indent: false,
            block_comment: Some(("<!@#".into(), "#@!>".into())),
        },
        LineCommentType::Toggle,
        true,
        true,
        &[
            "\t\tfirst\t    ",
            "\t\tsecond line",
            "\tthird line",
            "fourth line",
            "\t\t<!@#fifth#@!>\t\t",
        ],
        sel(5, 4, 5, 4),
        &[
            "\t\tfirst\t    ",
            "\t\tsecond line",
            "\tthird line",
            "fourth line",
            "\t\tfifth\t\t",
        ],
        sel(5, 3, 5, 3),
    );
    check_line(
        Comments {
            line_comment: None,
            line_comment_no_indent: false,
            block_comment: Some(("<!@#".into(), "#@!>".into())),
        },
        LineCommentType::Toggle,
        true,
        true,
        &[
            "\t\tfirst\t    ",
            "\t\tsecond line",
            "\tthird line",
            "fourth line",
            "\t\t<!@#fifth#@!>\t\t",
        ],
        sel(5, 16, 5, 3),
        &[
            "\t\tfirst\t    ",
            "\t\tsecond line",
            "\tthird line",
            "fourth line",
            "\t\tfifth\t\t",
        ],
        sel(5, 8, 5, 3),
    );
    check_line(
        Comments {
            line_comment: None,
            line_comment_no_indent: false,
            block_comment: Some(("<!@#".into(), "#@!>".into())),
        },
        LineCommentType::Toggle,
        true,
        true,
        &[
            "\t\tfirst\t    ",
            "\t\tsecond line",
            "\tthird line",
            "fourth line",
            "\t\t<!@#fifth#@!>\t\t",
        ],
        sel(5, 12, 5, 7),
        &[
            "\t\tfirst\t    ",
            "\t\tsecond line",
            "\tthird line",
            "fourth line",
            "\t\tfifth\t\t",
        ],
        sel(5, 8, 5, 3),
    );
    check_line(
        Comments {
            line_comment: None,
            line_comment_no_indent: false,
            block_comment: Some(("<!@#".into(), "#@!>".into())),
        },
        LineCommentType::Toggle,
        true,
        true,
        &[
            "\t\tfirst\t    ",
            "\t\tsecond line",
            "\tthird line",
            "fourth line",
            "\t\t<!@#fifth#@!>\t\t",
        ],
        sel(5, 18, 5, 18),
        &[
            "\t\tfirst\t    ",
            "\t\tsecond line",
            "\tthird line",
            "fourth line",
            "\t\tfifth\t\t",
        ],
        sel(5, 10, 5, 10),
    );
}

/// Monaco: issue #993: Remove comment does not work consistently in HTML
#[test]
fn line_issue_993_remove_comment_does_not_work_consistently_in_html() {
    check_line(
        Comments {
            line_comment: None,
            line_comment_no_indent: false,
            block_comment: Some(("<!@#".into(), "#@!>".into())),
        },
        LineCommentType::Toggle,
        true,
        true,
        &["     asd qwe", "     asd qwe", ""],
        sel(1, 1, 3, 1),
        &["     <!@# asd qwe", "     asd qwe #@!>", ""],
        sel(1, 1, 3, 1),
    );
    check_line(
        Comments {
            line_comment: None,
            line_comment_no_indent: false,
            block_comment: Some(("<!@#".into(), "#@!>".into())),
        },
        LineCommentType::Toggle,
        true,
        true,
        &["     <!@#asd qwe", "     asd qwe#@!>", ""],
        sel(1, 1, 3, 1),
        &["     asd qwe", "     asd qwe", ""],
        sel(1, 1, 3, 1),
    );
}

/// Monaco: issue #24047 (part 1): Commenting code in JSX files
#[test]
fn line_issue_24047_part_1_commenting_code_in_jsx_files() {
    check_mixed(
        &[
            "import React from 'react';",
            "const Loader = () => (",
            "  <div>",
            "    Loading...",
            "  </div>",
            ");",
            "export default Loader;",
        ],
        sel(1, 1, 7, 22),
        &[
            "// import React from 'react';",
            "// const Loader = () => (",
            "//   <div>",
            "//     Loading...",
            "//   </div>",
            "// );",
            "// export default Loader;",
        ],
        sel(1, 4, 7, 25),
    );
}

/// Monaco: issue #24047 (part 2): Commenting code in JSX files
#[test]
fn line_issue_24047_part_2_commenting_code_in_jsx_files() {
    check_mixed(
        &[
            "import React from 'react';",
            "const Loader = () => (",
            "  <div>",
            "    Loading...",
            "  </div>",
            ");",
            "export default Loader;",
        ],
        sel(3, 4, 3, 4),
        &[
            "import React from 'react';",
            "const Loader = () => (",
            "  {/* <div> */}",
            "    Loading...",
            "  </div>",
            ");",
            "export default Loader;",
        ],
        sel(3, 8, 3, 8),
    );
}

/// Monaco: issue #36173: Commenting code in JSX tag body
#[test]
fn line_issue_36173_commenting_code_in_jsx_tag_body() {
    check_mixed(
        &["<div>", "  {123}", "</div>"],
        sel(2, 4, 2, 4),
        &["<div>", "  {/* {123} */}", "</div>"],
        sel(2, 8, 2, 8),
    );
}

/// Monaco: empty selection wraps itself
#[test]
fn block_empty_selection_wraps_itself() {
    check_block(
        true,
        &[
            "first",
            "\tsecond line",
            "third line",
            "fourth line",
            "fifth",
        ],
        sel(1, 3, 1, 3),
        &[
            "fi<0  0>rst",
            "\tsecond line",
            "third line",
            "fourth line",
            "fifth",
        ],
        sel(1, 6, 1, 6),
    );
}

/// Monaco: invisible selection ignored
#[test]
fn block_invisible_selection_ignored() {
    check_block(
        true,
        &[
            "first",
            "\tsecond line",
            "third line",
            "fourth line",
            "fifth",
        ],
        sel(2, 1, 1, 1),
        &[
            "<0 first",
            " 0>\tsecond line",
            "third line",
            "fourth line",
            "fifth",
        ],
        sel(1, 4, 2, 1),
    );
}

/// Monaco: bug9511
#[test]
fn block_bug9511() {
    check_block(
        true,
        &[
            "first",
            "\tsecond line",
            "third line",
            "fourth line",
            "fifth",
        ],
        sel(1, 6, 1, 1),
        &[
            "<0 first 0>",
            "\tsecond line",
            "third line",
            "fourth line",
            "fifth",
        ],
        sel(1, 4, 1, 9),
    );
    check_block(
        true,
        &[
            "<0first0>",
            "\tsecond line",
            "third line",
            "fourth line",
            "fifth",
        ],
        sel(1, 8, 1, 3),
        &[
            "first",
            "\tsecond line",
            "third line",
            "fourth line",
            "fifth",
        ],
        sel(1, 1, 1, 6),
    );
}

/// Monaco: one line selection
#[test]
fn block_one_line_selection() {
    check_block(
        true,
        &[
            "first",
            "\tsecond line",
            "third line",
            "fourth line",
            "fifth",
        ],
        sel(1, 6, 1, 3),
        &[
            "fi<0 rst 0>",
            "\tsecond line",
            "third line",
            "fourth line",
            "fifth",
        ],
        sel(1, 6, 1, 9),
    );
}

/// Monaco: one line selection toggle
#[test]
fn block_one_line_selection_toggle() {
    check_block(
        true,
        &[
            "first",
            "\tsecond line",
            "third line",
            "fourth line",
            "fifth",
        ],
        sel(1, 6, 1, 3),
        &[
            "fi<0 rst 0>",
            "\tsecond line",
            "third line",
            "fourth line",
            "fifth",
        ],
        sel(1, 6, 1, 9),
    );
    check_block(
        true,
        &[
            "fi<0rst0>",
            "\tsecond line",
            "third line",
            "fourth line",
            "fifth",
        ],
        sel(1, 8, 1, 5),
        &[
            "first",
            "\tsecond line",
            "third line",
            "fourth line",
            "fifth",
        ],
        sel(1, 3, 1, 6),
    );
    check_block(
        true,
        &[
            "<0 first 0>",
            "\tsecond line",
            "third line",
            "fourth line",
            "fifth",
        ],
        sel(1, 10, 1, 1),
        &[
            "first",
            "\tsecond line",
            "third line",
            "fourth line",
            "fifth",
        ],
        sel(1, 1, 1, 6),
    );
    check_block(
        true,
        &[
            "<0 first0>",
            "\tsecond line",
            "third line",
            "fourth line",
            "fifth",
        ],
        sel(1, 9, 1, 1),
        &[
            "first",
            "\tsecond line",
            "third line",
            "fourth line",
            "fifth",
        ],
        sel(1, 1, 1, 6),
    );
    check_block(
        true,
        &[
            "<0first 0>",
            "\tsecond line",
            "third line",
            "fourth line",
            "fifth",
        ],
        sel(1, 9, 1, 1),
        &[
            "first",
            "\tsecond line",
            "third line",
            "fourth line",
            "fifth",
        ],
        sel(1, 1, 1, 6),
    );
    check_block(
        true,
        &[
            "fi<0rst0>",
            "\tsecond line",
            "third line",
            "fourth line",
            "fifth",
        ],
        sel(1, 8, 1, 5),
        &[
            "first",
            "\tsecond line",
            "third line",
            "fourth line",
            "fifth",
        ],
        sel(1, 3, 1, 6),
    );
}

/// Monaco: multi line selection
#[test]
fn block_multi_line_selection() {
    check_block(
        true,
        &[
            "first",
            "\tsecond line",
            "third line",
            "fourth line",
            "fifth",
        ],
        sel(2, 4, 1, 1),
        &[
            "<0 first",
            "\tse 0>cond line",
            "third line",
            "fourth line",
            "fifth",
        ],
        sel(1, 4, 2, 4),
    );
}

/// Monaco: multi line selection toggle
#[test]
fn block_multi_line_selection_toggle() {
    check_block(
        true,
        &[
            "first",
            "\tsecond line",
            "third line",
            "fourth line",
            "fifth",
        ],
        sel(2, 4, 1, 1),
        &[
            "<0 first",
            "\tse 0>cond line",
            "third line",
            "fourth line",
            "fifth",
        ],
        sel(1, 4, 2, 4),
    );
    check_block(
        true,
        &[
            "<0first",
            "\tse0>cond line",
            "third line",
            "fourth line",
            "fifth",
        ],
        sel(2, 4, 1, 3),
        &[
            "first",
            "\tsecond line",
            "third line",
            "fourth line",
            "fifth",
        ],
        sel(1, 1, 2, 4),
    );
    check_block(
        true,
        &[
            "<0 first",
            "\tse0>cond line",
            "third line",
            "fourth line",
            "fifth",
        ],
        sel(2, 4, 1, 3),
        &[
            "first",
            "\tsecond line",
            "third line",
            "fourth line",
            "fifth",
        ],
        sel(1, 1, 2, 4),
    );
    check_block(
        true,
        &[
            "<0first",
            "\tse 0>cond line",
            "third line",
            "fourth line",
            "fifth",
        ],
        sel(2, 4, 1, 3),
        &[
            "first",
            "\tsecond line",
            "third line",
            "fourth line",
            "fifth",
        ],
        sel(1, 1, 2, 4),
    );
    check_block(
        true,
        &[
            "<0 first",
            "\tse 0>cond line",
            "third line",
            "fourth line",
            "fifth",
        ],
        sel(2, 4, 1, 3),
        &[
            "first",
            "\tsecond line",
            "third line",
            "fourth line",
            "fifth",
        ],
        sel(1, 1, 2, 4),
    );
}

/// Monaco: fuzzy removes
#[test]
fn block_fuzzy_removes() {
    check_block(
        true,
        &["asd <0 qwe", "asd 0> qwe"],
        sel(2, 5, 1, 7),
        &["asd qwe", "asd qwe"],
        sel(1, 5, 2, 4),
    );
    check_block(
        true,
        &["asd <0 qwe", "asd 0> qwe"],
        sel(2, 5, 1, 6),
        &["asd qwe", "asd qwe"],
        sel(1, 5, 2, 4),
    );
    check_block(
        true,
        &["asd <0 qwe", "asd 0> qwe"],
        sel(2, 5, 1, 5),
        &["asd qwe", "asd qwe"],
        sel(1, 5, 2, 4),
    );
    check_block(
        true,
        &["asd <0 qwe", "asd 0> qwe"],
        sel(2, 5, 1, 11),
        &["asd qwe", "asd qwe"],
        sel(1, 5, 2, 4),
    );
    check_block(
        true,
        &["asd <0 qwe", "asd 0> qwe"],
        sel(2, 1, 1, 11),
        &["asd qwe", "asd qwe"],
        sel(1, 5, 2, 4),
    );
    check_block(
        true,
        &["asd <0 qwe", "asd 0> qwe"],
        sel(2, 7, 1, 11),
        &["asd qwe", "asd qwe"],
        sel(1, 5, 2, 4),
    );
}

/// Monaco: bug #30358
#[test]
fn block_bug_30358() {
    check_block(
        true,
        &["<0 start 0> middle end"],
        sel(1, 20, 1, 23),
        &["<0 start 0> middle <0 end 0>"],
        sel(1, 23, 1, 26),
    );
    check_block(
        true,
        &["<0 start 0> middle <0 end 0>"],
        sel(1, 13, 1, 19),
        &["<0 start 0> <0 middle 0> <0 end 0>"],
        sel(1, 16, 1, 22),
    );
}

/// Monaco: issue #34618
#[test]
fn block_issue_34618() {
    check_block(
        true,
        &["<0  0> middle end"],
        sel(1, 4, 1, 4),
        &[" middle end"],
        sel(1, 1, 1, 1),
    );
}

/// Monaco: insertSpace false
#[test]
fn block_insertspace_false() {
    check_block(
        false,
        &["some text"],
        sel(1, 1, 1, 5),
        &["<0some0> text"],
        sel(1, 3, 1, 7),
    );
}

/// Monaco: insertSpace false does not remove space
#[test]
fn block_insertspace_false_does_not_remove_space() {
    check_block(
        false,
        &["<0 some 0> text"],
        sel(1, 4, 1, 8),
        &[" some  text"],
        sel(1, 1, 1, 7),
    );
}
