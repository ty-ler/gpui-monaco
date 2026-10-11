//! Monaco's view zone, content widget and overlay widget API (#252):
//! `IViewZone`'s fields, `IContentWidget`'s and `IOverlayWidget`'s, and
//! what happens to them when the model changes. Every test draws frames,
//! since zones and widgets are laid out while painting.

use std::cell::Cell;
use std::rc::Rc;

use gpui::{
    AppContext as _, Bounds, Context, Entity, Focusable as _, InteractiveElement as _, IntoElement,
    Modifiers, MouseButton, ParentElement as _, Pixels, Render, Styled as _, TestAppContext,
    VisualTestContext, Window, canvas, div, point, px, size,
};
use gpui_monaco::mouse_target::{EmbeddedKind, MouseTargetDetail, MouseTargetKind};
use gpui_monaco::text::PointUtf16;
use gpui_monaco::{
    CodeEditor, CodeEditorEvent, ContentWidget, ContentWidgetPosition, ContentWidgetPreference,
    GlyphMarginWidget, OverlayPosition, OverlayWidget, PositionAffinity, ScrollType, TextModel,
    ViewZone, ZoneHeight,
};

fn p(row: u32, column: u32) -> PointUtf16 {
    PointUtf16::new(row, column)
}

/// GPUI lays out on whole and half pixels.
#[track_caller]
fn assert_near(a: Pixels, b: Pixels) {
    assert!((a - b).abs() < px(1.), "{a:?} != {b:?}");
}

fn setup<'a>(
    text: &str,
    cx: &'a mut TestAppContext,
) -> (Entity<CodeEditor>, &'a mut VisualTestContext) {
    cx.update(gpui_monaco::init);
    let text = text.to_string();
    let (editor, cx) = cx.add_window_view(|window, cx| {
        let model = cx.new(|_| TextModel::new(&text));
        let mut editor = CodeEditor::new(model, window, cx);
        let mut options = editor.options().clone();
        options.minimap.enabled = false;
        editor.update_options(options, cx);
        editor
    });
    cx.update(|window, _| window.activate_window());
    cx.update(|window, cx| window.focus(&editor.focus_handle(cx), cx));
    draw(cx);
    (editor, cx)
}

fn draw(cx: &mut VisualTestContext) {
    cx.update(|window, _| window.refresh());
    cx.run_until_parked();
}

/// Where a probe was painted last.
type Painted = Rc<Cell<Option<Bounds<Pixels>>>>;

/// A view that records where it's painted and how often it's clicked.
struct Probe {
    bounds: Painted,
    clicks: Rc<Cell<usize>>,
    width: Option<Pixels>,
    height: Option<Pixels>,
    text: Option<String>,
}

impl Probe {
    fn new(cx: &mut VisualTestContext) -> (Entity<Probe>, Painted) {
        let bounds = Rc::new(Cell::new(None));
        let probe = cx.update(|_, cx| {
            cx.new(|_| Probe {
                bounds: bounds.clone(),
                clicks: Default::default(),
                width: None,
                height: None,
                text: None,
            })
        });
        (probe, bounds)
    }

    fn sized(cx: &mut VisualTestContext, width: f32, height: f32) -> (Entity<Probe>, Painted) {
        let (probe, bounds) = Probe::new(cx);
        cx.update(|_, cx| {
            probe.update(cx, |p, _| {
                p.width = Some(px(width));
                p.height = Some(px(height));
            })
        });
        (probe, bounds)
    }
}

impl Render for Probe {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let bounds = self.bounds.clone();
        let clicks = self.clicks.clone();
        let mut root = div().relative();
        root = match (self.width, self.height) {
            (Some(w), Some(h)) => root.w(w).h(h),
            _ => root.size_full(),
        };
        root.when_some(self.text.clone(), |root, text| {
            root.whitespace_nowrap().overflow_hidden().child(text)
        })
        .on_mouse_down(MouseButton::Left, move |_, _, _| {
            clicks.set(clicks.get() + 1)
        })
        .child(
            canvas(move |b, _, _| bounds.set(Some(b)), |_, _, _, _| {})
                .absolute()
                .size_full(),
        )
    }
}

