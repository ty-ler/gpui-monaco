//! Monaco's remaining decoration options and editor API (#272):
//! `showIfCollapsed`, `firstLineDecorationClassName`, injected text cursor
//! stops (in moves, clicks and mouse targets) and `attachedData`, theme
//! colors for the overview ruler, token change ranges, a model's
//! `onWillDispose`, contributions, `getFontSizeAtPosition`, the aria
//! options, `setModel(null)`, `registerCommand`'s `IDisposable` and
//! `createModel` with a taken uri.

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::Arc;

use gpui::{
    AppContext as _, Entity, Focusable as _, Modifiers, Pixels, TestAppContext, VisualTestContext,
    px,
};
use gpui_monaco::cursor::Selection;
use gpui_monaco::options::WordWrap;
use gpui_monaco::providers::Command;
use gpui_monaco::text::{Point, PointUtf16};
use gpui_monaco::{
    AriaOptions, AttachedData, CodeEditor, CodeEditorEvent, DecorationColor, DecorationId,
    DecorationOptions, InjectedText, InjectedTextCursorStops, LinesDecoration,
    OverviewRulerDecoration, OverviewRulerLane, TextModel, TextModelEvent, Theme,
};

fn p(row: u32, column: u32) -> PointUtf16 {
    PointUtf16::new(row, column)
}

fn setup<'a>(
    text: &str,
    cx: &'a mut TestAppContext,
) -> (Entity<CodeEditor>, &'a mut VisualTestContext) {
    cx.update(gpui_monaco::init);
    let text = text.to_string();
    let (editor, cx) = cx.add_window_view(|window, cx| {
        let model = cx.new(|_| TextModel::new(&text));
        CodeEditor::new(model, window, cx)
    });
    cx.update(|window, cx| window.focus(&editor.focus_handle(cx), cx));
    draw(cx);
    (editor, cx)
}

fn draw(cx: &mut VisualTestContext) {
    cx.update(|window, cx| {
        window.refresh();
        window.draw(cx).clear(cx);
    });
    cx.run_until_parked();
}

fn decorate(
    editor: &Entity<CodeEditor>,
    decorations: Vec<(std::ops::Range<usize>, DecorationOptions)>,
    cx: &mut VisualTestContext,
) -> Vec<DecorationId> {
    let ids = editor.update(cx, |e, cx| {
        e.model().update(cx, |model, cx| {
            model.delta_decorations(None, &[], decorations, cx)
        })
    });
    draw(cx);
    ids
}

/// "XYZ" injected before byte `at` (an empty range), with `stops`.
fn inject(
    editor: &Entity<CodeEditor>,
    at: usize,
    stops: InjectedTextCursorStops,
    cx: &mut VisualTestContext,
) {
    decorate(
        editor,
        vec![(
            at..at,
            DecorationOptions {
                before: Some(InjectedText {
                    content: "XYZ".into(),
                    cursor_stops: stops,
                    attached_data: Some(AttachedData::new("payload")),
                    ..Default::default()
                }),
                show_if_collapsed: true,
                ..Default::default()
            },
        )],
        cx,
    );
}

/// Where `point`'s character starts (after injected text before it).
fn x_of(editor: &Entity<CodeEditor>, point: PointUtf16, cx: &mut VisualTestContext) -> Pixels {
    editor.read_with(cx, |e, _| e.scrolled_visible_position(point).unwrap().x)
}

/// The primary cursor's left edge, drawn.
fn caret_x(editor: &Entity<CodeEditor>, cx: &mut VisualTestContext) -> Pixels {
    draw(cx);
    editor.read_with(cx, |e, _| e.cursor_shapes_for_tests().0[0].left())
}

fn select(editor: &Entity<CodeEditor>, at: PointUtf16, cx: &mut VisualTestContext) {
    editor.update(cx, |e, cx| {
        e.set_selections(&[Selection::collapsed(at)], cx)
    });
    draw(cx);
}

fn head(editor: &Entity<CodeEditor>, cx: &mut VisualTestContext) -> PointUtf16 {
    editor.read_with(cx, |e, _| e.selections()[0].head)
}

fn close(a: Pixels, b: Pixels) -> bool {
    (a - b).abs() < px(0.5)
}

