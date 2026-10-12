//! Monaco's `wordSegmenterLocales`: word navigation, deletion and
//! double-click selection find Japanese, Chinese and Thai words with a word
//! segmenter (needs the `word-segmenter` cargo feature).

#![cfg(feature = "word-segmenter")]

use gpui::{
    AppContext as _, Entity, Focusable, Modifiers, MouseButton, MouseDownEvent, TestAppContext,
    VisualTestContext, point, px,
};
use gpui_monaco::actions::{
    CursorWordLeft, CursorWordRight, CursorWordRightSelect, DeleteInsideWord, DeleteWordLeft,
    DeleteWordRight, TransformToUppercase,
};
use gpui_monaco::cursor::Selection;
use gpui_monaco::text::PointUtf16;
use gpui_monaco::{CodeEditor, TextModel};

fn setup<'a>(
    text: &str,
    locales: &[&str],
    cx: &'a mut TestAppContext,
) -> (Entity<CodeEditor>, &'a mut VisualTestContext) {
    cx.update(gpui_monaco::init);
    let text = text.to_string();
    let (editor, cx) = cx.add_window_view(|window, cx| {
        let model = cx.new(|_| TextModel::new(&text));
        CodeEditor::new(model, window, cx)
    });
    set_locales(&editor, locales, cx);
    cx.update(|window, cx| window.focus(&editor.focus_handle(cx), cx));
    paint(cx);
    (editor, cx)
}

fn set_locales(editor: &Entity<CodeEditor>, locales: &[&str], cx: &mut VisualTestContext) {
    editor.update(cx, |e, cx| {
        let mut options = e.options().clone();
        options.word_segmenter_locales = locales.iter().map(|l| l.to_string().into()).collect();
        e.update_options(options, cx);
    });
    paint(cx);
}

fn paint(cx: &mut VisualTestContext) {
    cx.update(|window, _| window.refresh());
    cx.run_until_parked();
}

fn set_cursor(editor: &Entity<CodeEditor>, cx: &mut VisualTestContext, column: u32) {
    editor.update(cx, |e, cx| {
        e.set_selections(&[Selection::collapsed(PointUtf16::new(0, column))], cx)
    });
}

fn selection(editor: &Entity<CodeEditor>, cx: &mut VisualTestContext) -> (u32, u32) {
    let s = editor.read_with(cx, |e, _| e.selections()[0]);
    (s.anchor.column, s.head.column)
}

fn text(editor: &Entity<CodeEditor>, cx: &mut VisualTestContext) -> String {
    editor.read_with(cx, |e, cx| e.model().read(cx).text())
}

/// The cursor columns `action` stops at, starting at `from`, until it stops
/// moving.
fn stops(
    editor: &Entity<CodeEditor>,
    cx: &mut VisualTestContext,
    from: u32,
    action: impl gpui::Action + Clone,
) -> Vec<u32> {
    set_cursor(editor, cx, from);
    let mut stops = Vec::new();
    loop {
        cx.dispatch_action(action.clone());
        let (_, head) = selection(editor, cx);
        if stops.last() == Some(&head) || (stops.is_empty() && head == from) {
            return stops;
        }
        stops.push(head);
    }
}

#[gpui::test]
fn word_movement_finds_japanese_words(cx: &mut TestAppContext) {
    // `cursorWordLeft/Right - Recognize words`.
    let (editor, cx) = setup("/* これはテストです /*", &["ja"], cx);
    assert_eq!(stops(&editor, cx, 0, CursorWordRight), [2, 5, 6, 9, 11, 14]);
    assert_eq!(stops(&editor, cx, 14, CursorWordLeft), [12, 9, 6, 5, 3, 0]);
    // Without locales the Japanese run is one word.
    set_locales(&editor, &[], cx);
    assert_eq!(stops(&editor, cx, 0, CursorWordRight), [2, 11, 14]);
}

#[gpui::test]
fn word_movement_finds_chinese_and_thai_words(cx: &mut TestAppContext) {
    let (editor, cx) = setup("我们是学生", &["zh-CN"], cx);
    assert_eq!(stops(&editor, cx, 0, CursorWordRight), [2, 3, 5]);
    let (editor, cx) = setup("ทุกสองสัปดาห์", &["th"], cx);
    assert_eq!(stops(&editor, cx, 0, CursorWordRight), [3, 6, 13]);
}

#[gpui::test]
fn invalid_locales_are_ignored(cx: &mut TestAppContext) {
    let (editor, cx) = setup("これはテストです", &["", "not a locale!"], cx);
    assert_eq!(stops(&editor, cx, 0, CursorWordRight), [8]);
    set_locales(&editor, &["", "ja"], cx);
    assert_eq!(stops(&editor, cx, 0, CursorWordRight), [2, 3, 6, 8]);
}

#[gpui::test]
fn word_deletion_finds_japanese_words(cx: &mut TestAppContext) {
    let (editor, cx) = setup("これはテストです", &["ja"], cx);
    set_cursor(&editor, cx, 8);
    cx.dispatch_action(DeleteWordLeft);
    assert_eq!(text(&editor, cx), "これはテスト");
    cx.dispatch_action(DeleteWordLeft);
    assert_eq!(text(&editor, cx), "これは");
    set_cursor(&editor, cx, 0);
    cx.dispatch_action(DeleteWordRight);
    assert_eq!(text(&editor, cx), "は");

    editor.update(cx, |e, cx| {
        e.model()
            .update(cx, |m, cx| m.set_value("これはテストです", cx))
    });
    set_cursor(&editor, cx, 4);
    cx.dispatch_action(DeleteInsideWord::default());
    assert_eq!(text(&editor, cx), "これはです");
}

#[gpui::test]
fn selection_and_word_at_position_find_japanese_words(cx: &mut TestAppContext) {
    let (editor, cx) = setup("これはテストです", &["ja"], cx);
    set_cursor(&editor, cx, 0);
    cx.dispatch_action(CursorWordRightSelect);
    cx.dispatch_action(CursorWordRightSelect);
    assert_eq!(selection(&editor, cx), (0, 3));

    // Double click.
    let p = editor.read_with(cx, |e, _| {
        e.scrolled_visible_position(PointUtf16::new(0, 4)).unwrap()
    });
    let position = point(p.x + px(1.), p.y + px(5.));
    for click_count in 1..=2 {
        cx.simulate_event(MouseDownEvent {
            position,
            button: MouseButton::Left,
            modifiers: Modifiers::default(),
            click_count,
            first_mouse: false,
        });
        cx.simulate_mouse_up(position, MouseButton::Left, Modifiers::default());
    }
    assert_eq!(selection(&editor, cx), (3, 6));

    // `getConfiguredWordAtPosition` (case transforms of an empty selection).
    editor.update(cx, |e, cx| {
        e.model()
            .update(cx, |m, cx| m.set_value("ｗｏｒｄ字です", cx))
    });
    set_cursor(&editor, cx, 1);
    cx.dispatch_action(TransformToUppercase);
    assert_eq!(text(&editor, cx), "ＷＯＲＤ字です");
}
