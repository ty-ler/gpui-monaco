//! Monaco's global `monaco.editor.*` registries and events
//! (`standaloneEditor.ts`): editors, diff editors and models with their
//! events, markers across models (ported from `markerService.test.ts`),
//! themes by name, global commands, actions and keybinding rules, and
//! `colorizeModelLine`.

use std::cell::RefCell;
use std::rc::Rc;

use gpui::{
    AppContext as _, Entity, EntityId, Focusable, SharedString, TestAppContext, VisualTestContext,
};
use gpui_monaco::text::PointUtf16;
use gpui_monaco::{
    CodeEditor, DiffEditor, EditorActionDescriptor, KeybindingRule, Marker, MarkerFilter,
    MarkerSeverity, TextModel, TextModelEvent, Theme,
};

/// What listeners saw.
type Log<T> = Rc<RefCell<Vec<T>>>;

fn setup<'a>(
    text: &str,
    cx: &'a mut TestAppContext,
) -> (Entity<CodeEditor>, &'a mut VisualTestContext) {
    cx.update(gpui_monaco::init);
    let text = text.to_string();
    let (editor, cx) = cx.add_window_view(|window, cx| {
        let model = cx.new(|_| TextModel::new(&text).with_uri("file:///main.rs"));
        CodeEditor::new(model, window, cx)
    });
    cx.update(|window, cx| window.focus(&editor.focus_handle(cx), cx));
    cx.run_until_parked();
    (editor, cx)
}

/// Flushes effects, so dropped entities are released.
fn flush(cx: &mut VisualTestContext) {
    cx.update(|_, _| {});
    cx.run_until_parked();
}

fn marker(message: &str) -> Marker {
    Marker {
        severity: MarkerSeverity::Error,
        message: message.to_string().into(),
        range: PointUtf16::new(0, 0)..PointUtf16::new(0, 1),
        ..Default::default()
    }
}

fn model_with_uri(uri: &str, cx: &mut VisualTestContext) -> Entity<TextModel> {
    let uri = uri.to_string();
    cx.update(|_, cx| cx.new(|_| TextModel::new("some text\nmore").with_uri(uri)))
}

fn set(model: &Entity<TextModel>, owner: &str, markers: Vec<Marker>, cx: &mut VisualTestContext) {
    let owner = owner.to_string();
    model.update(cx, |m, cx| m.set_markers(owner, markers, cx));
}

fn read(owner: Option<&str>, resource: Option<&str>, cx: &mut VisualTestContext) -> usize {
    let filter = MarkerFilter {
        owner: owner.map(|o| o.to_string().into()),
        resource: resource.map(|r| r.to_string().into()),
        take: None,
    };
    cx.update(|_, cx| gpui_monaco::model_markers(cx, &filter).len())
}