/// Monaco `getInjectedTextInInterval`: an empty range's injected text
/// shows only with `showIfCollapsed`.
#[gpui::test]
fn injected_text_on_an_empty_range_shows_only_if_collapsed(cx: &mut TestAppContext) {
    let (editor, cx) = setup("abcdef", cx);
    let plain = x_of(&editor, p(0, 3), cx);
    let options = |show_if_collapsed| DecorationOptions {
        after: Some(InjectedText {
            content: "hint".into(),
            ..Default::default()
        }),
        show_if_collapsed,
        ..Default::default()
    };
    let ids = decorate(&editor, vec![(1..1, options(false))], cx);
    assert_eq!(x_of(&editor, p(0, 3), cx), plain);
    editor.update(cx, |e, cx| {
        e.model().update(cx, |model, cx| {
            model.change_decoration_options(ids[0], options(true), cx);
        })
    });
    draw(cx);
    let shown = x_of(&editor, p(0, 3), cx);
    assert!(shown > plain);
    // A non-empty range's always shows.
    let plain = x_of(&editor, p(0, 6), cx);
    decorate(&editor, vec![(4..5, options(false))], cx);
    assert!(x_of(&editor, p(0, 6), cx) > plain);
}

/// Monaco's cursor at a model position is before injected text there
/// (`PositionAffinity.None`); moving onto it from either side puts it after
/// the text when the text has a right stop (`MoveOperations.left`/`right`,
/// then `_validateViewState`), and before it otherwise.
#[gpui::test]
fn cursor_moves_respect_cursor_stops(cx: &mut TestAppContext) {
    for (stops, after) in [
        (InjectedTextCursorStops::Both, true),
        (InjectedTextCursorStops::Right, true),
        (InjectedTextCursorStops::Left, false),
        (InjectedTextCursorStops::None, false),
    ] {
        let (editor, cx) = setup("ab\ncd", cx);
        // At column 1 of both lines.
        inject(&editor, 1, stops, cx);
        inject(&editor, 4, stops, cx);
        // How far the cursor goes to be after the text.
        let char_width = x_of(&editor, p(0, 2), cx) - x_of(&editor, p(0, 1), cx);
        let width = x_of(&editor, p(0, 1), cx) - x_of(&editor, p(0, 0), cx) - char_width;
        assert!(width > px(0.));
        // From the model, the cursor is before it.
        select(&editor, p(0, 1), cx);
        let before = caret_x(&editor, cx);
        assert!(
            before < x_of(&editor, p(0, 1), cx) - width / 2.,
            "{stops:?}"
        );
        let expected = |after: bool| if after { before + width } else { before };

        // Right from column 0.
        select(&editor, p(0, 0), cx);
        cx.simulate_keystrokes("right");
        assert_eq!(head(&editor, cx), p(0, 1));
        assert!(
            close(caret_x(&editor, cx), expected(after)),
            "right, {stops:?}"
        );
        // Left from column 2.
        select(&editor, p(0, 2), cx);
        cx.simulate_keystrokes("left");
        assert_eq!(head(&editor, cx), p(0, 1));
        assert!(
            close(caret_x(&editor, cx), expected(after)),
            "left, {stops:?}"
        );
        // Down (`RightOfInjectedText`) is after it as the stops allow; up
        // (`LeftOfInjectedText`) before it.
        select(&editor, p(0, 1), cx);
        cx.simulate_keystrokes("down");
        assert_eq!(head(&editor, cx), p(1, 1));
        assert!(
            close(caret_x(&editor, cx), expected(after)),
            "down, {stops:?}"
        );
        cx.simulate_keystrokes("up");
        assert_eq!(head(&editor, cx), p(0, 1));
        assert!(close(caret_x(&editor, cx), before), "up, {stops:?}");
        // Setting the selection from the model resets it.
        select(&editor, p(0, 0), cx);
        cx.simulate_keystrokes("right");
        select(&editor, p(0, 1), cx);
        assert!(close(caret_x(&editor, cx), before), "{stops:?}");
        // The model position doesn't stop inside the text: one more Right
        // goes past `b`.
        cx.simulate_keystrokes("right");
        assert_eq!(head(&editor, cx), p(0, 2));
    }
}

