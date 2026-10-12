//! Which binding a key runs when several match (Monaco's `KeybindingWeight`
//! and `when` clauses): the widget and state keys beat the editor's own,
//! ghost text, the quick input's list keys, Escape's order and the Menu
//! key.

use std::time::Duration;

use gpui::{App, AppContext as _, Entity, Focusable, Task, TestAppContext, VisualTestContext};
use gpui_monaco::cursor::Selection;
use gpui_monaco::options::{AccessibilitySupport, GoToLocation};
use gpui_monaco::providers::*;
use gpui_monaco::text::PointUtf16;
use gpui_monaco::{CodeEditor, Marker, MarkerSeverity, SnippetInsertOptions, TextModel};

fn p(row: u32, column: u32) -> PointUtf16 {
    PointUtf16::new(row, column)
}

fn keys(binding: &gpui::KeyBinding) -> String {
    binding
        .keystrokes()
        .iter()
        .map(|k| k.inner().unparse())
        .collect::<Vec<_>>()
        .join(" ")
}

/// The conjuncts of `predicate` (`a && b && c` → `[a, b, c]`).
fn conjuncts(
    predicate: &gpui::KeyBindingContextPredicate,
) -> Vec<&gpui::KeyBindingContextPredicate> {
    match predicate {
        gpui::KeyBindingContextPredicate::And(left, right) => {
            let mut all = conjuncts(left);
            all.extend(conjuncts(right));
            all
        }
        other => vec![other],
    }
}

/// Whether `general` holds wherever `specific` does, at the same depth:
/// each of its conjuncts is one of `specific`'s (or the context that a
/// `Parent > Child` predicate matches at).
fn implied_by(
    general: &gpui::KeyBindingContextPredicate,
    specific: &gpui::KeyBindingContextPredicate,
) -> bool {
    general != specific
        && conjuncts(general)
            .into_iter()
            .all(|conjunct| conjunct.is_superset(specific))
}

/// The regression test for #336. GPUI runs, of the bindings matching at
/// one depth, the one added last, so a binding with a more general context
/// added after one with a more specific context and the same keys would
/// shadow it, as the platform blocks once shadowed ghost text's
/// Cmd/Ctrl+Right and the quick input's keys. Monaco gives every such
/// widget or state rule more weight than the general rule it shares keys
/// with, so the specific binding must come later.
#[test]
fn no_general_binding_shadows_a_specific_one() {
    let bindings = gpui_monaco::actions::default_key_bindings();
    let mut shadowed = Vec::new();
    for (i, specific) in bindings.iter().enumerate() {
        let Some(specific_predicate) = specific.predicate() else {
            continue;
        };
        for general in &bindings[i + 1..] {
            if keys(general) != keys(specific) {
                continue;
            }
            let Some(general_predicate) = general.predicate() else {
                continue;
            };
            if implied_by(&general_predicate, &specific_predicate) {
                shadowed.push(format!(
                    "{}: {} [{specific_predicate}] by {} [{general_predicate}]",
                    keys(specific),
                    specific.action().name(),
                    general.action().name(),
                ));
            }
        }
    }
    assert!(shadowed.is_empty(), "{shadowed:#?}");
}

/// Monaco weighs `goToNextReference`'s F12 (`EditorContrib - 10`) below Go
/// to Definition (`EditorContrib`): the one exception, bound first.
#[test]
fn go_to_definition_outweighs_next_reference() {
    let bindings = gpui_monaco::actions::default_key_bindings();
    let index = |action: &str| {
        bindings
            .iter()
            .position(|b| keys(b) == "f12" && b.action().name() == action)
            .unwrap()
    };
    assert!(index("code_editor::GoToNextReference") < index("code_editor::GoToDefinition"));
}

// ----- ghost text -------------------------------------------------------------

/// Completes the line with "hello world".
struct Hello;

impl InlineCompletionsProvider for Hello {
    fn provide_inline_completions(
        &self,
        _: &ProviderContext,
        position: PointUtf16,
        _: InlineCompletionContext,
        _: &mut App,
    ) -> Task<anyhow::Result<InlineCompletionList>> {
        Task::ready(Ok(InlineCompletionList {
            items: vec![InlineCompletion {
                insert_text: "hello world".into(),
                range: Some(p(position.row, 0)..position),
                ..Default::default()
            }],
            ..Default::default()
        }))
    }
}

