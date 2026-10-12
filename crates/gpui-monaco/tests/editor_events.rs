//! Monaco's `ICodeEditor` events, geometry queries, `ScrollType`,
//! `revealAllCursors` and view state (#205).

use std::cell::RefCell;
use std::rc::Rc;

use gpui::{
    AppContext as _, Entity, EntityInputHandler as _, Focusable, Modifiers, MouseButton,
    ScrollDelta, ScrollWheelEvent, TestAppContext, VisualTestContext, point, px, size,
};
use gpui_monaco::cursor::Selection;
use gpui_monaco::text::PointUtf16;
use gpui_monaco::{
    CodeEditor, CodeEditorEvent, EditorOptions, LayoutInfo, ModelChange, RevealType, ScrollType,
    TextModel, ViewZone, ZoneHeight,
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
        let model = cx.new(|_| TextModel::new(&text).with_uri("file:///a.txt"));
        CodeEditor::new(model, window, cx)
    });
    cx.update(|window, cx| window.focus(&editor.focus_handle(cx), cx));
    cx.run_until_parked();
    (editor, cx)
}

fn draw(cx: &mut VisualTestContext) {
    cx.update(|window, cx| {
        window.refresh();
        window.draw(cx).clear(cx);
    });
    cx.run_until_parked();
}

/// changes, and mouse moves.
/// mouse moves.
fn record(
    editor: &Entity<CodeEditor>,
    cx: &mut VisualTestContext,
) -> Rc<RefCell<Vec<CodeEditorEvent>>> {
    let events = Rc::new(RefCell::new(Vec::new()));
    let sink = events.clone();
    cx.update(|_, cx| {
        cx.subscribe(editor, move |_, event: &CodeEditorEvent, _| {
            if !matches!(
                event,
                CodeEditorEvent::SelectionsChanged(_)
                    | CodeEditorEvent::CursorPositionChanged(_)
                    | CodeEditorEvent::ScrollChanged(_)
                    | CodeEditorEvent::ModelContentChanged(_)
                    | CodeEditorEvent::ModelDecorationsChanged(_)
                    | CodeEditorEvent::ModelTokensChanged { .. }
                    | CodeEditorEvent::MouseMove(_)
            ) {
                sink.borrow_mut().push(event.clone());
            }
        })
        .detach();
    });
    events
}

fn lines(count: usize) -> String {
    (0..count)
        .map(|i| format!("line {i}"))
        .collect::<Vec<_>>()
        .join("\n")
}

#[gpui::test]
fn model_language_and_configuration_events(cx: &mut TestAppContext) {
    let (editor, cx) = setup("hello", cx);
    let events = record(&editor, cx);

    // Monaco `onWillChangeModel`, then `onDidChangeModel`, with the uris,
    // in an update (`onBeginUpdate`/`onEndUpdate`).
    let other = cx.update(|_, cx| cx.new(|_| TextModel::new("other").with_uri("file:///b.txt")));
    editor.update(cx, |e, cx| e.set_model(other.clone(), cx));
    let change = ModelChange {
        old_uri: Some("file:///a.txt".into()),
        new_uri: Some("file:///b.txt".into()),
    };
    assert_eq!(
        events.borrow().as_slice(),
        [
            CodeEditorEvent::BeginUpdate,
            CodeEditorEvent::WillChangeModel(change.clone()),
            CodeEditorEvent::ModelChanged(change),
            CodeEditorEvent::EndUpdate,
        ]
    );
    // The same model again: nothing but the update, as in Monaco.
    events.borrow_mut().clear();
    editor.update(cx, |e, cx| e.set_model(other.clone(), cx));
    assert_eq!(
        events.borrow().as_slice(),
        [CodeEditorEvent::BeginUpdate, CodeEditorEvent::EndUpdate]
    );

    // The model's language change reaches the editor.
    other.update(cx, |m, cx| m.set_language(Some("rust".into()), cx));
    assert!(
        events
            .borrow()
            .contains(&CodeEditorEvent::ModelLanguageChanged {
                old_language: None,
                new_language: Some("rust".into()),
            })
    );

    // `onDidChangeConfiguration` says which options changed.
    events.borrow_mut().clear();
    let mut options = editor.read_with(cx, |e, _| e.options().clone());
    options.glyph_margin = !options.glyph_margin;
    editor.update(cx, |e, cx| e.update_options(options.clone(), cx));
    let change = events
        .borrow()
        .iter()
        .find_map(|e| match e {
            CodeEditorEvent::ConfigurationChanged(change) => Some(change.clone()),
            _ => None,
        })
        .expect("no ConfigurationChanged");
    assert!(change.has_changed(|o| &o.glyph_margin));
    assert!(!change.has_changed(|o| &o.font_size));
    assert_eq!(change.new_options().glyph_margin, options.glyph_margin);
    // Unchanged options: no event.
    events.borrow_mut().clear();
    editor.update(cx, |e, cx| e.update_options(options, cx));
    assert!(
        !events
            .borrow()
            .iter()
            .any(|e| matches!(e, CodeEditorEvent::ConfigurationChanged(_)))
    );
}

