//! Diagnostic tags, related information and `render_validation_decorations`
//! (Monaco's `markerDecorationsService` and marker hover).

use gpui::{AppContext as _, Entity, Focusable, TestAppContext, VisualTestContext};
use gpui_monaco::options::RenderValidationDecorations;
use gpui_monaco::text::PointUtf16;
use gpui_monaco::{
    CodeEditor, EditorOptions, Marker, MarkerSeverity, MarkerTag, RelatedInformation, TextModel,
};

fn setup<'a>(
    text: &str,
    options: impl FnOnce(&mut EditorOptions),
    cx: &'a mut TestAppContext,
) -> (Entity<CodeEditor>, &'a mut VisualTestContext) {
    cx.update(gpui_monaco::init);
    let text = text.to_string();
    let (editor, cx) = cx.add_window_view(|window, cx| {
        let model = cx.new(|_| TextModel::new(&text).with_uri("file:///main.rs"));
        CodeEditor::new(model, window, cx)
    });
    editor.update(cx, |editor, cx| {
        let mut opts = editor.options().clone();
        options(&mut opts);
        editor.update_options(opts, cx);
    });
    cx.update(|window, cx| window.focus(&editor.focus_handle(cx), cx));
    cx.run_until_parked();
    (editor, cx)
}

fn marker(severity: MarkerSeverity, range: std::ops::Range<PointUtf16>) -> Marker {
    Marker {
        severity,
        message: "problem".into(),
        range,
        ..Default::default()
    }
}

fn set_markers(editor: &Entity<CodeEditor>, cx: &mut VisualTestContext, markers: Vec<Marker>) {
    editor.update(cx, |e, cx| {
        e.model()
            .update(cx, |m, cx| m.set_markers("test", markers, cx))
    });
}

#[gpui::test]
fn tags_mark_decorations(cx: &mut TestAppContext) {
    let (editor, cx) = setup("let unused = old();\n", |_| {}, cx);
    set_markers(
        &editor,
        cx,
        vec![
            Marker {
                tags: vec![MarkerTag::Unnecessary],
                ..marker(
                    MarkerSeverity::Hint,
                    PointUtf16::new(0, 4)..PointUtf16::new(0, 10),
                )
            },
            Marker {
                tags: vec![MarkerTag::Deprecated, MarkerTag::Unnecessary],
                ..marker(
                    MarkerSeverity::Warning,
                    PointUtf16::new(0, 13)..PointUtf16::new(0, 16),
                )
            },
        ],
    );
    let decorations = editor.update(cx, |e, cx| {
        e.model().update(cx, |m, _| m.all_decorations(None))
    });
    let mut flags: Vec<_> = decorations
        .iter()
        .map(|d| {
            (
                d.range.clone(),
                d.options.validation,
                d.options.unnecessary,
                d.options.deprecated,
                d.options.underline.is_some(),
            )
        })
        .collect();
    flags.sort_by_key(|f| f.0.start);
    assert_eq!(
        flags,
        vec![
            // An unnecessary hint: faded, no squiggle.
            (4..10, true, true, false, false),
            // Deprecated wins over unnecessary; warnings keep their squiggle.
            (13..16, true, false, true, true),
        ]
    );
}

#[gpui::test]
fn untagged_hints_cover_two_columns(cx: &mut TestAppContext) {
    let (editor, cx) = setup("let value = 1;\n", |_| {}, cx);
    set_markers(
        &editor,
        cx,
        vec![marker(
            MarkerSeverity::Hint,
            PointUtf16::new(0, 4)..PointUtf16::new(1, 0),
        )],
    );
    let markers = editor.update(cx, |e, cx| e.model().read(cx).markers());
    assert_eq!(
        markers[0].range,
        PointUtf16::new(0, 4)..PointUtf16::new(0, 6)
    );
}

fn hover_markers(editor: &Entity<CodeEditor>, cx: &mut VisualTestContext) -> Vec<Marker> {
    editor.update(cx, |e, cx| {
        e.set_selections(
            &[gpui_monaco::cursor::Selection::collapsed(PointUtf16::new(
                0, 5,
            ))],
            cx,
        );
        e.show_hover_at_cursor(cx);
    });
    cx.run_until_parked();
    editor.read_with(cx, |e, cx| e.hover_markers_for_tests(cx))
}

#[gpui::test]
fn render_validation_decorations(cx: &mut TestAppContext) {
    let (editor, cx) = setup("let value = 1;\n", |_| {}, cx);
    set_markers(
        &editor,
        cx,
        vec![marker(
            MarkerSeverity::Error,
            PointUtf16::new(0, 4)..PointUtf16::new(0, 9),
        )],
    );
    // Editable (default): shown while editable, hidden when read-only.
    assert_eq!(hover_markers(&editor, cx).len(), 1);
    let set = |cx: &mut VisualTestContext, f: &dyn Fn(&mut EditorOptions)| {
        editor.update(cx, |e, cx| {
            e.hide_hover(cx);
            let mut options = e.options().clone();
            f(&mut options);
            e.update_options(options, cx);
        })
    };
    set(cx, &|o| o.read_only = true);
    assert!(hover_markers(&editor, cx).is_empty());
    assert!(!editor.read_with(cx, |e, _| e.shows_validation_decorations()));
    set(cx, &|o| {
        o.render_validation_decorations = RenderValidationDecorations::On
    });
    assert_eq!(hover_markers(&editor, cx).len(), 1);
    set(cx, &|o| {
        o.read_only = false;
        o.render_validation_decorations = RenderValidationDecorations::Off;
    });
    assert!(hover_markers(&editor, cx).is_empty());
}

