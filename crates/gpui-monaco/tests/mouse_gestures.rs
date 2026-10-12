//! Mouse gestures: Monaco's `ViewController.dispatchMouse` matrix
//! (`browser/view/viewController.ts`), `MouseDownState`'s click counting
//! and `MouseDownOperation`'s drags (`browser/controller/mouseHandler.ts`).

use gpui::{
    Entity, Focusable, Modifiers, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent,
    Pixels, Point, TestAppContext, VisualTestContext, point, px,
};
use gpui_monaco::cursor::Selection;
use gpui_monaco::options::MultiCursorModifier;
use gpui_monaco::text::PointUtf16;
use gpui_monaco::{CodeEditor, EditorOptions};

fn setup<'a>(
    text: &str,
    options: impl FnOnce(&mut EditorOptions),
    cx: &'a mut TestAppContext,
) -> (Entity<CodeEditor>, &'a mut VisualTestContext) {
    cx.update(gpui_monaco::init);
    let text = text.to_string();
    let (editor, cx) = cx.add_window_view(|window, cx| CodeEditor::with_text(&text, window, cx));
    editor.update(cx, |e, cx| {
        let mut opts = e.options().clone();
        opts.sticky_scroll.enabled = false;
        opts.minimap.enabled = false;
        options(&mut opts);
        e.update_options(opts, cx);
    });
    cx.update(|window, cx| window.focus(&editor.focus_handle(cx), cx));
    paint(cx);
    (editor, cx)
}

fn paint(cx: &mut VisualTestContext) {
    cx.update(|window, _| window.refresh());
    cx.run_until_parked();
}

/// Monaco 1-based (line, column).
fn p(line: u32, column: u32) -> PointUtf16 {
    PointUtf16::from_monaco(line, column)
}

/// A selection from `anchor` to `head`, Monaco 1-based.
fn sel(anchor: (u32, u32), head: (u32, u32)) -> Selection {
    Selection {
        anchor: p(anchor.0, anchor.1),
        head: p(head.0, head.1),
    }
}

fn selections(editor: &Entity<CodeEditor>, cx: &mut VisualTestContext) -> Vec<Selection> {
    editor.read_with(cx, |e, _| e.selections())
}

fn set(editor: &Entity<CodeEditor>, selections: &[Selection], cx: &mut VisualTestContext) {
    editor.update(cx, |e, cx| e.set_selections(selections, cx));
    paint(cx);
}

/// A window point inside the character after `point` (Monaco 1-based).
fn at(
    editor: &Entity<CodeEditor>,
    line: u32,
    column: u32,
    cx: &mut VisualTestContext,
) -> Point<Pixels> {
    let top_left = editor
        .read_with(cx, |e, _| e.scrolled_visible_position(p(line, column)))
        .unwrap();
    point(top_left.x + px(2.), top_left.y + px(5.))
}

/// A window point over `line`'s line number.
fn line_number(
    editor: &Entity<CodeEditor>,
    line: u32,
    cx: &mut VisualTestContext,
) -> Point<Pixels> {
    let y = at(editor, line, 1, cx).y;
    let mut x = px(1.);
    loop {
        let kind = editor.update_in(cx, |e, window, cx| {
            e.mouse_target(point(x, y), window, cx).kind
        });
        if kind == gpui_monaco::MouseTargetKind::GutterLineNumbers {
            return point(x, y);
        }
        x += px(2.);
        assert!(x < px(200.), "no line numbers");
    }
}

fn down(
    position: Point<Pixels>,
    button: MouseButton,
    modifiers: Modifiers,
    click_count: usize,
    cx: &mut VisualTestContext,
) {
    cx.simulate_event(MouseDownEvent {
        position,
        button,
        modifiers,
        click_count,
        first_mouse: false,
    });
}

fn up(
    position: Point<Pixels>,
    button: MouseButton,
    modifiers: Modifiers,
    cx: &mut VisualTestContext,
) {
    cx.simulate_event(MouseUpEvent {
        position,
        button,
        modifiers,
        click_count: 1,
    });
    paint(cx);
}

