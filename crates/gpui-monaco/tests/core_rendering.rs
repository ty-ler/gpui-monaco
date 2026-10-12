//! Core rendering from Monaco's view parts: cursor view positions at wrap
//! points (`CursorState.viewState`, ported from `cursor.test.ts`), cursor
//! geometry (`viewCursor.ts`), control characters (`viewLineRenderer.ts`)
//! and the selection's shape (`selections.ts`, `viewLines.ts`).

use gpui::{
    Entity, Focusable, Modifiers, MouseButton, MouseDownEvent, MouseUpEvent, Pixels,
    TestAppContext, VisualTestContext, point, px,
};
use gpui_monaco::cursor::Selection;
use gpui_monaco::options::{CursorStyle, WordWrap};
use gpui_monaco::text::PointUtf16;
use gpui_monaco::view::line_breaks::WrappingIndent;
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

/// Equal up to float rounding.
#[track_caller]
fn assert_px(a: Pixels, b: Pixels) {
    assert!((a - b).abs() < px(0.01), "{a:?} != {b:?}");
}

fn paint(cx: &mut VisualTestContext) {
    cx.update(|window, _| window.refresh());
    cx.run_until_parked();
}

fn wrap_at(column: u32) -> impl FnOnce(&mut EditorOptions) {
    move |o: &mut EditorOptions| {
        o.word_wrap = WordWrap::WordWrapColumn;
        o.word_wrap_column = column;
    }
}

/// Monaco 1-based (line, column).
fn p(line: u32, column: u32) -> PointUtf16 {
    PointUtf16::from_monaco(line, column)
}

fn set(editor: &Entity<CodeEditor>, selections: &[Selection], cx: &mut VisualTestContext) {
    editor.update(cx, |e, cx| e.set_selections(selections, cx));
    paint(cx);
}

fn keys(keys: &str, cx: &mut VisualTestContext) {
    cx.simulate_keystrokes(keys);
    paint(cx);
}

fn selections(editor: &Entity<CodeEditor>, cx: &mut VisualTestContext) -> Vec<Selection> {
    editor.read_with(cx, |e, _| e.selections())
}

fn head(editor: &Entity<CodeEditor>, cx: &mut VisualTestContext) -> PointUtf16 {
    selections(editor, cx)[0].head
}

/// The primary cursor's view line.
fn view_line(editor: &Entity<CodeEditor>, cx: &mut VisualTestContext) -> u32 {
    editor.read_with(cx, |e, _| e.cursor_view_positions_for_tests()[0].0)
}

fn cursors(editor: &Entity<CodeEditor>, cx: &mut VisualTestContext) -> Vec<gpui::Bounds<Pixels>> {
    paint(cx);
    editor.read_with(cx, |e, _| e.cursor_shapes_for_tests().0)
}

/// The window position of the top-left of `point`'s character.
fn position_of(
    editor: &Entity<CodeEditor>,
    point: PointUtf16,
    cx: &mut VisualTestContext,
) -> gpui::Point<Pixels> {
    editor
        .read_with(cx, |e, _| e.scrolled_visible_position(point))
        .unwrap()
}

