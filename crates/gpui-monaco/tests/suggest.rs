//! Suggest controller behavior (Monaco `suggestController.test.ts`,
//! `suggestModel.test.ts`): insert modes, Enter modes, selection memories,
//! quick suggestions, word-based suggestions, filtering, resolve and
//! commands.

use std::cell::Cell;
use std::rc::Rc;
use std::time::Duration;

use gpui::{App, AppContext as _, Entity, Focusable, Task, TestAppContext, VisualTestContext};
use gpui_monaco::options::{
    AcceptSuggestionOnEnter, QuickSuggestions, SuggestInsertMode, SuggestSelection,
    WordBasedSuggestions,
};
use gpui_monaco::providers::*;
use gpui_monaco::text::PointUtf16;
use gpui_monaco::{CodeEditor, EditorOptions, TextModel, register_command};

fn setup<'a>(
    text: &str,
    options: impl FnOnce(&mut EditorOptions),
    cx: &'a mut TestAppContext,
) -> (Entity<CodeEditor>, &'a mut VisualTestContext) {
    cx.update(gpui_monaco::init);
    let text = text.to_string();
    let (editor, cx) = cx.add_window_view(|window, cx| {
        let model = cx.new(|_| {
            TextModel::new(&text)
                .with_language("demo")
                .with_uri("file:///main.demo")
        });
        CodeEditor::new(model, window, cx)
    });
    editor.update(cx, |editor, cx| {
        let mut opts = editor.options().clone();
        options(&mut opts);
        editor.update_options(opts, cx);
    });
    cx.update(|window, cx| window.focus(&editor.focus_handle(cx), cx));
    cx.run_until_parked();
    (editor, cx)
}

/// Waits out the quick suggestions delay.
fn settle(cx: &mut VisualTestContext) {
    cx.executor().advance_clock(Duration::from_millis(50));
    cx.run_until_parked();
}

fn text(editor: &Entity<CodeEditor>, cx: &mut VisualTestContext) -> String {
    editor.read_with(cx, |e, cx| e.model().read(cx).text())
}

fn labels(editor: &Entity<CodeEditor>, cx: &mut VisualTestContext) -> Option<Vec<String>> {
    editor.read_with(cx, |e, _| e.suggest_labels_for_tests())
}

fn visible(editor: &Entity<CodeEditor>, cx: &mut VisualTestContext) -> bool {
    editor.read_with(cx, |e, _| e.is_suggest_visible())
}

fn move_to(editor: &Entity<CodeEditor>, cx: &mut VisualTestContext, row: u32, column: u32) {
    editor.update(cx, |e, cx| {
        e.set_selections(
            &[gpui_monaco::cursor::Selection::collapsed(PointUtf16::new(
                row, column,
            ))],
            cx,
        )
    });
}

/// Items from a closure, so each test describes its own.
struct Items<F>(F);
impl<F: Fn(PointUtf16) -> Vec<CompletionItem> + 'static> CompletionProvider for Items<F> {
    fn provide_completions(
        &self,
        _: &ProviderContext,
        position: PointUtf16,
        _: CompletionTrigger,
        _: &mut App,
    ) -> Task<anyhow::Result<CompletionList>> {
        Task::ready(Ok(CompletionList {
            items: (self.0)(position),
            incomplete: false,
        }))
    }
}

fn provide(cx: &mut VisualTestContext, f: impl Fn(PointUtf16) -> Vec<CompletionItem> + 'static) {
    cx.update(|_, cx| register_completion_provider(cx, LanguageSelector::All, Items(f)));
}

/// An item replacing the word `[start, end)` on row 0, inserting up to the
/// cursor.
fn ranged(label: &str, start: u32, end: u32, position: PointUtf16) -> CompletionItem {
    CompletionItem {
        range: Some(PointUtf16::new(0, start)..position),
        replace_range: Some(PointUtf16::new(0, start)..PointUtf16::new(0, end)),
        ..CompletionItem::new(label, CompletionItemKind::Property)
    }
}