#[gpui::test]
fn layout_and_content_size_events(cx: &mut TestAppContext) {
    let (editor, cx) = setup("short", cx);
    let events = record(&editor, cx);
    let layouts = |events: &Rc<RefCell<Vec<CodeEditorEvent>>>| -> Vec<LayoutInfo> {
        events
            .borrow()
            .iter()
            .filter_map(|e| match e {
                CodeEditorEvent::LayoutChanged(info) => Some(*info),
                _ => None,
            })
            .collect()
    };
    // Repainting without changes reports nothing.
    draw(cx);
    assert!(layouts(&events).is_empty());

    // The layout info matches the editor's parts.
    let info = editor.read_with(cx, |e, _| e.layout_info()).unwrap();
    let options = editor.read_with(cx, |e, _| e.options().clone());
    assert_eq!(info.glyph_margin_left, px(0.));
    assert_eq!(info.line_numbers_left, info.glyph_margin_width);
    assert_eq!(
        info.decorations_left,
        info.line_numbers_left + info.line_numbers_width
    );
    assert_eq!(
        info.content_left,
        info.decorations_left + info.decorations_width
    );
    assert_eq!(
        info.vertical_scrollbar_width,
        options.vertical_scrollbar_size
    );
    assert_eq!(
        info.horizontal_scrollbar_height,
        options.horizontal_scrollbar_size
    );
    // The content (with the scrollbar over it) and the minimap fill the
    // rest.
    assert_eq!(
        info.content_left + info.content_width + info.minimap.minimap_width,
        info.width
    );
    assert_eq!(info.overview_ruler.width, options.vertical_scrollbar_size);
    assert_eq!(info.overview_ruler.height, info.height);
    assert!(!info.is_viewport_wrapping);
    assert_eq!(info.wrapping_column, None);

    // Turning the glyph margin on (it's off by default, as in standalone
    // Monaco) moves everything right.
    assert_eq!(info.glyph_margin_width, px(0.));
    let mut glyphs = options.clone();
    glyphs.glyph_margin = true;
    editor.update(cx, |e, cx| e.update_options(glyphs, cx));
    draw(cx);
    let changed = layouts(&events);
    assert_eq!(changed.len(), 1);
    assert!(changed[0].glyph_margin_width > px(0.));
    assert_eq!(changed[0].line_numbers_left, changed[0].glyph_margin_width);
    assert!(changed[0].content_left > info.content_left);

    // Resizing the window reports the new size.
    events.borrow_mut().clear();
    cx.simulate_resize(size(px(500.), px(300.)));
    draw(cx);
    let resized = layouts(&events);
    assert_eq!(
        resized.last().map(|i| (i.width, i.height)),
        Some((px(500.), px(300.)))
    );

    // A longer line and more lines grow the content.
    events.borrow_mut().clear();
    let (width, height) = editor.read_with(cx, |e, _| {
        (e.content_width().unwrap(), e.content_height().unwrap())
    });
    editor.update(cx, |e, cx| {
        e.type_text(&format!("{}\nmore", "x".repeat(40)), cx)
    });
    draw(cx);
    let size_change = events
        .borrow()
        .iter()
        .find_map(|e| match e {
            CodeEditorEvent::ContentSizeChanged(change) => Some(*change),
            _ => None,
        })
        .expect("no ContentSizeChanged");
    assert!(size_change.content_width_changed && size_change.content_height_changed);
    assert!(size_change.content_width > width);
    assert!(size_change.content_height > height);
    assert_eq!(
        editor.read_with(cx, |e, _| e.content_height()),
        Some(size_change.content_height)
    );
}