#[gpui::test]
fn end_keeps_the_caret_on_a_wrapped_segment(cx: &mut TestAppContext) {
    // "aaaa bbbb " | "cccc dddd"
    let (editor, cx) = setup("aaaa bbbb cccc dddd", wrap_at(10), cx);
    let line_height = editor.read_with(cx, |e, _| e.view_metrics_for_tests().unwrap().3);
    let first_top = cursors(&editor, cx)[0].top();

    // Monaco `_moveToLineEnd`: the end of the view line, drawn there, not at
    // the start of the next one (the same model position).
    keys("end", cx);
    assert_eq!(head(&editor, cx), p(1, 11));
    assert_eq!(view_line(&editor, cx), 0);
    let caret = cursors(&editor, cx)[0];
    assert_eq!(caret.top(), first_top);
    // At the end of "aaaa bbbb ", right of where "cccc" starts.
    assert!(caret.left() > position_of(&editor, p(1, 10), cx).x);

    // At the end of the view line, End goes to the model line's end.
    keys("end", cx);
    assert_eq!(head(&editor, cx), p(1, 20));
    assert_eq!(view_line(&editor, cx), 1);
    assert_eq!(cursors(&editor, cx)[0].top(), first_top + line_height);

    // Home: the view line's start (the wrap point again, now drawn on the
    // second view line), then the model line's.
    keys("home", cx);
    assert_eq!(head(&editor, cx), p(1, 11));
    assert_eq!(view_line(&editor, cx), 1);
    keys("home", cx);
    assert_eq!(head(&editor, cx), p(1, 1));

    // Shift+End selects to the end of the view line.
    keys("shift-end", cx);
    assert_eq!(
        selections(&editor, cx),
        vec![Selection::new(p(1, 1), p(1, 11))]
    );
    assert_eq!(view_line(&editor, cx), 0);

    // Setting the selection from the model forgets the view position.
    set(&editor, &[Selection::collapsed(p(1, 11))], cx);
    assert_eq!(view_line(&editor, cx), 1);

    // Typing does too, as Monaco's `fromModelState`.
    set(&editor, &[Selection::collapsed(p(1, 1))], cx);
    keys("end x", cx);
    assert_eq!(head(&editor, cx), p(1, 12));
}

/// Monaco `cursor.test.ts` "issue #144041: Cursor up/down works", in model
/// positions (our continuation indent is part of the view line, so view
/// columns inside it differ) and view lines.
#[gpui::test]
fn issue_144041_cursor_up_down_on_wrapped_lines(cx: &mut TestAppContext) {
    let (editor, cx) = setup(
        "Word1 Word2 Word3 Word4\nWord5 Word6 Word7 Word8",
        |o| {
            wrap_at(20)(o);
            o.wrapping_indent = WrappingIndent::Indent;
        },
        cx,
    );
    let mut reported = vec![(head(&editor, cx), view_line(&editor, cx))];
    for key in ["down", "down", "down", "down", "up", "up", "up", "up"] {
        keys(key, cx);
        reported.push((head(&editor, cx), view_line(&editor, cx)));
    }
    assert_eq!(
        reported,
        vec![
            (p(1, 1), 0),
            (p(1, 19), 1),
            (p(2, 1), 2),
            (p(2, 19), 3),
            (p(2, 24), 3),
            (p(2, 1), 2),
            (p(1, 19), 1),
            (p(1, 1), 0),
            (p(1, 1), 0),
        ]
    );
}

/// Monaco `cursor.test.ts` "issue #36740: wordwrap creates an extra step /
/// character at the wrapping point": Right and Left step over a wrap point,
/// where two view positions are one model position (`normalizePosition`).
#[gpui::test]
fn issue_36740_no_extra_step_at_wrap_points(cx: &mut TestAppContext) {
    let (editor, cx) = setup(
        "Lorem ipsum dolor sit amet consectetur adipiscing elit",
        wrap_at(16),
        cx,
    );
    set(&editor, &[Selection::collapsed(p(1, 7))], cx);
    for column in 8..=13 {
        keys("right", cx);
        assert_eq!(head(&editor, cx), p(1, column));
    }
    // Moving right never stops at a wrapped segment's end: the caret is at
    // the start of view line 2.
    assert_eq!(view_line(&editor, cx), 1);
    keys("right", cx);
    assert_eq!(head(&editor, cx), p(1, 14));
    keys("left", cx);
    assert_eq!(head(&editor, cx), p(1, 13));
    assert_eq!(view_line(&editor, cx), 1);
    // Moving back to view line 1.
    keys("left", cx);
    assert_eq!(head(&editor, cx), p(1, 12));
    assert_eq!(view_line(&editor, cx), 0);
}

