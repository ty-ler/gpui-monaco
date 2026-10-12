//! Injected text in view columns (#298), as Monaco's
//! `ModelLineProjectionData` puts it: selections are drawn and hit-tested
//! in view columns, wrapping counts injected text, and Up/Down keep the
//! visible column with injected text included.

use gpui::{
    AppContext as _, Entity, Focusable as _, Modifiers, Pixels, TestAppContext, VisualTestContext,
    px,
};
use gpui_monaco::cursor::Selection;
use gpui_monaco::options::{WordWrap, WrappingStrategy};
use gpui_monaco::text::PointUtf16;
use gpui_monaco::{
    CodeEditor, DecorationOptions, InjectedText, InjectedTextCursorStops, TextModel,
};

fn p(row: u32, column: u32) -> PointUtf16 {
    PointUtf16::new(row, column)
}

fn setup<'a>(
    text: &str,
    cx: &'a mut TestAppContext,
) -> (Entity<CodeEditor>, &'a mut VisualTestContext) {
    cx.update(gpui_monaco::init);
    let text = text.to_string();
    let (editor, cx) = cx.add_window_view(|window, cx| {
        let model = cx.new(|_| TextModel::new(&text));
        CodeEditor::new(model, window, cx)
    });
    cx.update(|window, cx| window.focus(&editor.focus_handle(cx), cx));
    draw(cx);
    (editor, cx)
}

fn draw(cx: &mut VisualTestContext) {
    cx.update(|window, cx| {
        window.refresh();
        window.draw(cx).clear(cx);
    });
    cx.run_until_parked();
}

/// `content` injected before byte `at` (an empty range), with `stops`.
fn inject(
    editor: &Entity<CodeEditor>,
    at: usize,
    content: &str,
    stops: InjectedTextCursorStops,
    cx: &mut VisualTestContext,
) {
    let options = DecorationOptions {
        before: Some(InjectedText {
            content: content.to_string().into(),
            cursor_stops: stops,
            ..Default::default()
        }),
        show_if_collapsed: true,
        ..Default::default()
    };
    editor.update(cx, |e, cx| {
        e.model().update(cx, |model, cx| {
            model.delta_decorations(None, &[], vec![(at..at, options)], cx)
        })
    });
    draw(cx);
}

/// Where `point`'s character starts (after injected text before it).
fn x_of(editor: &Entity<CodeEditor>, point: PointUtf16, cx: &mut VisualTestContext) -> Pixels {
    editor.read_with(cx, |e, _| e.scrolled_visible_position(point).unwrap().x)
}

fn y_of(editor: &Entity<CodeEditor>, row: u32, cx: &mut VisualTestContext) -> Pixels {
    editor.read_with(cx, |e, _| e.scrolled_visible_position(p(row, 0)).unwrap().y) + px(2.)
}

fn select(editor: &Entity<CodeEditor>, selection: Selection, cx: &mut VisualTestContext) {
    editor.update(cx, |e, cx| e.set_selections(&[selection], cx));
    draw(cx);
}

fn head(editor: &Entity<CodeEditor>, cx: &mut VisualTestContext) -> PointUtf16 {
    editor.read_with(cx, |e, _| e.selections()[0].head)
}

/// The drawn selection's (left, right) on each view line.
fn spans(editor: &Entity<CodeEditor>, cx: &mut VisualTestContext) -> Vec<(u32, Pixels, Pixels)> {
    draw(cx);
    editor.read_with(cx, |e, _| e.selection_spans_for_tests())
}

fn close(a: Pixels, b: Pixels) -> bool {
    (a - b).abs() < px(0.5)
}

