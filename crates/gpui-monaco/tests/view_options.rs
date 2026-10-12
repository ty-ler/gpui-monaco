//! View options (Monaco `editorOptions.ts`): padding, scrolling past the
//! last column, cursor surrounding lines, the wheel, cursor blinking.

use gpui::{
    Entity, Focusable, Modifiers, ScrollDelta, ScrollWheelEvent, TestAppContext, VisualTestContext,
    point, px,
};
use gpui_monaco::CodeEditor;
use gpui_monaco::options::{CursorBlinking, CursorSurroundingLinesStyle, EditorOptions};

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
        // Keep the reveal context to what each test sets.
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

fn metrics(editor: &Entity<CodeEditor>, cx: &mut VisualTestContext) -> (f32, f32, f32, f32) {
    paint(cx);
    let (top, max_left, height, line) =
        editor.read_with(cx, |e, _| e.view_metrics_for_tests().unwrap());
    (
        f32::from(top),
        f32::from(max_left),
        f32::from(height),
        f32::from(line),
    )
}

#[gpui::test]
fn padding_moves_the_first_line_down(cx: &mut TestAppContext) {
    let (editor, cx) = setup("one line", |o| o.padding.top = px(20.), cx);
    let (first_top, _) = editor.read_with(cx, |e, _| e.layout_for_tests().unwrap());
    assert_eq!(first_top, px(20.));
}

#[gpui::test]
fn scroll_beyond_last_column_widens_the_content(cx: &mut TestAppContext) {
    let long = "x".repeat(400);
    let (editor, cx) = setup(&long, |o| o.scroll_beyond_last_column = 0, cx);
    let (_, narrow, _, _) = metrics(&editor, cx);
    editor.update(cx, |e, cx| {
        let mut o = e.options().clone();
        o.scroll_beyond_last_column = 10;
        e.update_options(o, cx);
    });
    let (_, wide, _, _) = metrics(&editor, cx);
    // Ten columns more (monospace).
    assert!(
        wide > narrow + 50. && wide < narrow + 150.,
        "{narrow} {wide}"
    );
}

#[gpui::test]
fn cursor_surrounding_lines_keep_context(cx: &mut TestAppContext) {
    let text = (0..300)
        .map(|i| format!("line {i}"))
        .collect::<Vec<_>>()
        .join("\n");
    let (editor, cx) = setup(&text, |o| o.cursor_surrounding_lines = 3, cx);
    let (_, _, height, line) = metrics(&editor, cx);
    let visible = (height / line).floor() as u32;
    // Walk down past the bottom: the cursor keeps three lines below it.
    for _ in 0..visible + 5 {
        cx.simulate_keystrokes("down");
    }
    let (top, _, height, line) = metrics(&editor, cx);
    let row = editor.read_with(cx, |e, _| e.selections()[0].head.row) as f32;
    assert!(
        (row + 1. + 3.) * line <= top + height + 0.5,
        "row {row} top {top}"
    );
    // And above when going back up.
    for _ in 0..visible {
        cx.simulate_keystrokes("up");
    }
    let (top, _, _, line) = metrics(&editor, cx);
    let row = editor.read_with(cx, |e, _| e.selections()[0].head.row) as f32;
    assert!((row - 3.) * line >= top - 0.5, "row {row} top {top}");
    // The style's default leaves mouse moves alone.
    assert_eq!(
        editor.read_with(cx, |e, _| e.options().cursor_surrounding_lines_style),
        CursorSurroundingLinesStyle::Default
    );
}

#[gpui::test]
fn alt_scrolls_faster_along_the_main_axis(cx: &mut TestAppContext) {
    let text = (0..300)
        .map(|i| format!("line {i}"))
        .collect::<Vec<_>>()
        .join("\n");
    let (editor, cx) = setup(&text, |_| {}, cx);
    let wheel = |cx: &mut VisualTestContext, alt: bool| {
        cx.simulate_event(ScrollWheelEvent {
            position: point(px(100.), px(100.)),
            // Mostly vertical: the horizontal part is dropped.
            delta: ScrollDelta::Pixels(point(px(-3.), px(-10.))),
            modifiers: Modifiers {
                alt,
                ..Default::default()
            },
            ..Default::default()
        });
    };
    wheel(cx, false);
    let (top, _, _, _) = metrics(&editor, cx);
    assert_eq!(top, 10.);
    wheel(cx, true);
    let (top, _, _, _) = metrics(&editor, cx);
    assert_eq!(top, 10. + 50.);
    assert_eq!(editor.read_with(cx, |e, _| e.scroll_position().x), px(0.));
}

#[gpui::test]
fn solid_cursors_dont_blink(cx: &mut TestAppContext) {
    let (editor, cx) = setup("abc", |o| o.cursor_blinking = CursorBlinking::Solid, cx);
    for _ in 0..4 {
        cx.executor()
            .advance_clock(std::time::Duration::from_millis(500));
        cx.run_until_parked();
        assert!(editor.read_with(cx, |e, _| e.cursor_visible_for_tests()));
    }
    // Blink turns it off within a second.
    editor.update(cx, |e, cx| {
        let mut o = e.options().clone();
        o.cursor_blinking = CursorBlinking::Blink;
        e.update_options(o, cx);
    });
    cx.simulate_keystrokes("right");
    let mut hidden = false;
    for _ in 0..4 {
        cx.executor()
            .advance_clock(std::time::Duration::from_millis(300));
        cx.run_until_parked();
        hidden |= !editor.read_with(cx, |e, _| e.cursor_visible_for_tests());
    }
    assert!(hidden);
}

