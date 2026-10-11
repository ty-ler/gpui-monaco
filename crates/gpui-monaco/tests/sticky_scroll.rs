//! Sticky scroll's model (Monaco `stickyScroll.defaultModel`): outline by
//! default, falling back to folding ranges; and its widget (Monaco
//! `StickyScrollController` and `StickyScrollWidget`).

use gpui::{AppContext as _, Entity, TestAppContext, VisualTestContext, px};
use gpui_monaco::options::{EditorOptions, StickyScrollModel, StickyScrollOptions};
use gpui_monaco::text::PointUtf16;
use gpui_monaco::{CodeEditor, TextModel};

fn source() -> String {
    let mut text = String::from("impl Foo {\n    fn run(&self) {\n        if true {\n");
    for i in 0..200 {
        text.push_str(&format!("            let v{i} = {i};\n"));
    }
    text.push_str("        }\n    }\n}\n");
    text
}

fn setup<'a>(
    language: &str,
    model: StickyScrollModel,
    cx: &'a mut TestAppContext,
) -> (Entity<CodeEditor>, &'a mut VisualTestContext) {
    cx.update(|cx| {
        gpui_monaco::init(cx);
        gpui_monaco::set_language_registry(cx, gpui_monaco_languages::registry());
    });
    let text = source();
    let language = language.to_string();
    let (editor, cx) = cx.add_window_view(|window, cx| {
        let model_entity = cx.new(|_| TextModel::new(&text).with_language(&language));
        let mut editor = CodeEditor::new(model_entity, window, cx);
        editor.update_options(
            EditorOptions {
                sticky_scroll: StickyScrollOptions {
                    default_model: model,
                    ..Default::default()
                },
                ..EditorOptions::default()
            },
            cx,
        );
        editor
    });
    cx.run_until_parked();
    (editor, cx)
}

fn sticky_rows_at(editor: &Entity<CodeEditor>, y: f32, cx: &mut VisualTestContext) -> Vec<u32> {
    editor.update(cx, |e, cx| {
        e.set_scroll_position(gpui::point(px(0.), px(y)), cx)
    });
    for _ in 0..3 {
        cx.update(|window, _| window.refresh());
        cx.run_until_parked();
        cx.executor()
            .advance_clock(std::time::Duration::from_millis(500));
        cx.run_until_parked();
    }
    editor.read_with(cx, |e, _| e.sticky_rows_for_tests().0)
}

#[gpui::test]
fn outline_model_sticks_symbols_only(cx: &mut TestAppContext) {
    let (editor, cx) = setup("rust", StickyScrollModel::Outline, cx);
    // The `if` block isn't a symbol.
    assert_eq!(sticky_rows_at(&editor, 1000., cx), vec![0, 1]);
}

#[gpui::test]
fn folding_model_sticks_every_block(cx: &mut TestAppContext) {
    let (editor, cx) = setup("rust", StickyScrollModel::FoldingProvider, cx);
    assert_eq!(sticky_rows_at(&editor, 1000., cx), vec![0, 1, 2]);
}

#[gpui::test]
fn indentation_model(cx: &mut TestAppContext) {
    let (editor, cx) = setup("rust", StickyScrollModel::Indentation, cx);
    assert_eq!(sticky_rows_at(&editor, 1000., cx), vec![0, 1, 2]);
}

#[gpui::test]
fn outline_without_symbols_falls_back_to_folding(cx: &mut TestAppContext) {
    // Plain text has no symbols (and indentation folds).
    let (editor, cx) = setup("plaintext", StickyScrollModel::Outline, cx);
    assert_eq!(sticky_rows_at(&editor, 1000., cx), vec![0, 1, 2]);
}

fn cursor_row(editor: &Entity<CodeEditor>, cx: &mut VisualTestContext) -> u32 {
    editor.read_with(cx, |e, _| e.selections()[0].head.row)
}

