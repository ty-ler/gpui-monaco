//! The content hover's anchor and lifetime (#474): the character under the
//! mouse (Monaco's mouse target `range`), keeping a hover the mouse moves
//! toward (`isMouseGettingCloser`), leaving the card to outside the editor,
//! focus across streamed results and when hiding, and the verbosity
//! command and buttons.

use std::cell::RefCell;
use std::ops::Range;
use std::rc::Rc;
use std::time::Duration;

use gpui::{
    App, AppContext as _, Context, Entity, Focusable as _, IntoElement, Modifiers, ParentElement,
    Pixels, Render, Styled, Task, TestAppContext, VisualTestContext, Window, div, px,
};
use gpui_monaco::providers::*;
use gpui_monaco::text::PointUtf16;
use gpui_monaco::{CodeEditor, DecorationOptions, Marker, MarkerSeverity, TextModel};

fn p(row: u32, column: u32) -> PointUtf16 {
    PointUtf16::new(row, column)
}

/// A window with the editor at its top left, `width` by `height`.
struct Host {
    editor: Entity<CodeEditor>,
    width: Pixels,
    height: Pixels,
}

impl Render for Host {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div().size_full().child(
            div()
                .w(self.width)
                .h(self.height)
                .child(self.editor.clone()),
        )
    }
}

fn setup<'a>(
    text: &str,
    size: Option<(f32, f32)>,
    cx: &'a mut TestAppContext,
) -> (Entity<CodeEditor>, &'a mut VisualTestContext) {
    cx.update(gpui_monaco::init);
    let text = text.to_string();
    let (width, height) = size.unwrap_or((1000., 600.));
    let (host, cx) = cx.add_window_view(|window, cx| {
        let editor = cx.new(|cx| {
            let model = cx.new(|_| TextModel::new(&text));
            let mut editor = CodeEditor::new(model, window, cx);
            editor.update_options_with(
                |o| {
                    o.minimap.enabled = false;
                    o.sticky_scroll.enabled = false;
                },
                cx,
            );
            editor
        });
        Host {
            editor,
            width: px(width),
            height: px(height),
        }
    });
    let editor = host.read_with(cx, |host, _| host.editor.clone());
    cx.update(|window, cx| window.focus(&editor.focus_handle(cx), cx));
    paint(cx);
    (editor, cx)
}

fn paint(cx: &mut VisualTestContext) {
    cx.update(|window, _| window.refresh());
    cx.run_until_parked();
}

fn rest(cx: &mut VisualTestContext, ms: u64) {
    cx.executor().advance_clock(Duration::from_millis(ms));
    cx.run_until_parked();
    paint(cx);
}

/// The window position of the left (`right` off) or right edge of the
/// character at `row`, `column`, 2px into its line.
fn char_edge(
    editor: &Entity<CodeEditor>,
    cx: &mut VisualTestContext,
    row: u32,
    column: u32,
    right: bool,
) -> gpui::Point<Pixels> {
    let at = |cx: &mut VisualTestContext, column| {
        editor
            .read_with(cx, |e, _| e.scrolled_visible_position(p(row, column)))
            .expect("laid out")
    };
    let origin = at(cx, column);
    let x = if right {
        at(cx, column + 1).x - px(1.)
    } else {
        origin.x + px(1.)
    };
    gpui::point(x, origin.y + px(2.))
}

fn visible(editor: &Entity<CodeEditor>, cx: &mut VisualTestContext) -> bool {
    editor.read_with(cx, |e, _| e.is_hover_visible())
}

fn card(editor: &Entity<CodeEditor>, cx: &mut VisualTestContext) -> gpui::Bounds<Pixels> {
    editor
        .read_with(cx, |e, cx| e.hover_card_bounds_for_tests(cx))
        .expect("painted")
}

