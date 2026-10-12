//! Option validation (Monaco's `validate` methods and
//! `TextModelResolvedOptions`), the layout widths of Monaco's
//! `EditorLayoutInfoComputer`, the word highlighter's timing and
//! `autoDetectHighContrast`.

use std::time::Duration;

use gpui::{AppContext as _, Entity, Focusable, TestAppContext, VisualTestContext, px};
use gpui_monaco::cursor::Selection;
use gpui_monaco::options::{LineDecorationsWidth, ShowFoldingControls};
use gpui_monaco::text::{LineEnding, PointUtf16};
use gpui_monaco::{CodeEditor, EditorOptions, LayoutInfo, ModelOptions, TextModel};

fn setup(
    model: impl FnOnce() -> TextModel + 'static,
    cx: &mut TestAppContext,
) -> (Entity<CodeEditor>, &mut VisualTestContext) {
    cx.update(gpui_monaco::init);
    // With a glyph margin, a line height wide, to measure line heights.
    let (editor, cx) = cx.add_window_view(|window, cx| {
        let model = cx.new(|_| model());
        let mut editor = CodeEditor::new(model, window, cx);
        editor.update_options_with(|o| o.glyph_margin = true, cx);
        editor
    });
    cx.update(|window, cx| window.focus(&editor.focus_handle(cx), cx));
    paint(cx);
    (editor, cx)
}

fn paint(cx: &mut VisualTestContext) {
    cx.update(|window, _| window.refresh());
    cx.run_until_parked();
}

fn set_options(
    editor: &Entity<CodeEditor>,
    change: impl FnOnce(&mut EditorOptions),
    cx: &mut VisualTestContext,
) {
    editor.update(cx, |e, cx| {
        let mut options = e.options().clone();
        change(&mut options);
        e.update_options(options, cx);
    });
    paint(cx);
}

fn layout(editor: &Entity<CodeEditor>, cx: &mut VisualTestContext) -> LayoutInfo {
    paint(cx);
    editor.read_with(cx, |e, _| e.layout_info()).unwrap()
}

#[gpui::test]
fn a_zero_tab_size_is_one(cx: &mut TestAppContext) {
    // Monaco `TextModelResolvedOptions`: `Math.max(1, tabSize)`. A tab used
    // to divide by zero.
    let (editor, cx) = setup(
        || {
            TextModel::new("\tfoo\n\t\tbar").with_options(ModelOptions {
                tab_size: 0,
                indent_size: 0,
                detect_indentation: false,
                ..ModelOptions::default()
            })
        },
        cx,
    );
    let model = editor.read_with(cx, |e, _| e.model().clone());
    let options = model.read_with(cx, |m, _| m.options().clone());
    assert_eq!((options.tab_size, options.indent_size), (1, 1));
    // Typing, indenting, commenting and moving over tabs all work.
    cx.simulate_keystrokes("end tab");
    cx.simulate_input("x\t");
    cx.simulate_keystrokes("shift-tab down end left left");
    paint(cx);
    model.update(cx, |m, cx| {
        m.update_options(
            ModelOptions {
                tab_size: 0,
                indent_size: 0,
                ..m.options().clone()
            },
            cx,
        )
    });
    let options = model.read_with(cx, |m, _| m.options().clone());
    assert_eq!((options.tab_size, options.indent_size), (1, 1));
    editor.update(cx, |e, cx| {
        e.set_selections(&[Selection::collapsed(PointUtf16::new(1, 1))], cx);
    });
    cx.simulate_keystrokes("tab");
    paint(cx);
    assert!(model.read_with(cx, |m, _| m.text()).contains("bar"));
}

