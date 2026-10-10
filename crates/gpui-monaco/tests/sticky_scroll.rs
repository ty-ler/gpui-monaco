//! Sticky scroll's model (Monaco `stickyScroll.defaultModel`): outline by
//! default, falling back to folding ranges.

use gpui::{AppContext as _, Entity, TestAppContext, VisualTestContext, px};
use gpui_monaco::options::{EditorOptions, StickyScrollModel, StickyScrollOptions};
use gpui_monaco::{CodeEditor, TextModel};

fn source() -> String {
    let mut text = String::from("impl Foo {\n    fn run(&self) {\n        if true {\n");
    for i in 0..200 {
        text.push_str(&format!("            let v{i} = {i};\n"));
    }
    text.push_str("        }\n    }\n}\n");
    text
}

fn setup<'a>(
    language: &str,
    model: StickyScrollModel,
    cx: &'a mut TestAppContext,
) -> (Entity<CodeEditor>, &'a mut VisualTestContext) {
    cx.update(|cx| {
        gpui_monaco::init(cx);
        gpui_monaco::set_language_registry(cx, gpui_monaco_languages::registry());
    });
    let text = source();
    let language = language.to_string();
    let (editor, cx) = cx.add_window_view(|window, cx| {
        let model_entity = cx.new(|_| TextModel::new(&text).with_language(&language));
        let mut editor = CodeEditor::new(model_entity, window, cx);
        editor.update_options(
            EditorOptions {
                sticky_scroll: StickyScrollOptions {
                    default_model: model,
                    ..Default::default()
                },
                ..EditorOptions::default()
            },
            cx,
        );
        editor
    });
    cx.run_until_parked();
    (editor, cx)
}

fn sticky_rows_at(editor: &Entity<CodeEditor>, y: f32, cx: &mut VisualTestContext) -> Vec<u32> {
    editor.update(cx, |e, cx| {
        e.set_scroll_position(gpui::point(px(0.), px(y)), cx)
    });
    for _ in 0..3 {
        cx.update(|window, _| window.refresh());
        cx.run_until_parked();
        cx.executor()
            .advance_clock(std::time::Duration::from_millis(500));
        cx.run_until_parked();
    }
    editor.read_with(cx, |e, _| e.sticky_rows_for_tests().0)
}

#[gpui::test]
fn outline_model_sticks_symbols_only(cx: &mut TestAppContext) {
    let (editor, cx) = setup("rust", StickyScrollModel::Outline, cx);
    // The `if` block isn't a symbol.
    assert_eq!(sticky_rows_at(&editor, 1000., cx), vec![0, 1]);
}

#[gpui::test]
fn folding_model_sticks_every_block(cx: &mut TestAppContext) {
    let (editor, cx) = setup("rust", StickyScrollModel::FoldingProvider, cx);
    assert_eq!(sticky_rows_at(&editor, 1000., cx), vec![0, 1, 2]);
}

#[gpui::test]
fn indentation_model(cx: &mut TestAppContext) {
    let (editor, cx) = setup("rust", StickyScrollModel::Indentation, cx);
    assert_eq!(sticky_rows_at(&editor, 1000., cx), vec![0, 1, 2]);
}

#[gpui::test]
fn outline_without_symbols_falls_back_to_folding(cx: &mut TestAppContext) {
    // Plain text has no symbols (and indentation folds).
    let (editor, cx) = setup("plaintext", StickyScrollModel::Outline, cx);
    assert_eq!(sticky_rows_at(&editor, 1000., cx), vec![0, 1, 2]);
}

fn cursor_row(editor: &Entity<CodeEditor>, cx: &mut VisualTestContext) -> u32 {
    editor.read_with(cx, |e, _| e.selections()[0].head.row)
}

#[gpui::test]
fn keyboard_focus_moves_and_goes_to_lines(cx: &mut TestAppContext) {
    use gpui::Focusable;
    use gpui_monaco::actions::*;
    let (editor, cx) = setup("rust", StickyScrollModel::Outline, cx);
    cx.update(|window, cx| window.focus(&editor.focus_handle(cx), cx));
    assert_eq!(sticky_rows_at(&editor, 1000., cx), vec![0, 1]);
    // Focus starts on the last sticky line.
    cx.dispatch_action(FocusStickyScroll);
    assert!(editor.read_with(cx, |e, _| e.is_sticky_scroll_focused()));
    // Up, then Enter: goes to the first.
    cx.simulate_keystrokes("up enter");
    cx.run_until_parked();
    assert!(!editor.read_with(cx, |e, _| e.is_sticky_scroll_focused()));
    assert_eq!(cursor_row(&editor, cx), 0);

    // Escape returns to the text without moving.
    sticky_rows_at(&editor, 1000., cx);
    cx.dispatch_action(FocusStickyScroll);
    cx.simulate_keystrokes("escape");
    assert!(!editor.read_with(cx, |e, _| e.is_sticky_scroll_focused()));
    assert_eq!(cursor_row(&editor, cx), 0);
}

#[gpui::test]
fn toggle_turns_sticky_scroll_off_and_on(cx: &mut TestAppContext) {
    use gpui_monaco::actions::*;
    let (editor, cx) = setup("rust", StickyScrollModel::Outline, cx);
    assert_eq!(sticky_rows_at(&editor, 1000., cx), vec![0, 1]);
    editor.update(cx, |e, cx| e.toggle_sticky_scroll(cx));
    assert!(sticky_rows_at(&editor, 1000., cx).is_empty());
    cx.update(|window, cx| window.focus(&gpui::Focusable::focus_handle(&editor, cx), cx));
    cx.dispatch_action(ToggleStickyScroll);
    assert_eq!(sticky_rows_at(&editor, 1000., cx), vec![0, 1]);
}
