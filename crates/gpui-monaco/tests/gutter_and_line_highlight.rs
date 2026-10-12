//! The gutter and the current line highlight (#254): Monaco's
//! `CurrentLineHighlightOverlay` and `CurrentLineMarginHighlightOverlay`,
//! `LineNumbersOverlay`, the glyph margin's lanes (`GlyphMarginLanesModel`,
//! `GlyphMarginWidgets`) and the folding controls (`foldingDecorations.ts`).

use std::sync::Arc;

use gpui::{Entity, Focusable, Modifiers, TestAppContext, VisualTestContext, point, px};
use gpui_monaco::cursor::Selection;
use gpui_monaco::options::{
    EditorOptions, LineNumbers, RenderFinalNewline, RenderLineHighlight, ShowFoldingControls,
    WordWrap,
};
use gpui_monaco::text::PointUtf16;
use gpui_monaco::{CodeEditor, DecorationOptions, GlyphMarginLane, GlyphMarginOptions, Theme};

fn text(rows: usize) -> String {
    (0..rows)
        .map(|i| format!("line {i}"))
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
    editor.update(cx, |e, cx| {
        let mut opts = e.options().clone();
        opts.sticky_scroll.enabled = false;
        opts.minimap.enabled = false;
        options(&mut opts);
        e.update_options(opts, cx);
        // A theme with a line highlight fill, so it's drawn as a fill.
        let theme = Theme::from_vscode_json(
            r##"{"type": "dark", "colors": {
                "editor.lineHighlightBackground": "#ff000040",
                "editor.inactiveLineHighlightBackground": "#00ff0040"
            }}"##,
        )
        .unwrap();
        e.set_theme(Arc::new(theme), cx);
    });
    focus(&editor, cx);
    (editor, cx)
}

fn focus(editor: &Entity<CodeEditor>, cx: &mut VisualTestContext) {
    cx.update(|window, cx| window.focus(&editor.focus_handle(cx), cx));
    paint(cx);
}

fn paint(cx: &mut VisualTestContext) {
    cx.update(|window, _| window.refresh());
    cx.run_until_parked();
}

fn select(editor: &Entity<CodeEditor>, selections: &[Selection], cx: &mut VisualTestContext) {
    editor.update(cx, |e, cx| e.set_selections(selections, cx));
    paint(cx);
}

fn cursor(row: u32, column: u32) -> Selection {
    Selection::collapsed(PointUtf16::new(row, column))
}

/// (view line, gutter box) of the highlight's boxes.
fn highlights(editor: &Entity<CodeEditor>, cx: &mut VisualTestContext) -> Vec<(u32, bool)> {
    paint(cx);
    editor.read_with(cx, |e, _| {
        e.line_highlights_for_tests()
            .into_iter()
            .map(|(line, margin, _, _, _)| (line, margin))
            .collect()
    })
}

/// Monaco `renderLineHighlight: 'gutter'`: only the margin box, as wide as
/// the gutter, and kept while there's a selection.
#[gpui::test]
fn gutter_highlight_paints_only_the_gutter_and_stays_with_a_selection(cx: &mut TestAppContext) {
    let (editor, cx) = setup(
        &text(10),
        |o| o.render_line_highlight = RenderLineHighlight::Gutter,
        cx,
    );
    select(&editor, &[cursor(2, 1)], cx);
    assert_eq!(highlights(&editor, cx), vec![(2, true)]);
    let (bounds, content_left) = editor.read_with(cx, |e, _| {
        (
            e.line_highlights_for_tests()[0].2,
            e.layout_info().unwrap().content_left,
        )
    });
    assert_eq!(bounds.right(), content_left);
    select(
        &editor,
        &[Selection {
            anchor: PointUtf16::new(2, 0),
            head: PointUtf16::new(2, 3),
        }],
        cx,
    );
    assert_eq!(highlights(&editor, cx), vec![(2, true)]);
}