#[gpui::test]
fn indent_size_can_follow_the_tab_size(cx: &mut TestAppContext) {
    // Monaco: a model created with `detectIndentation` gets
    // `indentSize: 'tabSize'`, so a new tab size moves the indent size too.
    let (editor, cx) = setup(|| TextModel::new("fn a() {\n  b();\n}"), cx);
    let model = editor.read_with(cx, |e, _| e.model().clone());
    let options = model.read_with(cx, |m, _| m.options().clone());
    assert!(options.indent_size_is_tab_size);
    assert_eq!((options.tab_size, options.indent_size), (2, 2));
    model.update(cx, |m, cx| {
        m.update_options(
            ModelOptions {
                tab_size: 8,
                ..m.options().clone()
            },
            cx,
        )
    });
    let options = model.read_with(cx, |m, _| m.options().clone());
    assert_eq!((options.tab_size, options.indent_size), (8, 8));
    // A numeric indent size stays put.
    model.update(cx, |m, cx| {
        m.update_options(
            ModelOptions {
                indent_size: 4,
                indent_size_is_tab_size: false,
                ..m.options().clone()
            },
            cx,
        );
        m.update_options(
            ModelOptions {
                tab_size: 3,
                ..m.options().clone()
            },
            cx,
        );
    });
    let options = model.read_with(cx, |m, _| m.options().clone());
    assert_eq!((options.tab_size, options.indent_size), (3, 4));
    // `detectIndentation` sets a number, as Monaco's does.
    model.update(cx, |m, cx| m.detect_indentation(true, 4, cx));
    let options = model.read_with(cx, |m, _| m.options().clone());
    assert!(!options.indent_size_is_tab_size);
    assert_eq!((options.tab_size, options.indent_size), (2, 2));
}

#[gpui::test]
fn the_default_line_ending_is_platform_specific(cx: &mut TestAppContext) {
    let model = cx.new(|_| TextModel::new("one line"));
    let expected = if cfg!(windows) {
        LineEnding::Crlf
    } else {
        LineEnding::Lf
    };
    assert_eq!(ModelOptions::default().default_eol, expected);
    assert_eq!(
        model.read_with(cx, |m, _| m.buffer().line_ending()),
        expected
    );
}

#[gpui::test]
fn editors_render_with_validated_options(cx: &mut TestAppContext) {
    let (editor, cx) = setup(|| TextModel::new("a\nb"), cx);
    let default_line_height = EditorOptions::default().resolved_line_height();
    // Monaco `fontSize: 0` is the default size, and a `lineHeight` under 8
    // multiplies it.
    set_options(
        &editor,
        |o| {
            o.font_size = px(0.);
            o.line_height = Some(px(2.));
        },
        cx,
    );
    let font_size = f32::from(EditorOptions::default().font_size);
    // The glyph margin is a line height wide.
    assert_eq!(
        layout(&editor, cx).glyph_margin_width,
        px((font_size * 2.).round())
    );
    // The host gets its options back as it set them.
    editor.read_with(cx, |e, _| {
        assert_eq!(e.options().font_size, px(0.));
        assert_eq!(e.options().line_height, Some(px(2.)));
    });
    // A zero line height is automatic again, rather than dividing by zero.
    set_options(&editor, |o| o.line_height = Some(px(0.)), cx);
    assert_eq!(layout(&editor, cx).glyph_margin_width, default_line_height);
    // Out of range values are clamped.
    set_options(
        &editor,
        |o| {
            o.line_numbers_min_chars = 0;
            o.minimap.scale = 0.;
            o.mouse_wheel_scroll_sensitivity = 0.;
            o.sticky_scroll.max_line_count = 0;
        },
        cx,
    );
    let info = layout(&editor, cx);
    assert!(info.line_numbers_width > px(0.));
    assert_eq!(
        info.minimap.minimap_scale.max(1.),
        info.minimap.minimap_scale
    );
}

#[gpui::test]
fn gutter_widths_follow_compute_layout(cx: &mut TestAppContext) {
    let (editor, cx) = setup(|| TextModel::new("fn a() {\n    b();\n}"), cx);
    set_options(&editor, |o| o.font_size = px(12.), cx);
    let line_height = editor.read_with(cx, |e, _| e.options().resolved_line_height());
    let info = layout(&editor, cx);
    // Monaco: a line height of glyph margin, then `lineDecorationsWidth`
    // plus 16px of folding controls.
    assert_eq!(info.glyph_margin_width, line_height);
    assert_eq!(info.decorations_width, px(26.));
    // No folding column when the controls never show.
    set_options(
        &editor,
        |o| o.show_folding_controls = ShowFoldingControls::Never,
        cx,
    );
    assert_eq!(layout(&editor, cx).decorations_width, px(10.));
    set_options(
        &editor,
        |o| {
            o.show_folding_controls = ShowFoldingControls::Always;
            o.folding = false;
        },
        cx,
    );
    assert_eq!(layout(&editor, cx).decorations_width, px(10.));
    // `"2ch"`: two typical characters, truncated.
    set_options(
        &editor,
        |o| o.line_decorations_width = LineDecorationsWidth::Chars(2.),
        cx,
    );
    let info = layout(&editor, cx);
    assert!(info.decorations_width > px(10.) && info.decorations_width < px(20.));
    assert_eq!(info.decorations_width, info.decorations_width.floor());
    // The minimap takes its proportional share, with Monaco's 8px gutter.
    let info = layout(&editor, cx);
    assert!(info.minimap.minimap_width > px(8.));
    assert!(info.minimap.minimap_width <= px(120.));
    assert_eq!(
        info.content_left + info.content_width + info.minimap.minimap_width,
        info.width
    );
}

