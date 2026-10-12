//! Typing behavior that needs the whole editor: Insert Line with the
//! language's rules, indenting doc comments, the `type` and composition
//! commands' interceptors, auto-closed characters across edits from
//! elsewhere, and closing characters inserted by snippets.

use gpui::{
    AppContext as _, Entity, Focusable, TestAppContext, VisualContext as _, VisualTestContext,
};
use gpui_monaco::cursor::Selection;
use gpui_monaco::text::PointUtf16;
use gpui_monaco::{CodeEditor, SnippetInsertOptions, TextModel};

fn p(line: u32, column: u32) -> PointUtf16 {
    PointUtf16::from_monaco(line, column)
}

fn js_editor<'a>(
    text: &str,
    cx: &'a mut TestAppContext,
) -> (Entity<CodeEditor>, &'a mut VisualTestContext) {
    cx.update(|cx| {
        gpui_monaco::init(cx);
        gpui_monaco::set_language_registry(cx, gpui_monaco_languages::registry());
    });
    let text = text.to_string();
    let (editor, cx) = cx.add_window_view(|window, cx| {
        let model = cx.new(|_| TextModel::new(&text).with_language("javascript"));
        CodeEditor::new(model, window, cx)
    });
    cx.update(|window, cx| window.focus(&editor.focus_handle(cx), cx));
    cx.run_until_parked();
    (editor, cx)
}

fn text(editor: &Entity<CodeEditor>, cx: &mut VisualTestContext) -> String {
    editor.read_with(cx, |e, cx| e.model().read(cx).text())
}

fn set(editor: &Entity<CodeEditor>, selection: Selection, cx: &mut VisualTestContext) {
    editor.update(cx, |e, cx| e.set_selections(&[selection], cx));
}

fn heads(editor: &Entity<CodeEditor>, cx: &mut VisualTestContext) -> Vec<(u32, u32)> {
    editor.read_with(cx, |e, _| {
        e.selections().iter().map(|s| s.head.to_monaco()).collect()
    })
}

fn trigger(
    editor: &Entity<CodeEditor>,
    name: &str,
    payload: serde_json::Value,
    cx: &mut VisualTestContext,
) {
    editor.update_in(cx, |e, window, cx| {
        e.trigger_with_payload(name, payload, window, cx).unwrap()
    });
    cx.run_until_parked();
}

/// Monaco `editor.action.insertLineAfter`/`insertLineBefore` run Enter's
/// onEnter and indentation rules at the end of the line.
#[gpui::test]
fn insert_line_uses_the_language_rules(cx: &mut TestAppContext) {
    use gpui_monaco::actions::{InsertLineAfter, InsertLineBefore};
    let (editor, cx) = js_editor("if (x) {\n}", cx);
    set(&editor, Selection::collapsed(p(1, 3)), cx);
    cx.dispatch_action(InsertLineAfter);
    assert_eq!(text(&editor, cx), "if (x) {\n    \n}");
    assert_eq!(heads(&editor, cx), vec![(2, 5)]);

    editor.update(cx, |e, cx| e.set_value("/**\n * a\n */", cx));
    set(&editor, Selection::collapsed(p(3, 1)), cx);
    cx.dispatch_action(InsertLineBefore);
    assert_eq!(text(&editor, cx), "/**\n * a\n * \n */");
    assert_eq!(heads(&editor, cx), vec![(3, 4)]);
}

/// Tab and Shift+Tab on a selected doc comment keep the ` * ` lines'
/// alignment space (Monaco `ShiftCommand`), as do Indent and Outdent
/// Lines.
#[gpui::test]
fn indenting_keeps_doc_comment_alignment(cx: &mut TestAppContext) {
    use gpui_monaco::actions::{IndentLines, OutdentLines};
    let (editor, cx) = js_editor("/**\n * a\n */\nx();", cx);
    set(
        &editor,
        Selection {
            anchor: p(1, 1),
            head: p(4, 5),
        },
        cx,
    );
    cx.simulate_keystrokes("tab");
    assert_eq!(text(&editor, cx), "    /**\n     * a\n     */\n    x();");
    cx.simulate_keystrokes("shift-tab");
    assert_eq!(text(&editor, cx), "/**\n * a\n */\nx();");
    // Outdenting the unindented comment changes nothing.
    cx.simulate_keystrokes("shift-tab");
    assert_eq!(text(&editor, cx), "/**\n * a\n */\nx();");

    cx.dispatch_action(IndentLines);
    assert_eq!(text(&editor, cx), "    /**\n     * a\n     */\n    x();");
    cx.dispatch_action(OutdentLines);
    assert_eq!(text(&editor, cx), "/**\n * a\n */\nx();");
}