/// `count` clicks at `position`, as the platform reports them (1, 2, ...).
fn clicks(position: Point<Pixels>, modifiers: Modifiers, count: usize, cx: &mut VisualTestContext) {
    for click_count in 1..=count {
        down(position, MouseButton::Left, modifiers, click_count, cx);
        up(position, MouseButton::Left, modifiers, cx);
    }
}

fn drag_to(
    position: Point<Pixels>,
    button: MouseButton,
    modifiers: Modifiers,
    cx: &mut VisualTestContext,
) {
    cx.simulate_event(MouseMoveEvent {
        position,
        pressed_button: Some(button),
        modifiers,
    });
    paint(cx);
}

fn shift() -> Modifiers {
    Modifiers {
        shift: true,
        ..Default::default()
    }
}

fn alt() -> Modifiers {
    Modifiers {
        alt: true,
        ..Default::default()
    }
}

fn alt_shift() -> Modifiers {
    Modifiers {
        alt: true,
        shift: true,
        ..Default::default()
    }
}

fn secondary_shift() -> Modifiers {
    Modifiers {
        shift: true,
        ..Modifiers::secondary_key()
    }
}

const TEXT: &str = "foo bar baz qux\nline two here\nline three here\nline four here\nlast";

/// Monaco `mouseDownCount >= 4`: select all.
#[gpui::test]
fn quadruple_click_selects_all(cx: &mut TestAppContext) {
    let (editor, cx) = setup(TEXT, |_| {}, cx);
    let target = at(&editor, 2, 3, cx);
    clicks(target, Modifiers::default(), 3, cx);
    assert_eq!(selections(&editor, cx), vec![sel((2, 1), (3, 1))]);
    clicks(target, Modifiers::default(), 4, cx);
    assert_eq!(selections(&editor, cx), vec![sel((1, 1), (5, 5))]);
}

/// Monaco `MouseDownState.trySetCount`: a click at another position starts
/// counting again, and so does a click more than 400ms later.
#[gpui::test]
fn click_count_resets_at_another_position_or_after_400ms(cx: &mut TestAppContext) {
    let (editor, cx) = setup(TEXT, |_| {}, cx);
    let first = at(&editor, 1, 2, cx);
    down(first, MouseButton::Left, Modifiers::default(), 1, cx);
    up(first, MouseButton::Left, Modifiers::default(), cx);
    // The platform says double click, but on another word: a single click.
    let second = at(&editor, 1, 6, cx);
    down(second, MouseButton::Left, Modifiers::default(), 2, cx);
    up(second, MouseButton::Left, Modifiers::default(), cx);
    assert_eq!(selections(&editor, cx), vec![sel((1, 6), (1, 6))]);
    // A double click there now selects the word.
    down(second, MouseButton::Left, Modifiers::default(), 2, cx);
    up(second, MouseButton::Left, Modifiers::default(), cx);
    assert_eq!(selections(&editor, cx), vec![sel((1, 5), (1, 8))]);
    // Too late for a triple click.
    cx.executor()
        .advance_clock(std::time::Duration::from_millis(500));
    down(second, MouseButton::Left, Modifiers::default(), 3, cx);
    up(second, MouseButton::Left, Modifiers::default(), cx);
    assert_eq!(selections(&editor, cx), vec![sel((1, 6), (1, 6))]);
}

/// Monaco `_moveToSelect` keeps a double click's word selection: Shift+click
/// extends it by words, from the word clicked first.
#[gpui::test]
fn shift_click_after_a_double_click_extends_by_words(cx: &mut TestAppContext) {
    let (editor, cx) = setup(TEXT, |_| {}, cx);
    clicks(at(&editor, 1, 10, cx), Modifiers::default(), 2, cx);
    assert_eq!(selections(&editor, cx), vec![sel((1, 9), (1, 12))]);
    // Forwards: to the end of `qux`.
    clicks(at(&editor, 1, 14, cx), shift(), 1, cx);
    assert_eq!(selections(&editor, cx), vec![sel((1, 9), (1, 16))]);
    // Backwards: from the end of `baz` to the start of `foo`.
    clicks(at(&editor, 1, 2, cx), shift(), 1, cx);
    assert_eq!(selections(&editor, cx), vec![sel((1, 12), (1, 1))]);
}

