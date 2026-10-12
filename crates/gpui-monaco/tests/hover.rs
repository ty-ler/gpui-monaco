//! Hover options and keyboard focus (Monaco `contrib/hover`).

use std::time::Duration;

use gpui::{
    App, AppContext as _, Entity, Focusable as _, Modifiers, TestAppContext, VisualTestContext, px,
};
use gpui_monaco::cursor::Selection;
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
            ..Default::default()
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
    cx.dispatch_action(gpui_monaco::actions::ShowHover::default());
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
    cx.dispatch_action(gpui_monaco::actions::ShowHover::default());
    rest(cx, 0);
    assert!(visible(&editor, cx));
    cx.dispatch_action(gpui_monaco::actions::ShowHover::default());
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
                    glyph_margin_hover_message: vec!["Breakpoint".into()],
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

/// A hover whose detail goes from 0 to 2 (Monaco's verbosity levels),
/// recording the requests' deltas.
struct LevelHover(std::rc::Rc<std::cell::RefCell<Vec<i32>>>);
impl HoverProvider for LevelHover {
    fn provide_hover(
        &self,
        context: &ProviderContext,
        position: PointUtf16,
        cx: &mut App,
    ) -> gpui::Task<anyhow::Result<Option<Hover>>> {
        self.provide_hover_with_context(context, position, &HoverContext::default(), cx)
    }

    fn provide_hover_with_context(
        &self,
        _: &ProviderContext,
        _: PointUtf16,
        context: &HoverContext,
        _: &mut App,
    ) -> gpui::Task<anyhow::Result<Option<Hover>>> {
        let level = match &context.verbosity_request {
            Some(request) => {
                self.0.borrow_mut().push(request.verbosity_delta);
                let previous: i32 = request.previous_hover.contents[0]
                    .value
                    .trim_start_matches("level ")
                    .parse()
                    .unwrap();
                (previous + request.verbosity_delta).clamp(0, 2)
            }
            None => 0,
        };
        gpui::Task::ready(Ok(Some(Hover {
            contents: vec![format!("level {level}").into()],
            range: None,
            can_increase_verbosity: level < 2,
            can_decrease_verbosity: level > 0,
        })))
    }
}

fn setup_with<'a>(
    text: &str,
    provider: impl HoverProvider + 'static,
    cx: &'a mut TestAppContext,
) -> (Entity<CodeEditor>, &'a mut VisualTestContext) {
    cx.update(gpui_monaco::init);
    cx.update(|cx| register_hover_provider(cx, LanguageSelector::All, provider));
    let text = text.to_string();
    let (editor, cx) = cx.add_window_view(|window, cx| {
        let model = cx.new(|_| TextModel::new(&text));
        CodeEditor::new(model, window, cx)
    });
    cx.update(|window, cx| window.focus(&editor.read(cx).focus_handle(cx), cx));
    cx.run_until_parked();
    paint(cx);
    (editor, cx)
}

fn parts(editor: &Entity<CodeEditor>, cx: &mut VisualTestContext) -> Vec<Vec<String>> {
    editor.read_with(cx, |e, cx| {
        e.hover_parts_for_tests(cx)
            .into_iter()
            .map(|part| part.into_iter().map(|c| c.to_string()).collect())
            .collect()
    })
}

fn trigger(editor: &Entity<CodeEditor>, cx: &mut VisualTestContext, id: &str) {
    cx.update(|window, cx| {
        editor
            .update(cx, |e, cx| e.trigger(id, window, cx))
            .unwrap()
    });
    rest(cx, 0);
}

#[gpui::test]
fn hover_verbosity(cx: &mut TestAppContext) {
    let deltas = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
    let (editor, cx) = setup_with("alpha beta\n", LevelHover(deltas.clone()), cx);
    cx.dispatch_action(gpui_monaco::actions::ShowHover::default());
    rest(cx, 0);
    assert_eq!(parts(&editor, cx), [["level 0"]]);
    // Can't decrease at level 0.
    trigger(&editor, cx, "editor.action.decreaseHoverVerbosityLevel");
    assert_eq!(parts(&editor, cx), [["level 0"]]);
    trigger(&editor, cx, "editor.action.increaseHoverVerbosityLevel");
    assert_eq!(parts(&editor, cx), [["level 1"]]);
    trigger(&editor, cx, "editor.action.increaseHoverVerbosityLevel");
    assert_eq!(parts(&editor, cx), [["level 2"]]);
    // The provider says that's the most detail.
    trigger(&editor, cx, "editor.action.increaseHoverVerbosityLevel");
    assert_eq!(parts(&editor, cx), [["level 2"]]);
    trigger(&editor, cx, "editor.action.decreaseHoverVerbosityLevel");
    assert_eq!(parts(&editor, cx), [["level 1"]]);
    assert_eq!(*deltas.borrow(), [1, 1, -1]);
    assert!(visible(&editor, cx));
}

