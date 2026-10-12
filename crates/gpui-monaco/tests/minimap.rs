//! The minimap (Monaco `browser/viewParts/minimap/minimap.ts`): its scroll
//! and slider geometry, slider drags, clicks, selection and decoration
//! highlights, section headers, slider colors and the shadow.

use gpui::{
    Entity, Focusable, Hsla, Modifiers, MouseButton, Pixels, TestAppContext, VisualTestContext,
    point, px,
};
use gpui_monaco::cursor::Selection;
use gpui_monaco::options::{EditorOptions, MinimapSize};
use gpui_monaco::text::{Point, PointUtf16};
use gpui_monaco::{
    CodeEditor, DecorationOptions, Marker, MarkerSeverity, MinimapDecoration, MinimapPosition,
    MinimapSectionHeaderStyle,
};

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

/// `color` at `factor`, on the minimap's 0.9-opaque decorations canvas.
fn on_canvas(color: Hsla, factor: f32) -> Hsla {
    color.opacity(factor).opacity(0.9)
}

/// Equal up to float rounding.
fn same(a: Hsla, b: Hsla) -> bool {
    (a.h - b.h).abs() < 1e-4
        && (a.s - b.s).abs() < 1e-4
        && (a.l - b.l).abs() < 1e-4
        && (a.a - b.a).abs() < 1e-4
}

fn lines(count: usize) -> String {
    (0..count)
        .map(|i| format!("line {i} with some text"))
        .collect::<Vec<_>>()
        .join("\n")
}

fn scroll_top(editor: &Entity<CodeEditor>, cx: &mut VisualTestContext) -> Pixels {
    editor.read_with(cx, |e, _| e.scroll_position().y)
}

fn set_scroll_top(editor: &Entity<CodeEditor>, cx: &mut VisualTestContext, top: Pixels) {
    editor.update(cx, |e, cx| e.set_scroll_position(point(px(0.), top), cx));
    paint(cx);
}

fn minimap(editor: &Entity<CodeEditor>, cx: &mut VisualTestContext) -> gpui::Bounds<Pixels> {
    editor.read_with(cx, |e, _| e.minimap_for_tests().unwrap().0)
}

fn slider_state(
    editor: &Entity<CodeEditor>,
    cx: &mut VisualTestContext,
) -> (gpui::Bounds<Pixels>, bool, Option<Hsla>) {
    editor.read_with(cx, |e, _| e.minimap_slider_for_tests().unwrap())
}

/// Monaco `MinimapLayout.create`: at the end of a long file the minimap
/// shows its last line, and the slider stays inside the minimap with the
/// last line at its top.
#[gpui::test]
fn the_end_of_a_long_file_shows(cx: &mut TestAppContext) {
    let (editor, cx) = setup(&lines(2000), |_| {}, cx);
    let (start, end, _) = editor.read_with(cx, |e, _| e.minimap_lines_for_tests().unwrap());
    assert_eq!(start, 1);
    assert!(end < 2000);
    set_scroll_top(&editor, cx, px(1e7));
    let (start, end, top_pad) = editor.read_with(cx, |e, _| e.minimap_lines_for_tests().unwrap());
    assert_eq!(end, 2000, "the last line is drawn");
    assert_eq!(top_pad, 0);
    let bounds = minimap(&editor, cx);
    let (slider, needed, _) = slider_state(&editor, cx);
    assert!(needed);
    assert!(
        slider.bottom() <= bounds.bottom() + px(0.5),
        "{slider:?} {bounds:?}"
    );
    // The last line (at the top of the scrolled-to-the-end viewport) is
    // at the slider's top.
    let (_, line, _) = editor.read_with(cx, |e, _| e.minimap_for_tests().unwrap());
    let last_line_top = bounds.top() + line * (2000 - start) as f32;
    assert!(
        (slider.top() - last_line_top).abs() < px(1.),
        "{slider:?} {last_line_top:?}"
    );
    // Halfway, the slider is near the middle of its travel.
    let max = scroll_top(&editor, cx);
    set_scroll_top(&editor, cx, max / 2.);
    let (slider, _, _) = slider_state(&editor, cx);
    let travel = bounds.size.height - slider.size.height;
    let at = slider.top() - bounds.top();
    assert!((at - travel / 2.).abs() < px(3.), "{at:?} {travel:?}");
}