/// Monaco's mouse targets are view positions: a click on injected text
/// lands on its nearer side, as its cursor stops allow.
#[gpui::test]
fn clicks_on_injected_text_respect_cursor_stops(cx: &mut TestAppContext) {
    for (stops, right_edge_after) in [
        (InjectedTextCursorStops::Both, true),
        (InjectedTextCursorStops::Left, false),
    ] {
        let (editor, cx) = setup("ab\ncd", cx);
        inject(&editor, 1, stops, cx);
        let char_width = x_of(&editor, p(0, 2), cx) - x_of(&editor, p(0, 1), cx);
        let start = x_of(&editor, p(0, 0), cx) + char_width;
        let end = x_of(&editor, p(0, 1), cx);
        let y =
            editor.read_with(cx, |e, _| e.scrolled_visible_position(p(0, 0)).unwrap().y) + px(2.);
        select(&editor, p(0, 1), cx);
        let before = caret_x(&editor, cx);
        select(&editor, p(1, 0), cx);
        // Just inside the right edge.
        cx.simulate_click(gpui::point(end - px(1.), y), Modifiers::none());
        draw(cx);
        assert_eq!(head(&editor, cx), p(0, 1));
        let expected = if right_edge_after {
            before + (end - start)
        } else {
            before
        };
        assert!(close(caret_x(&editor, cx), expected), "{stops:?}");
        // Just inside the left edge: before it.
        cx.simulate_click(gpui::point(start + px(1.), y), Modifiers::none());
        draw(cx);
        assert_eq!(head(&editor, cx), p(0, 1));
        assert!(close(caret_x(&editor, cx), before), "{stops:?}");
    }
}

/// Monaco `IMouseTargetContentText.detail.injectedText`: the decoration and
/// its `attachedData`.
#[gpui::test]
fn mouse_targets_carry_injected_text_data(cx: &mut TestAppContext) {
    let (editor, cx) = setup("ab", cx);
    inject(&editor, 1, InjectedTextCursorStops::Both, cx);
    let char_width = x_of(&editor, p(0, 2), cx) - x_of(&editor, p(0, 1), cx);
    let start = x_of(&editor, p(0, 0), cx) + char_width;
    let end = x_of(&editor, p(0, 1), cx);
    let y = editor.read_with(cx, |e, _| e.scrolled_visible_position(p(0, 0)).unwrap().y) + px(2.);
    let target = cx.update(|window, cx| {
        editor
            .read(cx)
            .mouse_target(gpui::point((start + end) / 2., y), window, cx)
    });
    let injected = target.injected_text.expect("injected text");
    assert_eq!(
        injected
            .attached_data
            .as_ref()
            .and_then(|data| data.downcast_ref::<&str>()),
        Some(&"payload")
    );
    assert!(injected.decoration.is_some());
    // Over plain text there's none.
    let target = cx.update(|window, cx| {
        editor
            .read(cx)
            .mouse_target(gpui::point(end + char_width / 2., y), window, cx)
    });
    assert_eq!(target.injected_text, None);
}

/// Monaco `firstLineDecorationClassName`: only on the first view line of a
/// wrapped line, while `linesDecorationsClassName` is on every view line
/// of the decoration's view range: all of a whole-line decoration's, only
/// the first of one on the line's first columns.
#[gpui::test]
fn first_line_decorations_skip_wrapped_lines(cx: &mut TestAppContext) {
    let text = "word ".repeat(60);
    let (editor, cx) = setup(&text, cx);
    editor.update(cx, |e, cx| {
        e.update_options_with(|o| o.word_wrap = WordWrap::On, cx)
    });
    draw(cx);
    let view_lines = editor.read_with(cx, |e, _| e.view_line_segments_for_tests().len());
    assert!(view_lines > 1);
    let red = gpui::red();
    let blue = gpui::blue();
    let green = gpui::green();
    decorate(
        &editor,
        vec![
            (
                0..1,
                DecorationOptions {
                    first_line_decoration: Some(LinesDecoration {
                        bar: Some(red),
                        text: None,
                    }),
                    ..Default::default()
                },
            ),
            (
                0..1,
                DecorationOptions {
                    gutter_bar: Some(blue),
                    is_whole_line: true,
                    ..Default::default()
                },
            ),
            (
                0..5,
                DecorationOptions {
                    gutter_bar: Some(green),
                    ..Default::default()
                },
            ),
        ],
        cx,
    );
    let bars = editor.read_with(cx, |e, _| e.gutter_bars_for_tests());
    assert_eq!(bars.iter().filter(|(_, c)| *c == red).count(), 1);
    // A whole-line `gutter_bar` is on every view line of the row.
    assert_eq!(bars.iter().filter(|(_, c)| *c == blue).count(), view_lines);
    // One on columns 0..5 only on the first.
    assert_eq!(bars.iter().filter(|(_, c)| *c == green).count(), 1);
    let first_top = bars.iter().find(|(_, c)| *c == red).unwrap().0.top();
    assert_eq!(
        bars.iter().find(|(_, c)| *c == blue).unwrap().0.top(),
        first_top
    );
    assert_eq!(
        bars.iter().find(|(_, c)| *c == green).unwrap().0.top(),
        first_top
    );
}