#[gpui::test]
fn editors_are_listed_and_announced(cx: &mut TestAppContext) {
    let (editor, cx) = setup("one", cx);
    let created: Rc<RefCell<Vec<EntityId>>> = Rc::default();
    let created_diffs: Rc<RefCell<Vec<EntityId>>> = Rc::default();
    let _subscriptions = cx.update(|_, cx| {
        let created = created.clone();
        let created_diffs = created_diffs.clone();
        [
            gpui_monaco::on_did_create_editor(cx, move |editor, cx| {
                // The editor exists and can be read.
                assert!(editor.read(cx).model().read(cx).text().len() < 10);
                created.borrow_mut().push(editor.entity_id());
            }),
            gpui_monaco::on_did_create_diff_editor(cx, move |diff, _| {
                created_diffs.borrow_mut().push(diff.entity_id());
            }),
        ]
    });
    let ids = |cx: &mut VisualTestContext| {
        cx.update(|_, cx| {
            gpui_monaco::editors(cx)
                .iter()
                .map(|e| e.entity_id())
                .collect::<Vec<_>>()
        })
    };
    assert_eq!(ids(cx), vec![editor.entity_id()]);

    let second = cx.update(|window, cx| cx.new(|cx| CodeEditor::with_text("two", window, cx)));
    flush(cx);
    assert_eq!(*created.borrow(), vec![second.entity_id()]);
    assert_eq!(ids(cx), vec![editor.entity_id(), second.entity_id()]);

    // A diff editor creates two code editors (Monaco says so too), then
    // itself.
    let diff = cx.update(|window, cx| {
        let original = cx.new(|_| TextModel::new("a"));
        let modified = cx.new(|_| TextModel::new("b"));
        cx.new(|cx| DiffEditor::new(original, modified, window, cx))
    });
    flush(cx);
    let (original, modified) = diff.read_with(cx, |d, _| {
        (
            d.original_editor().entity_id(),
            d.modified_editor().entity_id(),
        )
    });
    assert_eq!(
        *created.borrow(),
        vec![second.entity_id(), original, modified]
    );
    assert_eq!(*created_diffs.borrow(), vec![diff.entity_id()]);
    assert_eq!(cx.update(|_, cx| gpui_monaco::diff_editors(cx).len()), 1);
    assert_eq!(ids(cx).len(), 4);

    // Released editors leave the lists.
    drop(second);
    drop(diff);
    flush(cx);
    assert_eq!(ids(cx), vec![editor.entity_id()]);
    assert!(cx.update(|_, cx| gpui_monaco::diff_editors(cx).is_empty()));

    // Unsubscribed listeners aren't called.
    drop(_subscriptions);
    let _third = cx.update(|window, cx| cx.new(|cx| CodeEditor::with_text("three", window, cx)));
    flush(cx);
    assert_eq!(created.borrow().len(), 3);
}

#[gpui::test]
fn models_are_listed_and_announced(cx: &mut TestAppContext) {
    let (editor, cx) = setup("one", cx);
    let shown = editor.read_with(cx, |e, _| e.model().clone());
    let created: Rc<RefCell<Vec<EntityId>>> = Rc::default();
    let disposed: Log<(EntityId, Option<SharedString>)> = Rc::default();
    let _subscriptions = cx.update(|_, cx| {
        let created = created.clone();
        let disposed = disposed.clone();
        [
            gpui_monaco::on_did_create_model(cx, move |model, cx| {
                assert_eq!(model.read(cx).text(), "some text\nmore");
                created.borrow_mut().push(model.entity_id());
            }),
            gpui_monaco::on_will_dispose_model(cx, move |id, model, _| {
                disposed.borrow_mut().push((id, model.uri().cloned()));
            }),
        ]
    });
    let model = model_with_uri("inmemory://model/2", cx);
    flush(cx);
    assert_eq!(*created.borrow(), vec![model.entity_id()]);
    let models = cx.update(|_, cx| gpui_monaco::models(cx));
    assert_eq!(
        models.iter().map(|m| m.entity_id()).collect::<Vec<_>>(),
        vec![shown.entity_id(), model.entity_id()]
    );
    // Any live model is found by its uri (Monaco `getModel`).
    let found = cx.update(|_, cx| gpui_monaco::model_for_uri(cx, "inmemory://model/2"));
    assert_eq!(found.map(|m| m.entity_id()), Some(model.entity_id()));

    let id = model.entity_id();
    drop(models);
    drop(model);
    flush(cx);
    assert_eq!(
        *disposed.borrow(),
        vec![(id, Some("inmemory://model/2".into()))]
    );
    assert_eq!(cx.update(|_, cx| gpui_monaco::models(cx).len()), 1);
}

#[gpui::test]
fn models_from_before_init_join_when_shown(cx: &mut TestAppContext) {
    let model = cx.new(|_| TextModel::new("early"));
    assert!(cx.update(|cx| gpui_monaco::models(cx).is_empty()));
    let created = Rc::new(RefCell::new(0));
    let _subscription = cx.update(|cx| {
        let created = created.clone();
        gpui_monaco::on_did_create_model(cx, move |_, _| *created.borrow_mut() += 1)
    });
    let (_editor, cx) = cx.add_window_view(|window, cx| CodeEditor::new(model.clone(), window, cx));
    cx.run_until_parked();
    assert_eq!(*created.borrow(), 1);
    let models = cx.update(|_, cx| gpui_monaco::models(cx));
    assert_eq!(models.len(), 1);
    assert_eq!(models[0].entity_id(), model.entity_id());
}

