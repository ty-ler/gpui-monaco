//! Dragging the selection (Monaco `DragAndDropController`) and
//! `doubleClickSelectsBlock`.

use gpui::{
    AppContext, Entity, Focusable, Modifiers, MouseButton, MouseDownEvent, Pixels, Point,
    TestAppContext, VisualTestContext, point, px,
};
use gpui_monaco::cursor::Selection;
use gpui_monaco::text::PointUtf16;
use gpui_monaco::{CodeEditor, TextModel};

fn setup<'a>(
    text: &str,
    cx: &'a mut TestAppContext,
) -> (Entity<CodeEditor>, &'a mut VisualTestContext) {
    cx.update(|cx| {
        gpui_monaco::init(cx);
        gpui_monaco::set_language_registry(cx, gpui_monaco_languages::registry());
    });
    let text = text.to_string();
    let (editor, cx) = cx.add_window_view(|window, cx| {
        let model = cx.new(|_| TextModel::new(&text).with_language("rust"));
        CodeEditor::new(model, window, cx)
    });
    cx.update(|window, cx| window.focus(&editor.focus_handle(cx), cx));
    paint(cx);
    (editor, cx)
}

fn paint(cx: &mut VisualTestContext) {
    cx.update(|window, _| window.refresh());
    cx.run_until_parked();
}

/// The window point in the middle of the line, at `column`'s left edge
/// plus a bit.
fn at(
    editor: &Entity<CodeEditor>,
    cx: &mut VisualTestContext,
    row: u32,
    column: u32,
) -> Point<Pixels> {
    paint(cx);
    let p = editor.read_with(cx, |e, _| {
        e.scrolled_visible_position(PointUtf16::new(row, column))
            .unwrap()
    });
    point(p.x + px(1.), p.y + px(5.))
}

fn select(
    editor: &Entity<CodeEditor>,
    cx: &mut VisualTestContext,
    from: (u32, u32),
    to: (u32, u32),
) {
    editor.update(cx, |e, cx| {
        e.set_selections(
            &[Selection::new(
                PointUtf16::new(from.0, from.1),
                PointUtf16::new(to.0, to.1),
            )],
            cx,
        )
    });
}

fn text(editor: &Entity<CodeEditor>, cx: &mut VisualTestContext) -> String {
    editor.read_with(cx, |e, cx| e.model().read(cx).text())
}

fn drag(
    editor: &Entity<CodeEditor>,
    cx: &mut VisualTestContext,
    from: (u32, u32),
    to: (u32, u32),
    modifiers: Modifiers,
) {
    let start = at(editor, cx, from.0, from.1);
    let end = at(editor, cx, to.0, to.1);
    cx.simulate_mouse_down(start, MouseButton::Left, Modifiers::default());
    // Monaco picks the dragged selection on the first move, still on it.
    cx.simulate_mouse_move(
        point(start.x + px(2.), start.y),
        MouseButton::Left,
        modifiers,
    );
    cx.simulate_mouse_move(end, MouseButton::Left, modifiers);
    cx.simulate_mouse_up(end, MouseButton::Left, modifiers);
    paint(cx);
}

#[gpui::test]
fn dragging_moves_the_selection(cx: &mut TestAppContext) {
    let (editor, cx) = setup("one two three", cx);
    select(&editor, cx, (0, 4), (0, 8));
    drag(&editor, cx, (0, 5), (0, 13), Modifiers::default());
    assert_eq!(text(&editor, cx), "one threetwo ");
    let selection = editor.read_with(cx, |e, _| e.selections()[0]);
    assert_eq!(
        selection.range(),
        PointUtf16::new(0, 9)..PointUtf16::new(0, 13)
    );
}

#[gpui::test]
fn dragging_with_the_modifier_copies(cx: &mut TestAppContext) {
    let (editor, cx) = setup("one two three", cx);
    select(&editor, cx, (0, 4), (0, 8));
    let copy = if cfg!(target_os = "macos") {
        Modifiers::alt()
    } else {
        Modifiers::control()
    };
    drag(&editor, cx, (0, 5), (0, 0), copy);
    assert_eq!(text(&editor, cx), "two one two three");
}

#[gpui::test]
fn clicking_the_selection_without_dragging_places_the_cursor(cx: &mut TestAppContext) {
    let (editor, cx) = setup("one two three", cx);
    select(&editor, cx, (0, 4), (0, 8));
    let p = at(&editor, cx, 0, 6);
    cx.simulate_mouse_down(p, MouseButton::Left, Modifiers::default());
    cx.simulate_mouse_up(p, MouseButton::Left, Modifiers::default());
    let selection = editor.read_with(cx, |e, _| e.selections()[0]);
    assert!(selection.is_empty());
    assert_eq!(text(&editor, cx), "one two three");
}

#[gpui::test]
fn double_click_selects_blocks(cx: &mut TestAppContext) {
    let (editor, cx) = setup("f(a, b);\nlet s = \"hello world\";", cx);
    paint(cx);
    // Monaco counts a double click only after a click at the same place.
    let double_click = |cx: &mut VisualTestContext, position: Point<Pixels>| {
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
    };
    // Just inside the parentheses.
    let p = at(&editor, cx, 0, 2);
    double_click(cx, point(p.x - px(2.), p.y));
    let selection = editor.read_with(cx, |e, _| e.selections()[0]);
    assert_eq!(
        selection.range(),
        PointUtf16::new(0, 2)..PointUtf16::new(0, 6)
    );
    // Just inside the quotes.
    let p = at(&editor, cx, 1, 9);
    double_click(cx, point(p.x - px(2.), p.y));
    let selection = editor.read_with(cx, |e, _| e.selections()[0]);
    assert_eq!(
        selection.range(),
        PointUtf16::new(1, 9)..PointUtf16::new(1, 20)
    );
}