#[gpui::test]
fn keyboard_focus_moves_and_goes_to_lines(cx: &mut TestAppContext) {
    use gpui::Focusable;
    use gpui_monaco::actions::*;
    let (editor, cx) = setup("rust", StickyScrollModel::Outline, cx);
    cx.update(|window, cx| window.focus(&editor.focus_handle(cx), cx));
    assert_eq!(sticky_rows_at(&editor, 1000., cx), vec![0, 1]);
    // Focus starts on the last sticky line.
    cx.dispatch_action(FocusStickyScroll);
    assert!(editor.read_with(cx, |e, _| e.is_sticky_scroll_focused()));
    // Up, then Enter: goes to the first.
    cx.simulate_keystrokes("up enter");
    cx.run_until_parked();
    assert!(!editor.read_with(cx, |e, _| e.is_sticky_scroll_focused()));
    assert_eq!(cursor_row(&editor, cx), 0);

    // Escape returns to the text without moving.
    sticky_rows_at(&editor, 1000., cx);
    cx.dispatch_action(FocusStickyScroll);
    cx.simulate_keystrokes("escape");
    assert!(!editor.read_with(cx, |e, _| e.is_sticky_scroll_focused()));
    assert_eq!(cursor_row(&editor, cx), 0);
}

#[gpui::test]
fn toggle_turns_sticky_scroll_off_and_on(cx: &mut TestAppContext) {
    use gpui_monaco::actions::*;
    let (editor, cx) = setup("rust", StickyScrollModel::Outline, cx);
    assert_eq!(sticky_rows_at(&editor, 1000., cx), vec![0, 1]);
    editor.update(cx, |e, cx| e.toggle_sticky_scroll(cx));
    assert!(sticky_rows_at(&editor, 1000., cx).is_empty());
    cx.update(|window, cx| window.focus(&gpui::Focusable::focus_handle(&editor, cx), cx));
    cx.dispatch_action(ToggleStickyScroll);
    assert_eq!(sticky_rows_at(&editor, 1000., cx), vec![0, 1]);
}

// ----- #254: Monaco's sticky widget state, clicks and folding controls -----

fn setup_with<'a>(
    language: &str,
    configure: impl FnOnce(&mut EditorOptions),
    cx: &'a mut TestAppContext,
) -> (Entity<CodeEditor>, &'a mut VisualTestContext) {
    let (editor, cx) = setup(language, StickyScrollModel::FoldingProvider, cx);
    editor.update(cx, |e, cx| {
        let mut options = e.options().clone();
        configure(&mut options);
        e.update_options(options, cx);
    });
    cx.run_until_parked();
    (editor, cx)
}

fn line_height(editor: &Entity<CodeEditor>, cx: &mut VisualTestContext) -> f32 {
    cx.update(|window, _| window.refresh());
    cx.run_until_parked();
    editor.read_with(cx, |e, _| f32::from(e.view_metrics_for_tests().unwrap().3))
}

/// Monaco: the last sticky line slides up under the one above as its
/// scope scrolls out; it never goes above that line, and the lines above
/// stay put (the overdrawn `mod tests` / `fn transform_offsets` headers).
#[gpui::test]
fn the_pushed_last_line_never_overdraws_the_one_above(cx: &mut TestAppContext) {
    let (editor, cx) = setup("rust", StickyScrollModel::FoldingProvider, cx);
    let lh = line_height(&editor, cx);
    let mut pushed = false;
    // Across the `if` block's end (row 203) and the `fn`'s (204).
    let mut y = 196. * lh;
    while y < 206. * lh {
        sticky_rows_at(&editor, y, cx);
        let (lines, height) = editor.read_with(cx, |e, _| e.sticky_lines_for_tests());
        let top = editor.read_with(cx, |e, _| f32::from(e.widget_bounds_for_tests().0.top()));
        let n = lines.len();
        for (ix, (_, bounds, _)) in lines.iter().enumerate() {
            let line_top = f32::from(bounds.top()) - top;
            if ix + 1 < n {
                assert_eq!(line_top, ix as f32 * lh, "line {ix} at scroll {y}");
            } else {
                // Under the line above (at most all of it), not over it.
                assert!(
                    line_top >= (ix as f32 - 1.) * lh,
                    "last line at {line_top} at scroll {y}"
                );
                pushed |= line_top < ix as f32 * lh;
                assert_eq!(
                    f32::from(height),
                    f32::from(bounds.bottom()) - top,
                    "the widget ends with the last line"
                );
            }
        }
        y += lh / 3.;
    }
    assert!(pushed, "the last line was pushed up at some point");
}

