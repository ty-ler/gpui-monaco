//! The mouse pointer over the editor (Monaco's CSS cursors and its
//! `GlobalPointerMoveMonitor`'s pointer capture): the I-beam over the text
//! only; the gutter, scrollbars and minimap keep their arrow, also while
//! drag-selecting text; a drag started off the text keeps its pointer over
//! the whole window until the release, or until it's cancelled.

use gpui::{
    CursorStyle, Entity, Focusable, Modifiers, MouseButton, Pixels, Point, TestAppContext,
    VisualTestContext, point, px,
};
use gpui_monaco::CodeEditor;
use gpui_monaco::text::PointUtf16;

fn setup(cx: &mut TestAppContext) -> (Entity<CodeEditor>, &mut VisualTestContext) {
    cx.update(gpui_monaco::init);
    let text = (0..300)
        .map(|i| format!("line {i} with some text"))
        .collect::<Vec<_>>()
        .join("\n");
    let (editor, cx) = cx.add_window_view(|window, cx| CodeEditor::with_text(&text, window, cx));
    editor.update(cx, |e, cx| {
        let mut opts = e.options().clone();
        opts.sticky_scroll.enabled = false;
        e.update_options(opts, cx);
    });
    cx.update(|window, _| window.activate_window());
    cx.update(|window, cx| window.focus(&editor.focus_handle(cx), cx));
    paint(cx);
    (editor, cx)
}

fn paint(cx: &mut VisualTestContext) {
    cx.update(|window, _| window.refresh());
    cx.run_until_parked();
}

/// Points over the text, the gutter (line numbers), the minimap and the
/// vertical scrollbar's track, and the scrollbar's thumb.
struct Areas {
    text: Point<Pixels>,
    gutter: Point<Pixels>,
    minimap: Point<Pixels>,
    scrollbar: Point<Pixels>,
    thumb: Point<Pixels>,
}

fn areas(editor: &Entity<CodeEditor>, cx: &mut VisualTestContext) -> Areas {
    editor.read_with(cx, |e, _| {
        let (minimap, _, text_left) = e.minimap_for_tests().unwrap();
        let (bar, thumb) = e.vertical_scrollbar_for_tests().unwrap();
        Areas {
            text: point(text_left + px(40.), px(45.)),
            gutter: point(text_left - px(12.), px(45.)),
            minimap: point(minimap.center().x, minimap.bottom() - px(20.)),
            scrollbar: point(bar.center().x, bar.bottom() - px(10.)),
            thumb: thumb.center(),
        }
    })
}

/// A window point at `y` over the line numbers.
fn line_number(
    editor: &Entity<CodeEditor>,
    y: Pixels,
    cx: &mut VisualTestContext,
) -> Point<Pixels> {
    let mut x = px(1.);
    loop {
        let kind = editor.update_in(cx, |e, window, cx| {
            e.mouse_target(point(x, y), window, cx).kind
        });
        if kind == gpui_monaco::MouseTargetKind::GutterLineNumbers {
            return point(x, y);
        }
        x += px(1.);
        assert!(x < px(200.), "no line numbers");
    }
}

/// The pointer the editor shows at `at`.
fn pointer_at(
    editor: &Entity<CodeEditor>,
    at: Point<Pixels>,
    cx: &mut VisualTestContext,
) -> Option<CursorStyle> {
    editor.read_with(cx, |e, _| e.mouse_cursor_at_for_tests(at))
}

/// The pointer shown over the whole window, if a drag captured it.
fn captured(editor: &Entity<CodeEditor>, cx: &mut VisualTestContext) -> Option<CursorStyle> {
    editor.read_with(cx, |e, cx| e.captured_cursor_for_tests(cx))
}

fn assert_own_pointers(editor: &Entity<CodeEditor>, cx: &mut VisualTestContext) {
    let a = areas(editor, cx);
    assert_eq!(captured(editor, cx), None);
    assert_eq!(pointer_at(editor, a.text, cx), Some(CursorStyle::IBeam));
    assert_eq!(pointer_at(editor, a.gutter, cx), Some(CursorStyle::Arrow));
    assert_eq!(pointer_at(editor, a.minimap, cx), Some(CursorStyle::Arrow));
    assert_eq!(
        pointer_at(editor, a.scrollbar, cx),
        Some(CursorStyle::Arrow)
    );
}

fn press(at: Point<Pixels>, cx: &mut VisualTestContext) {
    cx.simulate_mouse_move(at, None, Modifiers::default());
    cx.simulate_mouse_down(at, MouseButton::Left, Modifiers::default());
    paint(cx);
}

fn drag_to(at: Point<Pixels>, cx: &mut VisualTestContext) {
    cx.simulate_mouse_move(at, MouseButton::Left, Modifiers::default());
    paint(cx);
}

fn release(at: Point<Pixels>, cx: &mut VisualTestContext) {
    cx.simulate_mouse_up(at, MouseButton::Left, Modifiers::default());
    paint(cx);
}

/// No drag: the I-beam over the text only.
#[gpui::test]
fn each_area_shows_its_own_pointer(cx: &mut TestAppContext) {
    let (editor, cx) = setup(cx);
    assert_own_pointers(&editor, cx);
}

