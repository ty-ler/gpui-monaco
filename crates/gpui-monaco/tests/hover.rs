//! Hover options and keyboard focus (Monaco `contrib/hover`).

use std::time::Duration;

use gpui::{
    App, AppContext as _, Entity, Focusable as _, Modifiers, TestAppContext, VisualTestContext, px,
};
use gpui_monaco::options::{HoverEnabled, HoverOptions};
use gpui_monaco::providers::*;
use gpui_monaco::text::PointUtf16;
use gpui_monaco::{CodeEditor, DecorationOptions, TextModel};

/// A hover with `lines` lines of text for every word.
struct LongHover(usize);
impl HoverProvider for LongHover {
    fn provide_hover(
        &self,
        _: &ProviderContext,
        _: PointUtf16,
        _: &mut App,
    ) -> gpui::Task<anyhow::Result<Option<Hover>>> {
        let text: String = (0..self.0).map(|i| format!("line {i}\n\n")).collect();
        gpui::Task::ready(Ok(Some(Hover {
            contents: vec![text.into()],
            range: None,
        })))
    }
}

fn setup(
    hover: HoverOptions,
    lines: usize,
    cx: &mut TestAppContext,
) -> (Entity<CodeEditor>, &mut VisualTestContext) {
    cx.update(gpui_monaco::init);
    cx.update(|cx| register_hover_provider(cx, LanguageSelector::All, LongHover(lines)));
    let (editor, cx) = cx.add_window_view(|window, cx| {
        let model = cx.new(|_| TextModel::new("alpha beta gamma\ndelta\nepsilon\n"));
        let mut editor = CodeEditor::new(model, window, cx);
        let mut options = editor.options().clone();
        options.hover = hover;
        editor.update_options(options, cx);
        editor
    });
    cx.update(|window, cx| window.focus(&editor.read(cx).focus_handle(cx), cx));
    cx.run_until_parked();
    paint(cx);
    (editor, cx)
}

fn paint(cx: &mut VisualTestContext) {
    cx.update(|window, _| window.refresh());
    cx.run_until_parked();
}

fn mouse_at(
    editor: &Entity<CodeEditor>,
    cx: &mut VisualTestContext,
    row: u32,
    column: u32,
) -> gpui::Point<gpui::Pixels> {
    let origin = editor
        .read_with(cx, |e, _| {
            e.scrolled_visible_position(PointUtf16::new(row, column))
        })
        .expect("laid out");
    gpui::point(origin.x + px(1.), origin.y + px(2.))
}

fn visible(editor: &Entity<CodeEditor>, cx: &mut VisualTestContext) -> bool {
    editor.read_with(cx, |e, _| e.is_hover_visible())
}

fn rest(cx: &mut VisualTestContext, ms: u64) {
    cx.executor().advance_clock(Duration::from_millis(ms));
    cx.run_until_parked();
    paint(cx);
}

#[gpui::test]
fn disabled_hovers_and_the_delay(cx: &mut TestAppContext) {
    let (editor, cx) = setup(
        HoverOptions {
            enabled: HoverEnabled::Off,
            ..Default::default()
        },
        1,
        cx,
    );
    let p = mouse_at(&editor, cx, 0, 7);
    cx.simulate_mouse_move(p, None, Modifiers::default());
    rest(cx, 1000);
    assert!(!visible(&editor, cx));
    // Show or Focus Hover still works.
    cx.dispatch_action(gpui_monaco::actions::ShowHover);
    rest(cx, 0);
    assert!(visible(&editor, cx));
}

#[gpui::test]
fn the_hover_delay(cx: &mut TestAppContext) {
    let (editor, cx) = setup(
        HoverOptions {
            delay: Duration::from_millis(1000),
            ..Default::default()
        },
        1,
        cx,
    );
    let p = mouse_at(&editor, cx, 0, 7);
    cx.simulate_mouse_move(p, None, Modifiers::default());
    rest(cx, 400);
    assert!(!visible(&editor, cx));
    rest(cx, 700);
    assert!(visible(&editor, cx));
}

#[gpui::test]
fn sticky_and_hiding_delay(cx: &mut TestAppContext) {
    // Not sticky: moving onto the card hides it.
    let (editor, cx) = setup(
        HoverOptions {
            sticky: false,
            ..Default::default()
        },
        1,
        cx,
    );
    let p = mouse_at(&editor, cx, 0, 7);
    cx.simulate_mouse_move(p, None, Modifiers::default());
    rest(cx, 400);
    assert!(visible(&editor, cx));
    let card = editor
        .read_with(cx, |e, cx| e.hover_card_bounds_for_tests(cx))
        .expect("painted");
    cx.simulate_mouse_move(card.center(), None, Modifiers::default());
    rest(cx, 0);
    assert!(!visible(&editor, cx));
}

