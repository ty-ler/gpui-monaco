//! Decoration and gutter hovers (Monaco `ContentHoverComputer.
//! _getLineDecorations`, `MarkdownHoverParticipant.computeSync`,
//! `GlyphHoverComputer` and `GlyphHoverWidget._showAt`): whole-line and
//! `showIfCollapsed` decorations, glyph margin lanes, line number hovers,
//! the gutter hover's placement and markdown hover messages.

use std::time::Duration;

use gpui::{
    AppContext as _, Entity, Focusable as _, Modifiers, Pixels, TestAppContext, VisualTestContext,
    px,
};
use gpui_monaco::providers::MarkdownString;
use gpui_monaco::text::PointUtf16;
use gpui_monaco::{CodeEditor, DecorationOptions, GlyphMarginLane, GlyphMarginOptions, TextModel};

const TEXT: &str = "alpha beta gamma\ndelta epsilon\nzeta eta theta\niota\n";

fn setup(cx: &mut TestAppContext) -> (Entity<CodeEditor>, &mut VisualTestContext) {
    cx.update(gpui_monaco::init);
    let (editor, cx) = cx.add_window_view(|window, cx| {
        let model = cx.new(|_| TextModel::new(TEXT));
        CodeEditor::new(model, window, cx)
    });
    cx.update(|window, cx| window.focus(&editor.read(cx).focus_handle(cx), cx));
    paint(cx);
    (editor, cx)
}

fn paint(cx: &mut VisualTestContext) {
    cx.update(|window, _| window.refresh());
    cx.run_until_parked();
}

fn rest(cx: &mut VisualTestContext, ms: u64) {
    cx.executor().advance_clock(Duration::from_millis(ms));
    cx.run_until_parked();
    paint(cx);
}

fn decorate(
    editor: &Entity<CodeEditor>,
    cx: &mut VisualTestContext,
    decorations: Vec<(std::ops::Range<usize>, DecorationOptions)>,
) {
    let model = editor.read_with(cx, |e, _| e.model().clone());
    let id = editor.entity_id();
    model.update(cx, |m, cx| {
        m.delta_decorations(Some(id), &[], decorations, cx);
    });
    paint(cx);
}

fn offset(row: u32, column: usize) -> usize {
    TEXT.split('\n')
        .take(row as usize)
        .map(|line| line.len() + 1)
        .sum::<usize>()
        + column
}

fn text_point(
    editor: &Entity<CodeEditor>,
    cx: &mut VisualTestContext,
    row: u32,
    column: u32,
) -> gpui::Point<Pixels> {
    let origin = editor
        .read_with(cx, |e, _| {
            e.scrolled_visible_position(PointUtf16::new(row, column))
        })
        .expect("laid out");
    gpui::point(origin.x + px(1.), origin.y + px(2.))
}

/// Rests the mouse at `point` past the hover delay; the hover's contents.
fn hover_at(
    editor: &Entity<CodeEditor>,
    cx: &mut VisualTestContext,
    point: gpui::Point<Pixels>,
) -> Vec<String> {
    // Off the editor first, so each call asks again.
    cx.simulate_mouse_move(gpui::point(px(-10.), px(-10.)), None, Modifiers::default());
    rest(cx, 0);
    editor.update(cx, |e, cx| e.hide_hover(cx));
    cx.simulate_mouse_move(point, None, Modifiers::default());
    rest(cx, 400);
    editor.read_with(cx, |e, cx| e.hover_contents_for_tests(cx))
}

fn hover_text(
    editor: &Entity<CodeEditor>,
    cx: &mut VisualTestContext,
    row: u32,
    column: u32,
) -> Vec<String> {
    let point = text_point(editor, cx, row, column);
    hover_at(editor, cx, point)
}

fn message(text: &str) -> Vec<MarkdownString> {
    vec![MarkdownString::new(text)]
}

/// The gutter's glyph margin and line numbers, from the editor's left.
fn gutter(editor: &Entity<CodeEditor>, cx: &mut VisualTestContext) -> (Pixels, Pixels, Pixels) {
    editor.read_with(cx, |e, _| {
        let info = e.layout_info().expect("laid out");
        let left = e.widget_bounds_for_tests().0.left();
        (
            left + info.glyph_margin_left,
            info.glyph_margin_width,
            info.line_numbers_width,
        )
    })
}

