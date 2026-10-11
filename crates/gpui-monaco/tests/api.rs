//! Embedder API parity with Monaco's `ITextModel` / `ICodeEditor` helpers.

use gpui::{AppContext as _, Entity, Focusable, TestAppContext, VisualTestContext, px};
use gpui_monaco::cursor::Selection;
use gpui_monaco::text::PointUtf16;
use gpui_monaco::{CodeEditor, EndOfLinePreference, RevealType, TextModel};

fn p(row: u32, column: u32) -> PointUtf16 {
    PointUtf16::new(row, column)
}

fn setup<'a>(
    text: &str,
    cx: &'a mut TestAppContext,
) -> (Entity<CodeEditor>, &'a mut VisualTestContext) {
    cx.update(gpui_monaco::init);
    let text = text.to_string();
    let (editor, cx) = cx.add_window_view(|window, cx| CodeEditor::with_text(&text, window, cx));
    cx.update(|window, cx| window.focus(&editor.focus_handle(cx), cx));
    cx.run_until_parked();
    (editor, cx)
}

fn draw(cx: &mut VisualTestContext) {
    cx.update(|window, cx| {
        window.refresh();
        window.draw(cx).clear(cx);
    });
}

#[gpui::test]
fn model_helpers(cx: &mut TestAppContext) {
    let model = cx.new(|_| TextModel::new("let foo_bar = 1;\nfoo(foo_bar)\n😀x"));
    model.read_with(cx, |m, _| {
        assert_eq!(m.line_count(), 3);
        assert_eq!(m.line_content(1), "foo(foo_bar)");
        assert_eq!(m.line_len_utf16(2), 3);
        // Validation clamps rows, columns and surrogate halves.
        assert_eq!(m.validate_position(p(9, 9)), p(2, 3));
        assert_eq!(m.validate_position(p(0, 99)), p(0, 16));
        assert_eq!(m.validate_position(p(2, 1)), p(2, 0));
        assert_eq!(m.validate_range(p(1, 3)..p(0, 4)), p(0, 4)..p(1, 3));
        assert_eq!(m.offset_at(p(1, 0)), 17);
        assert_eq!(m.position_at(17), p(1, 0));
        assert_eq!(
            m.value_in_range(p(0, 4)..p(1, 3), EndOfLinePreference::TextDefined),
            "foo_bar = 1;\nfoo"
        );

        let word = m.word_at_position(p(0, 6)).unwrap();
        assert_eq!(
            (word.word.as_str(), word.range),
            ("foo_bar", p(0, 4)..p(0, 11))
        );
        let until = m.word_until_position(p(0, 6));
        assert_eq!((until.word.as_str(), until.range), ("fo", p(0, 4)..p(0, 6)));
        assert!(m.word_at_position(p(0, 13)).is_none());

        assert_eq!(
            m.find_matches("foo", false, true, false, 10),
            vec![p(0, 4)..p(0, 7), p(1, 0)..p(1, 3), p(1, 4)..p(1, 7)]
        );
        assert_eq!(
            m.find_matches("foo", false, true, true, 10),
            vec![p(1, 0)..p(1, 3)]
        );
        assert_eq!(m.find_matches("f.o", true, true, false, 1).len(), 1);
    });
}

#[gpui::test]
fn reveal_and_visible_ranges(cx: &mut TestAppContext) {
    let text: Vec<String> = (0..500).map(|i| format!("line {i}")).collect();
    let (editor, cx) = setup(&text.join("\n"), cx);
    draw(cx);
    let line_height = editor.read_with(cx, |e, _| e.options().resolved_line_height());

    editor.update(cx, |e, cx| e.reveal_line_in_center(250, cx));
    draw(cx);
    let ranges = editor.read_with(cx, |e, cx| e.visible_ranges(cx));
    let middle = (ranges[0].start.row + ranges[0].end.row) / 2;
    assert!(middle.abs_diff(250) <= 1, "{ranges:?}");

    // Already visible: `IfOutsideViewport` doesn't move.
    let top = editor.read_with(cx, |e, _| e.scroll_position().y);
    editor.update(cx, |e, cx| {
        e.reveal_line_in_center_if_outside_viewport(251, cx)
    });
    draw(cx);
    assert_eq!(editor.read_with(cx, |e, _| e.scroll_position().y), top);

    editor.update(cx, |e, cx| {
        e.reveal_range(p(400, 0)..p(400, 0), RevealType::Top, cx)
    });
    draw(cx);
    let top = editor.read_with(cx, |e, _| e.scroll_position().y);
    // Sticky scroll keeps five context lines above.
    assert_eq!(top, line_height * 395.);

    // Minimal reveal of a line above the viewport.
    editor.update(cx, |e, cx| e.reveal_line(10, cx));
    draw(cx);
    let ranges = editor.read_with(cx, |e, cx| e.visible_ranges(cx));
    assert!(ranges[0].start.row <= 10 && 10 <= ranges[0].end.row);
    assert!(editor.read_with(cx, |e, _| e.scroll_position().y) > px(0.));
}

