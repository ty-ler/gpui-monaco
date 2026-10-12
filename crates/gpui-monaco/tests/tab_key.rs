//! Which rule Tab and Shift+Tab run (#332): Monaco's `tab` and `outdent`
//! need `editorTextFocus`, `tabDoesNotMoveFocus` and `writable`, while
//! suggest, snippet and tab completion rules outweigh tab focus mode, and a
//! snippet choice's `jumpToPrevSnippetPlaceholder` outweighs
//! `acceptAlternativeSelectedSuggestion`.

use std::time::Duration;

use gpui::{
    App, AppContext as _, Context, Entity, FocusHandle, Focusable, IntoElement, ParentElement,
    Render, Styled, Task, TestAppContext, VisualTestContext, Window, div,
};
use gpui_monaco::cursor::Selection;
use gpui_monaco::options::{QuickSuggestions, WordBasedSuggestions};
use gpui_monaco::providers::*;
use gpui_monaco::text::PointUtf16;
use gpui_monaco::{CodeEditor, EditorOptions, SnippetInsertOptions, TextModel};

fn p(row: u32, column: u32) -> PointUtf16 {
    PointUtf16::new(row, column)
}

/// Two editors, so the focus has somewhere to go.
struct TwoEditors {
    first: Entity<CodeEditor>,
    second: Entity<CodeEditor>,
}

impl Render for TwoEditors {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
            .size_full()
            .flex()
            .flex_col()
            .child(div().h_40().child(self.first.clone()))
            .child(div().h_40().child(self.second.clone()))
    }
}

/// Two editors with `text` and `options`; the first has the focus.
fn setup<'a>(
    text: &str,
    options: impl Fn(&mut EditorOptions),
    cx: &'a mut TestAppContext,
) -> (
    Entity<CodeEditor>,
    Entity<CodeEditor>,
    &'a mut VisualTestContext,
) {
    cx.update(gpui_monaco::init);
    let text = text.to_string();
    let (view, cx) = cx.add_window_view(|window, cx| {
        let editor = |window: &mut Window, cx: &mut Context<TwoEditors>| {
            cx.new(|cx| {
                let model = cx.new(|_| TextModel::new(&text).with_uri("file:///main.rs"));
                let mut editor = CodeEditor::new(model, window, cx);
                let mut opts = editor.options().clone();
                opts.quick_suggestions = QuickSuggestions::OFF;
                opts.word_based_suggestions = WordBasedSuggestions::Off;
                options(&mut opts);
                editor.update_options(opts, cx);
                editor
            })
        };
        TwoEditors {
            first: editor(window, cx),
            second: editor(window, cx),
        }
    });
    let (first, second) = view.read_with(cx, |v, _| (v.first.clone(), v.second.clone()));
    let handle = handle(&first, cx);
    cx.update(|window, cx| window.focus(&handle, cx));
    settle(cx);
    (first, second, cx)
}

fn handle(editor: &Entity<CodeEditor>, cx: &mut VisualTestContext) -> FocusHandle {
    cx.update(|_, cx| editor.focus_handle(cx))
}

fn focused(editor: &Entity<CodeEditor>, cx: &mut VisualTestContext) -> bool {
    let handle = handle(editor, cx);
    cx.update(|window, _| handle.is_focused(window))
}

fn settle(cx: &mut VisualTestContext) {
    cx.executor().advance_clock(Duration::from_millis(100));
    cx.run_until_parked();
    cx.update(|window, cx| {
        window.refresh();
        window.draw(cx).clear(cx);
    });
    cx.run_until_parked();
}

fn text(editor: &Entity<CodeEditor>, cx: &mut VisualTestContext) -> String {
    editor.read_with(cx, |e, cx| e.model().read(cx).text())
}

fn selection(editor: &Entity<CodeEditor>, cx: &mut VisualTestContext) -> (PointUtf16, PointUtf16) {
    editor.read_with(cx, |e, _| {
        let s = e.selections()[0];
        (s.anchor, s.head)
    })
}

fn can_undo(editor: &Entity<CodeEditor>, cx: &mut VisualTestContext) -> bool {
    editor.read_with(cx, |e, cx| e.model().read(cx).can_undo())
}

fn tab_focus_mode(options: &mut EditorOptions) {
    options.tab_focus_mode = true;
}

// ----- the quick input and the rename box (§C.35) --------------------------