/// Monaco `LinesDecorationsOverlay._getDecorations`: a
/// `firstLineDecorationClassName` is on its range's start line only, even
/// when the range spans several rows.
#[gpui::test]
fn first_line_decorations_are_on_the_start_line_only(cx: &mut TestAppContext) {
    let (editor, cx) = setup("a\nb\nc\nd\ne\nf\n", cx);
    let red = gpui::red();
    let blue = gpui::blue();
    // Rows 1 to 4.
    let options = |first: bool| DecorationOptions {
        first_line_decoration: first.then_some(LinesDecoration {
            bar: Some(red),
            text: None,
        }),
        gutter_bar: (!first).then_some(blue),
        ..Default::default()
    };
    decorate(
        &editor,
        vec![(2..9, options(true)), (2..9, options(false))],
        cx,
    );
    let bars = editor.read_with(cx, |e, _| e.gutter_bars_for_tests());
    let tops = |color| {
        bars.iter()
            .filter(|(_, c)| *c == color)
            .map(|(b, _)| b.top())
            .collect::<Vec<_>>()
    };
    assert_eq!(tops(red).len(), 1);
    assert_eq!(tops(blue).len(), 4);
    assert_eq!(tops(red)[0], tops(blue)[0]);
}

/// Monaco converts a range's start to its view line with
/// `PositionAffinity.Right` and its end with `Left`: at a wrap point, a
/// range starting there is on the later view line only, one ending there
/// on the earlier one only.
#[gpui::test]
fn line_decorations_follow_the_view_range_at_wrap_points(cx: &mut TestAppContext) {
    let text = "word ".repeat(60);
    let (editor, cx) = setup(&text, cx);
    editor.update(cx, |e, cx| {
        e.update_options_with(|o| o.word_wrap = WordWrap::On, cx)
    });
    draw(cx);
    let segments = editor.read_with(cx, |e, _| e.view_line_segments_for_tests());
    assert!(segments.len() > 1);
    let wrap = segments[1].1.start as usize;
    let red = gpui::red();
    let blue = gpui::blue();
    let bar = |color| DecorationOptions {
        gutter_bar: Some(color),
        ..Default::default()
    };
    decorate(
        &editor,
        vec![(wrap..wrap + 1, bar(red)), (0..wrap, bar(blue))],
        cx,
    );
    let bars = editor.read_with(cx, |e, _| e.gutter_bars_for_tests());
    let tops = |color| {
        bars.iter()
            .filter(|(_, c)| *c == color)
            .map(|(b, _)| b.top())
            .collect::<Vec<_>>()
    };
    assert_eq!(tops(red).len(), 1);
    assert_eq!(tops(blue).len(), 1);
    assert!(tops(red)[0] > tops(blue)[0]);
}

