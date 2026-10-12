//! Monaco's editor API details (#349): free-form cursor change sources,
//! decoration events from edits and their absence for no-op deltas, the
//! line height and font events, editor and model ids, action descriptors
//! with arguments and `keybindingContext`, `getAction`, the language
//! change source, `onWillTriggerEditorOperationEvent` and
//! `onDropIntoEditor`, and the decoration getters' filters.

use std::cell::RefCell;
use std::rc::Rc;

use gpui::{
    AppContext as _, Entity, Focusable as _, FontWeight, TestAppContext, VisualTestContext,
};
use gpui_monaco::cursor::Selection;
use gpui_monaco::text::PointUtf16;
use gpui_monaco::{
    CodeEditor, CodeEditorEvent, CursorChangeSource, DecorationFilter, DecorationOptions,
    DecorationsChange, DiffEditor, EditorActionDescriptor, EditorType, FontChange, KeybindingRule,
    LineHeightChange, MinimapDecoration, TextModel, TextModelEvent,
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
    focus(&editor, cx);
    (editor, cx)
}

fn focus(editor: &Entity<CodeEditor>, cx: &mut VisualTestContext) {
    cx.update(|window, cx| window.focus(&editor.focus_handle(cx), cx));
    cx.update(|window, _| window.refresh());
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

/// Records the model's events that `keep` picks.
fn record_model(
    model: &Entity<TextModel>,
    keep: impl Fn(&TextModelEvent) -> bool + 'static,
    cx: &mut VisualTestContext,
) -> Rc<RefCell<Vec<TextModelEvent>>> {
    let events = Rc::new(RefCell::new(Vec::new()));
    let sink = events.clone();
    cx.update(|_, cx| {
        cx.subscribe(model, move |_, event: &TextModelEvent, _| {
            if keep(event) {
                sink.borrow_mut().push(event.clone());
            }
        })
        .detach();
    });
    events
}

/// The sources of the `SelectionsChanged` events recorded.
fn sources(events: &Rc<RefCell<Vec<CodeEditorEvent>>>) -> Vec<CursorChangeSource> {
    events
        .borrow()
        .iter()
        .filter_map(|e| match e {
            CodeEditorEvent::SelectionsChanged(change) => Some(change.source.clone()),
            _ => None,
        })
        .collect()
}

fn selections_changed(e: &CodeEditorEvent) -> bool {
    matches!(e, CodeEditorEvent::SelectionsChanged(_))
}

fn decorations_changed(e: &TextModelEvent) -> bool {
    matches!(e, TextModelEvent::DecorationsChanged(_))
}

fn glyph() -> DecorationOptions {
    DecorationOptions {
        glyph: Some(("●".into(), gpui::red())),
        ..Default::default()
    }
}

// ----- cursor change sources ---------------------------------------------

/// Monaco's cursor event `source` is a free string: Monaco's own names
/// have variants, anything else is kept as given.
#[test]
fn cursor_change_sources_convert_from_strings() {
    use CursorChangeSource::*;
    for (name, source) in [
        ("keyboard", Keyboard),
        ("mouse", Mouse),
        ("api", Api),
        ("model", Model),
        ("modelChange", ModelChange),
        ("restoreState", RestoreState),
    ] {
        assert_eq!(CursorChangeSource::from(name), source);
        assert_eq!(source.as_str(), name);
    }
    // Monaco `source || 'keyboard'`.
    assert_eq!(CursorChangeSource::from(""), Keyboard);
    let mine = CursorChangeSource::from("my-extension");
    assert_eq!(mine, Other("my-extension".into()));
    assert_eq!(mine.to_string(), "my-extension");
    assert_eq!(CursorChangeSource::from(String::from("api")), Api);
}

/// The usual Monaco pattern: `executeEdits('my-ext', …)` and
/// `setSelection(s, 'my-ext')`, then ignore `e.source === 'my-ext'`.
#[gpui::test]
fn hosts_tag_their_own_cursor_changes(cx: &mut TestAppContext) {
    let (editor, cx) = setup("hello\nworld", cx);
    let events = record(&editor, selections_changed, cx);
    editor.update(cx, |e, cx| {
        e.set_selections_from("my-ext", &[Selection::collapsed(p(1, 1))], cx);
        e.set_selection_from("my-ext", Selection::new(p(0, 0), p(0, 2)), cx);
        e.set_position_from("my-ext", p(0, 4), cx);
        e.execute_edits_from(
            "my-ext",
            vec![(p(0, 0)..p(0, 0), "> ".into())],
            Some(vec![Selection::collapsed(p(0, 2))]),
            cx,
        );
        // Without a source, the API's.
        e.set_position(p(1, 0), cx);
        e.execute_edits(
            vec![(p(1, 0)..p(1, 0), "!".into())],
            Some(vec![Selection::collapsed(p(1, 1))]),
            cx,
        );
    });
    let mine = CursorChangeSource::Other("my-ext".into());
    assert_eq!(
        sources(&events),
        [
            mine.clone(),
            mine.clone(),
            mine.clone(),
            mine,
            CursorChangeSource::Api,
            CursorChangeSource::Api,
        ]
    );
}

/// Monaco `trigger(source, handlerId, payload)`: the source reaches
/// `onWillTriggerEditorOperationEvent` and the cursor events of the
/// command it runs.
#[gpui::test]
fn trigger_reports_its_source(cx: &mut TestAppContext) {
    let (editor, cx) = setup("hello\nworld", cx);
    let events = record(
        &editor,
        |e| {
            matches!(
                e,
                CodeEditorEvent::SelectionsChanged(_)
                    | CodeEditorEvent::WillTriggerEditorOperation(_)
            )
        },
        cx,
    );
    cx.update(|window, cx| {
        editor.update(cx, |e, cx| {
            e.trigger_from("my-ext", "cursorEnd", None, window, cx)
                .unwrap()
        })
    });
    cx.run_until_parked();
    {
        let events = events.borrow();
        let [
            CodeEditorEvent::WillTriggerEditorOperation(trigger),
            CodeEditorEvent::SelectionsChanged(change),
        ] = events.as_slice()
        else {
            panic!("{events:?}");
        };
        assert_eq!(trigger.source, CursorChangeSource::Other("my-ext".into()));
        assert_eq!(trigger.handler_id.as_ref(), "cursorEnd");
        assert_eq!(trigger.payload, None);
        assert_eq!(change.source, CursorChangeSource::Other("my-ext".into()));
        assert_eq!(change.selection, Selection::collapsed(p(0, 5)));
    }
    events.borrow_mut().clear();
    // `trigger` is the keyboard's, Monaco's usual `trigger('keyboard', …)`.
    cx.update(|window, cx| editor.update(cx, |e, cx| e.trigger("cursorDown", window, cx).unwrap()));
    cx.run_until_parked();
    assert_eq!(sources(&events), [CursorChangeSource::Keyboard]);
    let events = events.borrow();
    let CodeEditorEvent::WillTriggerEditorOperation(trigger) = &events[0] else {
        panic!("{events:?}");
    };
    assert_eq!(trigger.source, CursorChangeSource::Keyboard);
}

// ----- decoration events -------------------------------------------------

/// Monaco `modelDecorations.test.ts`' eventing tests: adding, changing and
/// removing decorations fire once, as does an edit while the model has
/// decorations, and a no-op `deltaDecorations` doesn't.
#[gpui::test]
fn decoration_events(cx: &mut TestAppContext) {
    let (editor, cx) = setup("My First Line\n\t\tMy Second Line\n    Third Line\n\n1", cx);
    let model = editor.read_with(cx, |e, _| e.model().clone());
    let events = record_model(&model, decorations_changed, cx);
    let count = |events: &Rc<RefCell<Vec<TextModelEvent>>>| events.borrow().len();

    // 'decorations do not emit event on no-op deltaDecorations'
    model.update(cx, |m, cx| {
        assert!(m.delta_decorations(None, &[], Vec::new(), cx).is_empty());
    });
    assert_eq!(count(&events), 0);
    // An edit with no decorations has nothing to tell.
    model.update(cx, |m, cx| m.edit([(0..0, "x")], cx).unwrap());
    assert_eq!(count(&events), 0);

    // 'decorations emit event on add'
    let ids = model.update(cx, |m, cx| {
        m.delta_decorations_at(None, &[], vec![(p(0, 1)..p(2, 1), glyph())], cx)
    });
    assert_eq!(count(&events), 1);

    // 'decorations emit event on change'
    model.update(cx, |m, cx| {
        let options = DecorationOptions {
            line_number_color: Some(gpui::blue()),
            ..glyph()
        };
        m.change_decoration_options(ids[0], options, cx)
    });
    assert_eq!(count(&events), 2);

    // 'decorations emit event when inserting one line text before it'
    events.borrow_mut().clear();
    model.update(cx, |m, cx| m.edit([(0..0, "Hallo ")], cx).unwrap());
    {
        let events = events.borrow();
        let [TextModelEvent::DecorationsChanged(change)] = events.as_slice() else {
            panic!("{events:?}");
        };
        // Monaco's `fire()`: what moving decorations affects.
        assert_eq!(
            *change,
            DecorationsChange {
                affects_minimap: true,
                affects_overview_ruler: true,
                affects_glyph_margin: true,
                affects_line_number: false,
            }
        );
    }
    // Undo and redo move them too.
    model.update(cx, |m, cx| {
        m.undo(None, cx);
        m.redo(None, cx);
    });
    assert_eq!(count(&events), 3);

    // 'decorations emit event on remove'
    events.borrow_mut().clear();
    model.update(cx, |m, cx| m.delta_decorations(None, &ids, Vec::new(), cx));
    assert_eq!(count(&events), 1);
    // Removing it again is a no-op.
    model.update(cx, |m, cx| m.delta_decorations(None, &ids, Vec::new(), cx));
    editor.update(cx, |e, cx| e.remove_decorations(&ids, cx));
    assert_eq!(count(&events), 1);
}

/// `IEditorDecorationsCollection.onDidChange` follows typing (Monaco
/// fires the decorations event for every edit), and the collection's own
/// no-op changes don't make it skip the next real one.
#[gpui::test]
fn collections_hear_about_edits(cx: &mut TestAppContext) {
    let (editor, cx) = setup("one\ntwo", cx);
    let model = editor.read_with(cx, |e, _| e.model().clone());
    let mut collection = editor.update(cx, |e, cx| e.create_decorations_collection(Vec::new(), cx));
    let heard = Rc::new(RefCell::new(0));
    let sink = heard.clone();
    let _subscription = cx
        .update(|_, cx| collection.on_did_change(move |_, _| *sink.borrow_mut() += 1, cx))
        .unwrap();
    // Nothing to clear: no event, and nothing to skip.
    cx.update(|_, cx| collection.clear(cx));
    cx.run_until_parked();
    model.update(cx, |m, cx| {
        m.delta_decorations_at(None, &[], vec![(p(1, 0)..p(1, 3), glyph())], cx)
    });
    cx.run_until_parked();
    assert_eq!(*heard.borrow(), 1);
    // Its own change isn't reported to it; typing before it is.
    cx.update(|_, cx| collection.set(vec![(p(1, 0)..p(1, 1), glyph())], cx));
    cx.run_until_parked();
    assert_eq!(*heard.borrow(), 1);
    // (The editor may also change its own decorations while typing.)
    editor.update(cx, |e, cx| e.type_text("a\n", cx));
    cx.run_until_parked();
    assert!(*heard.borrow() >= 2);
    cx.update(|_, cx| assert_eq!(collection.ranges(cx), vec![p(2, 0)..p(2, 1)]));
}

/// Monaco `onDidChangeLineHeight` and `onDidChangeFont`: the model reports
/// decorations that set a line height or a font being added, changed and
/// removed; an editor forwards those it shows.
#[gpui::test]
fn line_height_and_font_events(cx: &mut TestAppContext) {
    let (editor, cx) = setup("a\nb\nc", cx);
    let model = editor.read_with(cx, |e, _| e.model().clone());
    let model_events = record_model(
        &model,
        |e| {
            matches!(
                e,
                TextModelEvent::LineHeightChanged(_) | TextModelEvent::FontChanged(_)
            )
        },
        cx,
    );
    let editor_events = record(
        &editor,
        |e| {
            matches!(
                e,
                CodeEditorEvent::LineHeightChanged(_) | CodeEditorEvent::FontChanged(_)
            )
        },
        cx,
    );
    let tall = DecorationOptions {
        line_height: Some(2.),
        ..Default::default()
    };
    let bold = DecorationOptions {
        font_weight: Some(FontWeight::BOLD),
        ..Default::default()
    };
    let ids = model.update(cx, |m, cx| {
        m.delta_decorations_at(
            None,
            &[],
            vec![(p(1, 0)..p(1, 1), tall.clone()), (p(2, 0)..p(2, 1), bold)],
            cx,
        )
    });
    {
        let events = model_events.borrow();
        let [
            TextModelEvent::LineHeightChanged(heights),
            TextModelEvent::FontChanged(fonts),
        ] = events.as_slice()
        else {
            panic!("{events:?}");
        };
        assert_eq!(
            heights.as_ref(),
            [LineHeightChange {
                owner: None,
                decoration_id: ids[0],
                row: 1,
                line_height: Some(2.),
            }]
        );
        assert_eq!(
            fonts.as_ref(),
            [FontChange {
                owner: None,
                row: 2
            }]
        );
    }
    assert_eq!(editor_events.borrow().len(), 2);
    // A change of options, then a removal (no height any more).
    model_events.borrow_mut().clear();
    model.update(cx, |m, cx| {
        m.change_decoration_options(
            ids[0],
            DecorationOptions {
                line_height: Some(3.),
                ..Default::default()
            },
            cx,
        );
        m.delta_decorations(None, &ids[..1], Vec::new(), cx);
    });
    let heights: Vec<Option<f32>> = model_events
        .borrow()
        .iter()
        .flat_map(|e| match e {
            TextModelEvent::LineHeightChanged(changes) => {
                changes.iter().map(|c| c.line_height).collect::<Vec<_>>()
            }
            _ => Vec::new(),
        })
        .collect();
    assert_eq!(heights, [Some(3.), None]);
    // Another editor's decorations aren't this editor's (Monaco filters by
    // owner).
    editor_events.borrow_mut().clear();
    let other = cx.update(|window, cx| cx.new(|cx| CodeEditor::new(model.clone(), window, cx)));
    let other_id = other.entity_id();
    model.update(cx, |m, cx| {
        m.delta_decorations_at(Some(other_id), &[], vec![(p(0, 0)..p(0, 1), tall)], cx)
    });
    assert!(editor_events.borrow().is_empty());
}

// ----- ids -----------------------------------------------------------------

/// Monaco `getId`/`getEditorType` and `ITextModel.id`.
#[gpui::test]
fn editor_and_model_ids(cx: &mut TestAppContext) {
    let (editor, cx) = setup("a", cx);
    let other = cx.update(|window, cx| cx.new(|cx| CodeEditor::with_text("b", window, cx)));
    let (id, other_id) = (
        editor.read_with(cx, |e, _| e.id()),
        other.read_with(cx, |e, _| e.id()),
    );
    assert!(id.starts_with("vs.editor.ICodeEditor:"), "{id}");
    assert_ne!(id, other_id);
    assert_eq!(
        editor.read_with(cx, |e, _| e.editor_type()),
        EditorType::CodeEditor
    );
    assert_eq!(EditorType::CodeEditor.as_str(), "vs.editor.ICodeEditor");

    let model = editor.read_with(cx, |e, _| e.model().clone());
    let other_model = other.read_with(cx, |e, _| e.model().clone());
    let model_id = model.read_with(cx, |m, _| m.id().clone());
    assert!(model_id.starts_with("$model"), "{model_id}");
    assert_ne!(model_id, other_model.read_with(cx, |m, _| m.id().clone()));
    // A model without a uri has the same number in its `inmemory://` one.
    let number = other_model.read_with(cx, |m, _| m.id()["$model".len()..].to_string());
    assert_eq!(
        other_model.read_with(cx, |m, _| m.resource().to_string()),
        format!("inmemory://model/{number}")
    );

    let diff = cx.update(|window, cx| {
        let original = cx.new(|_| TextModel::new("a"));
        let modified = cx.new(|_| TextModel::new("b"));
        cx.new(|cx| DiffEditor::new(original, modified, window, cx))
    });
    let diff_id = diff.read_with(cx, |d, _| d.id());
    assert!(
        diff_id.starts_with("vs.editor.IDiffEditor:v2:"),
        "{diff_id}"
    );
    assert_eq!(
        diff.read_with(cx, |d, _| d.editor_type()),
        EditorType::DiffEditor
    );
}

// ----- actions -------------------------------------------------------------

/// Monaco `IActionDescriptor.run(editor, ...args)`: `trigger`'s payload, a
/// keybinding rule's `commandArgs` and `getAction(id).run(args)` reach the
/// action.
#[gpui::test]
fn actions_get_their_arguments(cx: &mut TestAppContext) {
    let (editor, cx) = setup("text", cx);
    let args = Rc::new(RefCell::new(Vec::new()));
    let sink = args.clone();
    let _action = editor.update(cx, |e, cx| {
        e.add_action(
            EditorActionDescriptor::new_with_args("my.action", "My Action", move |_, a, _, _| {
                sink.borrow_mut().push(a)
            }),
            cx,
        )
    });
    let payload = serde_json::json!({ "n": 1 });
    cx.update(|window, cx| {
        editor.update(cx, |e, cx| {
            e.trigger_with_payload("my.action", payload.clone(), window, cx)
                .unwrap()
        })
    });
    // `getAction`: its names, whether it can run, and running it.
    let action = editor
        .update(cx, |e, cx| e.get_action("my.action", cx))
        .unwrap();
    assert_eq!(action.id().as_ref(), "my.action");
    assert_eq!(action.label().as_ref(), "My Action");
    assert!(cx.update(|_, cx| action.is_supported(cx)));
    assert!(cx.update(|window, cx| action.run(Some(serde_json::json!(2)), window, cx)));
    // A keybinding rule's arguments.
    let _rule = cx.update(|_, cx| {
        gpui_monaco::add_keybinding_rule(
            cx,
            KeybindingRule::new("ctrl-alt-j", "my.action").args(serde_json::json!("rule")),
        )
        .unwrap()
    });
    cx.simulate_keystrokes("ctrl-alt-j");
    assert_eq!(
        *args.borrow(),
        [
            Some(payload),
            Some(serde_json::json!(2)),
            Some(serde_json::json!("rule")),
        ]
    );

    // Built-in actions are there whether they can run or not (Monaco
    // `getActions`), and aliases resolve.
    let all = editor.read_with(cx, |e, _| e.actions());
    assert!(
        all.iter()
            .any(|a| a.id.as_ref() == "editor.action.commentLine")
    );
    assert!(all.iter().any(|a| a.id.as_ref() == "my.action"));
    let anchor = editor
        .update(cx, |e, cx| {
            e.get_action("editor.action.goToSelectionAnchor", cx)
        })
        .unwrap();
    assert!(!cx.update(|_, cx| anchor.is_supported(cx)));
    assert!(editor.update(cx, |e, cx| e.get_action("no.such.action", cx).is_none()));
}

/// Monaco `keybindingContext`: the action's keybindings apply only where
/// it holds.
#[gpui::test]
fn action_keybinding_context(cx: &mut TestAppContext) {
    let (editor, cx) = setup("text", cx);
    let ran = Rc::new(RefCell::new(0));
    let sink = ran.clone();
    let _action = editor.update(cx, |e, cx| {
        e.add_action(
            EditorActionDescriptor::new("my.write", "Write", move |_, _, _| {
                *sink.borrow_mut() += 1
            })
            .keybinding("ctrl-alt-w")
            .keybinding_context("!editorReadonly"),
            cx,
        )
    });
    cx.simulate_keystrokes("ctrl-alt-w");
    assert_eq!(*ran.borrow(), 1);
    editor.update(cx, |e, cx| {
        let mut options = e.options().clone();
        options.read_only = true;
        e.update_options(options, cx);
    });
    focus(&editor, cx);
    cx.simulate_keystrokes("ctrl-alt-w");
    assert_eq!(*ran.borrow(), 1);
    // `run_action` still runs it: the context is the keybindings'.
    cx.update(|window, cx| editor.update(cx, |e, cx| e.run_action("my.write", window, cx)));
    assert_eq!(*ran.borrow(), 2);
}

// ----- other events --------------------------------------------------------

/// Monaco `onDropIntoEditor`, before the drop providers run.
#[gpui::test]
fn drop_into_editor_event(cx: &mut TestAppContext) {
    let (editor, cx) = setup("hello\nworld", cx);
    let events = record(
        &editor,
        |e| matches!(e, CodeEditorEvent::DropIntoEditor(_)),
        cx,
    );
    let path = std::path::PathBuf::from("/tmp/dropped.txt");
    editor.update(cx, |e, cx| e.drop_files(&[path], p(1, 2), cx));
    let events = events.borrow();
    let [CodeEditorEvent::DropIntoEditor(drop)] = events.as_slice() else {
        panic!("{events:?}");
    };
    assert_eq!(drop.position, p(1, 2));
    assert!(drop.data.matches("text/uri-list"));
}

// ----- decoration getters ----------------------------------------------------

/// Monaco's `filterFontDecorations`, `onlyMinimapDecorations` and
/// `onlyMarginDecorations`, and `getCustomLineHeightsDecorations(InRange)`.
#[gpui::test]
fn decoration_getter_filters(cx: &mut TestAppContext) {
    let (editor, cx) = setup("a\nb\nc\nd", cx);
    let model = editor.read_with(cx, |e, _| e.model().clone());
    let ids = model.update(cx, |m, cx| {
        m.delta_decorations_at(
            None,
            &[],
            vec![
                (
                    p(0, 0)..p(0, 1),
                    DecorationOptions {
                        font_weight: Some(FontWeight::BOLD),
                        ..Default::default()
                    },
                ),
                (
                    p(1, 0)..p(1, 1),
                    DecorationOptions {
                        minimap: Some(MinimapDecoration::default()),
                        ..Default::default()
                    },
                ),
                (p(2, 0)..p(2, 1), glyph()),
                (
                    p(3, 0)..p(3, 1),
                    DecorationOptions {
                        line_height: Some(2.),
                        ..Default::default()
                    },
                ),
            ],
            cx,
        )
    });
    let all = p(0, 0)..p(3, 1);
    let query = |filter: DecorationFilter, cx: &mut VisualTestContext| {
        model.read_with(cx, |m, _| {
            m.decorations_in_point_range(all.clone(), None, filter)
                .into_iter()
                .map(|d| d.id)
                .collect::<Vec<_>>()
        })
    };
    assert_eq!(query(DecorationFilter::default(), cx), ids);
    let no_fonts = DecorationFilter {
        filter_font_decorations: true,
        ..Default::default()
    };
    assert_eq!(query(no_fonts, cx), ids[1..]);
    let minimap = DecorationFilter {
        only_minimap_decorations: true,
        ..Default::default()
    };
    assert_eq!(query(minimap, cx), [ids[1]]);
    let margin = DecorationFilter {
        only_margin_decorations: true,
        ..Default::default()
    };
    assert_eq!(query(margin, cx), [ids[2]]);
    // A `bool` is still `filterOutValidation`.
    assert_eq!(
        model.read_with(cx, |m, _| m.all_model_decorations(None, true).len()),
        4
    );
    assert_eq!(
        model.read_with(cx, |m, _| m
            .custom_line_heights_decorations(None)
            .into_iter()
            .map(|d| d.id)
            .collect::<Vec<_>>()),
        [ids[3]]
    );
    assert!(model.read_with(cx, |m, _| {
        m.custom_line_heights_decorations_in_range(p(0, 0)..p(2, 1), None)
            .is_empty()
    }));
}
