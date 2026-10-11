//! Monaco `ICodeEditor` and `ITextModel` API correctness (#264): events
//! only when something changed (so editors can sync each other), Monaco's
//! UTF-16 offsets, line ending changes as content changes, `setValue` and
//! decorations, `executeEdits` and the undo stack, editor-level model
//! events, decoration queries, mouse target details and the global
//! openers.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use gpui::{
    App, AppContext as _, Context, Entity, Focusable as _, IntoElement, ParentElement as _, Render,
    Styled as _, Task, TestAppContext, VisualTestContext, Window, div, point, px,
};
use gpui_monaco::cursor::Selection;
use gpui_monaco::providers::{
    DefinitionProvider, LanguageSelector, Location, ProviderContext, register_definition_provider,
};
use gpui_monaco::text::{LineEnding, PointUtf16, Stickiness};
use gpui_monaco::{
    CodeEditor, CodeEditorEvent, CursorChangeReason, CursorChangeSource, DecorationOptions,
    EndCursorState, EndOfLinePreference as Eol, ModelOptions, MouseTargetDetail, MouseTargetKind,
    OverlayPosition, OverlayWidget, ScrollType, SingleEditOperation, TextModel, TextModelEvent,
    ViewZone, ZoneHeight,
};

fn p(row: u32, column: u32) -> PointUtf16 {
    PointUtf16::new(row, column)
}