/// Monaco's sticky model providers don't depend on the `folding` option.
#[gpui::test]
fn sticky_scroll_works_with_folding_off(cx: &mut TestAppContext) {
    let (editor, cx) = setup_with("rust", |o| o.folding = false, cx);
    assert_eq!(sticky_rows_at(&editor, 1000., cx), vec![0, 1, 2]);
    // With folding off there are no folding controls in it.
    let (lines, _) = editor.read_with(cx, |e, _| e.sticky_lines_for_tests());
    assert!(lines.iter().all(|(_, _, fold)| fold.is_none()));
}

/// Monaco's indentation model uses the language's `offSide` (YAML's
/// blocks end at their last indented line, before the blank lines).
#[gpui::test]
fn indentation_model_respects_off_side(cx: &mut TestAppContext) {
    cx.update(|cx| {
        gpui_monaco::init(cx);
        gpui_monaco::set_language_registry(cx, gpui_monaco_languages::registry());
    });
    let mut text = String::from("root:\n  a: 1\n");
    for i in 0..100 {
        text.push_str(&format!("  v{i}: {i}\n"));
    }
    text.push_str("\n\n\nother: 2\n");
    for i in 0..100 {
        text.push_str(&format!("x{i}: {i}\n"));
    }
    let (editor, cx) = cx.add_window_view(|window, cx| {
        let model = cx.new(|_| TextModel::new(&text).with_language("yaml"));
        let mut editor = CodeEditor::new(model, window, cx);
        let mut options = editor.options().clone();
        options.sticky_scroll.default_model = StickyScrollModel::Indentation;
        editor.update_options(options, cx);
        editor
    });
    cx.run_until_parked();
    let lh = line_height(&editor, cx);
    assert_eq!(sticky_rows_at(&editor, 50. * lh, cx), vec![0]);
    // The blank lines after the block (rows 102-104) aren't in it.
    assert!(sticky_rows_at(&editor, 103. * lh, cx).is_empty());
}

/// Monaco caps the sticky lines at a quarter of the editor's height.
#[gpui::test]
fn sticky_lines_take_at_most_a_quarter_of_the_height(cx: &mut TestAppContext) {
    let (editor, cx) = setup("rust", StickyScrollModel::FoldingProvider, cx);
    let lh = line_height(&editor, cx);
    cx.simulate_resize(gpui::size(px(800.), px(lh * 6.)));
    assert_eq!(sticky_rows_at(&editor, 100. * lh, cx), vec![0, 1]);
    cx.simulate_resize(gpui::size(px(800.), px(lh * 3.)));
    assert_eq!(sticky_rows_at(&editor, 100. * lh, cx), vec![0]);
}

fn click(cx: &mut VisualTestContext, at: gpui::Point<gpui::Pixels>, modifiers: gpui::Modifiers) {
    cx.simulate_event(gpui::MouseDownEvent {
        position: at,
        button: gpui::MouseButton::Left,
        modifiers,
        click_count: 1,
        first_mouse: false,
    });
    cx.simulate_event(gpui::MouseUpEvent {
        position: at,
        button: gpui::MouseButton::Left,
        modifiers,
        click_count: 1,
    });
    cx.run_until_parked();
}