use gpui::prelude::FluentBuilder as _;

fn widget_bounds(
    editor: &Entity<CodeEditor>,
    kind: EmbeddedKind,
    cx: &mut VisualTestContext,
) -> Vec<Bounds<Pixels>> {
    let (_, widgets) = editor.read_with(cx, |e, _| e.widget_bounds_for_tests());
    widgets
        .into_iter()
        .filter(|(_, k)| *k == kind)
        .map(|(b, _)| b)
        .collect()
}

fn line_top(editor: &Entity<CodeEditor>, cx: &mut VisualTestContext, row: u32) -> Pixels {
    editor
        .read_with(cx, |e, _| e.scrolled_visible_position(p(row, 0)))
        .unwrap()
        .y
}

fn line_height(editor: &Entity<CodeEditor>, cx: &mut VisualTestContext) -> Pixels {
    editor.read_with(cx, |e, _| e.layout_for_tests().unwrap().1)
}

fn text_left(editor: &Entity<CodeEditor>, cx: &mut VisualTestContext) -> Pixels {
    let info = editor.read_with(cx, |e, _| e.layout_info().unwrap());
    let (bounds, _) = editor.read_with(cx, |e, _| e.widget_bounds_for_tests());
    bounds.left() + info.content_left
}

#[gpui::test]
fn set_model_drops_zones_and_reanchors_widgets(cx: &mut TestAppContext) {
    let (editor, cx) = setup("zero\none\ntwo\nthree\nfour\nfive\nsix\nseven", cx);
    let (zone, _) = Probe::new(cx);
    let (content, content_bounds) = Probe::sized(cx, 20., 10.);
    let (glyph, glyph_bounds) = Probe::new(cx);
    let changes = Rc::new(Cell::new(0));
    let counter = changes.clone();
    cx.update(|_, cx| {
        cx.subscribe(&editor, move |_, event: &CodeEditorEvent, _| {
            if matches!(event, CodeEditorEvent::ViewZonesChanged) {
                counter.set(counter.get() + 1);
            }
        })
        .detach();
    });
    let (content_id, glyph_id) = editor.update(cx, |e, cx| {
        e.add_view_zone(ViewZone::new(3, ZoneHeight::Lines(2.), zone.into()), cx);
        let content_id = e.add_content_widget(
            ContentWidget::new(
                p(5, 2),
                vec![ContentWidgetPreference::Exact],
                content.into(),
            ),
            cx,
        );
        let glyph_id = e.add_glyph_margin_widget(GlyphMarginWidget::new(6, glyph), cx);
        (content_id, glyph_id)
    });
    draw(cx);
    assert_eq!(widget_bounds(&editor, EmbeddedKind::ViewZone, cx).len(), 1);
    changes.set(0);

    // A new model whose text before the old anchors differs: anchors into
    // the old buffer would land anywhere.
    let model = cx.update(|_, cx| {
        cx.new(|_| TextModel::new("a much longer first line\nb\nc\nd\ne\nfffff\ng\nh\ni"))
    });
    editor.update(cx, |e, cx| e.set_model(model, cx));
    draw(cx);
    // Monaco: the view zones went with the old view.
    assert!(widget_bounds(&editor, EmbeddedKind::ViewZone, cx).is_empty());
    assert_eq!(changes.get(), 1);
    assert_eq!(
        line_top(&editor, cx, 4) - line_top(&editor, cx, 3),
        line_height(&editor, cx)
    );
    // Content and glyph widgets stay, at the same position in the new
    // model.
    let expected = editor
        .read_with(cx, |e, _| e.scrolled_visible_position(p(5, 2)))
        .unwrap();
    let content = content_bounds.get().unwrap();
    assert_near(content.left(), expected.x);
    assert_eq!(content.top(), expected.y);
    assert_eq!(glyph_bounds.get().unwrap().top(), line_top(&editor, cx, 6));
    let hits = editor.read_with(cx, |e, _| e.widget_bounds_for_tests().1);
    assert!(hits.iter().any(|(_, k)| *k == EmbeddedKind::ContentWidget));
    assert!(
        hits.iter()
            .any(|(_, k)| *k == EmbeddedKind::GlyphMarginWidget)
    );

    // A shorter model: positions are clipped to it.
    let short = cx.update(|_, cx| cx.new(|_| TextModel::new("x\ny")));
    editor.update(cx, |e, cx| e.set_model(short, cx));
    draw(cx);
    let expected = editor
        .read_with(cx, |e, _| e.scrolled_visible_position(p(1, 1)))
        .unwrap();
    assert_near(content_bounds.get().unwrap().left(), expected.x);
    assert_eq!(content_bounds.get().unwrap().top(), expected.y);
    // Removing still works by id.
    editor.update(cx, |e, cx| {
        e.remove_widget(content_id, cx);
        e.remove_widget(glyph_id, cx);
    });
    draw(cx);
    let hits = editor.read_with(cx, |e, _| e.widget_bounds_for_tests().1);
    assert!(hits.is_empty());
}

