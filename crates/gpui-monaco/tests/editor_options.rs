//! Editor and model options from Monaco's `editorOptions.ts` and
//! `ITextModelCreationOptions`: custom and interval line numbers, cursor
//! styles, selecting from line numbers, the mouse, minimap autohide, word
//! wrap overrides and break rules, selected text matching, large files, the
//! default line ending, the word wrap indicator, the horizontal scrollbar in
//! the content height, inertial scrolling, two-cell full-width characters
//! and accessibility support.

use std::cell::RefCell;
use std::rc::Rc;

use gpui::{
    AppContext as _, Entity, Focusable, Modifiers, MouseButton, MouseDownEvent, Pixels,
    ScrollDelta, ScrollWheelEvent, TestAppContext, VisualTestContext, point, px,
};
use gpui_monaco::cursor::Selection;
use gpui_monaco::options::{
    AccessibilitySupport, CursorStyle, FullwidthCharacterWidth, LineNumbers, MinimapAutohide,
    MouseMiddleClickAction, MouseStyle, SelectedTextMatchMode, WordBreak, WordWrap,
    WordWrapOverride, WrappingStrategy,
};
use gpui_monaco::text::{LineEnding, PointUtf16};
use gpui_monaco::{CodeEditor, CodeEditorEvent, EditorOptions, ModelOptions, TextModel};

fn setup<'a>(
    text: &str,
    options: impl FnOnce(&mut EditorOptions),
    cx: &'a mut TestAppContext,
) -> (Entity<CodeEditor>, &'a mut VisualTestContext) {
    cx.update(gpui_monaco::init);
    let text = text.to_string();
    let (editor, cx) = cx.add_window_view(|window, cx| CodeEditor::with_text(&text, window, cx));
    set_options(&editor, options, cx);
    cx.update(|window, cx| window.focus(&editor.focus_handle(cx), cx));
    paint(cx);
    (editor, cx)
}

fn set_options(
    editor: &Entity<CodeEditor>,
    options: impl FnOnce(&mut EditorOptions),
    cx: &mut VisualTestContext,
) {
    editor.update(cx, |e, cx| {
        let mut opts = e.options().clone();
        opts.sticky_scroll.enabled = false;
        options(&mut opts);
        e.update_options(opts, cx);
    });
    paint(cx);
}

fn paint(cx: &mut VisualTestContext) {
    cx.update(|window, _| window.refresh());
    cx.run_until_parked();
}

fn p(row: u32, column: u32) -> PointUtf16 {
    PointUtf16::new(row, column)
}

fn lines(n: usize) -> String {
    (1..=n)
        .map(|i| format!("line {i}"))
        .collect::<Vec<_>>()
        .join("\n")
}

fn labels(editor: &Entity<CodeEditor>, cx: &mut VisualTestContext) -> Vec<(u32, String)> {
    paint(cx);
    editor.read_with(cx, |e, _| e.line_number_labels_for_tests())
}

#[gpui::test]
fn custom_line_numbers(cx: &mut TestAppContext) {
    let (editor, cx) = setup(
        "a\nb\nc",
        |o| {
            // Monaco `lineNumbers: (n) => ...`; an empty label skips the line.
            o.line_numbers = LineNumbers::custom(|n| {
                if n == 2 {
                    "".into()
                } else {
                    format!("L{n}").into()
                }
            })
        },
        cx,
    );
    assert_eq!(
        labels(&editor, cx),
        vec![(0, "L1".to_string()), (2, "L3".to_string())]
    );
}

#[gpui::test]
fn interval_line_numbers(cx: &mut TestAppContext) {
    let (editor, cx) = setup(&lines(25), |o| o.line_numbers = LineNumbers::Interval, cx);
    editor.update(cx, |e, cx| {
        e.set_selections(&[Selection::collapsed(p(4, 0))], cx)
    });
    // Monaco: every 10th line, the cursor's line and the last line.
    let rows: Vec<u32> = labels(&editor, cx).into_iter().map(|(r, _)| r).collect();
    assert_eq!(rows, vec![4, 9, 19, 24]);
}

