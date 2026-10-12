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
            commands: Vec::new(),
            ..Default::default()
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
            commands: Vec::new(),
            ..Default::default()
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

/// Records what Monaco reports to a provider about its items.
#[derive(Clone, Default)]
struct Recorder {
    group: Option<&'static str>,
    yields_to: Vec<&'static str>,
    text: &'static str,
    asked: Rc<RefCell<usize>>,
    events: Rc<RefCell<Vec<String>>>,
}

impl InlineCompletionsProvider for Recorder {
    fn provide_inline_completions(
        &self,
        context: &ProviderContext,
        position: PointUtf16,
        _: InlineCompletionContext,
        _: &mut App,
    ) -> Task<anyhow::Result<InlineCompletionList>> {
        *self.asked.borrow_mut() += 1;
        let line = context.snapshot.line(position.row);
        let items = (!self.text.is_empty() && self.text.starts_with(line.trim_end()))
            .then(|| InlineCompletion {
                insert_text: self.text.into(),
                range: Some(PointUtf16::new(position.row, 0)..position),
                ..Default::default()
            })
            .into_iter()
            .collect();
        Task::ready(Ok(InlineCompletionList {
            items,
            ..Default::default()
        }))
    }

    fn handle_rejection(&self, item: &InlineCompletion) {
        self.events
            .borrow_mut()
            .push(format!("rejection {}", item.insert_text));
    }

    fn handle_end_of_lifetime(
        &self,
        item: &InlineCompletion,
        reason: &InlineCompletionEndOfLifeReason,
    ) {
        let reason = match reason {
            InlineCompletionEndOfLifeReason::Accepted { .. } => "accepted".to_string(),
            InlineCompletionEndOfLifeReason::Rejected => "rejected".to_string(),
            InlineCompletionEndOfLifeReason::Ignored { superseded_by, .. } => format!(
                "ignored{}",
                if superseded_by.is_some() {
                    " superseded"
                } else {
                    ""
                }
            ),
        };
        self.events
            .borrow_mut()
            .push(format!("{reason} {}", item.insert_text));
    }

    fn group_id(&self) -> Option<gpui::SharedString> {
        self.group.map(Into::into)
    }

    fn yields_to_group_ids(&self) -> Vec<gpui::SharedString> {
        self.yields_to.iter().map(|&g| g.into()).collect()
    }
}

fn setup_with(
    providers: Vec<Recorder>,
    configure: impl FnOnce(&mut gpui_monaco::options::EditorOptions),
    cx: &mut TestAppContext,
) -> (Entity<CodeEditor>, &mut VisualTestContext) {
    cx.update(|cx| {
        gpui_monaco::init(cx);
        for provider in providers {
            register_inline_completions_provider(cx, LanguageSelector::All, provider);
        }
    });
    let (editor, cx) = cx.add_window_view(|window, cx| {
        let model = cx.new(|_| TextModel::new(""));
        let mut editor = CodeEditor::new(model, window, cx);
        let mut options = editor.options().clone();
        options.quick_suggestions = gpui_monaco::options::QuickSuggestions::OFF;
        options.word_based_suggestions = gpui_monaco::options::WordBasedSuggestions::Off;
        configure(&mut options);
        editor.update_options(options, cx);
        editor
    });
    cx.update(|window, cx| window.focus(&editor.focus_handle(cx), cx));
    cx.run_until_parked();
    (editor, cx)
}

/// Settles and draws, so ghost text renders (and counts as shown).
fn draw(cx: &mut VisualTestContext) {
    settle(cx);
    cx.update(|window, _| window.refresh());
    cx.run_until_parked();
}

#[gpui::test]
fn end_of_lifetime_reasons(cx: &mut TestAppContext) {
    let provider = Recorder {
        text: "hello world",
        ..Default::default()
    };
    let events = provider.events.clone();
    let (editor, cx) = setup_with(vec![provider], |_| {}, cx);
    cx.simulate_input("he");
    draw(cx);
    assert!(ghost(&editor, cx).is_some());
    // Typing more asks again: the old item is superseded by the same one.
    cx.simulate_input("l");
    draw(cx);
    assert_eq!(*events.borrow(), ["ignored superseded hello world"]);
    events.borrow_mut().clear();
    // Escape rejects the shown completion.
    cx.simulate_keystrokes("escape");
    assert_eq!(
        *events.borrow(),
        ["rejection hello world", "rejected hello world"]
    );
    events.borrow_mut().clear();
    cx.simulate_input("l");
    draw(cx);
    cx.simulate_keystrokes("tab");
    assert_eq!(text(&editor, cx), "hello world");
    assert_eq!(events.borrow()[0], "accepted hello world");
}

