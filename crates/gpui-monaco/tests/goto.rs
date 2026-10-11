//! Go to definition, declaration, type definition, implementations and
//! references, and the peek view (Monaco `contrib/gotoSymbol`).

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

use gpui::{App, AppContext as _, Entity, Focusable, Task, TestAppContext, VisualTestContext};
use gpui_monaco::cursor::Selection;
use gpui_monaco::options::{GoToLocation, LocationKind};
use gpui_monaco::providers::*;
use gpui_monaco::text::PointUtf16;
use gpui_monaco::{CodeEditor, CodeEditorEvent, TextModel};

fn p(row: u32, column: u32) -> PointUtf16 {
    PointUtf16::new(row, column)
}

fn here(row: u32, start: u32, end: u32) -> Location {
    Location {
        uri: None,
        range: p(row, start)..p(row, end),
    }
}

fn elsewhere(uri: &str, row: u32) -> Location {
    Location {
        uri: Some(uri.to_string().into()),
        range: p(row, 0)..p(row, 3),
    }
}

/// Answers every kind of location request from fixed lists.
#[derive(Clone, Default)]
struct Fixed {
    definitions: Vec<Location>,
    references: Vec<Location>,
    declarations: Vec<Location>,
    type_definitions: Vec<Location>,
    implementations: Vec<Location>,
    /// Requests made, by kind.
    log: Rc<RefCell<Vec<&'static str>>>,
}

impl Fixed {
    fn answer(
        &self,
        kind: &'static str,
        locations: &[Location],
    ) -> Task<anyhow::Result<Vec<Location>>> {
        self.log.borrow_mut().push(kind);
        Task::ready(Ok(locations.to_vec()))
    }
}

impl DefinitionProvider for Fixed {
    fn provide_definition(
        &self,
        _: &ProviderContext,
        _: PointUtf16,
        _: &mut App,
    ) -> Task<anyhow::Result<Vec<Location>>> {
        self.answer("definition", &self.definitions)
    }
}
impl ReferenceProvider for Fixed {
    fn provide_references(
        &self,
        _: &ProviderContext,
        _: PointUtf16,
        context: ReferenceContext,
        _: &mut App,
    ) -> Task<anyhow::Result<Vec<Location>>> {
        assert!(context.include_declaration);
        self.answer("references", &self.references)
    }
}
impl DeclarationProvider for Fixed {
    fn provide_declaration(
        &self,
        _: &ProviderContext,
        _: PointUtf16,
        _: &mut App,
    ) -> Task<anyhow::Result<Vec<Location>>> {
        self.answer("declaration", &self.declarations)
    }
}
impl TypeDefinitionProvider for Fixed {
    fn provide_type_definition(
        &self,
        _: &ProviderContext,
        _: PointUtf16,
        _: &mut App,
    ) -> Task<anyhow::Result<Vec<Location>>> {
        self.answer("type definition", &self.type_definitions)
    }
}
impl ImplementationProvider for Fixed {
    fn provide_implementation(
        &self,
        _: &ProviderContext,
        _: PointUtf16,
        _: &mut App,
    ) -> Task<anyhow::Result<Vec<Location>>> {
        self.answer("implementation", &self.implementations)
    }
}

const TEXT: &str = "fn add(a: i32) {}\nfn main() {\n    add(1);\n    add(2);\n}\n";

fn setup(fixed: Fixed, cx: &mut TestAppContext) -> (Entity<CodeEditor>, &mut VisualTestContext) {
    cx.update(|cx| {
        gpui_monaco::init(cx);
        let all = LanguageSelector::All;
        register_definition_provider(cx, all.clone(), fixed.clone());
        register_reference_provider(cx, all.clone(), fixed.clone());
        register_declaration_provider(cx, all.clone(), fixed.clone());
        register_type_definition_provider(cx, all.clone(), fixed.clone());
        register_implementation_provider(cx, all, fixed);
    });
    let (editor, cx) = cx.add_window_view(|window, cx| {
        let model = cx.new(|_| TextModel::new(TEXT).with_uri("file:///src/main.rs"));
        CodeEditor::new(model, window, cx)
    });
    cx.update(|window, cx| window.focus(&editor.focus_handle(cx), cx));
    draw(cx);
    (editor, cx)
}

fn draw(cx: &mut VisualTestContext) {
    cx.run_until_parked();
    cx.update(|window, cx| {
        window.refresh();
        window.draw(cx).clear(cx);
    });
    cx.run_until_parked();
}