/// Monaco `'line'` draws only the text area's box, and only while every
/// selection is empty; `'all'` draws both.
#[gpui::test]
fn line_highlight_needs_empty_selections(cx: &mut TestAppContext) {
    let (editor, cx) = setup(
        &text(10),
        |o| o.render_line_highlight = RenderLineHighlight::Line,
        cx,
    );
    select(&editor, &[cursor(1, 0)], cx);
    assert_eq!(highlights(&editor, cx), vec![(1, false)]);
    select(
        &editor,
        &[
            cursor(1, 0),
            Selection {
                anchor: PointUtf16::new(4, 0),
                head: PointUtf16::new(4, 2),
            },
        ],
        cx,
    );
    assert_eq!(highlights(&editor, cx), Vec::new());
    editor.update(cx, |e, cx| {
        let mut options = e.options().clone();
        options.render_line_highlight = RenderLineHighlight::All;
        e.update_options(options, cx);
    });
    select(&editor, &[cursor(1, 0)], cx);
    assert_eq!(highlights(&editor, cx), vec![(1, true), (1, false)]);
}

/// Monaco highlights every cursor's line, and with viewport wrapping every
/// view line of it, the border only on the cursor's own view line.
#[gpui::test]
fn every_cursor_line_and_its_wrapped_lines_are_highlighted(cx: &mut TestAppContext) {
    let long = "word ".repeat(60);
    let source = format!("short\n{long}\nshort\n{long}");
    let (editor, cx) = setup(
        &source,
        |o| {
            o.render_line_highlight = RenderLineHighlight::Line;
            o.word_wrap = WordWrap::On;
        },
        cx,
    );
    cx.simulate_resize(gpui::size(px(400.), px(600.)));
    select(&editor, &[cursor(0, 1), cursor(2, 1)], cx);
    let segments = editor.read_with(cx, |e, _| e.view_line_segments_for_tests());
    let view_line_of = |row: u32| segments.iter().position(|(r, _)| *r == row).unwrap() as u32;
    assert_eq!(
        highlights(&editor, cx),
        vec![(view_line_of(0), false), (view_line_of(2), false)]
    );
    let row_1_lines = segments.iter().filter(|(row, _)| *row == 1).count();
    assert!(row_1_lines > 2, "row 1 wraps: {segments:?}");
    // A cursor at the start of the wrapped row 1: all its view lines.
    select(&editor, &[cursor(1, 0)], cx);
    let boxes = editor.read_with(cx, |e, _| e.line_highlights_for_tests());
    assert_eq!(boxes.len(), row_1_lines);
    let bordered: Vec<u32> = boxes
        .iter()
        .filter(|(_, _, _, _, border)| *border)
        .map(|(line, ..)| *line)
        .collect();
    // vs-dark-like themes without a border set get none.
    assert!(bordered.len() <= 1);
}

/// Monaco's unfocused editor uses `editor.inactiveLineHighlightBackground`.
#[gpui::test]
fn unfocused_editors_use_the_inactive_color(cx: &mut TestAppContext) {
    let (editor, cx) = setup(&text(5), |_| {}, cx);
    select(&editor, &[cursor(1, 0)], cx);
    let background = |editor: &Entity<CodeEditor>, cx: &mut VisualTestContext| {
        paint(cx);
        editor.read_with(cx, |e, _| e.line_highlights_for_tests()[0].3)
    };
    let focused = background(&editor, cx);
    assert_eq!(
        focused,
        editor.read_with(cx, |e, _| e.theme().colors.line_highlight_background)
    );
    cx.update(|window, cx| window.blur(cx));
    let unfocused = background(&editor, cx);
    assert_eq!(
        unfocused,
        editor.read_with(cx, |e, _| e
            .theme()
            .colors
            .inactive_line_highlight_background)
    );
    assert_ne!(focused, unfocused);
}

fn decorate(
    editor: &Entity<CodeEditor>,
    decorations: Vec<(std::ops::Range<PointUtf16>, DecorationOptions)>,
    cx: &mut VisualTestContext,
) {
    editor.update(cx, |e, cx| {
        let model = e.model().clone();
        model.update(cx, |model, cx| {
            let rope = model.buffer().rope().clone();
            let decorations = decorations
                .into_iter()
                .map(|(range, options)| {
                    (
                        rope.point_utf16_to_offset(range.start)
                            ..rope.point_utf16_to_offset(range.end),
                        options,
                    )
                })
                .collect();
            model.delta_decorations(None, &[], decorations, cx);
        });
    });
    paint(cx);
}

fn glyph(lane: Option<GlyphMarginLane>, z_index: i32) -> DecorationOptions {
    DecorationOptions {
        glyph: Some(("●".into(), gpui::red())),
        glyph_margin: lane.map(|position| GlyphMarginOptions {
            position,
            persist_lane: false,
        }),
        z_index,
        ..Default::default()
    }
}

