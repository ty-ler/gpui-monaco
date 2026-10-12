//! Scrollbars, the overview ruler and the scroll shadow (Monaco
//! `DecorationsOverviewRuler`, `ScrollDecorationViewPart` and the
//! scrollable element's `ScrollbarState`).

use gpui::{
    AppContext as _, Bounds, Context, Entity, Focusable, Hsla, IntoElement, Modifiers, MouseButton,
    ParentElement as _, Pixels, Render, Styled as _, TestAppContext, VisualTestContext, Window,
    div, point, px, rgb,
};
use gpui_monaco::options::{EditorOptions, MinimapSide};
use gpui_monaco::{
    CodeEditor, DecorationOptions, Marker, MarkerSeverity, OverviewRulerDecoration,
    OverviewRulerLane, ScrollType, TextModel, Theme, ViewZone, ZoneHeight,
};
use text::PointUtf16;

/// A window with the editor at (0, 0), 600px wide and `height` tall.
struct Host {
    editor: Entity<CodeEditor>,
    height: Pixels,
}

impl Render for Host {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
            .size_full()
            .child(div().w(px(600.)).h(self.height).child(self.editor.clone()))
    }
}

fn setup<'a>(
    text: &str,
    height: f32,
    options: impl FnOnce(&mut EditorOptions),
    cx: &'a mut TestAppContext,
) -> (Entity<CodeEditor>, &'a mut VisualTestContext) {
    cx.update(gpui_monaco::init);
    let text = text.to_string();
    let (host, cx) = cx.add_window_view(|window, cx| {
        let editor = cx.new(|cx| {
            let model = cx.new(|_| TextModel::new(&text));
            let mut editor = CodeEditor::new(model, window, cx);
            let mut o = editor.options().clone();
            o.minimap.enabled = false;
            o.sticky_scroll.enabled = false;
            options(&mut o);
            editor.update_options(o, cx);
            editor
        });
        Host {
            editor,
            height: px(height),
        }
    });
    let editor = host.read_with(cx, |host, _| host.editor.clone());
    cx.update(|window, cx| window.focus(&editor.focus_handle(cx), cx));
    draw(cx);
    (editor, cx)
}

fn draw(cx: &mut VisualTestContext) {
    cx.update(|window, _| window.refresh());
    cx.run_until_parked();
}

fn lines(count: usize) -> String {
    (0..count)
        .map(|i| format!("line {i}"))
        .collect::<Vec<_>>()
        .join("\n")
}

fn update_options(
    editor: &Entity<CodeEditor>,
    cx: &mut VisualTestContext,
    f: impl FnOnce(&mut EditorOptions),
) {
    editor.update(cx, |e, cx| {
        let mut o = e.options().clone();
        f(&mut o);
        e.update_options(o, cx);
    });
    draw(cx);
}

/// Marks rows `rows` (inclusive) in `color`, with z-index `z`.
fn decorate(
    editor: &Entity<CodeEditor>,
    cx: &mut VisualTestContext,
    marks: &[(u32, u32, Hsla, OverviewRulerLane, i32)],
) {
    editor.update(cx, |e, cx| {
        e.model().update(cx, |m, cx| {
            let decorations = marks
                .iter()
                .map(|&(start, end, color, lane, z_index)| {
                    let buffer = m.buffer();
                    let start = buffer.point_to_offset(text::Point::new(start, 0));
                    let end = buffer.point_to_offset(text::Point::new(end, 1));
                    (
                        start..end,
                        DecorationOptions {
                            overview_ruler: Some(OverviewRulerDecoration::new(color, lane)),
                            z_index,
                            ..Default::default()
                        },
                    )
                })
                .collect();
            m.delta_decorations(None, &[], decorations, cx);
        })
    });
    draw(cx);
}

fn ruler(
    editor: &Entity<CodeEditor>,
    cx: &mut VisualTestContext,
) -> gpui_monaco::OverviewRulerForTests {
    editor.read_with(cx, |e, _| e.overview_ruler_for_tests().unwrap())
}

/// The marks in `color`.
fn marks_in(
    editor: &Entity<CodeEditor>,
    cx: &mut VisualTestContext,
    color: Hsla,
) -> Vec<Bounds<Pixels>> {
    ruler(editor, cx)
        .marks
        .into_iter()
        .filter(|(_, c)| *c == color)
        .map(|(bounds, _)| bounds)
        .collect()
}