#[gpui::test]
fn long_line_warnings(cx: &mut TestAppContext) {
    // Monaco's defaults: rendering stops after 10,000 characters and
    // tokenization skips lines of 20,000.
    let text = format!("short\n{}\n", "x ".repeat(10_500));
    let (editor, cx) = setup_with(&text, LongHover(1), cx);
    let show_at = |cx: &mut VisualTestContext, column: u32| {
        editor.update(cx, |e, cx| {
            e.set_selections(&[Selection::collapsed(PointUtf16::new(1, column))], cx);
            e.hide_hover(cx);
        });
        cx.dispatch_action(gpui_monaco::actions::ShowHover::default());
        rest(cx, 0);
    };
    show_at(cx, 4);
    let warning = parts(&editor, cx);
    assert!(warning[0][0].starts_with("Tokenization is skipped for long lines"));
    assert_eq!(warning[1], ["line 0\n\n"]);
    show_at(cx, 10_002);
    assert!(parts(&editor, cx)[0][0].starts_with("Rendering paused for long line"));
    // Short lines don't warn.
    editor.update(cx, |e, cx| {
        e.set_selections(&[Selection::collapsed(PointUtf16::new(0, 1))], cx);
        e.hide_hover(cx);
    });
    cx.dispatch_action(gpui_monaco::actions::ShowHover::default());
    rest(cx, 0);
    assert_eq!(parts(&editor, cx), [["line 0\n\n"]]);
    // "Don't Show Again".
    trigger(&editor, cx, "editor.action.hideLongLineWarningHover");
    assert!(!editor.read_with(cx, |e, _| e.options().hover.show_long_line_warning));
    show_at(cx, 4);
    assert_eq!(parts(&editor, cx), [["line 0\n\n"]]);
}

#[gpui::test]
fn copying_a_hover_part(cx: &mut TestAppContext) {
    let (editor, cx) = setup(HoverOptions::default(), 1, cx);
    cx.dispatch_action(gpui_monaco::actions::ShowHover::default());
    rest(cx, 0);
    let card = editor
        .read_with(cx, |e, cx| e.hover_card_bounds_for_tests(cx))
        .unwrap();
    // The copy button is at the part's top right.
    let button = gpui::point(card.right() - px(14.), card.top() + px(12.));
    cx.simulate_mouse_move(button, None, Modifiers::default());
    cx.simulate_click(button, Modifiers::default());
    rest(cx, 0);
    let copied = cx.read_from_clipboard().and_then(|item| item.text());
    assert_eq!(copied.as_deref(), Some("line 0\n\n"));
}

#[gpui::test]
fn resizing_the_hover(cx: &mut TestAppContext) {
    let (editor, cx) = setup(HoverOptions::default(), 3, cx);
    cx.dispatch_action(gpui_monaco::actions::ShowHover::default());
    rest(cx, 0);
    let bounds = |cx: &mut VisualTestContext| {
        editor
            .read_with(cx, |e, cx| e.hover_card_bounds_for_tests(cx))
            .unwrap()
    };
    let before = bounds(cx);
    // Row 0 has no room above, so the card is below and its handle is at
    // the bottom right.
    let corner = gpui::point(before.right(), before.bottom());
    cx.simulate_mouse_down(corner, gpui::MouseButton::Left, Modifiers::default());
    paint(cx);
    let to = gpui::point(corner.x + px(160.), corner.y + px(40.));
    cx.simulate_mouse_move(to, gpui::MouseButton::Left, Modifiers::default());
    cx.simulate_mouse_up(to, gpui::MouseButton::Left, Modifiers::default());
    paint(cx);
    // Resizing keeps the hover even though the mouse left its word.
    assert!(visible(&editor, cx));
    let after = bounds(cx);
    assert_eq!(after.size.width, before.size.width + px(160.));
    assert_eq!(after.size.height, before.size.height + px(40.));
    assert_eq!(after.origin, before.origin);
}

// ----- Dismissal (#335): Monaco `ContentHoverController` ---------------------