fn p(row: u32, column: u32) -> PointUtf16 {
    PointUtf16::new(row, column)
}

/// Monaco draws a glyph decoration on every line of its range.
#[gpui::test]
fn glyphs_show_on_every_line_of_a_multi_line_decoration(cx: &mut TestAppContext) {
    let (editor, cx) = setup(&text(10), |o| o.glyph_margin = true, cx);
    decorate(&editor, vec![(p(2, 0)..p(4, 3), glyph(None, 0))], cx);
    let drawn = editor.read_with(cx, |e, _| e.glyphs_for_tests());
    assert_eq!(drawn, vec![(2, 0), (3, 0), (4, 0)]);
}

/// Monaco `GlyphMarginLanesModel`: the margin is a line height per lane in
/// use, glyphs go in their lane, and the highest z-index wins a lane.
#[gpui::test]
fn glyph_margin_lanes(cx: &mut TestAppContext) {
    let (editor, cx) = setup(&text(10), |o| o.glyph_margin = true, cx);
    let line_height = editor.read_with(cx, |e, _| e.view_metrics_for_tests().unwrap().3);
    let width = |editor: &Entity<CodeEditor>, cx: &mut VisualTestContext| {
        editor.read_with(cx, |e, _| {
            let info = e.layout_info().unwrap();
            (
                info.glyph_margin_width,
                info.glyph_margin_decoration_lane_count,
            )
        })
    };
    assert_eq!(width(&editor, cx), (line_height, 1));
    decorate(
        &editor,
        vec![
            (p(1, 0)..p(3, 0), glyph(Some(GlyphMarginLane::Left), 0)),
            (p(2, 0)..p(2, 0), glyph(Some(GlyphMarginLane::Right), 0)),
            // Two in the center lane on row 5: one shows.
            (p(5, 0)..p(5, 0), glyph(None, 0)),
            (p(5, 0)..p(5, 0), glyph(None, 3)),
        ],
        cx,
    );
    assert_eq!(width(&editor, cx), (line_height * 2., 2));
    let drawn = editor.read_with(cx, |e, _| e.glyphs_for_tests());
    assert_eq!(drawn, vec![(1, 0), (2, 0), (2, 1), (3, 0), (5, 0)]);
}

/// Monaco: only the primary cursor's line number is active.
#[gpui::test]
fn only_the_primary_cursor_line_number_is_active(cx: &mut TestAppContext) {
    let (editor, cx) = setup(&text(10), |_| {}, cx);
    select(&editor, &[cursor(3, 0), cursor(6, 0)], cx);
    let (active, normal) = editor.read_with(cx, |e, _| {
        (
            e.theme().colors.active_line_number,
            e.theme().colors.line_number,
        )
    });
    let styles = editor.read_with(cx, |e, _| e.line_number_styles_for_tests());
    let color = |row: u32| styles.iter().find(|(r, _, _)| *r == row).unwrap().2;
    assert_eq!(color(3), active);
    assert_eq!(color(6), normal);
}

/// Monaco's relative line numbers show the current line's number left
/// aligned (`.relative-current-line-number`).
#[gpui::test]
fn relative_current_line_number_is_left_aligned(cx: &mut TestAppContext) {
    let (editor, cx) = setup(&text(30), |o| o.line_numbers = LineNumbers::Relative, cx);
    select(&editor, &[cursor(12, 0)], cx);
    let (styles, labels, numbers_left) = editor.read_with(cx, |e, _| {
        let info = e.layout_info().unwrap();
        (
            e.line_number_styles_for_tests(),
            e.line_number_labels_for_tests(),
            info.line_numbers_left,
        )
    });
    let x = |row: u32| styles.iter().find(|(r, _, _)| *r == row).unwrap().1;
    let label = |row: u32| labels.iter().find(|(r, _)| *r == row).unwrap().1.clone();
    assert_eq!(label(12), "13");
    assert_eq!(label(10), "2");
    assert_eq!(x(12), numbers_left);
    assert!(x(10) > x(12), "the others are right aligned");
}