fn lines(count: usize) -> String {
    (0..count)
        .map(|i| format!("line {i}"))
        .collect::<Vec<_>>()
        .join("\n")
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

/// Records the editor's events that `keep` picks.
fn record(
    editor: &Entity<CodeEditor>,
    keep: impl Fn(&CodeEditorEvent) -> bool + 'static,
    cx: &mut VisualTestContext,
) -> Rc<RefCell<Vec<CodeEditorEvent>>> {
    let events = Rc::new(RefCell::new(Vec::new()));
    let sink = events.clone();
    cx.update(|_, cx| {
        cx.subscribe(editor, move |_, event: &CodeEditorEvent, _| {
            if keep(event) {
                sink.borrow_mut().push(event.clone());
            }
        })
        .detach();
    });
    events
}

/// Monaco's usual way to sync two editors' scroll positions, both ways
/// (`a.onDidScrollChange(e => b.setScrollTop(e.scrollTop))` and back):
/// it settles because setting the position an editor already has reports
/// nothing.
#[gpui::test]
fn scroll_sync_between_two_editors_settles(cx: &mut TestAppContext) {
    let text = lines(300);
    let (a, cx) = setup(&text, cx);
    let b = cx.update(|window, cx| cx.new(|cx| CodeEditor::with_text(&text, window, cx)));
    draw(cx);
    let events = Rc::new(Cell::new(0usize));
    for (from, to) in [(a.clone(), b.clone()), (b.clone(), a.clone())] {
        let events = events.clone();
        cx.update(|_, cx| {
            cx.subscribe(&from, move |_, event: &CodeEditorEvent, cx| {
                if let CodeEditorEvent::ScrollChanged(scroll) = event {
                    events.set(events.get() + 1);
                    assert!(events.get() < 20, "the scroll sync loops");
                    if scroll.scroll_top_changed {
                        let top = scroll.scroll_top;
                        to.update(cx, |to, cx| {
                            to.set_scroll_top(top, ScrollType::Immediate, cx)
                        });
                    }
                }
            })
            .detach();
        });
    }
    a.update(cx, |a, cx| {
        a.set_scroll_top(px(100.), ScrollType::Immediate, cx)
    });
    cx.run_until_parked();
    assert_eq!(b.read_with(cx, |b, _| b.scroll_top()), px(100.));
    // `a`'s change, then `b`'s; `b` setting `a` where it is reports nothing.
    assert_eq!(events.get(), 2);
    events.set(0);
    a.update(cx, |a, cx| {
        a.set_scroll_top(px(100.), ScrollType::Immediate, cx)
    });
    b.update(cx, |b, cx| {
        b.set_scroll_position(point(px(0.), px(100.)), cx)
    });
    cx.run_until_parked();
    assert_eq!(events.get(), 0);

    // The event says what changed (Monaco `IScrollEvent`).
    let scrolls = record(&a, |e| matches!(e, CodeEditorEvent::ScrollChanged(_)), cx);
    a.update(cx, |a, cx| {
        a.set_scroll_top(px(40.), ScrollType::Immediate, cx)
    });
    cx.run_until_parked();
    let CodeEditorEvent::ScrollChanged(scroll) = scrolls.borrow()[0].clone() else {
        unreachable!()
    };
    assert_eq!(scroll.scroll_top, px(40.));
    assert!(scroll.scroll_top_changed);
    assert!(!scroll.scroll_left_changed && !scroll.scroll_height_changed);
    assert_eq!(
        Some(scroll.scroll_height),
        a.read_with(cx, |a, _| a.scroll_height())
    );
}

/// The same for selections (Monaco compares cursor states before firing
/// `onDidChangeCursorSelection`).
#[gpui::test]
fn selection_sync_between_two_editors_settles(cx: &mut TestAppContext) {
    let text = lines(10);
    let (a, cx) = setup(&text, cx);
    let b = cx.update(|window, cx| cx.new(|cx| CodeEditor::with_text(&text, window, cx)));
    let events = Rc::new(Cell::new(0usize));
    for (from, to) in [(a.clone(), b.clone()), (b.clone(), a.clone())] {
        let events = events.clone();
        cx.update(|_, cx| {
            cx.subscribe(&from, move |_, event: &CodeEditorEvent, cx| {
                if let CodeEditorEvent::SelectionsChanged(change) = event {
                    events.set(events.get() + 1);
                    assert!(events.get() < 20, "the selection sync loops");
                    let mut selections = vec![change.selection];
                    selections.extend(&change.secondary_selections);
                    to.update(cx, |to, cx| to.set_selections(&selections, cx));
                }
            })
            .detach();
        });
    }
    let selections = [
        Selection::new(p(1, 0), p(1, 3)),
        Selection::collapsed(p(4, 2)),
    ];
    a.update(cx, |a, cx| a.set_selections(&selections, cx));
    cx.run_until_parked();
    assert_eq!(b.read_with(cx, |b, _| b.selections()), selections);
    assert_eq!(events.get(), 2);
    // The same selections again: nothing.
    events.set(0);
    a.update(cx, |a, cx| a.set_selections(&selections, cx));
    cx.run_until_parked();
    assert_eq!(events.get(), 0);
}

/// Monaco `ICursorPositionChangedEvent` and `ICursorSelectionChangedEvent`
/// payloads.
#[gpui::test]
fn cursor_event_payloads(cx: &mut TestAppContext) {
    let (editor, cx) = setup("hello\nworld", cx);
    let events = record(
        &editor,
        |e| {
            matches!(
                e,
                CodeEditorEvent::SelectionsChanged(_) | CodeEditorEvent::CursorPositionChanged(_)
            )
        },
        cx,
    );
    let first = [
        Selection::new(p(0, 0), p(0, 2)),
        Selection::collapsed(p(1, 1)),
    ];
    editor.update(cx, |e, cx| e.set_selections(&first, cx));
    events.borrow_mut().clear();
    editor.update(cx, |e, cx| e.set_position(p(1, 4), cx));
    let version = editor.read_with(cx, |e, cx| e.model().read(cx).version());
    let events = events.borrow();
    let [
        CodeEditorEvent::CursorPositionChanged(position),
        CodeEditorEvent::SelectionsChanged(selection),
    ] = events.as_slice()
    else {
        panic!("{events:?}");
    };
    assert_eq!(position.position, p(1, 4));
    assert!(position.secondary_positions.is_empty());
    assert_eq!(
        (position.reason, position.source),
        (CursorChangeReason::NotSet, CursorChangeSource::Api)
    );
    assert_eq!(selection.selection, Selection::collapsed(p(1, 4)));
    assert_eq!(selection.old_selections.as_deref(), Some(&first[..]));
    assert_eq!(selection.model_version_id, version);
    assert_eq!(selection.old_model_version_id, version);
    drop(events);
    assert_eq!(editor.read_with(cx, |e, _| e.position()), p(1, 4));
    editor.update(cx, |e, cx| {
        e.set_selection(Selection::new(p(0, 1), p(0, 3)), cx)
    });
    assert_eq!(
        editor.read_with(cx, |e, _| e.selection()),
        Selection::new(p(0, 1), p(0, 3))
    );
}

/// Monaco `executeEdits` returns `false` in a read-only editor without the
/// read-only message, which is only for the user's own edits.
#[gpui::test]
fn execute_edits_in_a_read_only_editor_is_silent(cx: &mut TestAppContext) {
    let (editor, cx) = setup("hello", cx);
    let attempts = record(
        &editor,
        |e| matches!(e, CodeEditorEvent::ReadOnlyEditAttempt),
        cx,
    );
    editor.update(cx, |e, cx| {
        e.update_options_with(|o| o.read_only = true, cx);
        assert!(!e.execute_edits(vec![(p(0, 0)..p(0, 0), "x".into())], None, cx));
        assert!(!e.execute_edit_operations(
            vec![SingleEditOperation::new(p(0, 0)..p(0, 1), "y")],
            None,
            cx
        ));
    });
    cx.run_until_parked();
    assert!(attempts.borrow().is_empty());
    assert!(editor.read_with(cx, |e, _| e.message_text()).is_none());
    assert_eq!(
        editor.read_with(cx, |e, cx| e.model().read(cx).text()),
        "hello"
    );
    // Typing still says so (Monaco `onDidAttemptReadOnlyEdit`).
    cx.simulate_input("x");
    assert_eq!(attempts.borrow().len(), 1);
}

/// Monaco `getOffsetAt` and `getPositionAt` count UTF-16 units and the
/// model's line ending, unlike the byte `offset_at`.
#[gpui::test]
fn monaco_offsets(cx: &mut TestAppContext) {
    let model = cx.new(|_| TextModel::new("a😀b\r\ncd"));
    model.read_with(cx, |m, _| {
        assert_eq!(m.line_ending(), LineEnding::Crlf);
        let offsets: Vec<usize> = [p(0, 0), p(0, 1), p(0, 3), p(0, 4), p(1, 0), p(1, 2)]
            .into_iter()
            .map(|position| m.get_offset_at(position))
            .collect();
        assert_eq!(offsets, [0, 1, 3, 4, 6, 8]);
        // The stored text's bytes: "a", four for the emoji, "b", "\n".
        assert_eq!(m.offset_at(p(1, 0)), 7);
        assert_eq!(m.get_position_at(6), p(1, 0));
        assert_eq!(m.get_position_at(8), p(1, 2));
        assert_eq!(m.get_position_at(99), p(1, 2));
        // Inside the CRLF and inside the surrogate pair: before them.
        assert_eq!(m.get_position_at(5), p(0, 4));
        assert_eq!(m.get_position_at(2), p(0, 1));
        assert_eq!(m.value_length(), 8);
        // They agree with `modify_position`.
        for offset in 0..=8 {
            assert_eq!(
                m.modify_position(p(0, 0), offset as isize),
                m.get_position_at(offset)
            );
        }
    });
}

/// Monaco `textModel.test.ts` "getValueLengthInRange" and
/// "getValueLengthInRange different EOL" (1-based ranges there).
#[gpui::test]
fn value_length_in_range(cx: &mut TestAppContext) {
    for (text, eol) in [
        ("My First Line\r\nMy Second Line\r\nMy Third Line", "\r\n"),
        ("My First Line\nMy Second Line\nMy Third Line", "\n"),
    ] {
        let model = cx.new(|_| TextModel::new(text));
        model.read_with(cx, |m, _| {
            let len = |range: std::ops::Range<PointUtf16>| {
                m.value_length_in_range(range, Eol::TextDefined)
            };
            let utf16 = |s: &str| s.replace('\n', eol).encode_utf16().count();
            assert_eq!(len(p(0, 0)..p(0, 0)), 0);
            assert_eq!(len(p(0, 0)..p(0, 1)), 1);
            assert_eq!(len(p(0, 1)..p(0, 2)), 1);
            assert_eq!(len(p(0, 0)..p(0, 13)), utf16("My First Line"));
            assert_eq!(len(p(0, 0)..p(1, 0)), utf16("My First Line\n"));
            assert_eq!(len(p(0, 1)..p(1, 0)), utf16("y First Line\n"));
            assert_eq!(len(p(0, 1)..p(1, 1)), utf16("y First Line\nM"));
            assert_eq!(
                len(p(0, 1)..p(1, 999)),
                utf16("y First Line\nMy Second Line")
            );
            assert_eq!(
                len(p(0, 1)..p(2, 0)),
                utf16("y First Line\nMy Second Line\n")
            );
            assert_eq!(
                len(p(0, 0)..p(999, 999)),
                utf16("My First Line\nMy Second Line\nMy Third Line")
            );
            // Other line endings.
            let full = p(0, 0)..p(999, 999);
            assert_eq!(m.value_length_in_range(p(0, 0)..p(1, 0), Eol::Crlf), 15);
            assert_eq!(m.value_length_in_range(p(0, 0)..p(1, 0), Eol::Lf), 14);
            assert_eq!(m.value_length_in_range(full.clone(), Eol::Crlf), 44);
            assert_eq!(m.value_length_in_range(full, Eol::Lf), 42);
        });
    }
}

/// Monaco `model.test.ts` "getValueInRange", `getValue(eol, preserveBOM)`
/// (`textModel.test.ts` "file with BOM") and `getCharacterCountInRange`.
#[gpui::test]
fn values_with_line_endings_and_bom(cx: &mut TestAppContext) {
    let model = cx
        .new(|_| TextModel::new("My First Line\r\n\t\tMy Second Line\r\n    Third Line\r\n\r\n1"));
    model.read_with(cx, |m, _| {
        let value = |range: std::ops::Range<PointUtf16>| m.value_in_range(range, Eol::TextDefined);
        assert_eq!(value(p(0, 0)..p(0, 0)), "");
        assert_eq!(value(p(0, 0)..p(0, 1)), "M");
        assert_eq!(value(p(0, 0)..p(1, 0)), "My First Line\r\n");
        assert_eq!(value(p(0, 0)..p(1, 2)), "My First Line\r\n\t\t");
        assert_eq!(
            value(p(0, 0)..p(3, 0)),
            "My First Line\r\n\t\tMy Second Line\r\n    Third Line\r\n"
        );
        assert_eq!(
            m.value_in_range(p(0, 0)..p(2, 0), Eol::Lf),
            "My First Line\n\t\tMy Second Line\n"
        );
        assert_eq!(m.value(Eol::TextDefined, false), m.text());
        assert_eq!(m.value(Eol::Lf, false), m.text().replace("\r\n", "\n"));
    });

    let model = cx.new(|_| TextModel::new("\u{feff}Hello"));
    model.read_with(cx, |m, _| {
        assert_eq!(m.line_content(0), "Hello");
        assert_eq!(m.value(Eol::TextDefined, true), "\u{feff}Hello");
        assert_eq!(m.value(Eol::TextDefined, false), "Hello");
        assert_eq!(m.value_length_with(Eol::TextDefined, true), 6);
        assert_eq!(m.value_length_with(Eol::TextDefined, false), 5);
    });
    // `setValue` detects the BOM of the new text.
    model.update(cx, |m, cx| m.set_value("Bye", cx));
    assert_eq!(
        model.read_with(cx, |m, _| m.value(Eol::TextDefined, true)),
        "Bye"
    );
    model.update(cx, |m, cx| m.set_value("\u{feff}Back", cx));
    assert_eq!(
        model.read_with(cx, |m, _| m.value(Eol::TextDefined, true)),
        "\u{feff}Back"
    );

    // A surrogate pair is one character; line breaks count as `eol`.
    let model = cx.new(|_| TextModel::new("a😀\r\nb"));
    model.read_with(cx, |m, _| {
        let full = m.full_model_range();
        assert_eq!(
            m.character_count_in_range(full.clone(), Eol::TextDefined),
            5
        );
        assert_eq!(m.character_count_in_range(full.clone(), Eol::Lf), 4);
        assert_eq!(m.value_length_in_range(full, Eol::TextDefined), 6);
    });
}

/// Monaco `editableTextModel.test.ts` "issue #1580: Changes in line endings
/// are not correctly reflected in the extension host": `setEOL` is a new
/// version and a content change, so a mirror of the text stays right.
#[gpui::test]
fn line_ending_changes_are_content_changes(cx: &mut TestAppContext) {
    let model = cx.new(|_| TextModel::new("Hello\nWorld!"));
    // A Monaco-style mirror: the value with the model's line ending,
    // changed with the UTF-16 offsets.
    struct Mirror {
        value: Vec<u16>,
        version: usize,
        eol_changes: usize,
    }
    let mirror = Rc::new(RefCell::new(Mirror {
        value: "Hello\nWorld!".encode_utf16().collect(),
        version: model.read_with(cx, |m, _| m.version()),
        eol_changes: 0,
    }));
    let sink = mirror.clone();
    let _subscription = cx.update(|cx| {
        cx.subscribe(&model, move |_, event: &TextModelEvent, _| {
            if let TextModelEvent::Edited {
                changes,
                version,
                eol,
                is_eol_change,
                ..
            } = event
            {
                let mut mirror = sink.borrow_mut();
                assert!(*version > mirror.version, "the version goes up");
                mirror.version = *version;
                mirror.eol_changes += usize::from(*is_eol_change);
                for change in changes.iter() {
                    let text: Vec<u16> = change
                        .text
                        .replace('\n', eol.as_str())
                        .encode_utf16()
                        .collect();
                    let start = change.range_offset_utf16;
                    mirror
                        .value
                        .splice(start..start + change.range_length_utf16, text);
                }
            }
        })
    });
    let check = |cx: &mut TestAppContext| {
        let (text, version) = model.read_with(cx, |m, _| (m.text(), m.version()));
        let mirror = mirror.borrow();
        assert_eq!(String::from_utf16(&mirror.value).unwrap(), text);
        assert_eq!(mirror.version, version);
    };
    let version = model.read_with(cx, |m, _| m.version());
    model.update(cx, |m, cx| m.set_line_ending(LineEnding::Crlf, cx));
    check(cx);
    assert_eq!(model.read_with(cx, |m, _| m.version()), version + 1);
    assert_eq!(mirror.borrow().eol_changes, 1);
    // The same line ending: nothing.
    model.update(cx, |m, cx| m.set_line_ending(LineEnding::Crlf, cx));
    assert_eq!(model.read_with(cx, |m, _| m.version()), version + 1);
    // Edits after it count the CRLF as two.
    model.update(cx, |m, cx| {
        m.apply_edits(vec![(p(1, 0)..p(1, 0), "x\ny".into())], cx)
            .unwrap();
    });
    check(cx);
    // `pushEOL`, its undo and its redo.
    model.update(cx, |m, cx| m.push_line_ending(LineEnding::Lf, cx));
    check(cx);
    model.update(cx, |m, cx| m.undo(None, cx));
    check(cx);
    assert_eq!(
        model.read_with(cx, |m, _| m.line_ending()),
        LineEnding::Crlf
    );
    model.update(cx, |m, cx| m.redo(None, cx));
    check(cx);
    assert_eq!(mirror.borrow().eol_changes, 4);
}

/// Monaco `setValue` destroys every decoration.
#[gpui::test]
fn set_value_removes_decorations(cx: &mut TestAppContext) {
    let model = cx.new(|_| TextModel::new("one\ntwo"));
    let changes = Rc::new(Cell::new(0));
    let seen = changes.clone();
    let _subscription = cx.update(|cx| {
        cx.subscribe(&model, move |_, event: &TextModelEvent, _| {
            if matches!(event, TextModelEvent::DecorationsChanged(_)) {
                seen.set(seen.get() + 1);
            }
        })
    });
    let ids = model.update(cx, |m, cx| {
        m.delta_decorations_at(
            None,
            &[],
            vec![(p(1, 0)..p(1, 3), DecorationOptions::default())],
            cx,
        )
    });
    assert_eq!(changes.get(), 1);
    model.update(cx, |m, cx| m.set_value("three", cx));
    assert_eq!(changes.get(), 2);
    model.read_with(cx, |m, _| {
        assert!(m.decoration_range(ids[0]).is_none());
        assert!(m.all_model_decorations(None, false).is_empty());
    });
}

/// Monaco's `IEditorDecorationsCollection` acts on the editor's current
/// model, and an editor's decorations leave the model it stops showing.
#[gpui::test]
fn decorations_collection_follows_the_model(cx: &mut TestAppContext) {
    let (editor, cx) = setup("one\ntwo", cx);
    let first = editor.read_with(cx, |e, _| e.model().clone());
    let options = DecorationOptions {
        is_whole_line: true,
        ..Default::default()
    };
    let mut collection = editor.update(cx, |e, cx| {
        e.create_decorations_collection(vec![(p(0, 0)..p(0, 3), options.clone())], cx)
    });
    let others = Rc::new(Cell::new(0));
    let seen = others.clone();
    let _subscription = cx
        .update(|_, cx| collection.on_did_change(move |_, _| seen.set(seen.get() + 1), cx))
        .unwrap();
    // Its own changes aren't reported to it.
    cx.update(|_, cx| collection.set(vec![(p(1, 0)..p(1, 3), options.clone())], cx));
    cx.run_until_parked();
    assert_eq!(others.get(), 0);
    // The editor's own (Monaco `deltaDecorations`) are.
    let own = editor.update(cx, |e, cx| {
        e.delta_decorations(&[], vec![(p(0, 0)..p(0, 1), options.clone())], cx)
    });
    cx.run_until_parked();
    assert_eq!(others.get(), 1);
    assert_eq!(
        editor.update(cx, |e, cx| e.line_decorations(0, cx)).len(),
        1
    );
    editor.update(cx, |e, cx| e.remove_decorations(&own, cx));

    let second = cx.update(|_, cx| cx.new(|_| TextModel::new("alpha\nbeta\ngamma")));
    editor.update(cx, |e, cx| e.set_model(second.clone(), cx));
    // Monaco `_detachModel`: the editor's decorations are gone.
    assert!(first.read_with(cx, |m, _| m.all_model_decorations(None, false).is_empty()));
    cx.update(|_, cx| {
        assert!(collection.ranges(cx).is_empty());
        collection.set(vec![(p(2, 0)..p(2, 5), options.clone())], cx);
        assert_eq!(collection.ranges(cx), vec![p(2, 0)..p(2, 5)]);
    });
    let id = collection.ids()[0];
    assert!(second.read_with(cx, |m, _| m.decoration(id).is_some()));
    // Even when set while the editor is being updated.
    editor.update(cx, |_, cx| {
        collection.set(vec![(p(1, 0)..p(1, 4), options.clone())], cx);
    });
    cx.update(|_, cx| assert_eq!(collection.ranges(cx), vec![p(1, 0)..p(1, 4)]));
}

/// Monaco `executeEdits` with a cursor state computer and
/// `forceMoveMarkers`, joining the open undo step; `popUndoStop`.
#[gpui::test]
fn execute_edit_operations(cx: &mut TestAppContext) {
    let (editor, cx) = setup("hello world", cx);
    let model = editor.read_with(cx, |e, _| e.model().clone());
    let never_grows = DecorationOptions {
        stickiness: Stickiness::NeverGrowsWhenTypingAtEdges,
        ..Default::default()
    };
    let ids = model.update(cx, |m, cx| {
        m.delta_decorations_at(
            None,
            &[],
            vec![
                (p(0, 0)..p(0, 5), never_grows.clone()),
                (p(0, 6)..p(0, 11), never_grows.clone()),
            ],
            cx,
        )
    });
    // Forced, the decoration ending at the edit grows past its text.
    editor.update(cx, |e, cx| {
        assert!(e.execute_edit_operations(
            vec![SingleEditOperation::new(p(0, 5)..p(0, 5), "!").force_move_markers()],
            None,
            cx,
        ));
    });
    let range = |id, cx: &mut VisualTestContext| model.read_with(cx, |m, _| m.decoration_range(id));
    assert_eq!(range(ids[0], cx), Some(p(0, 0)..p(0, 6)));
    // Not forced, it doesn't.
    editor.update(cx, |e, cx| {
        e.execute_edits(vec![(p(0, 12)..p(0, 12), "?".into())], None, cx)
    });
    assert_eq!(range(ids[1], cx), Some(p(0, 7)..p(0, 12)));

    // A cursor state computer gets the undo edits.
    let undo_edits = Rc::new(RefCell::new(Vec::new()));
    let sink = undo_edits.clone();
    editor.update(cx, |e, cx| {
        e.execute_edit_operations(
            vec![SingleEditOperation::new(p(0, 0)..p(0, 5), "bye")],
            Some(EndCursorState::Computer(Box::new(move |edits| {
                *sink.borrow_mut() = edits.to_vec();
                Some(vec![Selection::collapsed(p(0, 1))])
            }))),
            cx,
        );
    });
    assert_eq!(
        *undo_edits.borrow(),
        vec![(p(0, 0)..p(0, 3), "hello".to_string())]
    );
    assert_eq!(
        editor.read_with(cx, |e, _| e.selections()),
        [Selection::collapsed(p(0, 1))]
    );
    assert_eq!(model.read_with(cx, |m, _| m.text()), "bye! world?");
}

/// Monaco's `executeEdits` and `pushEditOperations` append to the open
/// undo element, typing's too; `pushUndoStop` and `popUndoStop` close and
/// reopen it.
#[gpui::test]
fn execute_edits_join_the_open_undo_step(cx: &mut TestAppContext) {
    let (editor, cx) = setup("", cx);
    let text = |cx: &mut VisualTestContext| editor.read_with(cx, |e, cx| e.model().read(cx).text());
    editor.update(cx, |e, cx| {
        e.type_text("ab", cx);
        e.execute_edits(vec![(p(0, 2)..p(0, 2), "c".into())], None, cx);
    });
    editor.update(cx, |e, cx| e.undo(cx));
    assert_eq!(text(cx), "");

    editor.update(cx, |e, cx| {
        e.execute_edits(vec![(p(0, 0)..p(0, 0), "1".into())], None, cx);
        e.push_undo_stop(cx);
        e.execute_edits(vec![(p(0, 1)..p(0, 1), "2".into())], None, cx);
        e.push_undo_stop(cx);
        e.pop_undo_stop(cx);
        e.execute_edits(vec![(p(0, 2)..p(0, 2), "3".into())], None, cx);
    });
    assert_eq!(text(cx), "123");
    editor.update(cx, |e, cx| e.undo(cx));
    assert_eq!(text(cx), "1");
}

/// Monaco `setHiddenAreas(ranges, source)`: each source's areas are kept
/// apart, and the rows hidden are their union.
#[gpui::test]
fn hidden_areas_by_source(cx: &mut TestAppContext) {
    let (editor, cx) = setup(&lines(10), cx);
    editor.update(cx, |e, cx| {
        e.set_hidden_areas_from("a", std::iter::once(1..2).collect(), cx);
        e.set_hidden_areas_from("b", std::iter::once(4..6).collect(), cx);
        assert_eq!(e.hidden_areas(), vec![1..2, 4..6]);
        e.set_hidden_areas(std::iter::once(0..1).collect(), cx);
        assert_eq!(e.hidden_areas(), vec![0..2, 4..6]);
        e.set_hidden_areas_from("a", Vec::new(), cx);
        assert_eq!(e.hidden_areas(), vec![0..1, 4..6]);
    });
    draw(cx);
    let visible = editor.read_with(cx, |e, cx| e.visible_ranges(cx));
    assert_eq!(visible[0].start.row, 1);
}

/// Monaco's editor-level model events: `onDidChangeModelContent`,
/// `onDidChangeModelDecorations`, `onDidChangeModelOptions`, for whichever
/// model the editor shows.
#[gpui::test]
fn model_events_are_forwarded(cx: &mut TestAppContext) {
    let (editor, cx) = setup("hello", cx);
    let events = record(
        &editor,
        |e| {
            matches!(
                e,
                CodeEditorEvent::ModelContentChanged(_)
                    | CodeEditorEvent::ModelDecorationsChanged(_)
                    | CodeEditorEvent::ModelOptionsChanged(_)
            )
        },
        cx,
    );
    let model = editor.read_with(cx, |e, _| e.model().clone());
    model.update(cx, |m, cx| {
        m.edit([(0..0, "> ")], cx).unwrap();
        m.update_options(
            ModelOptions {
                tab_size: 2,
                ..m.options().clone()
            },
            cx,
        );
        m.delta_decorations_at(
            None,
            &[],
            vec![(
                p(0, 0)..p(0, 1),
                DecorationOptions {
                    glyph: Some(("●".into(), gpui::red())),
                    ..Default::default()
                },
            )],
            cx,
        );
        m.set_line_ending(LineEnding::Crlf, cx);
    });
    cx.run_until_parked();
    let version = model.read_with(cx, |m, _| m.version());
    {
        let events = events.borrow();
        let [
            CodeEditorEvent::ModelContentChanged(edit),
            CodeEditorEvent::ModelOptionsChanged(options),
            CodeEditorEvent::ModelDecorationsChanged(decorations),
            CodeEditorEvent::ModelContentChanged(eol),
        ] = events.as_slice()
        else {
            panic!("{events:?}");
        };
        assert_eq!(edit.changes[0].text, "> ");
        assert!(!edit.is_flush && !edit.is_eol_change && !edit.is_undoing);
        assert!(options.tab_size && !options.insert_spaces);
        assert!(decorations.affects_glyph_margin && !decorations.affects_line_number);
        assert!(eol.is_eol_change);
        assert_eq!((eol.eol, eol.version_id), (LineEnding::Crlf, version));
    }
    // After `set_model`, the new model's events, not the old one's.
    let other = cx.update(|_, cx| cx.new(|_| TextModel::new("other")));
    editor.update(cx, |e, cx| e.set_model(other.clone(), cx));
    events.borrow_mut().clear();
    model.update(cx, |m, cx| m.edit([(0..0, "old")], cx).unwrap());
    other.update(cx, |m, cx| m.set_value("new", cx));
    cx.run_until_parked();
    let events = events.borrow();
    // (`set_value` also removes the editor's own decorations, a
    // decorations change.)
    let [CodeEditorEvent::ModelContentChanged(flush), ..] = events.as_slice() else {
        panic!("{events:?}");
    };
    assert!(flush.is_flush);
    assert_eq!(
        events
            .iter()
            .filter(|e| matches!(e, CodeEditorEvent::ModelContentChanged(_)))
            .count(),
        1
    );
}

/// Monaco's model decoration getters, in positions.
#[gpui::test]
fn decoration_queries(cx: &mut TestAppContext) {
    let (editor, cx) = setup("one\ntwo\nthree", cx);
    let model = editor.read_with(cx, |e, _| e.model().clone());
    let glyph = DecorationOptions {
        glyph: Some(("●".into(), gpui::red())),
        ..Default::default()
    };
    let validation = DecorationOptions {
        validation: true,
        overview_ruler: Some((gpui::red(), gpui_monaco::OverviewRulerLane::Right)),
        ..Default::default()
    };
    let ids = model.update(cx, |m, cx| {
        m.delta_decorations_at(
            None,
            &[],
            vec![
                (p(1, 0)..p(1, 3), glyph.clone()),
                (p(2, 1)..p(2, 2), validation),
            ],
            cx,
        )
    });
    // One only this editor shows.
    let owned = editor.update(cx, |e, cx| {
        e.delta_decorations(
            &[],
            vec![(p(0, 0)..p(0, 1), DecorationOptions::default())],
            cx,
        )
    });
    let editor_id = editor.entity_id();
    model.read_with(cx, |m, _| {
        assert_eq!(m.decoration_range(ids[0]), Some(p(1, 0)..p(1, 3)));
        assert!(m.decoration_options(ids[0]).unwrap().glyph.is_some());
        assert_eq!(m.line_decorations(1, None, false)[0].id, ids[0]);
        assert_eq!(m.lines_decorations(1, 2, None, false).len(), 2);
        assert_eq!(m.lines_decorations(1, 2, None, true).len(), 1);
        assert_eq!(m.all_model_decorations(None, false).len(), 3);
        // Another editor doesn't see this one's.
        let other = gpui::EntityId::from(u64::MAX);
        assert_eq!(m.all_model_decorations(Some(other), false).len(), 2);
        assert_eq!(m.all_model_decorations(Some(editor_id), false).len(), 3);
        assert_eq!(m.overview_ruler_decorations(None, false)[0].id, ids[1]);
        assert!(m.overview_ruler_decorations(None, true).is_empty());
        assert_eq!(m.all_margin_decorations(None)[0].id, ids[0]);
        assert!(m.injected_text_decorations(None).is_empty());
        let in_range = m.decorations_in_point_range(p(0, 0)..p(1, 0), None, false);
        assert_eq!(
            in_range.iter().map(|d| d.id).collect::<Vec<_>>(),
            [owned[0], ids[0]]
        );
    });
    // The editor's own getters leave out validation decorations when the
    // editor is read-only (`renderValidationDecorations: 'editable'`).
    let in_editor = |cx: &mut VisualTestContext| {
        editor.update(cx, |e, cx| {
            e.decorations_in_range(p(0, 0)..p(2, 5), cx).len()
        })
    };
    assert_eq!(in_editor(cx), 3);
    editor.update(cx, |e, cx| {
        e.update_options_with(|o| o.read_only = true, cx)
    });
    assert_eq!(in_editor(cx), 2);
}

/// Monaco `findMatches` merges only intersecting search scopes, so no
/// match spans two touching ones.
#[gpui::test]
fn find_matches_keeps_touching_scopes_apart(cx: &mut TestAppContext) {
    let model = cx.new(|_| TextModel::new("abab"));
    model.read_with(cx, |m, _| {
        let find = |scope: &[std::ops::Range<PointUtf16>]| {
            m.find_matches_in("ba", scope, false, true, None, false, 10)
                .into_iter()
                .map(|found| found.range)
                .collect::<Vec<_>>()
        };
        assert!(find(&[p(0, 0)..p(0, 2), p(0, 2)..p(0, 4)]).is_empty());
        assert_eq!(
            find(&[p(0, 0)..p(0, 3), p(0, 2)..p(0, 4)]),
            [p(0, 1)..p(0, 3)]
        );
        assert_eq!(find(&[p(0, 0)..p(0, 4)]), [p(0, 1)..p(0, 3)]);
    });
}

struct Blank {
    width: f32,
    height: f32,
}

impl Render for Blank {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div().w(px(self.width)).h(px(self.height)).child("")
    }
}