#[gpui::test]
fn providers_yield_to_groups(cx: &mut TestAppContext) {
    let first = Recorder {
        group: Some("first"),
        text: "alpha",
        ..Default::default()
    };
    let second = Recorder {
        yields_to: vec!["first"],
        text: "another",
        ..Default::default()
    };
    let (first_asked, second_asked) = (first.asked.clone(), second.asked.clone());
    let (editor, cx) = setup_with(vec![first, second], |_| {}, cx);
    // "al": the first group has a visible completion, so the second isn't
    // asked.
    cx.simulate_input("al");
    settle(cx);
    assert_eq!(ghost(&editor, cx), Some(vec![(2, "pha".into())]));
    assert!(*first_asked.borrow() > 0);
    assert_eq!(*second_asked.borrow(), 0);
    // "an": nothing from the first group, so the second is asked.
    cx.simulate_keystrokes("escape backspace");
    cx.simulate_input("n");
    settle(cx);
    assert!(*second_asked.borrow() > 0);
    assert_eq!(ghost(&editor, cx), Some(vec![(2, "other".into())]));
}

#[gpui::test]
fn the_always_shown_toolbar(cx: &mut TestAppContext) {
    use gpui_monaco::options::InlineSuggestShowToolbar;
    let (editor, cx, requests) = setup(vec!["println!(\"hi\");", "print!(\"hi\");"], cx);
    editor.update(cx, |e, cx| {
        let mut options = e.options().clone();
        options.inline_suggest.show_toolbar = InlineSuggestShowToolbar::Always;
        e.update_options(options, cx);
    });
    cx.simulate_input("pri");
    draw(cx);
    // The toolbar shows at the ghost text and asks for all completions.
    let at = |cx: &mut VisualTestContext| {
        editor.read_with(cx, |e, _| e.inline_toolbar_position_for_tests())
    };
    assert_eq!(at(cx), Some(PointUtf16::new(0, 3)));
    assert!(
        requests
            .borrow()
            .contains(&InlineCompletionTriggerKind::Explicit)
    );
    draw(cx);
    assert_eq!(
        editor.read_with(cx, |e, cx| e.inline_suggestion_index(cx)),
        Some((0, 2))
    );
    // Typing on keeps it where it first showed on the line.
    cx.simulate_input("n");
    draw(cx);
    assert_eq!(at(cx), Some(PointUtf16::new(0, 3)));
    // Always Show Toolbar off: it goes.
    cx.update(|window, cx| {
        editor
            .update(cx, |e, cx| {
                e.trigger(
                    "editor.action.inlineSuggest.toggleAlwaysShowToolbar",
                    window,
                    cx,
                )
            })
            .unwrap()
    });
    draw(cx);
    assert_eq!(at(cx), None);
    assert_eq!(
        editor.read_with(cx, |e, _| e.options().inline_suggest.show_toolbar),
        InlineSuggestShowToolbar::OnHover
    );
}

#[gpui::test]
fn the_toolbar_in_the_hover(cx: &mut TestAppContext) {
    let (editor, cx, _) = setup(vec!["println!(\"hi\");"], cx);
    cx.simulate_input("pri");
    draw(cx);
    // The cursor is where the ghost text starts, so the hover there has the
    // toolbar.
    cx.dispatch_action(gpui_monaco::actions::ShowHover::default());
    draw(cx);
    assert!(editor.read_with(cx, |e, cx| e.hover_has_inline_toolbar_for_tests(cx)));
    assert!(editor.read_with(cx, |e, _| e.is_hover_visible()));
}