/// Rests the mouse at `point` past the hover delay, from off the editor
/// without a hover.
fn hover_at(editor: &Entity<CodeEditor>, cx: &mut VisualTestContext, point: gpui::Point<Pixels>) {
    cx.simulate_mouse_move(gpui::point(px(-10.), px(-10.)), None, Modifiers::default());
    rest(cx, 0);
    editor.update(cx, |e, cx| e.hide_hover(cx));
    cx.simulate_mouse_move(point, None, Modifiers::default());
    rest(cx, 400);
}

/// [`hover_at`] the left or right edge of a character.
fn hover_char(
    editor: &Entity<CodeEditor>,
    cx: &mut VisualTestContext,
    row: u32,
    column: u32,
    right: bool,
) {
    let point = char_edge(editor, cx, row, column, right);
    hover_at(editor, cx, point);
}

fn contents(editor: &Entity<CodeEditor>, cx: &mut VisualTestContext) -> Vec<String> {
    editor.read_with(cx, |e, cx| e.hover_contents_for_tests(cx))
}

fn markers(editor: &Entity<CodeEditor>, cx: &mut VisualTestContext) -> usize {
    editor.read_with(cx, |e, cx| e.hover_markers_for_tests(cx).len())
}

/// Answers with `text` (after `delay`), recording where it was asked.
#[derive(Clone)]
struct Recorded {
    text: String,
    delay: Option<Duration>,
    asked: Rc<RefCell<Vec<PointUtf16>>>,
}

impl Recorded {
    fn new(text: &str) -> Self {
        Self {
            text: text.into(),
            delay: None,
            asked: Default::default(),
        }
    }
}

impl HoverProvider for Recorded {
    fn provide_hover(
        &self,
        _: &ProviderContext,
        position: PointUtf16,
        cx: &mut App,
    ) -> Task<anyhow::Result<Option<Hover>>> {
        self.asked.borrow_mut().push(position);
        let hover = Hover {
            contents: vec![self.text.clone().into()],
            ..Default::default()
        };
        match self.delay {
            None => Task::ready(Ok(Some(hover))),
            Some(delay) => {
                let timer = cx.background_executor().timer(delay);
                cx.background_executor().spawn(async move {
                    timer.await;
                    Ok(Some(hover))
                })
            }
        }
    }
}

fn set_markers(
    editor: &Entity<CodeEditor>,
    cx: &mut VisualTestContext,
    ranges: Vec<Range<PointUtf16>>,
) {
    let markers = ranges
        .into_iter()
        .map(|range| Marker {
            severity: MarkerSeverity::Error,
            message: "bad".into(),
            range,
            ..Default::default()
        })
        .collect();
    editor.update(cx, |e, cx| {
        e.model()
            .update(cx, |m, cx| m.set_markers("test", markers, cx))
    });
    paint(cx);
}

/// Monaco `HoverRangeAnchor` on the mouse target's `range`: a decoration's
/// message shows only when the character under the mouse is in it, not
/// when the nearer caret position touches it.
#[gpui::test]
fn decorations_hover_for_the_character_under_the_mouse(cx: &mut TestAppContext) {
    let (editor, cx) = setup("let x = foo; y\n", None, cx);
    let model = editor.read_with(cx, |e, _| e.model().clone());
    let id = editor.entity_id();
    model.update(cx, |m, cx| {
        m.delta_decorations(
            Some(id),
            &[],
            vec![(
                8..11,
                DecorationOptions {
                    hover_message: vec!["deco".into()],
                    ..Default::default()
                },
            )],
            cx,
        );
    });
    paint(cx);
    // The left half of `;`, right after `foo`.
    hover_char(&editor, cx, 0, 11, false);
    assert!(contents(&editor, cx).is_empty());
    // The right half of the last `o`.
    hover_char(&editor, cx, 0, 10, true);
    assert_eq!(contents(&editor, cx), ["deco"]);
    // The right half of the space before `foo`.
    hover_char(&editor, cx, 0, 7, true);
    assert!(contents(&editor, cx).is_empty());
}

