//! Occurrence highlighting (Monaco's word highlighter and selection
//! highlighter).

use std::time::Duration;

use gpui::{
    App, AppContext as _, Entity, Focusable, Hsla, Task, TestAppContext, VisualTestContext,
};
use gpui_monaco::cursor::Selection;
use gpui_monaco::providers::*;
use gpui_monaco::text::PointUtf16;
use gpui_monaco::{CodeEditor, TextModel};

fn setup<'a>(
    text: &str,
    cx: &'a mut TestAppContext,
) -> (Entity<CodeEditor>, &'a mut VisualTestContext) {
    cx.update(gpui_monaco::init);
    let text = text.to_string();
    let (editor, cx) = cx.add_window_view(|window, cx| {
        let model = cx.new(|_| TextModel::new(&text).with_uri("file:///a.txt"));
        CodeEditor::new(model, window, cx)
    });
    cx.update(|window, cx| window.focus(&editor.focus_handle(cx), cx));
    cx.run_until_parked();
    (editor, cx)
}

fn p(line: u32, column: u32) -> PointUtf16 {
    PointUtf16::from_monaco(line, column)
}

fn wait(cx: &mut VisualTestContext) {
    cx.executor().advance_clock(Duration::from_millis(400));
    cx.run_until_parked();
}

/// Ranges (Monaco 1-based) of this editor's decorations with `background`.
fn highlighted(
    editor: &Entity<CodeEditor>,
    cx: &mut VisualTestContext,
    background: impl Fn(&gpui_monaco::EditorColors) -> Hsla,
) -> Vec<(u32, u32, u32, u32)> {
    let id = editor.entity_id();
    let color = editor.read_with(cx, |e, _| background(e.colors()));
    let model = editor.read_with(cx, |e, _| e.model().clone());
    model.update(cx, |model, _| {
        let buffer = model.buffer().clone();
        let mut ranges: Vec<_> = model
            .all_decorations(Some(id))
            .into_iter()
            .filter(|d| d.owner == Some(id) && d.options.background == Some(color))
            .map(|d| {
                let a = buffer.offset_to_point_utf16(d.range.start).to_monaco();
                let b = buffer.offset_to_point_utf16(d.range.end).to_monaco();
                (a.0, a.1, b.0, b.1)
            })
            .collect();
        ranges.sort();
        ranges
    })
}

fn words(editor: &Entity<CodeEditor>, cx: &mut VisualTestContext) -> Vec<(u32, u32, u32, u32)> {
    highlighted(editor, cx, |c| c.word_highlight_background)
}

fn selected_text_matches(
    editor: &Entity<CodeEditor>,
    cx: &mut VisualTestContext,
) -> Vec<(u32, u32, u32, u32)> {
    highlighted(editor, cx, |c| c.selection_highlight_background)
}

fn set(
    editor: &Entity<CodeEditor>,
    cx: &mut VisualTestContext,
    anchor: PointUtf16,
    head: PointUtf16,
) {
    editor.update(cx, |e, cx| {
        e.set_selections(&[Selection { anchor, head }], cx)
    });
}

#[gpui::test]
fn textual_word_highlights(cx: &mut TestAppContext) {
    let (editor, cx) = setup("foo bar foo\nfoobar foo", cx);
    set(&editor, cx, p(1, 2), p(1, 2));
    // Nothing until the cursor rests.
    assert!(words(&editor, cx).is_empty());
    wait(cx);
    // Whole-word, case-sensitive matches; `foobar` doesn't count.
    assert_eq!(
        words(&editor, cx),
        vec![(1, 1, 1, 4), (1, 9, 1, 12), (2, 8, 2, 11)]
    );

    // F7 / Shift+F7 move through them.
    cx.simulate_keystrokes("f7");
    let s = editor.read_with(cx, |e, _| e.selections()[0]);
    assert_eq!((s.anchor, s.head), (p(1, 9), p(1, 12)));
    cx.simulate_keystrokes("f7 f7");
    let s = editor.read_with(cx, |e, _| e.selections()[0]);
    assert_eq!((s.anchor, s.head), (p(1, 1), p(1, 4)));
    cx.simulate_keystrokes("shift-f7");
    let s = editor.read_with(cx, |e, _| e.selections()[0]);
    assert_eq!((s.anchor, s.head), (p(2, 8), p(2, 11)));

    // Typing clears them until the cursor moves again.
    set(&editor, cx, p(1, 6), p(1, 6));
    wait(cx);
    assert_eq!(words(&editor, cx), vec![(1, 5, 1, 8)]);
    cx.simulate_input("x");
    wait(cx);
    assert!(words(&editor, cx).is_empty());

    // Off: nothing.
    editor.update(cx, |e, cx| {
        let mut options = e.options().clone();
        options.occurrences_highlight = gpui_monaco::options::OccurrencesHighlight::Off;
        e.update_options(options, cx);
    });
    set(&editor, cx, p(1, 2), p(1, 2));
    wait(cx);
    assert!(words(&editor, cx).is_empty());
}

struct Writes;
impl DocumentHighlightProvider for Writes {
    fn provide_document_highlights(
        &self,
        _: &ProviderContext,
        position: PointUtf16,
        _: &mut App,
    ) -> Task<anyhow::Result<Option<Vec<DocumentHighlight>>>> {
        Task::ready(Ok(Some(vec![
            DocumentHighlight {
                range: PointUtf16::new(position.row, 0)..PointUtf16::new(position.row, 1),
                kind: DocumentHighlightKind::Write,
            },
            DocumentHighlight {
                range: PointUtf16::new(position.row, 2)..PointUtf16::new(position.row, 3),
                kind: DocumentHighlightKind::Read,
            },
        ])))
    }
}

#[gpui::test]
fn document_highlight_provider(cx: &mut TestAppContext) {
    let (editor, cx) = setup("a = a + 1", cx);
    cx.update(|_, cx| register_document_highlight_provider(cx, LanguageSelector::All, Writes));
    set(&editor, cx, p(1, 1), p(1, 1));
    wait(cx);
    assert_eq!(words(&editor, cx), vec![(1, 3, 1, 4)]);
    assert_eq!(
        highlighted(&editor, cx, |c| c.word_highlight_strong_background),
        vec![(1, 1, 1, 2)]
    );
}

#[gpui::test]
fn selection_highlights(cx: &mut TestAppContext) {
    let (editor, cx) = setup("abc xabc abc\n  \n  abc", cx);
    set(&editor, cx, p(1, 1), p(1, 4));
    cx.run_until_parked();
    // Not the selection itself; not whole-word.
    assert_eq!(
        selected_text_matches(&editor, cx),
        vec![(1, 6, 1, 9), (1, 10, 1, 13), (3, 3, 3, 6)]
    );
    // Whitespace-only and multi-line selections don't highlight.
    set(&editor, cx, p(2, 1), p(2, 3));
    cx.run_until_parked();
    assert!(selected_text_matches(&editor, cx).is_empty());
    set(&editor, cx, p(1, 1), p(2, 1));
    cx.run_until_parked();
    assert!(selected_text_matches(&editor, cx).is_empty());
    // Collapsing the selection clears them.
    set(&editor, cx, p(1, 1), p(1, 4));
    cx.run_until_parked();
    set(&editor, cx, p(1, 2), p(1, 2));
    cx.run_until_parked();
    assert!(selected_text_matches(&editor, cx).is_empty());
}
