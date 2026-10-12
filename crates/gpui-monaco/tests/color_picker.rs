//! The color picker (Monaco `colorPicker`): in the hover over a color and
//! standalone.

use std::time::Duration;

use gpui::{AppContext as _, Entity, Focusable, TestAppContext, VisualTestContext};
use gpui_monaco::actions::*;
use gpui_monaco::cursor::Selection;
use gpui_monaco::text::PointUtf16;
use gpui_monaco::{CodeEditor, TextModel};

fn setup<'a>(
    text: &str,
    cx: &'a mut TestAppContext,
) -> (Entity<CodeEditor>, &'a mut VisualTestContext) {
    cx.update(gpui_monaco::init);
    let text = text.to_string();
    let (editor, cx) = cx.add_window_view(|window, cx| {
        let model = cx.new(|_| TextModel::new(&text).with_language("css"));
        CodeEditor::new(model, window, cx)
    });
    cx.update(|window, cx| window.focus(&editor.focus_handle(cx), cx));
    settle(cx);
    (editor, cx)
}

fn settle(cx: &mut VisualTestContext) {
    for _ in 0..3 {
        cx.update(|window, _| window.refresh());
        cx.run_until_parked();
        cx.executor().advance_clock(Duration::from_millis(1100));
        cx.run_until_parked();
    }
}

fn text(editor: &Entity<CodeEditor>, cx: &mut VisualTestContext) -> String {
    editor.read_with(cx, |e, cx| e.model().read(cx).text())
}

fn state(
    editor: &Entity<CodeEditor>,
    cx: &mut VisualTestContext,
) -> Option<(String, Option<String>, std::ops::Range<PointUtf16>)> {
    cx.run_until_parked();
    editor.read_with(cx, |e, cx| e.color_picker_state_for_tests(cx))
}

fn picker(
    editor: &Entity<CodeEditor>,
    cx: &mut VisualTestContext,
) -> Entity<gpui_monaco::ColorPickerWidget> {
    editor
        .read_with(cx, |e, cx| e.color_picker_for_tests(cx))
        .expect("a color picker")
}

fn put_cursor(editor: &Entity<CodeEditor>, row: u32, column: u32, cx: &mut VisualTestContext) {
    editor.update(cx, |e, cx| {
        e.set_selections(&[Selection::collapsed(PointUtf16::new(row, column))], cx)
    });
}

#[gpui::test]
fn hover_picker_rewrites_the_color(cx: &mut TestAppContext) {
    let (editor, cx) = setup("a { color: #ff0000; }", cx);
    put_cursor(&editor, 0, 13, cx);
    cx.dispatch_action(ShowHover::default());
    let range = PointUtf16::new(0, 11)..PointUtf16::new(0, 18);
    // The default presentations are rgb, hsl and hex; the one written
    // like the text is chosen.
    assert_eq!(
        state(&editor, cx),
        Some(("#ff0000ff".to_string(), Some("#ff0000".to_string()), range))
    );

    // Letting go of a new color writes it in the chosen presentation, and
    // the hover stays.
    let widget = picker(&editor, cx);
    widget.update(cx, |w, cx| w.pick_css_for_tests("#00ff00", cx));
    cx.run_until_parked();
    assert_eq!(text(&editor, cx), "a { color: #00ff00; }");
    assert!(editor.read_with(cx, |e, _| e.is_hover_visible()));

    // Clicking the header switches to the next presentation.
    widget.update(cx, |w, cx| w.click_header_for_tests(cx));
    cx.run_until_parked();
    assert_eq!(text(&editor, cx), "a { color: rgb(0, 255, 0); }");
    let (_, label, range) = state(&editor, cx).unwrap();
    assert_eq!(label.as_deref(), Some("rgb(0, 255, 0)"));
    assert_eq!(range, PointUtf16::new(0, 11)..PointUtf16::new(0, 25));

    // One undo step per change.
    cx.dispatch_action(Undo);
    cx.run_until_parked();
    assert_eq!(text(&editor, cx), "a { color: #00ff00; }");
}