#[gpui::test]
fn zones_scroll_horizontally_and_have_a_margin(cx: &mut TestAppContext) {
    let long = "x".repeat(400);
    let (editor, cx) = setup(&format!("short\n{long}\nend\nlast"), cx);
    let (zone, zone_bounds) = Probe::new(cx);
    let (margin, margin_bounds) = Probe::new(cx);
    let id = editor.update(cx, |e, cx| {
        let mut zone = ViewZone::new(1, ZoneHeight::Lines(2.), zone.into());
        zone.margin_view = Some(margin.into());
        e.add_view_zone(zone, cx)
    });
    draw(cx);
    let left = text_left(&editor, cx);
    let scroll_width = editor.read_with(cx, |e, _| e.scroll_width().unwrap());
    let zone = zone_bounds.get().unwrap();
    assert_near(zone.left(), left);
    // Monaco: the zones layer is the scroll width wide.
    assert_near(zone.size.width, scroll_width);
    assert_eq!(zone.size.height, line_height(&editor, cx) * 2.);
    // The margin part: the gutter, beside the zone.
    let (editor_bounds, _) = editor.read_with(cx, |e, _| e.widget_bounds_for_tests());
    let margin = margin_bounds.get().unwrap();
    assert_eq!(margin.left(), editor_bounds.left());
    assert_near(margin.right(), left);
    assert_eq!(
        (margin.top(), margin.size.height),
        (zone.top(), zone.size.height)
    );

    // Scrolled horizontally, the zone moves with the text; the margin stays.
    editor.update(cx, |e, cx| {
        e.set_scroll_left(px(120.), ScrollType::Immediate, cx)
    });
    draw(cx);
    assert_near(zone_bounds.get().unwrap().left(), left - px(120.));
    assert_eq!(margin_bounds.get().unwrap().left(), editor_bounds.left());

    // The margin is a gutter view zone target.
    let at = margin.center();
    let target = editor.update_in(cx, |e, window, cx| e.mouse_target(at, window, cx));
    assert_eq!(target.kind, MouseTargetKind::GutterViewZone);
    assert!(matches!(
        target.detail,
        MouseTargetDetail::ViewZone { id: Some(zone), after_line: Some(1), .. } if zone == id
    ));
    assert_eq!(
        widget_bounds(&editor, EmbeddedKind::ViewZoneMargin, cx).len(),
        1
    );
}

