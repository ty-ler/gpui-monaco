//! Monaco's scroll commands (`EditorScroll`, `RevealLine` in
//! `browser/coreCommands.ts`) and smooth scrolling (`Scrollable`,
//! `SmoothScrollingOperation` in `base/common/scrollable.ts`) (#355).

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

use gpui::{AppContext as _, Entity, Focusable, Pixels, TestAppContext, VisualTestContext, px};
use gpui_monaco::{CodeEditor, CodeEditorEvent, ScrollType, TextModel};
use serde_json::json;

/// A smooth scroll ticks about once a frame.
const FRAME: Duration = Duration::from_millis(17);

fn setup<'a>(
    text: &str,
    smooth: bool,
    cx: &'a mut TestAppContext,
) -> (Entity<CodeEditor>, &'a mut VisualTestContext) {
    cx.update(gpui_monaco::init);
    let text = text.to_string();
    let (editor, cx) = cx.add_window_view(|window, cx| {
        let model = cx.new(|_| TextModel::new(&text));
        CodeEditor::new(model, window, cx)
    });
    cx.update(|window, cx| window.focus(&editor.focus_handle(cx), cx));
    let mut options = editor.read_with(cx, |e, _| e.options().clone());
    options.smooth_scrolling = smooth;
    options.sticky_scroll.enabled = false;
    editor.update(cx, |e, cx| e.update_options(options, cx));
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

/// Lets `duration` pass without drawing: only the smooth scroll's own
/// ticks move it.
fn advance(cx: &mut VisualTestContext, duration: Duration) {
    cx.executor().advance_clock(duration);
    cx.run_until_parked();
}

fn lines(count: usize) -> String {
    (0..count)
        .map(|i| format!("line {i}"))
        .collect::<Vec<_>>()
        .join("\n")
}

fn scroll_tops(
    editor: &Entity<CodeEditor>,
    cx: &mut VisualTestContext,
) -> Rc<RefCell<Vec<Pixels>>> {
    let tops = Rc::new(RefCell::new(Vec::new()));
    let sink = tops.clone();
    cx.update(|_, cx| {
        cx.subscribe(editor, move |_, event: &CodeEditorEvent, _| {
            if let CodeEditorEvent::ScrollChanged(scroll) = event
                && scroll.scroll_top_changed
            {
                sink.borrow_mut().push(scroll.scroll_top);
            }
        })
        .detach();
    });
    tops
}

fn line_height(editor: &Entity<CodeEditor>, cx: &mut VisualTestContext) -> Pixels {
    editor.read_with(cx, |e, _| e.options().resolved_line_height())
}

fn top(editor: &Entity<CodeEditor>, cx: &mut VisualTestContext) -> Pixels {
    editor.read_with(cx, |e, _| e.scroll_top())
}

fn future_top(editor: &Entity<CodeEditor>, cx: &mut VisualTestContext) -> Pixels {
    editor.read_with(cx, |e, _| e.future_scroll_position().y)
}

/// Monaco `getScrollTop` and `onDidScrollChange` follow a smooth scroll
/// step by step; `getFutureScrollPosition` is where it's going.
#[gpui::test]
fn smooth_scrolls_report_the_current_position(cx: &mut TestAppContext) {
    let (editor, cx) = setup(&lines(500), true, cx);
    let lh = line_height(&editor, cx);
    let tops = scroll_tops(&editor, cx);
    editor.update(cx, |e, cx| {
        e.set_scroll_top(lh * 20., ScrollType::Smooth, cx)
    });
    // At most the frame drawn right away has moved it (10ms in, as
    // Monaco's animation starts).
    let start = top(&editor, cx);
    assert!(start < lh * 20.);
    assert_eq!(future_top(&editor, cx), lh * 20.);
    assert!(editor.read_with(cx, |e, _| e.has_pending_scroll_animation()));
    // A step without a frame: further on the way, and reported.
    advance(cx, FRAME);
    let step = top(&editor, cx);
    assert!(step > start && step < lh * 20.);
    assert_eq!(tops.borrow().last(), Some(&step));
    // More steps, each further, until it's there.
    for _ in 0..10 {
        advance(cx, FRAME);
    }
    assert_eq!(top(&editor, cx), lh * 20.);
    assert!(!editor.read_with(cx, |e, _| e.has_pending_scroll_animation()));
    let tops = tops.borrow();
    assert!(tops.len() > 2);
    assert!(tops.windows(2).all(|pair| pair[0] < pair[1]));
    assert_eq!(tops.last(), Some(&(lh * 20.)));
}

/// Monaco `_initAnimation`: past 2.5 viewports, the first third of the
/// animation eases through 0.75 viewport, then the rest eases in from 0.75
/// viewport before the end.
#[gpui::test]
fn long_smooth_scrolls_skip_the_middle(cx: &mut TestAppContext) {
    let (editor, cx) = setup(&lines(2000), true, cx);
    let height = editor.read_with(cx, |e, _| e.layout_info().unwrap().height);
    editor.update(cx, |e, cx| e.scroll_to_edge(true, cx));
    let target = future_top(&editor, cx);
    assert!(target > height * 2.5);
    // 27ms of 135 (Monaco starts 10ms early): in the first part.
    advance(cx, FRAME);
    let first = top(&editor, cx);
    assert!(first > px(0.) && first <= height * 0.75);
    // 61ms: past the cut, within 0.75 viewport of the end.
    advance(cx, FRAME);
    advance(cx, FRAME);
    let second = top(&editor, cx);
    assert!(second >= target - height * 0.75 && second < target);
    advance(cx, Duration::from_millis(200));
    assert_eq!(top(&editor, cx), target);
}

/// Monaco `scrollEditorBottom`: line `lineCount - pageSize` at the top, not
/// the last line.
#[gpui::test]
fn scroll_editor_bottom_keeps_a_page(cx: &mut TestAppContext) {
    let (editor, cx) = setup(&lines(500), false, cx);
    let lh = line_height(&editor, cx);
    let height = editor.read_with(cx, |e, _| e.layout_info().unwrap().height);
    editor.update_in(cx, |e, window, cx| {
        e.trigger_with_payload("scrollEditorBottom", json!(null), window, cx)
            .unwrap()
    });
    draw(cx);
    let page_size = ((height / lh).floor() as u32).saturating_sub(2).max(1);
    let row = 500 - page_size - 1;
    let expected = editor.read_with(cx, |e, _| e.top_for_line_number(row, false).unwrap());
    assert_eq!(top(&editor, cx), expected);
    // The last line shows at the bottom, with `scrollBeyondLastLine` room
    // left below.
    let max = editor.read_with(cx, |e, _| e.scroll_height().unwrap()) - height;
    assert!(top(&editor, cx) < max);
    editor.update_in(cx, |e, window, cx| {
        e.trigger_with_payload("scrollEditorTop", json!(null), window, cx)
            .unwrap()
    });
    draw(cx);
    assert_eq!(top(&editor, cx), px(0.));
}

/// Monaco `_runVerticalEditorScroll` scrolls by pages and lines with
/// `ScrollType.Smooth`, from where the scroll is now.
#[gpui::test]
fn scroll_commands_are_smooth(cx: &mut TestAppContext) {
    let (editor, cx) = setup(&lines(500), true, cx);
    let lh = line_height(&editor, cx);
    let page = editor.read_with(cx, |e, _| {
        let height = e.layout_info().unwrap().height;
        ((height / lh).floor() as u32).saturating_sub(2).max(1) as f32
    });
    editor.update(cx, |e, cx| e.scroll_pages(1., cx));
    assert!(editor.read_with(cx, |e, _| e.has_pending_scroll_animation()));
    assert!(top(&editor, cx) < lh * page);
    assert_eq!(future_top(&editor, cx), lh * page);
    // Mid-way, another line counts from the current position (Monaco
    // `getCurrentScrollTop() + delta`).
    advance(cx, FRAME);
    let now = top(&editor, cx);
    assert!(now > px(0.) && now < lh * page);
    editor.update(cx, |e, cx| e.scroll_lines(1., cx));
    assert_eq!(future_top(&editor, cx), now + lh);
    advance(cx, Duration::from_millis(200));
    assert_eq!(top(&editor, cx), now + lh);

    // Columns too.
    editor.update(cx, |e, cx| {
        e.model()
            .update(cx, |m, cx| m.set_value(&"x".repeat(2000), cx))
    });
    draw(cx);
    editor.update_in(cx, |e, window, cx| {
        e.trigger_with_payload("scrollRight", json!(null), window, cx)
            .unwrap()
    });
    let left = editor.read_with(cx, |e, _| e.future_scroll_position().x);
    assert!(left > px(0.));
    assert!(editor.read_with(cx, |e, _| e.scroll_left()) < left);
    advance(cx, Duration::from_millis(200));
    assert_eq!(editor.read_with(cx, |e, _| e.scroll_left()), left);
}

/// Monaco `RevealLine` passes `revealHorizontal = false`.
#[gpui::test]
fn reveal_line_command_is_vertical(cx: &mut TestAppContext) {
    let long = format!("{}\n", "x".repeat(300)).repeat(200);
    let (editor, cx) = setup(&long, false, cx);
    editor.update(cx, |e, cx| {
        e.set_scroll_left(px(400.), ScrollType::Immediate, cx)
    });
    draw(cx);
    editor.update_in(cx, |e, window, cx| {
        e.trigger_with_payload("revealLine", json!({ "lineNumber": 150 }), window, cx)
            .unwrap()
    });
    draw(cx);
    assert!(top(&editor, cx) > px(0.));
    assert_eq!(editor.read_with(cx, |e, _| e.scroll_left()), px(400.));
}

/// A host mirroring the scroll follows a smooth scroll step by step: the
/// diff editor's original side keeps up with the modified one (Monaco's
/// `onDidScrollChange` with the current position).
#[gpui::test]
fn diff_editor_sides_follow_a_smooth_scroll(cx: &mut TestAppContext) {
    cx.update(gpui_monaco::init);
    let text = lines(300);
    let (diff, cx) = cx.add_window_view(|window, cx| {
        let original = cx.new(|_| TextModel::new(&text));
        let modified = cx.new(|_| TextModel::new(&format!("{text}\nmore")));
        gpui_monaco::DiffEditor::new(original, modified, window, cx)
    });
    draw(cx);
    let (original, modified) = diff.read_with(cx, |d, _| {
        (d.original_editor().clone(), d.modified_editor().clone())
    });
    let mut options = modified.read_with(cx, |e, _| e.options().clone());
    options.smooth_scrolling = true;
    modified.update(cx, |e, cx| e.update_options(options, cx));
    draw(cx);
    let lh = line_height(&modified, cx);
    modified.update(cx, |e, cx| {
        e.set_scroll_top(lh * 50., ScrollType::Smooth, cx)
    });
    advance(cx, FRAME);
    let step = top(&modified, cx);
    assert!(step > px(0.) && step < lh * 50.);
    assert_eq!(top(&original, cx), step);
    advance(cx, Duration::from_millis(200));
    assert_eq!(top(&modified, cx), lh * 50.);
    assert_eq!(top(&original, cx), lh * 50.);
}
