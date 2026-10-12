//! Language-aware typing with the real VS Code configurations bundled in
//! `gpui-monaco-languages` (not ported from Monaco).

use super::harness::{TestEditor, pos, sel};
use crate::cursor::{LanguageContext, Position, StandardTokenType, matching_bracket};

fn rust(lines: &[&str]) -> TestEditor {
    TestEditor::lines(lines).with_language(gpui_monaco_languages::rust().config)
}

fn typescript(lines: &[&str]) -> TestEditor {
    TestEditor::lines(lines).with_language(gpui_monaco_languages::typescript().config)
}

#[test]
fn rust_brace_then_enter_indents() {
    let mut ed = rust(&["fn main() "]);
    ed.move_to(1, 11, false);
    ed.type_keyboard("{");
    assert_eq!(ed.value(), "fn main() {}");
    ed.type_keyboard("\n");
    assert_eq!(ed.value(), "fn main() {\n    \n}");
    ed.assert_cursor(2, 5);
    ed.type_keyboard("let x = (1");
    assert_eq!(ed.line(2), "    let x = (1)");
    ed.type_keyboard(")");
    assert_eq!(
        ed.line(2),
        "    let x = (1)",
        "types over the auto-closed paren"
    );
}

#[test]
fn rust_enter_after_open_brace_indents_next_line() {
    let mut ed = rust(&["fn main() {"]);
    ed.move_to(1, 12, false);
    ed.type_keyboard("\n");
    assert_eq!(ed.value(), "fn main() {\n    ");
    ed.type_keyboard("x();");
    ed.type_keyboard("\n");
    assert_eq!(ed.line(3), "    ");
    ed.type_keyboard("}");
    assert_eq!(
        ed.value(),
        "fn main() {\n    x();\n}",
        "closing brace outdents"
    );
}

#[test]
fn rust_quotes_auto_close_and_delete_as_a_pair() {
    let mut ed = rust(&["let s = ;"]);
    ed.move_to(1, 9, false);
    ed.type_keyboard("\"");
    assert_eq!(ed.line(1), "let s = \"\";");
    ed.delete_left();
    assert_eq!(ed.line(1), "let s = ;");
}

#[test]
fn rust_line_comment_continues_on_enter() {
    let mut ed = rust(&["    // hello world"]);
    ed.move_to(1, 13, false);
    ed.type_keyboard("\n");
    // Like Monaco, the space after the cursor stays.
    assert_eq!(ed.value(), "    // hello\n    //  world");
}

#[test]
fn rust_surrounds_selection() {
    let mut ed = rust(&["a + b"]);
    ed.set_selection(sel(1, 1, 1, 6));
    ed.type_keyboard("(");
    assert_eq!(ed.value(), "(a + b)");
    ed.assert_selection(sel(1, 2, 1, 7));
}

#[test]
fn rust_tab_on_empty_line_indents_to_the_expected_level() {
    let mut ed = rust(&["fn main() {", "    if x {", "", "    }", "}"]);
    ed.move_to(3, 1, false);
    ed.tab();
    assert_eq!(ed.line(3), "        ");
}

#[test]
fn typescript_doc_comment_auto_closes_and_continues() {
    let mut ed = typescript(&["/*"]);
    ed.move_to(1, 3, false);
    ed.type_keyboard("*");
    assert_eq!(ed.value(), "/** */");
    ed.type_keyboard("\n");
    assert_eq!(ed.value(), "/**\n * \n */");
    ed.assert_cursor(2, 4);
}

#[test]
fn typescript_enter_between_parens_indents_and_outdents() {
    let mut ed = typescript(&["foo()"]);
    ed.move_to(1, 5, false);
    ed.type_keyboard("\n");
    assert_eq!(ed.value(), "foo(\n    \n)");
    ed.assert_cursor(2, 5);
}