/// Monaco `ThemeColor` in `overviewRuler.color`: looked up in the theme,
/// so the mark follows theme changes; `darkColor` in dark themes.
#[gpui::test]
fn overview_ruler_marks_follow_theme_colors(cx: &mut TestAppContext) {
    let (editor, cx) = setup(&"line\n".repeat(40), cx);
    let id = "editorOverviewRuler.errorForeground";
    decorate(
        &editor,
        vec![(
            10..12,
            DecorationOptions {
                overview_ruler: Some(OverviewRulerDecoration::new(
                    DecorationColor::theme(id),
                    OverviewRulerLane::Full,
                )),
                ..Default::default()
            },
        )],
        cx,
    );
    let mark_colors = |cx: &mut VisualTestContext| {
        draw(cx);
        editor.read_with(cx, |e, _| {
            e.overview_ruler_for_tests()
                .map(|ruler| ruler.marks.iter().map(|(_, c)| *c).collect::<Vec<_>>())
                .unwrap_or_default()
        })
    };
    let dark = Theme::vs_dark().color(id).unwrap();
    let high_contrast = Theme::hc_black().color(id).unwrap();
    assert_ne!(dark, high_contrast);
    assert_eq!(mark_colors(cx), [dark]);
    editor.update(cx, |e, cx| e.set_theme(Arc::new(Theme::hc_black()), cx));
    assert_eq!(mark_colors(cx), [high_contrast]);
    // `darkColor` in a dark theme only.
    let ruler = OverviewRulerDecoration {
        dark_color: Some(gpui::red().into()),
        ..OverviewRulerDecoration::new(gpui::blue(), OverviewRulerLane::Left)
    };
    assert_eq!(ruler.resolve(&Theme::vs_dark()), Some(gpui::red()));
    assert_eq!(ruler.resolve(&Theme::vs()), Some(gpui::blue()));
    // An unknown id draws nothing.
    assert_eq!(
        DecorationColor::theme("no.such.color").resolve(&Theme::vs()),
        None
    );
}

/// Monaco `effectiveAllowVariableFonts` and `filterFontDecorations`: with
/// a screen reader (unless `allowVariableFontsInAccessibilityMode`) or
/// without `allowVariableFonts`, decorations that change the font are left
/// out of the view, their overview ruler and minimap marks and the
/// editor's getters included, and their font changes aren't forwarded.
#[gpui::test]
fn font_decorations_follow_allow_variable_fonts(cx: &mut TestAppContext) {
    use gpui_monaco::options::AccessibilitySupport;
    let (editor, cx) = setup(&"line\n".repeat(40), cx);
    let font_changes = Rc::new(Cell::new(0));
    let sink = font_changes.clone();
    cx.update(|_, cx| {
        cx.subscribe(&editor, move |_, event, _| {
            if let CodeEditorEvent::FontChanged(_) = event {
                sink.set(sink.get() + 1);
            }
        })
        .detach();
    });
    let bold_mark = |color: gpui::Hsla| DecorationOptions {
        font_weight: Some(gpui::FontWeight::BOLD),
        overview_ruler: Some(OverviewRulerDecoration::new(color, OverviewRulerLane::Full)),
        minimap: Some(gpui_monaco::MinimapDecoration {
            color: Some(color.into()),
            position: gpui_monaco::MinimapPosition::Gutter,
            ..Default::default()
        }),
        ..Default::default()
    };
    // Rows 2 and 4; the plain one on row 4 stays in every case.
    decorate(
        &editor,
        vec![
            (10..12, bold_mark(gpui::red())),
            (
                20..22,
                DecorationOptions {
                    overview_ruler: Some(OverviewRulerDecoration::new(
                        gpui::blue(),
                        OverviewRulerLane::Full,
                    )),
                    ..Default::default()
                },
            ),
        ],
        cx,
    );
    font_changes.set(0);
    let shown = |cx: &mut VisualTestContext| {
        draw(cx);
        let marks = editor.read_with(cx, |e, _| {
            e.overview_ruler_for_tests()
                .map(|ruler| ruler.marks.iter().map(|(_, c)| *c).collect::<Vec<_>>())
                .unwrap_or_default()
        });
        assert!(marks.contains(&gpui::blue()));
        // The minimap's gutter bar, on its 0.9-opaque decorations canvas.
        let red = gpui::red().opacity(0.9);
        let in_minimap = editor.read_with(cx, |e, _| {
            e.minimap_marks_for_tests().iter().any(|(_, c)| {
                (c.h - red.h).abs() < 1e-4
                    && (c.s - red.s).abs() < 1e-4
                    && (c.l - red.l).abs() < 1e-4
                    && (c.a - red.a).abs() < 1e-4
            })
        });
        let on_row = editor.update(cx, |e, cx| e.line_decorations(2, cx).len());
        (marks.contains(&gpui::red()), in_minimap, on_row)
    };
    let set = |change: fn(&mut gpui_monaco::EditorOptions), cx: &mut VisualTestContext| {
        editor.update(cx, |e, cx| e.update_options_with(change, cx));
    };
    assert!(editor.read_with(cx, |e, _| e.allows_variable_fonts()));
    assert_eq!(shown(cx), (true, true, 1));
    // With a screen reader, `allowVariableFontsInAccessibilityMode` (off by
    // default) decides.
    set(|o| o.accessibility_support = AccessibilitySupport::On, cx);
    assert!(!editor.read_with(cx, |e, _| e.allows_variable_fonts()));
    assert_eq!(shown(cx), (false, false, 0));
    // The model reports the font change; the editor doesn't forward it.
    decorate(&editor, vec![(30..32, bold_mark(gpui::green()))], cx);
    assert_eq!(font_changes.get(), 0);
    set(|o| o.allow_variable_fonts_in_accessibility_mode = true, cx);
    assert_eq!(shown(cx), (true, true, 1));
    // Without one, `allowVariableFonts` does.
    set(
        |o| {
            o.accessibility_support = AccessibilitySupport::Off;
            o.allow_variable_fonts = false;
        },
        cx,
    );
    assert_eq!(shown(cx), (false, false, 0));
    set(|o| o.allow_variable_fonts = true, cx);
    assert_eq!(shown(cx), (true, true, 1));
    decorate(&editor, vec![(40..42, bold_mark(gpui::green()))], cx);
    assert_eq!(font_changes.get(), 1);
}