#[gpui::test]
fn hover_without_color_has_no_picker(cx: &mut TestAppContext) {
    let (editor, cx) = setup("a { color: red; }", cx);
    put_cursor(&editor, 0, 1, cx);
    cx.dispatch_action(ShowHover::default());
    assert_eq!(state(&editor, cx), None);
}

#[gpui::test]
fn standalone_picker_inserts_and_replaces(cx: &mut TestAppContext) {
    let (editor, cx) = setup("a { color: ; }\nb { color: #00f; }", cx);
    put_cursor(&editor, 0, 11, cx);
    cx.dispatch_action(ShowOrFocusStandaloneColorPicker);
    cx.run_until_parked();
    assert!(editor.read_with(cx, |e, _| e.is_standalone_color_picker_visible()));
    // Not on a color: black, inserted at the cursor.
    let (color, _, _) = state(&editor, cx).unwrap();
    assert_eq!(color, "#000000ff");
    let widget = picker(&editor, cx);
    widget.update(cx, |w, cx| w.pick_css_for_tests("#ff0000", cx));
    widget.update(cx, |w, cx| w.insert_for_tests(cx));
    cx.run_until_parked();
    assert!(!editor.read_with(cx, |e, _| e.is_standalone_color_picker_visible()));
    assert_eq!(
        text(&editor, cx),
        "a { color: rgb(255, 0, 0); }\nb { color: #00f; }"
    );

    // On a color: it's selected and replaced. `#0000ff` doesn't start
    // with `#00f`, so Monaco's guess falls back to the first presentation.
    settle(cx);
    put_cursor(&editor, 1, 12, cx);
    cx.dispatch_action(ShowOrFocusStandaloneColorPicker);
    cx.run_until_parked();
    let (color, label, range) = state(&editor, cx).unwrap();
    assert_eq!(color, "#0000ffff");
    assert_eq!(label.as_deref(), Some("rgb(0, 0, 255)"));
    assert_eq!(range, PointUtf16::new(1, 11)..PointUtf16::new(1, 15));
    let selection = editor.read_with(cx, |e, _| e.selections()[0]);
    assert_eq!(
        (selection.start(), selection.end()),
        (range.start, range.end)
    );
    let widget = picker(&editor, cx);
    widget.update(cx, |w, cx| w.pick_css_for_tests("#ffffff", cx));
    widget.update(cx, |w, cx| w.insert_for_tests(cx));
    cx.run_until_parked();
    assert_eq!(
        text(&editor, cx),
        "a { color: rgb(255, 0, 0); }\nb { color: rgb(255, 255, 255); }"
    );
}

#[gpui::test]
fn escape_hides_the_standalone_picker(cx: &mut TestAppContext) {
    let (editor, cx) = setup("a { color: ; }", cx);
    cx.dispatch_action(ShowOrFocusStandaloneColorPicker);
    cx.run_until_parked();
    assert!(editor.read_with(cx, |e, _| e.is_standalone_color_picker_visible()));
    cx.simulate_keystrokes("escape");
    cx.run_until_parked();
    assert!(!editor.read_with(cx, |e, _| e.is_standalone_color_picker_visible()));
}