/// Monaco `ViewCursors.onConfigurationChanged` calls `_updateBlinking`:
/// turning the editor read-only (or the cursor solid) shows the cursor at
/// once and stops the blink.
#[gpui::test]
fn option_changes_reset_the_blink(cx: &mut TestAppContext) {
    let (editor, cx) = setup("abc", |_| {}, cx);
    let visible =
        |cx: &mut VisualTestContext| editor.read_with(cx, |e, _| e.cursor_visible_for_tests());
    let hide = |cx: &mut VisualTestContext| {
        for _ in 0..4 {
            cx.executor()
                .advance_clock(std::time::Duration::from_millis(250));
            cx.run_until_parked();
            if !visible(cx) {
                return;
            }
        }
        panic!("the cursor never blinked off");
    };
    let set = |f: fn(&mut EditorOptions), cx: &mut VisualTestContext| {
        editor.update(cx, |e, cx| {
            let mut o = e.options().clone();
            f(&mut o);
            e.update_options(o, cx);
        });
        cx.run_until_parked();
    };
    let stays_visible = |cx: &mut VisualTestContext| {
        assert!(visible(cx));
        for _ in 0..4 {
            cx.executor()
                .advance_clock(std::time::Duration::from_millis(500));
            cx.run_until_parked();
            assert!(visible(cx));
        }
    };
    hide(cx);
    set(|o| o.read_only = true, cx);
    stays_visible(cx);
    set(|o| o.read_only = false, cx);
    hide(cx);
    set(|o| o.cursor_blinking = CursorBlinking::Solid, cx);
    stays_visible(cx);
}

fn long_text() -> String {
    (0..500)
        .map(|i| format!("line {i}"))
        .collect::<Vec<_>>()
        .join("\n")
}

#[gpui::test]
fn scroll_by_page_pages_the_track(cx: &mut TestAppContext) {
    let (editor, cx) = setup(&long_text(), |o| o.scrollbar.scroll_by_page = true, cx);
    let (bar, thumb) = editor.read_with(cx, |e, _| e.vertical_scrollbar_for_tests().unwrap());
    let (_, _, height, _) = metrics(&editor, cx);
    cx.simulate_click(
        point(bar.center().x, thumb.bottom() + px(40.)),
        Modifiers::default(),
    );
    let (top, _, _, _) = metrics(&editor, cx);
    assert_eq!(top, height);
}

#[gpui::test]
fn arrows_scroll_wheel_steps(cx: &mut TestAppContext) {
    let (editor, cx) = setup(&long_text(), |o| o.scrollbar.vertical_has_arrows = true, cx);
    let (bar, thumb) = editor.read_with(cx, |e, _| e.vertical_scrollbar_for_tests().unwrap());
    // The track starts below the top arrow.
    assert!(thumb.top() >= bar.top() + px(11.));
    cx.simulate_click(
        point(bar.center().x, bar.bottom() - px(4.)),
        Modifiers::default(),
    );
    let (top, _, _, _) = metrics(&editor, cx);
    assert_eq!(top, 50.);
}

#[gpui::test]
fn minimap_side_and_size(cx: &mut TestAppContext) {
    use gpui_monaco::options::{MinimapSide, MinimapSize};
    let text = (0..20)
        .map(|i| format!("line {i}"))
        .collect::<Vec<_>>()
        .join("\n");
    let (editor, cx) = setup(&text, |o| o.minimap.side = MinimapSide::Left, cx);
    paint(cx);
    let (minimap, line, text_left) = editor.read_with(cx, |e, _| e.minimap_for_tests().unwrap());
    // Left of everything else.
    assert!(minimap.right() <= text_left);
    assert_eq!(line, px(2.));

    // Fill: a short document's lines grow to fill the height.
    editor.update(cx, |e, cx| {
        let mut o = e.options().clone();
        o.minimap.side = MinimapSide::Right;
        o.minimap.size = MinimapSize::Fill;
        e.update_options(o, cx);
    });
    paint(cx);
    let (minimap, line, text_left) = editor.read_with(cx, |e, _| e.minimap_for_tests().unwrap());
    assert!(minimap.left() > text_left);
    assert!(line > px(2.), "{line:?}");
    // Fit doesn't grow them.
    editor.update(cx, |e, cx| {
        let mut o = e.options().clone();
        o.minimap.size = MinimapSize::Fit;
        e.update_options(o, cx);
    });
    paint(cx);
    let (_, line, _) = editor.read_with(cx, |e, _| e.minimap_for_tests().unwrap());
    assert_eq!(line, px(2.));
}

#[gpui::test]
fn minimap_fit_samples_long_documents(cx: &mut TestAppContext) {
    use gpui_monaco::options::MinimapSize;
    let text = (0..20_000)
        .map(|i| format!("line {i}"))
        .collect::<Vec<_>>()
        .join("\n");
    let (editor, cx) = setup(&text, |o| o.minimap.size = MinimapSize::Fit, cx);
    paint(cx);
    let (minimap, line, _) = editor.read_with(cx, |e, _| e.minimap_for_tests().unwrap());
    // Every view line takes less than a pixel; the whole document fits.
    assert!(line < px(1.));
    assert!(line * 20_000. <= minimap.size.height * 1.5);
}
