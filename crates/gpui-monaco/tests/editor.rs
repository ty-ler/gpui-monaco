//! Integration tests for `CodeEditor` running in a GPUI test window: input
//! routing, keymap, model synchronisation, undo cursor restore, clipboard and
//! IME composition.

use gpui::{
    AppContext as _, ClipboardItem, Entity, EntityInputHandler, Focusable, TestAppContext,
    VisualTestContext,
};
use gpui_monaco::cursor::Selection;
use gpui_monaco::text::PointUtf16;
use gpui_monaco::{CodeEditor, EditorOptions, TextModel};

fn p(line: u32, column: u32) -> PointUtf16 {
    PointUtf16::from_monaco(line, column)
}

fn editor_with_text<'a>(
    text: &str,
    cx: &'a mut TestAppContext,
) -> (Entity<CodeEditor>, &'a mut VisualTestContext) {
    cx.update(gpui_monaco::init);
    let text = text.to_string();
    let (editor, cx) = cx.add_window_view(|window, cx| CodeEditor::with_text(&text, window, cx));
    cx.update(|window, cx| window.focus(&editor.focus_handle(cx), cx));
    cx.run_until_parked();
    (editor, cx)
}

fn text(editor: &Entity<CodeEditor>, cx: &mut VisualTestContext) -> String {
    editor.read_with(cx, |editor, cx| editor.model().read(cx).text())
}

fn heads(editor: &Entity<CodeEditor>, cx: &mut VisualTestContext) -> Vec<(u32, u32)> {
    editor.read_with(cx, |editor, _| {
        editor
            .selections()
            .iter()
            .map(|s| s.head.to_monaco())
            .collect()
    })
}

#[gpui::test]
fn typing_and_keymap(cx: &mut TestAppContext) {
    let (editor, cx) = editor_with_text("", cx);
    cx.simulate_input("hello");
    assert_eq!(text(&editor, cx), "hello");
    cx.simulate_keystrokes("enter");
    cx.simulate_input("world");
    assert_eq!(text(&editor, cx), "hello\nworld");
    cx.simulate_keystrokes("left left backspace");
    assert_eq!(text(&editor, cx), "hello\nwold");
    assert_eq!(heads(&editor, cx), vec![(2, 3)]);
    cx.simulate_keystrokes("up");
    assert_eq!(heads(&editor, cx), vec![(1, 3)]);
    cx.simulate_keystrokes("secondary-a backspace");
    assert_eq!(text(&editor, cx), "");
}

#[gpui::test]
fn undo_groups_typing_and_restores_cursor(cx: &mut TestAppContext) {
    let (editor, cx) = editor_with_text("x", cx);
    cx.simulate_keystrokes("secondary-right");
    cx.simulate_input("abc");
    cx.simulate_keystrokes("left");
    cx.simulate_input("Z");
    assert_eq!(text(&editor, cx), "xabZc");
    cx.simulate_keystrokes("secondary-z");
    assert_eq!(text(&editor, cx), "xabc");
    assert_eq!(heads(&editor, cx), vec![(1, 4)]);
    cx.simulate_keystrokes("secondary-z");
    assert_eq!(text(&editor, cx), "x");
    assert_eq!(heads(&editor, cx), vec![(1, 2)]);
    redo(&editor, cx);
    assert_eq!(text(&editor, cx), "xabc");
}

fn redo(editor: &Entity<CodeEditor>, cx: &mut VisualTestContext) {
    editor.update(cx, |editor, cx| editor.redo(cx));
    cx.run_until_parked();
}

