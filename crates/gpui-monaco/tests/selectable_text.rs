//! Selecting text in markdown widgets (#306): Monaco's hovers are DOM, so
//! the browser lets their text be selected and copied.

use std::time::Duration;

use gpui::{
    App, AppContext as _, Entity, Focusable as _, Modifiers, MouseButton, Point, Task,
    TestAppContext, VisualTestContext, point, px,
};
use gpui_monaco::cursor::Selection;
use gpui_monaco::providers::*;
use gpui_monaco::text::PointUtf16;
use gpui_monaco::{CodeEditor, TextModel};

struct Docs;
impl HoverProvider for Docs {
    fn provide_hover(
        &self,
        _: &ProviderContext,
        _: PointUtf16,
        _: &mut App,
    ) -> Task<anyhow::Result<Option<Hover>>> {
        Task::ready(Ok(Some(Hover {
            contents: vec!["alpha beta gamma".into(), "```\nsecond block\n```".into()],
            ..Default::default()
        })))
    }
}

fn paint(cx: &mut VisualTestContext) {
    cx.update(|window, _| window.refresh());
    cx.run_until_parked();
}

fn setup(cx: &mut TestAppContext) -> (Entity<CodeEditor>, &mut VisualTestContext) {
    cx.update(|cx| {
        gpui_monaco::init(cx);
        register_hover_provider(cx, LanguageSelector::All, Docs);
    });
    let (editor, cx) = cx.add_window_view(|window, cx| {
        let model = cx.new(|_| TextModel::new("word one\nword two\n"));
        CodeEditor::new(model, window, cx)
    });
    cx.update(|window, cx| window.focus(&editor.focus_handle(cx), cx));
    cx.run_until_parked();
    paint(cx);
    editor.update(cx, |e, cx| {
        e.set_selections(&[Selection::collapsed(PointUtf16::new(0, 1))], cx);
        e.show_hover_at_cursor(cx);
    });
    cx.executor().advance_clock(Duration::from_millis(500));
    cx.run_until_parked();
    paint(cx);
    assert!(editor.read_with(cx, |e, _| e.is_hover_visible()));
    (editor, cx)
}

fn drag(cx: &mut VisualTestContext, from: Point<gpui::Pixels>, to: Point<gpui::Pixels>) {
    cx.simulate_mouse_move(from, None, Modifiers::default());
    cx.simulate_mouse_down(from, MouseButton::Left, Modifiers::default());
    paint(cx);
    cx.simulate_mouse_move(to, MouseButton::Left, Modifiers::default());
    cx.simulate_mouse_up(to, MouseButton::Left, Modifiers::default());
    paint(cx);
}

fn copied(cx: &mut VisualTestContext) -> Option<String> {
    cx.simulate_keystrokes("secondary-c");
    cx.read_from_clipboard().and_then(|item| item.text())
}

#[gpui::test]
fn dragging_over_hover_text_selects_it_for_copy(cx: &mut TestAppContext) {
    let (editor, cx) = setup(cx);
    let card = editor
        .read_with(cx, |e, cx| e.hover_card_bounds_for_tests(cx))
        .expect("painted");
    // From the start of the first content to far right of it, in its first
    // line (contents are padded 8px by 4px).
    let line = card.top() + px(4.) + px(6.);
    drag(
        cx,
        point(card.left() + px(10.), line),
        point(card.right() - px(2.), line),
    );
    assert_eq!(copied(cx).as_deref(), Some("alpha beta gamma"));

    // Down into the code block: both blocks, a line break between.
    drag(
        cx,
        point(card.left() + px(10.), line),
        point(card.right() - px(2.), card.bottom() - px(6.)),
    );
    assert_eq!(
        copied(cx).as_deref(),
        Some("alpha beta gamma\nsecond block")
    );

    // A click elsewhere clears it: copy is the editor's again (the line
    // clicked).
    let outside = point(card.left() + px(10.), card.bottom() + px(40.));
    cx.simulate_mouse_down(outside, MouseButton::Left, Modifiers::default());
    cx.simulate_mouse_up(outside, MouseButton::Left, Modifiers::default());
    paint(cx);
    let copied = copied(cx).unwrap_or_default();
    assert!(!copied.contains("alpha"), "{copied:?}");
}

#[gpui::test]
fn double_click_selects_a_word(cx: &mut TestAppContext) {
    let (editor, cx) = setup(cx);
    let card = editor
        .read_with(cx, |e, cx| e.hover_card_bounds_for_tests(cx))
        .expect("painted");
    let at = point(card.left() + px(10.), card.top() + px(10.));
    cx.simulate_mouse_move(at, None, Modifiers::default());
    cx.simulate_mouse_down(at, MouseButton::Left, Modifiers::default());
    cx.simulate_mouse_up(at, MouseButton::Left, Modifiers::default());
    cx.simulate_event(gpui::MouseDownEvent {
        button: MouseButton::Left,
        position: at,
        modifiers: Modifiers::default(),
        click_count: 2,
        first_mouse: false,
    });
    cx.simulate_mouse_up(at, MouseButton::Left, Modifiers::default());
    paint(cx);
    assert_eq!(copied(cx).as_deref(), Some("alpha"));
}