/// Monaco's `tab` and `outdent` need `editorTextFocus`: in the quick input
/// they don't indent the editor at its hidden cursor.
#[gpui::test]
fn tab_in_the_quick_input_leaves_the_editor_alone(cx: &mut TestAppContext) {
    let (editor, _, cx) = setup("fn main() {}", |_| {}, cx);
    for open in ["f1", "ctrl-g"] {
        cx.simulate_keystrokes(open);
        settle(cx);
        assert!(editor.read_with(cx, |e, _| e.is_quick_access_open()));
        cx.simulate_keystrokes("tab shift-tab tab");
        settle(cx);
        assert!(editor.read_with(cx, |e, _| e.is_quick_access_open()));
        cx.simulate_keystrokes("escape");
        settle(cx);
        assert_eq!(text(&editor, cx), "fn main() {}");
        assert!(!can_undo(&editor, cx));
    }
}

/// Opens the rename box on any word.
struct Renames;

impl RenameProvider for Renames {
    fn provide_rename_edits(
        &self,
        _: &ProviderContext,
        _: PointUtf16,
        _: &str,
        _: &mut App,
    ) -> Task<anyhow::Result<Option<WorkspaceEdit>>> {
        Task::ready(Ok(None))
    }
}

#[gpui::test]
fn tab_in_the_rename_box_leaves_the_editor_alone(cx: &mut TestAppContext) {
    let (editor, _, cx) = setup("fn main() {}", |_| {}, cx);
    cx.update(|_, cx| register_rename_provider(cx, LanguageSelector::All, Renames));
    editor.update(cx, |e, cx| {
        e.set_selections(&[Selection::collapsed(p(0, 4))], cx)
    });
    cx.simulate_keystrokes("f2");
    settle(cx);
    assert!(editor.read_with(cx, |e, _| e.is_rename_visible()));
    cx.simulate_keystrokes("tab shift-tab");
    settle(cx);
    assert!(editor.read_with(cx, |e, _| e.is_rename_visible()));
    cx.simulate_keystrokes("escape");
    settle(cx);
    assert_eq!(text(&editor, cx), "fn main() {}");
    assert!(!can_undo(&editor, cx));
}

/// The find widget keeps taking Tab from its inputs in tab focus mode and
/// in read-only editors: neither `editorTextFocus` rule applies there.
#[gpui::test]
fn tab_in_the_find_widget_stays_in_it(cx: &mut TestAppContext) {
    let (editor, _, cx) = setup(
        "one",
        |o| {
            o.tab_focus_mode = true;
            o.read_only = true;
        },
        cx,
    );
    cx.simulate_keystrokes("secondary-f");
    settle(cx);
    let focus = |cx: &mut VisualTestContext| {
        editor.update_in(cx, |e, window, cx| {
            (
                e.find_widget_focus_for_tests(window, cx),
                e.find_widget_focused_control_for_tests(window, cx),
            )
        })
    };
    let ((find_input, _, _), _) = focus(cx);
    assert!(find_input);
    cx.simulate_keystrokes("tab");
    settle(cx);
    let ((find_input, _, widget), control) = focus(cx);
    assert!(!find_input && widget && control.is_some(), "{control:?}");
    assert_eq!(text(&editor, cx), "one");
}

// ----- tab focus mode (§H.3) ---------------------------------------------------

/// Suggests `hello`.
struct Hello;

impl CompletionProvider for Hello {
    fn provide_completions(
        &self,
        _: &ProviderContext,
        position: PointUtf16,
        _: CompletionTrigger,
        _: &mut App,
    ) -> Task<anyhow::Result<CompletionList>> {
        Task::ready(Ok(CompletionList {
            items: vec![CompletionItem {
                range: Some(p(position.row, 0)..position),
                ..CompletionItem::new("hello", CompletionItemKind::Text)
            }],
            incomplete: false,
        }))
    }
}

/// `acceptSelectedSuggestion` doesn't test `tabMovesFocus`.
#[gpui::test]
fn tab_accepts_a_suggestion_in_tab_focus_mode(cx: &mut TestAppContext) {
    let (editor, second, cx) = setup("", tab_focus_mode, cx);
    cx.update(|_, cx| register_completion_provider(cx, LanguageSelector::All, Hello));
    cx.simulate_input("he");
    cx.simulate_keystrokes("ctrl-space");
    settle(cx);
    assert!(editor.read_with(cx, |e, _| e.is_suggest_visible()));
    cx.simulate_keystrokes("tab");
    settle(cx);
    assert_eq!(text(&editor, cx), "hello");
    assert!(focused(&editor, cx));
    // With nothing else to do, Tab moves the focus.
    cx.simulate_keystrokes("tab");
    assert!(focused(&second, cx));
    assert_eq!(text(&editor, cx), "hello");
}