/// Monaco `IMouseTarget`'s `detail`, `range` and `mouseColumn`: which zone
/// or widget, the margin, the distance past a line's end.
#[gpui::test]
fn mouse_target_details(cx: &mut TestAppContext) {
    let (editor, cx) = setup("short\na much longer line\nthird", cx);
    draw(cx);
    let zone_view = cx.update(|_, cx| {
        cx.new(|_| Blank {
            width: 10.,
            height: 10.,
        })
    });
    let overlay_view = cx.update(|_, cx| {
        cx.new(|_| Blank {
            width: 40.,
            height: 20.,
        })
    });
    let (zone, overlay) = editor.update(cx, |e, cx| {
        let zone = e.add_view_zone(
            ViewZone::new(1, ZoneHeight::Pixels(px(40.)), zone_view.into()),
            cx,
        );
        let overlay = e.add_overlay_widget(
            OverlayWidget::new(OverlayPosition::BottomRight, overlay_view.into()),
            cx,
        );
        (zone, overlay)
    });
    draw(cx);
    let target = |at, cx: &mut VisualTestContext| {
        editor.update_in(cx, |e, window, cx| e.mouse_target(at, window, cx))
    };
    let (_, widgets) = editor.read_with(cx, |e, _| e.widget_bounds_for_tests());
    let bounds = |kind| {
        widgets
            .iter()
            .find(|(_, k)| *k == kind)
            .map(|(bounds, _)| *bounds)
            .unwrap()
    };
    use gpui_monaco::mouse_target::EmbeddedKind;
    let t = target(bounds(EmbeddedKind::ViewZone).center(), cx);
    assert_eq!(t.kind, MouseTargetKind::ContentViewZone);
    assert_eq!(
        t.detail,
        MouseTargetDetail::ViewZone {
            id: Some(zone),
            after_line: Some(1),
            position_before: Some(p(0, 5)),
            position_after: Some(p(1, 0)),
        }
    );
    // Monaco `getZoneAtCoord`: the lower half is the position after it.
    assert_eq!(t.position, Some(p(1, 0)));
    let upper = bounds(EmbeddedKind::ViewZone).top() + px(2.);
    let t = target(point(bounds(EmbeddedKind::ViewZone).center().x, upper), cx);
    assert_eq!(t.position, Some(p(0, 5)));
    let t = target(bounds(EmbeddedKind::OverlayWidget).center(), cx);
    assert_eq!(
        (t.kind, t.detail),
        (
            MouseTargetKind::OverlayWidget,
            MouseTargetDetail::OverlayWidget(overlay)
        )
    );

    // Past the end of "short": how far, and the mouse column there.
    let start = editor
        .read_with(cx, |e, _| e.scrolled_visible_position(p(0, 0)))
        .unwrap();
    let end = editor
        .read_with(cx, |e, _| e.scrolled_visible_position(p(0, 5)))
        .unwrap();
    let char_width = (end.x - start.x) / 5.;
    let past = point(end.x + char_width * 3. + px(1.), end.y + px(2.));
    let t = target(past, cx);
    let MouseTargetDetail::ContentEmpty {
        is_after_lines: false,
        horizontal_distance_to_text: Some(distance),
    } = t.detail
    else {
        panic!("{t:?}");
    };
    assert!((distance - (char_width * 3. + px(1.))).abs() < px(0.5));
    assert_eq!(t.mouse_column, 8);
    assert_eq!(t.range, Some(p(0, 5)..p(0, 5)));
    // The gutter: the whole line, and the margin's layout.
    let t = target(point(px(2.), end.y + px(2.)), cx);
    assert_eq!(t.kind, MouseTargetKind::GutterGlyphMargin);
    assert_eq!(t.range, Some(p(0, 0)..p(0, 5)));
    let MouseTargetDetail::Margin {
        glyph_margin_lane,
        offset_x,
        is_after_lines,
        ..
    } = t.detail
    else {
        panic!("{t:?}");
    };
    assert_eq!((glyph_margin_lane, is_after_lines), (Some(1), false));
    assert_eq!(
        offset_x,
        px(2.) - editor.read_with(cx, |e, _| e.widget_bounds_for_tests().0.left())
    );

    // Monaco `getScrolledVisiblePosition` is relative to the editor.
    let (editor_bounds, _) = editor.read_with(cx, |e, _| e.widget_bounds_for_tests());
    let relative = editor
        .read_with(cx, |e, _| e.scrolled_visible_position_in_editor(p(0, 5)))
        .unwrap();
    assert_eq!(relative.left, end.x - editor_bounds.left());
    assert_eq!(relative.top, end.y - editor_bounds.top());
    let font = editor.read_with(cx, |e, _| e.font_info()).unwrap();
    assert_eq!(relative.height, font.line_height);
}