/// Monaco `renderFinalNewline: 'dimmed'`: the empty last line's number in
/// the line number color at 40% (the first line too, when it's the only
/// one), even when it's the active line.
#[gpui::test]
fn the_final_newline_number_is_dimmed(cx: &mut TestAppContext) {
    let (editor, cx) = setup(
        "",
        |o| o.render_final_newline = RenderFinalNewline::Dimmed,
        cx,
    );
    let normal = editor.read_with(cx, |e, _| e.theme().colors.line_number);
    let styles = editor.read_with(cx, |e, _| e.line_number_styles_for_tests());
    assert_eq!(styles.len(), 1);
    assert_eq!(styles[0].2, normal.opacity(0.4));
}

/// Monaco: no folding column with `showFoldingControls: never`, else the
/// controls fill the line decorations column (with a 2px margin).
#[gpui::test]
fn folding_controls_column(cx: &mut TestAppContext) {
    let source = "fn a() {\n    one();\n    two();\n}\n";
    let (editor, cx) = setup(
        source,
        |o| o.show_folding_controls = ShowFoldingControls::Always,
        cx,
    );
    cx.executor()
        .advance_clock(std::time::Duration::from_millis(500));
    paint(cx);
    let (info, controls) = editor.read_with(cx, |e, _| {
        (e.layout_info().unwrap(), e.fold_controls_for_tests())
    });
    let (row, bounds, collapsed) = controls[0];
    assert_eq!((row, collapsed), (0, false));
    assert_eq!(bounds.left(), info.decorations_left + px(2.));
    assert_eq!(bounds.size.width, info.decorations_width);
    let with_controls = info.decorations_width;

    editor.update(cx, |e, cx| {
        let mut options = e.options().clone();
        options.show_folding_controls = ShowFoldingControls::Never;
        e.update_options(options, cx);
    });
    paint(cx);
    let (info, controls) = editor.read_with(cx, |e, _| {
        (e.layout_info().unwrap(), e.fold_controls_for_tests())
    });
    assert!(controls.is_empty());
    assert_eq!(info.decorations_width, with_controls - px(16.));
}

/// Monaco `_onMouseDown`: clicks in the glyph margin and line decorations
/// don't take focus; text and (selecting) line numbers do.
#[gpui::test]
fn gutter_clicks_dont_steal_focus(cx: &mut TestAppContext) {
    let (editor, cx) = setup(&text(10), |o| o.glyph_margin = true, cx);
    let info = editor.read_with(cx, |e, _| e.layout_info().unwrap());
    let line_height = editor.read_with(cx, |e, _| e.view_metrics_for_tests().unwrap().3);
    let y = line_height * 2.5;
    let is_focused = |cx: &mut VisualTestContext| {
        cx.update(|window, cx| editor.focus_handle(cx).is_focused(window))
    };
    cx.update(|window, cx| window.blur(cx));
    paint(cx);
    assert!(!is_focused(cx));
    cx.simulate_click(
        point(info.glyph_margin_left + px(2.), y),
        Modifiers::default(),
    );
    assert!(!is_focused(cx), "the glyph margin");
    cx.simulate_click(
        point(info.decorations_left + px(2.), y),
        Modifiers::default(),
    );
    assert!(!is_focused(cx), "the line decorations");
    cx.simulate_click(
        point(info.line_numbers_left + px(2.), y),
        Modifiers::default(),
    );
    assert!(is_focused(cx), "the line numbers select");
    cx.update(|window, cx| window.blur(cx));
    paint(cx);
    cx.simulate_click(point(info.content_left + px(20.), y), Modifiers::default());
    assert!(is_focused(cx), "the text");
}

// ----- #295: manual folding icons, glyph margin widget ranges -----

