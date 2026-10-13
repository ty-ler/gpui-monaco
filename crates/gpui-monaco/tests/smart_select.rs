//! Expand and shrink selection (Monaco `contrib/smartSelect`).

use std::ops::Range;

use gpui::{App, AppContext as _, Entity, Focusable, Task, TestAppContext, VisualTestContext};
use gpui_monaco::cursor::Selection;
use gpui_monaco::providers::{
    LanguageSelector, ProviderContext, SelectionRangeProvider, register_selection_range_provider,
};
use gpui_monaco::text::PointUtf16;
use gpui_monaco::{CodeEditor, TextModel};

fn p(row: u32, column: u32) -> PointUtf16 {
    PointUtf16::new(row, column)
}

fn setup<'a>(
    text: &str,
    language: Option<&str>,
    cx: &'a mut TestAppContext,
) -> (Entity<CodeEditor>, &'a mut VisualTestContext) {
    cx.update(|cx| {
        gpui_monaco::init(cx);
        gpui_monaco::set_language_registry(cx, gpui_monaco_languages::registry());
    });
    let text = text.to_string();
    let language = language.map(str::to_string);
    let (editor, cx) = cx.add_window_view(|window, cx| {
        let model = cx.new(|_| {
            let model = TextModel::new(&text);
            match language {
                Some(language) => model.with_language(language),
                None => model,
            }
        });
        CodeEditor::new(model, window, cx)
    });
    cx.update(|window, cx| window.focus(&editor.focus_handle(cx), cx));
    cx.run_until_parked();
    cx.update(|window, cx| {
        window.refresh();
        window.draw(cx).clear(cx);
    });
    cx.run_until_parked();
    (editor, cx)
}

fn selected(editor: &Entity<CodeEditor>, cx: &mut VisualTestContext) -> String {
    editor.read_with(cx, |e, cx| {
        let s = e.selections()[0];
        e.model()
            .read(cx)
            .value_in_range(s.start()..s.end(), Default::default())
    })
}

fn step(editor: &Entity<CodeEditor>, cx: &mut VisualTestContext, expand: bool) -> String {
    editor.update(cx, |e, cx| {
        if expand {
            e.expand_selection(cx)
        } else {
            e.shrink_selection(cx)
        }
    });
    cx.run_until_parked();
    selected(editor, cx)
}

#[gpui::test]
fn grows_through_syntax_nodes(cx: &mut TestAppContext) {
    let (editor, cx) = setup(
        "fn main() {\n    let total = add(fooBar, 2);\n}\n",
        Some("rust"),
        cx,
    );
    // Inside `fooBar`, on `Bar`.
    editor.update(cx, |e, cx| {
        e.set_selections(&[Selection::collapsed(p(1, 24))], cx)
    });
    let mut grown = Vec::new();
    for _ in 0..5 {
        grown.push(step(&editor, cx, true));
    }
    assert_eq!(
        grown,
        [
            "Bar",
            "fooBar",
            "(fooBar, 2)",
            "add(fooBar, 2)",
            "let total = add(fooBar, 2);",
        ]
    );
    // Shrinking walks back.
    assert_eq!(step(&editor, cx, false), "add(fooBar, 2)");
    assert_eq!(step(&editor, cx, false), "(fooBar, 2)");
    // Moving the cursor starts over.
    editor.update(cx, |e, cx| {
        e.set_selections(&[Selection::collapsed(p(1, 9))], cx)
    });
    assert_eq!(step(&editor, cx, true), "total");
}

#[gpui::test]
fn registered_provider_replaces_brackets(cx: &mut TestAppContext) {
    // Monaco adds brackets only when no provider is registered: one that
    // answers nothing still leaves just the words (and lines). The same
    // goes for the syntax tree's ranges.
    struct Silent;
    impl SelectionRangeProvider for Silent {
        fn provide_selection_ranges(
            &self,
            _: &ProviderContext,
            _: &[PointUtf16],
            _: &mut App,
        ) -> Task<anyhow::Result<Vec<Vec<Range<PointUtf16>>>>> {
            Task::ready(Ok(Vec::new()))
        }
    }
    let _provider =
        cx.update(|cx| register_selection_range_provider(cx, LanguageSelector::All, Silent));
    let (editor, cx) = setup("a (b c) d\nnext", None, cx);
    editor.update(cx, |e, cx| {
        e.set_selections(&[Selection::collapsed(p(0, 3))], cx)
    });
    let grown: Vec<String> = (0..3).map(|_| step(&editor, cx, true)).collect();
    assert_eq!(grown, ["b", "a (b c) d", "a (b c) d\nnext"]);
}

#[gpui::test]
fn plain_text_uses_brackets_and_lines(cx: &mut TestAppContext) {
    let (editor, cx) = setup("a (b c) d\nnext", None, cx);
    editor.update(cx, |e, cx| {
        e.set_selections(&[Selection::collapsed(p(0, 3))], cx)
    });
    let grown: Vec<String> = (0..5).map(|_| step(&editor, cx, true)).collect();
    // The line comes before the whole text (Monaco's
    // `selectLeadingAndTrailingWhitespace`).
    assert_eq!(grown, ["b", "b c", "(b c)", "a (b c) d", "a (b c) d\nnext"]);
}