/// Monaco `_startSliderDragging`: pressing the slider doesn't scroll, and
/// it follows the pointer by the delta (`getDesiredScrollTopFromDelta`).
#[gpui::test]
fn dragging_the_slider_follows_the_pointer(cx: &mut TestAppContext) {
    let (editor, cx) = setup(&lines(300), |_| {}, cx);
    set_scroll_top(&editor, cx, px(400.));
    let (before, _, _) = slider_state(&editor, cx);
    let grab = point(before.center().x, before.top() + px(3.));
    cx.simulate_mouse_down(grab, MouseButton::Left, Modifiers::default());
    paint(cx);
    assert_eq!(scroll_top(&editor, cx), px(400.), "no jump");
    // Monaco captures the pointer on the slider: the arrow for the whole
    // drag, over the text too.
    let drag_cursor =
        |cx: &mut VisualTestContext| editor.read_with(cx, |e, _| e.drag_mouse_cursor_for_tests());
    assert_eq!(drag_cursor(cx), Some(gpui::CursorStyle::Arrow));
    // Active while dragging.
    let colors = editor.read_with(cx, |e, _| e.theme().colors.clone());
    // (Fading in over 100ms: compare the color without its alpha.)
    let hsl = |c: Hsla| (c.h, c.s, c.l);
    let active = slider_state(&editor, cx).2.unwrap();
    assert_eq!(hsl(active), hsl(colors.minimap_slider_active));
    assert!(active.a > 0.);

    cx.simulate_mouse_move(
        point(grab.x, grab.y + px(20.)),
        MouseButton::Left,
        Modifiers::default(),
    );
    paint(cx);
    let (after, _, _) = slider_state(&editor, cx);
    assert!(
        (after.top() - before.top() - px(20.)).abs() < px(1.),
        "{before:?} {after:?}"
    );
    assert!(scroll_top(&editor, cx) > px(400.));
    // Far outside the minimap the drag goes on, and the slider stays
    // shown in its active color.
    cx.simulate_mouse_move(
        point(px(10.), grab.y + px(20.)),
        MouseButton::Left,
        Modifiers::default(),
    );
    paint(cx);
    assert_eq!(
        hsl(slider_state(&editor, cx).2.unwrap()),
        hsl(colors.minimap_slider_active)
    );
    // Back where it started.
    cx.simulate_mouse_move(grab, MouseButton::Left, Modifiers::default());
    paint(cx);
    assert_eq!(scroll_top(&editor, cx), px(400.));
    cx.simulate_mouse_up(grab, MouseButton::Left, Modifiers::default());
    paint(cx);
    assert_eq!(drag_cursor(cx), None);
}

/// Monaco `revealLineNumber`: a click off the slider centers the clicked
/// line and doesn't start a drag.
#[gpui::test]
fn clicking_reveals_the_line_in_the_center(cx: &mut TestAppContext) {
    let (editor, cx) = setup(&lines(300), |o| o.smooth_scrolling = false, cx);
    let bounds = minimap(&editor, cx);
    let (_, line, _) = editor.read_with(cx, |e, _| e.minimap_for_tests().unwrap());
    // All 300 lines fit: line 200 is at 200 lines' height.
    let target = point(bounds.center().x, bounds.top() + line * 200.5);
    cx.simulate_mouse_down(target, MouseButton::Left, Modifiers::default());
    cx.simulate_mouse_up(target, MouseButton::Left, Modifiers::default());
    paint(cx);
    let (top, _, height, line_height) =
        editor.read_with(cx, |e, _| e.view_metrics_for_tests().unwrap());
    let center_line = (top + height / 2.) / line_height;
    assert!((center_line - 200.5).abs() < 1.5, "{center_line}");
    // Moving with the button up doesn't drag.
    let scrolled = scroll_top(&editor, cx);
    cx.simulate_mouse_move(
        point(target.x, target.y + px(30.)),
        MouseButton::Left,
        Modifiers::default(),
    );
    paint(cx);
    assert_eq!(scroll_top(&editor, cx), scrolled);
}