#[gpui::test]
fn language_changes_report_the_old_language(cx: &mut TestAppContext) {
    let (editor, cx) = setup("one", cx);
    let model = editor.read_with(cx, |e, _| e.model().clone());
    let global: Log<(EntityId, Option<SharedString>)> = Rc::default();
    let local: Log<(Option<SharedString>, Option<SharedString>)> = Rc::default();
    let _subscriptions = cx.update(|_, cx| {
        let global = global.clone();
        let local = local.clone();
        [
            gpui_monaco::on_did_change_model_language(cx, move |model, old, cx| {
                // The model can be read after the change.
                assert!(model.read(cx).language_id().is_some());
                global.borrow_mut().push((model.entity_id(), old.cloned()));
            }),
            cx.subscribe(&model, move |_, event: &TextModelEvent, _| {
                if let TextModelEvent::LanguageChanged {
                    old_language,
                    new_language,
                } = event
                {
                    local
                        .borrow_mut()
                        .push((old_language.clone(), new_language.clone()));
                }
            }),
        ]
    });
    model.update(cx, |m, cx| m.set_language(Some("rust".into()), cx));
    model.update(cx, |m, cx| m.set_language(Some("plaintext".into()), cx));
    // The same language again isn't a change.
    model.update(cx, |m, cx| m.set_language(Some("plaintext".into()), cx));
    flush(cx);
    assert_eq!(
        *global.borrow(),
        vec![
            (model.entity_id(), None),
            (model.entity_id(), Some("rust".into()))
        ]
    );
    assert_eq!(
        *local.borrow(),
        vec![
            (None, Some("rust".into())),
            (Some("rust".into()), Some("plaintext".into()))
        ]
    );
}

// ----- markers (markerService.test.ts) ----------------------------------------

#[gpui::test]
fn markers_query(cx: &mut TestAppContext) {
    let (_, cx) = setup("", cx);
    let file = model_with_uri("file:///c/test/file.cs", cx);
    set(&file, "far", vec![marker("far")], cx);
    assert_eq!(read(None, None, cx), 1);
    assert_eq!(read(Some("far"), None, cx), 1);
    assert_eq!(read(None, Some("file:///c/test/file.cs"), cx), 1);
    assert_eq!(read(Some("far"), Some("file:///c/test/file.cs"), cx), 1);

    set(&file, "boo", vec![marker("boo")], cx);
    assert_eq!(read(None, None, cx), 2);
    assert_eq!(read(Some("far"), None, cx), 1);
    assert_eq!(read(Some("boo"), None, cx), 1);
    assert_eq!(read(Some("boo"), Some("file:///c/test/other.cs"), cx), 0);
}

#[gpui::test]
fn markers_change_one_overrides(cx: &mut TestAppContext) {
    let (_, cx) = setup("", cx);
    let only = model_with_uri("file:///path/only.cs", cx);
    set(&only, "far", vec![marker("a")], cx);
    assert_eq!(read(None, None, cx), 1);
    assert_eq!(read(Some("far"), None, cx), 1);
    set(&only, "boo", vec![marker("b")], cx);
    assert_eq!(read(None, None, cx), 2);
    assert_eq!(read(Some("far"), None, cx), 1);
    assert_eq!(read(Some("boo"), None, cx), 1);
    set(&only, "far", vec![marker("c"), marker("d")], cx);
    assert_eq!(read(Some("far"), None, cx), 2);
    assert_eq!(read(Some("boo"), None, cx), 1);
}

#[gpui::test]
fn markers_change_one_and_all_clear(cx: &mut TestAppContext) {
    let (_, cx) = setup("", cx);
    let only = model_with_uri("file:///path/only.cs", cx);
    set(&only, "far", vec![marker("a")], cx);
    set(&only, "boo", vec![marker("b")], cx);
    assert_eq!(read(Some("far"), None, cx), 1);
    assert_eq!(read(Some("boo"), None, cx), 1);
    assert_eq!(read(None, None, cx), 2);

    set(&only, "far", Vec::new(), cx);
    assert_eq!(read(Some("far"), None, cx), 0);
    assert_eq!(read(Some("boo"), None, cx), 1);
    assert_eq!(read(None, None, cx), 1);

    cx.update(|_, cx| gpui_monaco::remove_all_markers(cx, "boo"));
    assert_eq!(read(Some("far"), None, cx), 0);
    assert_eq!(read(Some("boo"), None, cx), 0);
    assert_eq!(read(None, None, cx), 0);
}

