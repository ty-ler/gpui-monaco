//! Linked editing (Monaco `contrib/linkedEditing`): HTML tags from the
//! syntax tree and mock providers.

use std::time::Duration;

use gpui::{App, AppContext as _, Entity, Focusable, Task, TestAppContext, VisualTestContext};
use gpui_monaco::cursor::Selection;
use gpui_monaco::providers::*;
use gpui_monaco::text::PointUtf16;
use gpui_monaco::{CodeEditor, TextModel};

fn setup<'a>(
    text: &str,
    language: &str,
    cx: &'a mut TestAppContext,
) -> (Entity<CodeEditor>, &'a mut VisualTestContext) {
    cx.update(|cx| {
        gpui_monaco::init(cx);
        gpui_monaco::set_language_registry(cx, gpui_monaco_languages::registry());
    });
    let text = text.to_string();
    let language = language.to_string();
    let (editor, cx) = cx.add_window_view(|window, cx| {
        let model = cx.new(|_| TextModel::new(&text).with_language(language));
        let mut editor = CodeEditor::new(model, window, cx);
        let mut options = editor.options().clone();
        options.linked_editing = true;
        options.quick_suggestions = gpui_monaco::options::QuickSuggestions::OFF;
        options.word_based_suggestions = gpui_monaco::options::WordBasedSuggestions::Off;
        editor.update_options(options, cx);
        editor
    });
    cx.update(|window, cx| window.focus(&editor.focus_handle(cx), cx));
    cx.run_until_parked();
    (editor, cx)
}

fn settle(cx: &mut VisualTestContext) {
    cx.executor().advance_clock(Duration::from_millis(300));
    cx.run_until_parked();
}

fn move_to(editor: &Entity<CodeEditor>, cx: &mut VisualTestContext, row: u32, column: u32) {
    editor.update(cx, |e, cx| {
        e.set_selections(&[Selection::collapsed(PointUtf16::new(row, column))], cx)
    });
    settle(cx);
}

fn text(editor: &Entity<CodeEditor>, cx: &mut VisualTestContext) -> String {
    editor.read_with(cx, |e, cx| e.model().read(cx).text())
}

fn ranges(editor: &Entity<CodeEditor>, cx: &mut VisualTestContext) -> Vec<(u32, u32)> {
    editor.read_with(cx, |e, cx| {
        e.linked_editing_ranges_for_tests(cx)
            .into_iter()
            .map(|r| (r.start.column, r.end.column))
            .collect()
    })
}

#[gpui::test]
fn renaming_an_html_start_tag_renames_its_end_tag(cx: &mut TestAppContext) {
    let (editor, cx) = setup("<div>hi</div>", "html", cx);
    move_to(&editor, cx, 0, 2);
    // The tag at the cursor first.
    assert_eq!(ranges(&editor, cx), vec![(1, 4), (9, 12)]);
    cx.simulate_input("x");
    cx.run_until_parked();
    assert_eq!(text(&editor, cx), "<dxiv>hi</dxiv>");
    // At the end of the name too.
    cx.simulate_keystrokes("right right");
    cx.simulate_input("s");
    cx.run_until_parked();
    assert_eq!(text(&editor, cx), "<dxivs>hi</dxivs>");
    // Each edit and its mirror are one undo step.
    editor.update(cx, |e, cx| e.undo(cx));
    assert_eq!(text(&editor, cx), "<dxiv>hi</dxiv>");
    editor.update(cx, |e, cx| e.undo(cx));
    assert_eq!(text(&editor, cx), "<div>hi</div>");
}

#[gpui::test]
fn the_end_tag_renames_the_start_tag(cx: &mut TestAppContext) {
    let (editor, cx) = setup("<p>a</p>", "html", cx);
    move_to(&editor, cx, 0, 8 - 1);
    cx.simulate_keystrokes("backspace");
    cx.run_until_parked();
    assert_eq!(text(&editor, cx), "<>a</>");
    cx.simulate_input("em");
    cx.run_until_parked();
    assert_eq!(text(&editor, cx), "<em>a</em>");
    // The cursor stays in the end tag.
    assert_eq!(
        editor.read_with(cx, |e, _| e.selections()[0].head),
        PointUtf16::new(0, 9)
    );
}

#[gpui::test]
fn off_by_default_and_escape_ends_it(cx: &mut TestAppContext) {
    let (editor, cx) = setup("<b>x</b>", "html", cx);
    editor.update(cx, |e, cx| {
        let mut options = e.options().clone();
        options.linked_editing = false;
        e.update_options(options, cx);
    });
    move_to(&editor, cx, 0, 2);
    assert!(ranges(&editor, cx).is_empty());
    // The command works anyway.
    cx.simulate_keystrokes("cmd-shift-f2");
    cx.run_until_parked();
    assert_eq!(ranges(&editor, cx).len(), 2);
    cx.simulate_keystrokes("escape");
    assert!(ranges(&editor, cx).is_empty());
}

#[gpui::test]
fn non_word_characters_end_it(cx: &mut TestAppContext) {
    let (editor, cx) = setup("<div></div>", "html", cx);
    move_to(&editor, cx, 0, 4);
    cx.simulate_input(" ");
    cx.run_until_parked();
    // A space starts attributes: the end tag is left alone.
    assert_eq!(text(&editor, cx), "<div ></div>");
}

/// Links every `$x` to the others.
struct Dollars;
impl LinkedEditingRangeProvider for Dollars {
    fn provide_linked_editing_ranges(
        &self,
        context: &ProviderContext,
        _: PointUtf16,
        _: &mut App,
    ) -> Task<anyhow::Result<Option<LinkedEditingRanges>>> {
        let line = context.snapshot.line(0);
        let ranges = line
            .match_indices("$x")
            .map(|(ix, _)| PointUtf16::new(0, ix as u32 + 1)..PointUtf16::new(0, ix as u32 + 2))
            .collect();
        Task::ready(Ok(Some(LinkedEditingRanges {
            ranges,
            word_pattern: None,
        })))
    }
}

#[gpui::test]
fn providers_supply_ranges(cx: &mut TestAppContext) {
    cx.update(|cx| {
        gpui_monaco::init(cx);
        register_linked_editing_range_provider(cx, LanguageSelector::All, Dollars);
    });
    let (editor, cx) = setup("$x = $x + $x", "plaintext", cx);
    move_to(&editor, cx, 0, 6);
    assert_eq!(ranges(&editor, cx), vec![(6, 7), (1, 2), (11, 12)]);
    cx.simulate_input("y");
    cx.run_until_parked();
    assert_eq!(text(&editor, cx), "$yx = $yx + $yx");
}