/// Monaco `_getLineDecorations`: a whole-line decoration's message shows
/// anywhere on its line, as the common blame pattern
/// `{ range: (2,1,2,1), options: { isWholeLine: true, hoverMessage } }`
/// relies on; other decorations only inside their range.
#[gpui::test]
fn whole_line_decorations_hover_anywhere_on_the_line(cx: &mut TestAppContext) {
    let (editor, cx) = setup(cx);
    decorate(
        &editor,
        cx,
        vec![
            (
                offset(1, 0)..offset(1, 0),
                DecorationOptions {
                    is_whole_line: true,
                    hover_message: message("blame"),
                    ..Default::default()
                },
            ),
            (
                offset(2, 0)..offset(2, 0),
                DecorationOptions {
                    hover_message: message("collapsed"),
                    ..Default::default()
                },
            ),
        ],
    );
    assert_eq!(hover_text(&editor, cx, 1, 8), ["blame"]);
    assert_eq!(hover_text(&editor, cx, 1, 0), ["blame"]);
    // Its part is about its own (empty) range, which isn't highlighted.
    let highlight = editor.read_with(cx, |e, cx| e.hover_highlight_for_tests(cx));
    assert_eq!(highlight, None);
    // Not on other lines, and a plain empty decoration only at its column.
    assert!(hover_text(&editor, cx, 0, 8).is_empty());
    assert!(hover_text(&editor, cx, 2, 8).is_empty());
}

/// Monaco: a `showIfCollapsed` decoration takes a character more each side.
#[gpui::test]
fn show_if_collapsed_decorations_take_a_character_more(cx: &mut TestAppContext) {
    let (editor, cx) = setup(cx);
    decorate(
        &editor,
        cx,
        vec![
            (
                offset(0, 6)..offset(0, 6),
                DecorationOptions {
                    show_if_collapsed: true,
                    hover_message: message("collapsed"),
                    ..Default::default()
                },
            ),
            (
                offset(2, 5)..offset(2, 5),
                DecorationOptions {
                    hover_message: message("plain"),
                    ..Default::default()
                },
            ),
        ],
    );
    assert_eq!(hover_text(&editor, cx, 0, 7), ["collapsed"]);
    assert!(hover_text(&editor, cx, 0, 9).is_empty());
    assert!(hover_text(&editor, cx, 2, 6).is_empty());
}

/// Monaco `MarkdownHoverParticipant.computeSync`: a multi-line decoration's
/// part is about its range on the hovered line.
#[gpui::test]
fn multi_line_decorations_are_clipped_to_the_line(cx: &mut TestAppContext) {
    let (editor, cx) = setup(cx);
    decorate(
        &editor,
        cx,
        vec![(
            offset(0, 6)..offset(2, 4),
            DecorationOptions {
                hover_message: message("span"),
                ..Default::default()
            },
        )],
    );
    assert_eq!(hover_text(&editor, cx, 1, 3), ["span"]);
    let highlight = editor.read_with(cx, |e, cx| e.hover_highlight_for_tests(cx));
    assert_eq!(
        highlight,
        Some(PointUtf16::new(1, 0)..PointUtf16::new(1, 13))
    );
    // Past its end on the last line: nothing.
    assert!(hover_text(&editor, cx, 2, 9).is_empty());
}

/// Monaco `hoverMessage: IMarkdownString | IMarkdownString[]`: the strings
/// are one part's contents, and empty ones don't hover.
#[gpui::test]
fn decoration_hover_messages_are_markdown(cx: &mut TestAppContext) {
    let (editor, cx) = setup(cx);
    decorate(
        &editor,
        cx,
        vec![
            (
                offset(0, 0)..offset(0, 5),
                DecorationOptions {
                    hover_message: vec![
                        MarkdownString::new("[run](command:noop)").trusted(true),
                        MarkdownString::new("$(error) second").with_theme_icons(true),
                    ],
                    ..Default::default()
                },
            ),
            (
                offset(0, 0)..offset(0, 5),
                DecorationOptions {
                    hover_message: vec![MarkdownString::new("")],
                    ..Default::default()
                },
            ),
        ],
    );
    assert_eq!(
        hover_text(&editor, cx, 0, 2),
        ["[run](command:noop)", "$(error) second"]
    );
    assert_eq!(editor.read_with(cx, |e, cx| e.hover_part_count(cx)), 1);
}

/// Monaco `GlyphHoverComputer`: every decoration covering the row, not just
/// those starting on it.
#[gpui::test]
fn glyph_hovers_cover_the_decorations_rows(cx: &mut TestAppContext) {
    let (editor, cx) = setup(cx);
    decorate(
        &editor,
        cx,
        vec![(
            offset(0, 0)..offset(2, 3),
            DecorationOptions {
                glyph: Some(("●".into(), gpui::red())),
                glyph_margin_hover_message: vec![
                    MarkdownString::new("Breakpoint"),
                    MarkdownString::new("Condition"),
                ],
                ..Default::default()
            },
        )],
    );
    let (glyph_left, _, _) = gutter(&editor, cx);
    let y = text_point(&editor, cx, 1, 0).y;
    let contents = hover_at(&editor, cx, gpui::point(glyph_left + px(4.), y));
    assert_eq!(contents, ["Breakpoint", "Condition"]);
    // A hover row per message.
    assert_eq!(editor.read_with(cx, |e, cx| e.hover_part_count(cx)), 2);
    let y = text_point(&editor, cx, 3, 0).y;
    assert!(hover_at(&editor, cx, gpui::point(glyph_left + px(4.), y)).is_empty());
}