#[gpui::test]
fn related_information_and_view_problem(cx: &mut TestAppContext) {
    let (editor, cx) = setup("fn a() {}\nfn b() { a(1) }\n", |_| {}, cx);
    let related = RelatedInformation {
        uri: None,
        range: PointUtf16::new(0, 3)..PointUtf16::new(0, 4),
        message: "defined here".into(),
    };
    set_markers(
        &editor,
        cx,
        vec![Marker {
            related_information: vec![related.clone()],
            ..marker(
                MarkerSeverity::Error,
                PointUtf16::new(1, 9)..PointUtf16::new(1, 13),
            )
        }],
    );
    editor.update(cx, |e, cx| {
        e.set_selections(
            &[gpui_monaco::cursor::Selection::collapsed(PointUtf16::new(
                1, 10,
            ))],
            cx,
        );
        e.show_hover_at_cursor(cx);
    });
    cx.run_until_parked();
    let markers = editor.read_with(cx, |e, cx| e.hover_markers_for_tests(cx));
    assert_eq!(markers[0].related_information, vec![related]);
    // "View Problem" opens the marker zone at it.
    editor.update(cx, |e, cx| {
        e.set_selections(
            &[gpui_monaco::cursor::Selection::collapsed(PointUtf16::new(
                0, 0,
            ))],
            cx,
        );
        e.show_marker(&markers[0], cx)
    });
    assert_eq!(
        editor.read_with(cx, |e, _| e.selections()[0].head),
        PointUtf16::new(1, 9)
    );
    assert!(editor.update(cx, |e, cx| e.close_marker_zone(cx)));
}

fn draw(cx: &mut VisualTestContext) {
    cx.update(|window, _| window.refresh());
    cx.run_until_parked();
}

/// Monaco `MarkerNavigationWidget`: 3 lines plus the body (at most 17), and
/// the body scrolls on its own, without scrolling the editor.
#[gpui::test]
fn the_marker_zone_fits_its_content_and_scrolls(cx: &mut TestAppContext) {
    let text: String = (0..80).map(|i| format!("line {i}\n")).collect();
    let (editor, cx) = setup(&text, |_| {}, cx);
    let related = |row: u32| RelatedInformation {
        uri: None,
        range: PointUtf16::new(row, 0)..PointUtf16::new(row, 4),
        message: "expected because of this".into(),
    };
    let short = Marker {
        message: "mismatched types".into(),
        source: Some("demo".into()),
        code: Some("E0308".into()),
        related_information: vec![related(0)],
        ..marker(
            MarkerSeverity::Error,
            PointUtf16::new(2, 0)..PointUtf16::new(2, 4),
        )
    };
    let long_message: Vec<String> = (0..30).map(|i| format!("note {i}")).collect();
    let long = Marker {
        message: long_message.join("\n").into(),
        ..marker(
            MarkerSeverity::Warning,
            PointUtf16::new(40, 0)..PointUtf16::new(40, 4),
        )
    };
    set_markers(&editor, cx, vec![short.clone(), long.clone()]);
    draw(cx);
    let zone = |cx: &mut VisualTestContext| {
        editor
            .read_with(cx, |e, cx| e.marker_zone_for_tests(cx))
            .expect("marker zone")
    };
    // One message line, a spacer and one related line: 3 + 3.
    editor.update(cx, |e, cx| e.show_marker(&short, cx));
    draw(cx);
    let (_, lines, _) = zone(cx);
    assert!((lines - 6.).abs() < 0.01, "{lines}");
    // Thirty message lines: 3 + 17, and the rest scrolls.
    editor.update(cx, |e, cx| e.show_marker(&long, cx));
    draw(cx);
    let (bounds, lines, offset) = zone(cx);
    assert!((lines - 20.).abs() < 0.01, "{lines}");
    assert_eq!(offset.y, gpui::px(0.));
    let scroll_top = editor.read_with(cx, |e, _| e.scroll_top());
    cx.simulate_event(gpui::ScrollWheelEvent {
        position: bounds.center(),
        delta: gpui::ScrollDelta::Pixels(gpui::point(gpui::px(0.), gpui::px(-60.))),
        ..Default::default()
    });
    draw(cx);
    let (_, _, offset) = zone(cx);
    assert!(offset.y < gpui::px(0.), "the body scrolled: {offset:?}");
    assert_eq!(editor.read_with(cx, |e, _| e.scroll_top()), scroll_top);
    // Clicks inside the zone don't move the cursor.
    let head = editor.read_with(cx, |e, _| e.selections()[0].head);
    cx.simulate_click(bounds.center(), gpui::Modifiers::default());
    draw(cx);
    assert_eq!(editor.read_with(cx, |e, _| e.selections()[0].head), head);
    assert!(editor.update(cx, |e, cx| e.close_marker_zone(cx)));
}
