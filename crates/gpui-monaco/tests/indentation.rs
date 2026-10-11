//! Indentation commands and detection: ports of Monaco's
//! `contrib/indentation/test/browser/indentation.test.ts` suites "Change
//! Indentation to Spaces" and "Change Indentation to Tabs" (MIT).

use gpui::{AppContext as _, Entity, Focusable as _, TestAppContext, VisualTestContext};
use gpui_monaco::cursor::Selection;
use gpui_monaco::text::PointUtf16;
use gpui_monaco::{CodeEditor, TextModel};

fn editor<'a>(
    text: &str,
    cx: &'a mut TestAppContext,
) -> (Entity<CodeEditor>, &'a mut VisualTestContext) {
    cx.update(gpui_monaco::init);
    let text = text.to_string();
    let (editor, cx) = cx.add_window_view(|window, cx| {
        let model = cx.new(|_| TextModel::new(&text));
        CodeEditor::new(model, window, cx)
    });
    cx.update(|window, cx| window.focus(&editor.read(cx).focus_handle(cx), cx));
    (editor, cx)
}

/// Monaco `testIndentationToSpacesCommand` / `testIndentationToTabsCommand`
/// (1-based positions).
fn check(
    to_spaces: bool,
    lines: &[&str],
    selection: (u32, u32),
    tab_size: u32,
    expected: &[&str],
    expected_selection: (u32, u32),
    cx: &mut TestAppContext,
) {
    let (editor, cx) = editor(&lines.join("\n"), cx);
    let model = editor.read_with(cx, |e, _| e.model().clone());
    model.update(cx, |m, cx| {
        let mut options = m.options().clone();
        options.tab_size = tab_size;
        m.update_options(options, cx);
    });
    let p = |(line, column): (u32, u32)| PointUtf16::from_monaco(line, column);
    editor.update(cx, |e, cx| {
        e.set_selections(&[Selection::collapsed(p(selection))], cx);
        if to_spaces {
            e.indentation_to_spaces(cx);
        } else {
            e.indentation_to_tabs(cx);
        }
    });
    assert_eq!(model.read_with(cx, |m, _| m.text()), expected.join("\n"));
    assert_eq!(
        editor.read_with(cx, |e, _| e.selections()[0]),
        Selection::collapsed(p(expected_selection))
    );
    assert_eq!(
        model.read_with(cx, |m, _| m.options().insert_spaces),
        to_spaces
    );
}

// Monaco: `single tabs only at start of line`.
#[gpui::test]
fn to_spaces_single_tabs_only_at_start_of_line(cx: &mut TestAppContext) {
    check(
        true,
        &[
            "first",
            "second line",
            "third line",
            "\tfourth line",
            "\tfifth",
        ],
        (2, 3),
        4,
        &[
            "first",
            "second line",
            "third line",
            "    fourth line",
            "    fifth",
        ],
        (2, 3),
        cx,
    );
}

// Monaco: `multiple tabs at start of line`.
#[gpui::test]
fn to_spaces_multiple_tabs_at_start_of_line(cx: &mut TestAppContext) {
    check(
        true,
        &[
            "\t\tfirst",
            "\tsecond line",
            "\t\t\t third line",
            "fourth line",
            "fifth",
        ],
        (1, 5),
        3,
        &[
            "      first",
            "   second line",
            "          third line",
            "fourth line",
            "fifth",
        ],
        (1, 9),
        cx,
    );
}

// Monaco: `multiple tabs`.
#[gpui::test]
fn to_spaces_multiple_tabs(cx: &mut TestAppContext) {
    check(
        true,
        &[
            "\t\tfirst\t",
            "\tsecond  \t line \t",
            "\t\t\t third line",
            " \tfourth line",
            "fifth",
        ],
        (1, 5),
        2,
        &[
            "    first\t",
            "  second  \t line \t",
            "       third line",
            "   fourth line",
            "fifth",
        ],
        (1, 7),
        cx,
    );
}

// Monaco: `empty lines`.
#[gpui::test]
fn to_spaces_empty_lines(cx: &mut TestAppContext) {
    check(
        true,
        &["\t\t\t", "\t", "\t\t"],
        (1, 4),
        2,
        &["      ", "  ", "    "],
        (1, 4),
        cx,
    );
}