#[gpui::test]
fn insert_and_replace_modes(cx: &mut TestAppContext) {
    // Monaco: "abc" at "a|bc": insert keeps "bc", replace overwrites it.
    let (editor, cx) = setup("abc", |_| {}, cx);
    provide(cx, |p| vec![ranged("aaa", 0, 3, p)]);
    move_to(&editor, cx, 0, 1);
    cx.simulate_keystrokes("ctrl-space");
    cx.run_until_parked();
    cx.simulate_keystrokes("enter");
    assert_eq!(text(&editor, cx), "aaabc");

    // Shift+Enter accepts in the other mode.
    editor.update(cx, |e, cx| {
        e.model().update(cx, |m, cx| m.set_value("abc", cx))
    });
    move_to(&editor, cx, 0, 1);
    cx.simulate_keystrokes("ctrl-space");
    cx.run_until_parked();
    cx.simulate_keystrokes("shift-enter");
    assert_eq!(text(&editor, cx), "aaa");

    // With `insert_mode: Replace` it's the other way round.
    editor.update(cx, |e, cx| {
        let mut options = e.options().clone();
        options.suggest.insert_mode = SuggestInsertMode::Replace;
        e.update_options(options, cx);
        e.model().update(cx, |m, cx| m.set_value("abc", cx));
    });
    move_to(&editor, cx, 0, 1);
    cx.simulate_keystrokes("ctrl-space");
    cx.run_until_parked();
    cx.simulate_keystrokes("tab");
    assert_eq!(text(&editor, cx), "aaa");
    editor.update(cx, |e, cx| {
        e.model().update(cx, |m, cx| m.set_value("abc", cx))
    });
    move_to(&editor, cx, 0, 1);
    cx.simulate_keystrokes("ctrl-space");
    cx.run_until_parked();
    cx.simulate_keystrokes("shift-tab");
    assert_eq!(text(&editor, cx), "aaabc");
}

#[gpui::test]
fn replace_without_a_range_overwrites_the_word(cx: &mut TestAppContext) {
    let (editor, cx) = setup(
        "pr_old",
        |o| o.suggest.insert_mode = SuggestInsertMode::Replace,
        cx,
    );
    provide(cx, |_| {
        vec![CompletionItem::new("print", CompletionItemKind::Function)]
    });
    move_to(&editor, cx, 0, 2);
    cx.simulate_keystrokes("ctrl-space");
    cx.run_until_parked();
    cx.simulate_keystrokes("enter");
    assert_eq!(text(&editor, cx), "print");
}

#[gpui::test]
fn accept_on_enter_modes(cx: &mut TestAppContext) {
    let (editor, cx) = setup(
        "",
        |o| o.accept_suggestion_on_enter = AcceptSuggestionOnEnter::Off,
        cx,
    );
    provide(cx, |_| {
        vec![CompletionItem::new("print", CompletionItemKind::Function)]
    });
    cx.simulate_input("pr");
    settle(cx);
    assert!(visible(&editor, cx));
    cx.simulate_keystrokes("enter");
    assert_eq!(text(&editor, cx), "pr\n");
    // Tab still accepts.
    cx.simulate_input("pr");
    settle(cx);
    cx.simulate_keystrokes("tab");
    assert_eq!(text(&editor, cx), "pr\nprint");

    // Smart: Enter types a new line when the word is already complete in a
    // session that opened by itself...
    editor.update(cx, |e, cx| {
        let mut options = e.options().clone();
        options.accept_suggestion_on_enter = AcceptSuggestionOnEnter::Smart;
        e.update_options(options, cx);
        e.model().update(cx, |m, cx| m.set_value("", cx));
    });
    cx.simulate_input("print");
    settle(cx);
    assert!(visible(&editor, cx));
    cx.simulate_keystrokes("enter");
    assert_eq!(text(&editor, cx), "print\n");
    assert!(!visible(&editor, cx));
    // ...accepts when it changes the text...
    cx.simulate_input("pri");
    settle(cx);
    cx.simulate_keystrokes("enter");
    assert_eq!(text(&editor, cx), "print\nprint");
    // ...and always accepts in an explicitly invoked session.
    cx.simulate_input(" print");
    cx.simulate_keystrokes("escape ctrl-space");
    cx.run_until_parked();
    assert!(visible(&editor, cx));
    cx.simulate_keystrokes("enter");
    assert_eq!(text(&editor, cx), "print\nprint print");
}