/// Monaco `getDecorationOption`: manual ranges (Create Folding Range from
/// Selection) get `foldingManualCollapsedIcon` and
/// `foldingManualExpandedIcon`, which default to the other icons'
/// codicons and can be changed on their own.
#[gpui::test]
fn manual_folding_ranges_have_their_own_icons(cx: &mut TestAppContext) {
    use gpui_monaco::codicons::Codicon;
    use gpui_monaco::view::folding::{FoldingIcon, set_folding_icon};
    let source = "fn a() {\n    one();\n    two();\n}\nlet x = 1;\nlet y = 2;\nlet z = 3;\n";
    let (editor, cx) = setup(
        source,
        |o| o.show_folding_controls = ShowFoldingControls::Always,
        cx,
    );
    cx.executor()
        .advance_clock(std::time::Duration::from_millis(500));
    paint(cx);
    select(
        &editor,
        &[Selection::new(PointUtf16::new(4, 0), PointUtf16::new(6, 3))],
        cx,
    );
    editor.update(cx, |e, cx| e.create_fold_from_selection(cx));
    paint(cx);
    let icons = |cx: &mut VisualTestContext| {
        paint(cx);
        editor.read_with(cx, |e, cx| e.fold_control_icons_for_tests(cx))
    };
    assert_eq!(
        icons(cx),
        vec![
            (0, FoldingIcon::Expanded, Codicon::ChevronDown),
            (4, FoldingIcon::ManualCollapsed, Codicon::ChevronRight),
        ]
    );
    editor.update(cx, |e, cx| e.unfold(cx));
    assert_eq!(
        icons(cx)[1],
        (4, FoldingIcon::ManualExpanded, Codicon::ChevronDown)
    );
    // A product icon theme's `folding-manual-expanded`.
    cx.update(|_, cx| set_folding_icon(FoldingIcon::ManualExpanded, Some(Codicon::Ellipsis), cx));
    assert_eq!(
        icons(cx),
        vec![
            (0, FoldingIcon::Expanded, Codicon::ChevronDown),
            (4, FoldingIcon::ManualExpanded, Codicon::Ellipsis),
        ]
    );
    cx.update(|_, cx| set_folding_icon(FoldingIcon::ManualExpanded, None, cx));
    assert_eq!(icons(cx)[1].2, Codicon::ChevronDown);
}

struct Glyph;

impl gpui::Render for Glyph {
    fn render(
        &mut self,
        _: &mut gpui::Window,
        _: &mut gpui::Context<Self>,
    ) -> impl gpui::IntoElement {
        gpui::div()
    }
}

/// Monaco `IGlyphMarginWidget.getPosition()`: a range, a lane and a
/// z-index. The widget shows once, on the range's first line or the first
/// one in view (`_collectWidgetBasedGlyphRenderRequest`), and its lane is
/// taken on every line of the range (`_computeGlyphMarginLanes`).
#[gpui::test]
fn glyph_margin_widgets_take_a_range(cx: &mut TestAppContext) {
    use gpui::AppContext as _;
    use gpui_monaco::{GlyphMarginWidget, GlyphMarginWidgetPosition};
    let (editor, cx) = setup(&text(40), |o| o.glyph_margin = true, cx);
    let line_height = editor.read_with(cx, |e, _| e.view_metrics_for_tests().unwrap().3);
    let id = editor.update(cx, |e, cx| {
        let view = cx.new(|_| Glyph);
        e.add_glyph_margin_widget(
            GlyphMarginWidget {
                lane: GlyphMarginLane::Left,
                ..GlyphMarginWidget::with_range(p(3, 2)..p(6, 1), view)
            },
            cx,
        )
    });
    // A center glyph inside the range takes the second lane.
    decorate(&editor, vec![(p(5, 0)..p(5, 0), glyph(None, 0))], cx);
    let drawn = |cx: &mut VisualTestContext| {
        paint(cx);
        editor.read_with(cx, |e, _| e.glyphs_for_tests())
    };
    assert_eq!(drawn(cx), vec![(3, 0), (5, 1)]);
    // Scrolled into the range: on the first visible line.
    let scroll = |y: f32, cx: &mut VisualTestContext| {
        editor.update(cx, |e, cx| {
            e.set_scroll_position(point(px(0.), line_height * y), cx)
        });
    };
    scroll(4., cx);
    assert_eq!(drawn(cx)[..2], [(4, 0), (5, 1)]);
    // Past it: not shown.
    scroll(7., cx);
    assert!(drawn(cx).is_empty());
    // Monaco `layoutGlyphMarginWidget`.
    editor.update(cx, |e, cx| {
        e.layout_glyph_margin_widget(
            id,
            GlyphMarginWidgetPosition {
                range: p(10, 0)..p(10, 0),
                lane: GlyphMarginLane::Center,
                z_index: 0,
            },
            cx,
        )
    });
    assert_eq!(drawn(cx), vec![(10, 0)]);
    // The row constructor still shows beside its row.
    scroll(0., cx);
    editor.update(cx, |e, cx| {
        let view = cx.new(|_| Glyph);
        e.add_glyph_margin_widget(GlyphMarginWidget::new(2, view), cx)
    });
    assert_eq!(drawn(cx)[0], (2, 0));
}