/// Monaco: in `fill` and `fit` mode a click drags the slider's center to
/// the pointer.
#[gpui::test]
fn clicking_a_filled_minimap_drags_the_slider(cx: &mut TestAppContext) {
    let (editor, cx) = setup(&lines(5000), |o| o.minimap.size = MinimapSize::Fill, cx);
    let bounds = minimap(&editor, cx);
    let (before, needed, _) = slider_state(&editor, cx);
    assert!(needed);
    let target = point(bounds.center().x, bounds.top() + bounds.size.height / 2.);
    cx.simulate_mouse_down(target, MouseButton::Left, Modifiers::default());
    paint(cx);
    let (after, _, _) = slider_state(&editor, cx);
    assert!(after.top() > before.top());
    assert!(
        (after.center().y - target.y).abs() < px(2.),
        "{after:?} {target:?}"
    );
    cx.simulate_mouse_up(target, MouseButton::Left, Modifiers::default());
}

/// Monaco `_renderSelectionLineHighlights` and
/// `_renderSelectionsHighlights`: a band at half `minimap.selectionHighlight`
/// over a multi-line selection's lines but its last, and the selected
/// columns in full.
#[gpui::test]
fn selections_are_highlighted(cx: &mut TestAppContext) {
    let (editor, cx) = setup(&lines(50), |_| {}, cx);
    let color = editor
        .read_with(cx, |e, _| e.theme().colors.minimap_selection_highlight)
        .unwrap();
    assert!(
        editor
            .read_with(cx, |e, _| e.minimap_marks_for_tests())
            .is_empty()
    );
    editor.update(cx, |e, cx| {
        e.set_selections(
            &[Selection::new(PointUtf16::new(2, 0), PointUtf16::new(5, 4))],
            cx,
        )
    });
    paint(cx);
    let marks = editor.read_with(cx, |e, _| e.minimap_marks_for_tests());
    let band = on_canvas(color, 0.5);
    let full = on_canvas(color, 1.);
    let bands: Vec<_> = marks.iter().filter(|(_, c)| same(*c, band)).collect();
    assert_eq!(bands.len(), 1);
    let (_, line, _) = editor.read_with(cx, |e, _| e.minimap_for_tests().unwrap());
    assert!((bands[0].0.size.height - line * 3.).abs() < px(0.01));
    // Four lines of selected columns.
    assert_eq!(marks.iter().filter(|(_, c)| same(*c, full)).count(), 4);
    // A cursor has no highlight (Monaco draws none).
    editor.update(cx, |e, cx| {
        e.set_selections(&[Selection::collapsed(PointUtf16::new(2, 1))], cx)
    });
    paint(cx);
    assert!(
        editor
            .read_with(cx, |e, _| e.minimap_marks_for_tests())
            .is_empty()
    );
}