#[gpui::test]
fn typing_clipboard_and_key_events(cx: &mut TestAppContext) {
    let (editor, cx) = setup("hello", cx);
    let model = editor.read_with(cx, |e, _| e.model().clone());
    model.update(cx, |m, cx| m.set_language(Some("plaintext".into()), cx));
    let events = record(&editor, cx);
    let keys_and_typing = |events: &Rc<RefCell<Vec<CodeEditorEvent>>>| -> Vec<String> {
        events
            .borrow()
            .iter()
            .filter_map(|e| match e {
                CodeEditorEvent::KeyDown(k) if !k.key.is_empty() => Some(format!("down {}", k.key)),
                CodeEditorEvent::KeyUp(k) => Some(format!("up {}", k.key)),
                CodeEditorEvent::WillType(text) => Some(format!("will type {text}")),
                CodeEditorEvent::DidType(text) => Some(format!("did type {text}")),
                _ => None,
            })
            .collect()
    };

    // Typing: key down (before bindings), will type, did type.
    cx.simulate_input("x");
    assert_eq!(
        keys_and_typing(&events),
        ["down x", "will type x", "did type x"]
    );
    // Bound keys reach `KeyDown` too.
    events.borrow_mut().clear();
    cx.simulate_keystrokes("left");
    assert_eq!(keys_and_typing(&events), ["down left"]);
    events.borrow_mut().clear();
    cx.simulate_event(gpui::KeyUpEvent {
        keystroke: gpui::Keystroke::parse("left").unwrap(),
    });
    assert_eq!(keys_and_typing(&events), ["up left"]);
    let only_clipboard = |events: &Rc<RefCell<Vec<CodeEditorEvent>>>| {
        events.borrow_mut().retain(|e| {
            matches!(
                e,
                CodeEditorEvent::WillCopy(_)
                    | CodeEditorEvent::WillCut(_)
                    | CodeEditorEvent::WillPaste(_)
                    | CodeEditorEvent::DidPaste(_)
            )
        })
    };

    // Copy and cut announce what goes to the clipboard.
    editor.update(cx, |e, cx| {
        e.set_selections(&[Selection::new(p(0, 0), p(0, 2))], cx)
    });
    events.borrow_mut().clear();
    editor.update_in(cx, |e, window, cx| {
        e.trigger("code_editor::Copy", window, cx).unwrap()
    });
    only_clipboard(&events);
    let copy = match events.borrow().as_slice() {
        [CodeEditorEvent::WillCopy(copy)] => copy.clone(),
        other => panic!("{other:?}"),
    };
    assert_eq!((copy.is_cut, copy.text.as_str()), (false, "xh"));
    assert_eq!(copy.language_id.as_deref(), Some("plaintext"));
    events.borrow_mut().clear();
    editor.update_in(cx, |e, window, cx| {
        e.trigger("code_editor::Cut", window, cx).unwrap()
    });
    only_clipboard(&events);
    assert!(matches!(
        events.borrow().as_slice(),
        [CodeEditorEvent::WillCut(cut)] if cut.is_cut && cut.text == "xh"
    ));

    // Pasting: what's on the clipboard, then the pasted range (from the
    // primary selection's start before to after) and the copied language.
    events.borrow_mut().clear();
    editor.update(cx, |e, cx| {
        e.set_selections(&[Selection::collapsed(p(0, 1))], cx)
    });
    editor.update_in(cx, |e, window, cx| {
        e.trigger("code_editor::Paste", window, cx).unwrap()
    });
    only_clipboard(&events);
    let events = events.borrow();
    let [
        CodeEditorEvent::WillPaste(will),
        CodeEditorEvent::DidPaste(did),
    ] = events.as_slice()
    else {
        panic!("{events:?}");
    };
    assert_eq!(will.text, "xh");
    assert_eq!(will.language_id.as_deref(), Some("plaintext"));
    assert_eq!(did.range, p(0, 1)..p(0, 3));
    assert_eq!(did.language_id.as_deref(), Some("plaintext"));
    assert_eq!(model.read_with(cx, |m, _| m.text()), "exhllo");
}

#[gpui::test]
fn mouse_wheel_drag_and_drop_events(cx: &mut TestAppContext) {
    let (editor, cx) = setup("one two three", cx);
    draw(cx);
    let events = record(&editor, cx);
    let at = |cx: &mut VisualTestContext, column| {
        draw(cx);
        editor
            .read_with(cx, |e, _| e.scrolled_visible_position(p(0, column)))
            .unwrap()
            + point(px(1.), px(5.))
    };

    let start = at(cx, 5);
    cx.simulate_event(ScrollWheelEvent {
        position: start,
        delta: ScrollDelta::Lines(point(0., 2.)),
        ..Default::default()
    });
    let line_height = editor.read_with(cx, |e, _| e.options().resolved_line_height());
    assert!(matches!(
        events.borrow().first(),
        Some(CodeEditorEvent::MouseWheel(wheel))
            if wheel.lines == Some(point(0., 2.)) && wheel.delta.y == line_height * 2.
    ));

    // Dragging the selection: drag events, then the drop.
    editor.update(cx, |e, cx| {
        e.set_selections(&[Selection::new(p(0, 4), p(0, 8))], cx)
    });
    let end = at(cx, 13);
    events.borrow_mut().clear();
    cx.simulate_mouse_down(start, MouseButton::Left, Modifiers::default());
    cx.simulate_mouse_move(
        point(start.x + px(2.), start.y),
        MouseButton::Left,
        Modifiers::default(),
    );
    cx.simulate_mouse_move(end, MouseButton::Left, Modifiers::default());
    cx.simulate_mouse_up(end, MouseButton::Left, Modifiers::default());
    let kinds: Vec<&str> = events
        .borrow()
        .iter()
        .filter_map(|e| match e {
            CodeEditorEvent::MouseDrag(_) => Some("drag"),
            CodeEditorEvent::MouseDrop(_) => Some("drop"),
            CodeEditorEvent::MouseDropCanceled => Some("canceled"),
            _ => None,
        })
        .collect();
    assert_eq!(kinds, ["drag", "drag", "drop"]);
    let drop = events.borrow().iter().find_map(|e| match e {
        CodeEditorEvent::MouseDrop(mouse) => Some(mouse.target.position),
        _ => None,
    });
    assert_eq!(drop, Some(Some(p(0, 13))));

    // Escape while dragging cancels the drop.
    editor.update(cx, |e, cx| {
        e.set_selections(&[Selection::new(p(0, 0), p(0, 3))], cx)
    });
    let start = at(cx, 1);
    events.borrow_mut().clear();
    cx.simulate_mouse_down(start, MouseButton::Left, Modifiers::default());
    cx.simulate_mouse_move(
        point(start.x + px(20.), start.y),
        MouseButton::Left,
        Modifiers::default(),
    );
    cx.simulate_keystrokes("escape");
    assert!(
        events
            .borrow()
            .contains(&CodeEditorEvent::MouseDropCanceled)
    );
}

