//! Layout changes (Monaco `ViewModel._onConfigurationChanged` and
//! `setHiddenAreas` with `_captureStableViewport`): re-wrapping, a new
//! font, zoom or width and hidden areas keep the first visible line; zoomed
//! line heights stay fractional (`BareFontInfo._create`); the typical
//! half-width character is an `n` (`fontMeasurements.ts`).

use gpui::{Entity, Focusable, Pixels, TestAppContext, VisualTestContext, px, size};
use gpui_monaco::options::WordWrap;
use gpui_monaco::text::PointUtf16;
use gpui_monaco::{CodeEditor, EditorOptions, ScrollType};

/// 2,000 lines, each long enough to wrap several times.
fn long_lines() -> String {
    (1..=2000)
        .map(|n| format!("line {n} {}", "word ".repeat(60)))
        .collect::<Vec<_>>()
        .join("\n")
}

fn setup<'a>(
    text: &str,
    options: impl FnOnce(&mut EditorOptions),
    cx: &'a mut TestAppContext,
) -> (Entity<CodeEditor>, &'a mut VisualTestContext) {
    cx.update(gpui_monaco::init);
    let text = text.to_string();
    let (editor, cx) = cx.add_window_view(|window, cx| CodeEditor::with_text(&text, window, cx));
    cx.simulate_resize(size(px(800.), px(600.)));
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
        opts.smooth_scrolling = false;
        options(&mut opts);
        e.update_options(opts, cx);
    });
    paint(cx);
}

fn paint(cx: &mut VisualTestContext) {
    cx.update(|window, _| window.refresh());
    cx.run_until_parked();
}

fn line_height(editor: &Entity<CodeEditor>, cx: &mut VisualTestContext) -> Pixels {
    editor.read_with(cx, |e, _| e.font_info().unwrap().line_height)
}

/// Scrolls the top of view line `line` (0-based, no wrapping) plus
/// `delta` to the viewport's top.
fn scroll_to_line(
    editor: &Entity<CodeEditor>,
    line: u32,
    delta: Pixels,
    cx: &mut VisualTestContext,
) {
    let top = line_height(editor, cx) * line as f32 + delta;
    editor.update(cx, |e, cx| e.set_scroll_top(top, ScrollType::Immediate, cx));
    paint(cx);
}

/// The first visible position and how far below the viewport's top its
/// line starts (Monaco `saveViewState`'s `firstPosition` and
/// `firstPositionDeltaTop`).
fn first_visible(editor: &Entity<CodeEditor>, cx: &mut VisualTestContext) -> (PointUtf16, Pixels) {
    paint(cx);
    let state = editor.read_with(cx, |e, cx| e.save_view_state(cx));
    (
        state.first_position.unwrap(),
        state.first_position_delta_top,
    )
}

fn assert_close(a: Pixels, b: Pixels) {
    assert!((a - b).abs() < px(0.01), "{a:?} != {b:?}");
}

#[gpui::test]
fn turning_wrapping_on_keeps_the_first_visible_line(cx: &mut TestAppContext) {
    let (editor, cx) = setup(&long_lines(), |_| {}, cx);
    scroll_to_line(&editor, 999, px(5.), cx);
    let (position, delta) = first_visible(&editor, cx);
    assert_eq!(position, PointUtf16::new(999, 0));
    assert_close(delta, px(-5.));
    set_options(&editor, |o| o.word_wrap = WordWrap::On, cx);
    let (position, after) = first_visible(&editor, cx);
    assert_eq!(position, PointUtf16::new(999, 0));
    // Monaco `startLineDelta`: the part of the line above the viewport.
    assert_close(after, delta);
    // And off again.
    set_options(&editor, |o| o.word_wrap = WordWrap::Off, cx);
    assert_eq!(first_visible(&editor, cx).0, PointUtf16::new(999, 0));
}

#[gpui::test]
fn a_new_line_height_or_font_size_keeps_the_first_visible_line(cx: &mut TestAppContext) {
    let (editor, cx) = setup(&long_lines(), |_| {}, cx);
    scroll_to_line(&editor, 999, px(0.), cx);
    set_options(&editor, |o| o.line_height = Some(px(30.)), cx);
    assert_eq!(first_visible(&editor, cx).0, PointUtf16::new(999, 0));
    assert_eq!(line_height(&editor, cx), px(30.));
    set_options(&editor, |o| o.font_size = px(20.), cx);
    assert_eq!(first_visible(&editor, cx).0, PointUtf16::new(999, 0));
}