#[gpui::test]
fn syntax_highlighted_ghost_text(cx: &mut TestAppContext) {
    let provider = Recorder {
        text: "let x = 42;",
        ..Default::default()
    };
    cx.update(|cx| {
        gpui_monaco::init(cx);
        gpui_monaco::set_language_registry(cx, gpui_monaco_languages::registry());
        register_inline_completions_provider(cx, LanguageSelector::All, provider);
    });
    let (editor, cx) = cx.add_window_view(|window, cx| {
        let model = cx.new(|_| TextModel::new("").with_language("rust"));
        CodeEditor::new(model, window, cx)
    });
    cx.update(|window, cx| window.focus(&editor.focus_handle(cx), cx));
    cx.simulate_input("le");
    draw(cx);
    // The ghost text's pieces, each as its own injected text.
    let pieces = |editor: &Entity<CodeEditor>, cx: &mut VisualTestContext| {
        editor.update(cx, |e, cx| {
            let id = cx.entity_id();
            e.model().update(cx, |model, _| {
                model
                    .all_decorations(Some(id))
                    .into_iter()
                    .filter_map(|d| d.options.after.clone())
                    .map(|after| (after.content.to_string(), after.style))
                    .collect::<Vec<_>>()
            })
        })
    };
    let highlighted = pieces(&editor, cx);
    let text: String = highlighted.iter().map(|(t, _)| t.as_str()).collect();
    assert_eq!(text, "t x = 42;");
    assert!(highlighted.len() > 1, "{highlighted:?}");
    assert!(
        highlighted
            .iter()
            .all(|(_, s)| s.italic && s.opacity == Some(0.7))
    );
    // The keyword and the punctuation are colored differently.
    let color = |piece: &str| {
        highlighted
            .iter()
            .find(|(t, _)| t == piece)
            .map(|(_, s)| s.foreground)
    };
    assert_ne!(color("t"), color(";"), "{highlighted:?}");
    // Off: one plain piece in the ghost text color.
    editor.update(cx, |e, cx| {
        let mut options = e.options().clone();
        options.inline_suggest.syntax_highlighting_enabled = false;
        e.update_options(options, cx);
        e.hide_inline_suggestion(cx);
    });
    cx.simulate_input("t");
    draw(cx);
    let plain = pieces(&editor, cx);
    assert_eq!(plain.len(), 1, "{plain:?}");
    assert_eq!(plain[0].0, " x = 42;");
}