/// Monaco `_renderDecorationsLineHighlights` and
/// `_renderDecorationsHighlights`: inline decorations get a half-color band
/// and their columns, gutter ones a 2px bar; markers take the theme's
/// `minimap.*Highlight`.
#[gpui::test]
fn decorations_are_drawn(cx: &mut TestAppContext) {
    let (editor, cx) = setup(&lines(50), |_| {}, cx);
    let red = gpui::red();
    let blue = gpui::blue();
    editor.update(cx, |e, cx| {
        e.model().update(cx, |model, cx| {
            model.delta_decorations(
                None,
                &[],
                vec![
                    (
                        66..70,
                        DecorationOptions {
                            minimap: Some(MinimapDecoration {
                                color: Some(red.into()),
                                position: MinimapPosition::Inline,
                                ..Default::default()
                            }),
                            ..Default::default()
                        },
                    ),
                    (
                        30..31,
                        DecorationOptions {
                            minimap: Some(MinimapDecoration {
                                color: Some(blue.into()),
                                position: MinimapPosition::Gutter,
                                ..Default::default()
                            }),
                            ..Default::default()
                        },
                    ),
                ],
                cx,
            );
        })
    });
    paint(cx);
    let marks = editor.read_with(cx, |e, _| e.minimap_marks_for_tests());
    let bounds = minimap(&editor, cx);
    let red_band = on_canvas(red, 0.5);
    let red_span = on_canvas(red, 1.);
    let band = marks
        .iter()
        .find(|(_, c)| same(*c, red_band))
        .unwrap_or_else(|| panic!("{marks:?}"))
        .0;
    let span = marks.iter().find(|(_, c)| same(*c, red_span)).unwrap().0;
    // The band starts after the 8 canvas pixel gutter and spans the width.
    assert!(band.left() > bounds.left() && band.right() >= bounds.right() - px(0.5));
    // `line` is four characters.
    assert_eq!(span.left(), band.left());
    assert!(span.size.width > px(0.) && span.size.width < band.size.width);
    let bar = marks
        .iter()
        .find(|(_, c)| same(*c, on_canvas(blue, 1.)))
        .unwrap()
        .0;
    assert!(bar.left() > bounds.left() && bar.right() <= band.left());

    // An error marker.
    let error = editor
        .read_with(cx, |e, _| e.theme().colors.minimap_error_highlight)
        .unwrap();
    editor.update(cx, |e, cx| {
        e.model().update(cx, |model, cx| {
            model.set_markers(
                "test",
                vec![Marker {
                    severity: MarkerSeverity::Error,
                    message: "bad".into(),
                    range: PointUtf16::new(10, 0)..PointUtf16::new(10, 4),
                    ..Default::default()
                }],
                cx,
            )
        })
    });
    paint(cx);
    let marks = editor.read_with(cx, |e, _| e.minimap_marks_for_tests());
    assert!(marks.iter().any(|(_, c)| same(*c, on_canvas(error, 1.))));
}

/// Find matches are drawn in `minimap.findMatchHighlight`.
#[gpui::test]
fn find_matches_are_drawn(cx: &mut TestAppContext) {
    let (editor, cx) = setup(&lines(50), |_| {}, cx);
    let color = editor
        .read_with(cx, |e, _| e.theme().colors.minimap_find_match_highlight)
        .unwrap();
    editor.update(cx, |e, cx| {
        e.start_find_model_for_tests(cx);
        let mut options = e.find_options().clone();
        options.query = "line 4".to_string();
        e.change_find_options(options, false, cx);
    });
    paint(cx);
    let marks = editor.read_with(cx, |e, _| e.minimap_marks_for_tests());
    // "line 4" and "line 40" to "line 49".
    let spans = marks
        .iter()
        .filter(|(_, c)| same(*c, on_canvas(color, 1.)))
        .count();
    assert_eq!(spans, 11, "{marks:?}");
}

/// Monaco's section header decorations (`minimap.sectionHeaderText` and
/// `sectionHeaderStyle`).
#[gpui::test]
fn decorations_add_section_headers(cx: &mut TestAppContext) {
    let (editor, cx) = setup(&lines(50), |_| {}, cx);
    editor.update(cx, |e, cx| {
        e.model().update(cx, |model, cx| {
            model.delta_decorations(
                None,
                &[],
                vec![(
                    50..51,
                    DecorationOptions {
                        minimap: Some(MinimapDecoration {
                            section_header_style: Some(MinimapSectionHeaderStyle::Underlined),
                            section_header_text: Some("Setup".into()),
                            ..Default::default()
                        }),
                        ..Default::default()
                    },
                )],
                cx,
            );
        })
    });
    paint(cx);
    let headers = editor.read_with(cx, |e, _| e.minimap_section_headers_for_tests());
    assert_eq!(headers, vec![("Setup".to_string(), true)]);
}