#[gpui::test]
fn zooming_keeps_the_first_visible_line(cx: &mut TestAppContext) {
    let (editor, cx) = setup(&long_lines(), |_| {}, cx);
    scroll_to_line(&editor, 999, px(0.), cx);
    cx.update(|_, cx| gpui_monaco::set_editor_zoom_level(1., cx));
    assert_eq!(first_visible(&editor, cx).0, PointUtf16::new(999, 0));
    cx.update(|_, cx| gpui_monaco::set_editor_zoom_level(-2., cx));
    assert_eq!(first_visible(&editor, cx).0, PointUtf16::new(999, 0));
    cx.update(|_, cx| gpui_monaco::set_editor_zoom_level(0., cx));
}

#[gpui::test]
fn resizing_a_wrapped_editor_keeps_the_first_visible_line(cx: &mut TestAppContext) {
    let (editor, cx) = setup(&long_lines(), |_| {}, cx);
    // Line 1000 at the top, wrapped.
    scroll_to_line(&editor, 999, px(0.), cx);
    set_options(&editor, |o| o.word_wrap = WordWrap::On, cx);
    assert_eq!(first_visible(&editor, cx).0, PointUtf16::new(999, 0));
    cx.simulate_resize(size(px(500.), px(600.)));
    assert_eq!(first_visible(&editor, cx).0, PointUtf16::new(999, 0));
    cx.simulate_resize(size(px(1100.), px(600.)));
    assert_eq!(first_visible(&editor, cx).0, PointUtf16::new(999, 0));
}

#[gpui::test]
fn the_top_of_the_file_stays_at_the_top(cx: &mut TestAppContext) {
    let (editor, cx) = setup(&long_lines(), |_| {}, cx);
    set_options(&editor, |o| o.word_wrap = WordWrap::On, cx);
    assert_eq!(editor.read_with(cx, |e, _| e.scroll_top()), px(0.));
    assert_eq!(first_visible(&editor, cx).0, PointUtf16::new(0, 0));
}

#[gpui::test]
fn hiding_areas_keeps_the_first_visible_line(cx: &mut TestAppContext) {
    let (editor, cx) = setup(&long_lines(), |_| {}, cx);
    scroll_to_line(&editor, 999, px(0.), cx);
    // Rows above the viewport.
    editor.update(cx, |e, cx| {
        e.set_hidden_areas(std::iter::once(10..20).collect(), cx)
    });
    assert_eq!(first_visible(&editor, cx).0, PointUtf16::new(999, 0));
    // Hiding the first visible line itself: Monaco doesn't recover the
    // viewport start, the scroll position stays.
    let scroll_top = editor.read_with(cx, |e, _| e.scroll_top());
    editor.update(cx, |e, cx| e.set_hidden_areas(vec![10..20, 995..1005], cx));
    paint(cx);
    assert_eq!(editor.read_with(cx, |e, _| e.scroll_top()), scroll_top);
}

/// Monaco `BareFontInfo._create`: the zoomed line height isn't rounded,
/// 18px is 19.8px at zoom level 1.
#[gpui::test]
fn zoomed_line_heights_are_fractional(cx: &mut TestAppContext) {
    let (editor, cx) = setup(
        "one\ntwo",
        |o| {
            o.font_size = px(12.);
            o.line_height = Some(px(18.));
        },
        cx,
    );
    cx.update(|_, cx| gpui_monaco::set_editor_zoom_level(1., cx));
    paint(cx);
    assert_close(line_height(&editor, cx), px(19.8));
    let computed = editor.read_with(cx, |e, _| e.computed_options());
    assert_close(computed.line_height.unwrap(), px(19.8));
    // The host's options keep their line height.
    assert_eq!(
        editor.read_with(cx, |e, _| e.options().line_height),
        Some(px(18.))
    );
    cx.update(|_, cx| gpui_monaco::set_editor_zoom_level(0., cx));
    paint(cx);
    assert_eq!(line_height(&editor, cx), px(18.));
}

/// Monaco `typicalHalfwidthCharacterWidth` is the width of `n`.
#[gpui::test]
fn the_typical_halfwidth_character_is_an_n(cx: &mut TestAppContext) {
    let (editor, cx) = setup("one", |_| {}, cx);
    let info = editor.read_with(cx, |e, _| e.font_info().unwrap());
    let n_width = cx.update(|window, _| {
        let run = gpui::TextRun {
            len: 1,
            font: gpui::Font {
                weight: info.font_weight,
                ..gpui::font(info.font_family.clone())
            },
            color: gpui::black(),
            background_color: None,
            underline: None,
            strikethrough: None,
        };
        window
            .text_system()
            .shape_line("n".into(), info.font_size, &[run], None)
            .width
    });
    assert_eq!(info.typical_halfwidth_character_width, n_width);
}