fn head(editor: &Entity<CodeEditor>, cx: &mut VisualTestContext) -> PointUtf16 {
    editor.read_with(cx, |e, _| e.selections()[0].head)
}

fn set_cursor(editor: &Entity<CodeEditor>, cx: &mut VisualTestContext, at: PointUtf16) {
    editor.update(cx, |e, cx| {
        e.set_selections(&[Selection::collapsed(at)], cx)
    });
}

fn set_goto(
    editor: &Entity<CodeEditor>,
    cx: &mut VisualTestContext,
    f: impl FnOnce(&mut gpui_monaco::options::GotoLocationOptions),
) {
    editor.update(cx, |e, cx| {
        let mut options = e.options().clone();
        f(&mut options.goto_location);
        e.update_options(options, cx);
    });
}

#[gpui::test]
fn a_single_result_is_selected_and_highlighted(cx: &mut TestAppContext) {
    let fixed = Fixed {
        definitions: vec![here(0, 3, 6)],
        ..Default::default()
    };
    let (editor, cx) = setup(fixed, cx);
    set_cursor(&editor, cx, p(2, 5));
    cx.simulate_keystrokes("f12");
    cx.run_until_parked();
    assert_eq!(head(&editor, cx), p(0, 3));
    let id = editor.entity_id();
    let model = editor.read_with(cx, |e, _| e.model().clone());
    let color = editor.read_with(cx, |e, _| e.colors().symbol_highlight_background);
    let highlighted = |cx: &mut VisualTestContext| {
        model.update(cx, |m, _| {
            m.all_decorations(Some(id))
                .iter()
                .filter(|d| d.options.background == Some(color))
                .count()
        })
    };
    assert_eq!(highlighted(cx), 1);
    cx.executor().advance_clock(Duration::from_millis(400));
    cx.run_until_parked();
    assert_eq!(highlighted(cx), 0);
}

#[gpui::test]
fn no_results_show_a_message(cx: &mut TestAppContext) {
    let (editor, cx) = setup(Fixed::default(), cx);
    set_cursor(&editor, cx, p(2, 5));
    cx.simulate_keystrokes("shift-f12");
    cx.run_until_parked();
    assert!(editor.read_with(cx, |e, _| e.is_message_visible()));
    // Moving the cursor closes it.
    cx.simulate_keystrokes("right");
    assert!(!editor.read_with(cx, |e, _| e.is_message_visible()));
}

#[gpui::test]
fn the_only_result_at_the_cursor_runs_the_alternative(cx: &mut TestAppContext) {
    let fixed = Fixed {
        definitions: vec![here(0, 3, 6)],
        references: vec![here(0, 3, 6), here(2, 4, 7), here(3, 4, 7)],
        ..Default::default()
    };
    let log = fixed.log.clone();
    let (editor, cx) = setup(fixed, cx);
    // On the definition itself: go to references instead, in a peek view.
    set_cursor(&editor, cx, p(0, 4));
    cx.simulate_keystrokes("f12");
    draw(cx);
    assert_eq!(*log.borrow(), ["definition", "references"]);
    assert!(editor.read_with(cx, |e, _| e.is_peek_visible()));
    // No alternative for implementations.
    log.borrow_mut().clear();
    cx.simulate_keystrokes("escape");
    editor.update(cx, |e, cx| e.go_to(LocationKind::Implementation, cx));
    cx.run_until_parked();
    assert_eq!(*log.borrow(), ["implementation"]);
}