/// A selection drag started on the text captures nothing: the I-beam
/// stays over the text, and the gutter, minimap and scrollbar keep their
/// arrow (not the I-beam) while the mouse is over them.
#[gpui::test]
fn a_text_selection_drag_keeps_each_areas_pointer(cx: &mut TestAppContext) {
    let (editor, cx) = setup(cx);
    let a = areas(&editor, cx);
    press(a.text, cx);
    let below = a.text + point(px(30.), px(40.));
    for over in [a.gutter, a.minimap, a.scrollbar, below] {
        drag_to(over, cx);
        assert_eq!(captured(&editor, cx), None);
        assert_eq!(pointer_at(&editor, a.text, cx), Some(CursorStyle::IBeam));
        assert_eq!(pointer_at(&editor, a.gutter, cx), Some(CursorStyle::Arrow));
        assert_eq!(pointer_at(&editor, a.minimap, cx), Some(CursorStyle::Arrow));
        assert_eq!(
            pointer_at(&editor, a.scrollbar, cx),
            Some(CursorStyle::Arrow)
        );
    }
    let selection = editor.read_with(cx, |e, _| e.selections()[0]);
    assert!(!selection.is_empty(), "the drag selected");
    release(below, cx);
    assert_own_pointers(&editor, cx);
}

/// Monaco captures the pointer on the scrollbar's slider: its arrow over
/// the whole window, the text included, until the release.
#[gpui::test]
fn a_scrollbar_drag_keeps_the_arrow_everywhere(cx: &mut TestAppContext) {
    let (editor, cx) = setup(cx);
    let a = areas(&editor, cx);
    press(a.thumb, cx);
    assert_eq!(captured(&editor, cx), Some(CursorStyle::Arrow));
    drag_to(a.text, cx);
    assert_eq!(captured(&editor, cx), Some(CursorStyle::Arrow));
    assert_eq!(pointer_at(&editor, a.text, cx), Some(CursorStyle::Arrow));
    // Outside the editor too.
    drag_to(point(px(-50.), px(-50.)), cx);
    assert_eq!(captured(&editor, cx), Some(CursorStyle::Arrow));
    release(a.text, cx);
    assert_own_pointers(&editor, cx);
}

/// The minimap's slider too.
#[gpui::test]
fn a_minimap_slider_drag_keeps_the_arrow_everywhere(cx: &mut TestAppContext) {
    let (editor, cx) = setup(cx);
    let (slider, _, _) = editor.read_with(cx, |e, _| e.minimap_slider_for_tests().unwrap());
    let a = areas(&editor, cx);
    press(slider.center(), cx);
    drag_to(a.text, cx);
    assert_eq!(captured(&editor, cx), Some(CursorStyle::Arrow));
    assert_eq!(pointer_at(&editor, a.text, cx), Some(CursorStyle::Arrow));
    release(a.text, cx);
    assert_own_pointers(&editor, cx);
}

/// A line selection started on the line numbers keeps their arrow over the
/// text.
#[gpui::test]
fn a_line_numbers_drag_keeps_the_arrow(cx: &mut TestAppContext) {
    let (editor, cx) = setup(cx);
    let a = areas(&editor, cx);
    let line_number = line_number(&editor, a.text.y, cx);
    press(line_number, cx);
    drag_to(a.text + point(px(0.), px(40.)), cx);
    assert_eq!(captured(&editor, cx), Some(CursorStyle::Arrow));
    assert_eq!(pointer_at(&editor, a.text, cx), Some(CursorStyle::Arrow));
    release(a.text, cx);
    assert_own_pointers(&editor, cx);
}

/// A capture ends when the release wasn't seen (the next move has no
/// button) and when the editor loses focus.
#[gpui::test]
fn a_cancelled_drag_gives_the_pointers_back(cx: &mut TestAppContext) {
    let (editor, cx) = setup(cx);
    let a = areas(&editor, cx);
    press(a.thumb, cx);
    drag_to(a.text, cx);
    assert_eq!(captured(&editor, cx), Some(CursorStyle::Arrow));
    cx.simulate_mouse_move(a.text, None, Modifiers::default());
    paint(cx);
    assert_own_pointers(&editor, cx);

    let a = areas(&editor, cx);
    press(a.thumb, cx);
    drag_to(a.text, cx);
    assert_eq!(captured(&editor, cx), Some(CursorStyle::Arrow));
    cx.update(|window, cx| window.blur(cx));
    paint(cx);
    assert_eq!(captured(&editor, cx), None);
    assert_eq!(pointer_at(&editor, a.text, cx), Some(CursorStyle::IBeam));
}

/// Dragging the selection keeps Monaco's `mouseStyle` (the arrow, or copy
/// with the modifier) over the editor, and drops back to the pointers on
/// release.
#[gpui::test]
fn dragging_the_selection_shows_the_mouse_style(cx: &mut TestAppContext) {
    let (editor, cx) = setup(cx);
    editor.update(cx, |e, cx| {
        e.set_selections(
            &[gpui_monaco::cursor::Selection {
                anchor: PointUtf16::new(1, 0),
                head: PointUtf16::new(1, 10),
            }],
            cx,
        )
    });
    paint(cx);
    let inside = editor
        .read_with(cx, |e, _| {
            e.scrolled_visible_position(PointUtf16::new(1, 3))
        })
        .unwrap()
        + point(px(2.), px(5.));
    press(inside, cx);
    drag_to(inside + point(px(1.), px(0.)), cx);
    let target = inside + point(px(0.), px(60.));
    drag_to(target, cx);
    assert_eq!(captured(&editor, cx), None);
    assert_eq!(pointer_at(&editor, target, cx), Some(CursorStyle::Arrow));
    release(target, cx);
    assert_own_pointers(&editor, cx);
}