// Monaco: `spaces only at start of line`.
#[gpui::test]
fn to_tabs_spaces_only_at_start_of_line(cx: &mut TestAppContext) {
    check(
        false,
        &[
            "    first",
            "second line",
            "    third line",
            "fourth line",
            "fifth",
        ],
        (2, 3),
        4,
        &[
            "\tfirst",
            "second line",
            "\tthird line",
            "fourth line",
            "fifth",
        ],
        (2, 3),
        cx,
    );
}

// Monaco: `multiple spaces at start of line`.
#[gpui::test]
fn to_tabs_multiple_spaces_at_start_of_line(cx: &mut TestAppContext) {
    check(
        false,
        &[
            "first",
            "   second line",
            "          third line",
            "fourth line",
            "     fifth",
        ],
        (1, 5),
        3,
        &[
            "first",
            "\tsecond line",
            "\t\t\t third line",
            "fourth line",
            "\t  fifth",
        ],
        (1, 5),
        cx,
    );
}

// Monaco: `multiple spaces`.
#[gpui::test]
fn to_tabs_multiple_spaces(cx: &mut TestAppContext) {
    check(
        false,
        &[
            "      first   ",
            "  second     line \t",
            "       third line",
            "   fourth line",
            "fifth",
        ],
        (1, 8),
        2,
        &[
            "\t\t\tfirst   ",
            "\tsecond     line \t",
            "\t\t\t third line",
            "\t fourth line",
            "fifth",
        ],
        (1, 5),
        cx,
    );
}

// Monaco: `issue #45996`.
#[gpui::test]
fn to_spaces_issue_45996(cx: &mut TestAppContext) {
    check(true, &["\tabc"], (1, 3), 4, &["    abc"], (1, 6), cx);
}

#[gpui::test]
fn new_models_detect_indentation(cx: &mut TestAppContext) {
    // Monaco's `detectIndentation` (default on) guesses from the text.
    let model = cx.new(|_| TextModel::new("fn a() {\n\tb();\n\tc();\n}\n"));
    model.read_with(cx, |m, _| assert!(!m.options().insert_spaces));
    let model = cx.new(|_| TextModel::new("a:\n  b:\n    c: 1\n  d: 2\n"));
    model.read_with(cx, |m, _| {
        assert!(m.options().insert_spaces);
        assert_eq!(m.options().tab_size, 2);
        assert_eq!(m.options().indent_size, 2);
    });
    // Turned off, the given options stay.
    let model = cx.new(|_| {
        TextModel::new("").with_options(gpui_monaco::ModelOptions {
            detect_indentation: false,
            insert_spaces: false,
            ..Default::default()
        })
    });
    model.update(cx, |m, cx| {
        m.set_value("x\n  y\n  z\n", cx);
        assert!(!m.options().insert_spaces);
        // The command guesses again.
        m.detect_indentation(true, 4, cx);
        assert!(m.options().insert_spaces);
        assert_eq!(m.options().tab_size, 2);
    });
}

#[gpui::test]
fn choosing_the_indent_size(cx: &mut TestAppContext) {
    use gpui_monaco::actions::{ChangeTabDisplaySize, IndentUsingTabs};
    let (editor, cx) = editor("a\n    b\n", cx);
    let model = editor.read_with(cx, |e, _| e.model().clone());
    // Monaco's picker lists 1 to 8; picking 2 indents with tabs of 2.
    cx.dispatch_action(IndentUsingTabs);
    cx.run_until_parked();
    cx.simulate_input("2");
    cx.simulate_keystrokes("enter");
    cx.run_until_parked();
    model.read_with(cx, |m, _| {
        let o = m.options();
        assert_eq!((o.insert_spaces, o.tab_size, o.indent_size), (false, 2, 2));
    });
    // The display size alone leaves indenting as it was.
    cx.update(|window, cx| window.focus(&editor.read(cx).focus_handle(cx), cx));
    cx.dispatch_action(ChangeTabDisplaySize);
    cx.run_until_parked();
    cx.simulate_input("8");
    cx.simulate_keystrokes("enter");
    cx.run_until_parked();
    model.read_with(cx, |m, _| {
        let o = m.options();
        assert_eq!((o.insert_spaces, o.tab_size, o.indent_size), (false, 8, 2));
    });
}
