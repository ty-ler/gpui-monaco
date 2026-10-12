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
use gpui_monaco::{CodeEditor, FindOptions, TextModel};

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

    // F7 / Shift+F7 move the caret to the start of the next / previous one
    // (Monaco `WordHighlighter.moveNext` / `moveBack`).
    let caret = |editor: &Entity<CodeEditor>, cx: &mut VisualTestContext| {
        let s = editor.read_with(cx, |e, _| e.selections()[0]);
        assert!(s.is_empty(), "{s:?}");
        s.head
    };
    cx.simulate_keystrokes("f7");
    assert_eq!(caret(&editor, cx), p(1, 9));
    // The screen-reader alert: "line, i of n for 'word'".
    let announcement = editor.read_with(cx, |e, _| e.last_announcement_for_tests());
    assert_eq!(
        announcement.as_deref(),
        Some("foo bar foo, 2 of 3 for 'foo'")
    );
    cx.simulate_keystrokes("f7 f7");
    assert_eq!(caret(&editor, cx), p(1, 1));
    cx.simulate_keystrokes("shift-f7");
    assert_eq!(caret(&editor, cx), p(2, 8));
    // The highlights stay while the caret is on one of them.
    wait(cx);
    assert_eq!(
        words(&editor, cx),
        vec![(1, 1, 1, 4), (1, 9, 1, 12), (2, 8, 2, 11)]
    );

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