#[gpui::test]
fn recently_used_selection(cx: &mut TestAppContext) {
    let (editor, cx) = setup(
        "",
        |o| o.suggest_selection = SuggestSelection::RecentlyUsed,
        cx,
    );
    provide(cx, |_| {
        ["print", "println", "process"]
            .into_iter()
            .map(|label| CompletionItem::new(label, CompletionItemKind::Function))
            .collect()
    });
    cx.simulate_input("p");
    settle(cx);
    let selected = |cx: &mut VisualTestContext| {
        editor.read_with(cx, |e, _| {
            e.selected_suggestion()
                .map(|(_, item)| item.label.to_string())
        })
    };
    assert_eq!(selected(cx).as_deref(), Some("print"));
    cx.simulate_keystrokes("down down enter");
    assert_eq!(text(&editor, cx), "process");
    cx.simulate_input(" p");
    settle(cx);
    assert_eq!(selected(cx).as_deref(), Some("process"));
}

#[gpui::test]
fn quick_suggestions_rules(cx: &mut TestAppContext) {
    let (editor, cx) = setup("", |_| {}, cx);
    provide(cx, |_| {
        vec![CompletionItem::new("print", CompletionItemKind::Function)]
    });
    // Not before the delay.
    cx.simulate_input("p");
    cx.run_until_parked();
    assert!(!visible(&editor, cx));
    settle(cx);
    assert!(visible(&editor, cx));
    cx.simulate_keystrokes("escape");
    // Numbers don't trigger.
    cx.simulate_input(" 12");
    settle(cx);
    assert!(!visible(&editor, cx));

    // Off.
    editor.update(cx, |e, cx| {
        let mut options = e.options().clone();
        options.quick_suggestions = QuickSuggestions::OFF;
        e.update_options(options, cx);
    });
    cx.simulate_input(" pr");
    settle(cx);
    assert!(!visible(&editor, cx));
    // Ctrl+Space still works.
    cx.simulate_keystrokes("ctrl-space");
    cx.run_until_parked();
    assert!(visible(&editor, cx));
}

struct Dotted;
impl CompletionProvider for Dotted {
    fn trigger_characters(&self) -> Vec<char> {
        vec!['.']
    }

    fn provide_completions(
        &self,
        _: &ProviderContext,
        _: PointUtf16,
        _: CompletionTrigger,
        _: &mut App,
    ) -> Task<anyhow::Result<CompletionList>> {
        Task::ready(Ok(CompletionList {
            items: vec![CompletionItem::new("len", CompletionItemKind::Method)],
            incomplete: false,
        }))
    }
}

#[gpui::test]
fn trigger_characters_option(cx: &mut TestAppContext) {
    let (editor, cx) = setup(
        "",
        |o| {
            o.suggest_on_trigger_characters = false;
            o.quick_suggestions = QuickSuggestions::OFF;
        },
        cx,
    );
    cx.update(|_, cx| register_completion_provider(cx, LanguageSelector::All, Dotted));
    cx.simulate_input("a.");
    settle(cx);
    assert!(!visible(&editor, cx));
    editor.update(cx, |e, cx| {
        let mut options = e.options().clone();
        options.suggest_on_trigger_characters = true;
        e.update_options(options, cx);
    });
    cx.simulate_input("b.");
    settle(cx);
    assert_eq!(labels(&editor, cx), Some(vec!["len".to_string()]));
}

#[gpui::test]
fn word_based_suggestions(cx: &mut TestAppContext) {
    // No providers: words of the document, except the one being typed and
    // numbers.
    let (editor, cx) = setup("alpha beta 42 alphabet\n", |_| {}, cx);
    move_to(&editor, cx, 1, 0);
    cx.simulate_input("al");
    settle(cx);
    assert_eq!(
        labels(&editor, cx),
        Some(vec!["alpha".to_string(), "alphabet".to_string()])
    );
    cx.simulate_keystrokes("enter");
    assert_eq!(text(&editor, cx), "alpha beta 42 alphabet\nalpha");

    // Off.
    editor.update(cx, |e, cx| {
        let mut options = e.options().clone();
        options.word_based_suggestions = WordBasedSuggestions::Off;
        e.update_options(options, cx);
    });
    cx.simulate_input(" be");
    settle(cx);
    assert!(!visible(&editor, cx));
}