#[gpui::test]
fn removing_all_markers_sends_one_event(cx: &mut TestAppContext) {
    let (_, cx) = setup("", cx);
    let first = model_with_uri("file:///c/path", cx);
    let second = model_with_uri("file:///d/path", cx);
    set(&first, "far", vec![marker("a")], cx);
    set(&second, "far", vec![marker("b")], cx);
    set(&second, "boo", vec![marker("c")], cx);
    assert_eq!(read(Some("far"), None, cx), 2);
    flush(cx);

    let events: Rc<RefCell<Vec<Vec<SharedString>>>> = Rc::default();
    let _subscription = cx.update(|_, cx| {
        let events = events.clone();
        gpui_monaco::on_did_change_markers(cx, move |uris, cx| {
            // The markers are gone when the event arrives.
            let left = gpui_monaco::model_markers(
                cx,
                &MarkerFilter {
                    owner: Some("far".into()),
                    ..Default::default()
                },
            );
            assert!(left.is_empty());
            events.borrow_mut().push(uris.to_vec());
        })
    });
    cx.update(|_, cx| gpui_monaco::remove_all_markers(cx, "far"));
    flush(cx);
    assert_eq!(
        *events.borrow(),
        vec![vec![
            SharedString::from("file:///c/path"),
            SharedString::from("file:///d/path")
        ]]
    );
    assert_eq!(read(Some("boo"), None, cx), 1);

    // Removing nothing isn't a change.
    cx.update(|_, cx| gpui_monaco::remove_all_markers(cx, "far"));
    set(&first, "nobody", Vec::new(), cx);
    flush(cx);
    assert_eq!(events.borrow().len(), 1);

    // Changes in one update arrive together, each uri once.
    cx.update(|_, cx| {
        first.update(cx, |m, cx| m.set_markers("x", vec![marker("1")], cx));
        first.update(cx, |m, cx| m.set_markers("y", vec![marker("2")], cx));
        second.update(cx, |m, cx| m.set_markers("x", vec![marker("3")], cx));
    });
    flush(cx);
    assert_eq!(
        events.borrow()[1],
        vec![
            SharedString::from("file:///c/path"),
            SharedString::from("file:///d/path")
        ]
    );
}

#[gpui::test]
fn invalid_marker_data(cx: &mut TestAppContext) {
    let (_, cx) = setup("", cx);
    let model = model_with_uri("some:uri/path", cx);
    set(&model, "far", vec![marker("")], cx);
    assert_eq!(read(Some("far"), None, cx), 0);

    // An end before the start moves to the start's line.
    let backwards = Marker {
        range: PointUtf16::new(1, 2)..PointUtf16::new(0, 4),
        ..marker("null")
    };
    set(&model, "far", vec![backwards], cx);
    let markers = cx.update(|_, cx| gpui_monaco::model_markers(cx, &MarkerFilter::default()));
    assert_eq!(markers.len(), 1);
    assert_eq!(markers[0].owner.as_ref(), "far");
    assert_eq!(markers[0].resource.as_ref(), "some:uri/path");
    assert_eq!(
        markers[0].marker.range,
        PointUtf16::new(1, 2)..PointUtf16::new(1, 4)
    );
}

