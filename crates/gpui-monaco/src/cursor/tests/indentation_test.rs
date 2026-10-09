//! Port of the typing tests of Monaco's
//! `src/vs/editor/contrib/indentation/test/browser/indentation.test.ts`
//! (MIT): suites `Auto Indent On Type - TypeScript/JavaScript`, `- Ruby`,
//! `- PHP` and `- Visual Basic`. The language configurations are the test
//! modes of `src/vs/editor/test/common/modes/supports/` (see
//! `language_configs.rs`).
//!
//! Skipped:
//! - `test.skip` in Monaco (known failures): the `issue #43244` method
//!   chain/assignment tests, `issue #40115`, `issue #193875`, `issue #67678`,
//!   `issue #46401`, `issue #125261`, `issue #199846`.
//! - Other suites: `Change Indentation to Spaces/Tabs`, `Indent With Tab`
//!   and `Auto Indent On Paste` test the indentation contribution's commands
//!   (convert indentation, reindent, auto-indent on paste), not typing; the
//!   remaining `Auto Indent On Type` suites (Go, CPP, HTML, Latex, Lua) only
//!   have placeholder or skipped tests.

use syntax::LanguageConfiguration;

use super::harness::{TestEditor, sel};
use super::language_configs::*;
use super::tokenizer::Tokenizer;
use crate::cursor::{AutoIndent, CursorConfig, StandardTokenType};

fn editor(text: &[&str], language: LanguageConfiguration, auto_indent: AutoIndent) -> TestEditor {
    let config = CursorConfig {
        auto_indent,
        ..CursorConfig::default()
    };
    TestEditor::lines_with_config(text, config).with_language(language)
}

fn ts(text: &[&str]) -> TestEditor {
    editor(text, typescript_test_language(), AutoIndent::Full)
}

// ---------------------------------------------------------------------------
// TypeScript/JavaScript

#[test]
fn issue_208215_indent_after_arrow_function() {
    let mut ed = ts(&[""]);
    ed.type_raw("const add1 = (n) =>");
    ed.type_keyboard("\n");
    assert_eq!(ed.value(), ["const add1 = (n) =>", "    "].join("\n"));
}

#[test]
fn issue_208215_indent_after_arrow_function_2() {
    let mut ed = ts(&["const array = [1, 2, 3, 4, 5];", "array.map(", "    v =>"]);
    ed.set_selection(sel(3, 9, 3, 9));
    ed.type_keyboard("\n");
    assert_eq!(
        ed.value(),
        [
            "const array = [1, 2, 3, 4, 5];",
            "array.map(",
            "    v =>",
            "        ",
        ]
        .join("\n")
    );
}

#[test]
fn issue_116843_indent_after_arrow_function() {
    let mut ed = ts(&[""]);
    ed.type_raw(&["const add1 = (n) =>", "    n + 1;"].join("\n"));
    ed.type_keyboard("\n");
    assert_eq!(
        ed.value(),
        ["const add1 = (n) =>", "    n + 1;", ""].join("\n")
    );
}

#[test]
fn issue_29755_do_not_add_indentation_on_enter_if_indentation_is_already_valid() {
    let mut ed = ts(&[
        "function f() {",
        "    const one = 1;",
        "    const two = 2;",
        "}",
    ]);
    ed.set_selection(sel(3, 1, 3, 1));
    ed.type_keyboard("\n");
    assert_eq!(
        ed.value(),
        [
            "function f() {",
            "    const one = 1;",
            "",
            "    const two = 2;",
            "}",
        ]
        .join("\n")
    );
}

#[test]
fn issue_36090() {
    let mut ed = editor(
        &[
            "class ItemCtrl {",
            "    getPropertiesByItemId(id) {",
            "        return this.fetchItem(id)",
            "            .then(item => {",
            "                return this.getPropertiesOfItem(item);",
            "            });",
            "    }",
            "}",
        ],
        typescript_test_language(),
        AutoIndent::Advanced,
    );
    ed.set_selection(sel(7, 6, 7, 6));
    ed.type_keyboard("\n");
    assert_eq!(
        ed.value(),
        [
            "class ItemCtrl {",
            "    getPropertiesByItemId(id) {",
            "        return this.fetchItem(id)",
            "            .then(item => {",
            "                return this.getPropertiesOfItem(item);",
            "            });",
            "    }",
            "    ",
            "}",
        ]
        .join("\n")
    );
    ed.assert_selection(sel(8, 5, 8, 5));
}

#[test]
fn issue_115304_indent_block_comment_on_enter() {
    let mut ed = editor(
        &["/** */", "function f() {}"],
        typescript_test_language(),
        AutoIndent::Advanced,
    );
    ed.set_selection(sel(1, 4, 1, 4));
    ed.type_keyboard("\n");
    assert_eq!(
        ed.value(),
        ["/**", " * ", " */", "function f() {}"].join("\n")
    );
    ed.assert_selection(sel(2, 4, 2, 4));
}