fn word_highlights(editor: &Entity<CodeEditor>, cx: &mut VisualTestContext) -> usize {
    let id = editor.entity_id();
    let color = editor.read_with(cx, |e, _| e.colors().word_highlight_background);
    let model = editor.read_with(cx, |e, _| e.model().clone());
    model.update(cx, |m, _| {
        m.all_decorations(Some(id))
            .iter()
            .filter(|d| d.owner == Some(id) && d.options.background == Some(color))
            .count()
    })
}

#[gpui::test]
fn occurrences_highlight_after_monaco_delays(cx: &mut TestAppContext) {
    let (editor, cx) = setup(
        || TextModel::new("foo bar foo").with_uri("file:///a.txt"),
        cx,
    );
    let move_to = |editor: &Entity<CodeEditor>, column, cx: &mut VisualTestContext| {
        editor.update(cx, |e, cx| {
            e.set_selections(&[Selection::collapsed(PointUtf16::new(0, column))], cx);
        });
    };
    let advance = |ms, cx: &mut VisualTestContext| {
        cx.executor().advance_clock(Duration::from_millis(ms));
        cx.run_until_parked();
    };
    // Monaco's default delay is 0: only the 50ms run delayer.
    assert_eq!(
        editor.read_with(cx, |e, _| e.options().occurrences_highlight_delay),
        Duration::ZERO
    );
    move_to(&editor, 1, cx);
    advance(40, cx);
    assert_eq!(word_highlights(&editor, cx), 0);
    advance(20, cx);
    assert_eq!(word_highlights(&editor, cx), 2);
    // A delay holds them back from the cursor move on.
    set_options(
        &editor,
        |o| o.occurrences_highlight_delay = Duration::from_millis(300),
        cx,
    );
    move_to(&editor, 5, cx);
    advance(200, cx);
    assert_eq!(word_highlights(&editor, cx), 0);
    advance(110, cx);
    assert_eq!(word_highlights(&editor, cx), 1);
    // Moving keeps restarting the 50ms wait.
    set_options(
        &editor,
        |o| o.occurrences_highlight_delay = Duration::ZERO,
        cx,
    );
    move_to(&editor, 9, cx);
    advance(30, cx);
    move_to(&editor, 10, cx);
    advance(30, cx);
    assert_eq!(word_highlights(&editor, cx), 0);
    advance(30, cx);
    assert_eq!(word_highlights(&editor, cx), 2);
}

#[gpui::test]
fn high_contrast_follows_the_os(cx: &mut TestAppContext) {
    let (editor, cx) = setup(|| TextModel::new("a"), cx);
    let theme = |editor: &Entity<CodeEditor>, cx: &mut VisualTestContext| {
        editor.read_with(cx, |e, _| e.theme().name.clone())
    };
    // Monaco's default theme.
    assert_eq!(theme(&editor, cx), "vs");
    assert!(cx.update(|_, cx| gpui_monaco::auto_detect_high_contrast(cx)));
    // Monaco `autoDetectHighContrast`: the OS's high contrast mode switches
    // to the high contrast theme, staying dark or light.
    cx.update(|_, cx| gpui_monaco::set_os_high_contrast(cx, true));
    assert_eq!(theme(&editor, cx), "hc-light");
    cx.update(|_, cx| gpui_monaco::set_os_high_contrast(cx, false));
    assert_eq!(theme(&editor, cx), "vs");
    cx.update(|_, cx| gpui_monaco::set_theme(cx, "vs-dark"));
    cx.update(|_, cx| gpui_monaco::set_os_high_contrast(cx, true));
    assert_eq!(theme(&editor, cx), "hc-black");
    // Off: the theme stays; turning it on again applies the OS state.
    cx.update(|_, cx| {
        gpui_monaco::set_os_high_contrast(cx, false);
        gpui_monaco::set_auto_detect_high_contrast(cx, false);
        gpui_monaco::set_os_high_contrast(cx, true);
    });
    assert_eq!(theme(&editor, cx), "vs-dark");
    cx.update(|_, cx| gpui_monaco::set_auto_detect_high_contrast(cx, true));
    assert_eq!(theme(&editor, cx), "hc-black");
}