fn scale(cx: &mut VisualTestContext) -> f32 {
    cx.update(|window, _| window.scale_factor())
}

/// Monaco's `(offset * canvasHeight / scrollHeight) | 0`, back in pixels
/// from the ruler's top.
fn monaco_y(editor: &Entity<CodeEditor>, cx: &mut VisualTestContext, offset: f32) -> f32 {
    let pixel_ratio = scale(cx);
    let ruler = ruler(editor, cx);
    let scroll_height = editor.read_with(cx, |e, _| f32::from(e.scroll_height().unwrap()));
    let canvas_height = (f32::from(ruler.bounds.size.height) * pixel_ratio) as i32 as f32;
    ((offset * canvas_height / scroll_height) as i32) as f32 / pixel_ratio
}

fn line_height(editor: &Entity<CodeEditor>, cx: &mut VisualTestContext) -> f32 {
    editor.read_with(cx, |e, _| f32::from(e.view_metrics_for_tests().unwrap().3))
}

fn red() -> Hsla {
    rgb(0xff0000).into()
}

fn blue() -> Hsla {
    rgb(0x0000ff).into()
}

#[gpui::test]
fn marks_follow_the_scroll_height_like_the_slider(cx: &mut TestAppContext) {
    // 10 lines with `scrollBeyondLastLine`: the scroll height is the lines
    // plus a viewport less a line, so the last line's mark is well above
    // the bottom (it used to be at the bottom).
    let (editor, cx) = setup(&lines(10), 400., |_| {}, cx);
    decorate(&editor, cx, &[(9, 9, red(), OverviewRulerLane::Right, 0)]);
    let lh = line_height(&editor, cx);
    let ruler_bounds = ruler(&editor, cx).bounds;
    let mark = marks_in(&editor, cx, red())[0];
    let top = f32::from(mark.top() - ruler_bounds.top());
    assert_eq!(top, monaco_y(&editor, cx, 9. * lh));
    assert!(top < f32::from(ruler_bounds.size.height) / 2., "{top}");

    // `padding.top` moves it down.
    update_options(&editor, cx, |o| o.padding.top = px(50.));
    let top = f32::from(marks_in(&editor, cx, red())[0].top() - ruler_bounds.top());
    assert_eq!(top, monaco_y(&editor, cx, 50. + 9. * lh));

    // So does a view zone above it (a code lens, say).
    editor.update(cx, |e, cx| {
        let view = cx.new(|_| Blank).into();
        e.add_view_zone(ViewZone::new(2, ZoneHeight::Pixels(px(300.)), view), cx);
    });
    draw(cx);
    let top = f32::from(marks_in(&editor, cx, red())[0].top() - ruler_bounds.top());
    assert_eq!(top, monaco_y(&editor, cx, 50. + 300. + 9. * lh));

    // Without scrolling beyond the last line, short content fills the
    // viewport's height (Monaco's scroll height is at least the viewport).
    update_options(&editor, cx, |o| {
        o.scroll_beyond_last_line = false;
        o.padding.top = px(0.);
    });
    let top = f32::from(marks_in(&editor, cx, red())[0].top() - ruler_bounds.top());
    assert_eq!(top, monaco_y(&editor, cx, 300. + 9. * lh));
}

struct Blank;

impl Render for Blank {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
    }
}

#[gpui::test]
fn marks_span_their_lines_merge_and_sort(cx: &mut TestAppContext) {
    let (editor, cx) = setup(&lines(400), 400., |_| {}, cx);
    decorate(
        &editor,
        cx,
        &[
            // A range of lines spans them.
            (100, 199, red(), OverviewRulerLane::Right, 0),
            // Neighbours of a color in a lane merge.
            (300, 300, red(), OverviewRulerLane::Right, 0),
            (301, 301, red(), OverviewRulerLane::Right, 0),
            // Under the red ones (a lower z-index paints first).
            (100, 100, blue(), OverviewRulerLane::Right, -1),
        ],
    );
    let lh = line_height(&editor, cx);
    let ruler = ruler(&editor, cx);
    let red_marks = marks_in(&editor, cx, red());
    assert_eq!(red_marks.len(), 2, "{red_marks:?}");
    let span = red_marks[0];
    assert_eq!(
        f32::from(span.top() - ruler.bounds.top()),
        monaco_y(&editor, cx, 100. * lh)
    );
    assert_eq!(
        f32::from(span.bottom() - ruler.bounds.top()),
        monaco_y(&editor, cx, 200. * lh)
    );
    // Short ranges get Monaco's 6px minimum.
    assert!(red_marks[1].size.height >= px(6.) - px(0.01));
    assert_eq!(ruler.marks[0].1, blue());
}