#[gpui::test]
fn several_results_open_a_peek_view(cx: &mut TestAppContext) {
    let fixed = Fixed {
        references: vec![
            elsewhere("file:///src/lib.rs", 4),
            here(3, 4, 7),
            here(0, 3, 6),
            here(2, 4, 7),
        ],
        ..Default::default()
    };
    let (editor, cx) = setup(fixed, cx);
    set_cursor(&editor, cx, p(3, 5));
    cx.simulate_keystrokes("shift-f12");
    draw(cx);
    let (locations, selected) = editor.read_with(cx, |e, cx| e.peek_results(cx)).unwrap();
    // Sorted by uri and position, nearest to the cursor selected.
    let rows: Vec<_> = locations
        .iter()
        .map(|l| (l.uri.clone(), l.range.start.row))
        .collect();
    assert_eq!(
        rows,
        vec![
            (Some("file:///src/lib.rs".into()), 4),
            (Some("file:///src/main.rs".into()), 0),
            (Some("file:///src/main.rs".into()), 2),
            (Some("file:///src/main.rs".into()), 3),
        ]
    );
    assert_eq!(selected, 3);
    // The preview shows the selected result's model.
    let preview = editor.read_with(cx, |e, cx| e.peek_preview(cx)).unwrap();
    let main_model = editor.read_with(cx, |e, _| e.model().clone());
    assert_eq!(preview.read_with(cx, |p, _| p.model().clone()), main_model);
    assert_eq!(
        preview.read_with(cx, |p, _| p.selections()[0].head),
        p(3, 4)
    );

    // The list has focus: arrows move through results, F4 and Shift+F4 too.
    cx.simulate_keystrokes("up");
    draw(cx);
    assert_eq!(
        editor.read_with(cx, |e, cx| e.peek_results(cx)).unwrap().1,
        2
    );
    cx.simulate_keystrokes("f4");
    cx.simulate_keystrokes("f4");
    draw(cx);
    // Wraps around to the first.
    assert_eq!(
        editor.read_with(cx, |e, cx| e.peek_results(cx)).unwrap().1,
        0
    );
    // A file that can't be opened has no preview.
    let preview_text = preview.read_with(cx, |p, cx| p.model().read(cx).text());
    assert_eq!(preview_text, "no preview available");
    cx.simulate_keystrokes("shift-f4");
    draw(cx);
    assert_eq!(
        editor.read_with(cx, |e, cx| e.peek_results(cx)).unwrap().1,
        3
    );

    // Enter on a result in this model moves the peek view there.
    cx.simulate_keystrokes("up enter");
    draw(cx);
    assert_eq!(head(&editor, cx), p(2, 4));
    assert!(editor.read_with(cx, |e, _| e.is_peek_visible()));

    // Escape closes it and the editor gets focus back.
    cx.simulate_keystrokes("escape");
    draw(cx);
    assert!(!editor.read_with(cx, |e, _| e.is_peek_visible()));
    assert!(cx.update(|window, cx| editor.focus_handle(cx).is_focused(window)));
}

#[gpui::test]
fn opening_a_result_elsewhere_asks_the_host(cx: &mut TestAppContext) {
    let fixed = Fixed {
        references: vec![elsewhere("file:///src/lib.rs", 4), here(2, 4, 7)],
        ..Default::default()
    };
    let (editor, cx) = setup(fixed, cx);
    // Other documents come from the resolver.
    cx.update(|_, cx| {
        gpui_monaco::set_model_resolver(cx, |uri, cx| {
            (uri == "file:///src/lib.rs")
                .then(|| cx.new(|_| TextModel::new("\n\n\n\npub fn add() {}").with_uri(uri)))
        })
    });
    let opened = Rc::new(RefCell::new(None));
    let sink = opened.clone();
    cx.update(|_, cx| {
        cx.subscribe(&editor, move |_, event: &CodeEditorEvent, _| {
            if let CodeEditorEvent::OpenLocation(location) = event {
                *sink.borrow_mut() = Some(location.clone());
            }
        })
        .detach();
    });
    set_cursor(&editor, cx, p(2, 5));
    cx.simulate_keystrokes("shift-f12");
    draw(cx);
    cx.simulate_keystrokes("home");
    draw(cx);
    let preview = editor.read_with(cx, |e, cx| e.peek_preview(cx)).unwrap();
    let text = preview.read_with(cx, |p, cx| p.model().read(cx).text());
    assert_eq!(text, "\n\n\n\npub fn add() {}");
    cx.simulate_keystrokes("enter");
    draw(cx);
    assert!(!editor.read_with(cx, |e, _| e.is_peek_visible()));
    let location = opened.borrow().clone().unwrap();
    assert_eq!(location.uri.as_deref(), Some("file:///src/lib.rs"));
    assert_eq!(location.range.start, p(4, 0));
}