/// Monaco's marker decorations are `showIfCollapsed`: a character more each
/// side of the marker, by the character under the mouse.
#[gpui::test]
fn markers_hover_for_the_character_under_the_mouse(cx: &mut TestAppContext) {
    let (editor, cx) = setup("let x = foo; y\n", None, cx);
    set_markers(&editor, cx, vec![p(0, 8)..p(0, 11)]);
    // All of `;`, its right half too.
    hover_char(&editor, cx, 0, 11, true);
    assert_eq!(markers(&editor, cx), 1);
    // The space after it.
    hover_char(&editor, cx, 0, 12, false);
    assert_eq!(markers(&editor, cx), 0);
    // The space before `foo`, and not the `=` before that.
    hover_char(&editor, cx, 0, 7, false);
    assert_eq!(markers(&editor, cx), 1);
    hover_char(&editor, cx, 0, 6, true);
    assert_eq!(markers(&editor, cx), 0);
}

/// Monaco `MarkdownHoverParticipant._getMarkdownHovers`: providers are
/// asked at the start of the character under the mouse.
#[gpui::test]
fn providers_are_asked_about_the_character_under_the_mouse(cx: &mut TestAppContext) {
    let provider = Recorded::new("docs");
    let asked = provider.asked.clone();
    cx.update(|cx| register_hover_provider(cx, LanguageSelector::All, provider));
    let (editor, cx) = setup("foo.bar\n", None, cx);
    // The right half of `.`: about the `.`, not `bar`.
    hover_char(&editor, cx, 0, 3, true);
    assert_eq!(asked.borrow().last(), Some(&p(0, 3)));
    assert_eq!(contents(&editor, cx), ["docs"]);
}

/// Monaco `isMouseGettingCloser`: after the hiding delay, a sticky hover
/// stays while the mouse isn't more than 4px farther from it than it has
/// been, and goes once it is.
#[gpui::test]
fn a_hover_stays_while_the_mouse_moves_toward_it(cx: &mut TestAppContext) {
    let provider = Recorded::new("docs");
    cx.update(|cx| register_hover_provider(cx, LanguageSelector::All, provider));
    let (editor, cx) = setup("alpha\n\n\n\n\n\n\n\n", None, cx);
    let start = char_edge(&editor, cx, 0, 1, false);
    hover_at(&editor, cx, start);
    assert!(visible(&editor, cx));
    // No room above the first line: the card is below it.
    let bounds = card(&editor, cx);
    assert!(bounds.top() > start.y);
    // Past the end of `alpha` (nothing hovers there), toward the card.
    let end_of_line = char_edge(&editor, cx, 0, 4, true).x + px(4.);
    assert!(end_of_line < bounds.right());
    let closer = gpui::point(end_of_line, bounds.top() - px(3.));
    cx.simulate_mouse_move(closer, None, Modifiers::default());
    rest(cx, 1000);
    assert!(visible(&editor, cx));
    // Farther away: it goes after the hiding delay.
    let away = gpui::point(end_of_line, bounds.bottom() + px(60.));
    cx.simulate_mouse_move(away, None, Modifiers::default());
    rest(cx, 400);
    assert!(!visible(&editor, cx));
}

/// A tall hover for every position.
struct TallHover;
impl HoverProvider for TallHover {
    fn provide_hover(
        &self,
        _: &ProviderContext,
        _: PointUtf16,
        _: &mut App,
    ) -> Task<anyhow::Result<Option<Hover>>> {
        let text: String = (0..12).map(|i| format!("line {i}\n\n")).collect();
        Task::ready(Ok(Some(Hover {
            contents: vec![text.into()],
            can_increase_verbosity: true,
            ..Default::default()
        })))
    }
}