/// Monaco `.minimap-slider`: hidden until the mouse is over the minimap
/// (`showSlider: mouseover`), in the hover color over the slider, and not
/// shown when nothing scrolls.
#[gpui::test]
fn slider_visibility_and_colors(cx: &mut TestAppContext) {
    let (editor, cx) = setup(&lines(300), |_| {}, cx);
    let colors = editor.read_with(cx, |e, _| e.theme().colors.clone());
    assert_eq!(slider_state(&editor, cx).2, None);
    let bounds = minimap(&editor, cx);
    let (slider_bounds, _, _) = slider_state(&editor, cx);
    cx.simulate_mouse_move(
        point(bounds.center().x, bounds.bottom() - px(5.)),
        None,
        Modifiers::default(),
    );
    paint(cx);
    // Fading in over 100ms; once in, the plain color.
    let shown = slider_state(&editor, cx).2.unwrap();
    assert_eq!(
        (shown.h, shown.s, shown.l),
        (
            colors.minimap_slider.h,
            colors.minimap_slider.s,
            colors.minimap_slider.l
        )
    );
    cx.simulate_mouse_move(slider_bounds.center(), None, Modifiers::default());
    paint(cx);
    let hovered = slider_state(&editor, cx).2.unwrap();
    assert_eq!(
        (hovered.h, hovered.s, hovered.l),
        (
            colors.minimap_slider_hover.h,
            colors.minimap_slider_hover.s,
            colors.minimap_slider_hover.l
        )
    );

    // A document that doesn't scroll has no slider.
    let (editor, cx) = setup(
        "one\ntwo",
        |o| {
            o.minimap.always_show_slider = true;
            o.scroll_beyond_last_line = false;
        },
        cx,
    );
    let (_, needed, color) = slider_state(&editor, cx);
    assert!(!needed);
    assert_eq!(color, None);
}

/// Monaco `minimap-shadow-visible`: shown while the text scrolls right.
#[gpui::test]
fn shadow_while_text_continues_right(cx: &mut TestAppContext) {
    let (editor, cx) = setup(&"x".repeat(1000), |_| {}, cx);
    assert!(editor.read_with(cx, |e, _| e.minimap_shadow_for_tests()));
    editor.update(cx, |e, cx| {
        e.set_scroll_position(point(px(1e6), px(0.)), cx)
    });
    paint(cx);
    assert!(!editor.read_with(cx, |e, _| e.minimap_shadow_for_tests()));
}

/// Scrolling moves the text canvas's tiles instead of drawing and uploading
/// a new canvas: tiles still in view keep their images.
#[gpui::test]
fn scrolling_reuses_the_drawn_tiles(cx: &mut TestAppContext) {
    let (editor, cx) = setup(&lines(5000), |_| {}, cx);
    set_scroll_top(&editor, cx, px(40_000.));
    let before = editor.read_with(cx, |e, _| e.minimap_tiles_for_tests());
    assert!(!before.is_empty());
    let (start, end, _) = editor.read_with(cx, |e, _| e.minimap_lines_for_tests().unwrap());
    // Every line drawn is in a tile.
    assert!(before.first().unwrap().0 <= start);
    assert!(before.last().unwrap().0 + 127 >= end);
    // A page further, the tiles still in view are the same images.
    set_scroll_top(&editor, cx, px(40_600.));
    let after = editor.read_with(cx, |e, _| e.minimap_tiles_for_tests());
    let kept = after.iter().filter(|tile| before.contains(tile)).count();
    assert!(kept >= after.len() - 1, "{before:?} {after:?}");
    // Typing redraws them.
    editor.update(cx, |e, cx| {
        e.model()
            .update(cx, |m, cx| m.set_value("changed\ntext", cx))
    });
    paint(cx);
    let typed = editor.read_with(cx, |e, _| e.minimap_tiles_for_tests());
    assert!(typed.iter().all(|tile| !after.contains(tile)));
}

/// Each drawn tile's first minimap line and pixels.
type TilePixels = Vec<(u32, Vec<u8>)>;

/// The tiles' pixels, then the pixels of the same tiles drawn from scratch.
fn redrawn_from_scratch(
    editor: &Entity<CodeEditor>,
    cx: &mut VisualTestContext,
) -> (TilePixels, TilePixels) {
    let kept = editor.read_with(cx, |e, _| e.minimap_tile_pixels_for_tests());
    editor.update(cx, |e, _| e.clear_minimap_tiles_for_tests());
    paint(cx);
    let fresh = editor.read_with(cx, |e, _| e.minimap_tile_pixels_for_tests());
    (kept, fresh)
}

