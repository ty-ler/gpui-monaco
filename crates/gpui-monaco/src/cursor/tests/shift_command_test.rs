//! Tests of Monaco's
//! `src/vs/editor/test/browser/commands/shiftCommand.test.ts` (MIT) that
//! need the language: the doc comment alignment of `ShiftCommand`, and the
//! leading whitespace cases of issue #1620. Monaco runs the command on its
//! own; here it runs through Indent Lines and Outdent Lines, which are one
//! `ShiftCommand` per selection.

use syntax::LanguageConfiguration;

use super::harness::{TestEditor, sel};
use super::language_configs::{brackets, javascript_on_enter_rules};
use crate::cursor::{CursorConfig, Selection};

/// Monaco's `DocBlockCommentMode`.
fn doc_block_comment_mode() -> LanguageConfiguration {
    LanguageConfiguration {
        brackets: Some(brackets(&[("(", ")"), ("{", "}"), ("[", "]")])),
        on_enter_rules: Some(javascript_on_enter_rules()),
        ..Default::default()
    }
}

fn shift_config(insert_spaces: bool, use_tab_stops: bool) -> CursorConfig {
    CursorConfig {
        tab_size: 4,
        indent_size: 4,
        insert_spaces,
        use_tab_stops,
        ..CursorConfig::default()
    }
}

/// Monaco's `testCommand` with a `ShiftCommand` (`unshift` for
/// `testUnshiftCommand`).
#[track_caller]
#[allow(clippy::too_many_arguments)]
fn test_shift(
    lines: &[&str],
    language: Option<LanguageConfiguration>,
    insert_spaces: bool,
    use_tab_stops: bool,
    unshift: bool,
    selection: Selection,
    expected_lines: &[&str],
    expected_selection: Selection,
) {
    let mut ed = TestEditor::lines_with_config(lines, shift_config(insert_spaces, use_tab_stops));
    if let Some(language) = language {
        ed = ed.with_language(language);
    }
    ed.set_selection(selection);
    if unshift {
        ed.outdent_lines();
    } else {
        ed.indent_lines();
    }
    assert_eq!(ed.value(), expected_lines.join("\n"));
    ed.assert_selection(expected_selection);
}

/// Monaco `issue #348: indenting around doc block comments`.
#[test]
fn issue_348_indenting_around_doc_block_comments() {
    let lang = || Some(doc_block_comment_mode());
    test_shift(
        &["", "/**", " * a doc comment", " */", "function hello() {}"],
        lang(),
        false,
        true,
        false,
        sel(1, 1, 5, 20),
        &[
            "",
            "\t/**",
            "\t * a doc comment",
            "\t */",
            "\tfunction hello() {}",
        ],
        sel(1, 1, 5, 21),
    );

    test_shift(
        &["", "/**", " * a doc comment", " */", "function hello() {}"],
        lang(),
        false,
        true,
        true,
        sel(1, 1, 5, 20),
        &["", "/**", " * a doc comment", " */", "function hello() {}"],
        sel(1, 1, 5, 20),
    );

    test_shift(
        &[
            "\t",
            "\t/**",
            "\t * a doc comment",
            "\t */",
            "\tfunction hello() {}",
        ],
        lang(),
        false,
        true,
        true,
        sel(1, 1, 5, 21),
        &["", "/**", " * a doc comment", " */", "function hello() {}"],
        sel(1, 1, 5, 20),
    );
}

/// Monaco `issue #1609: Wrong indentation of block comments`.
#[test]
fn issue_1609_wrong_indentation_of_block_comments() {
    test_shift(
        &[
            "",
            "/**",
            " * test",
            " *",
            " * @type {number}",
            " */",
            "var foo = 0;",
        ],
        Some(doc_block_comment_mode()),
        false,
        true,
        false,
        sel(1, 1, 7, 13),
        &[
            "",
            "\t/**",
            "\t * test",
            "\t *",
            "\t * @type {number}",
            "\t */",
            "\tvar foo = 0;",
        ],
        sel(1, 1, 7, 14),
    );
}

const TABLE: &[&str] = &[
    "   Written | Numeric",
    "       one | 1",
    "       two | 2",
    "     three | 3",
    "      four | 4",
    "      five | 5",
    "       six | 6",
    "     seven | 7",
    "     eight | 8",
    "      nine | 9",
    "       ten | 10",
    "    eleven | 11",
    "",
];

/// [`TABLE`] with `prefix` before every line but the last.
fn table(prefix: &str) -> Vec<String> {
    TABLE
        .iter()
        .map(|line| {
            if line.is_empty() {
                String::new()
            } else {
                format!("{prefix}{line}")
            }
        })
        .collect()
}

fn strs(lines: &[String]) -> Vec<&str> {
    lines.iter().map(String::as_str).collect()
}

/// Monaco `issue #1620: a) Line indent doesn't handle leading whitespace
/// properly`.
#[test]
fn issue_1620_a_line_indent_doesnt_handle_leading_whitespace_properly() {
    test_shift(
        TABLE,
        None,
        true,
        false,
        false,
        sel(1, 1, 13, 1),
        &strs(&table("    ")),
        sel(1, 1, 13, 1),
    );
}

/// Monaco `issue #1620: b) Line indent doesn't handle leading whitespace
/// properly`.
#[test]
fn issue_1620_b_line_indent_doesnt_handle_leading_whitespace_properly() {
    test_shift(
        &strs(&table("    ")),
        None,
        true,
        false,
        true,
        sel(1, 1, 13, 1),
        TABLE,
        sel(1, 1, 13, 1),
    );
}

/// Monaco `issue #1620: c) Line indent doesn't handle leading whitespace
/// properly`.
#[test]
fn issue_1620_c_line_indent_doesnt_handle_leading_whitespace_properly() {
    test_shift(
        &strs(&table("    ")),
        None,
        false,
        false,
        true,
        sel(1, 1, 13, 1),
        TABLE,
        sel(1, 1, 13, 1),
    );
}

/// Monaco `issue #1620: d) Line indent doesn't handle leading whitespace
/// properly`.
#[test]
fn issue_1620_d_line_indent_doesnt_handle_leading_whitespace_properly() {
    test_shift(
        &strs(&table("\t")),
        None,
        true,
        false,
        true,
        sel(1, 1, 13, 1),
        TABLE,
        sel(1, 1, 13, 1),
    );
}

/// The audit's repro (#331): with 4 spaces, Tab on a selected doc comment
/// keeps each continuation line's alignment space, and Shift+Tab on the
/// unindented comment leaves it alone.
#[test]
fn tab_and_shift_tab_keep_doc_comment_alignment_with_spaces() {
    let lines = ["/**", " * a", " */", "x();"];
    let mut ed = TestEditor::lines_with_config(&lines, shift_config(true, true))
        .with_language(doc_block_comment_mode());
    ed.set_selection(sel(1, 1, 4, 5));
    ed.tab();
    assert_eq!(ed.value(), "    /**\n     * a\n     */\n    x();");

    let mut ed = TestEditor::lines_with_config(&lines, shift_config(true, true))
        .with_language(doc_block_comment_mode());
    ed.set_selection(sel(1, 1, 4, 5));
    ed.outdent();
    assert_eq!(ed.value(), lines.join("\n"));
}