/// Monaco draws selections in view columns: one over injected text covers
/// it; one ending at its column (a model position, before it) doesn't; one
/// whose head moved past it does.
#[gpui::test]
fn selections_are_drawn_in_view_columns(cx: &mut TestAppContext) {
    let (editor, cx) = setup("ab\ncd", cx);
    let plain_b = x_of(&editor, p(0, 1), cx);
    inject(&editor, 1, "XYZ", InjectedTextCursorStops::Both, cx);
    let a = x_of(&editor, p(0, 0), cx);
    let b = x_of(&editor, p(0, 1), cx);
    let end = x_of(&editor, p(0, 2), cx);
    assert!(b > plain_b);
    // Across the text.
    select(&editor, Selection::new(p(0, 0), p(0, 2)), cx);
    let drawn = spans(&editor, cx);
    assert_eq!(drawn.len(), 1);
    assert!(close(drawn[0].1, a) && close(drawn[0].2, end), "{drawn:?}");
    // From its column: the model position is before it, so it's covered.
    select(&editor, Selection::new(p(0, 1), p(0, 2)), cx);
    let drawn = spans(&editor, cx);
    assert!(
        close(drawn[0].1, plain_b) && close(drawn[0].2, end),
        "{drawn:?}"
    );
    // To its column: it isn't.
    select(&editor, Selection::new(p(0, 0), p(0, 1)), cx);
    let drawn = spans(&editor, cx);
    assert!(
        close(drawn[0].1, a) && close(drawn[0].2, plain_b),
        "{drawn:?}"
    );
    // Shift+Right steps over it, and the selection then covers it.
    select(&editor, Selection::collapsed(p(0, 0)), cx);
    cx.simulate_keystrokes("shift-right");
    assert_eq!(head(&editor, cx), p(0, 1));
    let drawn = spans(&editor, cx);
    assert!(close(drawn[0].1, a) && close(drawn[0].2, b), "{drawn:?}");
    // Across lines, it continues past the end of the first.
    select(&editor, Selection::new(p(0, 1), p(1, 1)), cx);
    let drawn = spans(&editor, cx);
    assert_eq!(drawn.len(), 2);
    assert!(close(drawn[0].1, plain_b) && drawn[0].2 > end, "{drawn:?}");
}

/// Selections are hit-tested in view columns: a Shift+click on injected
/// text extends to the side of it the cursor stops allow, and the drawn
/// selection ends there.
#[gpui::test]
fn selections_are_hit_tested_in_view_columns(cx: &mut TestAppContext) {
    for (stops, covers) in [
        (InjectedTextCursorStops::Both, true),
        (InjectedTextCursorStops::Left, false),
    ] {
        let (editor, cx) = setup("ab\ncd", cx);
        let plain_b = x_of(&editor, p(0, 1), cx);
        inject(&editor, 1, "XYZ", stops, cx);
        let a = x_of(&editor, p(0, 0), cx);
        let b = x_of(&editor, p(0, 1), cx);
        let y = y_of(&editor, 0, cx);
        select(&editor, Selection::collapsed(p(0, 0)), cx);
        // Just inside the text's right edge.
        cx.simulate_click(gpui::point(b - px(1.), y), Modifiers::shift());
        draw(cx);
        assert_eq!(
            editor.read_with(cx, |e, _| e.selections()[0]),
            Selection::new(p(0, 0), p(0, 1))
        );
        let drawn = spans(&editor, cx);
        let to = if covers { b } else { plain_b };
        assert!(
            close(drawn[0].1, a) && close(drawn[0].2, to),
            "{stops:?} {drawn:?}"
        );
        // Just inside its left edge: before it. (From a collapsed cursor:
        // a click inside the selection would start dragging it.)
        select(&editor, Selection::collapsed(p(0, 0)), cx);
        cx.simulate_click(gpui::point(plain_b + px(1.), y), Modifiers::shift());
        draw(cx);
        let drawn = spans(&editor, cx);
        assert!(close(drawn[0].2, plain_b), "{stops:?} {drawn:?}");
    }
}