/// Rests the mouse on `beta` until the hover shows.
fn hover_beta(editor: &Entity<CodeEditor>, cx: &mut VisualTestContext) {
    let p = mouse_at(editor, cx, 0, 7);
    cx.simulate_mouse_move(p, None, Modifiers::default());
    rest(cx, 400);
    assert!(visible(editor, cx));
}

fn head(editor: &Entity<CodeEditor>, cx: &mut VisualTestContext) -> PointUtf16 {
    editor.read_with(cx, |e, _| e.selections()[0].head)
}

#[gpui::test]
fn keys_hide_the_hover_and_still_run(cx: &mut TestAppContext) {
    let (editor, cx) = setup(HoverOptions::default(), 1, cx);
    hover_beta(&editor, cx);
    // A lone modifier keeps it.
    cx.simulate_modifiers_change(Modifiers::shift());
    cx.simulate_modifiers_change(Modifiers::default());
    rest(cx, 0);
    assert!(visible(&editor, cx));
    // Any other key hides it, and its command runs.
    cx.simulate_keystrokes("right");
    rest(cx, 0);
    assert!(!visible(&editor, cx));
    assert_eq!(head(&editor, cx), PointUtf16::new(0, 1));
}

#[gpui::test]
fn chord_prefixes_and_hover_commands_keep_the_hover(cx: &mut TestAppContext) {
    let (editor, cx) = setup(HoverOptions::default(), 1, cx);
    hover_beta(&editor, cx);
    let (prefix, show) = if cfg!(target_os = "macos") {
        ("cmd-k", "cmd-i")
    } else {
        ("ctrl-k", "ctrl-i")
    };
    // The first key of Cmd/Ctrl+K Cmd/Ctrl+I keeps it, and the second (Show
    // or Focus Hover) focuses it.
    cx.simulate_keystrokes(prefix);
    rest(cx, 0);
    assert!(visible(&editor, cx));
    cx.simulate_keystrokes(show);
    rest(cx, 0);
    assert!(visible(&editor, cx));
    assert!(cx.update(|window, cx| editor.read(cx).is_hover_focused(window, cx)));
}

#[gpui::test]
fn a_keyboard_hover_goes_with_the_arrow_keys(cx: &mut TestAppContext) {
    let (editor, cx) = setup(HoverOptions::default(), 1, cx);
    cx.dispatch_action(gpui_monaco::actions::ShowHover::default());
    rest(cx, 0);
    assert!(visible(&editor, cx));
    cx.simulate_keystrokes("down");
    rest(cx, 0);
    assert!(!visible(&editor, cx));
    assert_eq!(head(&editor, cx), PointUtf16::new(1, 0));
}

#[gpui::test]
fn one_escape_hides_the_hover_and_the_other_cursors(cx: &mut TestAppContext) {
    let (editor, cx) = setup(HoverOptions::default(), 1, cx);
    editor.update(cx, |e, cx| {
        e.set_selections(
            &[
                Selection::collapsed(PointUtf16::new(1, 0)),
                Selection::collapsed(PointUtf16::new(2, 0)),
            ],
            cx,
        )
    });
    hover_beta(&editor, cx);
    cx.simulate_keystrokes("escape");
    rest(cx, 0);
    assert!(!visible(&editor, cx));
    assert_eq!(editor.read_with(cx, |e, _| e.selections().len()), 1);
}

#[gpui::test]
fn clicks_off_the_card_hide_the_hover(cx: &mut TestAppContext) {
    let (editor, cx) = setup(HoverOptions::default(), 1, cx);
    hover_beta(&editor, cx);
    // On the card: it stays.
    let card = editor
        .read_with(cx, |e, cx| e.hover_card_bounds_for_tests(cx))
        .expect("painted");
    cx.simulate_mouse_move(card.center(), None, Modifiers::default());
    cx.simulate_click(card.center(), Modifiers::default());
    rest(cx, 0);
    assert!(visible(&editor, cx));
    // On the word it's about: it goes.
    let p = mouse_at(&editor, cx, 0, 7);
    cx.simulate_mouse_move(p, None, Modifiers::default());
    rest(cx, 0);
    assert!(visible(&editor, cx));
    cx.simulate_click(p, Modifiers::default());
    rest(cx, 0);
    assert!(!visible(&editor, cx));
    // A right click too.
    hover_beta(&editor, cx);
    cx.simulate_mouse_down(p, gpui::MouseButton::Right, Modifiers::default());
    cx.simulate_mouse_up(p, gpui::MouseButton::Right, Modifiers::default());
    rest(cx, 0);
    assert!(!visible(&editor, cx));
}