/// Monaco `_wordSelectDrag`: Shift+double-click extends the selection to
/// the word.
#[gpui::test]
fn shift_double_click_extends_by_words(cx: &mut TestAppContext) {
    let (editor, cx) = setup(TEXT, |_| {}, cx);
    set(&editor, &[Selection::collapsed(p(1, 5))], cx);
    clicks(at(&editor, 1, 14, cx), shift(), 2, cx);
    assert_eq!(selections(&editor, cx), vec![sel((1, 5), (1, 16))]);
}

/// Monaco `_lineSelectDrag`: Shift+triple-click extends the selection by
/// lines.
#[gpui::test]
fn shift_triple_click_extends_by_lines(cx: &mut TestAppContext) {
    let (editor, cx) = setup(TEXT, |_| {}, cx);
    set(&editor, &[Selection::collapsed(p(2, 3))], cx);
    clicks(at(&editor, 4, 3, cx), shift(), 3, cx);
    assert_eq!(selections(&editor, cx), vec![sel((2, 3), (5, 1))]);
}

/// Monaco `startedOnLineNumbers` with Shift: `_lineSelectDrag`, extending
/// the line selection.
#[gpui::test]
fn shift_click_on_line_numbers_extends_by_lines(cx: &mut TestAppContext) {
    let (editor, cx) = setup(TEXT, |_| {}, cx);
    clicks(line_number(&editor, 2, cx), Modifiers::default(), 1, cx);
    assert_eq!(selections(&editor, cx), vec![sel((2, 1), (3, 1))]);
    clicks(line_number(&editor, 4, cx), shift(), 1, cx);
    assert_eq!(selections(&editor, cx), vec![sel((2, 1), (5, 1))]);
    // Upwards from the line first clicked: its end is the anchor.
    clicks(line_number(&editor, 1, cx), shift(), 1, cx);
    assert_eq!(selections(&editor, cx), vec![sel((3, 1), (1, 1))]);
}

/// Monaco: dragging from the line numbers selects lines, even over the
/// text.
#[gpui::test]
fn drag_from_line_numbers_selects_lines(cx: &mut TestAppContext) {
    let (editor, cx) = setup(TEXT, |_| {}, cx);
    let start = line_number(&editor, 1, cx);
    down(start, MouseButton::Left, Modifiers::default(), 1, cx);
    drag_to(
        at(&editor, 3, 4, cx),
        MouseButton::Left,
        Modifiers::default(),
        cx,
    );
    up(
        at(&editor, 3, 4, cx),
        MouseButton::Left,
        Modifiers::default(),
        cx,
    );
    assert_eq!(selections(&editor, cx), vec![sel((1, 1), (4, 1))]);
}

/// The default `multiCursorModifier: alt`: Shift+Alt drag column-selects.
#[gpui::test]
fn shift_alt_drag_column_selects(cx: &mut TestAppContext) {
    let (editor, cx) = setup(TEXT, |_| {}, cx);
    set(&editor, &[Selection::collapsed(p(2, 2))], cx);
    let start = at(&editor, 2, 2, cx);
    down(start, MouseButton::Left, alt_shift(), 1, cx);
    let end = at(&editor, 4, 5, cx);
    drag_to(end, MouseButton::Left, alt_shift(), cx);
    up(end, MouseButton::Left, alt_shift(), cx);
    assert_eq!(
        selections(&editor, cx),
        vec![
            sel((2, 2), (2, 5)),
            sel((3, 2), (3, 5)),
            sel((4, 2), (4, 5))
        ]
    );
}