#[gpui::test]
fn zones_after_one_line_follow_their_ordinals(cx: &mut TestAppContext) {
    let (editor, cx) = setup("a\nb\nc\nd", cx);
    let (first, first_bounds) = Probe::new(cx);
    let (second, second_bounds) = Probe::new(cx);
    let (third, third_bounds) = Probe::new(cx);
    editor.update(cx, |e, cx| {
        // Added first, but with a higher ordinal.
        let mut zone = ViewZone::new(2, ZoneHeight::Pixels(px(10.)), first.into());
        zone.ordinal = Some(5);
        e.add_view_zone(zone, cx);
        let mut zone = ViewZone::new(2, ZoneHeight::Pixels(px(20.)), second.into());
        zone.ordinal = Some(1);
        e.add_view_zone(zone, cx);
        // No ordinal: Monaco's 10000, after both.
        e.add_view_zone(
            ViewZone::new(2, ZoneHeight::Pixels(px(30.)), third.into()),
            cx,
        );
    });
    draw(cx);
    let lh = line_height(&editor, cx);
    let top = line_top(&editor, cx, 1) + lh;
    assert_eq!(second_bounds.get().unwrap().top(), top);
    assert_eq!(first_bounds.get().unwrap().top(), top + px(20.));
    assert_eq!(third_bounds.get().unwrap().top(), top + px(30.));
    assert_eq!(line_top(&editor, cx, 2), top + px(60.));
}

#[gpui::test]
fn after_column_places_a_zone_inside_a_wrapped_line(cx: &mut TestAppContext) {
    let words = (0..60)
        .map(|i| format!("w{i:02}"))
        .collect::<Vec<_>>()
        .join(" ");
    let (editor, cx) = setup(&format!("first\n{words}\nlast"), cx);
    editor.update(cx, |e, cx| {
        let mut options = e.options().clone();
        options.word_wrap = gpui_monaco::options::WordWrap::WordWrapColumn;
        options.word_wrap_column = 40;
        e.update_options(options, cx);
    });
    draw(cx);
    let lh = line_height(&editor, cx);
    let (at_column, column_bounds) = Probe::new(cx);
    let (at_end, end_bounds) = Probe::new(cx);
    editor.update(cx, |e, cx| {
        let mut zone = ViewZone::new(2, ZoneHeight::Pixels(px(10.)), at_column.into());
        // In the first view line of the wrapped row.
        zone.after_column = Some(5);
        e.add_view_zone(zone, cx);
        e.add_view_zone(
            ViewZone::new(2, ZoneHeight::Pixels(px(10.)), at_end.into()),
            cx,
        );
    });
    draw(cx);
    let row_top = line_top(&editor, cx, 1);
    assert_eq!(column_bounds.get().unwrap().top(), row_top + lh);
    // Without a column: after the last view line, before line 3.
    let end = end_bounds.get().unwrap();
    assert_eq!(end.bottom(), line_top(&editor, cx, 2));
    assert!(end.top() > row_top + lh * 2.);
    // At a wrap boundary, the left affinity means the previous view line.
    let boundary = editor.read_with(cx, |e, _| {
        // The first view line's last column.
        let first = e.scrolled_visible_position(p(1, 0)).unwrap().y;
        (0..words.len() as u32)
            .find(|&column| e.scrolled_visible_position(p(1, column)).unwrap().y > first)
            .unwrap()
    });
    let (left, left_bounds) = Probe::new(cx);
    editor.update(cx, |e, cx| {
        let mut zone = ViewZone::new(2, ZoneHeight::Pixels(px(10.)), left.into());
        zone.after_column = Some(boundary);
        zone.after_column_affinity = PositionAffinity::Left;
        e.add_view_zone(zone, cx);
    });
    draw(cx);
    // After the first view line too, ordered after the column 5 zone by
    // its ordinal (Monaco's default: the column).
    let row_top = line_top(&editor, cx, 1);
    assert_eq!(left_bounds.get().unwrap().top(), row_top + lh + px(10.));
}

