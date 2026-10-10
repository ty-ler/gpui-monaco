//! Inline completions (Monaco `contrib/inlineCompletions`, ghost text) with
//! mock providers.

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

use gpui::{App, AppContext as _, Entity, Focusable, Task, TestAppContext, VisualTestContext};
use gpui_monaco::providers::*;
use gpui_monaco::text::PointUtf16;
use gpui_monaco::{CodeEditor, TextModel};

/// Completes the line with each of `completions` whose text starts with
/// what's before the cursor.
#[derive(Clone)]
struct Lines {
    completions: Vec<&'static str>,
    requests: Rc<RefCell<Vec<InlineCompletionTriggerKind>>>,
}

impl InlineCompletionsProvider for Lines {
    fn provide_inline_completions(
        &self,
        context: &ProviderContext,
        position: PointUtf16,
        completion_context: InlineCompletionContext,
        _: &mut App,
    ) -> Task<anyhow::Result<InlineCompletionList>> {
        self.requests
            .borrow_mut()
            .push(completion_context.trigger_kind);
        let line = context.snapshot.line(position.row);
        let typed: String = line.chars().take(position.column as usize).collect();
        let items = self
            .completions
            .iter()
            .filter(|c| c.starts_with(typed.trim_start()) && !typed.trim().is_empty())
            .map(|c| {
                let indent = typed.len() - typed.trim_start().len();
                InlineCompletion {
                    insert_text: format!("{}{c}", &typed[..indent]),
                    range: Some(
                        PointUtf16::new(position.row, 0)
                            ..PointUtf16::new(position.row, line.len() as u32),
                    ),
                    ..Default::default()
                }
            })
            .collect();
        Task::ready(Ok(InlineCompletionList {
            items,
            suppress_suggestions: None,
        }))
    }
}

fn setup<'a>(
    completions: Vec<&'static str>,
    cx: &'a mut TestAppContext,
) -> (
    Entity<CodeEditor>,
    &'a mut VisualTestContext,
    Rc<RefCell<Vec<InlineCompletionTriggerKind>>>,
) {
    let requests = Rc::new(RefCell::new(Vec::new()));
    let provider = Lines {
        completions,
        requests: requests.clone(),
    };
    cx.update(|cx| {
        gpui_monaco::init(cx);
        register_inline_completions_provider(cx, LanguageSelector::All, provider);
    });
    let (editor, cx) = cx.add_window_view(|window, cx| {
        let model = cx.new(|_| TextModel::new(""));
        let mut editor = CodeEditor::new(model, window, cx);
        let mut options = editor.options().clone();
        // Keep the suggest widget out of the way.
        options.quick_suggestions = gpui_monaco::options::QuickSuggestions::OFF;
        options.word_based_suggestions = gpui_monaco::options::WordBasedSuggestions::Off;
        editor.update_options(options, cx);
        editor
    });
    cx.update(|window, cx| window.focus(&editor.focus_handle(cx), cx));
    cx.run_until_parked();
    (editor, cx, requests)
}

fn settle(cx: &mut VisualTestContext) {
    cx.executor().advance_clock(Duration::from_millis(100));
    cx.run_until_parked();
}

fn ghost(editor: &Entity<CodeEditor>, cx: &mut VisualTestContext) -> Option<Vec<(u32, String)>> {
    editor.read_with(cx, |e, cx| {
        e.ghost_text_for_tests(cx).map(|(_, parts)| parts)
    })
}

fn text(editor: &Entity<CodeEditor>, cx: &mut VisualTestContext) -> String {
    editor.read_with(cx, |e, cx| e.model().read(cx).text())
}

fn cursor(editor: &Entity<CodeEditor>, cx: &mut VisualTestContext) -> PointUtf16 {
    editor.read_with(cx, |e, _| e.selections()[0].head)
}

#[gpui::test]
fn typing_shows_ghost_text_and_tab_accepts(cx: &mut TestAppContext) {
    let (editor, cx, requests) = setup(vec!["println!(\"hi\");"], cx);
    cx.simulate_input("pri");
    // Debounced.
    assert_eq!(ghost(&editor, cx), None);
    settle(cx);
    assert_eq!(ghost(&editor, cx), Some(vec![(3, "ntln!(\"hi\");".into())]));
    assert_eq!(requests.borrow()[0], InlineCompletionTriggerKind::Automatic);
    // Typing what it suggests keeps it right away.
    cx.simulate_input("n");
    assert_eq!(ghost(&editor, cx), Some(vec![(4, "tln!(\"hi\");".into())]));
    cx.simulate_keystrokes("tab");
    assert_eq!(text(&editor, cx), "println!(\"hi\");");
    assert_eq!(cursor(&editor, cx), PointUtf16::new(0, 15));
    assert_eq!(ghost(&editor, cx), None);
}

#[gpui::test]
fn escape_and_cursor_moves_hide_it(cx: &mut TestAppContext) {
    let (editor, cx, _) = setup(vec!["hello world"], cx);
    cx.simulate_input("he");
    settle(cx);
    assert!(ghost(&editor, cx).is_some());
    cx.simulate_keystrokes("escape");
    assert_eq!(ghost(&editor, cx), None);
    // Typing again brings it back; moving the cursor hides it.
    cx.simulate_input("l");
    settle(cx);
    assert!(ghost(&editor, cx).is_some());
    cx.simulate_keystrokes("left");
    assert_eq!(ghost(&editor, cx), None);
    // Disagreeing text hides it too.
    cx.simulate_keystrokes("right");
    cx.simulate_input("x");
    settle(cx);
    assert_eq!(ghost(&editor, cx), None);
}