/// `trigger('keyboard', 'type', { text })` types each character through the
/// interceptors, as Monaco's `Cursor.type` does.
#[gpui::test]
fn type_command_runs_the_interceptors_per_character(cx: &mut TestAppContext) {
    let (editor, cx) = js_editor("", cx);
    trigger(&editor, "type", serde_json::json!({ "text": "f(" }), cx);
    assert_eq!(text(&editor, cx), "f()");
    assert_eq!(heads(&editor, cx), vec![(1, 3)]);
}

/// The `compositionStart`/`compositionEnd` commands: typing in between
/// skips the interceptors, and the end runs them.
#[gpui::test]
fn composition_commands_run_the_interceptors_at_the_end(cx: &mut TestAppContext) {
    let (editor, cx) = js_editor("", cx);
    trigger(&editor, "compositionStart", serde_json::json!({}), cx);
    trigger(&editor, "type", serde_json::json!({ "text": "'" }), cx);
    assert_eq!(text(&editor, cx), "'");
    trigger(&editor, "compositionEnd", serde_json::json!({}), cx);
    assert_eq!(text(&editor, cx), "''");
    assert_eq!(heads(&editor, cx), vec![(1, 2)]);
}

/// Auto-closed characters stay tracked through edits that don't come from
/// the editor's typing: another editor on the model, `apply_edits`, undo
/// (Monaco keeps them as model decorations).
#[gpui::test]
fn auto_closed_characters_survive_edits_from_elsewhere(cx: &mut TestAppContext) {
    let (editor, cx) = js_editor("foo\nbar", cx);
    set(&editor, Selection::collapsed(p(1, 4)), cx);
    cx.simulate_input("(");
    assert_eq!(text(&editor, cx), "foo()\nbar");

    // Another editor on the model types on another line.
    let model = editor.read_with(cx, |e, _| e.model().clone());
    let other = cx.new_window_entity(|window, cx| CodeEditor::new(model.clone(), window, cx));
    other.update(cx, |e, cx| {
        e.set_selections(&[Selection::collapsed(p(2, 4))], cx);
        e.type_text("x", cx);
    });
    // And the model is edited before the cursor, on its line.
    model.update(cx, |m, cx| {
        m.apply_edits(vec![(p(1, 1)..p(1, 1), "a".into())], cx)
            .unwrap();
    });
    cx.run_until_parked();
    assert_eq!(text(&editor, cx), "afoo()\nbarx");
    assert_eq!(heads(&editor, cx), vec![(1, 6)]);
    cx.simulate_input(")");
    assert_eq!(text(&editor, cx), "afoo()\nbarx");
    assert_eq!(heads(&editor, cx), vec![(1, 7)]);

    // Undo keeps them too.
    set(&editor, Selection::collapsed(p(2, 5)), cx);
    cx.simulate_input("[");
    cx.simulate_keystrokes("space");
    cx.simulate_input("1");
    assert_eq!(text(&editor, cx), "afoo()\nbarx[ 1]");
    // The space started a new undo step.
    cx.simulate_keystrokes("secondary-z");
    assert_eq!(text(&editor, cx), "afoo()\nbarx[]");
    assert_eq!(heads(&editor, cx), vec![(2, 6)]);
    cx.simulate_input("]");
    assert_eq!(text(&editor, cx), "afoo()\nbarx[]");
    assert_eq!(heads(&editor, cx), vec![(2, 7)]);
}

/// Closing characters a snippet inserts are tracked as auto-closed (Monaco
/// `executeEdits('snippet', ...)`, issue #78975), so typing them types over
/// them.
#[gpui::test]
fn snippet_closing_characters_are_auto_closed(cx: &mut TestAppContext) {
    let (editor, cx) = js_editor("<div ", cx);
    set(&editor, Selection::collapsed(p(1, 6)), cx);
    editor.update(cx, |e, cx| {
        e.insert_snippet("id=\"$1\"", SnippetInsertOptions::default(), cx)
    });
    assert_eq!(text(&editor, cx), "<div id=\"\"");
    assert_eq!(heads(&editor, cx), vec![(1, 10)]);
    cx.simulate_input("a\"");
    assert_eq!(text(&editor, cx), "<div id=\"a\"");
    assert_eq!(heads(&editor, cx), vec![(1, 12)]);

    editor.update(cx, |e, cx| e.set_value("", cx));
    editor.update(cx, |e, cx| {
        e.insert_snippet("log($1)", SnippetInsertOptions::default(), cx)
    });
    cx.simulate_input("x)");
    assert_eq!(text(&editor, cx), "log(x)");
}
