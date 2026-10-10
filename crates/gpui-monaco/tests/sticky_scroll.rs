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