#[gpui::test]
fn zones_hide_when_the_next_line_is_hidden_unless_asked(cx: &mut TestAppContext) {
    let (editor, cx) = setup("0\n1\n2\n3\n4\n5\n6", cx);
    let (hidden, _) = Probe::new(cx);
    let (shown, shown_bounds) = Probe::new(cx);
    let (hidden_id, shown_id) = editor.update(cx, |e, cx| {
        // After line 3 (row 2): row 3 starts the hidden area.
        let hidden_id = e.add_view_zone(
            ViewZone::new(3, ZoneHeight::Pixels(px(10.)), hidden.into()),
            cx,
        );
        let mut zone = ViewZone::new(3, ZoneHeight::Pixels(px(10.)), shown.into());
        zone.show_in_hidden_areas = true;
        let shown_id = e.add_view_zone(zone, cx);
        e.set_hidden_areas(std::iter::once(3..5).collect(), cx);
        (hidden_id, shown_id)
    });
    draw(cx);
    // Only the zone with `showInHiddenAreas` takes room.
    assert_eq!(widget_bounds(&editor, EmbeddedKind::ViewZone, cx).len(), 1);
    let target = editor.update_in(cx, |e, window, cx| {
        let at = shown_bounds.get().unwrap().center();
        e.mouse_target(at, window, cx)
    });
    assert!(matches!(
        target.detail,
        MouseTargetDetail::ViewZone { id: Some(id), .. } if id == shown_id && id != hidden_id
    ));
    let lh = line_height(&editor, cx);
    assert_eq!(
        shown_bounds.get().unwrap().top(),
        line_top(&editor, cx, 2) + lh
    );
    assert_eq!(
        line_top(&editor, cx, 5),
        line_top(&editor, cx, 2) + lh + px(10.)
    );
}

#[gpui::test]
fn suppress_mouse_down_puts_the_cursor_by_the_zone(cx: &mut TestAppContext) {
    let (editor, cx) = setup("alpha\nbeta\ngamma\ndelta", cx);
    let (plain, plain_bounds) = Probe::new(cx);
    let (suppressing, suppressing_bounds) = Probe::new(cx);
    let plain_clicks = cx.update(|_, cx| plain.read(cx).clicks.clone());
    editor.update(cx, |e, cx| {
        e.add_view_zone(
            ViewZone::new(1, ZoneHeight::Pixels(px(40.)), plain.into()),
            cx,
        );
        let mut zone = ViewZone::new(3, ZoneHeight::Pixels(px(40.)), suppressing.into());
        zone.suppress_mouse_down = true;
        e.add_view_zone(zone, cx);
    });
    draw(cx);
    let head = |cx: &mut VisualTestContext| editor.read_with(cx, |e, _| e.selections()[0].head);
    // A plain zone handles its own clicks; the cursor stays.
    cx.simulate_click(plain_bounds.get().unwrap().center(), Modifiers::default());
    draw(cx);
    assert_eq!(plain_clicks.get(), 1);
    assert_eq!(head(cx), p(0, 0));
    // With `suppressMouseDown`, the editor places the cursor: the upper
    // half is the end of the line above, the lower half the next line.
    let zone = suppressing_bounds.get().unwrap();
    cx.simulate_click(
        point(zone.center().x, zone.top() + px(5.)),
        Modifiers::default(),
    );
    draw(cx);
    assert_eq!(head(cx), p(2, 5));
    cx.simulate_click(
        point(zone.center().x, zone.bottom() - px(5.)),
        Modifiers::default(),
    );
    draw(cx);
    assert_eq!(head(cx), p(3, 0));
    assert!(cx.update(|window, cx| editor.focus_handle(cx).is_focused(window)));
}