#[gpui::test]
fn diagnostics_use_the_overview_ruler_theme_colors(cx: &mut TestAppContext) {
    let (editor, cx) = setup(&lines(100), 400., |_| {}, cx);
    let theme_colors = [
        ("editorOverviewRuler.errorForeground", "#ff0001"),
        ("editorOverviewRuler.warningForeground", "#ff0002"),
        ("editorOverviewRuler.infoForeground", "#ff0003"),
    ]
    .into_iter()
    .map(|(k, v)| (k.to_string(), v.to_string()))
    .collect();
    let theme = Theme::define("ruler", &Theme::vs_dark(), true, Vec::new(), &theme_colors);
    let colors = theme.colors.clone();
    cx.update(|_, cx| {
        gpui_monaco::define_theme(cx, theme).unwrap();
        gpui_monaco::set_theme(cx, "ruler");
    });
    let marker = |severity, row| Marker {
        severity,
        message: "problem".into(),
        range: PointUtf16::new(row, 0)..PointUtf16::new(row, 4),
        ..Default::default()
    };
    editor.update(cx, |e, cx| {
        e.model().update(cx, |m, cx| {
            m.set_markers(
                "test",
                vec![
                    marker(MarkerSeverity::Error, 10),
                    marker(MarkerSeverity::Warning, 40),
                    marker(MarkerSeverity::Info, 70),
                    marker(MarkerSeverity::Hint, 90),
                ],
                cx,
            )
        })
    });
    draw(cx);
    let ruler = ruler(&editor, cx);
    let mut marks: Vec<_> = ruler.marks.iter().map(|(b, c)| (b.top(), *c)).collect();
    marks.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap());
    let colors_in_order: Vec<Hsla> = marks.iter().map(|(_, c)| *c).collect();
    // Infos too, hints not, in the theme's colors.
    assert_eq!(
        colors_in_order,
        vec![
            colors.overview_ruler_error,
            colors.overview_ruler_warning,
            colors.overview_ruler_info,
        ]
    );
    // In the right lane, the right half with the default two lanes.
    let right = ruler.marks[0].0;
    assert!(right.left() >= ruler.bounds.left() + ruler.bounds.size.width / 2.);
    assert!((right.right() - ruler.bounds.right()).abs() < px(0.01));
}

#[gpui::test]
fn the_scroll_shadow_is_monacos_top_decoration(cx: &mut TestAppContext) {
    let wide = format!("{}\n{}", "x".repeat(400), lines(200));
    let (editor, cx) = setup(&wide, 400., |_| {}, cx);
    let shadow = |editor: &Entity<CodeEditor>, cx: &mut VisualTestContext| {
        editor.read_with(cx, |e, _| e.scroll_shadow_for_tests())
    };
    assert_eq!(shadow(&editor, cx), None);
    // Scrolled sideways only: no shadow (Monaco draws no left shadow).
    editor.update(cx, |e, cx| {
        e.set_scroll_position(point(px(100.), px(0.)), cx)
    });
    draw(cx);
    assert_eq!(shadow(&editor, cx), None);

    editor.update(cx, |e, cx| {
        e.set_scroll_top(px(100.), ScrollType::Immediate, cx)
    });
    draw(cx);
    // 6px across the whole editor, the scrollbar included.
    let shadow_box = shadow(&editor, cx).unwrap();
    assert_eq!(shadow_box.origin, point(px(0.), px(0.)));
    assert_eq!(shadow_box.size, gpui::size(px(600.), px(6.)));

    // With the minimap on the right, not over the scrollbar.
    update_options(&editor, cx, |o| o.minimap.enabled = true);
    let shadow_box = shadow(&editor, cx).unwrap();
    assert_eq!(shadow_box.size, gpui::size(px(600. - 14.), px(6.)));
    // On the left, across everything.
    update_options(&editor, cx, |o| o.minimap.side = MinimapSide::Left);
    let shadow_box = shadow(&editor, cx).unwrap();
    assert_eq!(shadow_box.right(), px(600.));

    // `scrollbar.useShadows: false` turns it off.
    update_options(&editor, cx, |o| o.scrollbar.use_shadows = false);
    assert_eq!(shadow(&editor, cx), None);
}