#[gpui::test]
fn goto_mode_steps_through_results_with_f4(cx: &mut TestAppContext) {
    let fixed = Fixed {
        implementations: vec![here(2, 4, 7), here(3, 4, 7), here(0, 3, 6)],
        ..Default::default()
    };
    let (editor, cx) = setup(fixed, cx);
    set_goto(&editor, cx, |o| {
        o.multiple_implementations = GoToLocation::Goto
    });
    set_cursor(&editor, cx, p(1, 3));
    let keys = if cfg!(target_os = "macos") {
        "cmd-f12"
    } else {
        "ctrl-f12"
    };
    cx.simulate_keystrokes(keys);
    cx.run_until_parked();
    assert!(!editor.read_with(cx, |e, _| e.is_peek_visible()));
    assert_eq!(head(&editor, cx), p(0, 3));
    assert!(cx.update(|_, cx| gpui_monaco::has_symbol_navigation(cx)));
    cx.simulate_keystrokes("f4");
    assert_eq!(head(&editor, cx), p(2, 4));
    cx.simulate_keystrokes("f4 f4");
    assert_eq!(head(&editor, cx), p(0, 3));
    // Leaving the results ends it.
    set_cursor(&editor, cx, p(1, 0));
    assert!(!cx.update(|_, cx| gpui_monaco::has_symbol_navigation(cx)));
}

#[gpui::test]
fn goto_and_peek_does_both(cx: &mut TestAppContext) {
    let fixed = Fixed {
        type_definitions: vec![here(2, 4, 7), here(0, 3, 6)],
        declarations: vec![here(0, 3, 6)],
        ..Default::default()
    };
    let (editor, cx) = setup(fixed, cx);
    set_goto(&editor, cx, |o| {
        o.multiple_type_definitions = GoToLocation::GotoAndPeek
    });
    set_cursor(&editor, cx, p(1, 3));
    editor.update(cx, |e, cx| e.go_to(LocationKind::TypeDefinition, cx));
    draw(cx);
    assert_eq!(head(&editor, cx), p(0, 3));
    assert!(editor.read_with(cx, |e, _| e.is_peek_visible()));
    // Peek commands show even a single result in a peek view.
    cx.simulate_keystrokes("escape");
    set_cursor(&editor, cx, p(3, 5));
    editor.update(cx, |e, cx| e.peek(LocationKind::Declaration, cx));
    draw(cx);
    assert_eq!(
        editor
            .read_with(cx, |e, cx| e.peek_results(cx))
            .unwrap()
            .0
            .len(),
        1
    );
}

#[gpui::test]
fn results_after_the_cursor_moved_are_dropped(cx: &mut TestAppContext) {
    let fixed = Fixed {
        definitions: vec![here(0, 3, 6)],
        ..Default::default()
    };
    let (editor, cx) = setup(fixed, cx);
    set_cursor(&editor, cx, p(2, 5));
    editor.update(cx, |e, cx| {
        e.go_to(LocationKind::Definition, cx);
        e.set_selections(&[Selection::collapsed(p(3, 1))], cx);
    });
    cx.run_until_parked();
    assert_eq!(head(&editor, cx), p(3, 1));
}

fn events(
    editor: &Entity<CodeEditor>,
    cx: &mut VisualTestContext,
) -> Rc<RefCell<Vec<CodeEditorEvent>>> {
    let events = Rc::new(RefCell::new(Vec::new()));
    let sink = events.clone();
    cx.update(|_, cx| {
        cx.subscribe(editor, move |_, event: &CodeEditorEvent, _| {
            sink.borrow_mut().push(event.clone())
        })
        .detach();
    });
    events
}

fn to_side(events: &Rc<RefCell<Vec<CodeEditorEvent>>>) -> Vec<Location> {
    events
        .borrow()
        .iter()
        .filter_map(|event| match event {
            CodeEditorEvent::OpenLocationToSide(location) => Some(location.clone()),
            _ => None,
        })
        .collect()
}

#[gpui::test]
fn reveal_definition_aside_asks_the_host(cx: &mut TestAppContext) {
    let fixed = Fixed {
        definitions: vec![here(0, 3, 6)],
        ..Default::default()
    };
    let (editor, cx) = setup(fixed, cx);
    let events = events(&editor, cx);
    set_cursor(&editor, cx, p(2, 5));
    // Monaco's Cmd/Ctrl+K F12: even a result in this model goes to the host,
    // and the cursor stays.
    cx.simulate_keystrokes("secondary-k f12");
    cx.run_until_parked();
    let sent = to_side(&events);
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].range, p(0, 3)..p(0, 6));
    assert_eq!(head(&editor, cx), p(2, 5));
    // Monaco's alias for the command.
    cx.update(|window, cx| {
        editor.update(cx, |e, cx| {
            e.trigger("editor.action.openDeclarationToTheSide", window, cx)
        })
    })
    .unwrap();
    cx.run_until_parked();
    assert_eq!(to_side(&events).len(), 2);
}