/// Monaco `GlyphHoverComputer`: in the glyph margin, only the hovered
/// lane's decorations; over the line numbers, `lineNumberHoverMessage`.
#[gpui::test]
fn glyph_hovers_by_lane_and_line_number(cx: &mut TestAppContext) {
    let (editor, cx) = setup(cx);
    let glyph = |lane, text: &str| DecorationOptions {
        glyph: Some(("●".into(), gpui::red())),
        glyph_margin: Some(GlyphMarginOptions {
            position: lane,
            persist_lane: false,
        }),
        glyph_margin_hover_message: message(text),
        ..Default::default()
    };
    decorate(
        &editor,
        cx,
        vec![
            (
                offset(1, 0)..offset(1, 0),
                glyph(GlyphMarginLane::Left, "left"),
            ),
            (
                offset(1, 0)..offset(1, 0),
                glyph(GlyphMarginLane::Right, "right"),
            ),
            (
                offset(1, 0)..offset(1, 0),
                DecorationOptions {
                    line_number_hover_message: message("line"),
                    ..Default::default()
                },
            ),
        ],
    );
    let (glyph_left, glyph_width, numbers_width) = gutter(&editor, cx);
    let lane = glyph_width / 2.;
    let y = text_point(&editor, cx, 1, 0).y;
    assert_eq!(
        hover_at(&editor, cx, gpui::point(glyph_left + lane / 2., y)),
        ["left"]
    );
    assert_eq!(
        hover_at(&editor, cx, gpui::point(glyph_left + lane * 1.5, y)),
        ["right"]
    );
    let numbers = glyph_left + glyph_width + numbers_width / 2.;
    assert_eq!(hover_at(&editor, cx, gpui::point(numbers, y)), ["line"]);
    // A row with lanes of its own but no line number message.
    let y = text_point(&editor, cx, 0, 0).y;
    assert!(hover_at(&editor, cx, gpui::point(numbers, y)).is_empty());
}

/// Monaco `GlyphHoverWidget._showAt`: right of the glyph margin (or of the
/// line numbers), centered on the line, inside the editor.
#[gpui::test]
fn gutter_hovers_show_beside_the_gutter(cx: &mut TestAppContext) {
    let (editor, cx) = setup(cx);
    decorate(
        &editor,
        cx,
        vec![(
            offset(2, 0)..offset(2, 0),
            DecorationOptions {
                glyph: Some(("●".into(), gpui::red())),
                glyph_margin_hover_message: message("glyph"),
                line_number_hover_message: message("line"),
                ..Default::default()
            },
        )],
    );
    let (glyph_left, glyph_width, numbers_width) = gutter(&editor, cx);
    let line = text_point(&editor, cx, 2, 0).y - px(2.);
    let line_height = editor.read_with(cx, |e, _| e.options().resolved_line_height());
    let center = line + line_height / 2.;

    hover_at(&editor, cx, gpui::point(glyph_left + px(4.), line + px(2.)));
    let card = editor
        .read_with(cx, |e, cx| e.hover_card_bounds_for_tests(cx))
        .expect("painted");
    assert_eq!(card.left(), glyph_left + glyph_width);
    assert!((card.center().y - center).abs() <= px(1.), "{card:?}");

    hover_at(
        &editor,
        cx,
        gpui::point(glyph_left + glyph_width + numbers_width / 2., line + px(2.)),
    );
    let card = editor
        .read_with(cx, |e, cx| e.hover_card_bounds_for_tests(cx))
        .expect("painted");
    assert_eq!(card.left(), glyph_left + glyph_width + numbers_width);
    assert!((card.center().y - center).abs() <= px(1.), "{card:?}");

    // On the first line, it's kept inside the editor.
    decorate(
        &editor,
        cx,
        vec![(
            offset(0, 0)..offset(0, 0),
            DecorationOptions {
                line_number_hover_message: vec![
                    MarkdownString::new("one"),
                    MarkdownString::new("two"),
                    MarkdownString::new("three"),
                ],
                ..Default::default()
            },
        )],
    );
    let top = editor.read_with(cx, |e, _| e.widget_bounds_for_tests().0.top());
    let y = text_point(&editor, cx, 0, 0).y;
    hover_at(
        &editor,
        cx,
        gpui::point(glyph_left + glyph_width + numbers_width / 2., y),
    );
    let card = editor
        .read_with(cx, |e, cx| e.hover_card_bounds_for_tests(cx))
        .expect("painted");
    assert_eq!(card.top(), top);
}