#[gpui::test]
fn block_outline_and_underline_thin_cursors(cx: &mut TestAppContext) {
    let (editor, cx) = setup("abc", |o| o.cursor_style = CursorStyle::Block, cx);
    let (block, outlined) = editor.read_with(cx, |e, _| e.cursor_shapes_for_tests());
    assert!(!outlined);
    set_options(&editor, |o| o.cursor_style = CursorStyle::BlockOutline, cx);
    let (outline, outlined) = editor.read_with(cx, |e, _| e.cursor_shapes_for_tests());
    // The same box, outlined.
    assert!(outlined);
    assert_eq!(outline, block);

    set_options(&editor, |o| o.cursor_style = CursorStyle::Underline, cx);
    let (underline, _) = editor.read_with(cx, |e, _| e.cursor_shapes_for_tests());
    set_options(&editor, |o| o.cursor_style = CursorStyle::UnderlineThin, cx);
    let (thin, outlined) = editor.read_with(cx, |e, _| e.cursor_shapes_for_tests());
    assert!(!outlined);
    assert_eq!(underline[0].size.height, px(2.));
    assert_eq!(thin[0].size.height, px(1.));
    // Both at the bottom of the line, as wide as the character.
    assert_eq!(thin[0].bottom(), underline[0].bottom());
    assert_eq!(thin[0].size.width, underline[0].size.width);
}