#[gpui::test]
fn cursor_marks_are_translucent_and_two_pixels(cx: &mut TestAppContext) {
    let (editor, cx) = setup(&lines(100), 400., |_| {}, cx);
    let colors = editor.read_with(cx, |e, _| e.colors().clone());
    let pixel_ratio = scale(cx);
    let ruler = ruler(&editor, cx);
    assert_eq!(ruler.cursors.len(), 1);
    let (cursor, color) = ruler.cursors[0];
    assert_eq!(color, colors.cursor.opacity(0.7));
    assert_eq!(
        cursor.size.height,
        px(((2. * pixel_ratio) as i32) as f32 / pixel_ratio)
    );
    // Across every lane, from 1px in.
    assert_eq!(cursor.left(), ruler.bounds.left() + px(1. / pixel_ratio));
    assert_eq!(cursor.right(), ruler.bounds.right());

    // Several cursors: the primary and secondary colors.
    editor.update(cx, |e, cx| {
        e.set_selections(
            &[
                gpui_monaco::cursor::Selection::collapsed(PointUtf16::new(60, 0)),
                gpui_monaco::cursor::Selection::collapsed(PointUtf16::new(10, 0)),
            ],
            cx,
        )
    });
    draw(cx);
    let ruler_now = editor.read_with(cx, |e, _| e.overview_ruler_for_tests().unwrap());
    let mut cursors = ruler_now.cursors.clone();
    cursors.sort_by(|a, b| a.0.top().partial_cmp(&b.0.top()).unwrap());
    assert_eq!(
        cursors.iter().map(|(_, c)| *c).collect::<Vec<_>>(),
        vec![
            colors.multi_cursor_secondary_foreground.opacity(0.7),
            colors.multi_cursor_primary_foreground.opacity(0.7),
        ]
    );

    update_options(&editor, cx, |o| o.hide_cursor_in_overview_ruler = true);
    assert!(
        editor
            .read_with(cx, |e, _| e.overview_ruler_for_tests().unwrap())
            .cursors
            .is_empty()
    );
}

#[gpui::test]
fn lanes_border_and_background(cx: &mut TestAppContext) {
    let (editor, cx) = setup(&lines(100), 400., |_| {}, cx);
    decorate(
        &editor,
        cx,
        &[
            (10, 10, red(), OverviewRulerLane::Left, 0),
            (10, 10, blue(), OverviewRulerLane::Center, 0),
        ],
    );
    let pixel_ratio = scale(cx);
    let canvas_width = (14. * pixel_ratio) as i32;
    let device = |v: i32| px(v as f32 / pixel_ratio);
    // Two lanes by default (standalone Monaco), `_initLanes(1, canvasWidth,
    // 2)`: 1px in, the left half, which the center shares, and the right
    // the rest.
    let ruler = ruler(&editor, cx);
    let half = (canvas_width - 1) / 2;
    for color in [red(), blue()] {
        let mark = marks_in(&editor, cx, color)[0];
        assert_eq!(mark.left(), ruler.bounds.left() + device(1));
        assert_eq!(mark.size.width, device(half));
    }
    // `_initLanes(1, canvasWidth, 3)`: the sides floored thirds, the center
    // the rest.
    update_options(&editor, cx, |o| o.overview_ruler_lanes = 3);
    let ruler = self::ruler(&editor, cx);
    let third = (canvas_width - 1) / 3;
    let left = marks_in(&editor, cx, red())[0];
    assert_eq!(left.left(), ruler.bounds.left() + device(1));
    assert_eq!(left.size.width, device(third));
    let center = marks_in(&editor, cx, blue())[0];
    assert_eq!(center.left(), ruler.bounds.left() + device(1 + third));
    assert_eq!(center.size.width, device(canvas_width - 1 - 2 * third));

    // A left and a top edge, one device pixel each, half covered.
    let colors = editor.read_with(cx, |e, _| e.colors().clone());
    let ([border_left, border_top], color) = ruler.border.unwrap();
    assert_eq!(color, colors.overview_ruler_border.opacity(0.5));
    assert_eq!(border_left.size.width, device(1));
    assert_eq!(border_left.size.height, ruler.bounds.size.height);
    assert_eq!(border_top.size.height, device(1));
    assert_eq!(border_top.left(), ruler.bounds.left() + device(1));
    update_options(&editor, cx, |o| o.overview_ruler_border = false);
    assert!(
        editor
            .read_with(cx, |e, _| e.overview_ruler_for_tests().unwrap())
            .border
            .is_none()
    );

    // No background, unless the minimap is on the right: then the
    // editor's.
    assert_eq!(ruler.background, None);
    update_options(&editor, cx, |o| o.minimap.enabled = true);
    let background = editor
        .read_with(cx, |e, _| e.overview_ruler_for_tests().unwrap())
        .background;
    assert_eq!(background, Some(colors.background));
}

