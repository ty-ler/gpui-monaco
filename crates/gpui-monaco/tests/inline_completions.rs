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
            InlineCompletionEndOfLifeReason::Accepted => "accepted".to_string(),
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
    cx.dispatch_action(gpui_monaco::actions::ShowHover);
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