/// Answers definitions from a fixed list.
struct Definitions(Vec<Location>);

impl DefinitionProvider for Definitions {
    fn provide_definition(
        &self,
        _: &ProviderContext,
        _: PointUtf16,
        _: &mut App,
    ) -> Task<anyhow::Result<Vec<Location>>> {
        Task::ready(Ok(self.0.clone()))
    }
}

/// Monaco `registerLinkOpener` and `registerEditorOpener`: hosts open links
/// and other models once for every editor; what they don't open is still
/// reported by the editor.
#[gpui::test]
fn global_openers(cx: &mut TestAppContext) {
    let elsewhere = Location::new(Some("file:///other.txt".into()), p(3, 1)..p(3, 4));
    cx.update(|cx| {
        gpui_monaco::init(cx);
        register_definition_provider(cx, LanguageSelector::All, Definitions(vec![elsewhere]));
    });
    let (editor, cx) = setup("https://handled.io https://other.io\nword", cx);
    let reported = record(
        &editor,
        |e| {
            matches!(
                e,
                CodeEditorEvent::OpenLink(_) | CodeEditorEvent::OpenLocation(_)
            )
        },
        cx,
    );
    let links = Rc::new(RefCell::new(Vec::new()));
    let sink = links.clone();
    let link_opener = cx.update(|_, cx| {
        gpui_monaco::register_link_opener(cx, move |url, _| {
            sink.borrow_mut().push(url.to_string());
            url.contains("handled")
        })
    });
    let opened = Rc::new(RefCell::new(Vec::new()));
    let sink = opened.clone();
    let editor_opener = cx.update(|_, cx| {
        gpui_monaco::register_editor_opener(cx, move |source, uri, selection, cx| {
            // It may update the editor that asked.
            let model = source.read(cx).model().clone();
            assert_eq!(
                model.read(cx).uri().map(|u| u.as_ref()),
                Some("file:///a.txt")
            );
            sink.borrow_mut().push((uri.to_string(), selection));
            true
        })
    });
    cx.run_until_parked();
    let open_link_at = |column, cx: &mut VisualTestContext| {
        editor.update(cx, |e, cx| {
            e.set_position(p(0, column), cx);
            e.open_link(cx);
        });
        cx.run_until_parked();
    };
    open_link_at(3, cx);
    assert_eq!(*links.borrow(), ["https://handled.io"]);
    assert!(reported.borrow().is_empty());
    // Not opened: the editor reports it.
    open_link_at(22, cx);
    assert_eq!(
        reported.borrow().as_slice(),
        [CodeEditorEvent::OpenLink("https://other.io".into())]
    );

    // Go to definition in another model.
    editor.update(cx, |e, cx| e.set_position(p(1, 1), cx));
    editor.update_in(cx, |e, window, cx| {
        e.trigger("editor.action.revealDefinition", window, cx)
            .unwrap()
    });
    cx.run_until_parked();
    assert_eq!(
        *opened.borrow(),
        [("file:///other.txt".to_string(), Some(p(3, 1)..p(3, 4)))]
    );
    assert_eq!(reported.borrow().len(), 1);
    // Without openers, the events again.
    cx.update(|_, cx| {
        link_opener.dispose(cx);
        editor_opener.dispose(cx);
    });
    open_link_at(3, cx);
    assert_eq!(reported.borrow().len(), 2);
}