#[gpui::test]
fn content_widgets_secondary_position_hiding_and_preference(cx: &mut TestAppContext) {
    let (editor, cx) = setup("let value = compute(alpha, beta);\nnext line\nthird", cx);
    let (widget, bounds) = Probe::sized(cx, 60., 12.);
    let id = editor.update(cx, |e, cx| {
        let mut widget = ContentWidget::new(
            p(0, 20),
            vec![ContentWidgetPreference::Exact],
            widget.into(),
        );
        widget.secondary_position = Some(p(0, 4));
        e.add_content_widget(widget, cx)
    });
    draw(cx);
    let x_of = |column, cx: &mut VisualTestContext| {
        editor
            .read_with(cx, |e, _| e.scrolled_visible_position(p(0, column)))
            .unwrap()
            .x
    };
    let primary = x_of(20, cx);
    let secondary = x_of(4, cx);
    let em = (x_of(1, cx) - x_of(0, cx)) * 2.;
    // Monaco `_reduceAnchorCoordinates`: toward the secondary position, but
    // still over the primary one.
    let left = bounds.get().unwrap().left();
    assert_near(left, secondary.max(primary - px(60.) + em));
    assert!(left < primary);

    // `layoutContentWidget` with no position hides it; a new preference
    // moves it below its line.
    editor.update(cx, |e, cx| {
        e.layout_content_widget(id, ContentWidgetPosition::default(), cx)
    });
    draw(cx);
    assert!(widget_bounds(&editor, EmbeddedKind::ContentWidget, cx).is_empty());
    editor.update(cx, |e, cx| {
        e.layout_content_widget(
            id,
            ContentWidgetPosition {
                position: Some(p(1, 2)),
                preference: vec![ContentWidgetPreference::Below],
                ..Default::default()
            },
            cx,
        )
    });
    draw(cx);
    let lh = line_height(&editor, cx);
    let shown = widget_bounds(&editor, EmbeddedKind::ContentWidget, cx);
    assert_eq!(shown.len(), 1);
    assert_eq!(shown[0].top(), line_top(&editor, cx, 1) + lh);
}

#[gpui::test]
fn content_widgets_are_capped_to_the_text_width(cx: &mut TestAppContext) {
    let (editor, cx) = setup("a\nb", cx);
    let (widget, _) = Probe::new(cx);
    cx.update(|_, cx| {
        widget.update(cx, |w, _| {
            w.text = Some("wide ".repeat(400));
            w.height = Some(px(16.));
        })
    });
    editor.update(cx, |e, cx| {
        e.add_content_widget(
            ContentWidget::new(p(0, 0), vec![ContentWidgetPreference::Below], widget.into()),
            cx,
        )
    });
    draw(cx);
    let info = editor.read_with(cx, |e, _| e.layout_info().unwrap());
    let shown = widget_bounds(&editor, EmbeddedKind::ContentWidget, cx);
    assert_eq!(shown.len(), 1);
    // Monaco's `maxWidth`: the layout's content width.
    assert!(shown[0].size.width <= info.content_width);
    assert!(shown[0].size.width > info.content_width / 2.);
}

#[gpui::test]
fn suppress_mouse_down_on_a_content_widget_keeps_the_editor_focused(cx: &mut TestAppContext) {
    let (editor, cx) = setup("alpha\nbeta", cx);
    let (widget, bounds) = Probe::sized(cx, 40., 14.);
    struct Focusing(gpui::FocusHandle);
    impl Render for Focusing {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            div().w(px(40.)).h(px(10.)).track_focus(&self.0)
        }
    }
    let focusing = cx.update(|_, cx| cx.new(|cx| Focusing(cx.focus_handle())));
    let focus = cx.update(|_, cx| focusing.read(cx).0.clone());
    editor.update(cx, |e, cx| {
        let mut widget =
            ContentWidget::new(p(1, 0), vec![ContentWidgetPreference::Exact], widget.into());
        widget.suppress_mouse_down = true;
        e.add_content_widget(widget, cx);
        e.add_content_widget(
            ContentWidget::new(
                p(0, 0),
                vec![ContentWidgetPreference::Exact],
                focusing.into(),
            ),
            cx,
        );
    });
    draw(cx);
    // A widget without `suppressMouseDown` takes focus itself.
    let other = widget_bounds(&editor, EmbeddedKind::ContentWidget, cx)
        .into_iter()
        .find(|b| b.top() < bounds.get().unwrap().top())
        .unwrap();
    cx.simulate_click(other.center(), Modifiers::default());
    draw(cx);
    assert!(cx.update(|window, _| focus.is_focused(window)));
    // With it, the editor has focus.
    cx.simulate_click(bounds.get().unwrap().center(), Modifiers::default());
    draw(cx);
    assert!(cx.update(|window, cx| editor.focus_handle(cx).is_focused(window)));
}