/// Monaco `onDidChangeTokens` ranges: everything after the first parse,
/// then the edited lines and what the parse changed around them.
#[gpui::test]
fn token_change_events_carry_line_ranges(cx: &mut TestAppContext) {
    cx.update(|cx| gpui_monaco::set_language_registry(cx, gpui_monaco_languages::registry()));
    let text: String = (0..30).map(|i| format!("let v{i} = {i};\n")).collect();
    let (editor, cx) = setup("", cx);
    let model = cx.update(|_, cx| cx.new(|_| TextModel::new(&text).with_language("rust")));
    let ranges: Rc<RefCell<Vec<Vec<std::ops::RangeInclusive<u32>>>>> = Default::default();
    let editor_ranges: Rc<RefCell<Vec<Vec<std::ops::RangeInclusive<u32>>>>> = Default::default();
    cx.update(|_, cx| {
        let sink = ranges.clone();
        cx.subscribe(&model, move |_, event: &TextModelEvent, _| {
            if let TextModelEvent::SyntaxUpdated { ranges } = event {
                sink.borrow_mut().push(ranges.to_vec());
            }
        })
        .detach();
        let sink = editor_ranges.clone();
        cx.subscribe(&editor, move |_, event: &CodeEditorEvent, _| {
            if let CodeEditorEvent::ModelTokensChanged { ranges } = event {
                sink.borrow_mut().push(ranges.to_vec());
            }
        })
        .detach();
    });
    editor.update(cx, |e, cx| e.set_model(model.clone(), cx));
    draw(cx);
    let whole = std::iter::once(0..=30).collect::<Vec<_>>();
    assert_eq!(ranges.borrow().last().cloned(), Some(whole));
    ranges.borrow_mut().clear();
    // An edit on row 12 that keeps the rest of the tree.
    model.update(cx, |model, cx| {
        let at = model.buffer().rope().point_to_offset(Point::new(12, 4));
        model.edit([(at..at, "x")], cx).unwrap();
    });
    draw(cx);
    let last = ranges.borrow().last().cloned().expect("a token change");
    assert!(last.iter().any(|r| r.contains(&12)), "{last:?}");
    assert!(
        !last.iter().any(|r| r.contains(&0) || r.contains(&29)),
        "{last:?}"
    );
    // The editor forwards them.
    assert_eq!(editor_ranges.borrow().last().cloned(), Some(last));
}