/// Monaco captures the pointer on a strip: `grabbing` over the whole
/// window, off the strip and over the text, until the release; the
/// saturation box keeps the arrow.
#[gpui::test]
fn dragging_a_strip_keeps_grabbing_everywhere(cx: &mut TestAppContext) {
    use gpui::{CursorStyle, Modifiers, MouseButton, point, px};
    let (editor, cx) = setup("a { color: #ff0000; }", cx);
    put_cursor(&editor, 0, 13, cx);
    cx.dispatch_action(ShowHover::default());
    cx.run_until_parked();
    cx.update(|window, _| window.refresh());
    cx.run_until_parked();
    let captured =
        |cx: &mut VisualTestContext| editor.read_with(cx, |e, cx| e.captured_cursor_for_tests(cx));
    assert_eq!(captured(cx), None);
    let hue = cx
        .debug_bounds("color-picker-hue-strip")
        .expect("the hue strip");
    let opacity = cx
        .debug_bounds("color-picker-opacity-strip")
        .expect("the opacity strip");
    let saturation = point(opacity.left() - px(60.), opacity.center().y);
    for (from, cursor) in [
        (hue.center(), CursorStyle::ClosedHand),
        (opacity.center(), CursorStyle::ClosedHand),
        (saturation, CursorStyle::Arrow),
    ] {
        cx.simulate_mouse_move(from, None, Modifiers::default());
        cx.simulate_mouse_down(from, MouseButton::Left, Modifiers::default());
        for to in [
            from + point(px(0.), px(10.)),
            point(px(5.), px(5.)),
            point(px(-20.), px(400.)),
        ] {
            cx.simulate_mouse_move(to, MouseButton::Left, Modifiers::default());
            cx.update(|window, _| window.refresh());
            cx.run_until_parked();
            assert_eq!(captured(cx), Some(cursor));
        }
        cx.simulate_mouse_up(
            point(px(5.), px(5.)),
            MouseButton::Left,
            Modifiers::default(),
        );
        cx.update(|window, _| window.refresh());
        cx.run_until_parked();
        assert_eq!(captured(cx), None);
        assert!(editor.read_with(cx, |e, _| e.is_hover_visible()));
    }
}

/// The standalone picker's bounds and the cursor line's top and bottom,
/// with the picker shown at row 20.
fn standalone_placement(
    above: bool,
    cx: &mut TestAppContext,
) -> (gpui::Bounds<gpui::Pixels>, gpui::Pixels, gpui::Pixels) {
    let text = format!("{}a {{ color: ; }}\n{}", "\n".repeat(20), "\n".repeat(20));
    let (editor, cx) = setup(&text, cx);
    editor.update(cx, |e, cx| {
        e.update_options_with(|o| o.hover.above = above, cx)
    });
    put_cursor(&editor, 20, 11, cx);
    cx.dispatch_action(ShowOrFocusStandaloneColorPicker);
    settle(cx);
    assert!(editor.read_with(cx, |e, _| e.is_standalone_color_picker_visible()));
    let line = editor
        .read_with(cx, |e, _| {
            e.scrolled_visible_position(PointUtf16::new(20, 11))
        })
        .unwrap();
    let (_, line_height) = editor.read_with(cx, |e, _| e.layout_for_tests()).unwrap();
    let (_, widgets) = editor.read_with(cx, |e, _| e.widget_bounds_for_tests());
    let picker = widgets.last().unwrap().0;
    (picker, line.y, line.y + line_height)
}

/// Monaco `StandaloneColorPickerWidget.getPosition`: below first when
/// `hover.above` is off, above first otherwise.
#[gpui::test]
fn standalone_picker_follows_hover_above(cx: &mut TestAppContext) {
    let (picker, _, bottom) = standalone_placement(false, cx);
    assert!(picker.top() >= bottom, "{picker:?} {bottom:?}");
}

#[gpui::test]
fn standalone_picker_shows_above_by_default(cx: &mut TestAppContext) {
    let (picker, top, _) = standalone_placement(true, cx);
    assert!(picker.bottom() <= top, "{picker:?} {top:?}");
}

/// Monaco sizes the standalone header from the body: the original color
/// is the body less the saturation box, the close button and 8px; the
/// hover's keeps its 74px.
#[gpui::test]
fn standalone_header_boxes_follow_the_body(cx: &mut TestAppContext) {
    let (editor, cx) = setup("a { color: ; }", cx);
    cx.dispatch_action(ShowOrFocusStandaloneColorPicker);
    settle(cx);
    assert!(editor.read_with(cx, |e, _| e.is_standalone_color_picker_visible()));
    let original = cx
        .debug_bounds("color-picker-original")
        .expect("the original color");
    assert_eq!(original.size.width, gpui::px(52.));
}
