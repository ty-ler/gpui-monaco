//! Editor and model options from Monaco's `editorOptions.ts` and
//! `ITextModelCreationOptions`: custom and interval line numbers, cursor
//! styles, selecting from line numbers, the mouse, minimap autohide, word
//! wrap overrides and break rules, selected text matching, large files and
//! the default line ending.

use std::cell::RefCell;
use std::rc::Rc;

use gpui::{
    AppContext as _, Entity, Focusable, Modifiers, MouseButton, MouseDownEvent, Pixels,
    ScrollDelta, ScrollWheelEvent, TestAppContext, VisualTestContext, point, px,
};
use gpui_monaco::cursor::Selection;
use gpui_monaco::options::{
    CursorStyle, LineNumbers, MinimapAutohide, MouseMiddleClickAction, MouseStyle,
    SelectedTextMatchMode, WordBreak, WordWrap, WordWrapOverride,
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
    // Selected text matches case-sensitively by default.
    assert_eq!(select_next(cx)[1].start(), p(0, 8));
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