#[gpui::test]
fn markers_take_and_order(cx: &mut TestAppContext) {
    let (_, cx) = setup("", cx);
    let a = model_with_uri("file:///a", cx);
    let b = model_with_uri("file:///b", cx);
    set(&b, "two", vec![marker("b2")], cx);
    set(&a, "one", vec![marker("a1"), marker("a2")], cx);
    set(&a, "zero", vec![marker("a0")], cx);
    // Replacing an owner's markers keeps its place.
    set(&a, "one", vec![marker("a1"), marker("a2")], cx);
    let messages = |take: Option<usize>, cx: &mut VisualTestContext| {
        cx.update(|_, cx| {
            gpui_monaco::model_markers(
                cx,
                &MarkerFilter {
                    take,
                    ..Default::default()
                },
            )
            .into_iter()
            .map(|m| m.marker.message.to_string())
            .collect::<Vec<_>>()
        })
    };
    // Monaco's `DoubleResourceMap`: by owner, in the order first set.
    assert_eq!(messages(None, cx), vec!["b2", "a1", "a2", "a0"]);
    assert_eq!(messages(Some(2), cx), vec!["b2", "a1"]);
    // Monaco: a `take` of 0 means all.
    assert_eq!(messages(Some(0), cx).len(), 4);
    // Markers as set, even after edits move their decorations.
    a.update(cx, |m, cx| m.set_value("XX some text\nmore", cx));
    assert_eq!(
        cx.update(|_, cx| gpui_monaco::model_markers(cx, &MarkerFilter::default()))[0]
            .marker
            .range,
        PointUtf16::new(0, 0)..PointUtf16::new(0, 1)
    );
}

/// Monaco gives a model without a uri `inmemory://model/N`, so its
/// markers are reported and found like any other's.
#[gpui::test]
fn markers_of_a_model_without_a_uri(cx: &mut TestAppContext) {
    let (_, cx) = setup("", cx);
    let model = cx.update(|_, cx| gpui_monaco::create_model(cx, "text", None, None).unwrap());
    let resource = model.read_with(cx, |m, _| m.resource().clone());
    assert!(resource.starts_with("inmemory://model/"));
    assert_eq!(model.read_with(cx, |m, _| m.uri().cloned()), None);
    let other = cx.update(|_, cx| gpui_monaco::create_model(cx, "text", None, None).unwrap());
    assert_ne!(other.read_with(cx, |m, _| m.resource().clone()), resource);
    flush(cx);

    let events: Log<Vec<SharedString>> = Rc::default();
    let _subscription = cx.update(|_, cx| {
        let events = events.clone();
        gpui_monaco::on_did_change_markers(cx, move |uris, _| {
            events.borrow_mut().push(uris.to_vec())
        })
    });
    set(&model, "far", vec![marker("a")], cx);
    flush(cx);
    assert_eq!(*events.borrow(), vec![vec![resource.clone()]]);
    assert_eq!(read(None, Some(&resource), cx), 1);
    let markers = cx.update(|_, cx| gpui_monaco::model_markers(cx, &MarkerFilter::default()));
    assert_eq!(markers[0].resource, resource);
    // `getModel` finds it by that uri.
    let found = cx.update(|_, cx| gpui_monaco::model_for_uri(cx, &resource));
    assert_eq!(found.map(|m| m.entity_id()), Some(model.entity_id()));
}

/// Monaco `MarkerDecorationsService._onModelRemoved`: an in-memory model's
/// markers go with it.
#[gpui::test]
fn markers_of_an_inmemory_model_go_with_it(cx: &mut TestAppContext) {
    let (_, cx) = setup("", cx);
    let model = cx.update(|_, cx| gpui_monaco::create_model(cx, "text", None, None).unwrap());
    let resource = model.read_with(cx, |m, _| m.resource().clone());
    set(&model, "far", vec![marker("a")], cx);
    flush(cx);
    let events: Log<Vec<SharedString>> = Rc::default();
    let _subscription = cx.update(|_, cx| {
        let events = events.clone();
        gpui_monaco::on_did_change_markers(cx, move |uris, _| {
            events.borrow_mut().push(uris.to_vec())
        })
    });
    drop(model);
    flush(cx);
    assert_eq!(read(None, None, cx), 0);
    assert_eq!(*events.borrow(), vec![vec![resource]]);
}