/// One completion item, `label`, for the word at the cursor.
struct Item(&'static str);

impl CompletionProvider for Item {
    fn provide_completions(
        &self,
        _: &ProviderContext,
        _: PointUtf16,
        _: CompletionTrigger,
        _: &mut App,
    ) -> Task<anyhow::Result<CompletionList>> {
        Task::ready(Ok(CompletionList {
            items: vec![CompletionItem::new(self.0, CompletionItemKind::Function)],
            incomplete: false,
        }))
    }
}

#[gpui::test]
fn quick_suggestions_inline_join_the_providers(cx: &mut TestAppContext) {
    // Monaco `quickSuggestions: 'inline'`: completion items are one more
    // inline completions provider, after the registered ones.
    let (editor, cx, _) = setup(vec!["alpha"], cx);
    cx.update(|_, cx| register_completion_provider(cx, LanguageSelector::All, Item("alps")));
    editor.update(cx, |e, cx| {
        let mut options = e.options().clone();
        options.quick_suggestions = gpui_monaco::options::QuickSuggestions::INLINE;
        e.update_options(options, cx);
    });
    cx.simulate_input("al");
    settle(cx);
    assert!(!editor.read_with(cx, |e, _| e.is_suggest_visible()));
    assert_eq!(ghost(&editor, cx), Some(vec![(2, "pha".into())]));
    editor.update(cx, |e, cx| e.show_next_inline_suggestion(true, cx));
    settle(cx);
    assert_eq!(ghost(&editor, cx), Some(vec![(2, "ps".into())]));
    assert_eq!(
        editor.read_with(cx, |e, cx| e.inline_suggestion_index(cx)),
        Some((1, 2))
    );
    cx.simulate_keystrokes("tab");
    assert_eq!(text(&editor, cx), "alps");
}

// ----- Inline completions leftovers (#297) ---------------------------------

/// A list disposed: its items' texts and the reason.
type Disposed = (Vec<String>, InlineCompletionsDisposeReason);

/// Monaco's `MockInlineCompletionsProvider`: offers the items set, records
/// the calls (position, line text, trigger kind) and the lists disposed.
#[derive(Clone, Default)]
struct Mock {
    items: Rc<RefCell<Vec<InlineCompletion>>>,
    forward_stable: bool,
    calls: Rc<RefCell<Vec<(PointUtf16, String, InlineCompletionTriggerKind)>>>,
    disposed: Rc<RefCell<Vec<Disposed>>>,
}

impl Mock {
    /// Monaco `setReturnValue(s)`: plain completions with the default range.
    fn set(&self, texts: &[&str]) {
        *self.items.borrow_mut() = texts
            .iter()
            .map(|text| InlineCompletion {
                insert_text: text.to_string(),
                ..Default::default()
            })
            .collect();
    }

    fn calls(&self) -> Vec<(PointUtf16, String, InlineCompletionTriggerKind)> {
        std::mem::take(&mut *self.calls.borrow_mut())
    }
}

impl InlineCompletionsProvider for Mock {
    fn provide_inline_completions(
        &self,
        context: &ProviderContext,
        position: PointUtf16,
        completion_context: InlineCompletionContext,
        _: &mut App,
    ) -> Task<anyhow::Result<InlineCompletionList>> {
        self.calls.borrow_mut().push((
            position,
            context.snapshot.line(position.row),
            completion_context.trigger_kind,
        ));
        Task::ready(Ok(InlineCompletionList {
            items: self.items.borrow().clone(),
            enable_forward_stability: self.forward_stable,
            ..Default::default()
        }))
    }

    fn dispose_inline_completions(
        &self,
        list: &InlineCompletionList,
        reason: InlineCompletionsDisposeReason,
    ) {
        self.disposed.borrow_mut().push((
            list.items.iter().map(|i| i.insert_text.clone()).collect(),
            reason,
        ));
    }
}

fn setup_mock<'a>(
    text: &str,
    provider: Mock,
    configure: impl FnOnce(&mut gpui_monaco::options::EditorOptions),
    cx: &'a mut TestAppContext,
) -> (Entity<CodeEditor>, &'a mut VisualTestContext) {
    cx.update(|cx| {
        gpui_monaco::init(cx);
        register_inline_completions_provider(cx, LanguageSelector::All, provider);
    });
    let text = text.to_string();
    let (editor, cx) = cx.add_window_view(move |window, cx| {
        let model = cx.new(|_| TextModel::new(&text));
        let mut editor = CodeEditor::new(model, window, cx);
        let mut options = editor.options().clone();
        options.quick_suggestions = gpui_monaco::options::QuickSuggestions::OFF;
        options.word_based_suggestions = gpui_monaco::options::WordBasedSuggestions::Off;
        configure(&mut options);
        editor.update_options(options, cx);
        editor
    });
    cx.update(|window, cx| window.focus(&editor.focus_handle(cx), cx));
    cx.run_until_parked();
    (editor, cx)
}

fn set_cursors(editor: &Entity<CodeEditor>, at: &[PointUtf16], cx: &mut VisualTestContext) {
    let selections: Vec<_> = at
        .iter()
        .map(|p| gpui_monaco::cursor::Selection::collapsed(*p))
        .collect();
    editor.update(cx, |e, cx| e.set_selections(&selections, cx));
}

fn trigger_explicitly(editor: &Entity<CodeEditor>, cx: &mut VisualTestContext) {
    editor.update(cx, |e, cx| e.trigger_inline_suggest(cx));
    draw(cx);
}

fn p(row: u32, column: u32) -> PointUtf16 {
    PointUtf16::new(row, column)
}

/// Ported from `inlineCompletions.test.ts`, "Forward Stability": "Typing
/// agrees" and "Support forward instability".
#[gpui::test]
fn forward_instability_shows_the_new_completion(cx: &mut TestAppContext) {
    let provider = Mock::default();
    let (editor, cx) = setup_mock("", provider.clone(), |_| {}, cx);
    provider.set(&["foo bar"]);
    cx.simulate_input("f");
    trigger_explicitly(&editor, cx);
    assert_eq!(ghost(&editor, cx), Some(vec![(1, "oo bar".into())]));
    provider.calls();
    // The provider changes its mind: without forward stability the new
    // completion shows once typing asks again.
    provider.set(&["foo baz"]);
    cx.simulate_input("o");
    assert_eq!(ghost(&editor, cx), Some(vec![(2, "o bar".into())]));
    draw(cx);
    assert_eq!(
        provider.calls(),
        [(p(0, 2), "fo".into(), InlineCompletionTriggerKind::Automatic)]
    );
    assert_eq!(ghost(&editor, cx), Some(vec![(2, "o baz".into())]));
}