/// Monaco's mouse `columnSelect` goes by view lines: a fold is one line,
/// so the drag puts no cursors inside it.
#[gpui::test]
fn column_select_drag_skips_folded_lines(cx: &mut TestAppContext) {
    let (editor, cx) = setup("a {\n  b\n  c\n  d\n  e\n}x", |_| {}, cx);
    set(&editor, &[Selection::collapsed(p(1, 1))], cx);
    editor.update(cx, |e, cx| e.fold(cx));
    paint(cx);
    let start = at(&editor, 1, 1, cx);
    down(start, MouseButton::Left, alt_shift(), 1, cx);
    let end = at(&editor, 6, 2, cx);
    drag_to(end, MouseButton::Left, alt_shift(), cx);
    up(end, MouseButton::Left, alt_shift(), cx);
    assert_eq!(
        selections(&editor, cx),
        vec![sel((1, 1), (1, 2)), sel((6, 1), (6, 2))]
    );
}

/// Monaco's mouse `columnSelect` goes by view lines: each segment of a
/// wrapped line gets a cursor.
#[gpui::test]
fn column_select_drag_goes_by_wrapped_segments(cx: &mut TestAppContext) {
    let (editor, cx) = setup(
        "x\naaaa bbbb cccc dddd\ny",
        |o| {
            o.word_wrap = gpui_monaco::options::WordWrap::WordWrapColumn;
            o.word_wrap_column = 10;
        },
        cx,
    );
    set(&editor, &[Selection::collapsed(p(2, 2))], cx);
    let start = at(&editor, 2, 2, cx);
    down(start, MouseButton::Left, alt_shift(), 1, cx);
    // Column 14 is on the second segment, three columns in.
    let end = at(&editor, 2, 14, cx);
    drag_to(end, MouseButton::Left, alt_shift(), cx);
    up(end, MouseButton::Left, alt_shift(), cx);
    assert_eq!(
        selections(&editor, cx),
        vec![sel((2, 2), (2, 4)), sel((2, 12), (2, 14))]
    );
}

/// Monaco: Shift+Alt+click column-selects from the cursor to the click.
#[gpui::test]
fn shift_alt_click_column_selects_from_the_cursor(cx: &mut TestAppContext) {
    let (editor, cx) = setup(TEXT, |_| {}, cx);
    set(&editor, &[Selection::collapsed(p(2, 2))], cx);
    clicks(at(&editor, 3, 4, cx), alt_shift(), 1, cx);
    assert_eq!(
        selections(&editor, cx),
        vec![sel((2, 2), (2, 4)), sel((3, 2), (3, 4))]
    );
}

/// With `multiCursorModifier: ctrlCmd`, Cmd/Ctrl+Shift+click column-selects
/// and an Alt drag column-selects too.
#[gpui::test]
fn column_select_follows_the_multi_cursor_modifier(cx: &mut TestAppContext) {
    let (editor, cx) = setup(
        TEXT,
        |o| o.multi_cursor_modifier = MultiCursorModifier::CtrlCmd,
        cx,
    );
    set(&editor, &[Selection::collapsed(p(2, 2))], cx);
    clicks(at(&editor, 3, 4, cx), secondary_shift(), 1, cx);
    assert_eq!(
        selections(&editor, cx),
        vec![sel((2, 2), (2, 4)), sel((3, 2), (3, 4))]
    );

    // An Alt drag: the click places the cursor, the drag column-selects.
    let start = at(&editor, 1, 2, cx);
    down(start, MouseButton::Left, alt(), 1, cx);
    assert_eq!(selections(&editor, cx), vec![sel((1, 2), (1, 2))]);
    let end = at(&editor, 3, 6, cx);
    drag_to(end, MouseButton::Left, alt(), cx);
    up(end, MouseButton::Left, alt(), cx);
    assert_eq!(
        selections(&editor, cx),
        vec![
            sel((1, 2), (1, 6)),
            sel((2, 2), (2, 6)),
            sel((3, 2), (3, 6))
        ]
    );

    // Shift+Alt+click extends with Alt held: a column selection too.
    set(&editor, &[Selection::collapsed(p(1, 1))], cx);
    clicks(at(&editor, 2, 3, cx), alt_shift(), 1, cx);
    assert_eq!(
        selections(&editor, cx),
        vec![sel((1, 1), (1, 3)), sel((2, 1), (2, 3))]
    );
    // A plain Shift+click extends the selection.
    set(&editor, &[Selection::collapsed(p(1, 1))], cx);
    clicks(at(&editor, 2, 3, cx), shift(), 1, cx);
    assert_eq!(selections(&editor, cx), vec![sel((1, 1), (2, 3))]);
}