#[gpui::test]
fn word_based_suggestions_from_other_documents(cx: &mut TestAppContext) {
    let (editor, cx) = setup("", |_| {}, cx);
    // A registered model of the same language, and one of another.
    let (same, other) = cx.update(|_, cx| {
        let same = cx.new(|_| {
            TextModel::new("gamma")
                .with_language("demo")
                .with_uri("file:///b.demo")
        });
        let other = cx.new(|_| {
            TextModel::new("gazelle")
                .with_language("other")
                .with_uri("file:///c.other")
        });
        gpui_monaco::register_model(cx, &same);
        gpui_monaco::register_model(cx, &other);
        (same, other)
    });
    cx.simulate_input("ga");
    settle(cx);
    assert_eq!(labels(&editor, cx), Some(vec!["gamma".to_string()]));
    cx.simulate_keystrokes("escape");
    editor.update(cx, |e, cx| {
        let mut options = e.options().clone();
        options.word_based_suggestions = WordBasedSuggestions::AllDocuments;
        e.update_options(options, cx);
    });
    cx.simulate_keystrokes("ctrl-space");
    cx.run_until_parked();
    assert_eq!(
        labels(&editor, cx),
        Some(vec!["gamma".to_string(), "gazelle".to_string()])
    );
    drop((same, other));
}

#[gpui::test]
fn hidden_kinds_deprecated_and_snippets(cx: &mut TestAppContext) {
    let (editor, cx) = setup("", |_| {}, cx);
    provide(cx, |_| {
        vec![
            CompletionItem::new("pa", CompletionItemKind::Function),
            CompletionItem::new("pb", CompletionItemKind::Keyword),
            CompletionItem {
                tags: vec![CompletionItemTag::Deprecated],
                ..CompletionItem::new("pc", CompletionItemKind::Function)
            },
            CompletionItem::new("pd", CompletionItemKind::Snippet),
        ]
    });
    let show = |cx: &mut VisualTestContext| {
        cx.simulate_keystrokes("escape ctrl-space");
        cx.run_until_parked();
        labels(&editor, cx).unwrap_or_default()
    };
    assert_eq!(show(cx), vec!["pa", "pb", "pc", "pd"]);
    editor.update(cx, |e, cx| {
        let mut options = e.options().clone();
        options.suggest.hidden_kinds = vec![CompletionItemKind::Keyword];
        options.suggest.show_deprecated = false;
        options.snippet_suggestions = None;
        e.update_options(options, cx);
    });
    assert_eq!(show(cx), vec!["pa"]);
}

/// Resolves items with docs and an import.
struct Resolving {
    resolved: Rc<Cell<usize>>,
}
impl CompletionProvider for Resolving {
    fn provide_completions(
        &self,
        _: &ProviderContext,
        _: PointUtf16,
        _: CompletionTrigger,
        _: &mut App,
    ) -> Task<anyhow::Result<CompletionList>> {
        Task::ready(Ok(CompletionList {
            items: vec![
                CompletionItem::new("first", CompletionItemKind::Function),
                CompletionItem {
                    command: Some(Command {
                        id: "test.after".into(),
                        title: "After".into(),
                        arguments: vec![serde_json::json!(7)],
                    }),
                    ..CompletionItem::new("second", CompletionItemKind::Function)
                },
            ],
            incomplete: false,
        }))
    }

    fn resolve_completion_item(
        &self,
        item: &CompletionItem,
        _: &mut App,
    ) -> Option<Task<anyhow::Result<CompletionItem>>> {
        self.resolved.set(self.resolved.get() + 1);
        Some(Task::ready(Ok(CompletionItem {
            detail: Some(format!("resolved {}", item.label).into()),
            additional_text_edits: vec![(
                PointUtf16::new(0, 0)..PointUtf16::new(0, 0),
                "use x;\n".to_string(),
            )],
            ..item.clone()
        })))
    }
}

