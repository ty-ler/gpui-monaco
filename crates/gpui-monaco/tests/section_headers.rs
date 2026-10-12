//! Minimap section headers (Monaco `sectionHeaders.ts`): `#region` labels
//! and `MARK:` comments.

use std::time::Duration;

use gpui::{AppContext, Entity, TestAppContext, VisualTestContext};
use gpui_monaco::{CodeEditor, TextModel};

fn setup<'a>(
    text: &str,
    cx: &'a mut TestAppContext,
) -> (Entity<CodeEditor>, &'a mut VisualTestContext) {
    cx.update(|cx| {
        gpui_monaco::init(cx);
        gpui_monaco::set_language_registry(cx, gpui_monaco_languages::registry());
    });
    let text = text.to_string();
    let (editor, cx) = cx.add_window_view(|window, cx| {
        let model = cx.new(|_| TextModel::new(&text).with_language("rust"));
        CodeEditor::new(model, window, cx)
    });
    paint(cx);
    (editor, cx)
}

fn paint(cx: &mut VisualTestContext) {
    for _ in 0..3 {
        cx.update(|window, _| window.refresh());
        cx.run_until_parked();
        cx.executor().advance_clock(Duration::from_millis(300));
    }
    cx.run_until_parked();
}

fn headers(editor: &Entity<CodeEditor>, cx: &mut VisualTestContext) -> Vec<(u32, String, bool)> {
    paint(cx);
    editor.read_with(cx, |e, cx| e.section_headers_for_tests(cx))
}

#[gpui::test]
fn regions_and_marks_in_comments(cx: &mut TestAppContext) {
    let text = "// #region Setup\nfn a() {}\n// #endregion\n// MARK: - Helpers\nfn b() {}\nlet s = \"MARK: not a comment\";\n";
    let (editor, cx) = setup(text, cx);
    assert_eq!(
        headers(&editor, cx),
        vec![
            (0, "Setup".to_string(), false),
            (3, "Helpers".to_string(), true),
        ]
    );

    // Edits find them again after the delay.
    editor.update(cx, |e, cx| {
        e.model().update(cx, |m, cx| {
            m.edit([(0..0, "// MARK: Top\n")], cx).unwrap();
        })
    });
    // Monaco lists region headers before mark headers.
    assert_eq!(
        headers(&editor, cx),
        vec![
            (1, "Setup".to_string(), false),
            (0, "Top".to_string(), false),
            (4, "Helpers".to_string(), true),
        ]
    );

    // Turning both kinds off clears them.
    editor.update(cx, |e, cx| {
        let mut o = e.options().clone();
        o.minimap.show_region_section_headers = false;
        o.minimap.show_mark_section_headers = false;
        e.update_options(o, cx);
    });
    assert!(headers(&editor, cx).is_empty());
}