#[gpui::test]
fn widget_focus_events(cx: &mut TestAppContext) {
    cx.update(gpui_monaco::init);
    let (editor, cx) =
        cx.add_window_view(|window, cx| CodeEditor::with_text("find me", window, cx));
    // Focus events need an active window.
    cx.update(|window, _| window.activate_window());
    draw(cx);
    let events = record(&editor, cx);
    let focus = |events: &Rc<RefCell<Vec<CodeEditorEvent>>>| -> Vec<&'static str> {
        events
            .borrow()
            .iter()
            .filter_map(|e| match e {
                CodeEditorEvent::Focused => Some("text focus"),
                CodeEditorEvent::Blurred => Some("text blur"),
                CodeEditorEvent::WidgetFocused => Some("widget focus"),
                CodeEditorEvent::WidgetBlurred => Some("widget blur"),
                _ => None,
            })
            .collect()
    };
    cx.update(|window, cx| window.focus(&editor.focus_handle(cx), cx));
    draw(cx);
    assert_eq!(focus(&events), ["text focus", "widget focus"]);
    assert!(editor.update_in(cx, |e, window, cx| e.has_widget_focus(window, cx)));

    // The find widget's input: the text loses focus, the widget keeps it.
    events.borrow_mut().clear();
    cx.simulate_keystrokes("secondary-f");
    draw(cx);
    assert_eq!(focus(&events), ["text blur"]);
    assert!(
        editor.update_in(cx, |e, window, cx| e.has_widget_focus(window, cx)
            && !e.has_text_focus(window))
    );

    // Focus leaving the editor.
    events.borrow_mut().clear();
    cx.update(|window, cx| window.blur(cx));
    draw(cx);
    assert_eq!(focus(&events), ["widget blur"]);
}

#[gpui::test]
fn vertical_geometry(cx: &mut TestAppContext) {
    let (editor, cx) = setup(&lines(50), cx);
    draw(cx);
    let lh = editor.read_with(cx, |e, _| e.options().resolved_line_height());
    let top = |cx: &mut VisualTestContext, row, zones| {
        editor
            .read_with(cx, |e, _| e.top_for_line_number(row, zones))
            .unwrap()
    };
    assert_eq!(top(cx, 0, false), px(0.));
    assert_eq!(top(cx, 10, false), lh * 10.);
    assert_eq!(
        editor.read_with(cx, |e, _| e.bottom_for_line_number(10, false)),
        Some(lh * 11.)
    );

    // A 50px view zone after row 2 (Monaco line 3).
    let view = cx.update(|_, cx| cx.new(|_| gpui::Empty).into());
    editor.update(cx, |e, cx| {
        e.add_view_zone(ViewZone::new(3, ZoneHeight::Pixels(px(50.)), view), cx);
    });
    draw(cx);
    // Monaco `getTopForLineNumber(4)` counts the zone above; with
    // `includeViewZones` it's the zone's top.
    assert_eq!(top(cx, 3, false), lh * 3. + px(50.));
    assert_eq!(top(cx, 3, true), lh * 3.);
    assert_eq!(
        editor.read_with(cx, |e, _| e.bottom_for_line_number(2, true)),
        Some(lh * 3. + px(50.))
    );
    assert_eq!(
        editor.read_with(cx, |e, _| e.top_for_position(p(3, 2))),
        Some(lh * 3. + px(50.))
    );
    assert_eq!(
        editor.read_with(cx, |e, _| e.line_height_for_position(p(3, 0))),
        Some(lh)
    );

    // The scroll height counts the zone and scrolling beyond the last line.
    let (viewport, scroll_height, content_height) = editor.read_with(cx, |e, _| {
        (
            e.layout_info().unwrap().height,
            e.scroll_height().unwrap(),
            e.content_height().unwrap(),
        )
    });
    assert_eq!(content_height, lh * 50. + px(50.));
    assert_eq!(scroll_height, content_height + viewport - lh);
}

#[gpui::test]
fn horizontal_geometry(cx: &mut TestAppContext) {
    let (editor, cx) = setup("\tab\nxy", cx);
    draw(cx);
    // Tabs expand to the tab size (Monaco's 1-based result minus one).
    assert_eq!(
        editor.read_with(cx, |e, cx| e.visible_column_from_position(p(0, 1), cx)),
        4
    );
    assert_eq!(
        editor.read_with(cx, |e, cx| e.visible_column_from_position(p(0, 2), cx)),
        5
    );
    // Offsets are from the line's start, as `scrolled_visible_position`
    // puts them after the content's left edge.
    let (offset, visible, info) = editor.read_with(cx, |e, _| {
        (
            e.offset_for_column(1, 1).unwrap(),
            e.scrolled_visible_position(p(1, 1)).unwrap(),
            e.layout_info().unwrap(),
        )
    });
    assert!(offset > px(0.));
    // "xy" is two of the offsets wide.
    assert_eq!(
        editor.read_with(cx, |e, _| e.width_of_line(1)),
        Some(offset * 2.)
    );
    assert_eq!(visible.x, info.content_left + offset);
    // The content width counts the vertical scrollbar; the scroll width is
    // at least the text area's.
    let (content, scroll) = editor.read_with(cx, |e, _| {
        (e.content_width().unwrap(), e.scroll_width().unwrap())
    });
    assert!(content > info.vertical_scrollbar_width);
    assert_eq!(scroll, content.max(info.content_width));
}

