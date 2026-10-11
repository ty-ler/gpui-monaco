//! Inline completions provider fields, context and options (Monaco
//! `languages.ts` `InlineCompletion`, `InlineCompletionContext` and
//! `InlineCompletionsProvider`, and `inlineSuggest`), the suggest widget
//! integration (`suggestWidgetAdapter.ts`, `_computeAugmentation`,
//! `handleSuggestAccepted`) and the toolbar menu's keyboard. Includes
//! ports of Monaco's "Change hint is passed from onDidChange to
//! provideInlineCompletions" and "Change hint is undefined when onDidChange
//! fires without hint" (`inlineCompletions.test.ts`).

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::Duration;

use gpui::{App, AppContext as _, Entity, Focusable, Task, TestAppContext, VisualTestContext};
use gpui_monaco::options::{EditorOptions, InlineSuggestShowToolbar, ShowOnSuggestConflict};
use gpui_monaco::providers::*;
use gpui_monaco::text::PointUtf16;
use gpui_monaco::{CodeEditor, TextModel};

type Make = Rc<dyn Fn(&str, PointUtf16, &InlineCompletionContext) -> Vec<InlineCompletion>>;

/// Offers what `make` returns for the text before the cursor, and records
/// the requests and what the editor reports.
#[derive(Clone)]
struct Recorder {
    make: Make,
    contexts: Rc<RefCell<Vec<(PointUtf16, InlineCompletionContext)>>>,
    events: Rc<RefCell<Vec<String>>>,
    group: Option<&'static str>,
    excludes: Vec<&'static str>,
}

impl Recorder {
    fn new(
        make: impl Fn(&str, PointUtf16, &InlineCompletionContext) -> Vec<InlineCompletion> + 'static,
    ) -> Self {
        Self {
            make: Rc::new(make),
            contexts: Rc::default(),
            events: Rc::default(),
            group: None,
            excludes: Vec::new(),
        }
    }
}

impl InlineCompletionsProvider for Recorder {
    fn provide_inline_completions(
        &self,
        context: &ProviderContext,
        position: PointUtf16,
        completion_context: InlineCompletionContext,
        _: &mut App,
    ) -> Task<anyhow::Result<InlineCompletionList>> {
        let line = context.snapshot.line(position.row);
        let before: String = line.chars().take(position.column as usize).collect();
        let items = (self.make)(&before, position, &completion_context);
        self.contexts
            .borrow_mut()
            .push((position, completion_context));
        Task::ready(Ok(InlineCompletionList {
            items,
            ..Default::default()
        }))
    }

    fn handle_item_did_show(&self, item: &InlineCompletion, updated_insert_text: &str) {
        self.events.borrow_mut().push(format!(
            "shown {} as {updated_insert_text}",
            item.insert_text
        ));
    }

    fn handle_partial_accept(&self, item: &InlineCompletion, info: PartialAcceptInfo) {
        self.events.borrow_mut().push(format!(
            "partial {:?} {} {}",
            info.kind, info.accepted_length, item.insert_text
        ));
    }

    fn group_id(&self) -> Option<gpui::SharedString> {
        self.group.map(Into::into)
    }

    fn excludes_group_ids(&self) -> Vec<gpui::SharedString> {
        self.excludes.iter().map(|g| (*g).into()).collect()
    }
}

/// A line completion: `text` over the row up to the cursor, when what's
/// typed starts it.
fn line_completion(before: &str, position: PointUtf16, text: &str) -> Vec<InlineCompletion> {
    if before.is_empty() || !text.starts_with(before) {
        return Vec::new();
    }
    vec![InlineCompletion {
        insert_text: text.into(),
        range: Some(PointUtf16::new(position.row, 0)..position),
        ..Default::default()
    }]
}

