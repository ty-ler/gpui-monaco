//! Occurrence highlighting (Monaco's word highlighter and selection
//! highlighter).

use std::cell::RefCell;
use std::rc::Rc;
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

/// Highlights on the cursor's row: (start column, end column, kind).
type Ranges = Vec<(u32, u32, DocumentHighlightKind)>;

/// Answers with fixed ranges on the cursor's row (`None` declines), and
/// records that it was asked.
struct Fixed {
    name: &'static str,
    answer: Option<Ranges>,
    calls: Rc<RefCell<Vec<&'static str>>>,
}

impl DocumentHighlightProvider for Fixed {
    fn provide_document_highlights(
        &self,
        _: &ProviderContext,
        position: PointUtf16,
        _: &mut App,
    ) -> Task<anyhow::Result<Option<Vec<DocumentHighlight>>>> {
        self.calls.borrow_mut().push(self.name);
        Task::ready(Ok(self.answer.as_ref().map(|ranges| {
            ranges
                .iter()
                .map(|&(start, end, kind)| DocumentHighlight {
                    range: PointUtf16::new(position.row, start)..PointUtf16::new(position.row, end),
                    kind,
                })
                .collect()
        })))
    }
}

/// Registers `Fixed` providers in order, returning what they were asked.
fn register(
    cx: &mut VisualTestContext,
    answers: Vec<(&'static str, Option<Ranges>)>,
) -> Rc<RefCell<Vec<&'static str>>> {
    let calls = Rc::new(RefCell::new(Vec::new()));
    for (name, answer) in answers {
        let provider = Fixed {
            name,
            answer,
            calls: calls.clone(),
        };
        cx.update(|_, cx| {
            register_document_highlight_provider(cx, LanguageSelector::All, provider)
        });
    }
    calls
}

fn from_all_providers(editor: &Entity<CodeEditor>, on: bool, cx: &mut VisualTestContext) {
    editor.update(cx, |e, cx| {
        let mut options = e.options().clone();
        options.occurrences_highlight_from_all_providers = on;
        e.update_options(options, cx);
    });
}

/// Every word highlight, strong (write) ones included.
fn all_words(editor: &Entity<CodeEditor>, cx: &mut VisualTestContext) -> Vec<(u32, u32, u32, u32)> {
    let mut all = words(editor, cx);
    all.extend(highlighted(editor, cx, |c| {
        c.word_highlight_strong_background
    }));
    all.sort();
    all
}

#[gpui::test]
fn occurrences_from_all_providers(cx: &mut TestAppContext) {
    use DocumentHighlightKind::*;
    // Monaco `wordHighlighter.test.ts` (`DocumentHighlightProviders`).
    let (editor, cx) = setup("word word word", cx);
    let calls = register(
        cx,
        vec![
            ("first", Some(vec![(0, 4, Read)])),
            ("second", Some(vec![(0, 4, Write), (5, 9, Write)])),
        ],
    );
    // "keeps the first answer when merging is disabled": the next provider
    // isn't even asked.
    set(&editor, cx, p(1, 2), p(1, 2));
    wait(cx);
    assert_eq!(all_words(&editor, cx), vec![(1, 1, 1, 5)]);
    assert_eq!(*calls.borrow(), vec!["first"]);

    // Turning merging on asks again, and every provider at once; the first
    // answer for a range wins.
    calls.borrow_mut().clear();
    from_all_providers(&editor, true, cx);
    wait(cx);
    assert_eq!(*calls.borrow(), vec!["first", "second"]);
    assert_eq!(words(&editor, cx), vec![(1, 1, 1, 5)]);
    assert_eq!(
        highlighted(&editor, cx, |c| c.word_highlight_strong_background),
        vec![(1, 6, 1, 10)]
    );
}

#[gpui::test]
fn empty_and_declined_answers(cx: &mut TestAppContext) {
    use DocumentHighlightKind::*;
    let (editor, cx) = setup("word word word", cx);
    let calls = register(
        cx,
        vec![
            ("empty", Some(Vec::new())),
            ("other", Some(vec![(5, 9, Read)])),
        ],
    );
    // "an empty answer still stops the default provider search".
    set(&editor, cx, p(1, 2), p(1, 2));
    wait(cx);
    assert!(all_words(&editor, cx).is_empty());
    assert_eq!(*calls.borrow(), vec!["empty"]);
    // "merges other providers when a higher-priority provider returns an
    // empty array".
    from_all_providers(&editor, true, cx);
    wait(cx);
    assert_eq!(all_words(&editor, cx), vec![(1, 6, 1, 10)]);
}

#[gpui::test]
fn no_textual_fallback_when_merging(cx: &mut TestAppContext) {
    let (editor, cx) = setup("word word word", cx);
    register(cx, vec![("declines", None)]);
    // "preserves textual fallback after a declined answer when merging is
    // disabled".
    set(&editor, cx, p(1, 2), p(1, 2));
    wait(cx);
    assert_eq!(
        all_words(&editor, cx),
        vec![(1, 1, 1, 5), (1, 6, 1, 10), (1, 11, 1, 15)]
    );
    // "does not use textual fallback when a matching provider returns
    // null".
    from_all_providers(&editor, true, cx);
    wait(cx);
    assert!(all_words(&editor, cx).is_empty());
}

#[gpui::test]
fn textual_occurrences_when_merging_without_providers(cx: &mut TestAppContext) {
    // "uses textual fallback only when no other provider matches".
    let (editor, cx) = setup("word word\nword", cx);
    from_all_providers(&editor, true, cx);
    set(&editor, cx, p(1, 2), p(1, 2));
    wait(cx);
    assert_eq!(
        all_words(&editor, cx),
        vec![(1, 1, 1, 5), (1, 6, 1, 10), (2, 1, 2, 5)]
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