/// Monaco `ITextModel.onWillDispose`: called when the model goes.
#[gpui::test]
fn a_model_announces_its_disposal(cx: &mut TestAppContext) {
    let disposed = Rc::new(Cell::new(None));
    let model = cx.new(|_| TextModel::new("bye"));
    let sink = disposed.clone();
    let _subscription = cx.update(|cx| {
        TextModel::on_will_dispose(&model, cx, move |model, _| {
            sink.set(Some(model.text()));
        })
    });
    assert_eq!(disposed.take(), None);
    drop(model);
    cx.update(|_| {});
    cx.run_until_parked();
    assert_eq!(disposed.take().as_deref(), Some("bye"));
}

#[derive(Debug, PartialEq)]
struct Breakpoints(Vec<u32>);

/// Monaco `getContribution`: a host's per-editor feature, found by type.
#[gpui::test]
fn contributions_by_type(cx: &mut TestAppContext) {
    let (editor, cx) = setup("x", cx);
    editor.update(cx, |e, _| {
        assert_eq!(e.get_contribution::<Breakpoints>(), None);
        assert_eq!(e.add_contribution(Breakpoints(vec![1])), None);
        e.get_contribution_mut::<Breakpoints>().unwrap().0.push(4);
        assert_eq!(
            e.get_contribution::<Breakpoints>(),
            Some(&Breakpoints(vec![1, 4]))
        );
        assert_eq!(
            e.add_contribution(Breakpoints(Vec::new())),
            Some(Breakpoints(vec![1, 4]))
        );
        assert_eq!(
            e.remove_contribution::<Breakpoints>(),
            Some(Breakpoints(Vec::new()))
        );
        assert_eq!(e.get_contribution::<Breakpoints>(), None);
    });
}

/// Monaco `getFontSizeAtPosition`, and the aria options.
#[gpui::test]
fn font_size_and_aria_options(cx: &mut TestAppContext) {
    let (editor, cx) = setup("x", cx);
    editor.update(cx, |e, cx| {
        let size = e.options().font_size;
        assert_eq!(e.get_font_size_at_position(p(0, 0)), Some(size));
        let options = AriaOptions {
            role: Some(gpui::Role::ComboBox),
            active_descendant: None,
        };
        e.set_aria_options(options.clone(), cx);
        assert_eq!(e.aria_options(), &options);
        e.write_screen_reader_content("test", cx);
    });
    draw(cx);
}

/// Monaco `setModel(null)`: no model, nothing drawn, input ignored, until a
/// model is set again.
#[gpui::test]
fn an_editor_without_a_model(cx: &mut TestAppContext) {
    let (editor, cx) = setup("hello", cx);
    let original = editor.read_with(cx, |e, _| e.model().clone());
    let changes = Rc::new(RefCell::new(Vec::new()));
    cx.update(|_, cx| {
        let sink = changes.clone();
        cx.subscribe(&editor, move |_, event: &CodeEditorEvent, _| {
            if let CodeEditorEvent::ModelChanged(change) = event {
                sink.borrow_mut().push(change.new_uri.clone());
            }
        })
        .detach();
    });
    editor.update(cx, |e, cx| e.set_model(None, cx));
    draw(cx);
    assert_eq!(changes.borrow().len(), 1);
    editor.read_with(cx, |e, cx| {
        assert!(!e.has_model());
        assert!(e.get_model().is_none());
        assert_eq!(e.get_font_size_at_position(p(0, 0)), None);
        // The stand-in isn't a model hosts see.
        assert!(!gpui_monaco::models(cx).contains(e.model()));
        // Nothing is laid out.
        assert!(e.cursor_shapes_for_tests().0.is_empty());
    });
    // Typing and edits do nothing.
    cx.simulate_keystrokes("a b enter");
    let edited = editor.update(cx, |e, cx| {
        e.execute_edits(vec![(p(0, 0)..p(0, 0), "x".into())], None, cx)
    });
    assert!(!edited);
    editor.read_with(cx, |e, cx| assert_eq!(e.model().read(cx).text(), ""));
    // A model again.
    editor.update(cx, |e, cx| e.set_model(original.clone(), cx));
    draw(cx);
    editor.read_with(cx, |e, _| {
        assert!(e.has_model());
        assert_eq!(e.get_model(), Some(&original));
        assert_eq!(e.cursor_shapes_for_tests().0.len(), 1);
    });
    cx.simulate_keystrokes("a");
    editor.read_with(cx, |e, cx| assert_eq!(e.model().read(cx).text(), "ahello"));
}