#[gpui::test]
fn leaving_the_editor_hides_and_cancels_the_hover(cx: &mut TestAppContext) {
    let (editor, cx) = setup(HoverOptions::default(), 1, cx);
    let outside = gpui::point(px(-20.), px(-20.));
    // Shown: it goes.
    hover_beta(&editor, cx);
    cx.simulate_mouse_move(outside, None, Modifiers::default());
    rest(cx, 0);
    assert!(!visible(&editor, cx));
    // Pending: it never shows.
    let p = mouse_at(&editor, cx, 0, 7);
    cx.simulate_mouse_move(p, None, Modifiers::default());
    rest(cx, 100);
    cx.simulate_mouse_move(outside, None, Modifiers::default());
    rest(cx, 1000);
    assert!(!visible(&editor, cx));
}

#[gpui::test]
fn a_drag_select_hides_the_hover_and_shows_none(cx: &mut TestAppContext) {
    let (editor, cx) = setup(HoverOptions::default(), 1, cx);
    let from = mouse_at(&editor, cx, 0, 1);
    let to = mouse_at(&editor, cx, 0, 13);
    cx.simulate_mouse_move(from, None, Modifiers::default());
    cx.simulate_mouse_down(from, gpui::MouseButton::Left, Modifiers::default());
    rest(cx, 0);
    // A hover shown from the keyboard meanwhile goes as the drag moves.
    editor.update(cx, |e, cx| e.show_hover_at_cursor(cx));
    rest(cx, 0);
    assert!(visible(&editor, cx));
    cx.simulate_mouse_move(to, gpui::MouseButton::Left, Modifiers::default());
    rest(cx, 0);
    assert!(!visible(&editor, cx));
    // Resting on a word mid-drag shows nothing.
    rest(cx, 1000);
    assert!(!visible(&editor, cx));
    cx.simulate_mouse_up(to, gpui::MouseButton::Left, Modifiers::default());
    rest(cx, 0);
    assert!(!visible(&editor, cx));
}

fn wheel(editor: &Entity<CodeEditor>, cx: &mut VisualTestContext, dy: f32) {
    let at = mouse_at(editor, cx, 1, 2);
    cx.simulate_event(gpui::ScrollWheelEvent {
        position: at,
        delta: gpui::ScrollDelta::Pixels(gpui::point(px(0.), px(dy))),
        ..Default::default()
    });
    rest(cx, 0);
}

#[gpui::test]
fn only_wheels_that_scroll_hide_the_hover(cx: &mut TestAppContext) {
    let (editor, cx) = setup(HoverOptions::default(), 1, cx);
    hover_beta(&editor, cx);
    // At the top, up scrolls nothing.
    wheel(&editor, cx, 40.);
    assert!(visible(&editor, cx));
    // Down does.
    wheel(&editor, cx, -10.);
    assert!(editor.read_with(cx, |e, _| e.scroll_top()) > px(0.));
    assert!(!visible(&editor, cx));
    // With the wheel off nothing scrolls.
    editor.update(cx, |e, cx| {
        e.set_scroll_position(gpui::point(px(0.), px(0.)), cx);
        let mut options = e.options().clone();
        options.scrollbar.handle_mouse_wheel = false;
        e.update_options(options, cx);
    });
    rest(cx, 0);
    hover_beta(&editor, cx);
    wheel(&editor, cx, -10.);
    assert!(visible(&editor, cx));
}

#[gpui::test]
fn scrolling_from_the_api_hides_the_hover(cx: &mut TestAppContext) {
    let (editor, cx) = setup(HoverOptions::default(), 1, cx);
    hover_beta(&editor, cx);
    editor.update(cx, |e, cx| {
        e.set_scroll_top(px(5.), gpui_monaco::ScrollType::Immediate, cx)
    });
    rest(cx, 0);
    assert!(!visible(&editor, cx));
}

#[gpui::test]
fn turning_hovers_off_hides_the_hover(cx: &mut TestAppContext) {
    let (editor, cx) = setup(HoverOptions::default(), 1, cx);
    hover_beta(&editor, cx);
    // Another hover option leaves it.
    editor.update(cx, |e, cx| {
        let mut options = e.options().clone();
        options.hover.above = false;
        e.update_options(options, cx);
    });
    rest(cx, 0);
    assert!(visible(&editor, cx));
    editor.update(cx, |e, cx| {
        let mut options = e.options().clone();
        options.hover.enabled = HoverEnabled::Off;
        e.update_options(options, cx);
    });
    rest(cx, 0);
    assert!(!visible(&editor, cx));
}