/// Monaco `createModel` and `setModelLanguage` take a MIME type too.
#[gpui::test]
fn languages_by_mime_type(cx: &mut TestAppContext) {
    cx.update(|cx| gpui_monaco::set_language_registry(cx, gpui_monaco_languages::registry()));
    let model = cx.new(|_| TextModel::new("{}").with_language("application/json"));
    model.update(cx, |m, cx| m.ensure_syntax(cx));
    assert_eq!(
        model.read_with(cx, |m, _| m.language_id().cloned()),
        Some("json".into())
    );
    let events = Rc::new(Cell::new(0));
    let seen = events.clone();
    let _subscription = cx.update(|cx| {
        cx.subscribe(&model, move |_, event: &TextModelEvent, _| {
            if matches!(event, TextModelEvent::LanguageChanged { .. }) {
                seen.set(seen.get() + 1);
            }
        })
    });
    // The MIME type of the language it has: no change.
    model.update(cx, |m, cx| {
        m.set_language(Some("application/json".into()), cx)
    });
    assert_eq!(events.get(), 0);
    model.update(cx, |m, cx| {
        m.set_language(Some("text/javascript".into()), cx)
    });
    assert_eq!(
        model.read_with(cx, |m, _| m.language_id().cloned()),
        Some("javascript".into())
    );
    assert_eq!(events.get(), 1);
}