/// Monaco `TextualDocumentHighlightProvider`: `findMatches` with the default
/// `LIMIT_FIND_COUNT`, so at most 999 occurrences highlight and F7 cycles
/// through those.
#[gpui::test]
fn textual_word_highlights_stop_at_999(cx: &mut TestAppContext) {
    let (editor, cx) = setup(&"x ".repeat(5000), cx);
    set(&editor, cx, p(1, 1), p(1, 1));
    wait(cx);
    let words = words(&editor, cx);
    assert_eq!(words.len(), 999);
    assert_eq!(words.last(), Some(&(1, 1997, 1, 1998)));
    // Shift+F7 from the first wraps to the 999th.
    cx.simulate_keystrokes("shift-f7");
    let head = editor.read_with(cx, |e, _| e.selections()[0].head);
    assert_eq!(head, p(1, 1997));
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

/// Highlights every `aa` on the cursor's row, wherever the cursor is.
struct EveryAa;
impl DocumentHighlightProvider for EveryAa {
    fn provide_document_highlights(
        &self,
        _: &ProviderContext,
        position: PointUtf16,
        _: &mut App,
    ) -> Task<anyhow::Result<Option<Vec<DocumentHighlight>>>> {
        Task::ready(Ok(Some(
            [0, 6, 12]
                .into_iter()
                .map(|column| DocumentHighlight {
                    range: PointUtf16::new(position.row, column)
                        ..PointUtf16::new(position.row, column + 2),
                    kind: DocumentHighlightKind::Text,
                })
                .collect(),
        )))
    }
}

/// Monaco `moveNext` / `moveBack` from outside every highlight (index -1):
/// F7 goes to the first, Shift+F7 to `(-1 - 1 + n) % n`.
#[gpui::test]
fn word_highlight_navigation_from_outside_the_highlights(cx: &mut TestAppContext) {
    let (editor, cx) = setup("aa bb aa bb aa", cx);
    cx.update(|_, cx| register_document_highlight_provider(cx, LanguageSelector::All, EveryAa));
    set(&editor, cx, p(1, 5), p(1, 5));
    wait(cx);
    assert_eq!(
        words(&editor, cx),
        vec![(1, 1, 1, 3), (1, 7, 1, 9), (1, 13, 1, 15)]
    );
    cx.simulate_keystrokes("f7");
    let head = editor.read_with(cx, |e, _| e.selections()[0].head);
    assert_eq!(head, p(1, 1));

    set(&editor, cx, p(1, 5), p(1, 5));
    wait(cx);
    cx.simulate_keystrokes("shift-f7");
    let head = editor.read_with(cx, |e, _| e.selections()[0].head);
    assert_eq!(head, p(1, 7));
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
    // Registered last to first: equal scores are tried newest first (Monaco
    // `LanguageFeatureRegistry.ordered`), so `answers` is in priority order.
    for (name, answer) in answers.into_iter().rev() {
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

/// Monaco `SelectionHighlighter._createState` reuses the Cmd/Ctrl+D
/// session: after Cmd+D on an empty selection it highlights only
/// whole-word, case-sensitive occurrences.
#[gpui::test]
fn selection_highlights_follow_a_cmd_d_session(cx: &mut TestAppContext) {
    let (editor, cx) = setup("abc xabc abc Abc", cx);
    set(&editor, cx, p(1, 2), p(1, 2));
    cx.simulate_keystrokes("secondary-d");
    cx.run_until_parked();
    assert_eq!(selected_text_matches(&editor, cx), vec![(1, 10, 1, 13)]);
    // Selecting the same text by hand matches substrings again, with the
    // find state's match case (off).
    set(&editor, cx, p(1, 4), p(1, 1));
    cx.run_until_parked();
    assert_eq!(
        selected_text_matches(&editor, cx),
        vec![(1, 6, 1, 9), (1, 10, 1, 13), (1, 14, 1, 17)]
    );
}

/// Monaco's "Selected text occurrence matching highlights" text.
const HIGHLIGHTED: &str = "foo\nFOO\nfooBar\nFOOBar";

fn match_mode(
    editor: &Entity<CodeEditor>,
    cx: &mut VisualTestContext,
    mode: gpui_monaco::options::SelectedTextMatchMode,
) {
    editor.update(cx, |e, cx| {
        let mut options = e.options().clone();
        options.selected_text_match_mode = mode;
        e.update_options(options, cx);
    });
    cx.run_until_parked();
}

/// Opens find with `options`, the focus back in the text.
fn find_open(editor: &Entity<CodeEditor>, cx: &mut VisualTestContext, options: FindOptions) {
    cx.simulate_keystrokes("secondary-f");
    editor.update(cx, |e, cx| e.change_find_options(options, false, cx));
    cx.update(|window, cx| window.focus(&editor.focus_handle(cx), cx));
    cx.run_until_parked();
}

/// The lines whose `foo` (columns 1-4) are highlighted.
fn highlighted_lines(editor: &Entity<CodeEditor>, cx: &mut VisualTestContext) -> Vec<u32> {
    selected_text_matches(editor, cx)
        .into_iter()
        .map(|(line, start, end_line, end)| {
            assert_eq!((start, end_line, end), (1, line, 4));
            line
        })
        .collect()
}

/// Monaco: "selection highlighting uses match case false/true, editor
/// focused true": with `caseSensitive` or `caseInsensitive`, the find
/// widget's toggles don't matter.
#[gpui::test]
fn selection_highlights_match_by_the_mode(cx: &mut TestAppContext) {
    use gpui_monaco::options::SelectedTextMatchMode::{CaseInsensitive, CaseSensitive};
    for (mode, expected) in [(CaseInsensitive, vec![2, 3, 4]), (CaseSensitive, vec![3])] {
        let (editor, vcx) = setup(HIGHLIGHTED, cx);
        match_mode(&editor, vcx, mode);
        find_open(
            &editor,
            vcx,
            FindOptions {
                query: "f.*".into(),
                match_case: mode != CaseSensitive,
                whole_word: true,
                is_regex: true,
                ..Default::default()
            },
        );
        set(&editor, vcx, p(1, 1), p(1, 4));
        vcx.run_until_parked();
        assert_eq!(highlighted_lines(&editor, vcx), expected, "{mode:?}");
    }
}

/// Monaco: "only suppress duplicate Find highlights with the same matching
/// rules": the find widget showing the same search hides the highlights,
/// until its match case differs.
#[gpui::test]
fn selection_highlights_hide_only_the_find_widgets_own_matches(cx: &mut TestAppContext) {
    use gpui_monaco::options::SelectedTextMatchMode::{CaseInsensitive, CaseSensitive};
    for (mode, expected) in [(CaseInsensitive, vec![2, 3, 4]), (CaseSensitive, vec![3])] {
        let (editor, vcx) = setup(HIGHLIGHTED, cx);
        match_mode(&editor, vcx, mode);
        set(&editor, vcx, p(1, 1), p(1, 4));
        let match_case = mode == CaseSensitive;
        find_open(
            &editor,
            vcx,
            FindOptions {
                query: "foo".into(),
                match_case,
                ..Default::default()
            },
        );
        let same_options = highlighted_lines(&editor, vcx);
        editor.update(vcx, |e, cx| {
            let mut options = e.find_options().clone();
            options.match_case = !match_case;
            e.change_find_options(options, false, cx);
        });
        vcx.run_until_parked();
        assert_eq!(
            (same_options, highlighted_lines(&editor, vcx)),
            (vec![], expected),
            "{mode:?}"
        );
    }
}

/// Monaco: "selection highlighting refreshes on case-only configuration
/// changes without cursor movement". By default the find state's match
/// case (off, with find closed) applies.
#[gpui::test]
fn selection_highlights_refresh_when_the_match_mode_changes(cx: &mut TestAppContext) {
    use gpui_monaco::options::SelectedTextMatchMode::{CaseInsensitive, CaseSensitive};
    let (editor, cx) = setup(HIGHLIGHTED, cx);
    set(&editor, cx, p(1, 1), p(1, 4));
    cx.run_until_parked();
    let mut actual = vec![highlighted_lines(&editor, cx)];
    match_mode(&editor, cx, CaseSensitive);
    actual.push(highlighted_lines(&editor, cx));
    match_mode(&editor, cx, CaseInsensitive);
    actual.push(highlighted_lines(&editor, cx));
    assert_eq!(actual, vec![vec![2, 3, 4], vec![3], vec![2, 3, 4]]);
}

/// With the default `findOptions`, the highlights follow the find state's
/// match case even with find closed, and refresh when it changes.
#[gpui::test]
fn selection_highlights_follow_the_closed_find_states_match_case(cx: &mut TestAppContext) {
    let (editor, cx) = setup(HIGHLIGHTED, cx);
    set(&editor, cx, p(1, 1), p(1, 4));
    cx.run_until_parked();
    assert_eq!(highlighted_lines(&editor, cx), vec![2, 3, 4]);
    editor.update(cx, |e, cx| {
        let mut options = e.find_options().clone();
        options.match_case = true;
        e.change_find_options(options, false, cx);
    });
    cx.run_until_parked();
    assert_eq!(highlighted_lines(&editor, cx), vec![3]);
}

fn occurrences_highlight(
    editor: &Entity<CodeEditor>,
    value: gpui_monaco::options::OccurrencesHighlight,
    cx: &mut VisualTestContext,
) {
    editor.update(cx, |e, cx| {
        e.update_options_with(|o| o.occurrences_highlight = value, cx)
    });
}

/// Monaco `WordHighlighter` with `occurrencesHighlight: 'multiFile'`: with
/// other models open, standalone Monaco's textual multi-document provider
/// highlights the word in every model (the document highlight providers
/// aren't asked), and each editor shows its model's highlights.
#[gpui::test]
fn multi_file_highlights_in_the_other_editors(cx: &mut TestAppContext) {
    use gpui_monaco::options::OccurrencesHighlight;
    let (editor, cx) = setup("foo bar foo", cx);
    let (other, same) = cx.update(|window, cx| {
        let model = cx.new(|_| TextModel::new("bar foo\nfoo").with_uri("file:///b.txt"));
        let other = cx.new(|cx| CodeEditor::new(model, window, cx));
        let shared = editor.read(cx).model().clone();
        let same = cx.new(|cx| CodeEditor::new(shared, window, cx));
        (other, same)
    });
    cx.update(|_, cx| register_document_highlight_provider(cx, LanguageSelector::All, Writes));
    occurrences_highlight(&editor, OccurrencesHighlight::MultiFile, cx);
    set(&editor, cx, p(1, 2), p(1, 2));
    wait(cx);
    assert_eq!(words(&editor, cx), vec![(1, 1, 1, 4), (1, 9, 1, 12)]);
    assert_eq!(words(&other, cx), vec![(1, 5, 1, 8), (2, 1, 2, 4)]);
    // An editor showing the same model shows them too.
    assert_eq!(words(&same, cx), vec![(1, 1, 1, 4), (1, 9, 1, 12)]);

    set(&editor, cx, p(1, 6), p(1, 6));
    wait(cx);
    assert_eq!(words(&editor, cx), vec![(1, 5, 1, 8)]);
    assert_eq!(words(&other, cx), vec![(1, 1, 1, 4)]);

    // Typing stops them everywhere (Monaco `_stopAll`).
    cx.simulate_input("x");
    wait(cx);
    assert!(words(&editor, cx).is_empty());
    assert!(words(&other, cx).is_empty());
    assert!(words(&same, cx).is_empty());

    // `SingleFile`: only this editor, from the provider.
    occurrences_highlight(&editor, OccurrencesHighlight::SingleFile, cx);
    set(&editor, cx, p(1, 2), p(1, 2));
    wait(cx);
    assert_eq!(words(&editor, cx), vec![(1, 3, 1, 4)]);
    assert!(words(&other, cx).is_empty());

    // Switching to `MultiFile` asks again, and leaving it stops the other
    // editors' highlights.
    occurrences_highlight(&editor, OccurrencesHighlight::MultiFile, cx);
    wait(cx);
    assert_eq!(words(&other, cx), vec![(1, 5, 1, 8), (2, 1, 2, 4)]);
    occurrences_highlight(&editor, OccurrencesHighlight::SingleFile, cx);
    wait(cx);
    assert!(words(&other, cx).is_empty());
}