/// `enableForwardStability`: while typing agrees with the shown completion,
/// it stays first, ahead of what newer requests return.
#[gpui::test]
fn forward_stability_keeps_the_shown_completion(cx: &mut TestAppContext) {
    let provider = Mock {
        forward_stable: true,
        ..Default::default()
    };
    let (editor, cx) = setup_mock("", provider.clone(), |_| {}, cx);
    provider.set(&["foo bar"]);
    cx.simulate_input("f");
    draw(cx);
    assert_eq!(ghost(&editor, cx), Some(vec![(1, "oo bar".into())]));
    provider.set(&["foo baz"]);
    cx.simulate_input("o");
    draw(cx);
    assert_eq!(provider.calls().last().unwrap().1, "fo");
    assert_eq!(ghost(&editor, cx), Some(vec![(2, "o bar".into())]));
    // Typing that disagrees drops it; the provider's completion shows.
    cx.simulate_input("o b");
    draw(cx);
    assert_eq!(ghost(&editor, cx), Some(vec![(5, "ar".into())]));
    provider.set(&["foo bz"]);
    cx.simulate_input("z");
    draw(cx);
    assert_eq!(ghost(&editor, cx), None);
}

/// Ported from "Push item to preserve to front": an explicit request keeps
/// the shown item first (Monaco's `preserveCurrentCompletion`).
#[gpui::test]
fn explicit_requests_keep_the_shown_completion_first(cx: &mut TestAppContext) {
    let provider = Mock {
        forward_stable: true,
        ..Default::default()
    };
    let (editor, cx) = setup_mock("", provider.clone(), |_| {}, cx);
    *provider.items.borrow_mut() = vec![InlineCompletion {
        insert_text: "foobar".into(),
        range: Some(p(0, 0)..p(0, 3)),
        ..Default::default()
    }];
    cx.simulate_input("foo");
    draw(cx);
    assert_eq!(ghost(&editor, cx), Some(vec![(3, "bar".into())]));
    *provider.items.borrow_mut() = ["foobar1", "foobar"]
        .iter()
        .map(|text| InlineCompletion {
            insert_text: text.to_string(),
            range: Some(p(0, 0)..p(0, 3)),
            ..Default::default()
        })
        .collect();
    provider.calls();
    trigger_explicitly(&editor, cx);
    assert_eq!(
        provider.calls(),
        [(p(0, 3), "foo".into(), InlineCompletionTriggerKind::Explicit)]
    );
    assert_eq!(ghost(&editor, cx), Some(vec![(3, "bar".into())]));
}

/// Monaco `disposeInlineCompletions`: each list is disposed once, when no
/// item of it is used any more, with why.
#[gpui::test]
fn lists_are_disposed_with_a_reason(cx: &mut TestAppContext) {
    use InlineCompletionsDisposeReason::*;
    let provider = Mock::default();
    let disposed = provider.disposed.clone();
    let (editor, cx) = setup_mock(
        "",
        provider.clone(),
        |options| options.inline_suggest.edits.enabled = false,
        cx,
    );
    let take = || std::mem::take(&mut *disposed.borrow_mut());
    // An empty list.
    cx.simulate_input("x");
    draw(cx);
    assert_eq!(take(), [(vec![], Empty)]);
    // A shown list lives while its item does; asking again replaces it.
    provider.set(&["xyzw"]);
    cx.simulate_input("y");
    draw(cx);
    assert!(ghost(&editor, cx).is_some());
    assert_eq!(take(), []);
    cx.simulate_input("z");
    draw(cx);
    assert_eq!(take(), [(vec!["xyzw".to_string()], Other)]);
    // Escape ends the item, and so the list.
    cx.simulate_keystrokes("escape");
    assert_eq!(take(), [(vec!["xyzw".to_string()], Other)]);
    // A list none of whose items is kept: inline edits aren't asked for.
    *provider.items.borrow_mut() = vec![InlineCompletion {
        insert_text: "edit".into(),
        range: Some(p(0, 0)..p(0, 1)),
        is_inline_edit: true,
        ..Default::default()
    }];
    cx.simulate_input("!");
    draw(cx);
    assert_eq!(take(), [(vec!["edit".to_string()], NotTaken)]);
    // A list that came back for a request superseded before it was used.
    editor.update(cx, |e, cx| {
        let mut options = e.options().clone();
        options.inline_suggest.min_show_delay = Duration::from_millis(500);
        e.update_options(options, cx);
    });
    provider.set(&["xyz!??x"]);
    cx.simulate_input("?");
    settle(cx);
    assert_eq!(take(), []);
    cx.simulate_input("?");
    assert_eq!(take(), [(vec!["xyz!??x".to_string()], TokenCancellation)]);
}