#[gpui::test]
fn accept_word_by_word_and_by_line(cx: &mut TestAppContext) {
    let (editor, cx, _) = setup(vec!["let total = add(1, 2);\nprint(total)"], cx);
    cx.simulate_input("let");
    settle(cx);
    let accept_word = |cx: &mut VisualTestContext| {
        editor.update(cx, |e, cx| e.accept_next_inline_word(cx));
    };
    // " " then "total".
    accept_word(cx);
    assert_eq!(text(&editor, cx), "let ");
    accept_word(cx);
    assert_eq!(text(&editor, cx), "let total");
    assert_eq!(
        ghost(&editor, cx),
        Some(vec![(9, " = add(1, 2);\nprint(total)".into())])
    );
    editor.update(cx, |e, cx| e.accept_next_inline_line(cx));
    assert_eq!(text(&editor, cx), "let total = add(1, 2);\n");
    assert_eq!(cursor(&editor, cx), PointUtf16::new(1, 0));
}

#[gpui::test]
fn cycling_asks_explicitly_and_counts(cx: &mut TestAppContext) {
    let (editor, cx, requests) = setup(vec!["alpha", "alps", "alto"], cx);
    cx.simulate_input("al");
    settle(cx);
    assert_eq!(ghost(&editor, cx), Some(vec![(2, "pha".into())]));
    assert_eq!(
        editor.read_with(cx, |e, cx| e.inline_suggestion_index(cx)),
        None
    );
    editor.update(cx, |e, cx| e.show_next_inline_suggestion(true, cx));
    settle(cx);
    assert_eq!(ghost(&editor, cx), Some(vec![(2, "ps".into())]));
    assert!(
        requests
            .borrow()
            .contains(&InlineCompletionTriggerKind::Explicit)
    );
    assert_eq!(
        editor.read_with(cx, |e, cx| e.inline_suggestion_index(cx)),
        Some((1, 3))
    );
    editor.update(cx, |e, cx| e.show_next_inline_suggestion(false, cx));
    editor.update(cx, |e, cx| e.show_next_inline_suggestion(false, cx));
    assert_eq!(ghost(&editor, cx), Some(vec![(2, "to".into())]));
}

/// Offers an indented body at the cursor.
struct Body;
impl InlineCompletionsProvider for Body {
    fn provide_inline_completions(
        &self,
        _: &ProviderContext,
        position: PointUtf16,
        _: InlineCompletionContext,
        _: &mut App,
    ) -> Task<anyhow::Result<InlineCompletionList>> {
        Task::ready(Ok(InlineCompletionList {
            items: vec![InlineCompletion {
                insert_text: "    body();".into(),
                // From the line start, as a real provider would.
                range: Some(PointUtf16::new(position.row, 0)..position),
                ..Default::default()
            }],
            suppress_suggestions: None,
        }))
    }
}

#[gpui::test]
fn tab_indents_when_ghost_text_starts_with_indentation(cx: &mut TestAppContext) {
    cx.update(|cx| {
        gpui_monaco::init(cx);
        register_inline_completions_provider(cx, LanguageSelector::All, Body);
    });
    let (editor, cx) = cx.add_window_view(|window, cx| {
        let model = cx.new(|_| TextModel::new(""));
        CodeEditor::new(model, window, cx)
    });
    cx.update(|window, cx| window.focus(&editor.focus_handle(cx), cx));
    editor.update(cx, |e, cx| e.trigger_inline_suggest(cx));
    cx.run_until_parked();
    assert_eq!(ghost(&editor, cx), Some(vec![(0, "    body();".into())]));
    // A full indent inside the indentation: Tab indents (Monaco
    // `inlineSuggestionHasIndentationLessThanTabSize`)...
    cx.simulate_keystrokes("tab");
    assert_eq!(text(&editor, cx), "    ");
    // ...and the ghost text continues after it; now Tab accepts.
    settle(cx);
    assert_eq!(ghost(&editor, cx), Some(vec![(4, "body();".into())]));
    cx.simulate_keystrokes("tab");
    assert_eq!(text(&editor, cx), "    body();");
}

struct Words;
impl CompletionProvider for Words {
    fn provide_completions(
        &self,
        _: &ProviderContext,
        _: PointUtf16,
        _: CompletionTrigger,
        _: &mut App,
    ) -> Task<anyhow::Result<CompletionList>> {
        Task::ready(Ok(CompletionList {
            items: vec![CompletionItem::new("print", CompletionItemKind::Function)],
            incomplete: false,
        }))
    }
}

#[gpui::test]
fn suggest_preview(cx: &mut TestAppContext) {
    cx.update(|cx| {
        gpui_monaco::init(cx);
        register_completion_provider(cx, LanguageSelector::All, Words);
    });
    let (editor, cx) = cx.add_window_view(|window, cx| {
        let model = cx.new(|_| TextModel::new(""));
        CodeEditor::new(model, window, cx)
    });
    cx.update(|window, cx| window.focus(&editor.focus_handle(cx), cx));
    cx.simulate_input("pr");
    settle(cx);
    assert!(editor.read_with(cx, |e, _| e.is_suggest_visible()));
    // Off by default, as in Monaco.
    assert_eq!(ghost(&editor, cx), None);
    editor.update(cx, |e, cx| {
        let mut options = e.options().clone();
        options.suggest.preview = true;
        e.update_options(options, cx);
    });
    assert_eq!(ghost(&editor, cx), Some(vec![(2, "int".into())]));
}