/// The window y of `row` and an x in its line numbers.
fn line_number_point(
    editor: &Entity<CodeEditor>,
    row: u32,
    cx: &mut VisualTestContext,
) -> gpui::Point<Pixels> {
    let y = editor
        .read_with(cx, |e, _| e.scrolled_visible_position(p(row, 0)))
        .unwrap()
        .y
        + px(2.);
    // Find an x over the line numbers.
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

fn mouse_down(at: gpui::Point<Pixels>, button: MouseButton, cx: &mut VisualTestContext) {
    cx.simulate_event(MouseDownEvent {
        position: at,
        button,
        modifiers: Modifiers::default(),
        click_count: 1,
        first_mouse: false,
    });
    cx.simulate_event(gpui::MouseUpEvent {
        position: at,
        button,
        modifiers: Modifiers::default(),
        click_count: 1,
    });
    paint(cx);
}

#[gpui::test]
fn select_on_line_numbers(cx: &mut TestAppContext) {
    let (editor, cx) = setup("one\ntwo\nthree", |_| {}, cx);
    let at = line_number_point(&editor, 1, cx);
    mouse_down(at, MouseButton::Left, cx);
    // The whole line, through the next line's start.
    assert_eq!(
        editor.read_with(cx, |e, _| e.selections()[0]),
        Selection {
            anchor: p(1, 0),
            head: p(2, 0)
        }
    );

    // Off: clicking the line numbers doesn't select.
    set_options(&editor, |o| o.select_on_line_numbers = false, cx);
    editor.update(cx, |e, cx| {
        e.set_selections(&[Selection::collapsed(p(0, 1))], cx)
    });
    let at = line_number_point(&editor, 2, cx);
    mouse_down(at, MouseButton::Left, cx);
    assert_eq!(
        editor.read_with(cx, |e, _| e.selections()[0]),
        Selection::collapsed(p(0, 1))
    );

    // Nor does the glyph margin (Monaco only selects from line numbers).
    set_options(&editor, |o| o.select_on_line_numbers = true, cx);
    let y = line_number_point(&editor, 2, cx).y;
    mouse_down(point(px(2.), y), MouseButton::Left, cx);
    assert_eq!(
        editor.read_with(cx, |e, _| e.selections()[0]),
        Selection::collapsed(p(0, 1))
    );
}

#[gpui::test]
fn mouse_style(cx: &mut TestAppContext) {
    let (editor, cx) = setup("text", |_| {}, cx);
    let cursor =
        |cx: &mut VisualTestContext| editor.read_with(cx, |e, _| e.text_mouse_cursor_for_tests());
    assert_eq!(cursor(cx), gpui::CursorStyle::IBeam);
    set_options(&editor, |o| o.mouse_style = MouseStyle::Default, cx);
    assert_eq!(cursor(cx), gpui::CursorStyle::Arrow);
    set_options(&editor, |o| o.mouse_style = MouseStyle::Copy, cx);
    assert_eq!(cursor(cx), gpui::CursorStyle::DragCopy);
}

#[gpui::test]
fn middle_click_opens_links(cx: &mut TestAppContext) {
    let (editor, cx) = setup("see https://a.io now", |_| {}, cx);
    let opened = Rc::new(RefCell::new(Vec::new()));
    let sink = opened.clone();
    cx.update(|_, cx| {
        cx.subscribe(&editor, move |_, event, _| {
            if let CodeEditorEvent::OpenLink(url) = event {
                sink.borrow_mut().push(url.to_string());
            }
        })
        .detach()
    });
    paint(cx);
    let at = editor
        .read_with(cx, |e, _| e.scrolled_visible_position(p(0, 8)))
        .unwrap()
        + point(px(1.), px(2.));
    // The default middle click doesn't open links.
    mouse_down(at, MouseButton::Middle, cx);
    assert!(opened.borrow().is_empty());
    set_options(
        &editor,
        |o| o.mouse_middle_click_action = MouseMiddleClickAction::OpenLink,
        cx,
    );
    mouse_down(at, MouseButton::Middle, cx);
    assert_eq!(*opened.borrow(), vec!["https://a.io".to_string()]);
    set_options(
        &editor,
        |o| o.mouse_middle_click_action = MouseMiddleClickAction::CtrlLeftClick,
        cx,
    );
    mouse_down(at, MouseButton::Middle, cx);
    assert_eq!(opened.borrow().len(), 2);
}

#[gpui::test]
fn minimap_autohide(cx: &mut TestAppContext) {
    let (editor, cx) = setup(&lines(300), |_| {}, cx);
    let visibility = |cx: &mut VisualTestContext| {
        paint(cx);
        editor
            .read_with(cx, |e, _| e.minimap_visibility_for_tests())
            .unwrap()
    };
    assert_eq!(visibility(cx), (1., true));
    // `mouseover`: hidden until hovered, but it still takes the mouse.
    set_options(
        &editor,
        |o| o.minimap.autohide = MinimapAutohide::Mouseover,
        cx,
    );
    assert_eq!(visibility(cx), (0., true));
    // `scroll`: hidden and click-through until the view scrolls.
    set_options(
        &editor,
        |o| o.minimap.autohide = MinimapAutohide::Scroll,
        cx,
    );
    assert_eq!(visibility(cx), (0., false));
    cx.simulate_event(ScrollWheelEvent {
        position: point(px(100.), px(100.)),
        delta: ScrollDelta::Pixels(point(px(0.), px(-200.))),
        ..Default::default()
    });
    // Shown (fading in) and taking the mouse; the fade out half a second
    // later runs on the real clock (see the `MinimapFade` unit test).
    let (_, interactive) = visibility(cx);
    assert!(interactive);
}

/// The number of view lines `row` wraps into.
fn segments(editor: &Entity<CodeEditor>, row: u32, cx: &mut VisualTestContext) -> Vec<String> {
    paint(cx);
    let text = editor.read_with(cx, |e, cx| e.model().read(cx).buffer().rope().line(row));
    editor.read_with(cx, |e, _| {
        e.view_line_segments_for_tests()
            .into_iter()
            .filter(|(r, _)| *r == row)
            .map(|(_, columns)| {
                let units: Vec<u16> = text.encode_utf16().collect();
                String::from_utf16_lossy(&units[columns.start as usize..columns.end as usize])
            })
            .collect()
    })
}

#[gpui::test]
fn word_wrap_overrides(cx: &mut TestAppContext) {
    let text = "aaaa bbbb cccc dddd eeee";
    let (editor, cx) = setup(
        text,
        |o| {
            o.word_wrap = WordWrap::WordWrapColumn;
            o.word_wrap_column = 10;
        },
        cx,
    );
    assert_eq!(
        segments(&editor, 0, cx),
        vec!["aaaa bbbb ", "cccc dddd ", "eeee"]
    );
    // Monaco `wordWrapOverride1: off` turns wrapping off.
    set_options(
        &editor,
        |o| o.word_wrap_override1 = WordWrapOverride::Off,
        cx,
    );
    assert_eq!(segments(&editor, 0, cx).len(), 1);
    // `wordWrapOverride2` beats `wordWrapOverride1`; `on` wraps at the
    // viewport.
    let resolve = |o: &mut EditorOptions, one, two| {
        o.word_wrap_override1 = one;
        o.word_wrap_override2 = two;
        o.effective_word_wrap()
    };
    let mut o = EditorOptions::default();
    let (inherit, on, off) = (
        WordWrapOverride::Inherit,
        WordWrapOverride::On,
        WordWrapOverride::Off,
    );
    assert_eq!(resolve(&mut o, inherit, inherit), WordWrap::Off);
    assert_eq!(resolve(&mut o, on, inherit), WordWrap::On);
    assert_eq!(resolve(&mut o, on, off), WordWrap::Off);
    o.word_wrap = WordWrap::Bounded;
    assert_eq!(resolve(&mut o, inherit, inherit), WordWrap::Bounded);
    assert_eq!(resolve(&mut o, off, on), WordWrap::On);
}

#[gpui::test]
fn word_wrap_break_characters_and_word_break(cx: &mut TestAppContext) {
    let (editor, cx) = setup(
        "aaaa.bbbb.cccc",
        |o| {
            o.word_wrap = WordWrap::WordWrapColumn;
            o.word_wrap_column = 6;
        },
        cx,
    );
    // `.` is a default break-after character.
    assert_eq!(segments(&editor, 0, cx), vec!["aaaa.", "bbbb.", "cccc"]);
    set_options(
        &editor,
        |o| o.word_wrap_break_after_characters = "".into(),
        cx,
    );
    assert_eq!(segments(&editor, 0, cx), vec!["aaaa.b", "bbb.cc", "cc"]);
    set_options(
        &editor,
        |o| o.word_wrap_break_before_characters = ".".into(),
        cx,
    );
    assert_eq!(segments(&editor, 0, cx), vec!["aaaa", ".bbbb", ".cccc"]);

    // `wordBreak: keepAll` keeps CJK runs together.
    editor.update(cx, |e, cx| {
        e.model().update(cx, |m, cx| m.set_value("aa 你好你好", cx))
    });
    set_options(
        &editor,
        |o| {
            o.word_wrap_break_before_characters =
                gpui_monaco::view::line_breaks::DEFAULT_BREAK_BEFORE.into();
            o.word_wrap_break_after_characters =
                gpui_monaco::view::line_breaks::DEFAULT_BREAK_AFTER.into();
            o.word_wrap_column = 6;
        },
        cx,
    );
    assert_eq!(segments(&editor, 0, cx), vec!["aa 你", "好你好"]);
    set_options(&editor, |o| o.word_break = WordBreak::KeepAll, cx);
    assert_eq!(segments(&editor, 0, cx), vec!["aa ", "你好你", "好"]);
}

#[gpui::test]
fn wrap_on_escaped_line_feeds(cx: &mut TestAppContext) {
    let (editor, cx) = setup(
        r#"let s = "one\ntwo";"#,
        |o| {
            o.word_wrap = WordWrap::WordWrapColumn;
            o.word_wrap_column = 80;
        },
        cx,
    );
    assert_eq!(segments(&editor, 0, cx).len(), 1);
    set_options(&editor, |o| o.wrap_on_escaped_line_feeds = true, cx);
    // Continuation lines keep the line's (zero) indentation.
    assert_eq!(
        segments(&editor, 0, cx),
        vec![r#"let s = "one\n"#.to_string(), r#"two";"#.to_string()]
    );
}

#[gpui::test]
fn selected_text_match_mode(cx: &mut TestAppContext) {
    let (editor, cx) = setup("foo Foo foo", |_| {}, cx);
    let select_next = |cx: &mut VisualTestContext| {
        editor.update(cx, |e, cx| {
            e.set_selections(
                &[Selection {
                    anchor: p(0, 0),
                    head: p(0, 3),
                }],
                cx,
            );
            e.add_selection_to_next_find_match(cx);
            e.selections()
        })
    };
    // By default selected text matches with the find state's match case,
    // off until set.
    assert_eq!(select_next(cx)[1].start(), p(0, 4));
    set_options(
        &editor,
        |o| o.selected_text_match_mode = SelectedTextMatchMode::CaseInsensitive,
        cx,
    );
    assert_eq!(select_next(cx)[1].start(), p(0, 4));
    set_options(
        &editor,
        |o| o.selected_text_match_mode = SelectedTextMatchMode::CaseSensitive,
        cx,
    );
    assert_eq!(select_next(cx)[1].start(), p(0, 8));
}

#[gpui::test]
fn large_file_optimizations(cx: &mut TestAppContext) {
    // Monaco's 300K line threshold.
    let text = "x y z\n".repeat(300_001);
    let large = cx.new(|_| TextModel::new(&text));
    assert!(large.read_with(cx, |m, _| m.is_too_large_for_tokenization()));
    let off = cx.new(|_| {
        TextModel::new(&text).with_options(ModelOptions {
            large_file_optimizations: false,
            ..ModelOptions::default()
        })
    });
    assert!(!off.read_with(cx, |m, _| m.is_too_large_for_tokenization()));
    assert!(
        !cx.new(|_| TextModel::new("small"))
            .read_with(cx, |m, _| m.is_too_large_for_tokenization())
    );

    // Such models don't wrap.
    cx.update(gpui_monaco::init);
    let (editor, cx) = cx.add_window_view(|window, cx| CodeEditor::new(large, window, cx));
    set_options(
        &editor,
        |o| {
            o.word_wrap = WordWrap::WordWrapColumn;
            o.word_wrap_column = 2;
        },
        cx,
    );
    assert_eq!(segments(&editor, 0, cx).len(), 1);
}

#[gpui::test]
fn default_eol(cx: &mut TestAppContext) {
    let crlf = ModelOptions {
        default_eol: LineEnding::Crlf,
        ..ModelOptions::default()
    };
    // Monaco `defaultEOL`: for text without line breaks.
    let model = cx.new(|_| TextModel::new("abc").with_options(crlf.clone()));
    assert_eq!(
        model.read_with(cx, |m, _| m.line_ending()),
        LineEnding::Crlf
    );
    let model2 = cx.new(|_| TextModel::new("a\nb").with_options(crlf.clone()));
    assert_eq!(model2.read_with(cx, |m, _| m.line_ending()), LineEnding::Lf);
    // `setValue` detects again, like Monaco's new text buffer.
    model2.update(cx, |m, cx| m.set_value("one line", cx));
    assert_eq!(
        model2.read_with(cx, |m, _| m.line_ending()),
        LineEnding::Crlf
    );
    model2.update(cx, |m, cx| m.set_value("a\nb\nc", cx));
    assert_eq!(model2.read_with(cx, |m, _| m.line_ending()), LineEnding::Lf);
    let lf = cx.new(|_| TextModel::new("abc"));
    lf.update(cx, |m, cx| m.set_value("a\r\nb", cx));
    assert_eq!(lf.read_with(cx, |m, _| m.text()), "a\r\nb");
}

/// Monaco `wordWrapIndicator.test.ts`: the rows (and columns, in ems) of
/// the `↩` glyphs.
fn indicators(editor: &Entity<CodeEditor>, cx: &mut VisualTestContext) -> Vec<(u32, f32)> {
    paint(cx);
    let em = editor.read_with(cx, |e, _| {
        e.offset_for_column(0, 1).unwrap() - e.offset_for_column(0, 0).unwrap()
    });
    editor.read_with(cx, |e, _| {
        e.wrap_indicators_for_tests()
            .into_iter()
            .map(|(row, left)| (row, (left / em * 100.).round() / 100.))
            .collect()
    })
}

#[gpui::test]
fn word_wrap_indicator(cx: &mut TestAppContext) {
    // Monaco's `WRAPPED_TEXT` with `wordWrapColumn: 6`: view lines
    // 'aaaaa ', 'bbbbb ', 'ccccc' / 'short' / 'ddddd ', 'eeeee'.
    let text = "aaaaa bbbbb ccccc\nshort\nddddd eeeee";
    let (editor, cx) = setup(
        text,
        |o| {
            o.word_wrap = WordWrap::WordWrapColumn;
            o.word_wrap_column = 6;
        },
        cx,
    );
    // Off by default.
    assert!(indicators(&editor, cx).is_empty());
    set_options(&editor, |o| o.word_wrap_indicator = true, cx);
    // One per soft-wrapped view line, at the wrapping column.
    assert_eq!(indicators(&editor, cx), vec![(0, 6.), (0, 6.), (2, 6.)]);
    // A fixed wrapping column keeps its width.
    assert_eq!(segments(&editor, 0, cx), vec!["aaaaa ", "bbbbb ", "ccccc"]);
    // Words broken mid token get one too (Monaco's 'aaaaaaaaaa' at 5).
    let model = editor.read_with(cx, |e, _| e.model().clone());
    model.update(cx, |m, cx| m.set_value("aaaaaaaaaa", cx));
    set_options(&editor, |o| o.word_wrap_column = 5, cx);
    assert_eq!(indicators(&editor, cx), vec![(0, 5.)]);
    // Nothing without wrapping.
    set_options(&editor, |o| o.word_wrap = WordWrap::Off, cx);
    assert!(indicators(&editor, cx).is_empty());
}

#[gpui::test]
fn word_wrap_indicator_leaves_the_last_column_free(cx: &mut TestAppContext) {
    // Monaco `editorLayoutProvider.test.ts` "leaves the rightmost column for
    // the word wrap indicator when wrapping to the viewport".
    let (editor, cx) = setup("a", |o| o.word_wrap = WordWrap::On, cx);
    let wrapping = |editor: &Entity<CodeEditor>, cx: &mut VisualTestContext| {
        paint(cx);
        let info = editor.read_with(cx, |e, _| e.layout_info()).unwrap();
        (info.viewport_column, info.wrapping_column.unwrap())
    };
    let (viewport, on) = wrapping(&editor, cx);
    set_options(&editor, |o| o.word_wrap_indicator = true, cx);
    // The viewport keeps its columns; wrapping stops one earlier.
    assert_eq!(wrapping(&editor, cx), (viewport, on - 1));
    // `bounded` and `wordWrapColumn` already leave room.
    set_options(
        &editor,
        |o| {
            o.word_wrap = WordWrap::Bounded;
            o.word_wrap_column = 20;
        },
        cx,
    );
    assert_eq!(wrapping(&editor, cx).1, 20);
    set_options(&editor, |o| o.word_wrap = WordWrap::WordWrapColumn, cx);
    assert_eq!(wrapping(&editor, cx).1, 20);
}

#[gpui::test]
fn horizontal_scrollbar_in_content_height(cx: &mut TestAppContext) {
    let long = "x".repeat(2000);
    let (editor, cx) = setup(
        &format!("a\n{long}\nb"),
        |o| o.scroll_beyond_last_line = false,
        cx,
    );
    let (line_height, hbar) = editor.read_with(cx, |e, _| {
        (
            e.options().resolved_line_height(),
            e.options().horizontal_scrollbar_size,
        )
    });
    let height = |editor: &Entity<CodeEditor>, cx: &mut VisualTestContext| {
        paint(cx);
        editor.read_with(cx, |e, _| e.content_height()).unwrap()
    };
    // Monaco `_getContentHeight`: the shown horizontal scrollbar's height
    // keeps the last line above it.
    assert_eq!(height(&editor, cx), line_height * 3. + hbar);
    set_options(
        &editor,
        |o| o.scrollbar.ignore_horizontal_scrollbar_in_content_height = true,
        cx,
    );
    assert_eq!(height(&editor, cx), line_height * 3.);
    // Nothing to add without a horizontal scrollbar.
    set_options(
        &editor,
        |o| {
            o.scrollbar.ignore_horizontal_scrollbar_in_content_height = false;
            o.scrollbar.horizontal = gpui_monaco::options::ScrollbarVisibility::Hidden;
        },
        cx,
    );
    assert_eq!(height(&editor, cx), line_height * 3.);
}

#[gpui::test]
fn inertial_scroll(cx: &mut TestAppContext) {
    let (editor, cx) = setup(&lines(2000), |_| {}, cx);
    let origin = editor
        .read_with(cx, |e, _| e.scrolled_visible_position(p(0, 0)))
        .unwrap();
    let wheel = |cx: &mut VisualTestContext| {
        cx.simulate_event(ScrollWheelEvent {
            position: point(origin.x + px(10.), origin.y + px(5.)),
            delta: ScrollDelta::Pixels(point(px(0.), px(-25.))),
            ..Default::default()
        });
    };
    let top = |cx: &mut VisualTestContext| editor.read_with(cx, |e, _| e.scroll_top());
    // Off: the wheel's own delta.
    wheel(cx);
    cx.executor()
        .advance_clock(std::time::Duration::from_secs(2));
    paint(cx);
    assert_eq!(top(cx), px(25.));

    // On: Monaco's `_periodicSync` keeps going after the event, slowing
    // down (half a unit: 0.5^1.02 * 100px, then 90% each 1/60s).
    set_options(&editor, |o| o.inertial_scroll = true, cx);
    wheel(cx);
    let first = 0.5f32.powf(1.02) * 100.;
    assert!((f32::from(top(cx)) - (50. + first)).abs() < 0.01);
    cx.executor()
        .advance_clock(std::time::Duration::from_millis(17));
    paint(cx);
    assert!(top(cx) > px(50. + first));
    cx.executor()
        .advance_clock(std::time::Duration::from_secs(2));
    paint(cx);
    let resting = top(cx);
    // In all about ten times the first step.
    assert!(resting > px(50. + first * 9.) && resting < px(50. + first * 11.));
    cx.executor()
        .advance_clock(std::time::Duration::from_secs(2));
    paint(cx);
    assert_eq!(top(cx), resting);
}

#[gpui::test]
fn full_width_characters_in_two_cells(cx: &mut TestAppContext) {
    let (editor, cx) = setup("a中b", |_| {}, cx);
    let x = |editor: &Entity<CodeEditor>, column: u32, cx: &mut VisualTestContext| {
        paint(cx);
        editor.read_with(cx, |e, _| e.offset_for_column(0, column).unwrap())
    };
    let em = x(&editor, 1, cx) - x(&editor, 0, cx);
    // The test font draws `中` one em wide.
    assert_eq!(x(&editor, 2, cx) - x(&editor, 1, cx), em);
    // Monaco `fullwidthCharacterWidth: 'twoCells'`: two spaces wide, and
    // what follows moves along.
    set_options(
        &editor,
        |o| o.fullwidth_character_width = FullwidthCharacterWidth::TwoCells,
        cx,
    );
    assert_eq!(x(&editor, 1, cx), em);
    assert_eq!(x(&editor, 2, cx), em * 3.);
    assert_eq!(x(&editor, 3, cx), em * 4.);
    // Clicks land on the nearer edge of the cell.
    let origin = editor
        .read_with(cx, |e, _| e.scrolled_visible_position(p(0, 0)))
        .unwrap();
    let column_at = |x: Pixels, cx: &mut VisualTestContext| {
        editor.update_in(cx, |e, window, cx| {
            e.mouse_target(point(origin.x + x, origin.y + px(2.)), window, cx)
                .position
                .unwrap()
                .column
        })
    };
    assert_eq!(column_at(em * 1.4, cx), 1);
    assert_eq!(column_at(em * 2.6, cx), 2);
    // Measured wrapping counts the two cells too.
    set_options(
        &editor,
        |o| {
            o.word_wrap = WordWrap::WordWrapColumn;
            o.word_wrap_column = 2;
            o.wrapping_strategy = WrappingStrategy::Advanced;
        },
        cx,
    );
    assert_eq!(segments(&editor, 0, cx), vec!["a", "中", "b"]);
}

#[gpui::test]
fn accessibility_support(cx: &mut TestAppContext) {
    // Mostly one long line: Monaco `isDominatedByLongLines`.
    let long = "word ".repeat(2_500);
    let (editor, cx) = setup(&format!("{long}\nshort"), |_| {}, cx);
    // Auto without assistive technology (as in tests): not optimized.
    assert!(!editor.read_with(cx, |e, _| e.is_screen_reader_optimized()));
    assert_eq!(segments(&editor, 0, cx).len(), 1);
    // On: a model dominated by long lines wraps at the viewport.
    set_options(
        &editor,
        |o| o.accessibility_support = AccessibilitySupport::On,
        cx,
    );
    assert!(editor.read_with(cx, |e, _| e.is_screen_reader_optimized()));
    assert!(segments(&editor, 0, cx).len() > 1);
    let info = editor.read_with(cx, |e, _| e.layout_info()).unwrap();
    assert!(info.is_word_wrap_minified && info.is_viewport_wrapping);
    // Unless `wordWrapOverride1` says otherwise.
    set_options(
        &editor,
        |o| o.word_wrap_override1 = WordWrapOverride::Off,
        cx,
    );
    assert_eq!(segments(&editor, 0, cx).len(), 1);
}

#[gpui::test]
fn screen_readers_get_wrapped_lines_without_indent(cx: &mut TestAppContext) {
    // Monaco's computed `wrappingIndent` is `none` with a screen reader, so
    // the wrapping matches the screen reader's.
    let (editor, cx) = setup(
        "    aaaa bbbb cccc",
        |o| {
            o.word_wrap = WordWrap::WordWrapColumn;
            o.word_wrap_column = 10;
            o.accessibility_support = AccessibilitySupport::On;
        },
        cx,
    );
    let continuation_x = |editor: &Entity<CodeEditor>, cx: &mut VisualTestContext| {
        paint(cx);
        let column = segments(editor, 0, cx)[0].encode_utf16().count() as u32;
        editor.read_with(cx, |e, _| {
            e.scrolled_visible_position(p(0, column + 1)).unwrap().x
                - e.scrolled_visible_position(p(0, 0)).unwrap().x
        })
    };
    let optimized = continuation_x(&editor, cx);
    set_options(
        &editor,
        |o| o.accessibility_support = AccessibilitySupport::Off,
        cx,
    );
    // Not optimized, continuation lines keep the indentation.
    assert!(continuation_x(&editor, cx) > optimized);
    assert!(!editor.read_with(cx, |e, _| e.is_screen_reader_optimized()));
}