/// Monaco `cursor.test.ts` "issue #110376: multiple selections with
/// wordwrap behave differently".
#[gpui::test]
fn issue_110376_multiple_selections_with_word_wrap(cx: &mut TestAppContext) {
    let (editor, cx) = setup(
        "just a sentence. just a sentence. just a sentence.",
        wrap_at(25),
        cx,
    );
    let initial = [
        Selection::new(p(1, 1), p(1, 16)),
        Selection::new(p(1, 18), p(1, 33)),
        Selection::new(p(1, 35), p(1, 50)),
    ];
    set(&editor, &initial, cx);
    keys("left", cx);
    assert_eq!(
        selections(&editor, cx),
        vec![
            Selection::collapsed(p(1, 1)),
            Selection::collapsed(p(1, 18)),
            Selection::collapsed(p(1, 35)),
        ]
    );
    set(&editor, &initial, cx);
    keys("right", cx);
    assert_eq!(
        selections(&editor, cx),
        vec![
            Selection::collapsed(p(1, 16)),
            Selection::collapsed(p(1, 33)),
            Selection::collapsed(p(1, 50)),
        ]
    );
}

/// Monaco `cursor.test.ts` "issue #98320: Multi-Cursor, Wrap lines and
/// cursorSelectRight ==> cursors out of sync".
#[gpui::test]
fn issue_98320_select_right_across_wrap_points(cx: &mut TestAppContext) {
    let (editor, cx) = setup(
        "lorem_ipsum-1993x11x13\ndolor_sit_amet-1998x04x27\nconsectetur-2007x10x08\nadipiscing-2012x07x27\nelit-2015x02x27",
        wrap_at(16),
        cx,
    );
    let starts = [(1, 13), (2, 16), (3, 13), (4, 12), (5, 6)];
    set(
        &editor,
        &starts
            .iter()
            .map(|&(l, c)| Selection::collapsed(p(l, c)))
            .collect::<Vec<_>>(),
        cx,
    );
    for n in 1..=4 {
        keys("shift-right", cx);
        assert_eq!(
            selections(&editor, cx),
            starts
                .iter()
                .map(|&(l, c)| Selection::new(p(l, c), p(l, c + n)))
                .collect::<Vec<_>>(),
            "after {n}"
        );
    }
}

/// A click past the end of a wrapped segment leaves the caret there
/// (Monaco's mouse targets are view positions).
#[gpui::test]
fn click_past_a_wrapped_segment(cx: &mut TestAppContext) {
    let (editor, cx) = setup("aaaa bbbb cccc dddd", wrap_at(10), cx);
    let start = position_of(&editor, p(1, 1), cx);
    let line_height = editor.read_with(cx, |e, _| e.view_metrics_for_tests().unwrap().3);
    let target = point(start.x + px(300.), start.y + line_height / 2.);
    let modifiers = Modifiers::default();
    cx.simulate_event(MouseDownEvent {
        position: target,
        button: MouseButton::Left,
        modifiers,
        click_count: 1,
        first_mouse: false,
    });
    cx.simulate_event(MouseUpEvent {
        position: target,
        button: MouseButton::Left,
        modifiers,
        click_count: 1,
    });
    paint(cx);
    assert_eq!(head(&editor, cx), p(1, 11));
    assert_eq!(view_line(&editor, cx), 0);
    assert_eq!(cursors(&editor, cx)[0].top(), start.y);
}

/// Monaco `ViewCursor._prepareRender`: a 2px line cursor is shifted 1px
/// left, between the characters; a block cursor on a tab is a half-width
/// character wide.
#[gpui::test]
fn cursor_geometry(cx: &mut TestAppContext) {
    let (editor, cx) = setup("ab\tc", |_| {}, cx);
    set(&editor, &[Selection::collapsed(p(1, 2))], cx);
    let x = position_of(&editor, p(1, 2), cx).x;
    let caret = cursors(&editor, cx)[0];
    assert_px(caret.left(), x - px(1.));
    assert_px(caret.size.width, px(2.));
    // Not at the very left of the text.
    set(&editor, &[Selection::collapsed(p(1, 1))], cx);
    let x0 = position_of(&editor, p(1, 1), cx).x;
    assert_px(cursors(&editor, cx)[0].left(), x0);

    editor.update(cx, |e, cx| {
        let mut options = e.options().clone();
        options.cursor_style = CursorStyle::Block;
        e.update_options(options, cx);
    });
    set(&editor, &[Selection::collapsed(p(1, 3))], cx);
    let char_width = position_of(&editor, p(1, 2), cx).x - x0;
    let tab = cursors(&editor, cx)[0];
    assert_px(tab.size.width, char_width);
    assert!(position_of(&editor, p(1, 4), cx).x - position_of(&editor, p(1, 3), cx).x > char_width);
    // On a character: as wide as it.
    set(&editor, &[Selection::collapsed(p(1, 1))], cx);
    assert_px(cursors(&editor, cx)[0].size.width, char_width);
}