#[gpui::test]
fn scroll_types(cx: &mut TestAppContext) {
    let (editor, cx) = setup(&lines(500), cx);
    let mut options = editor.read_with(cx, |e, _| e.options().clone());
    options.smooth_scrolling = true;
    editor.update(cx, |e, cx| e.update_options(options, cx));
    draw(cx);
    let lh = editor.read_with(cx, |e, _| e.options().resolved_line_height());
    let shown = |cx: &mut VisualTestContext| {
        editor.read_with(cx, |e, _| e.view_metrics_for_tests().unwrap().0)
    };

    // Reveals default to smooth: the frame shows a point on the way.
    editor.update(cx, |e, cx| e.reveal_line_in_center(250, cx));
    draw(cx);
    assert!(editor.read_with(cx, |e, _| e.has_pending_scroll_animation()));
    let target = editor.read_with(cx, |e, _| e.future_scroll_position().y);
    assert!(shown(cx) < target);
    // `scroll_top` is where the scroll is now, as shown (Monaco
    // `getScrollTop`).
    assert_eq!(editor.read_with(cx, |e, _| e.scroll_top()), shown(cx));

    // An immediate reveal jumps.
    editor.update(cx, |e, cx| {
        e.reveal_range_with_scroll_type(
            p(400, 0)..p(400, 0),
            RevealType::Top,
            ScrollType::Immediate,
            cx,
        )
    });
    draw(cx);
    assert!(!editor.read_with(cx, |e, _| e.has_pending_scroll_animation()));
    assert_eq!(shown(cx), editor.read_with(cx, |e, _| e.scroll_top()));

    // `set_scroll_top` defaults to immediate in Monaco; smooth on request.
    editor.update(cx, |e, cx| {
        e.set_scroll_top(lh * 10., ScrollType::Immediate, cx)
    });
    draw(cx);
    assert_eq!(shown(cx), lh * 10.);
    editor.update(cx, |e, cx| {
        e.set_scroll_top(lh * 100., ScrollType::Smooth, cx)
    });
    draw(cx);
    assert!(shown(cx) < lh * 100.);
    assert_eq!(editor.read_with(cx, |e, _| e.scroll_top()), shown(cx));
    assert_eq!(
        editor.read_with(cx, |e, _| e.future_scroll_position().y),
        lh * 100.
    );

    // A reveal moving a line or less jumps even when smooth (Monaco
    // `onRevealRangeRequest`). Row 8 at the top keeps the five sticky
    // scroll lines above it: a scroll top of three lines.
    editor.update(cx, |e, cx| {
        e.set_scroll_top(lh * 4., ScrollType::Immediate, cx)
    });
    draw(cx);
    editor.update(cx, |e, cx| {
        e.reveal_lines_with_scroll_type(8, 8, RevealType::Top, ScrollType::Smooth, cx)
    });
    draw(cx);
    assert_eq!(shown(cx), lh * 3.);
    assert!(!editor.read_with(cx, |e, _| e.has_pending_scroll_animation()));

    editor.update(cx, |e, cx| {
        e.set_scroll_left(px(5.), ScrollType::Immediate, cx)
    });
    assert_eq!(editor.read_with(cx, |e, _| e.scroll_left()), px(0.));
}

/// Monaco's `SmoothScrollingOperation` animates `scrollLeft` as it does
/// `scrollTop`.
#[gpui::test]
fn horizontal_smooth_scrolling(cx: &mut TestAppContext) {
    let long = format!("{}\n", "x".repeat(300)).repeat(50);
    let (editor, cx) = setup(&long, cx);
    let mut options = editor.read_with(cx, |e, _| e.options().clone());
    options.smooth_scrolling = true;
    editor.update(cx, |e, cx| e.update_options(options, cx));
    draw(cx);
    let shown = |cx: &mut VisualTestContext| {
        editor.read_with(cx, |e, _| e.shown_scroll_position_for_tests().unwrap())
    };

    // Smooth: the frame shows a point on the way.
    editor.update(cx, |e, cx| {
        e.set_scroll_left(px(400.), ScrollType::Smooth, cx)
    });
    draw(cx);
    assert!(editor.read_with(cx, |e, _| e.has_pending_scroll_animation()));
    assert_eq!(
        editor.read_with(cx, |e, _| e.future_scroll_position().x),
        px(400.)
    );
    assert_eq!(editor.read_with(cx, |e, _| e.scroll_left()), shown(cx).x);
    assert!(shown(cx).x < px(400.));
    assert_eq!(shown(cx).y, px(0.));

    // Immediate jumps.
    editor.update(cx, |e, cx| {
        e.set_scroll_left(px(100.), ScrollType::Immediate, cx)
    });
    draw(cx);
    assert!(!editor.read_with(cx, |e, _| e.has_pending_scroll_animation()));
    assert_eq!(shown(cx).x, px(100.));

    // Without `smooth_scrolling`, smooth requests jump too.
    let mut options = editor.read_with(cx, |e, _| e.options().clone());
    options.smooth_scrolling = false;
    editor.update(cx, |e, cx| e.update_options(options, cx));
    editor.update(cx, |e, cx| {
        e.set_scroll_left(px(300.), ScrollType::Smooth, cx)
    });
    draw(cx);
    assert_eq!(shown(cx).x, px(300.));
}