#[gpui::test]
fn two_editors_share_a_model(cx: &mut TestAppContext) {
    cx.update(gpui_monaco::init);
    let model = cx.new(|_| TextModel::new("one\ntwo"));
    let model_for_window = model.clone();
    let (first, cx) =
        cx.add_window_view(|window, cx| CodeEditor::new(model_for_window, window, cx));
    let second = cx.update(|window, cx| cx.new(|cx| CodeEditor::new(model.clone(), window, cx)));

    // Put the second editor's cursor at the start of "two".
    second.update(cx, |editor, cx| {
        editor.set_selections(&[Selection::collapsed(p(2, 1))], cx)
    });
    cx.update(|window, cx| window.focus(&first.focus_handle(cx), cx));
    cx.run_until_parked();
    cx.simulate_input(">> ");
    cx.run_until_parked();
    assert_eq!(model.read_with(cx, |m, _| m.text()), ">> one\ntwo");
    // Edits on another row don't move it.
    assert_eq!(
        second.read_with(cx, |e, _| e.selections()[0].head.to_monaco()),
        (2, 1)
    );
    cx.simulate_keystrokes("enter");
    assert_eq!(model.read_with(cx, |m, _| m.text()), ">> \none\ntwo");
    assert_eq!(
        second.read_with(cx, |e, _| e.selections()[0].head.to_monaco()),
        (3, 1)
    );
}

#[gpui::test]
fn copy_empty_selection_pastes_full_line_above(cx: &mut TestAppContext) {
    let (editor, cx) = editor_with_text("first\nsecond", cx);
    cx.simulate_keystrokes("right secondary-c down");
    let clipboard = cx.read_from_clipboard().and_then(|item| item.text());
    assert_eq!(clipboard.as_deref(), Some("first\n"));
    cx.simulate_keystrokes("secondary-v");
    assert_eq!(text(&editor, cx), "first\nfirst\nsecond");
}

#[gpui::test]
fn plain_clipboard_paste_replaces_selection(cx: &mut TestAppContext) {
    let (editor, cx) = editor_with_text("abc", cx);
    cx.write_to_clipboard(ClipboardItem::new_string("XY".into()));
    cx.simulate_keystrokes("shift-right secondary-v");
    assert_eq!(text(&editor, cx), "XYbc");
    assert_eq!(heads(&editor, cx), vec![(1, 3)]);
}

#[gpui::test]
fn ime_composition(cx: &mut TestAppContext) {
    let (editor, cx) = editor_with_text("a", cx);
    cx.simulate_keystrokes("secondary-right");
    cx.update(|window, cx| {
        editor.update(cx, |editor, cx| {
            editor.replace_and_mark_text_in_range(None, "に", None, window, cx);
            assert_eq!(editor.marked_text_range(window, cx), Some(1..2));
            editor.replace_and_mark_text_in_range(None, "にほ", None, window, cx);
            assert_eq!(editor.marked_text_range(window, cx), Some(1..3));
            editor.replace_text_in_range(None, "日本", window, cx);
            assert_eq!(editor.marked_text_range(window, cx), None);
        })
    });
    assert_eq!(text(&editor, cx), "a日本");
    assert_eq!(heads(&editor, cx), vec![(1, 4)]);
}

#[gpui::test]
fn read_only_ignores_input(cx: &mut TestAppContext) {
    let (editor, cx) = editor_with_text("keep", cx);
    editor.update(cx, |editor, cx| {
        let options = EditorOptions {
            read_only: true,
            ..editor.options().clone()
        };
        editor.update_options(options, cx);
    });
    cx.simulate_input("x");
    cx.simulate_keystrokes("backspace enter");
    assert_eq!(text(&editor, cx), "keep");
}

#[gpui::test]
fn view_state_round_trip(cx: &mut TestAppContext) {
    let (editor, cx) = editor_with_text("a\nb\nc", cx);
    cx.simulate_keystrokes("down shift-down");
    let state = editor.read_with(cx, |e, _| e.save_view_state());
    cx.simulate_keystrokes("secondary-up");
    assert_eq!(heads(&editor, cx), vec![(1, 1)]);
    editor.update(cx, |e, cx| e.restore_view_state(&state, cx));
    assert_eq!(heads(&editor, cx), vec![(3, 1)]);
}