/// Monaco's DOM line-break computer (`wrappingStrategy: advanced`): with
/// `wrappingIndent: indent`, a line indented by 2 continues 2 + 4 columns
/// (of spaces) in, where the monospace computer rounds to the tab stop (4).
#[gpui::test]
fn advanced_wrapping_indents_by_tab_size(cx: &mut TestAppContext) {
    use gpui_monaco::options::WrappingStrategy;
    let text = "  aaa bbb ccc ddd eee fff ggg hhh iii jjj kkk lll mmm";
    let (editor, cx) = setup(
        text,
        |o| {
            wrap_at(20)(o);
            o.wrapping_indent = WrappingIndent::Indent;
        },
        cx,
    );
    // Tab size 4 (not the 2 detected from the text).
    let model = editor.read_with(cx, |e, _| e.model().clone());
    model.update(cx, |m, cx| {
        let mut options = m.options().clone();
        options.tab_size = 4;
        m.update_options(options, cx);
    });
    let indent = |strategy, cx: &mut VisualTestContext| {
        editor.update(cx, |e, cx| {
            let mut options = e.options().clone();
            options.wrapping_strategy = strategy;
            e.update_options(options, cx);
        });
        paint(cx);
        let segments = editor.read_with(cx, |e, _| e.view_line_segments_for_tests());
        assert!(segments.len() > 1, "{segments:?}");
        position_of(&editor, p(1, segments[1].1.start + 1), cx).x
            - position_of(&editor, p(1, 1), cx).x
    };
    let simple = indent(WrappingStrategy::Simple, cx);
    let advanced = indent(WrappingStrategy::Advanced, cx);
    assert_px(advanced, simple / 4. * 6.);
}

/// Monaco `ViewCursor._getGraphemeAwarePosition`: a block cursor covers the
/// whole grapheme after it (a decomposed `é`), and inside a grapheme it
/// draws at the grapheme's start.
#[gpui::test]
fn block_cursor_covers_a_grapheme(cx: &mut TestAppContext) {
    let (editor, cx) = setup("e\u{301}x", |o| o.cursor_style = CursorStyle::Block, cx);
    set(&editor, &[Selection::collapsed(p(1, 1))], cx);
    let start = position_of(&editor, p(1, 1), cx).x;
    let end = position_of(&editor, p(1, 3), cx).x;
    let block = cursors(&editor, cx)[0];
    assert_px(block.left(), start);
    assert_px(block.size.width, end - start);
    // Between `e` and the accent: the same block.
    set(&editor, &[Selection::collapsed(p(1, 2))], cx);
    let inside = cursors(&editor, cx)[0];
    assert_px(inside.left(), start);
    assert_px(inside.size.width, end - start);
    // On `x`: one character.
    set(&editor, &[Selection::collapsed(p(1, 3))], cx);
    let x = cursors(&editor, cx)[0];
    assert_px(x.left(), end);
    assert_px(x.size.width, position_of(&editor, p(1, 4), cx).x - end);
}