/// Monaco `_columnSelect` for the middle button (without
/// `scrollOnMiddleClick`): a middle drag column-selects.
#[gpui::test]
fn middle_drag_column_selects(cx: &mut TestAppContext) {
    let (editor, cx) = setup(TEXT, |_| {}, cx);
    let start = at(&editor, 1, 3, cx);
    down(start, MouseButton::Middle, Modifiers::default(), 1, cx);
    let end = at(&editor, 2, 5, cx);
    drag_to(end, MouseButton::Middle, Modifiers::default(), cx);
    up(end, MouseButton::Middle, Modifiers::default(), cx);
    assert_eq!(
        selections(&editor, cx),
        vec![sel((1, 3), (1, 5)), sel((2, 3), (2, 5))]
    );
}

/// Monaco `CreateCursor`: the multi-cursor modifier adds a cursor, or
/// removes the one clicked; with a double click the new cursor selects its
/// word (`_lastCursorWordSelect`), and dragging extends it
/// (`_lastCursorMoveToSelect`).
#[gpui::test]
fn alt_click_adds_and_removes_cursors(cx: &mut TestAppContext) {
    let (editor, cx) = setup(TEXT, |_| {}, cx);
    set(&editor, &[Selection::collapsed(p(1, 1))], cx);
    clicks(at(&editor, 2, 3, cx), alt(), 1, cx);
    assert_eq!(
        selections(&editor, cx),
        vec![sel((1, 1), (1, 1)), sel((2, 3), (2, 3))]
    );
    // Again on the new cursor: it goes.
    cx.executor()
        .advance_clock(std::time::Duration::from_millis(500));
    clicks(at(&editor, 2, 3, cx), alt(), 1, cx);
    assert_eq!(selections(&editor, cx), vec![sel((1, 1), (1, 1))]);

    // Alt+double-click: a cursor selecting the word.
    cx.executor()
        .advance_clock(std::time::Duration::from_millis(500));
    clicks(at(&editor, 3, 7, cx), alt(), 2, cx);
    assert_eq!(
        selections(&editor, cx),
        vec![sel((1, 1), (1, 1)), sel((3, 6), (3, 11))]
    );

    // Alt+drag: a new cursor selecting to the mouse.
    cx.executor()
        .advance_clock(std::time::Duration::from_millis(500));
    let start = at(&editor, 4, 1, cx);
    down(start, MouseButton::Left, alt(), 1, cx);
    let end = at(&editor, 4, 5, cx);
    drag_to(end, MouseButton::Left, alt(), cx);
    up(end, MouseButton::Left, alt(), cx);
    assert_eq!(
        selections(&editor, cx),
        vec![
            sel((1, 1), (1, 1)),
            sel((3, 6), (3, 11)),
            sel((4, 1), (4, 5))
        ]
    );
}

/// Monaco `_onMouseDownThenMove` re-reads the modifiers: releasing Shift
/// mid-drag ends the column selection and drags the last cursor.
#[gpui::test]
fn drag_follows_the_current_modifiers(cx: &mut TestAppContext) {
    let (editor, cx) = setup(TEXT, |_| {}, cx);
    set(&editor, &[Selection::collapsed(p(1, 1))], cx);
    let start = at(&editor, 1, 1, cx);
    down(start, MouseButton::Left, alt_shift(), 1, cx);
    drag_to(at(&editor, 2, 3, cx), MouseButton::Left, alt_shift(), cx);
    assert_eq!(selections(&editor, cx).len(), 2);
    // Without Shift (Alt still held), the last added cursor drags.
    drag_to(at(&editor, 2, 6, cx), MouseButton::Left, alt(), cx);
    up(at(&editor, 2, 6, cx), MouseButton::Left, alt(), cx);
    assert_eq!(
        selections(&editor, cx),
        vec![sel((1, 1), (1, 3)), sel((2, 1), (2, 6))]
    );
}