/// Monaco `CommandsRegistry.registerCommand`: an `IDisposable`; the newest
/// handler runs, and disposing it brings back the one before.
#[gpui::test]
fn registered_commands_are_disposable(cx: &mut TestAppContext) {
    let ran = Rc::new(RefCell::new(Vec::new()));
    let command = Command {
        id: "test.disposable".into(),
        title: "".into(),
        ..Default::default()
    };
    cx.update(|cx| {
        let sink = ran.clone();
        let first = gpui_monaco::register_command(cx, "test.disposable", move |_, _| {
            sink.borrow_mut().push(1)
        });
        let sink = ran.clone();
        let second = gpui_monaco::register_command(cx, "test.disposable", move |_, _| {
            sink.borrow_mut().push(2)
        });
        gpui_monaco::execute_command(cx, &command).unwrap();
        second.dispose(cx);
        gpui_monaco::execute_command(cx, &command).unwrap();
        first.dispose(cx);
        assert!(gpui_monaco::execute_command(cx, &command).is_err());
    });
    assert_eq!(*ran.borrow(), [2, 1]);
}

/// Monaco `createModel` throws for a uri a live model has.
#[gpui::test]
fn creating_a_model_with_a_taken_uri_fails(cx: &mut TestAppContext) {
    cx.update(|cx| {
        gpui_monaco::init(cx);
        let uri = "file:///taken.rs";
        let first = gpui_monaco::create_model(cx, "a", Some("rust"), Some(uri)).unwrap();
        assert_eq!(first.read(cx).uri().map(|u| u.as_ref()), Some(uri));
        let err = gpui_monaco::create_model(cx, "b", None, Some(uri)).unwrap_err();
        assert!(err.to_string().contains("already exists"), "{err}");
        assert_eq!(gpui_monaco::model_for_uri(cx, uri), Some(first.clone()));
        // Without a uri there's nothing to clash.
        assert!(gpui_monaco::create_model(cx, "c", None, None).is_ok());
        drop(first);
    });
    cx.run_until_parked();
    cx.update(|cx| {
        assert!(gpui_monaco::create_model(cx, "d", None, Some("file:///taken.rs")).is_ok());
    });
}

/// Monaco `ThemeColor` in `minimap.color`: the minimap mark follows the
/// theme too.
#[gpui::test]
fn minimap_marks_follow_theme_colors(cx: &mut TestAppContext) {
    let (editor, cx) = setup(&"line with some text\n".repeat(40), cx);
    let id = "minimap.errorHighlight";
    decorate(
        &editor,
        vec![(
            30..31,
            DecorationOptions {
                minimap: Some(gpui_monaco::MinimapDecoration {
                    color: Some(DecorationColor::theme(id)),
                    position: gpui_monaco::MinimapPosition::Gutter,
                    ..Default::default()
                }),
                ..Default::default()
            },
        )],
        cx,
    );
    // The gutter bar, on the minimap's 0.9-opaque decorations canvas.
    let has_mark = |color: gpui::Hsla, cx: &mut VisualTestContext| {
        draw(cx);
        let expected = color.opacity(0.9);
        editor.read_with(cx, |e, _| {
            e.minimap_marks_for_tests().iter().any(|(_, c)| {
                (c.h - expected.h).abs() < 1e-4
                    && (c.s - expected.s).abs() < 1e-4
                    && (c.l - expected.l).abs() < 1e-4
                    && (c.a - expected.a).abs() < 1e-4
            })
        })
    };
    let dark = Theme::vs_dark().color(id).unwrap();
    let high_contrast = Theme::hc_black().color(id).unwrap();
    assert_ne!(dark, high_contrast);
    assert!(has_mark(dark, cx));
    editor.update(cx, |e, cx| e.set_theme(Arc::new(Theme::hc_black()), cx));
    assert!(has_mark(high_contrast, cx));
    assert!(!has_mark(dark, cx));
}