#[gpui::test]
fn resolves_the_focused_item_and_runs_commands(cx: &mut TestAppContext) {
    let (editor, cx) = setup("", |o| o.quick_suggestions = QuickSuggestions::OFF, cx);
    let resolved = Rc::new(Cell::new(0));
    let ran = Rc::new(Cell::new(None));
    cx.update(|_, cx| {
        register_completion_provider(
            cx,
            LanguageSelector::All,
            Resolving {
                resolved: resolved.clone(),
            },
        );
        let ran = ran.clone();
        register_command(cx, "test.after", move |args, _| {
            ran.set(args.first().and_then(|a| a.as_u64()))
        });
    });
    cx.simulate_keystrokes("ctrl-space");
    cx.run_until_parked();
    let detail = |cx: &mut VisualTestContext| {
        editor.read_with(cx, |e, _| {
            e.selected_suggestion()
                .and_then(|(_, item)| item.detail.map(|d| d.to_string()))
        })
    };
    // The selected item resolves; moving resolves the next one once.
    assert_eq!(detail(cx).as_deref(), Some("resolved first"));
    cx.simulate_keystrokes("down");
    cx.run_until_parked();
    assert_eq!(detail(cx).as_deref(), Some("resolved second"));
    cx.simulate_keystrokes("up down");
    cx.run_until_parked();
    assert_eq!(resolved.get(), 2);
    // Accepting applies the resolved import and runs the command.
    cx.simulate_keystrokes("enter");
    assert_eq!(text(&editor, cx), "use x;\nsecond");
    assert_eq!(ran.get(), Some(7));
}

/// Resolves after 100ms with an edit at `edit_at` on row 0.
struct SlowResolve {
    edit_at: u32,
}
impl CompletionProvider for SlowResolve {
    fn provide_completions(
        &self,
        _: &ProviderContext,
        _: PointUtf16,
        _: CompletionTrigger,
        _: &mut App,
    ) -> Task<anyhow::Result<CompletionList>> {
        Task::ready(Ok(CompletionList {
            items: vec![CompletionItem::new("item", CompletionItemKind::Function)],
            incomplete: false,
        }))
    }

    fn resolve_completion_item(
        &self,
        item: &CompletionItem,
        cx: &mut App,
    ) -> Option<Task<anyhow::Result<CompletionItem>>> {
        let item = item.clone();
        let at = PointUtf16::new(0, self.edit_at);
        let timer = cx.background_executor().timer(Duration::from_millis(100));
        Some(cx.background_executor().spawn(async move {
            timer.await;
            Ok(CompletionItem {
                additional_text_edits: vec![(at..at, "X".to_string())],
                ..item
            })
        }))
    }
}

#[gpui::test]
fn additional_edits_resolved_after_inserting(cx: &mut TestAppContext) {
    // Before the insertion: applied when they arrive.
    let (editor, cx) = setup("ab ", |o| o.quick_suggestions = QuickSuggestions::OFF, cx);
    cx.update(|_, cx| {
        register_completion_provider(cx, LanguageSelector::All, SlowResolve { edit_at: 0 })
    });
    move_to(&editor, cx, 0, 3);
    cx.simulate_keystrokes("ctrl-space");
    cx.run_until_parked();
    cx.simulate_keystrokes("enter");
    assert_eq!(text(&editor, cx), "ab item");
    cx.executor().advance_clock(Duration::from_millis(200));
    cx.run_until_parked();
    assert_eq!(text(&editor, cx), "Xab item");
}

#[gpui::test]
fn additional_edits_after_a_change_are_dropped(cx: &mut TestAppContext) {
    // After the insertion point: Monaco drops them.
    let (editor, cx) = setup(" end", |o| o.quick_suggestions = QuickSuggestions::OFF, cx);
    cx.update(|_, cx| {
        register_completion_provider(cx, LanguageSelector::All, SlowResolve { edit_at: 4 })
    });
    move_to(&editor, cx, 0, 0);
    cx.simulate_keystrokes("ctrl-space");
    cx.run_until_parked();
    cx.simulate_keystrokes("enter");
    cx.executor().advance_clock(Duration::from_millis(200));
    cx.run_until_parked();
    assert_eq!(text(&editor, cx), "item end");
}

fn functions(labels: &'static [&'static str]) -> impl Fn(PointUtf16) -> Vec<CompletionItem> {
    move |_| {
        labels
            .iter()
            .map(|label| CompletionItem::new(*label, CompletionItemKind::Function))
            .collect()
    }
}