/// Monaco `ContentHoverWidgetWrapper._onMouseLeave`: the mouse leaving the
/// card (where it overflows the editor) to outside the editor hides it.
#[gpui::test]
fn leaving_the_card_to_outside_the_editor_hides_it(cx: &mut TestAppContext) {
    cx.update(|cx| register_hover_provider(cx, LanguageSelector::All, TallHover));
    let (editor, cx) = setup("alpha\nbeta\n", Some((400., 60.)), cx);
    hover_char(&editor, cx, 0, 1, false);
    assert!(visible(&editor, cx));
    let bounds = card(&editor, cx);
    let editor_bottom = px(60.);
    assert!(bounds.bottom() > editor_bottom + px(20.));
    // Onto the card where it overflows: it stays.
    let on_card = gpui::point(bounds.left() + px(20.), editor_bottom + px(10.));
    cx.simulate_mouse_move(on_card, None, Modifiers::default());
    rest(cx, 1000);
    assert!(visible(&editor, cx));
    // Off the card, outside the editor.
    let outside = gpui::point(bounds.right() + px(30.), editor_bottom + px(10.));
    cx.simulate_mouse_move(outside, None, Modifiers::default());
    rest(cx, 0);
    assert!(!visible(&editor, cx));
}

fn hover_focused(editor: &Entity<CodeEditor>, cx: &mut VisualTestContext) -> bool {
    cx.update(|window, cx| editor.read(cx).is_hover_focused(window, cx))
}

fn editor_focused(editor: &Entity<CodeEditor>, cx: &mut VisualTestContext) -> bool {
    cx.update(|window, cx| editor.read(cx).focus_handle(cx).is_focused(window))
}

/// Monaco swaps only the card's contents for a new result, so a focused
/// card keeps the focus while a provider's part streams in.
#[gpui::test]
fn a_focused_hover_keeps_the_focus_as_parts_stream_in(cx: &mut TestAppContext) {
    let provider = Recorded {
        delay: Some(Duration::from_millis(1000)),
        ..Recorded::new("late docs")
    };
    cx.update(|cx| register_hover_provider(cx, LanguageSelector::All, provider));
    let (editor, cx) = setup("let x = foo;\n", None, cx);
    set_markers(&editor, cx, vec![p(0, 8)..p(0, 11)]);
    editor.update(cx, |e, cx| {
        e.set_selections(&[gpui_monaco::cursor::Selection::collapsed(p(0, 9))], cx)
    });
    // Cmd/Ctrl+K Cmd/Ctrl+I twice: the marker's card, focused.
    cx.dispatch_action(gpui_monaco::actions::ShowHover::default());
    rest(cx, 0);
    cx.dispatch_action(gpui_monaco::actions::ShowHover::default());
    rest(cx, 0);
    assert!(hover_focused(&editor, cx));
    assert!(contents(&editor, cx).iter().all(|c| c != "late docs"));
    rest(cx, 1000);
    assert!(contents(&editor, cx).iter().any(|c| c == "late docs"));
    assert!(hover_focused(&editor, cx));
}

/// Monaco `ContentHoverWidget.hide`: hiding a hover that had the focus
/// returns it to the editor (here by scrolling the text).
#[gpui::test]
fn hiding_a_focused_hover_refocuses_the_editor(cx: &mut TestAppContext) {
    cx.update(|cx| register_hover_provider(cx, LanguageSelector::All, Recorded::new("docs")));
    let text: String = (0..100).map(|i| format!("line {i}\n")).collect();
    let (editor, cx) = setup(&text, Some((600., 200.)), cx);
    cx.dispatch_action(gpui_monaco::actions::ShowHover::default());
    rest(cx, 0);
    cx.dispatch_action(gpui_monaco::actions::ShowHover::default());
    rest(cx, 0);
    assert!(hover_focused(&editor, cx));
    let at = char_edge(&editor, cx, 3, 1, false);
    cx.simulate_event(gpui::ScrollWheelEvent {
        position: at,
        delta: gpui::ScrollDelta::Pixels(gpui::point(px(0.), px(-60.))),
        ..Default::default()
    });
    rest(cx, 0);
    assert!(!visible(&editor, cx));
    assert!(editor_focused(&editor, cx));
}