#[gpui::test]
fn execute_edits_and_trigger(cx: &mut TestAppContext) {
    let (editor, cx) = setup("hello world", cx);
    let changed = editor.update(cx, |e, cx| {
        e.execute_edits(
            vec![(p(0, 0)..p(0, 5), "goodbye".into())],
            Some(vec![Selection::collapsed(p(0, 7))]),
            cx,
        )
    });
    assert!(changed);
    let (text, head) = editor.read_with(cx, |e, cx| {
        (e.model().read(cx).text(), e.selections()[0].head)
    });
    assert_eq!((text.as_str(), head), ("goodbye world", p(0, 7)));
    // Undo takes the whole edit back.
    editor.update(cx, |e, cx| e.undo(cx));
    assert_eq!(
        editor.read_with(cx, |e, cx| e.model().read(cx).text()),
        "hello world"
    );

    // Commands by name.
    editor.update_in(cx, |e, window, cx| {
        assert!(e.has_text_focus(window));
        e.trigger("code_editor::TransformToUppercase", window, cx)
            .unwrap();
        assert!(e.trigger("code_editor::NoSuchThing", window, cx).is_err());
    });
    cx.run_until_parked();
    assert_eq!(
        editor.read_with(cx, |e, cx| e.model().read(cx).text()),
        "HELLO world"
    );
    // Monaco's unlabelled commands by id, like Monaco's `trigger`.
    editor.update_in(cx, |e, window, cx| {
        e.trigger("undo", window, cx).unwrap();
    });
    cx.run_until_parked();
    assert_eq!(
        editor.read_with(cx, |e, cx| e.model().read(cx).text()),
        "hello world"
    );
    editor.update_in(cx, |e, window, cx| {
        e.set_selections(&[Selection::collapsed(p(0, 0))], cx);
        e.trigger("cursorWordEndRight", window, cx).unwrap();
        e.trigger("cursorRightSelect", window, cx).unwrap();
    });
    cx.run_until_parked();
    assert_eq!(
        editor.read_with(cx, |e, _| e.selections()[0]),
        Selection::new(p(0, 5), p(0, 6))
    );
    // Labelled ones are also listed for the command palette.
    let ids = editor.update_in(cx, |e, _, cx| {
        e.supported_actions(cx)
            .into_iter()
            .map(|a| a.id.to_string())
            .collect::<Vec<_>>()
    });
    assert!(ids.iter().any(|id| id == "actions.findWithSelection"));
    assert!(!ids.iter().any(|id| id == "cursorDown"));

    // Read-only editors refuse edits.
    editor.update(cx, |e, cx| {
        let mut options = e.options().clone();
        options.read_only = true;
        e.update_options(options, cx);
        assert!(!e.execute_edits(vec![(p(0, 0)..p(0, 1), "x".into())], None, cx));
    });
}