#[gpui::test]
fn tab_completion_inserts_the_best_and_cycles(cx: &mut TestAppContext) {
    let (editor, cx) = setup(
        "",
        |o| {
            o.tab_completion = gpui_monaco::options::TabCompletion::On;
            o.quick_suggestions = QuickSuggestions::OFF;
        },
        cx,
    );
    provide(cx, functions(&["print", "println", "private"]));
    cx.simulate_input("pri");
    cx.simulate_keystrokes("tab");
    cx.run_until_parked();
    assert_eq!(text(&editor, cx), "print");
    assert!(!visible(&editor, cx), "no widget");
    // Tab and Shift+Tab cycle through the others.
    cx.simulate_keystrokes("tab");
    assert_eq!(text(&editor, cx), "println");
    cx.simulate_keystrokes("tab");
    assert_eq!(text(&editor, cx), "private");
    cx.simulate_keystrokes("shift-tab shift-tab");
    assert_eq!(text(&editor, cx), "print");
    // Moving the cursor ends cycling: Tab completes again, and with the word
    // already complete it types a tab.
    cx.simulate_keystrokes("left right tab");
    cx.run_until_parked();
    assert_eq!(text(&editor, cx), "print   ");
    // Undo restores what was typed.
    cx.simulate_keystrokes("cmd-z cmd-z");
    assert_eq!(text(&editor, cx), "pri");
}

#[gpui::test]
fn tab_completion_off_by_default(cx: &mut TestAppContext) {
    let (editor, cx) = setup("", |o| o.quick_suggestions = QuickSuggestions::OFF, cx);
    provide(cx, functions(&["print"]));
    cx.simulate_input("pri");
    cx.simulate_keystrokes("tab");
    cx.run_until_parked();
    assert_eq!(text(&editor, cx), "pri ");
}

#[gpui::test]
fn locality_bonus_prefers_words_in_the_block(cx: &mut TestAppContext) {
    let source = "fn a() {\n    alpha\n}\nfn b() {\n    beta;\n    \n}\n";
    let (editor, cx) = setup(source, |o| o.quick_suggestions = QuickSuggestions::OFF, cx);
    provide(cx, |_| {
        ["alpha", "beta"]
            .into_iter()
            .map(|label| CompletionItem::new(label, CompletionItemKind::Variable))
            .collect()
    });
    move_to(&editor, cx, 5, 4);
    cx.simulate_keystrokes("ctrl-space");
    cx.run_until_parked();
    assert_eq!(labels(&editor, cx).unwrap(), vec!["alpha", "beta"]);
    cx.simulate_keystrokes("escape");
    editor.update(cx, |e, cx| {
        let mut options = e.options().clone();
        options.suggest.locality_bonus = true;
        e.update_options(options, cx);
    });
    cx.simulate_keystrokes("ctrl-space");
    cx.run_until_parked();
    assert_eq!(labels(&editor, cx).unwrap(), vec!["beta", "alpha"]);
}

#[gpui::test]
fn explicit_sessions_freeze_and_automatic_ones_close(cx: &mut TestAppContext) {
    let (editor, cx) = setup("", |_| {}, cx);
    provide(cx, functions(&["print", "process"]));
    // Typing something that matches nothing closes an automatic session...
    cx.simulate_input("pr");
    settle(cx);
    assert!(visible(&editor, cx));
    cx.simulate_input("z");
    assert!(!visible(&editor, cx));
    // ...but keeps the last matches of one requested with Ctrl+Space.
    cx.simulate_input(" pr");
    settle(cx);
    cx.simulate_keystrokes("escape ctrl-space");
    cx.run_until_parked();
    cx.simulate_input("z");
    assert_eq!(labels(&editor, cx).unwrap(), vec!["print", "process"]);
    // Leaving the word closes it.
    cx.simulate_input(" ");
    assert!(!visible(&editor, cx));
}

#[gpui::test]
fn deleting_before_the_request_asks_again(cx: &mut TestAppContext) {
    let calls = Rc::new(Cell::new(0));
    let (editor, cx) = setup("", |o| o.quick_suggestions = QuickSuggestions::OFF, cx);
    let counter = calls.clone();
    provide(cx, move |_| {
        counter.set(counter.get() + 1);
        vec![CompletionItem::new("print", CompletionItemKind::Function)]
    });
    cx.simulate_input("pri");
    cx.simulate_keystrokes("ctrl-space");
    cx.run_until_parked();
    assert_eq!(calls.get(), 1);
    // Still on a word: asks again.
    cx.simulate_keystrokes("backspace");
    cx.run_until_parked();
    assert_eq!(calls.get(), 2);
    assert!(visible(&editor, cx));
    // Off the word: closes.
    cx.simulate_keystrokes("backspace backspace");
    cx.run_until_parked();
    assert!(!visible(&editor, cx));
}

