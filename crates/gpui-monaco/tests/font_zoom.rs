//! Font zoom (Monaco `editorZoom.ts`, `contrib/fontZoom`) and font options.

use gpui::{
    Entity, Focusable, Modifiers, PinchEvent, ScrollDelta, ScrollWheelEvent, TestAppContext,
    VisualTestContext, point, px,
};
use gpui_monaco::CodeEditor;

fn setup(cx: &mut TestAppContext) -> (Entity<CodeEditor>, &mut VisualTestContext) {
    cx.update(gpui_monaco::init);
    let (editor, cx) = cx.add_window_view(|window, cx| {
        let mut editor = CodeEditor::with_text("one\ntwo\nthree", window, cx);
        let mut options = editor.options().clone();
        options.font_size = px(10.);
        options.line_height = Some(px(20.));
        editor.update_options(options, cx);
        editor
    });
    cx.update(|window, cx| window.focus(&editor.focus_handle(cx), cx));
    paint(cx);
    (editor, cx)
}

fn paint(cx: &mut VisualTestContext) {
    cx.update(|window, _| window.refresh());
    cx.run_until_parked();
}

fn line_height(editor: &Entity<CodeEditor>, cx: &mut VisualTestContext) -> f32 {
    paint(cx);
    f32::from(editor.read_with(cx, |e, _| e.view_metrics_for_tests().unwrap().3))
}

#[gpui::test]
fn zoom_actions_scale_every_editor(cx: &mut TestAppContext) {
    let (editor, cx) = setup(cx);
    assert_eq!(line_height(&editor, cx), 20.);
    editor.update_in(cx, |e, window, cx| {
        e.trigger("editor.action.fontZoomIn", window, cx).unwrap()
    });
    assert_eq!(line_height(&editor, cx), 22.);
    // The host's options keep the unzoomed size.
    editor.read_with(cx, |e, _| assert_eq!(e.options().font_size, px(10.)));
    editor.update_in(cx, |e, window, cx| {
        e.trigger("editor.action.fontZoomOut", window, cx).unwrap()
    });
    editor.update_in(cx, |e, window, cx| {
        e.trigger("editor.action.fontZoomOut", window, cx).unwrap()
    });
    assert_eq!(line_height(&editor, cx), 18.);
    editor.update_in(cx, |e, window, cx| {
        e.trigger("editor.action.fontZoomReset", window, cx)
            .unwrap()
    });
    assert_eq!(line_height(&editor, cx), 20.);

    // Clamped to [-5, 20].
    cx.update(|_, cx| gpui_monaco::set_editor_zoom_level(-9., cx));
    assert_eq!(cx.update(|_, cx| gpui_monaco::editor_zoom_level(cx)), -5.);
    assert_eq!(line_height(&editor, cx), 10.);
}

#[gpui::test]
fn mouse_wheel_zoom(cx: &mut TestAppContext) {
    let (editor, cx) = setup(cx);
    let wheel = |cx: &mut VisualTestContext, modifiers: Modifiers| {
        cx.simulate_event(ScrollWheelEvent {
            position: point(px(100.), px(10.)),
            delta: ScrollDelta::Lines(point(0., 1.)),
            modifiers,
            ..Default::default()
        });
    };
    // Off by default.
    wheel(cx, Modifiers::secondary_key());
    assert_eq!(cx.update(|_, cx| gpui_monaco::editor_zoom_level(cx)), 0.);

    editor.update(cx, |e, cx| {
        let mut options = e.options().clone();
        options.mouse_wheel_zoom = true;
        e.update_options(options, cx);
    });
    paint(cx);
    wheel(cx, Modifiers::secondary_key());
    assert_eq!(cx.update(|_, cx| gpui_monaco::editor_zoom_level(cx)), 1.);
    assert_eq!(line_height(&editor, cx), 22.);
    // Without the modifier the wheel scrolls.
    wheel(cx, Modifiers::default());
    assert_eq!(cx.update(|_, cx| gpui_monaco::editor_zoom_level(cx)), 1.);
}

/// Monaco zooms on a trackpad pinch, which browsers deliver as a Ctrl+wheel
/// of one notch per update: a level per five updates, as one gesture.
#[gpui::test]
fn pinch_zoom(cx: &mut TestAppContext) {
    let (editor, cx) = setup(cx);
    let pinch = |cx: &mut VisualTestContext, delta: f32, modifiers: Modifiers| {
        cx.simulate_event(PinchEvent {
            position: point(px(100.), px(10.)),
            delta,
            modifiers,
            ..Default::default()
        });
    };
    let level = |cx: &mut VisualTestContext| cx.update(|_, cx| gpui_monaco::editor_zoom_level(cx));
    // Off by default.
    pinch(cx, 0.1, Modifiers::default());
    assert_eq!(level(cx), 0.);

    editor.update(cx, |e, cx| {
        let mut options = e.options().clone();
        options.mouse_wheel_zoom = true;
        e.update_options(options, cx);
    });
    paint(cx);
    for _ in 0..5 {
        pinch(cx, 0.03, Modifiers::default());
    }
    assert!((level(cx) - 1.).abs() < 1e-4);
    assert_eq!(line_height(&editor, cx), 22.);
    for _ in 0..10 {
        pinch(cx, -0.5, Modifiers::default());
    }
    assert!((level(cx) + 1.).abs() < 1e-4);
}

/// Like the wheel, a pinch with Shift or Alt held doesn't zoom.
#[gpui::test]
fn pinch_with_shift_does_not_zoom(cx: &mut TestAppContext) {
    let (editor, cx) = setup(cx);
    editor.update(cx, |e, cx| {
        let mut options = e.options().clone();
        options.mouse_wheel_zoom = true;
        e.update_options(options, cx);
    });
    paint(cx);
    cx.simulate_event(PinchEvent {
        position: point(px(100.), px(10.)),
        delta: 0.5,
        modifiers: Modifiers::shift(),
        ..Default::default()
    });
    assert_eq!(cx.update(|_, cx| gpui_monaco::editor_zoom_level(cx)), 0.);
}