/// Monaco `_toggleFoldingRegionForLine`: the chevron folds without moving
/// the cursor.
#[gpui::test]
fn a_sticky_chevron_click_folds_without_moving_the_cursor(cx: &mut TestAppContext) {
    use PointUtf16;
    use gpui_monaco::cursor::Selection;
    let (editor, cx) = setup_with(
        "rust",
        |o| o.show_folding_controls = gpui_monaco::options::ShowFoldingControls::Always,
        cx,
    );
    let start = PointUtf16::new(0, 3);
    editor.update(cx, |e, cx| {
        e.set_selections(&[Selection::collapsed(start)], cx)
    });
    let lh = line_height(&editor, cx);
    assert_eq!(sticky_rows_at(&editor, 100. * lh, cx), vec![0, 1, 2]);
    let (lines, _) = editor.read_with(cx, |e, _| e.sticky_lines_for_tests());
    let fold = lines[2].2.expect("the `if` header has a folding control");
    click(cx, fold.center(), gpui::Modifiers::default());
    assert_eq!(editor.read_with(cx, |e, _| e.selections()[0].head), start);
    let folds = editor.read_with(cx, |e, cx| e.collapsed_folds(cx));
    assert!(folds.iter().any(|(row, _)| *row == 2), "{folds:?}");
}

/// Monaco: no sticky folding controls with `showFoldingControls: never`.
#[gpui::test]
fn no_sticky_chevrons_with_folding_controls_never(cx: &mut TestAppContext) {
    let (editor, cx) = setup_with(
        "rust",
        |o| o.show_folding_controls = gpui_monaco::options::ShowFoldingControls::Never,
        cx,
    );
    assert_eq!(sticky_rows_at(&editor, 1000., cx), vec![0, 1, 2]);
    let (lines, _) = editor.read_with(cx, |e, _| e.sticky_lines_for_tests());
    assert!(lines.iter().all(|(_, _, fold)| fold.is_none()));
}

/// The x of `column` on `row`, measured while the row shows.
fn column_x(
    editor: &Entity<CodeEditor>,
    row: u32,
    column: u32,
    cx: &mut VisualTestContext,
) -> gpui::Pixels {
    cx.update(|window, _| window.refresh());
    cx.run_until_parked();
    editor.read_with(cx, |e, _| {
        e.layout_info().unwrap().content_left + e.offset_for_column(row, column).unwrap()
    })
}

/// Monaco puts the cursor at the clicked column, on the click (mouse up).
#[gpui::test]
fn a_sticky_click_goes_to_the_clicked_column(cx: &mut TestAppContext) {
    use PointUtf16;
    let (editor, cx) = setup("rust", StickyScrollModel::FoldingProvider, cx);
    // `run` on row 1.
    let x = column_x(&editor, 1, 8, cx);
    let lh = line_height(&editor, cx);
    sticky_rows_at(&editor, 100. * lh, cx);
    let (lines, _) = editor.read_with(cx, |e, _| e.sticky_lines_for_tests());
    let at = gpui::point(x + px(2.), lines[1].1.center().y);
    let before = editor.read_with(cx, |e, _| e.selections()[0].head);
    cx.simulate_event(gpui::MouseDownEvent {
        position: at,
        button: gpui::MouseButton::Left,
        modifiers: gpui::Modifiers::default(),
        click_count: 1,
        first_mouse: false,
    });
    cx.run_until_parked();
    assert_eq!(editor.read_with(cx, |e, _| e.selections()[0].head), before);
    cx.simulate_event(gpui::MouseUpEvent {
        position: at,
        button: gpui::MouseButton::Left,
        modifiers: gpui::Modifiers::default(),
        click_count: 1,
    });
    cx.run_until_parked();
    assert_eq!(
        editor.read_with(cx, |e, _| e.selections()[0].head),
        PointUtf16::new(1, 8)
    );
}

