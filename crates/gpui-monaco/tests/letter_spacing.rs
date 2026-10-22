//! Monaco `letterSpacing`: room after each character, in the text's glyph
//! positions, hit-testing, measured font widths and wrapping.

use gpui::{Entity, Focusable, Modifiers, Pixels, TestAppContext, VisualTestContext, point, px};
use gpui_monaco::CodeEditor;
use gpui_monaco::options::{EditorOptions, WordWrap};
use gpui_monaco::text::PointUtf16;

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

fn set_spacing(editor: &Entity<CodeEditor>, spacing: f32, cx: &mut VisualTestContext) {
    editor.update(cx, |e, cx| {
        e.update_options_with(|o| o.letter_spacing = px(spacing), cx)
    });
    paint(cx);
}

fn x_of(editor: &Entity<CodeEditor>, column: u32, cx: &mut VisualTestContext) -> Pixels {
    editor.read_with(cx, |e, _| {
        e.scrolled_visible_position(PointUtf16::new(0, column))
            .unwrap()
            .x
    })
}

fn close(a: Pixels, b: Pixels) -> bool {
    (f32::from(a) - f32::from(b)).abs() < 0.01
}

#[gpui::test]
fn spacing_moves_each_character_over(cx: &mut TestAppContext) {
    let (editor, cx) = setup("abcdef", |_| {}, cx);
    // From the text's start (the line numbers widen with the spacing too).
    let from_start = |editor: &Entity<CodeEditor>, column: u32, cx: &mut VisualTestContext| {
        x_of(editor, column, cx) - x_of(editor, 0, cx)
    };
    let plain: Vec<Pixels> = (0..=6)
        .map(|column| from_start(&editor, column, cx))
        .collect();
    set_spacing(&editor, 3., cx);
    for (column, plain) in plain.iter().enumerate() {
        let x = from_start(&editor, column as u32, cx);
        assert!(
            close(x, *plain + px(3.) * column as f32),
            "column {column}: {x:?}"
        );
    }
    // Negative spacing pulls them together.
    set_spacing(&editor, -1., cx);
    assert!(close(from_start(&editor, 4, cx), plain[4] - px(4.)));
}

#[gpui::test]
fn clicks_find_the_spaced_characters(cx: &mut TestAppContext) {
    let (editor, cx) = setup("abcdefghij", |o| o.letter_spacing = px(10.), cx);
    let at = editor.read_with(cx, |e, _| {
        e.scrolled_visible_position(PointUtf16::new(0, 7)).unwrap()
    });
    cx.simulate_click(point(at.x + px(1.), at.y + px(2.)), Modifiers::default());
    paint(cx);
    assert_eq!(
        editor.read_with(cx, |e, _| e.position()),
        PointUtf16::new(0, 7)
    );
}

#[gpui::test]
fn font_widths_include_the_spacing(cx: &mut TestAppContext) {
    let (editor, cx) = setup("x", |_| {}, cx);
    let plain = editor.read_with(cx, |e, _| e.font_info().unwrap());
    assert_eq!(plain.letter_spacing, px(0.));
    set_spacing(&editor, 2.5, cx);
    let spaced = editor.read_with(cx, |e, _| e.font_info().unwrap());
    assert_eq!(spaced.letter_spacing, px(2.5));
    assert_eq!(
        spaced.typical_halfwidth_character_width,
        plain.typical_halfwidth_character_width + px(2.5)
    );
    assert_eq!(spaced.space_width, plain.space_width + px(2.5));
    assert_eq!(spaced.max_digit_width, plain.max_digit_width + px(2.5));
    assert_eq!(spaced.is_monospace, plain.is_monospace);
}

#[gpui::test]
fn wider_characters_wrap_sooner(cx: &mut TestAppContext) {
    let long = "x".repeat(2000);
    let (editor, cx) = setup(&long, |o| o.word_wrap = WordWrap::On, cx);
    let view_lines = |editor: &Entity<CodeEditor>, cx: &mut VisualTestContext| {
        editor.read_with(cx, |e, _| {
            // Where the line's last character shows, from its first.
            let first = e.scrolled_visible_position(PointUtf16::new(0, 0)).unwrap();
            let last = e.scrolled_visible_position(PointUtf16::new(0, 1999));
            last.map(|last| last.y - first.y)
        })
    };
    let plain = view_lines(&editor, cx);
    set_spacing(&editor, 8., cx);
    let spaced = view_lines(&editor, cx);
    match (plain, spaced) {
        (Some(plain), Some(spaced)) => assert!(spaced > plain, "{plain:?} {spaced:?}"),
        // The last character scrolled out of view: it wrapped further down.
        (Some(_), None) => {}
        other => panic!("{other:?}"),
    }
}

#[test]
fn spacing_is_clamped() {
    let validated = |spacing: f32| {
        EditorOptions {
            letter_spacing: px(spacing),
            ..Default::default()
        }
        .validated()
        .letter_spacing
    };
    assert_eq!(validated(1.5), px(1.5));
    assert_eq!(validated(-10.), px(-5.));
    assert_eq!(validated(30.), px(20.));
    assert_eq!(validated(f32::NAN), px(0.));
}
