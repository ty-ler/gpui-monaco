//! Port of the `Auto Indent On Paste - TypeScript/JavaScript` suite of
//! Monaco's `src/vs/editor/contrib/indentation/test/browser/indentation.test.ts`
//! (MIT), on the `Language.TypeScript` test configuration (see
//! `language_configs.rs`) and its fixed per-line tokens.
//!
//! Skipped:
//! - `test.skip` in Monaco (known failures): `issue #181065`,
//!   `issue #86301`, `issue #85781`, `issue #29589`, `issue #201420`.
//! - `Auto Indent On Paste - Go`: its only active test is a placeholder
//!   (`assert.ok(true)`), and `issue #199050` is a `test.skip`.

use super::harness::{TestEditor, pos, sel};
use super::language_configs::typescript_test_language;
use super::tokenizer::{LineTokens, Tokenizer};
use crate::cursor::{AutoIndent, CursorConfig, StandardTokenType, auto_indent_on_paste};

use StandardTokenType::{Comment, Other, String as Str};

fn ts(text: &[&str]) -> TestEditor {
    let config = CursorConfig {
        auto_indent: AutoIndent::Full,
        ..CursorConfig::default()
    };
    TestEditor::lines_with_config(text, config).with_language(typescript_test_language())
}

/// Monaco `autoIndentOnPasteController.trigger(new Range(...))` with the
/// default `autoIndentOnPasteWithinString: true`.
fn trigger(ed: &mut TestEditor, range: (u32, u32, u32, u32)) {
    trigger_with(ed, range, true);
}

fn trigger_with(ed: &mut TestEditor, range: (u32, u32, u32, u32), within_string: bool) {
    let range = pos(range.0, range.1)..pos(range.2, range.3);
    let op = ed.with_lang(|lang| {
        auto_indent_on_paste(
            &ed.config,
            &ed.buffer,
            &ed.cursors,
            range,
            lang.expect("language"),
            within_string,
        )
    });
    if let Some(op) = op {
        ed.apply(op);
    }
}

#[test]
fn issue_119225_do_not_add_extra_leading_space_when_pasting_jsdoc() {
    let paste_text = ["/**", " * JSDoc", " */", "function a() {}"].join("\n");
    let tokens: Vec<LineTokens> = vec![
        vec![(0, Comment), (3, Comment)],
        vec![(0, Comment), (2, Comment), (8, Comment)],
        vec![(0, Comment), (1, Comment), (3, Other)],
        vec![
            (0, Other),
            (8, Other),
            (9, Other),
            (10, Other),
            (11, Other),
            (12, Other),
            (13, Other),
            (14, Other),
            (15, Other),
        ],
    ];
    let mut ed = ts(&[""]).with_tokenizer(Tokenizer::Fixed(tokens));
    ed.paste(&paste_text, true, None);
    trigger(&mut ed, (1, 1, 4, 16));
    assert_eq!(ed.value(), paste_text);
}

#[test]
fn issue_167299_blank_line_removes_indent() {
    // No need for tokenization because there are no comments.
    let paste_text = [
        "",
        "export type IncludeReference =",
        "\t| BaseReference",
        "\t| SelfReference",
        "\t| RelativeReference;",
        "",
        "export const enum IncludeReferenceKind {",
        "\tBase,",
        "\tSelf,",
        "\tRelativeReference,",
        "}",
    ]
    .join("\n");
    let mut ed = ts(&[""]);
    ed.paste(&paste_text, true, None);
    trigger(&mut ed, (1, 1, 11, 2));
    assert_eq!(ed.value(), paste_text);
}

#[test]
fn issue_29803_do_not_indent_when_pasting_text_with_only_one_line() {
    let mut ed = ts(&["const linkHandler = new Class(a, b, c,", "    d)"]);
    ed.set_selection(sel(2, 6, 2, 6));
    ed.paste(", null", true, None);
    trigger(&mut ed, (2, 6, 2, 11));
    assert_eq!(
        ed.value(),
        ["const linkHandler = new Class(a, b, c,", "    d, null)"].join("\n")
    );
}

#[test]
fn issue_29753_incorrect_indentation_after_comment() {
    let mut ed = ts(&[
        "class A {",
        "    /**",
        "     * used only for debug purposes.",
        "     */",
        "    private _codeInfo: KeyMapping[];",
        "}",
    ]);
    ed.set_selection(sel(5, 24, 5, 34));
    ed.paste("IMacLinuxKeyMapping", true, None);
    trigger(&mut ed, (5, 24, 5, 43));
    assert_eq!(
        ed.value(),
        [
            "class A {",
            "    /**",
            "     * used only for debug purposes.",
            "     */",
            "    private _codeInfo: IMacLinuxKeyMapping[];",
            "}",
        ]
        .join("\n")
    );
}

#[test]
fn issue_29753_incorrect_indentation_of_header_comment() {
    let text = [
        "/*----------------",
        " *  Copyright (c) ",
        " *  Licensed under ...",
        " *-----------------*/",
    ]
    .join("\n");
    let mut ed = ts(&[""]);
    ed.paste(&text, true, None);
    trigger(&mut ed, (1, 1, 4, 22));
    assert_eq!(ed.value(), text);
}

#[test]
fn issue_209859_do_not_change_indentation_when_pasted_inside_of_a_string() {
    let initial_text = [
        "const foo = \"some text",
        "         which is strangely",
        "    indented\"",
    ];
    let tokens: Vec<LineTokens> = vec![vec![(0, Other), (12, Str)], vec![(0, Str)], vec![(0, Str)]];
    let mut ed = ts(&initial_text).with_tokenizer(Tokenizer::Fixed(tokens));
    ed.set_selection(sel(2, 10, 2, 15));
    ed.paste("which", true, None);
    // Monaco runs this with the default `autoIndentOnPasteWithinString:
    // true`; there line 2 is not tokenized yet when `trigger` runs, so (as far
    // as we can tell) `isCheapToTokenize` makes it return early. Without that
    // early return, `trigger` gives line 2 the indentation of line 1. The port
    // always has tokens, so it checks what the issue is about instead: with
    // `autoIndentOnPasteWithinString: false`, a paste that starts in a string
    // keeps its indentation.
    trigger_with(&mut ed, (2, 1, 2, 28), false);
    assert_eq!(ed.value(), initial_text.join("\n"));
}