/// Monaco renders sticky lines with their injected text (inlay hints), so
/// they line up with the real lines and clicks land on the right column.
#[gpui::test]
fn sticky_lines_show_injected_text(cx: &mut TestAppContext) {
    let (editor, cx) = setup("rust", StickyScrollModel::FoldingProvider, cx);
    editor.update(cx, |e, cx| {
        let model = e.model().clone();
        model.update(cx, |model, cx| {
            let at = model
                .buffer()
                .rope()
                .point_to_offset(text::Point::new(1, 4));
            model.delta_decorations(
                None,
                &[],
                vec![(
                    at..at,
                    gpui_monaco::DecorationOptions {
                        before: Some(gpui_monaco::InjectedText {
                            content: "hint: ".into(),
                            ..Default::default()
                        }),
                        show_if_collapsed: true,
                        ..Default::default()
                    },
                )],
                cx,
            );
        });
    });
    let x = column_x(&editor, 1, 8, cx);
    let lh = line_height(&editor, cx);
    sticky_rows_at(&editor, 100. * lh, cx);
    let (lines, _) = editor.read_with(cx, |e, _| e.sticky_lines_for_tests());
    click(
        cx,
        gpui::point(x + px(2.), lines[1].1.center().y),
        gpui::Modifiers::default(),
    );
    assert_eq!(
        editor.read_with(cx, |e, _| e.selections()[0].head),
        PointUtf16::new(1, 8)
    );
}

struct Definitions(Vec<gpui_monaco::providers::Location>);

impl gpui_monaco::providers::DefinitionProvider for Definitions {
    fn provide_definition(
        &self,
        _: &gpui_monaco::providers::ProviderContext,
        _: PointUtf16,
        _: &mut gpui::App,
    ) -> gpui::Task<anyhow::Result<Vec<gpui_monaco::providers::Location>>> {
        gpui::Task::ready(Ok(self.0.clone()))
    }
}

/// Monaco's sticky Cmd/Ctrl+click goes to the definition of the word.
#[gpui::test]
fn cmd_click_on_a_sticky_line_goes_to_the_definition(cx: &mut TestAppContext) {
    use PointUtf16;
    use gpui_monaco::providers::{LanguageSelector, Location, register_definition_provider};
    let (editor, cx) = setup_with("rust", |o| o.definition_link_opens_in_peek = false, cx);
    let target = PointUtf16::new(150, 12);
    cx.update(|_, cx| {
        register_definition_provider(
            cx,
            LanguageSelector::All,
            Definitions(vec![Location::new(None, target..target)]),
        );
    });
    let x = column_x(&editor, 1, 8, cx);
    let lh = line_height(&editor, cx);
    sticky_rows_at(&editor, 100. * lh, cx);
    let (lines, _) = editor.read_with(cx, |e, _| e.sticky_lines_for_tests());
    let at = gpui::point(x + px(2.), lines[1].1.center().y);
    let cmd = gpui::Modifiers {
        platform: cfg!(target_os = "macos"),
        control: !cfg!(target_os = "macos"),
        ..Default::default()
    };
    cx.simulate_mouse_move(at, None, cmd);
    cx.executor()
        .advance_clock(std::time::Duration::from_millis(200));
    cx.run_until_parked();
    click(cx, at, cmd);
    cx.executor()
        .advance_clock(std::time::Duration::from_millis(200));
    cx.run_until_parked();
    assert_eq!(editor.read_with(cx, |e, _| e.selections()[0].head), target);
}

/// Monaco `_renderStickyScroll`: after a scroll, the focused sticky line
/// stays focused while it shows, else the last line is.
#[gpui::test]
fn sticky_keyboard_focus_follows_the_lines(cx: &mut TestAppContext) {
    use gpui::Focusable;
    use gpui_monaco::actions::*;
    let (editor, cx) = setup("rust", StickyScrollModel::FoldingProvider, cx);
    cx.update(|window, cx| window.focus(&editor.focus_handle(cx), cx));
    let lh = line_height(&editor, cx);
    assert_eq!(sticky_rows_at(&editor, 100. * lh, cx), vec![0, 1, 2]);
    // Focus the last line (`if`), then scroll past the `if` block.
    cx.dispatch_action(FocusStickyScroll);
    assert_eq!(sticky_rows_at(&editor, 203. * lh, cx), vec![0, 1]);
    cx.simulate_keystrokes("enter");
    cx.run_until_parked();
    assert_eq!(cursor_row(&editor, cx), 1);
}