/// Ported from `inlineCompletions.test.ts`, "Multi Cursor Support":
/// "Basic", "Multi Part" and "Multi Part and Different Cursor Columns".
#[gpui::test]
fn secondary_cursors_get_the_completion(cx: &mut TestAppContext) {
    let provider = Mock::default();
    let (editor, cx) = setup_mock("console\nconsole\n", provider.clone(), |_| {}, cx);
    *provider.items.borrow_mut() = vec![InlineCompletion {
        insert_text: "console.log(\"hello\");".into(),
        range: Some(p(0, 0)..p(0, 999)),
        ..Default::default()
    }];
    set_cursors(&editor, &[p(0, 7), p(1, 7)], cx);
    trigger_explicitly(&editor, cx);
    // Ghost text at both cursors.
    assert_eq!(
        editor.read_with(cx, |e, cx| e.ghost_texts_for_tests(cx)),
        [
            (0, vec![(7, ".log(\"hello\");".to_string())]),
            (1, vec![(7, ".log(\"hello\");".to_string())])
        ]
    );
    editor.update(cx, |e, cx| e.accept_inline_suggestion(cx));
    assert_eq!(
        text(&editor, cx),
        "console.log(\"hello\");\nconsole.log(\"hello\");\n"
    );
    // A cursor after each.
    assert_eq!(
        editor.read_with(cx, |e, _| e
            .selections()
            .iter()
            .map(|s| s.head)
            .collect::<Vec<_>>()),
        [p(0, 21), p(1, 21)]
    );
}

/// Explicitly asks for `insert` (replacing the first line) with cursors
/// at `cursors` in `text_before`, then accepts it whole or (`words`
/// times) word by word, asking again before each, as Monaco's tests do.
fn accept_at_cursors(
    text_before: &str,
    cursors: &[PointUtf16],
    insert: &str,
    words: usize,
    cx: &mut TestAppContext,
) -> String {
    let provider = Mock::default();
    let (editor, cx) = setup_mock(text_before, provider.clone(), |_| {}, cx);
    *provider.items.borrow_mut() = vec![InlineCompletion {
        insert_text: insert.into(),
        range: Some(p(0, 0)..p(0, 999)),
        ..Default::default()
    }];
    set_cursors(&editor, cursors, cx);
    if words == 0 {
        trigger_explicitly(&editor, cx);
        editor.update(cx, |e, cx| e.accept_inline_suggestion(cx));
    }
    for _ in 0..words {
        trigger_explicitly(&editor, cx);
        editor.update(cx, |e, cx| e.accept_next_inline_word(cx));
    }
    text(&editor, cx)
}

/// Ported from "Multi Part": the text after the second cursor differs, so
/// it gets nothing.
#[gpui::test]
fn secondary_cursors_need_the_same_surroundings(cx: &mut TestAppContext) {
    assert_eq!(
        accept_at_cursors(
            "console.log()\nconsole.log\n",
            &[p(0, 11), p(1, 11)],
            "console.log(\"hello\");",
            0,
            cx
        ),
        "console.log(\"hello\");\nconsole.log\n"
    );
}