/// Monaco's line breaks computer counts injected text (`createLineBreaks`
/// with injected text), with either wrapping strategy, and may break
/// inside it.
#[gpui::test]
fn wrapping_counts_injected_text(cx: &mut TestAppContext) {
    for strategy in [WrappingStrategy::Simple, WrappingStrategy::Advanced] {
        let (editor, cx) = setup("aaaa bbbb cccc\nend", cx);
        editor.update(cx, |e, cx| {
            e.update_options_with(
                |o| {
                    o.word_wrap = WordWrap::WordWrapColumn;
                    o.word_wrap_column = 16;
                    o.wrapping_strategy = strategy;
                },
                cx,
            )
        });
        draw(cx);
        let segments = |cx: &mut VisualTestContext| {
            draw(cx);
            editor.read_with(cx, |e, _| e.view_line_segments_for_tests())
        };
        assert_eq!(segments(cx), vec![(0, 0..14), (1, 0..3)], "{strategy:?}");
        // A hint after `aaaa` wraps the row (with monospace columns,
        // before `cccc`; measured, where the test font's widths say).
        inject(&editor, 4, ": hint", InjectedTextCursorStops::Right, cx);
        let hinted = segments(cx);
        assert_eq!(hinted.len(), 3, "{strategy:?} {hinted:?}");
        if strategy == WrappingStrategy::Simple {
            assert_eq!(hinted, vec![(0, 0..10), (0, 10..14), (1, 0..3)]);
        }
        // A long one wraps inside itself: the first view line ends in it.
        inject(
            &editor,
            0,
            "a long piece of injected text",
            InjectedTextCursorStops::Both,
            cx,
        );
        let wrapped = segments(cx);
        assert_eq!(wrapped[0], (0, 0..0), "{strategy:?} {wrapped:?}");
        assert!(wrapped.len() >= 4, "{strategy:?} {wrapped:?}");
        // The cursor at the end of the row is on its last view line.
        select(&editor, Selection::collapsed(p(0, 14)), cx);
        let line = editor.read_with(cx, |e, _| e.cursor_view_positions_for_tests()[0].0);
        assert_eq!(line as usize, wrapped.len() - 2, "{strategy:?}");
    }
}

/// Monaco `MoveOperations.vertical` on view lines: the visible column
/// counts injected text (`CursorColumns` on the view line's content), and
/// moving back keeps it.
#[gpui::test]
fn up_and_down_keep_the_visible_column_with_injected_text(cx: &mut TestAppContext) {
    let (editor, cx) = setup("abcdefgh\nabcdefgh\nabcdefgh", cx);
    // "aXYZbcdefgh" on the middle row.
    inject(&editor, 10, "XYZ", InjectedTextCursorStops::Both, cx);
    select(&editor, Selection::collapsed(p(0, 5)), cx);
    let x = x_of(&editor, p(0, 5), cx);
    cx.simulate_keystrokes("down");
    // View column 5 is `c` (model column 2), right below.
    assert_eq!(head(&editor, cx), p(1, 2));
    assert!(close(x_of(&editor, p(1, 2), cx), x));
    cx.simulate_keystrokes("down");
    assert_eq!(head(&editor, cx), p(2, 5));
    cx.simulate_keystrokes("up up");
    assert_eq!(head(&editor, cx), p(0, 5));
    // Landing inside the text: up goes before it, down after it.
    select(&editor, Selection::collapsed(p(0, 2)), cx);
    cx.simulate_keystrokes("down");
    assert_eq!(head(&editor, cx), p(1, 1));
    let caret = |cx: &mut VisualTestContext| {
        draw(cx);
        editor.read_with(cx, |e, _| e.cursor_shapes_for_tests().0[0].left())
    };
    let after = caret(cx);
    assert!(close(after, x_of(&editor, p(1, 1), cx)) || after > x_of(&editor, p(1, 0), cx));
    select(&editor, Selection::collapsed(p(2, 2)), cx);
    cx.simulate_keystrokes("up");
    assert_eq!(head(&editor, cx), p(1, 1));
    let before = caret(cx);
    assert!(before < after);
    // And the leftover keeps the column for the next line.
    cx.simulate_keystrokes("up");
    assert_eq!(head(&editor, cx), p(0, 2));
}