#[test]
fn issue_43244_indent_when_lambda_arrow_function_is_detected_outdent_when_end_is_reached() {
    let mut ed = ts(&["const array = [1, 2, 3, 4, 5];", "array.map(_)"]);
    ed.set_selection(sel(2, 12, 2, 12));
    ed.type_keyboard("\n");
    assert_eq!(
        ed.value(),
        ["const array = [1, 2, 3, 4, 5];", "array.map(_", "    ", ")"].join("\n")
    );
}

#[test]
fn issue_43244_incorrect_indentation_after_if_for_while_without_braces() {
    let mut ed = ts(&["function f() {", "    if (condition)", "}"]);
    ed.set_selection(sel(2, 19, 2, 19));
    ed.type_keyboard("\n");
    assert_eq!(
        ed.value(),
        ["function f() {", "    if (condition)", "        ", "}"].join("\n")
    );

    ed.type_raw("return;");
    ed.type_keyboard("\n");
    assert_eq!(
        ed.value(),
        [
            "function f() {",
            "    if (condition)",
            "        return;",
            "    ",
            "}",
        ]
        .join("\n")
    );
}

#[test]
fn issue_208232_incorrect_indentation_inside_of_comments() {
    let comment = vec![(0, StandardTokenType::Comment)];
    let mut ed =
        ts(&["/**", "indentation done for {", "*/"]).with_tokenizer(Tokenizer::Fixed(vec![
            comment.clone(),
            comment.clone(),
            comment,
        ]));
    ed.set_selection(sel(2, 23, 2, 23));
    ed.type_keyboard("\n");
    assert_eq!(
        ed.value(),
        ["/**", "indentation done for {", "", "*/"].join("\n")
    );
}

#[test]
fn issue_209802_allman_style_braces_in_javascript() {
    let mut ed = ts(&["if (/*condition*/)"]);
    ed.set_selection(sel(1, 19, 1, 19));
    ed.type_keyboard("\n");
    assert_eq!(ed.value(), ["if (/*condition*/)", "    "].join("\n"));
    ed.type_keyboard("{");
    assert_eq!(ed.value(), ["if (/*condition*/)", "{}"].join("\n"));
    ed.set_selection(sel(2, 2, 2, 2));
    ed.type_keyboard("\n");
    assert_eq!(
        ed.value(),
        ["if (/*condition*/)", "{", "    ", "}"].join("\n")
    );
}

#[test]
fn issue_225916_surrounding_with_brackets_works_after_arrow_function() {
    for (open, close) in [("{", "}"), ("[", "]"), ("(", ")")] {
        let mut ed = ts(&["const fn = () =>", "    doSomething()"]);
        ed.set_selection(sel(2, 5, 2, 18));
        ed.type_keyboard(open);
        assert_eq!(
            ed.value(),
            [
                "const fn = () =>".to_string(),
                format!("    {open}doSomething(){close}"),
            ]
            .join("\n"),
            "failed to surround selection with {open}{close}"
        );
    }
}

// ---------------------------------------------------------------------------
// Ruby

#[test]
fn issue_198350_in_or_when_incorrectly_match_non_keywords_for_ruby() {
    let mut ed = editor(&[""], ruby_test_language(), AutoIndent::Full);
    ed.type_raw("def foo\n        i");
    ed.type_keyboard("n");
    assert_eq!(ed.value(), "def foo\n        in");
    ed.type_keyboard(" ");
    assert_eq!(ed.value(), "def foo\nin ");

    ed.set_value("");
    ed.type_raw("  # in");
    assert_eq!(ed.value(), "  # in");
    ed.type_keyboard(" ");
    assert_eq!(ed.value(), "  # in ");
}

// ---------------------------------------------------------------------------
// PHP

#[test]
fn issue_199050_should_not_indent_after_brace_detected_in_a_string() {
    let mut ed = editor(
        &["preg_replace('{');"],
        php_test_language(),
        AutoIndent::Full,
    )
    .with_tokenizer(Tokenizer::Fixed(vec![vec![
        (0, StandardTokenType::Other),
        (13, StandardTokenType::String),
        (16, StandardTokenType::Other),
    ]]));
    ed.set_selection(sel(1, 54, 1, 54));
    ed.type_keyboard("\n");
    assert_eq!(ed.value(), ["preg_replace('{');", ""].join("\n"));
}

// ---------------------------------------------------------------------------
// Visual Basic