fn message(editor: &Entity<CodeEditor>, cx: &mut VisualTestContext) -> Option<&'static str> {
    editor.read_with(cx, |e, cx| e.suggest_message_for_tests(cx))
}

#[gpui::test]
fn no_suggestions_only_when_requested(cx: &mut TestAppContext) {
    let (editor, cx) = setup(
        "",
        |o| o.word_based_suggestions = WordBasedSuggestions::Off,
        cx,
    );
    provide(cx, |_| Vec::new());
    // Typing: nothing shows.
    cx.simulate_input("ab");
    settle(cx);
    assert!(!visible(&editor, cx));
    // Ctrl+Space: "No suggestions.", until the cursor leaves.
    cx.simulate_keystrokes("ctrl-space");
    cx.run_until_parked();
    assert_eq!(message(&editor, cx), Some("No suggestions."));
    cx.simulate_keystrokes("escape");
    assert!(!visible(&editor, cx));
}

/// Answers after 200ms.
struct Slow;
impl CompletionProvider for Slow {
    fn provide_completions(
        &self,
        _: &ProviderContext,
        _: PointUtf16,
        _: CompletionTrigger,
        cx: &mut App,
    ) -> Task<anyhow::Result<CompletionList>> {
        let timer = cx.background_executor().timer(Duration::from_millis(200));
        cx.background_executor().spawn(async move {
            timer.await;
            Ok(CompletionList {
                items: vec![CompletionItem::new("print", CompletionItemKind::Function)],
                incomplete: false,
            })
        })
    }
}

#[gpui::test]
fn loading_message_for_slow_explicit_requests(cx: &mut TestAppContext) {
    let (editor, cx) = setup("", |_| {}, cx);
    cx.update(|_, cx| register_completion_provider(cx, LanguageSelector::All, Slow));
    // Automatic requests never show it.
    cx.simulate_input("p");
    settle(cx);
    assert!(!visible(&editor, cx));
    cx.executor().advance_clock(Duration::from_millis(300));
    cx.run_until_parked();
    assert_eq!(labels(&editor, cx), Some(vec!["print".to_string()]));
    cx.simulate_keystrokes("escape ctrl-space");
    cx.run_until_parked();
    assert!(!visible(&editor, cx), "not before 50ms");
    cx.executor().advance_clock(Duration::from_millis(60));
    cx.run_until_parked();
    assert_eq!(message(&editor, cx), Some("Loading..."));
    cx.executor().advance_clock(Duration::from_millis(300));
    cx.run_until_parked();
    assert_eq!(message(&editor, cx), None);
    assert_eq!(labels(&editor, cx), Some(vec!["print".to_string()]));
}

#[gpui::test]
fn ctrl_space_toggles_details(cx: &mut TestAppContext) {
    let (editor, cx) = setup("", |o| o.quick_suggestions = QuickSuggestions::OFF, cx);
    provide(cx, |_| {
        vec![
            CompletionItem {
                documentation: Some("Prints.".into()),
                ..CompletionItem::new("print", CompletionItemKind::Function)
            },
            CompletionItem::new("process", CompletionItemKind::Function),
        ]
    });
    let details = |cx: &mut VisualTestContext| {
        editor.read_with(cx, |e, cx| e.is_suggestion_details_visible(cx))
    };
    cx.simulate_keystrokes("ctrl-space");
    cx.run_until_parked();
    // Collapsed by default, as in Monaco.
    assert!(!details(cx));
    cx.simulate_keystrokes("ctrl-space");
    assert!(details(cx));
    // Nothing to show for an item without docs or detail.
    cx.simulate_keystrokes("down");
    assert!(!details(cx));
    cx.simulate_keystrokes("up");
    assert!(details(cx));
    // Remembered for the next session.
    cx.simulate_keystrokes("escape ctrl-space");
    cx.run_until_parked();
    assert!(details(cx));
    cx.simulate_keystrokes("ctrl-space");
    assert!(!details(cx));
    assert!(visible(&editor, cx));
}