fn ghost_setup(
    accessibility: AccessibilitySupport,
    cx: &mut TestAppContext,
) -> (Entity<CodeEditor>, &mut VisualTestContext) {
    cx.update(|cx| {
        gpui_monaco::init(cx);
        register_inline_completions_provider(cx, LanguageSelector::All, Hello);
    });
    let (editor, cx) = cx.add_window_view(|window, cx| {
        let model = cx.new(|_| TextModel::new(""));
        let mut editor = CodeEditor::new(model, window, cx);
        let mut options = editor.options().clone();
        options.quick_suggestions = gpui_monaco::options::QuickSuggestions::OFF;
        options.word_based_suggestions = gpui_monaco::options::WordBasedSuggestions::Off;
        options.accessibility_support = accessibility;
        editor.update_options(options, cx);
        editor
    });
    cx.update(|window, cx| window.focus(&editor.focus_handle(cx), cx));
    cx.run_until_parked();
    cx.simulate_input("he");
    settle(cx);
    assert!(editor.read_with(cx, |e, cx| e.inline_suggestion_visible(cx)));
    (editor, cx)
}

fn settle(cx: &mut VisualTestContext) {
    cx.executor().advance_clock(Duration::from_millis(100));
    cx.run_until_parked();
    cx.update(|window, _| window.refresh());
    cx.run_until_parked();
}

fn text(editor: &Entity<CodeEditor>, cx: &mut VisualTestContext) -> String {
    editor.read_with(cx, |e, cx| e.model().read(cx).text())
}

fn head(editor: &Entity<CodeEditor>, cx: &mut VisualTestContext) -> PointUtf16 {
    editor.read_with(cx, |e, _| e.selections()[0].head)
}

/// Monaco `editor.action.inlineSuggest.acceptNextWord` (CtrlCmd+Right,
/// `EditorContrib + 1`) beats Cmd+Right (End) on macOS and Ctrl+Right
/// (word right) elsewhere.
#[gpui::test]
fn secondary_right_accepts_the_next_word_of_ghost_text(cx: &mut TestAppContext) {
    let (editor, cx) = ghost_setup(AccessibilitySupport::Off, cx);
    cx.simulate_keystrokes(if cfg!(target_os = "macos") {
        "cmd-right"
    } else {
        "ctrl-right"
    });
    assert_eq!(text(&editor, cx), "hello");
    assert_eq!(head(&editor, cx), p(0, 5));
    assert!(editor.read_with(cx, |e, cx| e.inline_suggestion_visible(cx)));
}

/// With a screen reader (Monaco's `!accessibilityModeEnabled`), the key
/// moves the cursor instead.
#[gpui::test]
fn secondary_right_moves_the_cursor_for_screen_readers(cx: &mut TestAppContext) {
    let (editor, cx) = ghost_setup(AccessibilitySupport::On, cx);
    editor.update(cx, |e, cx| {
        e.set_selections(&[Selection::collapsed(p(0, 0))], cx)
    });
    settle(cx);
    cx.simulate_keystrokes(if cfg!(target_os = "macos") {
        "cmd-right"
    } else {
        "ctrl-right"
    });
    assert_eq!(text(&editor, cx), "he");
    assert_eq!(head(&editor, cx), p(0, 2));
}

// ----- quick input ----------------------------------------------------------

fn plain_setup<'a>(
    text: &str,
    cx: &'a mut TestAppContext,
) -> (Entity<CodeEditor>, &'a mut VisualTestContext) {
    cx.update(gpui_monaco::init);
    let text = text.to_string();
    let (editor, cx) = cx.add_window_view(|window, cx| {
        let model = cx.new(|_| TextModel::new(&text).with_uri("file:///main.rs"));
        CodeEditor::new(model, window, cx)
    });
    cx.update(|window, cx| window.focus(&editor.focus_handle(cx), cx));
    settle(cx);
    (editor, cx)
}

/// The focused row's index among the quick input's items.
fn active_index(editor: &Entity<CodeEditor>, cx: &mut VisualTestContext) -> Option<usize> {
    cx.run_until_parked();
    editor
        .read_with(cx, |e, cx| e.quick_access_rows_for_tests(cx))
        .iter()
        .filter(|row| !row.starts_with('['))
        .position(|row| row.starts_with("> "))
}

fn item_count(editor: &Entity<CodeEditor>, cx: &mut VisualTestContext) -> usize {
    editor
        .read_with(cx, |e, cx| e.quick_access_rows_for_tests(cx))
        .iter()
        .filter(|row| !row.starts_with('['))
        .count()
}