#[gpui::test]
fn paging_the_track_then_dragging(cx: &mut TestAppContext) {
    let (editor, cx) = setup(&lines(500), 400., |o| o.scrollbar.scroll_by_page = true, cx);
    let (bar, thumb) = editor.read_with(cx, |e, _| e.vertical_scrollbar_for_tests().unwrap());
    let start = point(bar.center().x, thumb.bottom() + px(100.));
    cx.simulate_mouse_down(start, MouseButton::Left, Modifiers::default());
    draw(cx);
    // A page is the viewport's height (Monaco's visible size).
    let scroll_top = |editor: &Entity<CodeEditor>, cx: &mut VisualTestContext| {
        editor.read_with(cx, |e, _| e.scroll_position().y)
    };
    assert_eq!(scroll_top(&editor, cx), px(400.));
    let (_, paged_thumb) = editor.read_with(cx, |e, _| e.vertical_scrollbar_for_tests().unwrap());
    // Then the slider follows the mouse from there.
    cx.simulate_mouse_move(
        start + point(px(0.), px(20.)),
        MouseButton::Left,
        Modifiers::default(),
    );
    draw(cx);
    let (_, dragged_thumb) = editor.read_with(cx, |e, _| e.vertical_scrollbar_for_tests().unwrap());
    assert!(
        (dragged_thumb.top() - paged_thumb.top() - px(20.)).abs() < px(0.5),
        "{paged_thumb:?} {dragged_thumb:?}"
    );
    cx.simulate_mouse_up(start, MouseButton::Left, Modifiers::default());
}

#[gpui::test]
fn slider_sizes_are_rounded(cx: &mut TestAppContext) {
    let (editor, cx) = setup(&lines(333), 401., |_| {}, cx);
    let (_, thumb) = editor.read_with(cx, |e, _| e.vertical_scrollbar_for_tests().unwrap());
    let height = f32::from(thumb.size.height);
    assert_eq!(height, height.round());
    assert!(height >= 20.);
}

#[gpui::test]
fn arrows_keep_their_size(cx: &mut TestAppContext) {
    let (editor, cx) = setup(
        &lines(100),
        120.,
        |o| {
            o.scrollbar.vertical_has_arrows = true;
            o.scrollbar.arrow_size = px(40.);
        },
        cx,
    );
    let (bar, thumb) = editor.read_with(cx, |e, _| e.vertical_scrollbar_for_tests().unwrap());
    // Not clamped to a quarter of the bar: the track is between 40px
    // arrows, where `layout_info().overview_ruler` puts the ruler.
    assert!(thumb.top() >= bar.top() + px(40.));
    let position = editor.read_with(cx, |e, _| e.layout_info().unwrap().overview_ruler);
    assert_eq!(position.top, px(40.));
    let ruler = ruler(&editor, cx);
    assert_eq!(ruler.bounds.top(), bar.top() + position.top);
    assert_eq!(ruler.bounds.size.height, position.height);
    assert_eq!(position.height, px(40.));
}