#[test]
fn typescript_quote_does_not_auto_close_inside_a_string() {
    let config = gpui_monaco_languages::typescript().config;
    let ed = TestEditor::lines(&["let s = 'ab';"]);
    // Mark `'ab'` as a string.
    let string = |p: Position| {
        if (9..=12).contains(&p.column) {
            StandardTokenType::String
        } else {
            StandardTokenType::Other
        }
    };
    let lang = LanguageContext::new(&config, &string);
    let mut cursors = ed.cursors.clone();
    cursors.set_selections(&[sel(1, 11, 1, 11)]);
    let op = crate::cursor::type_with_language(
        &ed.config,
        &ed.buffer,
        &cursors,
        "\"",
        None,
        Some(&lang),
    );
    assert_eq!(op.edits, vec![(pos(1, 11)..pos(1, 11), "\"".to_string())]);
}

#[test]
fn matching_bracket_finds_adjacent_and_enclosing_pairs() {
    let ed = TestEditor::lines(&["fn f(a: [u8; 2]) {", "    g(\")\");", "}"]);
    let config = gpui_monaco_languages::rust().config;
    // `"` strings: the `)` inside `")"` on line 2 is skipped.
    let skip = |p: Position| p.row == 1 && (6..=8).contains(&p.column);
    let m = |line, column| matching_bracket(&ed.buffer, &config, pos(line, column), &skip);

    // Right after `(`: its pair.
    assert_eq!(
        m(1, 5),
        Some((pos(1, 5)..pos(1, 6), pos(1, 16)..pos(1, 17)))
    );
    // Right before `[`.
    assert_eq!(
        m(1, 9),
        Some((pos(1, 9)..pos(1, 10), pos(1, 15)..pos(1, 16)))
    );
    // Inside `g(...)`: the skipped `)` in the string does not count.
    assert_eq!(
        m(2, 7),
        Some((pos(2, 6)..pos(2, 7), pos(2, 10)..pos(2, 11)))
    );
    // Between brackets but not next to one: the enclosing `{ }`.
    assert_eq!(
        m(2, 2),
        Some((pos(1, 18)..pos(1, 19), pos(3, 1)..pos(3, 2)))
    );
    // Outside of everything.
    assert_eq!(
        m(3, 2),
        Some((pos(1, 18)..pos(1, 19), pos(3, 1)..pos(3, 2)))
    );
    let plain = TestEditor::lines(&["abc"]);
    assert_eq!(
        matching_bracket(&plain.buffer, &config, pos(1, 2), &skip),
        None
    );
}

#[test]
fn next_bracket_skips_skippable_and_unmatched_closing_brackets() {
    use crate::cursor::{enclosing_brackets, match_bracket, next_bracket};
    let ed = TestEditor::lines(&["a ) \"(\" b", "  [c] (d"]);
    let config = gpui_monaco_languages::rust().config;
    let skip = |p: Position| p.row == 0 && p.column == 5;
    // Not next to a bracket, and nothing encloses it.
    assert_eq!(match_bracket(&ed.buffer, &config, pos(1, 2), &skip), None);
    assert_eq!(
        enclosing_brackets(&ed.buffer, &config, pos(1, 2), &skip),
        None
    );
    // The unmatched `)` and the `(` in the string are passed over.
    assert_eq!(
        next_bracket(&ed.buffer, &config, pos(1, 2), &skip),
        Some(pos(2, 3)..pos(2, 4))
    );
    // A matched closing bracket counts, and so does an unclosed opening one.
    assert_eq!(
        next_bracket(&ed.buffer, &config, pos(2, 5), &skip),
        Some(pos(2, 5)..pos(2, 6))
    );
    assert_eq!(
        next_bracket(&ed.buffer, &config, pos(2, 7), &skip),
        Some(pos(2, 7)..pos(2, 8))
    );
    assert_eq!(next_bracket(&ed.buffer, &config, pos(2, 8), &skip), None);
}