fn scroll(editor: &Entity<CodeEditor>, cx: &mut VisualTestContext) -> (Pixels, Pixels) {
    editor.read_with(cx, |e, _| (e.scroll_top(), e.scroll_left()))
}

/// Monaco `TopBottomDragScrolling`: dragging below the editor scrolls every
/// frame while the mouse rests, faster the farther away it is, and the
/// selection follows to the last visible line.
#[gpui::test]
fn drag_below_the_editor_keeps_scrolling(cx: &mut TestAppContext) {
    let text: Vec<String> = (1..=2000).map(|i| format!("line {i}")).collect();
    let (editor, cx) = setup(&text.join("\n"), |_| {}, cx);
    let height = editor.read_with(cx, |e, _| e.view_metrics_for_tests().unwrap().2);
    let start = at(&editor, 1, 2, cx);
    down(start, MouseButton::Left, Modifiers::default(), 1, cx);
    // Just below: one move, then the mouse rests.
    drag_to(
        point(start.x, height + px(30.)),
        MouseButton::Left,
        Modifiers::default(),
        cx,
    );
    let before = scroll(&editor, cx).0;
    cx.executor()
        .advance_clock(std::time::Duration::from_millis(100));
    paint(cx);
    let near = scroll(&editor, cx).0 - before;
    assert!(near > px(0.), "no scrolling while the mouse rests");
    // The selection reaches the last visible line.
    let head = selections(&editor, cx)[0].head;
    assert!(head.row > 10, "{head:?}");

    // Much farther away: faster.
    drag_to(
        point(start.x, height + px(400.)),
        MouseButton::Left,
        Modifiers::default(),
        cx,
    );
    let before = scroll(&editor, cx).0;
    cx.executor()
        .advance_clock(std::time::Duration::from_millis(100));
    paint(cx);
    let far = scroll(&editor, cx).0 - before;
    assert!(far > near * 2., "{far:?} vs {near:?}");

    // Releasing the button stops it.
    up(
        point(start.x, height + px(400.)),
        MouseButton::Left,
        Modifiers::default(),
        cx,
    );
    let stopped = scroll(&editor, cx).0;
    cx.executor()
        .advance_clock(std::time::Duration::from_millis(100));
    paint(cx);
    assert_eq!(scroll(&editor, cx).0, stopped);
}

/// Monaco `LeftRightDragScrolling`: dragging right of the text scrolls
/// horizontally and selects to the line's end.
#[gpui::test]
fn drag_right_of_the_text_scrolls_horizontally(cx: &mut TestAppContext) {
    let long = "x".repeat(2000);
    let (editor, cx) = setup(&format!("{long}\n{long}"), |_| {}, cx);
    let start = at(&editor, 1, 2, cx);
    down(start, MouseButton::Left, Modifiers::default(), 1, cx);
    let right = editor.read_with(cx, |e, _| {
        let info = e.layout_info().unwrap();
        info.content_left + info.content_width
    });
    drag_to(
        point(right + px(20.), start.y),
        MouseButton::Left,
        Modifiers::default(),
        cx,
    );
    cx.executor()
        .advance_clock(std::time::Duration::from_millis(100));
    paint(cx);
    assert!(scroll(&editor, cx).1 > px(0.));
    assert_eq!(selections(&editor, cx), vec![sel((1, 2), (1, 2001))]);
    up(
        point(right + px(20.), start.y),
        MouseButton::Left,
        Modifiers::default(),
        cx,
    );
}