/// One VB test: start text, cursor, text typed without interceptors, the
/// keyboard character, and the expected text.
#[track_caller]
fn vb(text: &[&str], cursor: Option<(u32, u32)>, typed: &str, keyboard: &str, expected: &[&str]) {
    let mut ed = editor(text, vb_test_language(), AutoIndent::Full);
    if let Some((line, column)) = cursor {
        ed.set_selection(sel(line, column, line, column));
    }
    if !typed.is_empty() {
        ed.type_raw(typed);
    }
    ed.type_keyboard(keyboard);
    assert_eq!(ed.value(), expected.join("\n"));
}

#[test]
fn issue_118932_no_indentation_in_visual_basic_files() {
    vb(
        &["If True Then", "    Some code", "    End I"],
        Some((3, 10)),
        "",
        "f",
        &["If True Then", "    Some code", "End If"],
    );
}

#[test]
fn issue_118932_indent_after_module_declaration() {
    vb(&[""], None, "Module Test", "\n", &["Module Test", "    "]);
}

#[test]
fn issue_118932_indent_after_sub_declaration() {
    vb(
        &["Module Test", "    "],
        Some((2, 5)),
        "Sub Main()",
        "\n",
        &["Module Test", "    Sub Main()", "        "],
    );
}

#[test]
fn issue_118932_dedent_on_end_sub() {
    vb(
        &[
            "Module Test",
            "    Sub Main()",
            "        Console.WriteLine(\"Hello\")",
            "        End Su",
        ],
        Some((4, 15)),
        "",
        "b",
        &[
            "Module Test",
            "    Sub Main()",
            "        Console.WriteLine(\"Hello\")",
            "    End Sub",
        ],
    );
}

#[test]
fn issue_118932_dedent_on_end_module() {
    vb(
        &["Module Test", "    Private x As Integer", "    End Modul"],
        Some((3, 14)),
        "",
        "e",
        &["Module Test", "    Private x As Integer", "End Module"],
    );
}

#[test]
fn issue_118932_indent_after_function_declaration() {
    vb(
        &["Module Test", "    "],
        Some((2, 5)),
        "Function Add(a As Integer, b As Integer) As Integer",
        "\n",
        &[
            "Module Test",
            "    Function Add(a As Integer, b As Integer) As Integer",
            "        ",
        ],
    );
}

#[test]
fn issue_118932_dedent_on_end_function() {
    vb(
        &[
            "Module Test",
            "    Function Add(a, b)",
            "        Return a + b",
            "        End Functio",
        ],
        Some((4, 20)),
        "",
        "n",
        &[
            "Module Test",
            "    Function Add(a, b)",
            "        Return a + b",
            "    End Function",
        ],
    );
}

#[test]
fn issue_118932_indent_after_if_then() {
    vb(
        &["Sub Test()", "    "],
        Some((2, 5)),
        "If x > 0 Then",
        "\n",
        &["Sub Test()", "    If x > 0 Then", "        "],
    );
}

#[test]
fn issue_118932_indent_after_else_if_then() {
    vb(
        &[
            "Sub Test()",
            "    If x > 0 Then",
            "        DoSomething()",
            "    ElseIf x < 0 Then",
        ],
        Some((4, 22)),
        "",
        "\n",
        &[
            "Sub Test()",
            "    If x > 0 Then",
            "        DoSomething()",
            "    ElseIf x < 0 Then",
            "        ",
        ],
    );
}

#[test]
fn issue_118932_dedent_and_indent_on_else() {
    vb(
        &[
            "Sub Test()",
            "    If x > 0 Then",
            "        DoSomething()",
            "        Els",
        ],
        Some((4, 12)),
        "",
        "e",
        &[
            "Sub Test()",
            "    If x > 0 Then",
            "        DoSomething()",
            "    Else",
        ],
    );
}

#[test]
fn issue_118932_indent_after_while() {
    vb(
        &["Sub Test()", "    "],
        Some((2, 5)),
        "While x > 0",
        "\n",
        &["Sub Test()", "    While x > 0", "        "],
    );
}

#[test]
fn issue_118932_dedent_on_end_while() {
    vb(
        &[
            "Sub Test()",
            "    While x > 0",
            "        x = x - 1",
            "        End Whil",
        ],
        Some((4, 17)),
        "",
        "e",
        &[
            "Sub Test()",
            "    While x > 0",
            "        x = x - 1",
            "    End While",
        ],
    );
}

#[test]
fn issue_118932_indent_after_for() {
    vb(
        &["Sub Test()", "    "],
        Some((2, 5)),
        "For i = 1 To 10",
        "\n",
        &["Sub Test()", "    For i = 1 To 10", "        "],
    );
}

#[test]
fn issue_118932_dedent_on_next() {
    vb(
        &[
            "Sub Test()",
            "    For i = 1 To 10",
            "        DoSomething(i)",
            "        Nex",
        ],
        Some((4, 12)),
        "",
        "t",
        &[
            "Sub Test()",
            "    For i = 1 To 10",
            "        DoSomething(i)",
            "    Next",
        ],
    );
}