/// Monaco's markers belong to the uri: they outlive a `file:` model and
/// decorate the next model with its uri.
#[gpui::test]
fn markers_outlive_their_model(cx: &mut TestAppContext) {
    let (_, cx) = setup("", cx);
    let uri = "file:///c/test/file.cs";
    let create = |cx: &mut VisualTestContext| {
        cx.update(|_, cx| gpui_monaco::create_model(cx, "some text", None, Some(uri)).unwrap())
    };
    let model = create(cx);
    set(&model, "far", vec![marker("a")], cx);
    flush(cx);
    let events: Log<Vec<SharedString>> = Rc::default();
    let _subscription = cx.update(|_, cx| {
        let events = events.clone();
        gpui_monaco::on_did_change_markers(cx, move |uris, _| {
            events.borrow_mut().push(uris.to_vec())
        })
    });
    drop(model);
    flush(cx);
    assert!(
        cx.update(|_, cx| gpui_monaco::model_for_uri(cx, uri))
            .is_none()
    );
    assert_eq!(read(Some("far"), Some(uri), cx), 1);
    assert!(events.borrow().is_empty());

    let again = create(cx);
    let live = again.read_with(cx, |m, _| m.markers());
    assert_eq!(live.len(), 1);
    assert_eq!(live[0].message.as_ref(), "a");

    // Removing them without a model is a change of the uri.
    drop(again);
    flush(cx);
    cx.update(|_, cx| gpui_monaco::remove_all_markers(cx, "far"));
    flush(cx);
    assert_eq!(read(None, None, cx), 0);
    assert_eq!(*events.borrow(), vec![vec![SharedString::from(uri)]]);
}

// ----- themes -----------------------------------------------------------------

#[gpui::test]
fn set_theme_applies_to_every_editor(cx: &mut TestAppContext) {
    let (editor, cx) = setup("one", cx);
    let theme_name = |editor: &Entity<CodeEditor>, cx: &mut VisualTestContext| {
        editor.read_with(cx, |e, _| e.theme().name.clone())
    };
    // Monaco's default theme.
    assert_eq!(theme_name(&editor, cx), "vs");
    cx.update(|_, cx| gpui_monaco::set_theme(cx, "hc-black"));
    assert_eq!(theme_name(&editor, cx), "hc-black");

    // Editors created later start with it.
    let later = cx.update(|window, cx| cx.new(|cx| CodeEditor::with_text("two", window, cx)));
    assert_eq!(theme_name(&later, cx), "hc-black");

    // Defined themes, and redefining the current one refreshes editors.
    let mine = Theme::define(
        "my-theme",
        &Theme::vs(),
        true,
        Vec::new(),
        &Default::default(),
    );
    cx.update(|_, cx| gpui_monaco::define_theme(cx, mine).unwrap());
    cx.update(|_, cx| gpui_monaco::set_theme(cx, "my-theme"));
    assert_eq!(theme_name(&editor, cx), "my-theme");
    let colors = [("editor.background".to_string(), "#123456".to_string())]
        .into_iter()
        .collect();
    let redefined = Theme::define("my-theme", &Theme::vs(), true, Vec::new(), &colors);
    let background = redefined.colors.background;
    cx.update(|_, cx| gpui_monaco::define_theme(cx, redefined).unwrap());
    assert_eq!(
        editor.read_with(cx, |e, _| e.colors().background),
        background
    );
    assert_eq!(
        later.read_with(cx, |e, _| e.colors().background),
        background
    );

    // Unknown names get `vs`; bad names are refused.
    cx.update(|_, cx| gpui_monaco::set_theme(cx, "no-such-theme"));
    assert_eq!(theme_name(&editor, cx), "vs");
    let bad = Theme::define("bad name!", &Theme::vs(), true, Vec::new(), &colors);
    assert!(
        cx.update(|_, cx| gpui_monaco::define_theme(cx, bad))
            .is_err()
    );
    assert!(cx.update(|_, cx| gpui_monaco::theme_named(cx, "hc-light").is_some()));
    assert_eq!(
        cx.update(|_, cx| gpui_monaco::global_theme(cx).map(|t| t.name.clone())),
        Some("vs".to_string())
    );
}

// ----- commands, actions and keybindings ----------------------------------------

fn run(id: &str, cx: &mut gpui::App) -> anyhow::Result<()> {
    gpui_monaco::execute_command(
        cx,
        &gpui_monaco::providers::Command {
            id: id.to_string().into(),
            title: SharedString::default(),
            arguments: vec![serde_json::json!(1)],
            ..Default::default()
        },
    )
}