/// A hover whose detail goes from 0 to 2 (Monaco's verbosity levels).
struct LevelHover(&'static str);
impl HoverProvider for LevelHover {
    fn provide_hover(
        &self,
        context: &ProviderContext,
        position: PointUtf16,
        cx: &mut App,
    ) -> Task<anyhow::Result<Option<Hover>>> {
        self.provide_hover_with_context(context, position, &HoverContext::default(), cx)
    }

    fn provide_hover_with_context(
        &self,
        _: &ProviderContext,
        _: PointUtf16,
        context: &HoverContext,
        _: &mut App,
    ) -> Task<anyhow::Result<Option<Hover>>> {
        let level = match &context.verbosity_request {
            Some(request) => {
                let previous: i32 = request.previous_hover.contents[0]
                    .value
                    .rsplit(' ')
                    .next()
                    .unwrap()
                    .parse()
                    .unwrap();
                (previous + request.verbosity_delta).clamp(0, 2)
            }
            None => 0,
        };
        Task::ready(Ok(Some(Hover {
            contents: vec![format!("{} {level}", self.0).into()],
            range: None,
            can_increase_verbosity: level < 2,
            can_decrease_verbosity: level > 0,
        })))
    }
}

/// Monaco `RenderedContentHoverParts.updateHoverVerbosityLevel` without a
/// focused part: every markdown part that supports it.
#[gpui::test]
fn the_verbosity_command_without_a_focused_part_updates_every_part(cx: &mut TestAppContext) {
    cx.update(|cx| {
        register_hover_provider(cx, LanguageSelector::All, LevelHover("first"));
        register_hover_provider(cx, LanguageSelector::All, LevelHover("second"));
    });
    let (editor, cx) = setup("alpha beta\n", None, cx);
    cx.dispatch_action(gpui_monaco::actions::ShowHover::default());
    rest(cx, 0);
    // The later registered provider first, as Monaco's registry orders.
    assert_eq!(contents(&editor, cx), ["second 0", "first 0"]);
    cx.update(|window, cx| {
        editor
            .update(cx, |e, cx| {
                e.trigger("editor.action.increaseHoverVerbosityLevel", window, cx)
            })
            .unwrap()
    });
    rest(cx, 0);
    assert_eq!(contents(&editor, cx), ["second 1", "first 1"]);
}

/// Monaco `MarkdownHoverParticipant.handleScroll`: a part taller than the
/// card keeps its + and - at the bottom of what the card shows of it.
#[gpui::test]
fn verbosity_buttons_follow_the_hover_scroll(cx: &mut TestAppContext) {
    cx.update(|cx| register_hover_provider(cx, LanguageSelector::All, TallHover));
    let (editor, cx) = setup("alpha\n", Some((600., 300.)), cx);
    cx.dispatch_action(gpui_monaco::actions::ShowHover::default());
    rest(cx, 0);
    paint(cx);
    let bounds = card(&editor, cx);
    let button = |cx: &mut VisualTestContext| cx.debug_bounds("verbosity-up-0").expect("painted");
    // The part runs past the card; its buttons are at the card's bottom.
    let up = button(cx);
    let down = cx.debug_bounds("verbosity-down-0").expect("painted");
    // Both inside the card's 1px border.
    assert!(
        down.bottom() <= bounds.bottom() - px(1.),
        "{down:?} in {bounds:?}"
    );
    assert!(up.top() >= bounds.top());
    // Scrolled to the end, they're at the part's (and card's) bottom.
    cx.dispatch_action(gpui_monaco::actions::ShowHover::default());
    rest(cx, 0);
    cx.simulate_keystrokes("end");
    rest(cx, 0);
    paint(cx);
    let up = button(cx);
    let down = cx.debug_bounds("verbosity-down-0").expect("painted");
    assert!(
        down.bottom() <= bounds.bottom() - px(1.),
        "{down:?} in {bounds:?}"
    );
    assert!(up.top() >= bounds.top());
}