fn setup<'a>(
    language: Option<&str>,
    providers: Vec<Recorder>,
    configure: impl FnOnce(&mut EditorOptions),
    cx: &'a mut TestAppContext,
) -> (
    Entity<CodeEditor>,
    &'a mut VisualTestContext,
    Vec<ProviderRegistration>,
) {
    let registrations = cx.update(|cx| {
        gpui_monaco::init(cx);
        gpui_monaco::set_language_registry(cx, gpui_monaco_languages::registry());
        providers
            .into_iter()
            .map(|provider| {
                register_inline_completions_provider(cx, LanguageSelector::All, provider)
            })
            .collect()
    });
    let language = language.map(str::to_string);
    let (editor, cx) = cx.add_window_view(move |window, cx| {
        let model = cx.new(|_| match language {
            Some(language) => TextModel::new("").with_language(language),
            None => TextModel::new(""),
        });
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
    (editor, cx, registrations)
}

/// Settles requests and draws (the editor follows the suggest widget and
/// renders ghost text while painting).
fn draw(cx: &mut VisualTestContext) {
    for _ in 0..2 {
        cx.executor().advance_clock(Duration::from_millis(100));
        cx.run_until_parked();
        cx.update(|window, _| window.refresh());
        cx.run_until_parked();
    }
}

fn ghost(editor: &Entity<CodeEditor>, cx: &mut VisualTestContext) -> Option<Vec<(u32, String)>> {
    editor.read_with(cx, |e, cx| {
        e.ghost_text_for_tests(cx).map(|(_, parts)| parts)
    })
}

fn text(editor: &Entity<CodeEditor>, cx: &mut VisualTestContext) -> String {
    editor.read_with(cx, |e, cx| e.model().read(cx).text())
}

fn set_cursor(editor: &Entity<CodeEditor>, at: PointUtf16, cx: &mut VisualTestContext) {
    editor.update(cx, |e, cx| {
        e.set_selections(&[gpui_monaco::cursor::Selection::collapsed(at)], cx)
    });
}

#[gpui::test]
fn complete_bracket_pairs_closes_what_it_opens(cx: &mut TestAppContext) {
    let provider = Recorder::new(|before, position, _| {
        let mut items = line_completion(before, position, "call(arg, \"(\"");
        for item in &mut items {
            item.complete_bracket_pairs = true;
        }
        items
    });
    let events = provider.events.clone();
    let (editor, cx, _) = setup(Some("rust"), vec![provider], |_| {}, cx);
    cx.simulate_input("ca");
    draw(cx);
    // The paren in the string stays open; the call is closed.
    assert_eq!(ghost(&editor, cx), Some(vec![(2, "ll(arg, \"(\")".into())]));
    assert!(
        events
            .borrow()
            .contains(&"shown call(arg, \"(\" as call(arg, \"(\")".to_string())
    );
    cx.simulate_keystrokes("tab");
    assert_eq!(text(&editor, cx), "call(arg, \"(\")");
}

#[gpui::test]
fn shown_command_runs_once(cx: &mut TestAppContext) {
    let provider = Recorder::new(|before, position, _| {
        let mut items = line_completion(before, position, "hello world");
        for item in &mut items {
            item.shown_command = Some(Command::new("test.shown", "Shown"));
        }
        items
    });
    let (editor, cx, _) = setup(None, vec![provider], |_| {}, cx);
    let runs = Rc::new(Cell::new(0));
    cx.update(|_, cx| {
        let runs = runs.clone();
        gpui_monaco::register_command(cx, "test.shown", move |_, _| runs.set(runs.get() + 1));
    });
    cx.simulate_input("he");
    draw(cx);
    assert!(ghost(&editor, cx).is_some());
    assert_eq!(runs.get(), 1);
    // Typing along keeps the same item: not again.
    cx.simulate_input("l");
    draw(cx);
    assert_eq!(runs.get(), 1);
}

#[gpui::test]
fn warnings_squiggle_and_show_their_message(cx: &mut TestAppContext) {
    let provider = Recorder::new(|before, position, _| {
        let mut items = line_completion(before, position, "unsafe { boom() }");
        for item in &mut items {
            item.warning = Some(InlineCompletionWarning {
                message: Documentation::PlainText("Might be unsound".into()),
                icon: None,
            });
        }
        items
    });
    let (editor, cx, _) = setup(
        None,
        vec![provider],
        |o| o.inline_suggest.show_toolbar = InlineSuggestShowToolbar::Always,
        cx,
    );
    cx.simulate_input("un");
    draw(cx);
    assert_eq!(
        editor.read_with(cx, |e, cx| e.inline_warning_for_tests(cx)),
        Some(("Might be unsound".to_string(), true))
    );
}

#[gpui::test]
fn partial_accepts_say_how(cx: &mut TestAppContext) {
    let provider = Recorder::new(|before, position, _| {
        line_completion(before, position, "let total = 1;\nprint(total)")
    });
    let events = provider.events.clone();
    let (editor, cx, _) = setup(None, vec![provider], |_| {}, cx);
    cx.simulate_input("let");
    draw(cx);
    editor.update(cx, |e, cx| e.accept_next_inline_word(cx));
    editor.update(cx, |e, cx| e.accept_next_inline_line(cx));
    let events = events.borrow();
    let partial: Vec<&String> = events.iter().filter(|e| e.starts_with("partial")).collect();
    assert_eq!(
        partial,
        [
            "partial Word 4 let total = 1;\nprint(total)",
            "partial Line 15 let total = 1;\nprint(total)",
        ]
    );
}

#[gpui::test]
fn requests_have_an_id_and_times(cx: &mut TestAppContext) {
    let provider = Recorder::new(|before, position, _| line_completion(before, position, "hello"));
    let contexts = provider.contexts.clone();
    let (editor, cx, _) = setup(
        None,
        vec![provider],
        |o| o.inline_suggest.min_show_delay = Duration::from_millis(30),
        cx,
    );
    cx.simulate_input("h");
    draw(cx);
    cx.simulate_input("x");
    draw(cx);
    editor.update(cx, |e, cx| e.trigger_inline_suggest(cx));
    draw(cx);
    let contexts = contexts.borrow();
    assert!(contexts.len() >= 3);
    let (_, first) = &contexts[0];
    assert!(!first.request_uuid.is_empty());
    assert_ne!(first.request_uuid, contexts[1].1.request_uuid);
    // Automatic requests show no sooner than `min_show_delay` after them.
    assert_eq!(
        first.earliest_shown_date_time,
        first.request_issued_date_time + Duration::from_millis(30)
    );
    let (_, explicit) = contexts.last().unwrap();
    assert_eq!(explicit.trigger_kind, InlineCompletionTriggerKind::Explicit);
    assert_eq!(
        explicit.earliest_shown_date_time,
        explicit.request_issued_date_time
    );
    assert!(contexts.iter().all(|(_, c)| c.change_hint.is_none()));
}

/// Monaco's "Change hint is passed from onDidChange to
/// provideInlineCompletions".
#[gpui::test]
fn change_hint_is_passed_to_the_next_request(cx: &mut TestAppContext) {
    let provider = Recorder::new(|before, position, _| line_completion(before, position, "foobar"));
    let contexts = provider.contexts.clone();
    let (editor, cx, registrations) = setup(None, vec![provider], |_| {}, cx);
    cx.simulate_input("foo");
    editor.update(cx, |e, cx| e.trigger_inline_suggest(cx));
    draw(cx);
    contexts.borrow_mut().clear();
    // Moved so the request isn't the same as the last.
    set_cursor(&editor, PointUtf16::new(0, 2), cx);
    let hint = InlineCompletionChangeHint {
        data: Some(serde_json::json!({ "reason": "modelUpdated", "version": 42 })),
    };
    let registration = registrations[0];
    cx.update(|_, cx| refresh_inline_completions(cx, registration, Some(hint.clone())));
    draw(cx);
    let contexts = contexts.borrow();
    assert_eq!(contexts.len(), 1);
    let (position, context) = &contexts[0];
    assert_eq!(*position, PointUtf16::new(0, 2));
    assert_eq!(context.trigger_kind, InlineCompletionTriggerKind::Automatic);
    assert_eq!(context.change_hint, Some(hint));
}

/// Monaco's "Change hint is undefined when onDidChange fires without hint".
#[gpui::test]
fn no_change_hint_without_one(cx: &mut TestAppContext) {
    let provider = Recorder::new(|before, position, _| line_completion(before, position, "foobar"));
    let contexts = provider.contexts.clone();
    let (editor, cx, registrations) = setup(None, vec![provider], |_| {}, cx);
    cx.simulate_input("foo");
    editor.update(cx, |e, cx| e.trigger_inline_suggest(cx));
    draw(cx);
    contexts.borrow_mut().clear();
    set_cursor(&editor, PointUtf16::new(0, 2), cx);
    let registration = registrations[0];
    cx.update(|_, cx| refresh_inline_completions(cx, registration, None));
    draw(cx);
    let contexts = contexts.borrow();
    assert_eq!(contexts.len(), 1);
    assert_eq!(contexts[0].0, PointUtf16::new(0, 2));
    assert_eq!(contexts[0].1.change_hint, None);
}

#[gpui::test]
fn refresh_asks_only_that_provider_unless_configured(cx: &mut TestAppContext) {
    let first = Recorder::new(|_, _, _| Vec::new());
    let second = Recorder::new(|before, position, _| line_completion(before, position, "second"));
    let (first_contexts, second_contexts) = (first.contexts.clone(), second.contexts.clone());
    let (editor, cx, registrations) = setup(None, vec![first, second], |_| {}, cx);
    cx.simulate_input("se");
    draw(cx);
    assert_eq!(first_contexts.borrow().len(), 1);
    assert!(ghost(&editor, cx).is_some());
    // The first provider's refresh is ignored while the second's shows.
    let first_registration = registrations[0];
    cx.update(|_, cx| refresh_inline_completions(cx, first_registration, None));
    draw(cx);
    assert_eq!(first_contexts.borrow().len(), 1);
    assert_eq!(second_contexts.borrow().len(), 1);
    // The second's asks only it.
    let second_registration = registrations[1];
    cx.update(|_, cx| refresh_inline_completions(cx, second_registration, None));
    draw(cx);
    assert_eq!(first_contexts.borrow().len(), 1);
    assert_eq!(second_contexts.borrow().len(), 2);
    // `trigger_command_on_provider_change`: everyone, for inline edits.
    editor.update(cx, |e, cx| {
        let mut options = e.options().clone();
        options.inline_suggest.trigger_command_on_provider_change = true;
        e.update_options(options, cx);
    });
    cx.update(|_, cx| refresh_inline_completions(cx, first_registration, None));
    draw(cx);
    assert_eq!(first_contexts.borrow().len(), 2);
    assert_eq!(second_contexts.borrow().len(), 3);
    let (_, last) = first_contexts.borrow().last().cloned().unwrap();
    assert!(!last.include_inline_completions);
    assert!(last.include_inline_edits);
}

#[gpui::test]
fn excluded_and_suppressed_groups_are_not_asked(cx: &mut TestAppContext) {
    let mut a = Recorder::new(|_, _, _| Vec::new());
    a.group = Some("a");
    let mut b = Recorder::new(|_, _, _| Vec::new());
    b.group = Some("b");
    b.excludes = vec!["a"];
    let mut c = Recorder::new(|_, _, _| Vec::new());
    c.group = Some("c");
    let (a_contexts, b_contexts, c_contexts) =
        (a.contexts.clone(), b.contexts.clone(), c.contexts.clone());
    let (_editor, cx, _) = setup(
        None,
        vec![a, b, c],
        |o| o.inline_suggest.experimental.suppress_inline_suggestions = vec!["c".into()],
        cx,
    );
    cx.simulate_input("x");
    draw(cx);
    assert_eq!(a_contexts.borrow().len(), 0);
    assert_eq!(b_contexts.borrow().len(), 1);
    assert_eq!(c_contexts.borrow().len(), 0);
}

/// Suggests its words for the word at the cursor.
struct Words(&'static [&'static str]);
impl CompletionProvider for Words {
    fn provide_completions(
        &self,
        _: &ProviderContext,
        _: PointUtf16,
        _: CompletionTrigger,
        _: &mut App,
    ) -> Task<anyhow::Result<CompletionList>> {
        Task::ready(Ok(CompletionList {
            items: self
                .0
                .iter()
                .map(|word| CompletionItem::new(*word, CompletionItemKind::Text))
                .collect(),
            incomplete: false,
        }))
    }
}

const HELLO: Words = Words(&["hello"]);

#[gpui::test]
fn the_suggest_selection_is_passed_and_extended(cx: &mut TestAppContext) {
    // Ghost text extending the selected suggestion, asked with it (Monaco
    // `_computeAugmentation`).
    let provider =
        Recorder::new(|before, position, _| line_completion(before, position, "hello world"));
    let (contexts, events) = (provider.contexts.clone(), provider.events.clone());
    let (editor, cx, _) = setup(None, vec![provider], |_| {}, cx);
    cx.update(|_, cx| register_completion_provider(cx, LanguageSelector::All, HELLO));
    cx.simulate_input("h");
    cx.simulate_keystrokes("ctrl-space");
    draw(cx);
    assert!(editor.read_with(cx, |e, _| e.is_suggest_visible()));
    let selected = contexts
        .borrow()
        .last()
        .and_then(|(_, c)| c.selected_suggestion_info.clone());
    assert_eq!(
        selected,
        Some(SelectedSuggestionInfo {
            range: PointUtf16::new(0, 0)..PointUtf16::new(0, 1),
            text: "hello".into(),
            completion_kind: CompletionItemKind::Text,
            is_snippet_text: false,
        })
    );
    // The preview goes on with the inline completion.
    assert_eq!(ghost(&editor, cx), Some(vec![(1, "ello world".into())]));
    // Accepting the suggestion partly accepts the inline completion, which
    // then shows the rest.
    cx.simulate_keystrokes("enter");
    draw(cx);
    assert_eq!(text(&editor, cx), "hello");
    assert!(
        events
            .borrow()
            .contains(&"partial Suggest 5 hello world".to_string())
    );
    assert_eq!(ghost(&editor, cx), Some(vec![(5, " world".into())]));
}

#[gpui::test]
fn show_on_suggest_conflict(cx: &mut TestAppContext) {
    let provider =
        Recorder::new(|before, position, _| line_completion(before, position, "hi there"));
    let contexts = provider.contexts.clone();
    let (editor, cx, _) = setup(None, vec![provider], |_| {}, cx);
    cx.update(|_, cx| {
        register_completion_provider(cx, LanguageSelector::All, Words(&["hello", "hint"]))
    });
    cx.simulate_input("h");
    draw(cx);
    assert_eq!(ghost(&editor, cx), Some(vec![(1, "i there".into())]));
    // Monaco's default: the suggest widget wins (without `suggest.preview`,
    // nothing shows).
    cx.simulate_keystrokes("ctrl-space");
    draw(cx);
    assert_eq!(ghost(&editor, cx), None);
    cx.simulate_keystrokes("escape");
    draw(cx);
    // `Always`: the inline completion shows, and requests don't pass the
    // selection.
    editor.update(cx, |e, cx| {
        let mut options = e.options().clone();
        options.inline_suggest.experimental.show_on_suggest_conflict =
            ShowOnSuggestConflict::Always;
        e.update_options(options, cx);
    });
    cx.simulate_input("i");
    draw(cx);
    assert_eq!(
        ghost(&editor, cx),
        Some(vec![(2, " there".into())]),
        "before"
    );
    contexts.borrow_mut().clear();
    cx.simulate_keystrokes("ctrl-space");
    draw(cx);
    assert!(editor.read_with(cx, |e, _| e.is_suggest_visible()));
    assert_eq!(ghost(&editor, cx), Some(vec![(2, " there".into())]));
    assert!(
        contexts
            .borrow()
            .iter()
            .all(|(_, c)| c.selected_suggestion_info.is_none())
    );
}

#[gpui::test]
fn the_toolbar_menu_takes_the_keyboard(cx: &mut TestAppContext) {
    let provider = Recorder::new(|before, position, _| {
        line_completion(before, position, "let total = 1;\nprint(total)")
    });
    let (editor, cx, _) = setup(
        None,
        vec![provider],
        |o| o.inline_suggest.show_toolbar = InlineSuggestShowToolbar::Always,
        cx,
    );
    cx.simulate_input("let");
    draw(cx);
    let menu = |cx: &mut VisualTestContext| {
        editor.read_with(cx, |e, cx| e.inline_toolbar_menu_for_tests(cx))
    };
    assert_eq!(menu(cx), None);
    editor.update_in(cx, |e, window, cx| {
        e.open_inline_toolbar_menu_for_tests(window, cx)
    });
    draw(cx);
    assert_eq!(menu(cx), Some(None));
    cx.simulate_keystrokes("down down");
    assert_eq!(menu(cx), Some(Some(1)));
    cx.simulate_keystrokes("up");
    assert_eq!(menu(cx), Some(Some(0)));
    cx.simulate_keystrokes("end");
    assert_eq!(menu(cx), Some(Some(1)));
    // Escape closes it and gives the editor the keyboard back.
    cx.simulate_keystrokes("escape");
    assert_eq!(menu(cx), None);
    assert!(editor.read_with(cx, |e, cx| e.inline_suggestion_visible(cx)));
    // Enter runs the row: Accept Line.
    editor.update_in(cx, |e, window, cx| {
        e.open_inline_toolbar_menu_for_tests(window, cx)
    });
    draw(cx);
    cx.simulate_keystrokes("down enter");
    assert_eq!(menu(cx), None);
    assert_eq!(text(&editor, cx), "let total = 1;\n");
    assert!(cx.update(|window, cx| editor.focus_handle(cx).is_focused(window)));
}