/// Monaco's `onLinesChanged`: typing in a line redraws only the tile with
/// that line, and its other lines keep their pixels.
#[gpui::test]
fn typing_redraws_only_the_edited_lines(cx: &mut TestAppContext) {
    let (editor, cx) = setup(&lines(5000), |_| {}, cx);
    set_scroll_top(&editor, cx, px(40_000.));
    let before = editor.read_with(cx, |e, _| e.minimap_tiles_for_tests());
    assert!(before.len() >= 2, "{before:?}");
    // A line in the second tile.
    let row = before[1].0 + 8;
    editor.update(cx, |e, cx| {
        e.model().update(cx, |m, cx| {
            let offset = m.buffer().rope().point_to_offset(Point::new(row, 4));
            m.edit([(offset..offset, "xyz")], cx).unwrap();
        })
    });
    paint(cx);
    let after = editor.read_with(cx, |e, _| e.minimap_tiles_for_tests());
    for (tile, image) in &after {
        let redrawn = !before.contains(&(*tile, *image));
        assert_eq!(redrawn, *tile == before[1].0, "{before:?} {after:?}");
    }
    let (kept, fresh) = redrawn_from_scratch(&editor, cx);
    assert_eq!(kept, fresh);
}

/// Monaco's `onLinesInserted` and `onLinesDeleted`: the lines below move
/// with their pixels, and the result is what drawing the tiles again gives.
#[gpui::test]
fn inserted_and_deleted_lines_move_the_others(cx: &mut TestAppContext) {
    let (editor, cx) = setup(&lines(5000), |_| {}, cx);
    set_scroll_top(&editor, cx, px(40_000.));
    let first = editor.read_with(cx, |e, _| e.minimap_tiles_for_tests())[1].0;
    editor.update(cx, |e, cx| {
        e.set_position(PointUtf16::new(first + 20, 3), cx);
        e.type_text("\n\nnew", cx);
    });
    paint(cx);
    let (kept, fresh) = redrawn_from_scratch(&editor, cx);
    assert_eq!(kept, fresh);
    // Deleting lines across a tile boundary.
    editor.update(cx, |e, cx| {
        e.set_selection(
            Selection::from_range(
                PointUtf16::new(first + 120, 2)..PointUtf16::new(first + 135, 5),
                false,
            ),
            cx,
        );
        e.type_text("q", cx);
    });
    paint(cx);
    let (kept, fresh) = redrawn_from_scratch(&editor, cx);
    assert_eq!(kept, fresh);
}

/// Monaco's `onTokensChanged`: lines whose colors the parser changed (an
/// opened string colors the lines after it) are drawn again too.
#[gpui::test]
fn lines_recolored_by_the_parser_are_redrawn(cx: &mut TestAppContext) {
    cx.update(|cx| {
        gpui_monaco::init(cx);
        gpui_monaco::set_language_registry(cx, gpui_monaco_languages::registry());
    });
    let text = (0..600)
        .map(|i| format!("fn f{i}() {{ let s = 1; }}"))
        .collect::<Vec<_>>()
        .join("\n");
    let (editor, cx) = cx.add_window_view(|window, cx| {
        let model = gpui::AppContext::new(cx, |_| {
            gpui_monaco::TextModel::new(&text).with_language("rust")
        });
        CodeEditor::new(model, window, cx)
    });
    cx.update(|window, cx| window.focus(&editor.focus_handle(cx), cx));
    paint(cx);
    let before = editor.read_with(cx, |e, _| e.minimap_tile_pixels_for_tests());
    editor.update(cx, |e, cx| {
        e.set_position(PointUtf16::new(3, 20), cx);
        e.type_text("\"", cx);
    });
    paint(cx);
    let (kept, fresh) = redrawn_from_scratch(&editor, cx);
    assert_ne!(kept, before);
    assert_eq!(kept, fresh);
}
