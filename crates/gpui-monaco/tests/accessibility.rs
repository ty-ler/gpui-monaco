//! Accessibility: tab focus mode (Monaco `toggleTabFocusMode`) and screen
//! reader announcements of inline suggestions (Monaco
//! `screenReaderAnnounceInlineSuggestion`).

use std::time::Duration;

use gpui::{
    App, AppContext, Context, Entity, FocusHandle, Focusable, IntoElement, ParentElement, Render,
    Styled, Task, TestAppContext, VisualTestContext, Window, div,
};
use gpui_monaco::CodeEditor;
use gpui_monaco::actions::ToggleTabFocusMode;
use gpui_monaco::cursor::Selection;
use gpui_monaco::options::AccessibilitySupport;
use gpui_monaco::providers::*;
use gpui_monaco::text::PointUtf16;

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

#[gpui::test]
fn tab_moves_focus_in_tab_focus_mode(cx: &mut TestAppContext) {
    cx.update(gpui_monaco::init);
    let (view, cx) = cx.add_window_view(|window, cx| TwoEditors {
        first: cx.new(|cx| CodeEditor::with_text("one", window, cx)),
        second: cx.new(|cx| CodeEditor::with_text("two", window, cx)),
    });
    let (first, second) = view.read_with(cx, |v, _| (v.first.clone(), v.second.clone()));
    let handle = |e: &Entity<CodeEditor>, cx: &mut VisualTestContext| -> FocusHandle {
        cx.update(|_, cx| e.focus_handle(cx))
    };
    let first_handle = handle(&first, cx);
    cx.update(|window, cx| window.focus(&first_handle, cx));
    cx.run_until_parked();

    // Normally Tab indents.
    cx.simulate_keystrokes("tab");
    assert_eq!(
        first.read_with(cx, |e, cx| e.model().read(cx).text()),
        "    one"
    );

    cx.dispatch_action(ToggleTabFocusMode);
    cx.update(|window, _| window.refresh());
    cx.run_until_parked();
    assert!(cx.update(|_, cx| gpui_monaco::tab_focus_mode(cx)));
    // Monaco alerts the new mode.
    assert_eq!(
        first.read_with(cx, |e, _| e.last_announcement_for_tests()),
        Some("Pressing Tab will now move focus to the next focusable element".into())
    );
    cx.simulate_keystrokes("tab");
    let second_handle = handle(&second, cx);
    assert!(cx.update(|window, _| second_handle.is_focused(window)));
    assert_eq!(
        first.read_with(cx, |e, cx| e.model().read(cx).text()),
        "    one"
    );
    cx.simulate_keystrokes("shift-tab");
    assert!(cx.update(|window, _| first_handle.is_focused(window)));
}

/// Completes the word before the cursor with `hello world`.
struct Hello;