/// `jumpToNextSnippetPlaceholder` and `jumpToPrevSnippetPlaceholder`
/// don't either.
#[gpui::test]
fn tab_moves_between_placeholders_in_tab_focus_mode(cx: &mut TestAppContext) {
    let (editor, second, cx) = setup("", tab_focus_mode, cx);
    editor.update(cx, |e, cx| {
        e.insert_snippet("${1:one} ${2:two}", SnippetInsertOptions::default(), cx)
    });
    settle(cx);
    assert_eq!(selection(&editor, cx), (p(0, 0), p(0, 3)));
    cx.simulate_keystrokes("tab");
    assert_eq!(selection(&editor, cx), (p(0, 4), p(0, 7)));
    cx.simulate_keystrokes("shift-tab");
    assert_eq!(selection(&editor, cx), (p(0, 0), p(0, 3)));
    // At the first placeholder Shift+Tab moves the focus back… to the
    // second editor (the focus cycles).
    cx.simulate_keystrokes("shift-tab");
    assert!(focused(&second, cx));
    assert_eq!(text(&editor, cx), "one two");
}

/// Completes the line with `hello world`.
struct Ghost;

impl InlineCompletionsProvider for Ghost {
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

/// `inlineSuggest.commit` requires `!tabMovesFocus`.
#[gpui::test]
fn tab_focus_mode_leaves_ghost_text(cx: &mut TestAppContext) {
    let (editor, second, cx) = setup("", tab_focus_mode, cx);
    cx.update(|_, cx| register_inline_completions_provider(cx, LanguageSelector::All, Ghost));
    cx.simulate_input("he");
    settle(cx);
    assert!(editor.read_with(cx, |e, cx| e.inline_suggestion_visible(cx)));
    cx.simulate_keystrokes("tab");
    assert!(focused(&second, cx));
    assert_eq!(text(&editor, cx), "he");
}

// ----- read-only editors (§H.4) --------------------------------------------

/// `tab` and `outdent` need `writable`: in a read-only editor the key moves
/// the focus, as the browser does, instead of trapping it.
#[gpui::test]
fn tab_moves_the_focus_out_of_read_only_editors(cx: &mut TestAppContext) {
    let (first, second, cx) = setup("one", |o| o.read_only = true, cx);
    cx.simulate_keystrokes("tab");
    assert!(focused(&second, cx));
    assert!(!first.read_with(cx, |e, _| e.is_message_visible()));
    cx.simulate_keystrokes("shift-tab");
    assert!(focused(&first, cx));
    assert_eq!(text(&first, cx), "one");
    assert_eq!(text(&second, cx), "one");
    // Run as commands, they do nothing, not even the read-only message.
    first.update_in(cx, |e, window, cx| {
        e.run_action("tab", window, cx);
        e.run_action("outdent", window, cx);
    });
    settle(cx);
    assert_eq!(text(&first, cx), "one");
    assert!(!first.read_with(cx, |e, _| e.is_message_visible()));
}

// ----- snippet choices (§H.5) --------------------------------------------------

/// Monaco's second `jumpToPrevSnippetPlaceholder` rule (`EditorContrib +
/// 91`) outweighs `acceptAlternativeSelectedSuggestion` (+90).
#[gpui::test]
fn shift_tab_in_a_snippet_choice_goes_back(cx: &mut TestAppContext) {
    let (editor, _, cx) = setup("", |_| {}, cx);
    editor.update(cx, |e, cx| {
        e.insert_snippet("${1:a} ${2|x,y|}", SnippetInsertOptions::default(), cx)
    });
    settle(cx);
    cx.simulate_keystrokes("tab");
    settle(cx);
    assert_eq!(selection(&editor, cx), (p(0, 2), p(0, 3)));
    assert!(editor.read_with(cx, |e, _| e.is_suggest_visible()));
    cx.simulate_keystrokes("shift-tab");
    settle(cx);
    assert_eq!(text(&editor, cx), "a x");
    assert_eq!(selection(&editor, cx), (p(0, 0), p(0, 1)));
}