/// Monaco `getConfiguredWordAtPosition` (the editor's `wordSeparators`),
/// `getVisibleRangesPlusViewportAboveBelow` and the editor's `getValue`.
#[gpui::test]
fn editor_conveniences(cx: &mut TestAppContext) {
    let (editor, cx) = setup(&format!("foo-bar baz\n{}", lines(200)), cx);
    let word = editor
        .read_with(cx, |e, cx| e.configured_word_at_position(p(0, 5), cx))
        .unwrap();
    assert_eq!((word.word.as_str(), word.range), ("bar", p(0, 4)..p(0, 7)));
    editor.update(cx, |e, cx| {
        e.update_options_with(|o| o.word_separators = " ".into(), cx)
    });
    let word = editor
        .read_with(cx, |e, cx| e.configured_word_at_position(p(0, 5), cx))
        .unwrap();
    assert_eq!(word.word, "foo-bar");

    draw(cx);
    let (visible, around) = editor.read_with(cx, |e, cx| {
        (
            e.visible_ranges(cx),
            e.visible_ranges_plus_viewport_above_below(cx),
        )
    });
    let (first, last) = (visible[0].start.row, visible.last().unwrap().end.row);
    assert_eq!(around[0].start.row, 0);
    assert!(around.last().unwrap().end.row >= last + (last - first) / 2);

    editor.update(cx, |e, cx| e.set_value("a\nb", cx));
    assert_eq!(
        editor.read_with(cx, |e, cx| e.value(Eol::Crlf, false, cx)),
        "a\r\nb"
    );
}