impl InlineCompletionsProvider for Hello {
    fn provide_inline_completions(
        &self,
        context: &ProviderContext,
        position: PointUtf16,
        _: InlineCompletionContext,
        _: &mut App,
    ) -> Task<anyhow::Result<InlineCompletionList>> {
        let line = context.snapshot.line(position.row);
        let items = (!line.is_empty() && "hello world".starts_with(&line))
            .then(|| InlineCompletion {
                insert_text: "hello world".into(),
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
}

fn announcement(editor: &Entity<CodeEditor>, cx: &mut VisualTestContext) -> Option<String> {
    cx.update(|window, _| window.refresh());
    cx.run_until_parked();
    editor.read_with(cx, |e, _| {
        e.last_announcement_for_tests().map(|m| m.to_string())
    })
}

#[gpui::test]
fn screen_readers_hear_inline_suggestions(cx: &mut TestAppContext) {
    cx.update(|cx| {
        gpui_monaco::init(cx);
        register_inline_completions_provider(cx, LanguageSelector::All, Hello);
    });
    let (editor, cx) = cx.add_window_view(|window, cx| {
        let mut editor = CodeEditor::with_text("", window, cx);
        let mut options = editor.options().clone();
        options.quick_suggestions = gpui_monaco::options::QuickSuggestions::OFF;
        options.word_based_suggestions = gpui_monaco::options::WordBasedSuggestions::Off;
        options.accessibility_support = AccessibilitySupport::On;
        editor.update_options(options, cx);
        editor
    });
    cx.update(|window, cx| window.focus(&editor.focus_handle(cx), cx));
    cx.simulate_input("he");
    cx.executor().advance_clock(Duration::from_millis(100));
    // Monaco `renderForScreenReader`: the ghost text.
    assert_eq!(announcement(&editor, cx).as_deref(), Some("llo world"));
    // Only once per suggestion.
    editor.update(cx, |e, cx| e.announce("other", cx));
    assert_eq!(announcement(&editor, cx).as_deref(), Some("other"));

    // Monaco `screenReaderAnnounceInlineSuggestion: false`.
    cx.simulate_keystrokes("escape");
    editor.update(cx, |e, cx| {
        let mut options = e.options().clone();
        options.screen_reader_announce_inline_suggestion = false;
        e.update_options(options, cx);
    });
    cx.simulate_input("l");
    cx.executor().advance_clock(Duration::from_millis(100));
    assert_eq!(announcement(&editor, cx).as_deref(), Some("other"));

    // Nothing without a screen reader.
    cx.simulate_keystrokes("escape");
    editor.update(cx, |e, cx| {
        let mut options = e.options().clone();
        options.screen_reader_announce_inline_suggestion = true;
        options.accessibility_support = AccessibilitySupport::Off;
        e.update_options(options, cx);
    });
    cx.simulate_input("l");
    cx.executor().advance_clock(Duration::from_millis(100));
    assert_eq!(announcement(&editor, cx).as_deref(), Some("other"));
    assert!(!editor.read_with(cx, |e, _| e.is_screen_reader_optimized()));
}

fn screen_reader_editor<'a>(
    text: &str,
    page_size: u32,
    cx: &'a mut TestAppContext,
) -> (Entity<CodeEditor>, &'a mut VisualTestContext) {
    cx.update(gpui_monaco::init);
    let text = text.to_string();
    let (editor, cx) = cx.add_window_view(move |window, cx| {
        let mut editor = CodeEditor::with_text(&text, window, cx);
        let mut options = editor.options().clone();
        options.accessibility_page_size = page_size;
        options.accessibility_support = AccessibilitySupport::On;
        editor.update_options(options, cx);
        editor
    });
    cx.update(|window, cx| window.focus(&editor.focus_handle(cx), cx));
    cx.run_until_parked();
    (editor, cx)
}

fn content(editor: &Entity<CodeEditor>, cx: &mut VisualTestContext) -> (String, usize, usize) {
    editor.update(cx, |e, _| e.screen_reader_content_for_tests())
}

fn select(editor: &Entity<CodeEditor>, cx: &mut VisualTestContext, points: &[(u32, u32)]) {
    let selections: Vec<Selection> = points
        .iter()
        .map(|&(row, column)| Selection::collapsed(PointUtf16::new(row, column)))
        .collect();
    editor.update(cx, |e, cx| e.set_selections(&selections, cx));
}

/// Monaco `SimplePagedScreenReaderStrategy`: Select All in a long file
/// reads the first and last pages, with `…` for the lines between, not
/// every line.
#[gpui::test]
fn screen_reader_content_is_at_most_two_pages(cx: &mut TestAppContext) {
    let text: String = (1..=1000).map(|i| format!("L{i}\n")).collect();
    let (editor, cx) = screen_reader_editor(text.trim_end(), 10, cx);
    let (value, start, end) = content(&editor, cx);
    assert_eq!(value, "L1\nL2\nL3\nL4\nL5\nL6\nL7\nL8\nL9\nL10\n");
    assert_eq!((start, end), (0, 0));
    cx.simulate_keystrokes("cmd-a");
    let (value, start, end) = content(&editor, cx);
    assert_eq!(value.lines().count(), 20);
    assert!(value.starts_with("L1\n"));
    assert!(value.contains("L10\n\u{2026}L991\n"));
    assert!(value.ends_with("L1000"));
    assert_eq!((start, end), (0, value.chars().count()));
}

/// Monaco reads the view model: folded lines are left out and a wrapped
/// line is a line per segment.
#[gpui::test]
fn screen_reader_content_reads_view_lines(cx: &mut TestAppContext) {
    let (editor, cx) = screen_reader_editor("a\nb\nc\nd\ne", 10, cx);
    editor.update(cx, |e, cx| {
        e.set_hidden_areas(std::iter::once(1..3).collect(), cx)
    });
    assert_eq!(content(&editor, cx).0, "a\nd\ne");
    editor.update(cx, |e, cx| {
        e.set_hidden_areas(Vec::new(), cx);
        e.model()
            .update(cx, |m, cx| m.set_value("x\naaaa bbbb cccc dddd\ny", cx));
        let mut options = e.options().clone();
        options.word_wrap = gpui_monaco::options::WordWrap::WordWrapColumn;
        options.word_wrap_column = 10;
        e.update_options(options, cx);
    });
    cx.run_until_parked();
    cx.update(|window, cx| {
        window.refresh();
        window.draw(cx).clear(cx);
    });
    // The cursor after `c` of `cccc`, on the wrapped line's second segment.
    select(&editor, cx, &[(1, 11)]);
    let (value, start, _) = content(&editor, cx);
    let lines: Vec<&str> = value.split('\n').collect();
    assert!(lines.len() > 3, "{value:?}");
    assert!(!lines.contains(&"aaaa bbbb cccc dddd"), "{value:?}");
    let before: String = value.chars().take(start).collect();
    assert!(before.ends_with("\nc"), "{before:?}");
}

/// Monaco's `aria.status` messages are polite and its `alert` ones
/// assertive.
#[gpui::test]
fn status_announcements_are_polite(cx: &mut TestAppContext) {
    let (editor, cx) = screen_reader_editor("one\ntwo", 10, cx);
    let polite = |editor: &Entity<CodeEditor>, cx: &mut VisualTestContext| {
        editor.read_with(cx, |e, _| e.last_announcement_is_status_for_tests())
    };
    editor.update(cx, |e, cx| e.announce("alert", cx));
    assert!(!polite(&editor, cx));
    editor.update(cx, |e, cx| e.announce_status("status", cx));
    assert!(polite(&editor, cx));
    assert_eq!(announcement(&editor, cx).as_deref(), Some("status"));
    // Monaco `removeSecondaryCursors`'s `status`.
    select(&editor, cx, &[(0, 0), (1, 0)]);
    cx.simulate_keystrokes("escape");
    assert_eq!(
        announcement(&editor, cx).as_deref(),
        Some("Removed secondary cursors")
    );
    assert!(polite(&editor, cx));
    // Monaco `SelectionAnchorController`'s `alert`.
    select(&editor, cx, &[(1, 2)]);
    editor.update(cx, |e, cx| e.set_selection_anchor(cx));
    assert_eq!(
        announcement(&editor, cx).as_deref(),
        Some("Anchor set at 2:3")
    );
    assert!(!polite(&editor, cx));
}