#[gpui::test]
fn added_commands_stack_and_dispose(cx: &mut TestAppContext) {
    let calls: Rc<RefCell<Vec<&'static str>>> = Rc::default();
    cx.update(|cx| {
        let first = {
            let calls = calls.clone();
            gpui_monaco::add_command(cx, "my.command", move |args, _| {
                assert_eq!(args, &[serde_json::json!(1)]);
                calls.borrow_mut().push("first");
            })
        };
        let second = {
            let calls = calls.clone();
            gpui_monaco::add_command(cx, "my.command", move |_, _| {
                calls.borrow_mut().push("second")
            })
        };
        run("my.command", cx).unwrap();
        // Disposing the newest brings back the older one.
        second.dispose(cx);
        run("my.command", cx).unwrap();
        first.dispose(cx);
        assert!(run("my.command", cx).is_err());
    });
    assert_eq!(*calls.borrow(), vec!["second", "first"]);
}

#[gpui::test]
fn editor_actions_reach_every_editor(cx: &mut TestAppContext) {
    let (editor, cx) = setup("abc", cx);
    let action = EditorActionDescriptor::new("my.upper", "Make Upper", |editor, _, cx| {
        let text = editor.model().read(cx).text().to_uppercase();
        editor.model().update(cx, |m, cx| m.set_value(&text, cx));
    })
    .keybinding("ctrl-alt-u")
    .context_menu("1_modification", 0.5);
    let disposable = cx.update(|_, cx| gpui_monaco::add_editor_action(cx, action));

    // Its keybinding, in the existing editor.
    cx.simulate_keystrokes("ctrl-alt-u");
    cx.run_until_parked();
    assert_eq!(
        editor.read_with(cx, |e, cx| e.model().read(cx).text()),
        "ABC"
    );
    // In the context menu, but not the command palette (it's a command in
    // Monaco).
    cx.simulate_keystrokes("shift-f10");
    cx.run_until_parked();
    let rows = editor.read_with(cx, |e, cx| e.context_menu_rows_for_tests(cx));
    assert!(rows.iter().any(|r| r == "Make Upper"), "{rows:?}");
    cx.simulate_keystrokes("escape");
    assert!(
        !editor
            .read_with(cx, |e, cx| e.supported_actions(cx))
            .iter()
            .any(|a| a.id.as_ref() == "my.upper")
    );

    // And in editors created later.
    let later = cx.update(|window, cx| cx.new(|cx| CodeEditor::with_text("xyz", window, cx)));
    let ran =
        cx.update(|window, cx| later.update(cx, |e, cx| e.run_action("my.upper", window, cx)));
    assert!(ran);
    assert_eq!(
        later.read_with(cx, |e, cx| e.model().read(cx).text()),
        "XYZ"
    );

    // Disposed: gone, with its keybinding.
    let bindings = cx.update(|_, cx| cx.key_bindings().borrow().bindings().len());
    cx.update(|_, cx| disposable.dispose(cx));
    assert_eq!(
        cx.update(|_, cx| cx.key_bindings().borrow().bindings().len()),
        bindings - 1
    );
    let ran =
        cx.update(|window, cx| later.update(cx, |e, cx| e.run_action("my.upper", window, cx)));
    assert!(!ran);
}