/// Monaco `renderControlCharacters`: directional formatting characters show
/// as `[U+202E]`, eight columns wide (so they can't reorder the line), and
/// C0 controls as their control pictures; tabs after them keep to the tab
/// stops.
#[gpui::test]
fn control_characters(cx: &mut TestAppContext) {
    let (editor, cx) = setup("x\u{202e}y\u{7}\t;", |_| {}, cx);
    let x = |column: u32, cx: &mut VisualTestContext| position_of(&editor, p(1, column), cx).x;
    let char_width = x(2, cx) - x(1, cx);
    assert_px(x(3, cx) - x(2, cx), char_width * 8.);
    assert_px(x(4, cx) - x(3, cx), char_width);
    assert_px(x(5, cx) - x(4, cx), char_width);
    // The tab after 11 columns goes to column 12.
    assert_px(x(6, cx) - x(5, cx), char_width);
    // Off, they're the raw characters.
    editor.update(cx, |e, cx| {
        let mut options = e.options().clone();
        options.render_control_characters = false;
        e.update_options(options, cx);
    });
    paint(cx);
    assert!(x(3, cx) - x(2, cx) < char_width * 8.);
}

/// Monaco `_renderLine` (issue #22832): a full-width character takes two
/// visible columns and other characters one per UTF-16 unit (an astral
/// emoji two), so the tabs after them stop at the tab stops.
#[gpui::test]
fn tabs_after_wide_characters(cx: &mut TestAppContext) {
    let text = "abcd\tX\n中\tX\nab\tX\n😀\tX\nasd = \"擦\"\t\t#asd";
    let (editor, cx) = setup(text, |_| {}, cx);
    let x = |line: u32, column: u32, cx: &mut VisualTestContext| {
        position_of(&editor, p(line, column), cx).x
    };
    let char_width = x(1, 2, cx) - x(1, 1, cx);
    // After 4 columns: to column 8.
    assert_px(x(1, 6, cx) - x(1, 5, cx), char_width * 4.);
    // After 2 columns (`中`, `ab` and `😀`): to column 4.
    assert_px(x(2, 3, cx) - x(2, 2, cx), char_width * 2.);
    assert_px(x(3, 4, cx) - x(3, 3, cx), char_width * 2.);
    assert_px(x(4, 4, cx) - x(4, 3, cx), char_width * 2.);
    // Monaco `issue-22832-1`: `asd = "擦"` is 10 columns, so the tabs
    // are 2 and 4 wide.
    assert_px(x(5, 11, cx) - x(5, 10, cx), char_width * 2.);
    assert_px(x(5, 12, cx) - x(5, 11, cx), char_width * 4.);
}

/// Monaco `ViewLayout._computeContentWidth`: wrapping at the viewport, the
/// content is as wide as its widest line (no `scrollBeyondLastColumn`), so
/// neither End nor a sideways wheel scrolls the wrapped text.
#[gpui::test]
fn word_wrap_doesnt_scroll_horizontally(cx: &mut TestAppContext) {
    let text = "a".repeat(300);
    let (editor, cx) = setup(&text, |o| o.word_wrap = WordWrap::On, cx);
    let scroll_left = |cx: &mut VisualTestContext| editor.read_with(cx, |e, _| e.scroll_left());
    // End on each wrapped segment, to the end of the line.
    for _ in 0..10 {
        keys("end", cx);
        assert_eq!(scroll_left(cx), px(0.));
        keys("right", cx);
    }
    assert_eq!(head(&editor, cx), p(1, 301));
    cx.simulate_event(gpui::ScrollWheelEvent {
        position: point(px(200.), px(20.)),
        delta: gpui::ScrollDelta::Pixels(point(px(-40.), px(0.))),
        ..Default::default()
    });
    paint(cx);
    assert_eq!(scroll_left(cx), px(0.));
    let (content, viewport) = editor.read_with(cx, |e, _| {
        (
            e.content_width().unwrap(),
            e.layout_info().unwrap().content_width,
        )
    });
    assert!(content <= viewport, "{content:?} > {viewport:?}");

    // Wrapping at a column isn't viewport wrapping: the content keeps
    // `scrollBeyondLastColumn` and scrolls when the column doesn't fit.
    editor.update(cx, |e, cx| {
        let mut options = e.options().clone();
        options.word_wrap = WordWrap::WordWrapColumn;
        options.word_wrap_column = 290;
        e.update_options(options, cx);
    });
    set(&editor, &[Selection::collapsed(p(1, 1))], cx);
    keys("end", cx);
    assert!(scroll_left(cx) > px(0.), "viewport {viewport:?}");
}
