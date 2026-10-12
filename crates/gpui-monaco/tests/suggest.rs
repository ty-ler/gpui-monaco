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
                        ..Default::default()
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
        let _ = register_command(cx, "test.after", move |args, _| {
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

/// A snippet suggestion wrapping `$SELECTION`.
fn provide_wrapping_snippet(cx: &mut VisualTestContext) {
    provide(cx, |_| {
        vec![CompletionItem {
            insert_text: Some("log($SELECTION)".into()),
            insert_text_is_snippet: true,
            ..CompletionItem::new("log", CompletionItemKind::Snippet)
        }]
    });
}

/// Monaco's `OvertypingCapturer`: typing over a selection starts a
/// suggestion, and `$SELECTION` in the one inserted is the text typed over.
#[gpui::test]
fn selection_variable_gets_the_overtyped_text(cx: &mut TestAppContext) {
    let (editor, cx) = setup("foo bar\nbaz", |_| {}, cx);
    provide_wrapping_snippet(cx);
    editor.update(cx, |e, cx| {
        e.set_selections(
            &[gpui_monaco::cursor::Selection::from_range(
                PointUtf16::new(0, 0)..PointUtf16::new(0, 3),
                false,
            )],
            cx,
        )
    });
    cx.simulate_input("l");
    settle(cx);
    // Typing on filters the session; it still has `foo`.
    cx.simulate_input("o");
    settle(cx);
    assert_eq!(labels(&editor, cx).unwrap(), vec!["log"]);
    cx.simulate_keystrokes("enter");
    cx.run_until_parked();
    assert_eq!(text(&editor, cx), "log(foo) bar\nbaz");

    // Multi-line text keeps its lines; typing without a selection forgets
    // it.
    editor.update(cx, |e, cx| {
        e.set_selections(
            &[gpui_monaco::cursor::Selection::from_range(
                PointUtf16::new(0, 9)..PointUtf16::new(1, 3),
                false,
            )],
            cx,
        )
    });
    cx.simulate_input("l");
    settle(cx);
    cx.simulate_keystrokes("enter");
    cx.run_until_parked();
    assert_eq!(text(&editor, cx), "log(foo) log(bar\nbaz)");
    cx.simulate_input(" l");
    settle(cx);
    cx.simulate_keystrokes("enter");
    cx.run_until_parked();
    assert_eq!(text(&editor, cx), "log(foo) log(bar\nbaz) log()");
}

/// Monaco feeds `WordDistance` to word-based suggestions as to any other:
/// with `localityBonus` the words used in the cursor's block come first.
#[gpui::test]
fn locality_bonus_ranks_word_based_suggestions(cx: &mut TestAppContext) {
    let source = "fn a() {\n    alpha\n}\nfn b() {\n    beta;\n    \n}\n";
    let (editor, cx) = setup(source, |o| o.quick_suggestions = QuickSuggestions::OFF, cx);
    move_to(&editor, cx, 5, 4);
    cx.simulate_keystrokes("ctrl-space");
    cx.run_until_parked();
    // Sorted by label.
    assert_eq!(
        labels(&editor, cx).unwrap(),
        vec!["a", "alpha", "b", "beta", "fn"]
    );
    cx.simulate_keystrokes("escape");
    editor.update(cx, |e, cx| {
        let mut options = e.options().clone();
        options.suggest.locality_bonus = true;
        e.update_options(options, cx);
    });
    cx.simulate_keystrokes("ctrl-space");
    cx.run_until_parked();
    // `beta` is in the block; `b` and `fn` on its first line, outside its
    // braces; `a` and `alpha` elsewhere.
    assert_eq!(
        labels(&editor, cx).unwrap(),
        vec!["beta", "b", "fn", "a", "alpha"]
    );
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

#[gpui::test]
fn monaco_secondary_keys_move_in_the_list(cx: &mut TestAppContext) {
    let (editor, cx) = setup("", |_| {}, cx);
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
    // Monaco `selectNextSuggestion` / `selectPrevSuggestion` secondaries.
    cx.simulate_keystrokes("secondary-down");
    assert_eq!(selected(cx).as_deref(), Some("println"));
    cx.simulate_keystrokes("secondary-up");
    assert_eq!(selected(cx).as_deref(), Some("print"));
    cx.simulate_keystrokes("secondary-pagedown");
    assert_eq!(selected(cx).as_deref(), Some("process"));
    // The list stays open; the cursor didn't move.
    assert!(visible(&editor, cx));
    assert_eq!(text(&editor, cx), "p");
}

#[gpui::test]
fn word_based_suggestions_off_with_inline_suggestions(cx: &mut TestAppContext) {
    struct Ghost;
    impl InlineCompletionsProvider for Ghost {
        fn provide_inline_completions(
            &self,
            _: &ProviderContext,
            _: PointUtf16,
            _: InlineCompletionContext,
            _: &mut App,
        ) -> Task<anyhow::Result<InlineCompletionList>> {
            Task::ready(Ok(InlineCompletionList {
                items: Vec::new(),
                suppress_suggestions: None,
                commands: Vec::new(),
                ..Default::default()
            }))
        }
    }
    // Monaco's default: words come from the document...
    let (editor, cx) = setup("alpha\n", |_| {}, cx);
    cx.simulate_keystrokes("down");
    cx.simulate_input("al");
    cx.simulate_keystrokes("ctrl-space");
    settle(cx);
    assert!(visible(&editor, cx));
    cx.simulate_keystrokes("escape");
    // ...until an inline completions provider serves the language.
    cx.update(|_, cx| {
        register_inline_completions_provider(cx, LanguageSelector::All, Ghost);
    });
    cx.simulate_keystrokes("ctrl-space");
    settle(cx);
    assert!(!visible(&editor, cx));
}

fn focused(editor: &Entity<CodeEditor>, cx: &mut VisualTestContext) -> Option<String> {
    editor.read_with(cx, |e, _| {
        e.selected_suggestion()
            .map(|(_, item)| item.label.to_string())
    })
}

fn set_selection_mode(
    editor: &Entity<CodeEditor>,
    cx: &mut VisualTestContext,
    mode: gpui_monaco::options::SuggestSelectionMode,
) {
    editor.update(cx, |e, cx| {
        let mut options = e.options().clone();
        options.suggest.selection_mode = mode;
        e.update_options(options, cx);
    });
}

#[gpui::test]
fn selection_mode_never_shows_without_focus(cx: &mut TestAppContext) {
    use gpui_monaco::options::SuggestSelectionMode;
    let (editor, cx) = setup(
        "",
        |o| o.suggest.selection_mode = SuggestSelectionMode::Never,
        cx,
    );
    provide(cx, |_| {
        ["print", "process"]
            .into_iter()
            .map(|label| CompletionItem::new(label, CompletionItemKind::Function))
            .collect()
    });
    cx.simulate_input("p");
    settle(cx);
    assert!(visible(&editor, cx));
    assert_eq!(focused(&editor, cx), None);
    assert!(editor.read_with(cx, |e, _| e.is_suggestion_unfocused()));
    // Enter isn't taken by the widget: it types a line break.
    cx.simulate_keystrokes("enter");
    assert_eq!(text(&editor, cx), "p\n");
    assert!(!visible(&editor, cx));

    // Typing more keeps it unfocused; Down focuses the first item.
    cx.simulate_input("p");
    settle(cx);
    cx.simulate_input("r");
    settle(cx);
    assert_eq!(focused(&editor, cx), None);
    cx.simulate_keystrokes("down");
    assert_eq!(focused(&editor, cx).as_deref(), Some("print"));
    cx.simulate_keystrokes("tab");
    assert_eq!(text(&editor, cx), "p\nprint");

    // Up from no focus goes to the last item.
    cx.simulate_input(" p");
    settle(cx);
    cx.simulate_keystrokes("up");
    assert_eq!(focused(&editor, cx).as_deref(), Some("process"));
    cx.simulate_keystrokes("escape");

    // Ctrl+Space focuses (Monaco `focusSuggestion`), then toggles details.
    cx.simulate_input(" p");
    settle(cx);
    assert_eq!(focused(&editor, cx), None);
    cx.simulate_keystrokes("ctrl-space");
    assert_eq!(focused(&editor, cx).as_deref(), Some("print"));
    cx.simulate_keystrokes("escape");

    // `focusAndAcceptSuggestion` by id.
    cx.simulate_input(" p");
    settle(cx);
    cx.update(|window, cx| {
        editor.update(cx, |e, cx| {
            e.trigger("focusAndAcceptSuggestion", window, cx)
        })
    })
    .unwrap();
    cx.run_until_parked();
    assert_eq!(text(&editor, cx), "p\nprint p p print");

    // Explicit requests always focus.
    cx.simulate_keystrokes("ctrl-space");
    cx.run_until_parked();
    assert_eq!(focused(&editor, cx).as_deref(), Some("print"));
}

#[gpui::test]
fn selection_modes_for_trigger_characters(cx: &mut TestAppContext) {
    use gpui_monaco::options::SuggestSelectionMode;
    let (editor, cx) = setup("", |_| {}, cx);
    cx.update(|_, cx| register_completion_provider(cx, LanguageSelector::All, Dotted));

    // `WhenTriggerCharacter`: focused after `.`, not after typing a word.
    set_selection_mode(&editor, cx, SuggestSelectionMode::WhenTriggerCharacter);
    cx.simulate_input("a.");
    settle(cx);
    assert_eq!(focused(&editor, cx).as_deref(), Some("len"));
    cx.simulate_keystrokes("escape");
    cx.simulate_input(" l");
    settle(cx);
    assert!(visible(&editor, cx));
    assert_eq!(focused(&editor, cx), None);
    cx.simulate_keystrokes("escape");

    // `WhenQuickSuggestion`: the other way round, until more is typed after
    // the trigger character.
    set_selection_mode(&editor, cx, SuggestSelectionMode::WhenQuickSuggestion);
    cx.simulate_input(" b.");
    settle(cx);
    assert!(visible(&editor, cx));
    assert_eq!(focused(&editor, cx), None);
    cx.simulate_input("l");
    settle(cx);
    assert_eq!(focused(&editor, cx).as_deref(), Some("len"));
    cx.simulate_keystrokes("escape");
    cx.simulate_input(" l");
    settle(cx);
    assert_eq!(focused(&editor, cx).as_deref(), Some("len"));
}

#[gpui::test]
fn snippets_prevent_quick_suggestions(cx: &mut TestAppContext) {
    let (editor, cx) = setup("", |_| {}, cx);
    provide(cx, |_| {
        vec![CompletionItem::new("print", CompletionItemKind::Function)]
    });
    let snippet = |editor: &Entity<CodeEditor>, cx: &mut VisualTestContext| {
        editor.update(cx, |e, cx| {
            e.insert_snippet("${1:a} ${2:b}", Default::default(), cx)
        });
    };
    // Off by default: quick suggestions open inside a snippet.
    snippet(&editor, cx);
    cx.simulate_input("p");
    settle(cx);
    assert!(visible(&editor, cx));
    cx.simulate_keystrokes("escape escape");

    editor.update(cx, |e, cx| {
        e.model().update(cx, |m, cx| m.set_value("", cx));
        let mut options = e.options().clone();
        options.suggest.snippets_prevent_quick_suggestions = true;
        e.update_options(options, cx);
    });
    snippet(&editor, cx);
    assert!(editor.read_with(cx, |e, _| e.in_snippet_mode()));
    cx.simulate_input("p");
    settle(cx);
    assert!(!visible(&editor, cx));
    // Ctrl+Space still works.
    cx.simulate_keystrokes("ctrl-space");
    cx.run_until_parked();
    assert!(visible(&editor, cx));
}

struct Named;
impl CompletionProvider for Named {
    fn provide_completions(
        &self,
        _: &ProviderContext,
        _: PointUtf16,
        _: CompletionTrigger,
        _: &mut App,
    ) -> Task<anyhow::Result<CompletionList>> {
        Task::ready(Ok(CompletionList {
            items: vec![CompletionItem {
                sort_text: Some("a".into()),
                commit_characters: vec!['(', '.'],
                ..CompletionItem::new("print", CompletionItemKind::Function)
            }],
            incomplete: false,
        }))
    }

    fn debug_display_name(&self) -> gpui::SharedString {
        "named".into()
    }
}

#[gpui::test]
fn explain_mode_shows_the_scoring(cx: &mut TestAppContext) {
    let (editor, cx) = setup("", |o| o.quick_suggestions = QuickSuggestions::OFF, cx);
    cx.update(|_, cx| register_completion_provider(cx, LanguageSelector::All, Named));
    cx.simulate_input("pr");
    cx.simulate_keystrokes("ctrl-space");
    cx.run_until_parked();
    let details =
        |cx: &mut VisualTestContext| editor.read_with(cx, |e, cx| e.suggest_details_for_tests(cx));
    // Nothing to expand without docs or detail.
    assert_eq!(details(cx), None);
    // Monaco `toggleExplainMode` (Cmd/Ctrl+/) opens the details with it.
    cx.simulate_keystrokes("secondary-/");
    let (detail, docs) = details(cx).expect("details show");
    assert_eq!(detail.as_deref(), Some("Provider: named"));
    let docs = docs.unwrap();
    assert!(docs.starts_with("```empty\nscore: "), "{docs}");
    assert!(docs.contains("\nprefix: pr\n"), "{docs}");
    assert!(docs.contains("\nword: print\n"), "{docs}");
    assert!(
        docs.contains("\ndistance: 0 (localityBonus-setting)\n"),
        "{docs}"
    );
    assert!(
        docs.contains("\nindex: 0, based on sortText: \"a\"\n"),
        "{docs}"
    );
    assert!(docs.contains("\ncommit_chars: (.\n"), "{docs}");
    // The comment toggle didn't run.
    assert_eq!(text(&editor, cx), "pr");
    cx.simulate_keystrokes("secondary-/");
    assert!(!editor.read_with(cx, |e, _| e.is_suggest_explain_mode()));
    // Off again once the widget closes.
    cx.simulate_keystrokes("secondary-/");
    assert!(editor.read_with(cx, |e, _| e.is_suggest_explain_mode()));
    cx.simulate_keystrokes("escape");
    assert!(!editor.read_with(cx, |e, _| e.is_suggest_explain_mode()));
}

#[gpui::test]
fn reset_suggest_size(cx: &mut TestAppContext) {
    let (editor, cx) = setup("", |_| {}, cx);
    cx.update(|_, cx| {
        CodeEditor::set_suggest_size_for_tests(gpui::size(gpui::px(300.), gpui::px(200.)), cx)
    });
    assert!(
        cx.update(|_, cx| CodeEditor::suggest_size_for_tests(cx))
            .is_some()
    );
    cx.update(|window, cx| {
        editor.update(cx, |e, cx| {
            e.run_action("editor.action.resetSuggestSize", window, cx)
        })
    });
    cx.run_until_parked();
    assert_eq!(
        cx.update(|_, cx| CodeEditor::suggest_size_for_tests(cx)),
        None
    );
}

#[gpui::test]
fn monaco_completion_kinds(cx: &mut TestAppContext) {
    let (editor, cx) = setup("", |o| o.quick_suggestions = QuickSuggestions::OFF, cx);
    provide(cx, |_| {
        [
            ("red", CompletionItemKind::Customcolor),
            ("rob", CompletionItemKind::User),
            ("r12", CompletionItemKind::Issue),
            ("run", CompletionItemKind::Tool),
        ]
        .into_iter()
        .map(|(label, kind)| CompletionItem::new(label, kind))
        .collect()
    });
    cx.simulate_keystrokes("ctrl-space");
    cx.run_until_parked();
    assert_eq!(labels(&editor, cx).map(|l| l.len()), Some(4));
    // Hidden like the other kinds (Monaco `showUsers: false` and the like).
    editor.update(cx, |e, cx| {
        let mut options = e.options().clone();
        options.suggest.hidden_kinds = vec![CompletionItemKind::User, CompletionItemKind::Issue];
        e.update_options(options, cx);
    });
    cx.simulate_keystrokes("escape ctrl-space");
    cx.run_until_parked();
    assert_eq!(
        labels(&editor, cx),
        Some(vec!["red".to_string(), "run".to_string()])
    );
}

#[gpui::test]
fn toggle_suggestion_focus_scrolls_and_copies_the_details(cx: &mut TestAppContext) {
    let (editor, cx) = setup("", |o| o.quick_suggestions = QuickSuggestions::OFF, cx);
    // Longer than the panel can be, so it scrolls.
    let docs = format!("Prints.{}", "\n\nMore.".repeat(100));
    let item_docs = docs.clone();
    provide(cx, move |_| {
        vec![
            CompletionItem {
                detail: Some("fn print()".into()),
                documentation: Some(item_docs.clone().into()),
                ..CompletionItem::new("print", CompletionItemKind::Function)
            },
            CompletionItem::new("process", CompletionItemKind::Function),
        ]
    });
    let details_focused =
        |cx: &mut VisualTestContext| editor.read_with(cx, |e, _| e.is_suggestion_details_focused());
    let scroll = |cx: &mut VisualTestContext| {
        editor.read_with(cx, |e, cx| e.suggest_details_scroll_for_tests(cx))
    };
    cx.simulate_keystrokes("ctrl-space");
    cx.run_until_parked();
    assert!(!editor.read_with(cx, |e, cx| e.is_suggestion_details_visible(cx)));
    // Monaco `toggleSuggestionFocus` opens the details and focuses them.
    cx.simulate_keystrokes("ctrl-alt-space");
    assert!(details_focused(cx));
    assert!(editor.read_with(cx, |e, cx| e.is_suggestion_details_visible(cx)));
    // The keys scroll the details (Monaco `scrollDown`, `pageDown`)
    // instead of moving in the list.
    cx.simulate_keystrokes("down");
    assert_eq!(scroll(cx), Some(gpui::px(8.)));
    // A page (Monaco's 80px), within the docs.
    cx.simulate_keystrokes("pagedown");
    let paged = scroll(cx).unwrap();
    assert!(paged > gpui::px(8.) && paged <= gpui::px(88.), "{paged:?}");
    cx.simulate_keystrokes("up pageup");
    assert_eq!(scroll(cx), Some(gpui::px(0.)));
    assert_eq!(focused(&editor, cx).as_deref(), Some("print"));
    // Monaco `suggestWidgetCopy`.
    cx.simulate_keystrokes("secondary-c");
    assert_eq!(
        cx.update(|_, cx| cx.read_from_clipboard())
            .and_then(|item| item.text())
            .as_deref(),
        Some(format!("fn print()\n\n{docs}").as_str())
    );
    assert_eq!(text(&editor, cx), "");
    // Back to the list.
    cx.simulate_keystrokes("ctrl-alt-space");
    assert!(!details_focused(cx));
    cx.simulate_keystrokes("down");
    assert_eq!(focused(&editor, cx).as_deref(), Some("process"));
    // Nothing to focus for an item without details.
    cx.simulate_keystrokes("ctrl-alt-space");
    assert!(!details_focused(cx));
    // Typing goes back to the list; hiding resets it.
    cx.simulate_keystrokes("up ctrl-alt-space");
    assert!(details_focused(cx));
    cx.simulate_input("p");
    assert!(!details_focused(cx));
    cx.simulate_keystrokes("ctrl-alt-space escape");
    assert!(!details_focused(cx));
    assert!(!visible(&editor, cx));
}

/// `hello` and `world` for the word at the cursor.
fn provide_hello_world(cx: &mut VisualTestContext) {
    provide(cx, |_| {
        ["hello", "world"]
            .into_iter()
            .map(|label| CompletionItem::new(label, CompletionItemKind::Function))
            .collect()
    });
}

fn ghost(editor: &Entity<CodeEditor>, cx: &mut VisualTestContext) -> Option<String> {
    editor.read_with(cx, |e, cx| {
        e.ghost_text_for_tests(cx)
            .map(|(_, parts)| parts.into_iter().map(|(_, text)| text).collect())
    })
}

/// Waits out the quick suggestions and inline completions delays.
fn settle_inline(cx: &mut VisualTestContext) {
    cx.executor().advance_clock(Duration::from_millis(200));
    cx.run_until_parked();
}

#[gpui::test]
fn quick_suggestions_inline_show_as_ghost_text(cx: &mut TestAppContext) {
    let (editor, cx) = setup("", |o| o.quick_suggestions = QuickSuggestions::INLINE, cx);
    provide_hello_world(cx);
    cx.simulate_input("he");
    settle_inline(cx);
    // No widget: the matching item shows as ghost text.
    assert!(!visible(&editor, cx));
    assert_eq!(ghost(&editor, cx).as_deref(), Some("llo"));
    // Typing more refilters it.
    cx.simulate_input("l");
    settle_inline(cx);
    assert_eq!(ghost(&editor, cx).as_deref(), Some("lo"));
    cx.simulate_keystrokes("tab");
    assert_eq!(text(&editor, cx), "hello");
    // Ctrl+Space still opens the widget.
    cx.simulate_input(" w");
    settle_inline(cx);
    assert_eq!(ghost(&editor, cx).as_deref(), Some("orld"));
    cx.simulate_keystrokes("ctrl-space");
    cx.run_until_parked();
    assert!(visible(&editor, cx));
}

#[gpui::test]
fn quick_suggestions_inline_only_where_set(cx: &mut TestAppContext) {
    use gpui_monaco::options::QuickSuggestionsValue;
    // `Inline` in comments and strings only: code gets neither.
    let (editor, cx) = setup(
        "",
        |o| {
            o.quick_suggestions = QuickSuggestions {
                other: QuickSuggestionsValue::Off,
                ..QuickSuggestions::INLINE
            }
        },
        cx,
    );
    provide_hello_world(cx);
    cx.simulate_input("he");
    settle_inline(cx);
    assert!(!visible(&editor, cx));
    assert_eq!(ghost(&editor, cx), None);
}

/// An inline completions provider offering `text` for a line it starts
/// with, after `delay`.
struct InlineText {
    text: Option<&'static str>,
    delay: Duration,
}

impl InlineCompletionsProvider for InlineText {
    fn provide_inline_completions(
        &self,
        context: &ProviderContext,
        position: PointUtf16,
        _: InlineCompletionContext,
        cx: &mut App,
    ) -> Task<anyhow::Result<InlineCompletionList>> {
        let line = context.snapshot.line(position.row);
        let items = self
            .text
            .filter(|text| text.starts_with(&line))
            .map(|text| InlineCompletion {
                insert_text: text.to_string(),
                range: Some(
                    PointUtf16::new(position.row, 0)
                        ..PointUtf16::new(position.row, line.len() as u32),
                ),
                ..Default::default()
            })
            .into_iter()
            .collect();
        let delay = self.delay;
        cx.spawn(async move |cx| {
            cx.background_executor().timer(delay).await;
            Ok(InlineCompletionList {
                items,
                ..Default::default()
            })
        })
    }
}

fn provide_inline(cx: &mut VisualTestContext, text: Option<&'static str>, delay: Duration) {
    cx.update(|_, cx| {
        register_inline_completions_provider(cx, LanguageSelector::All, InlineText { text, delay })
    });
}

#[gpui::test]
fn off_when_inline_completions_keeps_the_widget_closed_for_ghost_text(cx: &mut TestAppContext) {
    // Monaco's default, `offWhenInlineCompletions` in code.
    let (editor, cx) = setup("", |_| {}, cx);
    assert_eq!(
        editor.read_with(cx, |e, _| e.options().quick_suggestions),
        QuickSuggestions::default()
    );
    provide_hello_world(cx);
    provide_inline(cx, Some("hello there"), Duration::ZERO);
    cx.simulate_input("he");
    settle_inline(cx);
    assert!(!visible(&editor, cx));
    assert_eq!(ghost(&editor, cx).as_deref(), Some("llo there"));

    // `On` opens the widget anyway (which hides the ghost text).
    editor.update(cx, |e, cx| {
        let mut options = e.options().clone();
        options.quick_suggestions = QuickSuggestions::ON;
        e.update_options(options, cx);
    });
    cx.simulate_input("l");
    settle_inline(cx);
    assert!(visible(&editor, cx));
}

#[gpui::test]
fn off_when_inline_completions_opens_without_ghost_text(cx: &mut TestAppContext) {
    let (editor, cx) = setup("", |_| {}, cx);
    provide_hello_world(cx);
    provide_inline(cx, None, Duration::ZERO);
    cx.simulate_input("he");
    settle_inline(cx);
    assert!(visible(&editor, cx));
    assert_eq!(labels(&editor, cx), Some(vec!["hello".to_string()]));
}

#[gpui::test]
fn off_when_inline_completions_stops_waiting_after_750ms(cx: &mut TestAppContext) {
    let (editor, cx) = setup("", |_| {}, cx);
    provide_hello_world(cx);
    provide_inline(cx, Some("hello there"), Duration::from_secs(5));
    cx.simulate_input("he");
    settle_inline(cx);
    // Still waiting for inline completions.
    assert!(!visible(&editor, cx));
    cx.executor().advance_clock(Duration::from_millis(600));
    cx.run_until_parked();
    // Monaco opens the widget and stops the inline completions.
    assert!(visible(&editor, cx));
    cx.executor().advance_clock(Duration::from_secs(5));
    cx.run_until_parked();
    assert_eq!(ghost(&editor, cx), None);
}

/// Counts its requests and answers `items` (default ranges), declaring
/// `triggers`.
struct Counted {
    count: Rc<Cell<usize>>,
    items: &'static [&'static str],
    triggers: Vec<char>,
    incomplete: bool,
}

impl CompletionProvider for Counted {
    fn trigger_characters(&self) -> Vec<char> {
        self.triggers.clone()
    }

    fn provide_completions(
        &self,
        _: &ProviderContext,
        _: PointUtf16,
        _: CompletionTrigger,
        _: &mut App,
    ) -> Task<anyhow::Result<CompletionList>> {
        self.count.set(self.count.get() + 1);
        Task::ready(Ok(CompletionList {
            items: self
                .items
                .iter()
                .map(|label| CompletionItem::new(*label, CompletionItemKind::Property))
                .collect(),
            incomplete: self.incomplete,
        }))
    }
}

/// Registers a [`Counted`] provider, returning its request count.
fn counted(
    cx: &mut VisualTestContext,
    selector: LanguageSelector,
    items: &'static [&'static str],
    triggers: &[char],
    incomplete: bool,
) -> Rc<Cell<usize>> {
    let count = Rc::new(Cell::new(0));
    let provider = Counted {
        count: count.clone(),
        items,
        triggers: triggers.to_vec(),
        incomplete,
    };
    cx.update(|_, cx| register_completion_provider(cx, selector, provider));
    count
}

fn sorted_labels(editor: &Entity<CodeEditor>, cx: &mut VisualTestContext) -> Vec<String> {
    let mut labels = labels(editor, cx).unwrap_or_default();
    labels.sort();
    labels
}

/// Monaco `provideSuggestionItems`: providers are asked in groups of equal
/// score, and a group with items stops the lower ones (a language's own
/// providers over a `*` one).
#[gpui::test]
fn provider_groups_stop_at_the_first_with_items(cx: &mut TestAppContext) {
    let (editor, cx) = setup(
        "",
        |o| o.word_based_suggestions = WordBasedSuggestions::Off,
        cx,
    );
    let star = counted(cx, LanguageSelector::All, &["star"], &[], false);
    let own = counted(cx, LanguageSelector::language("demo"), &["own"], &[], false);
    let other = counted(
        cx,
        LanguageSelector::language("demo"),
        &["other"],
        &[],
        false,
    );
    cx.simulate_keystrokes("ctrl-space");
    cx.run_until_parked();
    assert_eq!(sorted_labels(&editor, cx), ["other", "own"]);
    assert_eq!((own.get(), other.get(), star.get()), (1, 1, 0));
}

/// The `*` group answers when the language's providers have no items.
#[gpui::test]
fn lower_provider_groups_answer_when_higher_ones_are_empty(cx: &mut TestAppContext) {
    let (editor, cx) = setup(
        "",
        |o| o.word_based_suggestions = WordBasedSuggestions::Off,
        cx,
    );
    let star = counted(cx, LanguageSelector::All, &["star"], &[], false);
    let own = counted(cx, LanguageSelector::language("demo"), &[], &[], false);
    cx.simulate_keystrokes("ctrl-space");
    cx.run_until_parked();
    assert_eq!(sorted_labels(&editor, cx), ["star"]);
    assert_eq!((own.get(), star.get()), (1, 1));
}

/// Monaco `suggestModel.test.ts` "registerCompletionItemProvider with
/// letters as trigger characters block other completion items to show up
/// #127815": a trigger character asks only the providers declaring it,
/// unless it's a word character starting quick suggestions.
#[gpui::test]
fn trigger_characters_ask_only_their_providers(cx: &mut TestAppContext) {
    let (editor, cx) = setup(
        "",
        |o| o.word_based_suggestions = WordBasedSuggestions::Off,
        cx,
    );
    let word = counted(cx, LanguageSelector::All, &["AAAA"], &[], false);
    let triggered = counted(cx, LanguageSelector::All, &["AAAA"], &['a', '.'], false);
    cx.simulate_input(".");
    settle(cx);
    assert_eq!(labels(&editor, cx).map(|l| l.len()), Some(1));
    assert_eq!((word.get(), triggered.get()), (0, 1));
    cx.simulate_keystrokes("escape");
    editor.update(cx, |e, cx| {
        e.model().update(cx, |m, cx| m.set_value("", cx))
    });
    cx.simulate_input("a");
    settle(cx);
    assert_eq!(labels(&editor, cx).map(|l| l.len()), Some(2));
}

/// Monaco `suggestModel.test.ts` "#21484: Trigger character always force a
/// new completion session": the open session's items of providers that
/// don't declare the typed trigger character are kept
/// (`providerItemsToReuse`, filtered by their own ranges), and only the
/// providers that do are asked.
#[gpui::test]
fn trigger_characters_keep_the_other_providers_items(cx: &mut TestAppContext) {
    let (editor, cx) = setup(
        "",
        |o| o.word_based_suggestions = WordBasedSuggestions::Off,
        cx,
    );
    let count = Rc::new(Cell::new(0));
    let counter = count.clone();
    provide(cx, move |p| {
        counter.set(counter.get() + 1);
        vec![CompletionItem {
            range: Some(PointUtf16::new(p.row, 0)..p),
            ..CompletionItem::new("foo.bar", CompletionItemKind::Property)
        }]
    });
    cx.update(|_, cx| register_completion_provider(cx, LanguageSelector::All, Boom));
    cx.simulate_input("foo");
    settle(cx);
    assert_eq!(labels(&editor, cx), Some(vec!["foo.bar".to_string()]));
    cx.simulate_input(".");
    settle(cx);
    assert_eq!(sorted_labels(&editor, cx), ["boom", "foo.bar"]);
    assert_eq!(count.get(), 1);
}

/// The `.`-triggered provider of Monaco's #21484 test: `boom` replacing
/// the character before the cursor, or nothing after a `.`.
struct Boom;
impl CompletionProvider for Boom {
    fn trigger_characters(&self) -> Vec<char> {
        vec!['.']
    }

    fn provide_completions(
        &self,
        context: &ProviderContext,
        position: PointUtf16,
        _: CompletionTrigger,
        _: &mut App,
    ) -> Task<anyhow::Result<CompletionList>> {
        let line = context.snapshot.line(position.row);
        let after_dot = line[..position.column as usize].ends_with('.');
        let start = if after_dot {
            position
        } else {
            PointUtf16::new(position.row, position.column.saturating_sub(1))
        };
        Task::ready(Ok(CompletionList {
            items: vec![CompletionItem {
                range: Some(start..position),
                ..CompletionItem::new("boom", CompletionItemKind::Property)
            }],
            incomplete: false,
        }))
    }
}

/// Monaco `suggestModel.test.ts` "Snippets gone from IntelliSense #173244"
/// and `_onNewContext`: typing on after an incomplete list asks only the
/// providers whose lists were incomplete, keeping the others' items.
#[gpui::test]
fn incomplete_lists_ask_only_their_providers_again(cx: &mut TestAppContext) {
    let (editor, cx) = setup(
        "",
        |o| o.word_based_suggestions = WordBasedSuggestions::Off,
        cx,
    );
    let complete = counted(cx, LanguageSelector::All, &["log"], &[], false);
    let incomplete = counted(cx, LanguageSelector::All, &["locals"], &[], true);
    cx.simulate_input("l");
    settle(cx);
    assert_eq!(sorted_labels(&editor, cx), ["locals", "log"]);
    cx.simulate_input("o");
    settle(cx);
    assert_eq!(sorted_labels(&editor, cx), ["locals", "log"]);
    assert_eq!((complete.get(), incomplete.get()), (1, 2));
    // Accepting a kept item still replaces the typed word.
    cx.simulate_keystrokes("down");
    let selected = editor.read_with(cx, |e, _| e.selected_suggestion().unwrap().1.label);
    cx.simulate_keystrokes("enter");
    assert_eq!(text(&editor, cx), selected.to_string());
}

/// Monaco `suggestModel.test.ts` "Trigger (full) completions when
/// (incomplete) completions are already active #99504": starting a new
/// word keeps the items providers gave and asks the others again.
#[gpui::test]
fn a_new_word_keeps_the_items_providers_gave(cx: &mut TestAppContext) {
    let (editor, cx) = setup(
        "",
        |o| o.word_based_suggestions = WordBasedSuggestions::Off,
        cx,
    );
    let count_a = Rc::new(Cell::new(0));
    let count_b = Rc::new(Cell::new(0));
    let a = count_a.clone();
    cx.update(|_, cx| {
        register_completion_provider(
            cx,
            LanguageSelector::All,
            Items(move |p: PointUtf16| {
                a.set(a.get() + 1);
                vec![CompletionItem {
                    range: Some(PointUtf16::new(0, 0)..p),
                    ..CompletionItem::new("Z aaa", CompletionItemKind::Class)
                }]
            }),
        )
    });
    let b = count_b.clone();
    cx.update(|_, cx| {
        register_completion_provider(
            cx,
            LanguageSelector::All,
            Items(move |p: PointUtf16| {
                b.set(b.get() + 1);
                // Only after `Z `: the word at the cursor starts with `a`.
                if p.column < 3 {
                    return Vec::new();
                }
                vec![CompletionItem::new("aaa", CompletionItemKind::Folder)]
            }),
        )
    });
    cx.simulate_input("Z");
    settle(cx);
    assert_eq!(labels(&editor, cx), Some(vec!["Z aaa".to_string()]));
    cx.simulate_input(" a");
    settle(cx);
    assert_eq!(sorted_labels(&editor, cx), ["Z aaa", "aaa"]);
    assert_eq!((count_a.get(), count_b.get()), (1, 2));
}

fn foo_items(_: PointUtf16) -> Vec<CompletionItem> {
    ["foo", "foobar", "fizz"]
        .into_iter()
        .map(|label| CompletionItem::new(label, CompletionItemKind::Variable))
        .collect()
}

fn cursor(editor: &Entity<CodeEditor>, cx: &mut VisualTestContext) -> PointUtf16 {
    editor.read_with(cx, |e, _| e.selections()[0].head)
}

/// Monaco `SuggestModel._onCursorChange`: Left and Right are explicit
/// keyboard moves, so the list refilters at the new position (asking again
/// left of where it was requested) and closes only once the cursor leaves
/// the word. A selection closes it.
#[gpui::test]
fn left_and_right_refilter_in_the_word(cx: &mut TestAppContext) {
    let (editor, cx) = setup("", |_| {}, cx);
    provide(cx, foo_items);
    cx.simulate_input("foob");
    settle(cx);
    assert_eq!(labels(&editor, cx), Some(vec!["foobar".to_string()]));
    // Left of the request: asked again for `foo`.
    cx.simulate_keystrokes("left");
    settle(cx);
    assert!(visible(&editor, cx));
    assert_eq!(
        labels(&editor, cx),
        Some(vec!["foo".to_string(), "foobar".to_string()])
    );
    // Right: refiltered for `foob`.
    cx.simulate_keystrokes("right");
    settle(cx);
    assert_eq!(labels(&editor, cx), Some(vec!["foobar".to_string()]));
    assert_eq!(cursor(&editor, cx), PointUtf16::new(0, 4));
    // Out of the word: closed.
    cx.simulate_keystrokes("home");
    settle(cx);
    assert!(!visible(&editor, cx));
    // A selection closes it.
    cx.simulate_keystrokes("end ctrl-space");
    settle(cx);
    assert!(visible(&editor, cx));
    cx.simulate_keystrokes("shift-left");
    assert!(!visible(&editor, cx));
}

/// Monaco's list keys need several suggestions or none focused
/// (`suggestWidgetMultipleSuggestions || !suggestWidgetHasFocusedSuggestion`):
/// with a single focused one, Up and Down move the cursor, which closes
/// the list.
#[gpui::test]
fn up_and_down_with_one_focused_suggestion_move_the_cursor(cx: &mut TestAppContext) {
    let (editor, cx) = setup("one\nfi\nthree", |_| {}, cx);
    provide(cx, foo_items);
    move_to(&editor, cx, 1, 2);
    cx.simulate_keystrokes("ctrl-space");
    settle(cx);
    assert_eq!(labels(&editor, cx), Some(vec!["fizz".to_string()]));
    cx.simulate_keystrokes("down");
    assert!(!visible(&editor, cx));
    assert_eq!(cursor(&editor, cx).row, 2);
    move_to(&editor, cx, 1, 2);
    cx.simulate_keystrokes("ctrl-space");
    settle(cx);
    cx.simulate_keystrokes("up");
    assert!(!visible(&editor, cx));
    assert_eq!(cursor(&editor, cx).row, 0);
    // The secondary keys fall through too (Cmd/Ctrl+Down is
    // `cursorBottom`).
    move_to(&editor, cx, 1, 2);
    cx.simulate_keystrokes("ctrl-space");
    settle(cx);
    cx.simulate_keystrokes("secondary-down");
    assert!(!visible(&editor, cx));
    assert_eq!(cursor(&editor, cx).row, 2);
    // With several suggestions the keys move in the list.
    move_to(&editor, cx, 1, 1);
    cx.simulate_keystrokes("ctrl-space");
    settle(cx);
    assert_eq!(labels(&editor, cx).map(|l| l.len()), Some(3));
    cx.simulate_keystrokes("down");
    assert!(visible(&editor, cx));
    assert_eq!(cursor(&editor, cx), PointUtf16::new(1, 1));
}

/// A single suggestion shown without focus (`selection_mode: Never`): Down
/// focuses it, as Monaco's precondition allows.
#[gpui::test]
fn down_focuses_a_single_unfocused_suggestion(cx: &mut TestAppContext) {
    use gpui_monaco::options::SuggestSelectionMode;
    let (editor, cx) = setup("", |_| {}, cx);
    set_selection_mode(&editor, cx, SuggestSelectionMode::Never);
    provide(cx, foo_items);
    cx.simulate_input("fi");
    settle(cx);
    assert_eq!(labels(&editor, cx), Some(vec!["fizz".to_string()]));
    assert_eq!(focused(&editor, cx), None);
    cx.simulate_keystrokes("down");
    assert_eq!(focused(&editor, cx).as_deref(), Some("fizz"));
    assert!(visible(&editor, cx));
}

/// Monaco's status bar menu (`suggestWidgetStatusbarMenu`): "Select" while
/// nothing is focused; Insert, and Replace when the item's ranges differ,
/// on the left; Show More / Show Less on the right.
#[gpui::test]
fn status_bar_actions(cx: &mut TestAppContext) {
    use gpui_monaco::options::SuggestSelectionMode;
    let (editor, cx) = setup("abc", |o| o.suggest.show_status_bar = true, cx);
    provide(cx, |p| {
        vec![CompletionItem {
            documentation: Some("Docs.".to_string().into()),
            ..ranged("aaa", 0, 3, p)
        }]
    });
    move_to(&editor, cx, 0, 1);
    cx.simulate_keystrokes("ctrl-space");
    settle(cx);
    let status = |cx: &mut VisualTestContext| {
        editor
            .read_with(cx, |e, cx| e.suggest_status_bar_for_tests(cx))
            .unwrap()
    };
    let mac = cfg!(target_os = "macos");
    let (enter, shift_enter, space) = if mac {
        ("⏎", "⇧⏎", "⌃Space")
    } else {
        ("Enter", "Shift+Enter", "Ctrl+Space")
    };
    assert_eq!(
        status(cx),
        (
            format!("Insert ({enter}), Replace ({shift_enter})"),
            format!("Show More ({space})")
        )
    );
    cx.simulate_keystrokes("ctrl-space");
    assert_eq!(status(cx).1, format!("Show Less ({space})"));
    cx.simulate_keystrokes("escape");
    // Nothing focused: "Select".
    set_selection_mode(&editor, cx, SuggestSelectionMode::Never);
    editor.update(cx, |e, cx| {
        e.model().update(cx, |m, cx| m.set_value("", cx))
    });
    cx.simulate_input("a");
    settle(cx);
    let down = if mac { "↓" } else { "DownArrow" };
    assert_eq!(status(cx), (format!("Select ({down})"), String::new()));
}

/// `suggest.shareSuggestSelections`: each window remembers its own
/// selections (Monaco's workspace storage) unless they're shared (its
/// profile storage).
#[gpui::test]
fn suggestion_memories_per_window_unless_shared(cx: &mut TestAppContext) {
    cx.update(gpui_monaco::init);
    let open = |share: bool, cx: &mut TestAppContext| {
        let window = cx.add_window(|window, cx| {
            let model = cx.new(|_| TextModel::new("").with_language("demo"));
            let mut editor = CodeEditor::new(model, window, cx);
            let mut options = editor.options().clone();
            options.suggest_selection = SuggestSelection::RecentlyUsedByPrefix;
            options.suggest.share_suggest_selections = share;
            editor.update_options(options, cx);
            editor
        });
        let editor = window.root(cx).unwrap();
        let mut vcx = VisualTestContext::from_window(window.into(), cx);
        vcx.update(|window, cx| window.focus(&editor.focus_handle(cx), cx));
        vcx.run_until_parked();
        (editor, vcx)
    };
    cx.update(|cx| {
        register_completion_provider(
            cx,
            LanguageSelector::All,
            Items(|_: PointUtf16| {
                ["aaa", "abb"]
                    .into_iter()
                    .map(|label| CompletionItem::new(label, CompletionItemKind::Variable))
                    .collect()
            }),
        )
    });
    // Accepts `abb` after `a` in a window, then asks after `a` in another.
    let remembered = |share: bool, cx: &mut TestAppContext| {
        let (first, mut first_cx) = open(share, cx);
        first_cx.simulate_input("a");
        settle(&mut first_cx);
        first_cx.simulate_keystrokes("down enter");
        assert_eq!(text(&first, &mut first_cx), "abb");
        let (second, mut second_cx) = open(share, cx);
        second_cx.simulate_input("a");
        settle(&mut second_cx);
        focused(&second, &mut second_cx)
    };
    assert_eq!(remembered(false, cx).as_deref(), Some("aaa"));
    assert_eq!(remembered(true, cx).as_deref(), Some("abb"));
}

/// Screen readers read the focused suggestion (Monaco's list aria label),
/// and the list's messages.
#[gpui::test]
fn screen_readers_hear_the_focused_suggestion(cx: &mut TestAppContext) {
    let (editor, cx) = setup(
        "",
        |o| o.word_based_suggestions = WordBasedSuggestions::Off,
        cx,
    );
    // Items for the first word only.
    provide(cx, |p| {
        if p.column > 1 {
            Vec::new()
        } else {
            foo_items(p)
        }
    });
    let heard = |cx: &mut VisualTestContext| {
        editor.read_with(cx, |e, _| {
            e.last_announcement_for_tests().map(|m| m.to_string())
        })
    };
    cx.simulate_input("f");
    settle(cx);
    assert_eq!(heard(cx).as_deref(), Some("fizz, Variable"));
    cx.simulate_keystrokes("down");
    assert_eq!(heard(cx).as_deref(), Some("foo, Variable"));
    cx.simulate_keystrokes("escape");
    // An explicit request with nothing: "No suggestions.".
    cx.simulate_input(" ");
    cx.simulate_keystrokes("ctrl-space");
    settle(cx);
    assert_eq!(heard(cx).as_deref(), Some("No suggestions."));
}

/// Items without docs whose docs arrive from a 300ms resolve.
struct DocsOnResolve;
impl CompletionProvider for DocsOnResolve {
    fn provide_completions(
        &self,
        _: &ProviderContext,
        _: PointUtf16,
        _: CompletionTrigger,
        _: &mut App,
    ) -> Task<anyhow::Result<CompletionList>> {
        Task::ready(Ok(CompletionList {
            items: ["alpha", "beta", "gamma"]
                .into_iter()
                .map(|label| CompletionItem::new(label, CompletionItemKind::Function))
                .collect(),
            incomplete: false,
        }))
    }

    fn resolve_completion_item(
        &self,
        item: &CompletionItem,
        cx: &mut App,
    ) -> Option<Task<anyhow::Result<CompletionItem>>> {
        let item = item.clone();
        let timer = cx.background_executor().timer(Duration::from_millis(300));
        Some(cx.background_executor().spawn(async move {
            timer.await;
            Ok(CompletionItem {
                documentation: Some(format!("Docs for {}.", item.label).into()),
                ..item
            })
        }))
    }
}

/// The details panel's docs (or its detail without docs), while it shows.
fn details_docs(editor: &Entity<CodeEditor>, cx: &mut VisualTestContext) -> Option<String> {
    editor
        .read_with(cx, |e, cx| e.suggest_details_for_tests(cx))
        .map(|(detail, docs)| docs.or(detail).unwrap_or_default())
}

/// Monaco `SuggestWidget._onListFocus`: while the focused item resolves,
/// the details panel keeps what it showed, shows "Loading..." after 250ms,
/// then the resolved docs (#344).
#[gpui::test]
fn details_stay_while_an_item_resolves(cx: &mut TestAppContext) {
    let (editor, cx) = setup("", |o| o.quick_suggestions = QuickSuggestions::OFF, cx);
    cx.update(|_, cx| register_completion_provider(cx, LanguageSelector::All, DocsOnResolve));
    cx.simulate_keystrokes("ctrl-space");
    cx.run_until_parked();
    // Nothing to expand until it resolves (Monaco `toggleDetails`).
    cx.simulate_keystrokes("ctrl-space");
    assert_eq!(details_docs(&editor, cx), None);
    cx.executor().advance_clock(Duration::from_millis(300));
    cx.run_until_parked();
    cx.simulate_keystrokes("ctrl-space");
    assert_eq!(
        details_docs(&editor, cx).as_deref(),
        Some("Docs for alpha.")
    );
    // The next item resolves: alpha's docs stay meanwhile.
    cx.simulate_keystrokes("down");
    cx.executor().advance_clock(Duration::from_millis(200));
    cx.run_until_parked();
    assert_eq!(
        details_docs(&editor, cx).as_deref(),
        Some("Docs for alpha.")
    );
    // After 250ms, "Loading...".
    cx.executor().advance_clock(Duration::from_millis(60));
    cx.run_until_parked();
    assert_eq!(details_docs(&editor, cx).as_deref(), Some("Loading..."));
    cx.executor().advance_clock(Duration::from_millis(50));
    cx.run_until_parked();
    assert_eq!(details_docs(&editor, cx).as_deref(), Some("Docs for beta."));
    // Back to a resolved item: at once.
    cx.simulate_keystrokes("up");
    assert_eq!(
        details_docs(&editor, cx).as_deref(),
        Some("Docs for alpha.")
    );
    // Typing refilters without resolving the focused item again.
    cx.simulate_input("a");
    assert_eq!(
        details_docs(&editor, cx).as_deref(),
        Some("Docs for alpha.")
    );
}

fn focused_label(editor: &Entity<CodeEditor>, cx: &mut VisualTestContext) -> Option<String> {
    editor.read_with(cx, |e, _| {
        e.selected_suggestion()
            .map(|(_, item)| item.label.to_string())
    })
}

/// Monaco's `toggleDetails` opens the details only when the focused item
/// has something to show (#344).
#[gpui::test]
fn toggling_details_needs_something_to_show(cx: &mut TestAppContext) {
    let (editor, cx) = setup("", |o| o.quick_suggestions = QuickSuggestions::OFF, cx);
    provide(cx, |_| {
        vec![
            CompletionItem::new("plain", CompletionItemKind::Function),
            CompletionItem {
                documentation: Some("Prints.".into()),
                ..CompletionItem::new("print", CompletionItemKind::Function)
            },
        ]
    });
    let details = |cx: &mut VisualTestContext| {
        editor.read_with(cx, |e, cx| e.is_suggestion_details_visible(cx))
    };
    cx.simulate_keystrokes("ctrl-space");
    cx.run_until_parked();
    assert_eq!(focused_label(&editor, cx).as_deref(), Some("plain"));
    // No docs: nothing happens, so the next item doesn't open them either.
    cx.simulate_keystrokes("ctrl-space");
    cx.simulate_keystrokes("down");
    assert!(!details(cx));
    cx.simulate_keystrokes("ctrl-space");
    assert!(details(cx));
}

/// Monaco's status bar "Show More" follows `suggestionCanResolve`: word
/// based suggestions (no detail) have it, though they have nothing to
/// show (#344).
#[gpui::test]
fn show_more_follows_suggestion_can_resolve(cx: &mut TestAppContext) {
    let (editor, cx) = setup(
        "alpha alphabet\n",
        |o| {
            o.suggest.show_status_bar = true;
            o.quick_suggestions = QuickSuggestions::OFF;
        },
        cx,
    );
    move_to(&editor, cx, 1, 0);
    cx.simulate_input("al");
    cx.simulate_keystrokes("ctrl-space");
    settle(cx);
    let status = |cx: &mut VisualTestContext| {
        editor
            .read_with(cx, |e, cx| e.suggest_status_bar_for_tests(cx))
            .unwrap()
            .1
    };
    let space = if cfg!(target_os = "macos") {
        "⌃Space"
    } else {
        "Ctrl+Space"
    };
    assert_eq!(status(cx), format!("Show More ({space})"));
    // Toggling shows nothing: still "Show More".
    cx.simulate_keystrokes("ctrl-space");
    assert_eq!(status(cx), format!("Show More ({space})"));
    cx.simulate_keystrokes("escape");

    // A detail equal to the label, no docs and no resolve: none.
    provide(cx, |_| {
        vec![CompletionItem {
            detail: Some("same".into()),
            ..CompletionItem::new("same", CompletionItemKind::Function)
        }]
    });
    cx.simulate_input(" s");
    cx.simulate_keystrokes("ctrl-space");
    settle(cx);
    assert_eq!(status(cx), "");
}

/// Monaco's details show a detail equal to the label when there are docs
/// too (`renderItem`; only `canExpandCompletionItem` compares them) (#344).
#[gpui::test]
fn details_show_a_detail_equal_to_the_label(cx: &mut TestAppContext) {
    let (editor, cx) = setup("", |o| o.quick_suggestions = QuickSuggestions::OFF, cx);
    provide(cx, |_| {
        vec![CompletionItem {
            detail: Some("print".into()),
            documentation: Some("Prints.".into()),
            ..CompletionItem::new("print", CompletionItemKind::Function)
        }]
    });
    cx.simulate_keystrokes("ctrl-space");
    cx.run_until_parked();
    cx.simulate_keystrokes("ctrl-space");
    assert_eq!(
        editor.read_with(cx, |e, cx| e.suggest_details_for_tests(cx)),
        Some((Some("print".to_string()), Some("Prints.".to_string())))
    );
}

/// Replaces the last accepted suggestion with the next (Monaco
/// `insertNextSuggestion`), if Tab may.
fn insert_next_suggestion(editor: &Entity<CodeEditor>, cx: &mut VisualTestContext) {
    cx.update(|window, cx| {
        editor.update(cx, |e, cx| e.trigger("insertNextSuggestion", window, cx))
    })
    .unwrap();
    cx.run_until_parked();
}

/// Monaco keeps the other suggestions for Tab only after Tab or Enter
/// (`KeepAlternativeSuggestions`); Shift+Enter and commit characters
/// don't (#344).
#[gpui::test]
fn only_tab_and_enter_keep_the_alternatives(cx: &mut TestAppContext) {
    let (editor, cx) = setup(
        "",
        |o| {
            o.tab_completion = gpui_monaco::options::TabCompletion::On;
            o.quick_suggestions = QuickSuggestions::OFF;
        },
        cx,
    );
    provide(cx, |_| {
        ["print", "println"]
            .into_iter()
            .map(|label| CompletionItem {
                commit_characters: vec![';'],
                ..CompletionItem::new(label, CompletionItemKind::Function)
            })
            .collect()
    });
    // Enter: the next replaces it.
    cx.simulate_input("pri");
    cx.simulate_keystrokes("ctrl-space");
    cx.run_until_parked();
    cx.simulate_keystrokes("enter");
    assert_eq!(text(&editor, cx), "print");
    insert_next_suggestion(&editor, cx);
    assert_eq!(text(&editor, cx), "println");
    // Shift+Enter: not.
    cx.simulate_input(" pri");
    cx.simulate_keystrokes("ctrl-space");
    cx.run_until_parked();
    cx.simulate_keystrokes("shift-enter");
    assert_eq!(text(&editor, cx), "println print");
    insert_next_suggestion(&editor, cx);
    assert_eq!(text(&editor, cx), "println print");
    // A commit character: not either.
    cx.simulate_input(" pri");
    cx.simulate_keystrokes("ctrl-space");
    cx.run_until_parked();
    cx.simulate_input(";");
    assert_eq!(text(&editor, cx), "println print print;");
    insert_next_suggestion(&editor, cx);
    assert_eq!(text(&editor, cx), "println print print;");
}

/// Monaco `SnippetController2`: while a snippet choice is active, its
/// options are the only suggestions (an exclusive provider), so they show
/// again after the list is dismissed (audit 3 §A.23).
#[gpui::test]
fn snippet_choices_are_offered_again(cx: &mut TestAppContext) {
    let (editor, cx) = setup("", |_| {}, cx);
    provide(cx, |_| {
        vec![CompletionItem::new("print", CompletionItemKind::Function)]
    });
    editor.update(cx, |e, cx| {
        e.insert_snippet("${1|one,two,three|} $2", Default::default(), cx)
    });
    cx.run_until_parked();
    let choices = Some(vec!["one".into(), "two".into(), "three".into()]);
    assert_eq!(labels(&editor, cx), choices);
    cx.simulate_keystrokes("escape");
    assert!(!visible(&editor, cx));
    assert!(editor.read_with(cx, |e, _| e.in_snippet_mode()));
    cx.simulate_keystrokes("ctrl-space");
    cx.run_until_parked();
    assert_eq!(labels(&editor, cx), choices);
    // Choosing one goes on to the next placeholder, where the other
    // providers are asked again.
    cx.simulate_keystrokes("down enter");
    cx.run_until_parked();
    assert_eq!(text(&editor, cx), "two ");
    cx.simulate_keystrokes("ctrl-space");
    cx.run_until_parked();
    assert_eq!(labels(&editor, cx), Some(vec!["print".into()]));
}