#[gpui::test]
fn overlay_widgets_use_monaco_corners_and_stack(cx: &mut TestAppContext) {
    let (editor, cx) = setup(&"line\n".repeat(80), cx);
    let (a, a_bounds) = Probe::sized(cx, 50., 20.);
    let (b, b_bounds) = Probe::sized(cx, 70., 30.);
    let (c, c_bounds) = Probe::sized(cx, 40., 10.);
    let c_id = editor.update(cx, |e, cx| {
        let mut first = OverlayWidget::new(OverlayPosition::TopRight, a.into());
        first.stack_ordinal = Some(2);
        let mut second = OverlayWidget::new(OverlayPosition::TopRight, b.into());
        second.stack_ordinal = Some(1);
        e.add_overlay_widget(first, cx);
        e.add_overlay_widget(second, cx);
        e.add_overlay_widget(
            OverlayWidget::new(OverlayPosition::BottomRight, c.into()),
            cx,
        )
    });
    draw(cx);
    let info = editor.read_with(cx, |e, _| e.layout_info().unwrap());
    let (editor_bounds, _) = editor.read_with(cx, |e, _| e.widget_bounds_for_tests());
    // Monaco `_renderWidget`: `right = 2 * verticalScrollbarWidth +
    // minimapWidth`.
    let right = editor_bounds.right() - info.vertical_scrollbar_width * 2.;
    // Stacked by ordinal: the second (ordinal 1) on top.
    let b = b_bounds.get().unwrap();
    let a = a_bounds.get().unwrap();
    assert_eq!((b.right(), b.top()), (right, editor_bounds.top()));
    assert_eq!((a.right(), a.top()), (right, editor_bounds.top() + px(30.)));
    // Bottom right keeps twice the horizontal scrollbar's height.
    let c = c_bounds.get().unwrap();
    assert_eq!(c.right(), right);
    assert_eq!(
        c.bottom(),
        editor_bounds.bottom() - info.horizontal_scrollbar_height * 2.
    );
    // `layoutOverlayWidget` moves one.
    editor.update(cx, |e, cx| {
        e.layout_overlay_widget(
            c_id,
            OverlayPosition::Custom(point(px(5.), px(6.))),
            None,
            cx,
        )
    });
    draw(cx);
    assert_eq!(
        c_bounds.get().unwrap().origin,
        editor_bounds.origin + point(px(5.), px(6.))
    );
}

#[gpui::test]
fn overlay_min_content_width_widens_the_scroll_range(cx: &mut TestAppContext) {
    let (editor, cx) = setup("short\nlines", cx);
    let before = editor.read_with(cx, |e, _| e.scroll_width().unwrap());
    let (widget, _) = Probe::sized(cx, 10., 10.);
    editor.update(cx, |e, cx| {
        let mut widget = OverlayWidget::new(OverlayPosition::TopRight, widget.into());
        widget.min_content_width = px(5000.);
        e.add_overlay_widget(widget, cx);
    });
    draw(cx);
    let after = editor.read_with(cx, |e, _| e.scroll_width().unwrap());
    assert!(before < px(5000.));
    assert!(after >= px(5000.));
}

#[gpui::test]
fn manual_overlay_widgets_get_the_editor(cx: &mut TestAppContext) {
    let (editor, cx) = setup("a", cx);
    let (widget, bounds) = Probe::new(cx);
    editor.update(cx, |e, cx| {
        e.add_overlay_widget(
            OverlayWidget::new(OverlayPosition::Manual, widget.into()),
            cx,
        )
    });
    draw(cx);
    let (editor_bounds, _) = editor.read_with(cx, |e, _| e.widget_bounds_for_tests());
    let painted = bounds.get().unwrap();
    assert_eq!(painted.origin, editor_bounds.origin);
    assert_eq!(
        painted.size,
        size(editor_bounds.size.width, editor_bounds.size.height)
    );
}