#[gpui::test]
fn sticky_hovers_wait_for_the_mouse(cx: &mut TestAppContext) {
    let (editor, cx) = setup(HoverOptions::default(), 1, cx);
    let p = mouse_at(&editor, cx, 0, 7);
    cx.simulate_mouse_move(p, None, Modifiers::default());
    rest(cx, 400);
    assert!(visible(&editor, cx));
    // Leaving the word: the hover waits `hiding_delay` for the mouse...
    let other = mouse_at(&editor, cx, 2, 2);
    cx.simulate_mouse_move(other, None, Modifiers::default());
    rest(cx, 100);
    assert!(visible(&editor, cx));
    // ...which reaches the card, so it stays.
    let card = editor
        .read_with(cx, |e, cx| e.hover_card_bounds_for_tests(cx))
        .expect("painted");
    cx.simulate_mouse_move(card.center(), None, Modifiers::default());
    rest(cx, 1000);
    assert!(visible(&editor, cx));
    // Elsewhere for longer than the delay: it goes.
    cx.simulate_mouse_move(other, None, Modifiers::default());
    rest(cx, 350);
    assert!(!visible(&editor, cx));
}

#[gpui::test]
fn focusing_and_scrolling_the_hover(cx: &mut TestAppContext) {
    let (editor, cx) = setup(HoverOptions::default(), 40, cx);
    // Cmd/Ctrl+K Cmd/Ctrl+I shows the hover, then focuses it.
    cx.dispatch_action(gpui_monaco::actions::ShowHover);
    rest(cx, 0);
    assert!(visible(&editor, cx));
    cx.dispatch_action(gpui_monaco::actions::ShowHover);
    rest(cx, 0);
    assert!(cx.update(|window, cx| editor.read(cx).is_hover_focused(window, cx)));
    let offset = |cx: &mut VisualTestContext| {
        editor
            .read_with(cx, |e, cx| e.hover_scroll_offset_for_tests(cx))
            .unwrap()
            .y
    };
    assert_eq!(offset(cx), px(0.));
    cx.simulate_keystrokes("down");
    rest(cx, 0);
    assert!(offset(cx) < px(0.));
    cx.simulate_keystrokes("end");
    rest(cx, 0);
    let bottom = offset(cx);
    cx.simulate_keystrokes("home");
    rest(cx, 0);
    assert_eq!(offset(cx), px(0.));
    assert!(bottom < px(-100.));
    // Escape closes it and returns to the editor.
    cx.simulate_keystrokes("escape");
    rest(cx, 0);
    assert!(!visible(&editor, cx));
    assert!(cx.update(|window, cx| editor.read(cx).focus_handle(cx).is_focused(window)));
}

#[gpui::test]
fn hovers_on_keyboard_modifier(cx: &mut TestAppContext) {
    let (editor, cx) = setup(
        HoverOptions {
            enabled: HoverEnabled::OnKeyboardModifier,
            ..Default::default()
        },
        1,
        cx,
    );
    let p = mouse_at(&editor, cx, 0, 7);
    cx.simulate_mouse_move(p, None, Modifiers::default());
    rest(cx, 1000);
    assert!(!visible(&editor, cx));
    // The modifier opposite Alt (the default multi-cursor one).
    cx.simulate_modifiers_change(Modifiers::secondary_key());
    rest(cx, 400);
    assert!(visible(&editor, cx));
}

#[gpui::test]
fn glyph_margin_hovers(cx: &mut TestAppContext) {
    let (editor, cx) = setup(HoverOptions::default(), 1, cx);
    let model = editor.read_with(cx, |e, _| e.model().clone());
    let id = editor.entity_id();
    model.update(cx, |m, cx| {
        m.delta_decorations(
            Some(id),
            &[],
            vec![(
                17..17,
                DecorationOptions {
                    glyph: Some(("●".into(), gpui::red())),
                    glyph_margin_hover_message: Some("Breakpoint".into()),
                    ..Default::default()
                },
            )],
            cx,
        );
    });
    paint(cx);
    let row = mouse_at(&editor, cx, 1, 0);
    cx.simulate_mouse_move(gpui::point(px(4.), row.y), None, Modifiers::default());
    rest(cx, 400);
    assert!(visible(&editor, cx));
}