#[gpui::test]
fn mouse_targets_and_events(cx: &mut TestAppContext) {
    use gpui::{Modifiers, point};
    use gpui_monaco::{CodeEditorEvent, MouseTargetKind};
    let (editor, cx) = setup("short\na much longer line", cx);
    draw(cx);
    let text_point = |cx: &mut VisualTestContext, row, column| {
        editor
            .read_with(cx, |e, _| e.scrolled_visible_position(p(row, column)))
            .unwrap()
            + point(px(1.), px(2.))
    };
    let target = |cx: &mut VisualTestContext, at| {
        editor.update_in(cx, |e, window, cx| e.mouse_target(at, window, cx))
    };

    let in_text = text_point(cx, 1, 3);
    let t = target(cx, in_text);
    assert_eq!(
        (t.kind, t.position),
        (MouseTargetKind::ContentText, Some(p(1, 3)))
    );
    // Past the end of a short line.
    let past = point(text_point(cx, 1, 10).x, text_point(cx, 0, 0).y);
    assert_eq!(
        target(cx, past).kind,
        MouseTargetKind::ContentEmpty { after_lines: false }
    );
    // Below the last line.
    let below = point(in_text.x, in_text.y + px(200.));
    assert_eq!(
        target(cx, below).kind,
        MouseTargetKind::ContentEmpty { after_lines: true }
    );
    // The gutter: glyph margin, then line numbers.
    let row_y = in_text.y;
    let t = target(cx, point(px(2.), row_y));
    assert_eq!(
        (t.kind, t.position),
        (MouseTargetKind::GutterGlyphMargin, Some(p(1, 0)))
    );
    assert_eq!(
        target(cx, point(px(30.), row_y)).kind,
        MouseTargetKind::GutterLineNumbers
    );

    // Events: a glyph margin click (breakpoints) and a right click.
    let events = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
    let sink = events.clone();
    cx.update(|_, cx| {
        cx.subscribe(&editor, move |_, event: &CodeEditorEvent, _| {
            if !matches!(
                event,
                CodeEditorEvent::MouseMove(_) | CodeEditorEvent::SelectionsChanged(_)
            ) {
                sink.borrow_mut().push(event.clone());
            }
        })
        .detach();
    });
    cx.simulate_click(point(px(2.), row_y), Modifiers::default());
    let first = events.borrow()[0].clone();
    let CodeEditorEvent::MouseDown(down) = first else {
        panic!("expected MouseDown, got {first:?}");
    };
    assert_eq!(down.target.kind, MouseTargetKind::GutterGlyphMargin);
    assert_eq!(down.target.position, Some(p(1, 0)));
    assert!(
        events
            .borrow()
            .iter()
            .any(|e| matches!(e, CodeEditorEvent::MouseUp(_)))
    );

    events.borrow_mut().clear();
    cx.simulate_event(gpui::MouseDownEvent {
        position: in_text,
        button: gpui::MouseButton::Right,
        modifiers: Modifiers::default(),
        click_count: 1,
        first_mouse: false,
    });
    assert!(events.borrow().iter().any(|e| matches!(
        e,
        CodeEditorEvent::ContextMenu(m) if m.target.kind == MouseTargetKind::ContentText
    )));
}

#[gpui::test]
fn content_change_events(cx: &mut TestAppContext) {
    use gpui_monaco::{ContentChange, TextModelEvent};
    let model = cx.new(|_| TextModel::new("abc\ndéf\nghi"));
    let changes = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
    let sink = changes.clone();
    cx.update(|cx| {
        cx.subscribe(&model, move |_, event: &TextModelEvent, _| {
            if let TextModelEvent::Edited { changes, .. } = event {
                sink.borrow_mut().push(changes.to_vec());
            }
        })
        .detach();
    });
    let old = model.read_with(cx, |m, _| m.text());
    // Two edits in one step: "abc" -> "aXc", "déf" -> "dé" + "ef".
    model.update(cx, |m, cx| {
        m.edit([(1..2, "X"), (7..8, "ef")], cx).unwrap();
    });
    let last = changes.borrow().last().unwrap().clone();
    assert_eq!(
        last,
        vec![
            ContentChange {
                range: p(1, 2)..p(1, 3),
                range_offset: 7,
                range_length: 1,
                // Monaco's offsets count "é" as one UTF-16 unit.
                range_offset_utf16: 6,
                range_length_utf16: 1,
                text: "ef".into(),
            },
            ContentChange {
                range: p(0, 1)..p(0, 2),
                range_offset: 1,
                range_length: 1,
                range_offset_utf16: 1,
                range_length_utf16: 1,
                text: "X".into(),
            },
        ]
    );
    // Applied in order to the old text, they give the new text.
    let mut replay = old.clone();
    for change in &last {
        replay.replace_range(
            change.range_offset..change.range_offset + change.range_length,
            &change.text,
        );
    }
    assert_eq!(replay, model.read_with(cx, |m, _| m.text()));

    // Undo reports the reverse change.
    model.update(cx, |m, cx| m.undo(None, cx));
    let undo = changes.borrow().last().unwrap().clone();
    assert_eq!(undo.len(), 2);
    assert_eq!(undo[1].text, "b");
    // set_value replaces everything (Monaco's flush).
    model.update(cx, |m, cx| m.set_value("new", cx));
    let flush = changes.borrow().last().unwrap().clone();
    assert_eq!(flush[0].range, p(0, 0)..p(2, 3));
    assert_eq!(flush[0].text, "new");
}