/// Ported from "Multi Part and Different Cursor Columns".
#[gpui::test]
fn secondary_cursors_at_other_columns(cx: &mut TestAppContext) {
    assert_eq!(
        accept_at_cursors(
            "console.log()\nconsole.warn\n",
            &[p(0, 11), p(1, 13)],
            "console.log(\"hello\");",
            0,
            cx
        ),
        "console.log(\"hello\");\nconsole.warn\n"
    );
}

/// Ported from "Basic Partial Completion".
#[gpui::test]
fn secondary_cursors_accept_word_by_word(cx: &mut TestAppContext) {
    assert_eq!(
        accept_at_cursors(
            "let\nlet\n",
            &[p(0, 3), p(1, 3)],
            "let a = 'some word'; ",
            2,
            cx
        ),
        "let a\nlet a\n"
    );
}

/// Ported from "Partial Multi-Part Completion".
#[gpui::test]
fn secondary_cursors_partial_multi_part(cx: &mut TestAppContext) {
    assert_eq!(
        accept_at_cursors(
            "for ()\nfor \n",
            &[p(0, 4), p(1, 4)],
            "for (let i = 0; i < 10; i++) {",
            3,
            cx
        ),
        "for (let i)\nfor \n"
    );
}

/// Ported from "Partial Mutli-Part and Different Cursor Columns
/// Completion".
#[gpui::test]
fn secondary_cursors_partial_other_columns(cx: &mut TestAppContext) {
    assert_eq!(
        accept_at_cursors(
            "console.log()\nconsole.warnnnn\n",
            &[p(0, 11), p(1, 15)],
            "console.log(\"hello\" + \" \" + \"world\");",
            4,
            cx
        ),
        "console.log(\"hello\" + )\nconsole.warnnnn\n"
    );
}

/// Monaco's ghost text view: a multi-line completion inserted mid-line
/// moves the rest of the line to the end of its last line, hiding it in
/// the line itself.
#[gpui::test]
fn multi_line_ghost_text_mid_line_moves_the_rest(cx: &mut TestAppContext) {
    let provider = Mock::default();
    let (editor, cx) = setup_mock("call();\nnext", provider.clone(), |_| {}, cx);
    *provider.items.borrow_mut() = vec![InlineCompletion {
        insert_text: "call(x, {\n  y\n}".into(),
        range: Some(p(0, 0)..p(0, 5)),
        ..Default::default()
    }];
    set_cursors(&editor, &[p(0, 5)], cx);
    trigger_explicitly(&editor, cx);
    draw(cx);
    assert_eq!(ghost(&editor, cx), Some(vec![(5, "x, {\n  y\n}".into())]));
    assert_eq!(
        editor.read_with(cx, |e, cx| e.ghost_text_view_for_tests(cx)),
        [(
            0,
            Some(5),
            vec![("  y".to_string(), false), ("});".to_string(), false)]
        )]
    );
    // The line shows up to the cursor (and the ghost text's first line).
    let segments = editor.read_with(cx, |e, _| e.view_line_segments_for_tests());
    assert!(segments.contains(&(0, 0..5)), "{segments:?}");
    // Accepting inserts it before the rest of the line.
    editor.update(cx, |e, cx| e.accept_inline_suggestion(cx));
    assert_eq!(text(&editor, cx), "call(x, {\n  y\n});\nnext");
    draw(cx);
    let segments = editor.read_with(cx, |e, _| e.view_line_segments_for_tests());
    assert!(segments.contains(&(2, 0..3)), "{segments:?}");
    // At the end of a line, nothing is hidden.
    editor.update(cx, |e, cx| e.hide_inline_suggestion(cx));
    *provider.items.borrow_mut() = vec![InlineCompletion {
        insert_text: "next {\n}".into(),
        range: Some(p(3, 0)..p(3, 4)),
        ..Default::default()
    }];
    set_cursors(&editor, &[p(3, 4)], cx);
    trigger_explicitly(&editor, cx);
    assert_eq!(
        editor.read_with(cx, |e, cx| e.ghost_text_view_for_tests(cx)),
        [(3, None, vec![("}".to_string(), false)])]
    );
}