#[gpui::test]
fn reveal_lines_leave_horizontal_scroll(cx: &mut TestAppContext) {
    let long = format!("{}\n", "x".repeat(300)).repeat(200);
    let (editor, cx) = setup(&long, cx);
    draw(cx);
    editor.update(cx, |e, cx| {
        e.set_scroll_left(px(400.), ScrollType::Immediate, cx)
    });
    // Monaco `revealLine` doesn't scroll horizontally…
    editor.update(cx, |e, cx| e.reveal_line(150, cx));
    draw(cx);
    assert_eq!(editor.read_with(cx, |e, _| e.scroll_left()), px(400.));
    // …`revealPosition` does…
    editor.update(cx, |e, cx| e.reveal_position(p(150, 0), cx));
    draw(cx);
    assert_eq!(editor.read_with(cx, |e, _| e.scroll_left()), px(0.));
    // …and a range over several lines scrolls to their start.
    editor.update(cx, |e, cx| {
        e.set_scroll_left(px(400.), ScrollType::Immediate, cx)
    });
    editor.update(cx, |e, cx| {
        e.reveal_range(p(10, 250)..p(11, 250), RevealType::Simple, cx)
    });
    draw(cx);
    assert_eq!(editor.read_with(cx, |e, _| e.scroll_left()), px(0.));
}

#[gpui::test]
fn reveal_all_cursors(cx: &mut TestAppContext) {
    let (editor, cx) = setup(&lines(500), cx);
    draw(cx);
    let visible = |cx: &mut VisualTestContext| {
        let ranges = editor.read_with(cx, |e, cx| e.visible_ranges(cx));
        (ranges[0].start.row, ranges.last().unwrap().end.row)
    };
    let select = |cx: &mut VisualTestContext, selections: &[Selection]| {
        editor.update(cx, |e, cx| e.set_selections(selections, cx));
        draw(cx);
        editor.update(cx, |e, cx| {
            e.set_scroll_top(px(0.), ScrollType::Immediate, cx)
        });
        draw(cx);
    };
    // Cursors on rows 200 and 205: both show.
    select(
        cx,
        &[
            Selection::collapsed(p(200, 0)),
            Selection::collapsed(p(205, 0)),
        ],
    );
    assert_eq!(visible(cx).0, 0);
    editor.update(cx, |e, cx| e.reveal_all_cursors(true, false, cx));
    draw(cx);
    let (first, last) = visible(cx);
    assert!(first <= 200 && 205 <= last, "{first}..{last}");

    // Cursors further apart than the viewport: no scrolling.
    select(
        cx,
        &[
            Selection::collapsed(p(10, 0)),
            Selection::collapsed(p(400, 0)),
        ],
    );
    editor.update(cx, |e, cx| e.reveal_all_cursors(true, false, cx));
    draw(cx);
    assert_eq!(editor.read_with(cx, |e, _| e.scroll_top()), px(0.));

    // A single cursor is revealed like `revealPosition`.
    select(cx, &[Selection::collapsed(p(300, 0))]);
    editor.update(cx, |e, cx| e.reveal_all_cursors(false, true, cx));
    draw(cx);
    let (first, last) = visible(cx);
    assert!(first <= 300 && 300 <= last, "{first}..{last}");
}

#[gpui::test]
fn view_state_keeps_folds_and_the_first_line(cx: &mut TestAppContext) {
    let text = "fn a() {\n    one\n    two\n}\nfn b() {\n    three\n}\n".repeat(20);
    let (editor, cx) = setup(&text, cx);
    // Wait for the indentation folding ranges.
    cx.executor()
        .advance_clock(std::time::Duration::from_millis(500));
    cx.run_until_parked();
    draw(cx);
    let lh = editor.read_with(cx, |e, _| e.options().resolved_line_height());
    editor.update(cx, |e, cx| {
        e.set_selections(&[Selection::collapsed(p(4, 0))], cx);
        e.fold(cx);
        e.set_selections(&[Selection::collapsed(p(30, 0))], cx);
        e.set_scroll_top(lh * 20., ScrollType::Immediate, cx);
    });
    draw(cx);
    let folds = editor.read_with(cx, |e, cx| e.collapsed_folds(cx));
    assert_eq!(folds, [(4, 5)]);
    let state = editor.read_with(cx, |e, cx| e.save_view_state(cx));
    assert_eq!(state.folding.regions.len(), 1);
    assert!(state.folding.regions[0].is_collapsed);
    // Row 21 is the first visible line (row 5 is folded above it).
    assert_eq!(state.first_position, Some(p(21, 0)));

    // Away and back, as hosts switch models.
    let model = editor.read_with(cx, |e, _| e.model().clone());
    let other = cx.update(|_, cx| cx.new(|_| TextModel::new("other")));
    editor.update(cx, |e, cx| e.set_model(other, cx));
    draw(cx);
    // A zone above now: the first line stays at the top anyway.
    editor.update(cx, |e, cx| {
        e.set_model(model.clone(), cx);
        let view = cx.new(|_| gpui::Empty).into();
        e.add_view_zone(ViewZone::new(1, ZoneHeight::Pixels(px(40.)), view), cx);
        e.restore_view_state(&state, cx);
    });
    draw(cx);
    assert_eq!(
        editor.read_with(cx, |e, cx| e.collapsed_folds(cx)),
        [(4, 5)]
    );
    assert_eq!(
        editor.read_with(cx, |e, _| e.selections()[0].head),
        p(30, 0)
    );
    assert_eq!(
        editor.read_with(cx, |e, _| e.scroll_top()),
        state.scroll_top + px(40.)
    );

    // A region whose lines changed isn't restored (Monaco's checksum).
    editor.update(cx, |e, cx| {
        e.unfold_all(cx);
        e.model().update(cx, |m, cx| {
            let offset = m.offset_at(p(5, 4));
            m.edit([(offset..offset, "changed ")], cx).unwrap();
        });
        e.restore_view_state(&state, cx);
    });
    draw(cx);
    assert!(
        editor
            .read_with(cx, |e, cx| e.collapsed_folds(cx))
            .is_empty()
    );
}