#[gpui::test]
fn selection_change_reasons(cx: &mut TestAppContext) {
    use gpui::{Modifiers, point};
    use gpui_monaco::{CodeEditorEvent, CursorChangeReason as R, CursorChangeSource as S};
    let (editor, cx) = setup("hello\nworld", cx);
    let changes = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
    let sink = changes.clone();
    cx.update(|_, cx| {
        cx.subscribe(&editor, move |_, event: &CodeEditorEvent, _| {
            if let CodeEditorEvent::SelectionsChanged(change) = event {
                sink.borrow_mut().push((change.reason, change.source));
            }
        })
        .detach();
    });
    let last = |cx: &mut VisualTestContext| {
        cx.run_until_parked();
        let last = changes.borrow_mut().drain(..).next_back();
        last.expect("no SelectionsChanged")
    };

    cx.simulate_keystrokes("right");
    assert_eq!(last(cx), (R::Explicit, S::Keyboard));
    cx.simulate_input("x");
    assert_eq!(last(cx), (R::NotSet, S::Keyboard));
    editor.update_in(cx, |e, window, cx| {
        e.trigger("code_editor::Undo", window, cx).unwrap()
    });
    assert_eq!(last(cx), (R::Undo, S::ModelChange));
    editor.update(cx, |e, cx| {
        e.set_selections(&[Selection::collapsed(p(1, 2))], cx)
    });
    assert_eq!(last(cx), (R::NotSet, S::Api));
    // An edit that moves the cursor (Monaco reports nothing when the
    // cursor stays put).
    editor.update(cx, |e, cx| {
        e.execute_edits(vec![(p(1, 0)..p(1, 0), "a".into())], None, cx);
    });
    assert_eq!(last(cx), (R::NotSet, S::Api));

    cx.write_to_clipboard(gpui::ClipboardItem::new_string("pasted".into()));
    editor.update_in(cx, |e, window, cx| {
        e.trigger("code_editor::Paste", window, cx).unwrap()
    });
    assert_eq!(last(cx), (R::Paste, S::Keyboard));
    // Not left over for the next change.
    cx.simulate_keystrokes("left");
    assert_eq!(last(cx), (R::Explicit, S::Keyboard));

    // Edits made elsewhere move the cursors along.
    let model = editor.read_with(cx, |e, _| e.model().clone());
    // (On the cursor's line: Monaco reports nothing when the cursors stay.)
    model.update(cx, |m, cx| {
        let start = m.offset_at(p(1, 0));
        m.edit([(start..start, "zz")], cx).unwrap();
    });
    assert_eq!(last(cx), (R::RecoverFromMarkers, S::ModelChange));
    model.update(cx, |m, cx| m.set_value("fresh", cx));
    assert_eq!(last(cx), (R::ContentFlush, S::Model));

    draw(cx);
    let at = editor
        .read_with(cx, |e, _| e.scrolled_visible_position(p(0, 2)))
        .unwrap()
        + point(px(1.), px(2.));
    cx.simulate_click(at, Modifiers::default());
    assert_eq!(last(cx), (R::Explicit, S::Mouse));
}

#[gpui::test]
fn decorations_collection(cx: &mut TestAppContext) {
    use gpui_monaco::DecorationOptions;
    let (editor, cx) = setup("one\ntwo\nthree", cx);
    let options = DecorationOptions {
        is_whole_line: true,
        ..Default::default()
    };
    let mut collection = editor.update(cx, |e, cx| {
        e.create_decorations_collection(vec![(p(0, 1)..p(0, 3), options.clone())], cx)
    });
    assert_eq!(collection.len(), 1);
    let first = collection.ids()[0];
    assert!(collection.has(first));
    cx.update(|_, cx| {
        let ids = collection.append(vec![(p(2, 1)..p(2, 9), options.clone())], cx);
        assert_eq!(ids.len(), 1);
        // Clamped to the text.
        assert_eq!(
            collection.ranges(cx),
            vec![p(0, 1)..p(0, 3), p(2, 1)..p(2, 5)]
        );
    });
    // Ranges follow edits (inserting at a range's edge would grow it).
    editor.update(cx, |e, cx| {
        e.execute_edits(vec![(p(0, 0)..p(0, 0), "\n".into())], None, cx);
    });
    cx.update(|_, cx| {
        assert_eq!(collection.range(0, cx), Some(p(1, 1)..p(1, 3)));
        // Owned by the editor.
        let model = collection_model(&editor, cx);
        let owner = model.read(cx).decoration(first).unwrap().owner;
        assert_eq!(owner, Some(editor.entity_id()));
        collection.set(vec![(p(3, 0)..p(3, 2), options.clone())], cx);
        assert!(!collection.has(first));
        assert!(model.read(cx).decoration(first).is_none());
        assert_eq!(collection.ranges(cx), vec![p(3, 0)..p(3, 2)]);
        collection.clear(cx);
        assert!(collection.is_empty());
        assert!(model.update(cx, |m, _| m.all_decorations(None)).is_empty());
    });
}

fn collection_model(editor: &Entity<CodeEditor>, cx: &gpui::App) -> Entity<TextModel> {
    editor.read(cx).model().clone()
}