/// Monaco's `warning` class covers every line of the ghost text, not the
/// line's text moved below it.
#[gpui::test]
fn warning_squiggles_every_ghost_line(cx: &mut TestAppContext) {
    let provider = Mock::default();
    let (editor, cx) = setup_mock("f()", provider.clone(), |_| {}, cx);
    *provider.items.borrow_mut() = vec![InlineCompletion {
        insert_text: "f(a,\n  b,\n  c".into(),
        range: Some(p(0, 0)..p(0, 2)),
        warning: Some(InlineCompletionWarning {
            message: "Careful".into(),
            icon: None,
        }),
        ..Default::default()
    }];
    set_cursors(&editor, &[p(0, 2)], cx);
    trigger_explicitly(&editor, cx);
    assert_eq!(
        editor.read_with(cx, |e, cx| e.inline_warning_for_tests(cx)),
        Some(("Careful".to_string(), true))
    );
    assert_eq!(
        editor.read_with(cx, |e, cx| e.ghost_text_view_for_tests(cx)),
        [(
            0,
            Some(2),
            vec![("  b,".to_string(), true), ("  c)".to_string(), true)]
        )]
    );
}

/// Several cursors each show the ghost text, with their own additional
/// lines.
#[gpui::test]
fn ghost_text_at_every_cursor(cx: &mut TestAppContext) {
    let provider = Mock::default();
    let (editor, cx) = setup_mock("fn a(\nfn a(\nfn b(", provider.clone(), |_| {}, cx);
    *provider.items.borrow_mut() = vec![InlineCompletion {
        insert_text: "fn a() {\n}".into(),
        range: Some(p(0, 0)..p(0, 5)),
        ..Default::default()
    }];
    set_cursors(&editor, &[p(0, 5), p(1, 5), p(2, 5)], cx);
    trigger_explicitly(&editor, cx);
    // At the third too: only the text the edit replaces (none here) has to
    // match, as in Monaco.
    assert_eq!(
        editor.read_with(cx, |e, cx| e.ghost_text_view_for_tests(cx)),
        [
            (0, None, vec![("}".to_string(), false)]),
            (1, None, vec![("}".to_string(), false)]),
            (2, None, vec![("}".to_string(), false)])
        ]
    );
    editor.update(cx, |e, cx| e.accept_inline_suggestion(cx));
    assert_eq!(text(&editor, cx), "fn a() {\n}\nfn a() {\n}\nfn b() {\n}");
}

/// Monaco `SuggestWidgetAdaptor`'s selector: the suggest widget selects
/// the suggestion the inline completion extends, the longest of several.
#[gpui::test]
fn suggest_preselects_what_the_inline_completion_extends(cx: &mut TestAppContext) {
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
                items: ["select", "selection", "selections"]
                    .into_iter()
                    .map(|label| CompletionItem::new(label, CompletionItemKind::Method))
                    .collect(),
                incomplete: false,
            }))
        }
    }
    let provider = Mock::default();
    let (editor, cx) = setup_mock("", provider.clone(), |_| {}, cx);
    cx.update(|_, cx| register_completion_provider(cx, LanguageSelector::All, Words));
    let selected = |cx: &mut VisualTestContext| {
        editor.read_with(cx, |e, _| {
            e.selected_suggestion()
                .map(|(_, item)| item.label.to_string())
        })
    };
    // Without an inline completion, the memory's choice.
    cx.simulate_input("se");
    cx.simulate_keystrokes("ctrl-space");
    draw(cx);
    let default = selected(cx);
    assert!(default.is_some() && default.as_deref() != Some("selections"));
    cx.simulate_keystrokes("escape");
    // With one, what it extends: "selections" inserts the most of it.
    provider.set(&["selections.len()"]);
    cx.simulate_input("l");
    draw(cx);
    assert_eq!(ghost(&editor, cx), Some(vec![(3, "ections.len()".into())]));
    cx.simulate_keystrokes("ctrl-space");
    draw(cx);
    assert_eq!(selected(cx).as_deref(), Some("selections"));
    cx.simulate_keystrokes("escape escape");
    // One it doesn't extend: the memory's choice.
    provider.set(&["sel_all"]);
    cx.simulate_input("e");
    draw(cx);
    cx.simulate_keystrokes("ctrl-space");
    draw(cx);
    assert_eq!(selected(cx), default);
}
