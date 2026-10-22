//! Diagnostic tags, related information, marker limits and
//! `render_validation_decorations` (Monaco's `markerDecorationsService`,
//! ported from `markerDecorationsService.test.ts`, and marker hover).

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use gpui::{AppContext as _, Entity, Focusable, SharedString, TestAppContext, VisualTestContext};
use gpui_monaco::options::RenderValidationDecorations;
use gpui_monaco::text::PointUtf16;
use gpui_monaco::{
    CodeEditor, DecorationId, EditorOptions, Marker, MarkerChangeOptions, MarkerFilter,
    MarkerLimit, MarkerSeverity, MarkerTag, RelatedInformation, TextModel, TextModelEvent,
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

// ----- markerDecorationsService.test.ts --------------------------------------

/// The decorated ranges of `markers` set on a model holding `text`.
fn decorated(
    text: &str,
    markers: Vec<Marker>,
    cx: &mut TestAppContext,
) -> Vec<std::ops::Range<PointUtf16>> {
    let (editor, cx) = setup(text, |_| {}, cx);
    set_markers(&editor, cx, markers);
    editor.update(cx, |e, cx| {
        e.model()
            .read(cx)
            .markers()
            .into_iter()
            .map(|m| m.range)
            .collect()
    })
}

fn at(row: u32, column: u32) -> PointUtf16 {
    PointUtf16::new(row, column)
}

#[gpui::test]
fn an_empty_marker_in_a_word_covers_the_word(cx: &mut TestAppContext) {
    let ranges = decorated(
        "  foobar baz\n",
        vec![marker(MarkerSeverity::Error, at(0, 5)..at(0, 5))],
        cx,
    );
    assert_eq!(ranges, vec![at(0, 2)..at(0, 8)]);
}

#[gpui::test]
fn an_empty_marker_at_the_end_of_a_line_stays_empty(cx: &mut TestAppContext) {
    // Behind the last non-whitespace character, and on an empty line:
    // drawn one character wide rather than over the line break.
    let ranges = decorated(
        "foobar  \n\nnext\n",
        vec![
            marker(MarkerSeverity::Error, at(0, 6)..at(0, 6)),
            marker(MarkerSeverity::Error, at(0, 7)..at(0, 7)),
            marker(MarkerSeverity::Warning, at(1, 0)..at(1, 0)),
        ],
        cx,
    );
    assert_eq!(
        ranges,
        vec![at(0, 6)..at(0, 6), at(0, 7)..at(0, 7), at(1, 0)..at(1, 0)]
    );
}

#[gpui::test]
fn an_empty_marker_between_words_stays_empty(cx: &mut TestAppContext) {
    let ranges = decorated(
        "foo  = bar\n",
        vec![marker(MarkerSeverity::Error, at(0, 4)..at(0, 4))],
        cx,
    );
    assert_eq!(ranges, vec![at(0, 4)..at(0, 4)]);
}

#[gpui::test]
fn a_whole_line_marker_starts_at_the_first_non_whitespace(cx: &mut TestAppContext) {
    let ranges = decorated(
        "fn main() {\n    let x = 1;\n}\n",
        vec![
            marker(MarkerSeverity::Error, at(1, 0)..at(1, u32::MAX)),
            // Not from column 0: kept.
            marker(MarkerSeverity::Warning, at(1, 1)..at(1, u32::MAX)),
            // Not to `u32::MAX`: kept.
            marker(MarkerSeverity::Info, at(1, 0)..at(1, 14)),
        ],
        cx,
    );
    assert_eq!(
        ranges,
        vec![
            at(1, 0)..at(1, 14),
            at(1, 1)..at(1, 14),
            at(1, 4)..at(1, 14)
        ]
    );
}

#[gpui::test]
fn only_the_first_5000_markers_are_decorated(cx: &mut TestAppContext) {
    let (editor, cx) = setup(&"x\n".repeat(5100), |_| {}, cx);
    let markers = (0..5100)
        .map(|row| marker(MarkerSeverity::Error, at(row, 0)..at(row, 1)))
        .collect();
    set_markers(&editor, cx, markers);
    let (live, all) = editor.update(cx, |e, cx| {
        let live = e.model().read(cx).markers().len();
        (
            live,
            gpui_monaco::model_markers(cx, &Default::default()).len(),
        )
    });
    assert_eq!(gpui_monaco::MAX_DECORATED_MARKERS, 5000);
    assert_eq!(live, gpui_monaco::MAX_DECORATED_MARKERS);
    assert_eq!(all, 5100);
    // Another owner's markers come after: none of them is decorated, until
    // the first owner's go.
    editor.update(cx, |e, cx| {
        e.model().update(cx, |m, cx| {
            m.set_markers(
                "other",
                vec![marker(MarkerSeverity::Warning, at(0, 0)..at(0, 1))],
                cx,
            )
        })
    });
    let warnings = |cx: &mut VisualTestContext| {
        editor.update(cx, |e, cx| {
            e.model()
                .read(cx)
                .markers()
                .iter()
                .filter(|m| m.severity == MarkerSeverity::Warning)
                .count()
        })
    };
    assert_eq!(warnings(cx), 0);
    set_markers(&editor, cx, Vec::new());
    assert_eq!(warnings(cx), 1);
    let live = editor.update(cx, |e, cx| e.model().read(cx).markers().len());
    assert_eq!(live, 1);
}

/// Monaco `MarkerDecorations.update`: an owner's markers keep their
/// decorations while another owner's change.
#[gpui::test]
fn unchanged_markers_keep_their_decorations(cx: &mut TestAppContext) {
    let (editor, cx) = setup("let value = 1;\n", |_| {}, cx);
    set_markers(
        &editor,
        cx,
        vec![marker(MarkerSeverity::Error, at(0, 4)..at(0, 9))],
    );
    let ids = |cx: &mut VisualTestContext| {
        let mut ids: Vec<_> = editor.update(cx, |e, cx| {
            e.model().update(cx, |m, _| {
                m.all_decorations(None)
                    .iter()
                    .filter(|d| d.options.validation)
                    .map(|d| d.id)
                    .collect()
            })
        });
        ids.sort();
        ids
    };
    let first = ids(cx);
    editor.update(cx, |e, cx| {
        e.model().update(cx, |m, cx| {
            m.set_markers(
                "other",
                vec![marker(MarkerSeverity::Warning, at(0, 12)..at(0, 13))],
                cx,
            )
        })
    });
    let both = ids(cx);
    assert_eq!(both.len(), 2);
    assert!(both.contains(&first[0]));
}

// ----- marker limits (markerDecorationsService.test.ts) -----------------------
//
// Not ported: "honors resource filters" and "preserves suppression after
// applying the decoration limit", as there's no `installResourceFilter` or
// `addMarkerSuppression` here.

const DIAGNOSTICS_URI: &str = "file:///diagnostics.txt";

/// Monaco's `createModel`: 5100 lines of `value`.
fn limit_model(uri: &str, cx: &mut TestAppContext) -> Entity<TextModel> {
    let text = vec!["value"; 5100].join("\n");
    let uri = uri.to_string();
    cx.new(|_| TextModel::new(&text).with_uri(uri))
}

/// Monaco's `createMarkers`: one marker per line, cycling through the
/// severities.
fn limit_markers(count: u32) -> Vec<Marker> {
    let severities = [
        MarkerSeverity::Error,
        MarkerSeverity::Warning,
        MarkerSeverity::Info,
        MarkerSeverity::Hint,
    ];
    (0..count)
        .map(|row| Marker {
            message: format!("problem {}", row + 1).into(),
            ..marker(severities[row as usize % 4], at(row, 0)..at(row, 1))
        })
        .collect()
}

/// Monaco's `changeMarkers`: `IMarkerService.changeOne`, then the events.
fn change_markers(uri: &str, count: u32, owner: &str, is_truncated: bool, cx: &mut TestAppContext) {
    cx.update(|cx| {
        gpui_monaco::set_markers_for_uri_with_options(
            cx,
            owner.to_string(),
            uri,
            limit_markers(count),
            MarkerChangeOptions { is_truncated },
        )
    });
    cx.run_until_parked();
}

fn limited(uri: &str, cx: &mut TestAppContext) -> Option<usize> {
    cx.read(|cx| gpui_monaco::marker_limits(cx, uri))
        .and_then(|limit| limit.is_markers_truncated)
}

fn live_markers(model: &Entity<TextModel>, cx: &mut TestAppContext) -> usize {
    model.read_with(cx, |m, _| m.markers().len())
}

fn decoration_ids(model: &Entity<TextModel>, cx: &mut TestAppContext) -> Vec<DecorationId> {
    let mut ids: Vec<_> = model.update(cx, |m, _| {
        m.all_decorations(None).iter().map(|d| d.id).collect()
    });
    ids.sort();
    ids
}

/// Counts the model's [`TextModelEvent::MarkersChanged`] (Monaco
/// `onDidChangeMarker`).
fn count_marker_changes(
    model: &Entity<TextModel>,
    cx: &mut TestAppContext,
) -> (Rc<Cell<usize>>, gpui::Subscription) {
    let count = Rc::new(Cell::new(0));
    let subscription = cx.update(|cx| {
        let count = count.clone();
        cx.subscribe(model, move |_, event: &TextModelEvent, _| {
            if matches!(event, TextModelEvent::MarkersChanged) {
                count.set(count.get() + 1);
            }
        })
    });
    (count, subscription)
}

/// What [`log_limit_changes`] saw: each model's resource and limit.
type LimitLog = Rc<RefCell<Vec<(SharedString, MarkerLimit)>>>;

/// Logs [`gpui_monaco::on_did_change_marker_limit`] events: each model's
/// resource and limit when it arrives.
fn log_limit_changes(cx: &mut TestAppContext) -> (LimitLog, gpui::Subscription) {
    let log = LimitLog::default();
    let subscription = cx.update(|cx| {
        let log = log.clone();
        gpui_monaco::on_did_change_marker_limit(cx, move |model, cx| {
            let resource = model.read(cx).resource().clone();
            let limit = gpui_monaco::marker_limits(cx, &resource).unwrap();
            assert_eq!(limit, model.read(cx).marker_limit());
            log.borrow_mut().push((resource, limit));
        })
    });
    (log, subscription)
}

#[gpui::test]
fn the_limit_is_reported_only_above_5000_and_every_marker_is_kept(cx: &mut TestAppContext) {
    cx.update(gpui_monaco::init);
    let model = limit_model(DIAGNOSTICS_URI, cx);
    let mut states = Vec::new();
    for count in [4999, 5000, 5001, 5100, 5000, 0] {
        change_markers(DIAGNOSTICS_URI, count, "test", false, cx);
        let diagnostics = cx.read(|cx| {
            gpui_monaco::model_markers(
                cx,
                &MarkerFilter {
                    resource: Some(DIAGNOSTICS_URI.into()),
                    ..Default::default()
                },
            )
            .len()
        });
        states.push((
            diagnostics,
            live_markers(&model, cx),
            limited(DIAGNOSTICS_URI, cx),
        ));
    }
    assert_eq!(
        states,
        vec![
            (4999, 4999, None),
            (5000, 5000, None),
            (5001, 5000, Some(5000)),
            (5100, 5000, Some(5000)),
            (5000, 5000, None),
            (0, 0, None),
        ]
    );
}

#[gpui::test]
fn only_a_limit_change_notifies_without_replacing_decorations(cx: &mut TestAppContext) {
    cx.update(gpui_monaco::init);
    let model = limit_model(DIAGNOSTICS_URI, cx);
    change_markers(DIAGNOSTICS_URI, 5000, "test", false, cx);
    let ids = decoration_ids(&model, cx);
    let (marker_changes, _markers) = count_marker_changes(&model, cx);
    let (limit_changes, _limits) = log_limit_changes(cx);

    change_markers(DIAGNOSTICS_URI, 1, "overflow", false, cx);
    change_markers(DIAGNOSTICS_URI, 2, "overflow", false, cx);
    change_markers(DIAGNOSTICS_URI, 0, "overflow", false, cx);

    assert_eq!(marker_changes.get(), 0);
    assert_eq!(
        limit_changes
            .borrow()
            .iter()
            .map(|(resource, limit)| (resource.to_string(), limit.is_markers_truncated))
            .collect::<Vec<_>>(),
        vec![
            (DIAGNOSTICS_URI.to_string(), Some(5000)),
            (DIAGNOSTICS_URI.to_string(), None),
        ]
    );
    assert_eq!(decoration_ids(&model, cx), ids);
}

#[gpui::test]
fn source_truncation_is_reported_apart_from_the_decoration_limit(cx: &mut TestAppContext) {
    cx.update(gpui_monaco::init);
    let model = limit_model(DIAGNOSTICS_URI, cx);
    change_markers(DIAGNOSTICS_URI, 5001, "test", false, cx);
    let ids = decoration_ids(&model, cx);
    let (marker_changes, _markers) = count_marker_changes(&model, cx);
    let (limit_changes, _limits) = log_limit_changes(cx);

    change_markers(DIAGNOSTICS_URI, 1, "truncated", true, cx);
    let truncated_ids = decoration_ids(&model, cx);
    change_markers(DIAGNOSTICS_URI, 1, "truncated", false, cx);
    let restored_ids = decoration_ids(&model, cx);
    change_markers(DIAGNOSTICS_URI, 0, "test", false, cx);
    change_markers(DIAGNOSTICS_URI, 1, "truncated", true, cx);

    let limit = |source, truncated| MarkerLimit {
        is_markers_source_truncated: source,
        is_markers_truncated: truncated,
    };
    assert_eq!(
        limit_changes
            .borrow()
            .iter()
            .map(|(_, limit)| *limit)
            .collect::<Vec<_>>(),
        vec![
            limit(true, Some(5000)),
            limit(false, Some(5000)),
            limit(false, None),
            limit(true, None),
        ]
    );
    // Removing the 5001 markers, then replacing the truncated owner's one.
    assert_eq!(marker_changes.get(), 2);
    // Over the limit, the truncated owner's marker isn't decorated: the
    // decorations stay as they were.
    assert_eq!(truncated_ids, ids);
    assert_eq!(restored_ids, ids);
}

#[gpui::test]
fn limits_are_kept_apart_for_uris_with_the_same_path(cx: &mut TestAppContext) {
    cx.update(gpui_monaco::init);
    let remote = "vscode-remote://first/diagnostics.txt";
    let other_remote = "vscode-remote://second/diagnostics.txt";
    let _models = [DIAGNOSTICS_URI, remote, other_remote].map(|uri| limit_model(uri, cx));
    change_markers(DIAGNOSTICS_URI, 5001, "test", false, cx);
    change_markers(remote, 5000, "test", false, cx);
    change_markers(other_remote, 5100, "test", false, cx);
    assert_eq!(
        [DIAGNOSTICS_URI, remote, other_remote].map(|uri| limited(uri, cx)),
        [Some(5000), None, Some(5000)]
    );
}

#[gpui::test]
fn the_limit_goes_with_the_model_and_comes_back_with_the_next(cx: &mut TestAppContext) {
    cx.update(gpui_monaco::init);
    let model = limit_model(DIAGNOSTICS_URI, cx);
    change_markers(DIAGNOSTICS_URI, 5001, "test", false, cx);
    assert_eq!(limited(DIAGNOSTICS_URI, cx), Some(5000));
    drop(model);
    cx.update(|_| {});
    cx.run_until_parked();
    assert_eq!(
        cx.read(|cx| gpui_monaco::marker_limits(cx, DIAGNOSTICS_URI)),
        None
    );

    let reopened = limit_model(DIAGNOSTICS_URI, cx);
    assert_eq!(limited(DIAGNOSTICS_URI, cx), Some(5000));
    assert_eq!(live_markers(&reopened, cx), 5000);
}

/// Monaco's "initializes limits for existing models and diagnostics": a
/// new `MarkerDecorationsService` picks up the models; here a model made
/// before `init` is picked up when registered.
#[gpui::test]
fn a_model_registered_later_gets_its_limit(cx: &mut TestAppContext) {
    let model = limit_model(DIAGNOSTICS_URI, cx);
    change_markers(DIAGNOSTICS_URI, 5001, "test", false, cx);
    assert_eq!(
        cx.read(|cx| gpui_monaco::marker_limits(cx, DIAGNOSTICS_URI)),
        None
    );
    assert_eq!(live_markers(&model, cx), 0);

    cx.update(|cx| gpui_monaco::register_model(cx, &model));
    assert_eq!(limited(DIAGNOSTICS_URI, cx), Some(5000));
    assert_eq!(live_markers(&model, cx), 5000);
}

/// A model's own markers can be truncated at their source too, and the
/// limit event comes once per update, only when the limit changed by its
/// end.
#[gpui::test]
fn a_models_markers_can_be_truncated_at_their_source(cx: &mut TestAppContext) {
    cx.update(gpui_monaco::init);
    let model = cx.new(|_| TextModel::new("let value = 1;\n").with_uri("file:///main.rs"));
    let (limit_changes, _limits) = log_limit_changes(cx);
    let truncated = MarkerChangeOptions { is_truncated: true };
    let set = |owner: &'static str, options, cx: &mut TestAppContext| {
        model.update(cx, |m, cx| {
            m.set_markers_with_options(
                owner,
                vec![marker(MarkerSeverity::Error, at(0, 4)..at(0, 9))],
                options,
                cx,
            )
        })
    };
    set("lsp", truncated, cx);
    cx.run_until_parked();
    assert!(model.read_with(cx, |m, _| m.marker_limit().is_markers_source_truncated));
    assert!(cx.read(|cx| gpui_monaco::is_source_truncated(cx, "file:///main.rs")));
    assert_eq!(limit_changes.borrow().len(), 1);

    // Truncated again, by another owner: no change.
    set("other", truncated, cx);
    cx.run_until_parked();
    assert_eq!(limit_changes.borrow().len(), 1);

    // Cleared and truncated again in one update: no change by its end.
    cx.update(|cx| {
        gpui_monaco::remove_all_markers(cx, "lsp");
        gpui_monaco::remove_all_markers(cx, "other");
        model.update(cx, |m, cx| {
            m.set_markers_with_options(
                "lsp",
                vec![marker(MarkerSeverity::Error, at(0, 4)..at(0, 9))],
                truncated,
                cx,
            )
        });
    });
    cx.run_until_parked();
    assert_eq!(limit_changes.borrow().len(), 1);

    // Set again without it: the source is whole.
    set("lsp", MarkerChangeOptions::default(), cx);
    cx.run_until_parked();
    assert!(!cx.read(|cx| gpui_monaco::is_source_truncated(cx, "file:///main.rs")));
    assert_eq!(
        limit_changes
            .borrow()
            .iter()
            .map(|(_, limit)| limit.is_markers_source_truncated)
            .collect::<Vec<_>>(),
        vec![true, false]
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