fn mouse_at(
    editor: &Entity<CodeEditor>,
    cx: &mut VisualTestContext,
    at: PointUtf16,
) -> gpui::Point<gpui::Pixels> {
    let origin = editor
        .read_with(cx, |e, _| e.scrolled_visible_position(at))
        .expect("laid out");
    gpui::point(origin.x + gpui::px(1.), origin.y + gpui::px(2.))
}

/// Cmd/Ctrl+hover, then Cmd/Ctrl+click (with `modifiers` added) at `at`.
fn definition_click(
    editor: &Entity<CodeEditor>,
    cx: &mut VisualTestContext,
    at: PointUtf16,
    modifiers: gpui::Modifiers,
) {
    let position = mouse_at(editor, cx, at);
    cx.simulate_mouse_move(position, None, modifiers);
    cx.executor().advance_clock(Duration::from_millis(60));
    cx.run_until_parked();
    cx.simulate_click(position, modifiers);
    draw(cx);
}

#[gpui::test]
fn definition_links_open_in_peek_or_to_the_side(cx: &mut TestAppContext) {
    let fixed = Fixed {
        definitions: vec![here(0, 3, 6)],
        ..Default::default()
    };
    let (editor, cx) = setup(fixed, cx);
    let events = events(&editor, cx);
    editor.update(cx, |e, cx| {
        let mut options = e.options().clone();
        options.definition_link_opens_in_peek = true;
        e.update_options(options, cx);
    });
    draw(cx);
    // Monaco `definitionLinkOpensInPeek`: even one result shows in a peek.
    definition_click(&editor, cx, p(2, 5), gpui::Modifiers::secondary_key());
    assert!(editor.read_with(cx, |e, _| e.is_peek_visible()));
    assert_eq!(head(&editor, cx), p(2, 5));
    cx.simulate_keystrokes("escape");
    draw(cx);

    // With the side-by-side modifier (Alt, since Alt adds cursors), the
    // host is asked to open it beside this editor.
    let side = gpui::Modifiers {
        alt: true,
        ..gpui::Modifiers::secondary_key()
    };
    definition_click(&editor, cx, p(3, 5), side);
    assert!(!editor.read_with(cx, |e, _| e.is_peek_visible()));
    assert_eq!(to_side(&events).len(), 1);
}

#[gpui::test]
fn peek_focus_option_and_toggle(cx: &mut TestAppContext) {
    let fixed = Fixed {
        references: vec![here(0, 3, 6), here(2, 4, 7), here(3, 4, 7)],
        ..Default::default()
    };
    let (editor, cx) = setup(fixed, cx);
    let preview_focused = |cx: &mut VisualTestContext| {
        cx.update(|window, cx| editor.read(cx).is_peek_preview_focused(window, cx))
    };
    set_cursor(&editor, cx, p(2, 5));
    cx.simulate_keystrokes("shift-f12");
    draw(cx);
    // The list has focus by default (`peekWidgetDefaultFocus: tree`).
    assert!(!preview_focused(cx));
    // Monaco `togglePeekWidgetFocus` (Cmd/Ctrl+K F2) goes back and forth.
    cx.simulate_keystrokes("secondary-k f2");
    draw(cx);
    assert!(preview_focused(cx));
    cx.simulate_keystrokes("secondary-k f2");
    draw(cx);
    assert!(!preview_focused(cx));
    // F12 in the list is Monaco's other key for the next reference.
    let selected =
        |cx: &mut VisualTestContext| editor.read_with(cx, |e, cx| e.peek_results(cx)).unwrap().1;
    assert_eq!(selected(cx), 1);
    cx.simulate_keystrokes("f12");
    draw(cx);
    assert_eq!(selected(cx), 2);
    cx.simulate_keystrokes("escape");
    draw(cx);

    // `peekWidgetDefaultFocus: editor` focuses the preview.
    editor.update(cx, |e, cx| {
        let mut options = e.options().clone();
        options.peek_widget_default_focus = gpui_monaco::options::PeekWidgetDefaultFocus::Editor;
        e.update_options(options, cx);
    });
    cx.simulate_keystrokes("shift-f12");
    draw(cx);
    assert!(preview_focused(cx));
}