/// VS Code's `quickInput.first`/`last` (Ctrl+Home/End, WinCtrl on macOS)
/// and `next`/`previous` with Ctrl move in the list, over the input's
/// Ctrl+Home/End and Ctrl+Up/Down.
#[gpui::test]
fn quick_input_first_last_and_ctrl_arrows(cx: &mut TestAppContext) {
    let (editor, cx) = plain_setup("fn main() {}\n", cx);
    cx.simulate_keystrokes("f1");
    settle(cx);
    let last = item_count(&editor, cx) - 1;
    assert_eq!(active_index(&editor, cx), Some(0));
    cx.simulate_keystrokes("ctrl-end");
    assert_eq!(active_index(&editor, cx), Some(last));
    cx.simulate_keystrokes("ctrl-home");
    assert_eq!(active_index(&editor, cx), Some(0));
    cx.simulate_keystrokes("ctrl-down");
    assert_eq!(active_index(&editor, cx), Some(1));
    cx.simulate_keystrokes("ctrl-up");
    assert_eq!(active_index(&editor, cx), Some(0));
    cx.simulate_keystrokes("ctrl-alt-end");
    assert_eq!(active_index(&editor, cx), Some(last));
    cx.simulate_keystrokes("ctrl-alt-home");
    assert_eq!(active_index(&editor, cx), Some(0));
    if cfg!(target_os = "macos") {
        // `withCmdMod`, and VS Code's `quickOpenSelectNext`/`Previous`.
        cx.simulate_keystrokes("cmd-ctrl-end");
        assert_eq!(active_index(&editor, cx), Some(last));
        cx.simulate_keystrokes("cmd-ctrl-home");
        assert_eq!(active_index(&editor, cx), Some(0));
        cx.simulate_keystrokes("ctrl-n ctrl-n");
        assert_eq!(active_index(&editor, cx), Some(2));
        cx.simulate_keystrokes("ctrl-p");
        assert_eq!(active_index(&editor, cx), Some(1));
        // Cmd+End is the input's own key, as in VS Code.
        cx.simulate_keystrokes("cmd-end");
        assert_eq!(active_index(&editor, cx), Some(1));
    }
}

// ----- Escape -----------------------------------------------------------------

#[derive(Clone)]
struct Fixed(Vec<Location>);

impl ReferenceProvider for Fixed {
    fn provide_references(
        &self,
        _: &ProviderContext,
        _: PointUtf16,
        _: ReferenceContext,
        _: &mut App,
    ) -> Task<anyhow::Result<Vec<Location>>> {
        Task::ready(Ok(self.0.clone()))
    }
}

impl ImplementationProvider for Fixed {
    fn provide_implementation(
        &self,
        _: &ProviderContext,
        _: PointUtf16,
        _: &mut App,
    ) -> Task<anyhow::Result<Vec<Location>>> {
        Task::ready(Ok(self.0.clone()))
    }
}

const TEXT: &str = "fn add(a: i32) {}\nfn main() {\n    add(1);\n    add(2);\n}\n";

fn locations() -> Fixed {
    let here = |row, start, end| Location {
        uri: None,
        range: p(row, start)..p(row, end),
        ..Default::default()
    };
    Fixed(vec![here(0, 3, 6), here(2, 4, 7), here(3, 4, 7)])
}

fn location_setup(cx: &mut TestAppContext) -> (Entity<CodeEditor>, &mut VisualTestContext) {
    cx.update(|cx| {
        register_reference_provider(cx, LanguageSelector::All, locations());
        register_implementation_provider(cx, LanguageSelector::All, locations());
    });
    plain_setup(TEXT, cx)
}

fn focus_editor(editor: &Entity<CodeEditor>, cx: &mut VisualTestContext) {
    cx.update(|window, cx| window.focus(&editor.focus_handle(cx), cx));
    settle(cx);
}

/// Monaco's `closeReferenceSearch` (`WorkbenchContrib + 50`) outweighs
/// `hideSuggestWidget` (`EditorContrib + 90`) and the parameter hints.
#[gpui::test]
fn escape_closes_the_peek_before_the_suggest_widget(cx: &mut TestAppContext) {
    let (editor, cx) = location_setup(cx);
    editor.update(cx, |e, cx| {
        e.set_selections(&[Selection::collapsed(p(2, 5))], cx)
    });
    cx.simulate_keystrokes("shift-f12");
    settle(cx);
    assert!(editor.read_with(cx, |e, _| e.is_peek_visible()));
    focus_editor(&editor, cx);
    editor.update(cx, |e, cx| {
        e.set_selections(&[Selection::collapsed(p(4, 1))], cx)
    });
    cx.simulate_keystrokes("enter");
    cx.simulate_input("ad");
    cx.simulate_keystrokes("ctrl-space");
    settle(cx);
    assert!(editor.read_with(cx, |e, _| e.is_suggest_visible()));
    cx.simulate_keystrokes("escape");
    settle(cx);
    assert!(!editor.read_with(cx, |e, _| e.is_peek_visible()));
    assert!(editor.read_with(cx, |e, _| e.is_suggest_visible()));
    cx.simulate_keystrokes("escape");
    assert!(!editor.read_with(cx, |e, _| e.is_suggest_visible()));
}