#[gpui::test]
fn view_state_keeps_manual_ranges(cx: &mut TestAppContext) {
    let (editor, cx) = setup(&lines(20), cx);
    draw(cx);
    editor.update(cx, |e, cx| {
        e.set_selections(&[Selection::new(p(2, 0), p(5, 3))], cx);
        e.create_fold_from_selection(cx);
    });
    let state = editor.read_with(cx, |e, cx| e.save_view_state(cx));
    assert!(matches!(
        state.folding.regions.as_slice(),
        [r] if r.manual && r.is_collapsed && (r.start_row, r.end_row) == (2, 5)
    ));
    let options = EditorOptions::default();
    let fresh = cx.update(|window, cx| {
        let model = editor.read(cx).model().clone();
        cx.new(|cx| {
            let mut e = CodeEditor::new(model, window, cx);
            e.update_options(options, cx);
            e
        })
    });
    fresh.update(cx, |e, cx| e.restore_view_state(&state, cx));
    assert_eq!(fresh.read_with(cx, |e, cx| e.collapsed_folds(cx)), [(2, 5)]);
}

/// Counts `event` in the recorded events and forgets them.
fn take_count(events: &Rc<RefCell<Vec<CodeEditorEvent>>>, event: &CodeEditorEvent) -> usize {
    let count = events.borrow().iter().filter(|e| *e == event).count();
    events.borrow_mut().clear();
    count
}

/// Monaco `onDidCompositionStart`/`onDidCompositionEnd` and
/// `inComposition`, from the IME's marked text.
#[gpui::test]
fn composition_events(cx: &mut TestAppContext) {
    let (editor, cx) = setup("ab", cx);
    let events = record(&editor, cx);
    let composition = |events: &Rc<RefCell<Vec<CodeEditorEvent>>>| -> Vec<&'static str> {
        let names = events
            .borrow()
            .iter()
            .filter_map(|e| match e {
                CodeEditorEvent::CompositionStarted => Some("start"),
                CodeEditorEvent::CompositionEnded => Some("end"),
                _ => None,
            })
            .collect();
        events.borrow_mut().clear();
        names
    };
    editor.update(cx, |e, cx| {
        e.set_selections(&[Selection::collapsed(p(0, 1))], cx)
    });
    events.borrow_mut().clear();
    // Marking text starts a composition; updating it doesn't again.
    editor.update_in(cx, |e, window, cx| {
        e.replace_and_mark_text_in_range(None, "n", None, window, cx);
        assert!(e.in_composition());
        e.replace_and_mark_text_in_range(None, "ni", None, window, cx);
    });
    assert_eq!(composition(&events), ["start"]);
    // Committing ends it.
    editor.update_in(cx, |e, window, cx| {
        e.replace_text_in_range(None, "你", window, cx);
        assert!(!e.in_composition());
    });
    assert_eq!(composition(&events), ["end"]);
    let model = editor.read_with(cx, |e, _| e.model().clone());
    assert_eq!(model.read_with(cx, |m, _| m.text()), "a你b");
    // Marking nothing cancels it, and so does unmarking.
    editor.update_in(cx, |e, window, cx| {
        e.replace_and_mark_text_in_range(None, "x", None, window, cx);
        e.replace_and_mark_text_in_range(None, "", None, window, cx);
        e.replace_and_mark_text_in_range(None, "y", None, window, cx);
        e.unmark_text(window, cx);
    });
    assert_eq!(composition(&events), ["start", "end", "start", "end"]);
    // Typing without an IME doesn't compose.
    cx.simulate_input("z");
    assert!(composition(&events).is_empty());
}