#[test]
fn issue_118932_indent_after_do() {
    vb(
        &["Sub Test()", "    "],
        Some((2, 5)),
        "Do",
        "\n",
        &["Sub Test()", "    Do", "        "],
    );
}

#[test]
fn issue_118932_dedent_on_loop() {
    vb(
        &["Sub Test()", "    Do", "        x = x + 1", "        Loo"],
        Some((4, 12)),
        "",
        "p",
        &["Sub Test()", "    Do", "        x = x + 1", "    Loop"],
    );
}

#[test]
fn issue_118932_indent_after_select_case() {
    vb(
        &["Sub Test()", "    "],
        Some((2, 5)),
        "Select Case x",
        "\n",
        &["Sub Test()", "    Select Case x", "        "],
    );
}

#[test]
fn issue_118932_dedent_on_end_select() {
    vb(
        &["Sub Test()", "    Select Case x", "        End Selec"],
        Some((3, 18)),
        "",
        "t",
        &["Sub Test()", "    Select Case x", "    End Select"],
    );
}

#[test]
fn issue_118932_indent_after_try() {
    vb(
        &["Sub Test()", "    "],
        Some((2, 5)),
        "Try",
        "\n",
        &["Sub Test()", "    Try", "        "],
    );
}

#[test]
fn issue_118932_dedent_and_indent_on_catch() {
    vb(
        &[
            "Sub Test()",
            "    Try",
            "        DoSomething()",
            "        Catc",
        ],
        Some((4, 13)),
        "",
        "h",
        &[
            "Sub Test()",
            "    Try",
            "        DoSomething()",
            "    Catch",
        ],
    );
}

#[test]
fn issue_118932_dedent_and_indent_on_finally() {
    vb(
        &[
            "Sub Test()",
            "    Try",
            "        DoSomething()",
            "    Catch",
            "        HandleError()",
            "        Finall",
        ],
        Some((6, 15)),
        "",
        "y",
        &[
            "Sub Test()",
            "    Try",
            "        DoSomething()",
            "    Catch",
            "        HandleError()",
            "    Finally",
        ],
    );
}

#[test]
fn issue_118932_dedent_on_end_try() {
    vb(
        &[
            "Sub Test()",
            "    Try",
            "        DoSomething()",
            "    Catch",
            "        HandleError()",
            "    Finally",
            "        Cleanup()",
            "        End Tr",
        ],
        Some((8, 15)),
        "",
        "y",
        &[
            "Sub Test()",
            "    Try",
            "        DoSomething()",
            "    Catch",
            "        HandleError()",
            "    Finally",
            "        Cleanup()",
            "    End Try",
        ],
    );
}

#[test]
fn issue_118932_indent_after_class() {
    vb(
        &[""],
        None,
        "Class MyClass",
        "\n",
        &["Class MyClass", "    "],
    );
}

#[test]
fn issue_118932_dedent_on_end_class() {
    vb(
        &["Class MyClass", "    Private x As Integer", "    End Clas"],
        Some((3, 14)),
        "",
        "s",
        &["Class MyClass", "    Private x As Integer", "End Class"],
    );
}

#[test]
fn issue_118932_full_program_indentation_flow() {
    let mut ed = editor(&[""], vb_test_language(), AutoIndent::Full);
    ed.type_raw("Module Test");
    ed.type_keyboard("\n");
    assert_eq!(
        ed.value(),
        ["Module Test", "    "].join("\n"),
        "After Module Test"
    );
    ed.type_raw("Sub Main()");
    ed.type_keyboard("\n");
    assert_eq!(
        ed.value(),
        ["Module Test", "    Sub Main()", "        "].join("\n"),
        "After Sub Main()"
    );
    ed.type_raw("Console.WriteLine(\"Hello, World!\")");
    ed.type_keyboard("\n");
    assert_eq!(
        ed.value(),
        [
            "Module Test",
            "    Sub Main()",
            "        Console.WriteLine(\"Hello, World!\")",
            "        ",
        ]
        .join("\n"),
        "After Console.WriteLine"
    );
    ed.type_raw("End Su");
    ed.type_keyboard("b");
    assert_eq!(
        ed.value(),
        [
            "Module Test",
            "    Sub Main()",
            "        Console.WriteLine(\"Hello, World!\")",
            "    End Sub",
        ]
        .join("\n"),
        "After End Sub"
    );
    ed.type_keyboard("\n");
    assert_eq!(
        ed.value(),
        [
            "Module Test",
            "    Sub Main()",
            "        Console.WriteLine(\"Hello, World!\")",
            "    End Sub",
            "    ",
        ]
        .join("\n"),
        "After Enter after End Sub"
    );
}