#[gpui::test]
fn keybinding_rules_run_commands_until_disposed(cx: &mut TestAppContext) {
    let (editor, cx) = setup("abc", cx);
    let args: Rc<RefCell<Vec<Vec<serde_json::Value>>>> = Rc::default();
    let _command = cx.update(|_, cx| {
        let args = args.clone();
        gpui_monaco::add_command(cx, "my.command", move |a, _| {
            args.borrow_mut().push(a.to_vec())
        })
    });
    let rule = cx.update(|_, cx| {
        gpui_monaco::add_keybinding_rule(
            cx,
            KeybindingRule::new("ctrl-alt-k", "my.command").args(serde_json::json!({"x": 1})),
        )
        .unwrap()
    });
    cx.simulate_keystrokes("ctrl-alt-k");
    cx.run_until_parked();
    assert_eq!(*args.borrow(), vec![vec![serde_json::json!({"x": 1})]]);

    // A rule for an editor command, shadowed by an unbinding rule.
    let cursor = |cx: &mut VisualTestContext| editor.read_with(cx, |e, _| e.selections()[0].head);
    let right = cx.update(|_, cx| {
        gpui_monaco::add_keybinding_rule(cx, KeybindingRule::new("ctrl-alt-r", "cursorRight"))
            .unwrap()
    });
    cx.simulate_keystrokes("ctrl-alt-r");
    cx.run_until_parked();
    assert_eq!(cursor(cx), PointUtf16::new(0, 1));
    let unbind = cx.update(|_, cx| {
        gpui_monaco::add_keybinding_rules(cx, vec![KeybindingRule::unbind("ctrl-alt-r")]).unwrap()
    });
    cx.simulate_keystrokes("ctrl-alt-r");
    cx.run_until_parked();
    assert_eq!(cursor(cx), PointUtf16::new(0, 1));
    // Disposing the unbinding restores what it shadowed.
    cx.update(|_, cx| unbind.dispose(cx));
    cx.simulate_keystrokes("ctrl-alt-r");
    cx.run_until_parked();
    assert_eq!(cursor(cx), PointUtf16::new(0, 2));

    cx.update(|_, cx| {
        rule.dispose(cx);
        right.dispose(cx);
    });
    cx.simulate_keystrokes("ctrl-alt-k ctrl-alt-r");
    cx.run_until_parked();
    assert_eq!(args.borrow().len(), 1);
    assert_eq!(cursor(cx), PointUtf16::new(0, 2));

    // Bad `when`s are refused.
    assert!(
        cx.update(|_, cx| gpui_monaco::add_keybinding_rule(
            cx,
            KeybindingRule::new("ctrl-alt-q", "my.command").when("((")
        ))
        .is_err()
    );
}

// ----- colorizeModelLine ----------------------------------------------------------

#[gpui::test]
fn colorize_model_line(cx: &mut TestAppContext) {
    let registry = gpui_monaco_languages::registry();
    cx.update(|cx| gpui_monaco::set_language_registry(cx, registry.clone()));
    let theme = Theme::vs_dark();
    let text = "fn main() {\n\tlet x = 1;\n}";
    let model = cx.new(|_| TextModel::new(text).with_language("rust"));
    let line = |model: &Entity<TextModel>, row: u32, cx: &mut TestAppContext| {
        cx.update(|cx| {
            gpui_monaco::colorize::colorize_model_line(model.read(cx), row, 2, &theme, cx)
        })
    };
    // What `colorize` gives for the same text.
    let expected = |text: &str, row: usize| {
        let options = gpui_monaco::colorize::ColorizeOptions { tab_size: 2 };
        gpui_monaco::colorize::colorize(text, "rust", &theme, &registry, options)[row].clone()
    };

    // Never shown, so never parsed: it's parsed by its language.
    let colorized = line(&model, 1, cx).unwrap();
    assert_eq!(colorized.text.as_ref(), "  let x = 1;");
    assert_eq!(colorized.highlights[0].0, 2..5);
    assert_eq!(colorized, expected(text, 1));
    assert!(line(&model, 3, cx).is_none());

    // Parsed, then edited: brought up to date without waiting.
    model.update(cx, |m, cx| m.ensure_syntax(cx));
    cx.run_until_parked();
    let edited = "fn main() {\n\tconst X: u8 = 1;\n}";
    model.update(cx, |m, cx| m.set_value(edited, cx));
    assert!(model.read_with(cx, |m, _| m.is_parsing()));
    let colorized = line(&model, 1, cx).unwrap();
    assert_eq!(colorized.text.as_ref(), "  const X: u8 = 1;");
    assert_eq!(colorized, expected(edited, 1));

    // A model without a language has no highlights.
    let plain = cx.new(|_| TextModel::new("\tlet"));
    let colorized = line(&plain, 0, cx).unwrap();
    assert_eq!(colorized.text.as_ref(), "  let");
    assert!(colorized.highlights.is_empty());
}