/// Monaco `onDidChangeHiddenAreas`, from hidden areas and folding.
#[gpui::test]
fn hidden_area_events(cx: &mut TestAppContext) {
    let (editor, cx) = setup("fn a() {\n    one\n}\nfn b() {\n    two\n}", cx);
    cx.executor()
        .advance_clock(std::time::Duration::from_millis(500));
    cx.run_until_parked();
    draw(cx);
    let events = record(&editor, cx);
    let hidden = CodeEditorEvent::HiddenAreasChanged;
    editor.update(cx, |e, cx| {
        e.set_hidden_areas(std::iter::once(1..2).collect(), cx)
    });
    draw(cx);
    assert_eq!(take_count(&events, &hidden), 1);
    // The same areas again: no change.
    editor.update(cx, |e, cx| {
        e.set_hidden_areas(std::iter::once(1..2).collect(), cx)
    });
    draw(cx);
    assert_eq!(take_count(&events, &hidden), 0);
    // Folding hides lines too.
    editor.update(cx, |e, cx| {
        e.set_selections(&[Selection::collapsed(p(3, 0))], cx);
        e.fold(cx);
    });
    draw(cx);
    assert_eq!(take_count(&events, &hidden), 1);
    editor.update(cx, |e, cx| {
        e.unfold(cx);
        e.set_hidden_areas(Vec::new(), cx);
    });
    draw(cx);
    assert_eq!(take_count(&events, &hidden), 1);
    assert!(editor.read_with(cx, |e, _| e.hidden_areas().is_empty()));
}

/// Monaco `onDidChangeViewZones`.
#[gpui::test]
fn view_zone_events(cx: &mut TestAppContext) {
    let (editor, cx) = setup(&lines(5), cx);
    let events = record(&editor, cx);
    let zones = CodeEditorEvent::ViewZonesChanged;
    let view = cx.update(|_, cx| cx.new(|_| gpui::Empty).into());
    let id = editor.update(cx, |e, cx| {
        e.add_view_zone(ViewZone::new(1, ZoneHeight::Lines(2.), view), cx)
    });
    assert_eq!(take_count(&events, &zones), 1);
    editor.update(cx, |e, cx| {
        e.set_view_zone_height(id, ZoneHeight::Lines(3.), cx);
        // The same height: no change.
        e.set_view_zone_height(id, ZoneHeight::Lines(3.), cx);
    });
    assert_eq!(take_count(&events, &zones), 1);
    editor.update(cx, |e, cx| {
        e.remove_widget(id, cx);
        e.remove_widget(id, cx);
    });
    assert_eq!(take_count(&events, &zones), 1);
}

/// Monaco `onBeginUpdate`/`onEndUpdate` bracket edits, typing, undo and
/// setting selections, once however nested.
#[gpui::test]
fn begin_and_end_update_events(cx: &mut TestAppContext) {
    let (editor, cx) = setup("hello", cx);
    let events = record(&editor, cx);
    let updates = |events: &Rc<RefCell<Vec<CodeEditorEvent>>>| -> Vec<String> {
        let names = events
            .borrow()
            .iter()
            .filter_map(|e| match e {
                CodeEditorEvent::BeginUpdate => Some("begin".to_string()),
                CodeEditorEvent::EndUpdate => Some("end".to_string()),
                CodeEditorEvent::WillType(text) => Some(format!("will type {text}")),
                CodeEditorEvent::DidType(text) => Some(format!("did type {text}")),
                _ => None,
            })
            .collect();
        events.borrow_mut().clear();
        names
    };
    // Typing runs edits inside its own update: one pair.
    editor.update(cx, |e, cx| e.type_text("x", cx));
    assert_eq!(
        updates(&events),
        ["begin", "will type x", "did type x", "end"]
    );
    editor.update(cx, |e, cx| {
        e.execute_edits(vec![(p(0, 0)..p(0, 0), "y".into())], None, cx);
    });
    assert_eq!(updates(&events), ["begin", "end"]);
    editor.update(cx, |e, cx| e.undo(cx));
    assert_eq!(updates(&events), ["begin", "end"]);
    editor.update(cx, |e, cx| {
        e.set_selections(&[Selection::collapsed(p(0, 1))], cx)
    });
    assert_eq!(updates(&events), ["begin", "end"]);
    // Scrolling isn't bracketed.
    editor.update(cx, |e, cx| {
        e.set_scroll_top(px(0.), ScrollType::Immediate, cx)
    });
    assert!(updates(&events).is_empty());
}

/// Monaco `getStatusbarColumn`: tabs to the next tab stop, 1-based.
#[gpui::test]
fn statusbar_column(cx: &mut TestAppContext) {
    let (editor, cx) = setup("\tab\n𐌀x", cx);
    editor.read_with(cx, |e, cx| {
        assert_eq!(e.statusbar_column(p(0, 0), cx), 1);
        assert_eq!(e.statusbar_column(p(0, 1), cx), 5);
        assert_eq!(e.statusbar_column(p(0, 3), cx), 7);
        // Past the end, as the position is validated.
        assert_eq!(e.statusbar_column(p(0, 99), cx), 7);
        // A surrogate pair counts as one.
        assert_eq!(e.statusbar_column(p(1, 2), cx), 2);
        assert_eq!(e.statusbar_column(p(1, 3), cx), 3);
        // The visible column is 0-based and counts wide characters twice.
        assert_eq!(e.visible_column_from_position(p(0, 1), cx), 4);
    });
}