/// Monaco's `closeMarkersNavigation` (`EditorContrib + 50`) outweighs
/// `leaveSnippet` (+30).
#[gpui::test]
fn escape_closes_the_problem_zone_before_the_snippet(cx: &mut TestAppContext) {
    let (editor, cx) = plain_setup("", cx);
    editor.update(cx, |e, cx| {
        e.insert_snippet("${1:one} ${2:two}", SnippetInsertOptions::default(), cx)
    });
    editor.update(cx, |e, cx| {
        e.model().update(cx, |m, cx| {
            m.set_markers(
                "test",
                vec![Marker {
                    severity: MarkerSeverity::Error,
                    message: "bad".into(),
                    range: p(0, 0)..p(0, 3),
                    ..Default::default()
                }],
                cx,
            )
        })
    });
    settle(cx);
    editor.update(cx, |e, cx| e.go_to_marker(true, cx));
    focus_editor(&editor, cx);
    let zone = |cx: &mut VisualTestContext| {
        editor.read_with(cx, |e, cx| e.marker_zone_state_for_tests(cx).is_some())
    };
    assert!(zone(cx));
    assert!(editor.read_with(cx, |e, _| e.in_snippet_mode()));
    cx.simulate_keystrokes("escape");
    settle(cx);
    assert!(!zone(cx));
    assert!(editor.read_with(cx, |e, _| e.in_snippet_mode()));
    cx.simulate_keystrokes("escape");
    assert!(!editor.read_with(cx, |e, _| e.in_snippet_mode()));
}

/// Monaco's `closeFindWidget` (`EditorContrib + 5`) outweighs
/// `editor.gotoNextSymbolFromResult.cancel` (`EditorContrib`).
#[gpui::test]
fn escape_closes_find_before_symbol_navigation(cx: &mut TestAppContext) {
    let (editor, cx) = location_setup(cx);
    editor.update(cx, |e, cx| {
        let mut options = e.options().clone();
        options.goto_location.multiple_implementations = GoToLocation::Goto;
        e.update_options(options, cx);
    });
    // The find widget first: its input taking the focus would end the
    // navigation.
    cx.simulate_keystrokes("secondary-f");
    settle(cx);
    focus_editor(&editor, cx);
    editor.update(cx, |e, cx| {
        e.set_selections(&[Selection::collapsed(p(1, 3))], cx)
    });
    cx.simulate_keystrokes(if cfg!(target_os = "macos") {
        "cmd-f12"
    } else {
        "ctrl-f12"
    });
    settle(cx);
    assert!(editor.read_with(cx, |e, _| e.is_find_visible()));
    assert!(cx.update(|_, cx| gpui_monaco::has_symbol_navigation(cx)));
    cx.simulate_keystrokes("escape");
    settle(cx);
    assert!(!editor.read_with(cx, |e, _| e.is_find_visible()));
    assert!(cx.update(|_, cx| gpui_monaco::has_symbol_navigation(cx)));
    cx.simulate_keystrokes("escape");
    assert!(!cx.update(|_, cx| gpui_monaco::has_symbol_navigation(cx)));
}

// ----- Menu key ---------------------------------------------------------------

/// Monaco's `KeyCode.ContextMenu`: the Menu (Apps) key, which GPUI calls
/// `menu`, opens the context menu.
#[gpui::test]
fn the_menu_key_opens_the_context_menu(cx: &mut TestAppContext) {
    let (editor, cx) = plain_setup("foo", cx);
    cx.simulate_keystrokes("menu");
    cx.run_until_parked();
    assert!(editor.read_with(cx, |e, _| e.is_context_menu_visible()));
    cx.simulate_keystrokes("escape");
    cx.run_until_parked();
    assert!(!editor.read_with(cx, |e, _| e.is_context_menu_visible()));
    // Not with `context_menu: false`.
    editor.update(cx, |e, cx| {
        let mut options = e.options().clone();
        options.context_menu = false;
        e.update_options(options, cx);
    });
    cx.simulate_keystrokes("menu");
    cx.run_until_parked();
    assert!(!editor.read_with(cx, |e, _| e.is_context_menu_visible()));
}
