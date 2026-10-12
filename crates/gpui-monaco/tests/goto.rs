//! Go to definition, declaration, type definition, implementations and
//! references, and the peek view (Monaco `contrib/gotoSymbol`).

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

use gpui::{App, AppContext as _, Entity, Focusable, Task, TestAppContext, VisualTestContext};
use gpui_monaco::cursor::Selection;
use gpui_monaco::options::{GoToLocation, LocationKind, NavigationCommand};
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
        ..Default::default()
    }
}

fn elsewhere(uri: &str, row: u32) -> Location {
    Location {
        uri: Some(uri.to_string().into()),
        range: p(row, 0)..p(row, 3),
        ..Default::default()
    }
}

/// Answers every kind of location request from fixed lists.
#[derive(Clone, Default)]
struct Fixed {
    definitions: Vec<Location>,
    references: Vec<Location>,
    /// The references without the declaration (`references` when `None`).
    references_without_declaration: Option<Vec<Location>>,
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
        if context.include_declaration {
            self.answer("references", &self.references)
        } else {
            let without = self.references_without_declaration.as_ref();
            self.answer(
                "references without declaration",
                without.unwrap_or(&self.references),
            )
        }
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
    setup_with(TEXT, fixed, cx)
}

fn setup_with<'a>(
    text: &str,
    fixed: Fixed,
    cx: &'a mut TestAppContext,
) -> (Entity<CodeEditor>, &'a mut VisualTestContext) {
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
        let model = cx.new(|_| TextModel::new(text).with_uri("file:///src/main.rs"));
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

fn announcement(editor: &Entity<CodeEditor>, cx: &mut VisualTestContext) -> Option<String> {
    editor.read_with(cx, |e, _| {
        e.last_announcement_for_tests().map(|m| m.to_string())
    })
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
    // Monaco alerts the results' `ariaMessage`.
    assert_eq!(
        announcement(&editor, cx).as_deref(),
        Some("Found 1 symbol in /src/main.rs")
    );
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
    // "No results found", then the message, as Monaco alerts both.
    assert_eq!(
        announcement(&editor, cx).as_deref(),
        Some("No references found for 'add'")
    );
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
    // Home focuses the first row, lib.rs's file row (Monaco
    // `list.focusFirst`); Down its result.
    cx.simulate_keystrokes("home down");
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
fn goto_mode_steps_through_results(cx: &mut TestAppContext) {
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
    // F4 is only for peek views (Monaco `goToNextReference`); with a
    // definition provider, F12 is Go to Definition, so step with the
    // command (Monaco `editor.gotoNextSymbolFromResult`).
    cx.simulate_keystrokes("f4");
    assert_eq!(head(&editor, cx), p(0, 3));
    let next = |cx: &mut VisualTestContext| {
        cx.dispatch_action(gpui_monaco::actions::GoToNextSymbolFromResult);
        cx.run_until_parked();
    };
    // Only the first result is highlighted: Monaco
    // `SymbolNavigationService.revealNext` doesn't highlight the others.
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
    next(cx);
    assert_eq!(head(&editor, cx), p(2, 4));
    assert_eq!(highlighted(cx), 0);
    next(cx);
    next(cx);
    assert_eq!(head(&editor, cx), p(0, 3));
    assert_eq!(highlighted(cx), 0);
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

/// Monaco `ClickLinkGesture`: Cmd/Ctrl+click goes to the definition on
/// mouse-up, on the row the mouse went down on with the modifier held; the
/// mouse-down places the cursor as usual, so Cmd/Ctrl+drag selects.
#[gpui::test]
fn definition_clicks_run_on_mouse_up(cx: &mut TestAppContext) {
    let fixed = Fixed {
        definitions: vec![here(0, 3, 6)],
        ..Default::default()
    };
    let (editor, cx) = setup(fixed, cx);
    let cmd = gpui::Modifiers::secondary_key();
    let left = gpui::MouseButton::Left;
    let at = mouse_at(&editor, cx, p(2, 5));
    cx.simulate_mouse_move(at, None, cmd);
    cx.executor().advance_clock(Duration::from_millis(60));
    cx.run_until_parked();
    cx.simulate_mouse_down(at, left, cmd);
    draw(cx);
    assert_eq!(head(&editor, cx), p(2, 5));
    cx.simulate_mouse_up(at, left, cmd);
    draw(cx);
    assert_eq!(head(&editor, cx), p(0, 3));

    // Cmd/Ctrl+drag selects: the drag cancels the gesture.
    let to = mouse_at(&editor, cx, p(2, 7));
    cx.simulate_mouse_move(at, None, cmd);
    cx.executor().advance_clock(Duration::from_millis(60));
    cx.run_until_parked();
    cx.simulate_mouse_down(at, left, cmd);
    cx.simulate_mouse_move(to, left, cmd);
    cx.simulate_mouse_up(to, left, cmd);
    draw(cx);
    assert_eq!(
        editor.read_with(cx, |e, _| e.selections()[0]),
        Selection {
            anchor: p(2, 5),
            head: p(2, 7)
        }
    );

    // Released on another row: nothing.
    set_cursor(&editor, cx, p(1, 0));
    let other_row = mouse_at(&editor, cx, p(3, 5));
    cx.simulate_mouse_down(at, left, cmd);
    cx.simulate_mouse_up(other_row, left, cmd);
    draw(cx);
    assert_eq!(head(&editor, cx), p(2, 5));

    // The modifier pressed after the mouse went down doesn't count (to copy
    // a selection, say).
    cx.simulate_mouse_down(at, left, gpui::Modifiers::default());
    cx.simulate_mouse_up(at, left, cmd);
    draw(cx);
    assert_eq!(head(&editor, cx), p(2, 5));

    // A Cmd/Ctrl+click before the link shows still goes there.
    let quick = mouse_at(&editor, cx, p(3, 5));
    cx.simulate_click(quick, cmd);
    draw(cx);
    assert_eq!(head(&editor, cx), p(0, 3));
}

/// Monaco's `.goto-definition-link`: `editorLink.activeForeground`.
#[gpui::test]
fn definition_links_use_the_link_color(cx: &mut TestAppContext) {
    let fixed = Fixed {
        definitions: vec![here(0, 3, 6)],
        ..Default::default()
    };
    let (editor, cx) = setup(fixed, cx);
    let at = mouse_at(&editor, cx, p(2, 5));
    cx.simulate_mouse_move(at, None, gpui::Modifiers::secondary_key());
    cx.executor().advance_clock(Duration::from_millis(60));
    cx.run_until_parked();
    let id = editor.entity_id();
    let model = editor.read_with(cx, |e, _| e.model().clone());
    let link = editor.read_with(cx, |e, _| e.colors().link_active_foreground);
    let styles: Vec<_> = model.update(cx, |m, _| {
        m.all_decorations(Some(id))
            .into_iter()
            .filter_map(|d| Some((d.options.text?.foreground, d.options.underline?.color)))
            .collect()
    });
    assert_eq!(styles, vec![(Some(link), link)]);
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

/// Cmd/Ctrl+hover at `at` until the hover shows; its markdown.
fn definition_hover(
    editor: &Entity<CodeEditor>,
    cx: &mut VisualTestContext,
    at: PointUtf16,
) -> Vec<String> {
    let position = mouse_at(editor, cx, at);
    cx.simulate_mouse_move(position, None, gpui::Modifiers::secondary_key());
    cx.executor().advance_clock(Duration::from_millis(60));
    cx.run_until_parked();
    cx.executor().advance_clock(Duration::from_millis(400));
    draw(cx);
    editor.read_with(cx, |e, cx| e.hover_contents_for_tests(cx))
}

#[gpui::test]
fn definition_links_show_the_definition_in_a_hover(cx: &mut TestAppContext) {
    let fixed = Fixed {
        definitions: vec![here(0, 0, 17)],
        ..Default::default()
    };
    let (editor, cx) = setup(fixed, cx);
    // Monaco's `goto-definition-link` hover: the definition's code.
    assert_eq!(
        definition_hover(&editor, cx, p(2, 5)),
        ["\n```\nfn add(a: i32) {}\n```\n"]
    );
}

#[gpui::test]
fn several_definitions_say_how_many(cx: &mut TestAppContext) {
    let fixed = Fixed {
        definitions: vec![here(0, 3, 6), here(1, 3, 7)],
        ..Default::default()
    };
    let (editor, cx) = setup(fixed, cx);
    assert_eq!(
        definition_hover(&editor, cx, p(3, 5)),
        ["Click to show 2 definitions."]
    );
}

#[gpui::test]
fn definitions_in_other_models_are_previewed(cx: &mut TestAppContext) {
    let uri = "file:///src/lib.rs";
    let fixed = Fixed {
        definitions: vec![Location {
            uri: Some(uri.into()),
            range: p(1, 0)..p(4, 1),
            ..Default::default()
        }],
        ..Default::default()
    };
    let lib = cx.new(|_| {
        TextModel::new("// lib\n    pub fn add() {\n        1\n    }\n}\n")
            .with_uri(uri)
            .with_language("rust")
    });
    cx.update(|cx| gpui_monaco::register_model(cx, &lib));
    let (editor, cx) = setup(fixed, cx);
    // In that model's language, without the common indentation.
    assert_eq!(
        definition_hover(&editor, cx, p(2, 5)),
        ["\n```rust\npub fn add() {\n    1\n}\n}\n```\n"]
    );
}

#[gpui::test]
fn the_definition_preview_hover_action_shows_it_at_the_cursor(cx: &mut TestAppContext) {
    let fixed = Fixed {
        definitions: vec![here(0, 0, 17)],
        ..Default::default()
    };
    let (editor, cx) = setup(fixed, cx);
    set_cursor(&editor, cx, p(2, 5));
    cx.update(|window, cx| {
        editor.update(cx, |e, cx| {
            e.trigger("editor.action.showDefinitionPreviewHover", window, cx)
        })
    })
    .unwrap();
    cx.executor().advance_clock(Duration::from_millis(60));
    draw(cx);
    assert_eq!(
        editor.read_with(cx, |e, cx| e.hover_contents_for_tests(cx)),
        ["\n```\nfn add(a: i32) {}\n```\n"]
    );
    // Moving the cursor removes the link: the hover has nothing to show
    // there anymore.
    set_cursor(&editor, cx, p(2, 6));
    editor.update(cx, |e, cx| e.show_hover_at_cursor(cx));
    draw(cx);
    assert!(
        editor
            .read_with(cx, |e, cx| e.hover_contents_for_tests(cx))
            .is_empty()
    );
}

#[gpui::test]
fn stable_peek_keeps_the_peek_view_open_on_escape(cx: &mut TestAppContext) {
    let fixed = Fixed {
        references: vec![here(0, 3, 6), here(2, 4, 7), here(3, 4, 7)],
        ..Default::default()
    };
    let (editor, cx) = setup(fixed, cx);
    // Monaco `stablePeek`: Escape doesn't close peek views.
    editor.update(cx, |e, cx| {
        let mut options = e.options().clone();
        options.stable_peek = true;
        e.update_options(options, cx);
    });
    set_cursor(&editor, cx, p(2, 5));
    cx.simulate_keystrokes("shift-f12");
    draw(cx);
    assert!(editor.read_with(cx, |e, _| e.is_peek_visible()));
    cx.simulate_keystrokes("escape");
    draw(cx);
    assert!(editor.read_with(cx, |e, _| e.is_peek_visible()));
    // From the editor too.
    cx.update(|window, cx| window.focus(&editor.focus_handle(cx), cx));
    cx.simulate_keystrokes("escape");
    draw(cx);
    assert!(editor.read_with(cx, |e, _| e.is_peek_visible()));
    // Without it, Escape closes them.
    editor.update(cx, |e, cx| {
        let mut options = e.options().clone();
        options.stable_peek = false;
        e.update_options(options, cx);
    });
    cx.simulate_keystrokes("escape");
    draw(cx);
    assert!(!editor.read_with(cx, |e, _| e.is_peek_visible()));
}

/// Monaco `LocationLink`: navigation selects the `targetSelectionRange`
/// (the definition's name) while the hover shows the whole `range`, and
/// Cmd/Ctrl+hover underlines the `originSelectionRange`.
#[gpui::test]
fn location_links_select_and_underline_their_ranges(cx: &mut TestAppContext) {
    let fixed = Fixed {
        definitions: vec![Location {
            range: p(0, 0)..p(0, 17),
            target_selection_range: Some(p(0, 3)..p(0, 6)),
            origin_selection_range: Some(p(2, 4)..p(2, 10)),
            ..Default::default()
        }],
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
    let highlighted: Vec<_> = model.update(cx, |m, _| {
        m.all_decorations(Some(id))
            .into_iter()
            .filter(|d| d.options.background == Some(color))
            .map(|d| d.range)
            .collect()
    });
    assert_eq!(highlighted, vec![3..6usize]);

    // The whole range in the hover, the origin range underlined: `add(1)`
    // on line 2 (bytes 34..40).
    assert_eq!(
        definition_hover(&editor, cx, p(2, 5)),
        ["\n```\nfn add(a: i32) {}\n```\n"]
    );
    let underlined: Vec<_> = model.update(cx, |m, _| {
        m.all_decorations(Some(id))
            .into_iter()
            .filter(|d| d.options.underline.is_some())
            .map(|d| d.range)
            .collect()
    });
    assert_eq!(underlined, vec![34..40usize]);
}

/// `fn add(a: i32) {}` as a `LocationLink`: the whole function, with its
/// name as the target selection range.
fn add_link() -> Location {
    Location {
        range: p(0, 0)..p(0, 17),
        target_selection_range: Some(p(0, 3)..p(0, 6)),
        ..Default::default()
    }
}

/// Monaco `referenceAt` tests `OneReference.range`, a link's
/// `targetSelectionRange`: inside the definition's body but off its name,
/// F12 goes to the name instead of running the alternative.
#[gpui::test]
fn the_alternative_runs_only_on_the_target_selection_range(cx: &mut TestAppContext) {
    let fixed = Fixed {
        definitions: vec![add_link()],
        references: vec![here(0, 3, 6), here(2, 4, 7), here(3, 4, 7)],
        ..Default::default()
    };
    let log = fixed.log.clone();
    let (editor, cx) = setup(fixed, cx);
    // In the parameter list, inside the link's full range.
    set_cursor(&editor, cx, p(0, 10));
    cx.simulate_keystrokes("f12");
    draw(cx);
    assert_eq!(*log.borrow(), ["definition"]);
    assert!(!editor.read_with(cx, |e, _| e.is_peek_visible()));
    assert_eq!(head(&editor, cx), p(0, 3));
    // On the name: Go to References.
    log.borrow_mut().clear();
    set_cursor(&editor, cx, p(0, 6));
    cx.simulate_keystrokes("f12");
    draw(cx);
    assert_eq!(*log.borrow(), ["definition", "references"]);
    assert!(editor.read_with(cx, |e, _| e.is_peek_visible()));
}

/// F12 stepping stops once the cursor leaves the results' target selection
/// ranges (Monaco `SymbolNavigationService`), and the peek view selects
/// the result whose name is nearest (`nearestReference`).
#[gpui::test]
fn stepping_and_the_peek_use_the_target_selection_range(cx: &mut TestAppContext) {
    let link = |row: u32| Location {
        range: p(row, 0)..p(row, 11),
        target_selection_range: Some(p(row, 4)..p(row, 7)),
        ..Default::default()
    };
    let fixed = Fixed {
        implementations: vec![link(2), link(3)],
        ..Default::default()
    };
    let (editor, cx) = setup(fixed, cx);
    set_goto(&editor, cx, |o| {
        o.multiple_implementations = GoToLocation::Goto
    });
    set_cursor(&editor, cx, p(1, 3));
    editor.update(cx, |e, cx| e.go_to(LocationKind::Implementation, cx));
    cx.run_until_parked();
    assert_eq!(head(&editor, cx), p(2, 4));
    assert!(cx.update(|_, cx| gpui_monaco::has_symbol_navigation(cx)));
    // Still inside the full range, off the name.
    set_cursor(&editor, cx, p(2, 9));
    assert!(!cx.update(|_, cx| gpui_monaco::has_symbol_navigation(cx)));

    // The peek view selects the result whose name is nearest the cursor.
    set_goto(&editor, cx, |o| {
        o.multiple_implementations = GoToLocation::Peek
    });
    editor.update(cx, |e, cx| {
        let link = |row: u32, name: u32| Location {
            range: p(row, 0)..p(row, 11),
            target_selection_range: Some(p(row, name)..p(row, name + 1)),
            ..Default::default()
        };
        // Full range starts: (2,0) is 100+9 = 109 away, (4,0) also 109;
        // names: (2,1) is 108 away, (4,9) is 100.
        e.peek_locations(p(3, 9), vec![link(2, 1), link(4, 9)], None, cx)
    });
    draw(cx);
    let (_, selected) = editor.read_with(cx, |e, cx| e.peek_results(cx)).unwrap();
    assert_eq!(selected, 1);
}

/// Monaco `getReferencesAtPosition` with `compact`: Go to References asks
/// again without the declaration when a provider finds exactly two, and
/// takes that answer if it has one; Peek References doesn't.
#[gpui::test]
fn go_to_references_is_compact(cx: &mut TestAppContext) {
    let fixed = Fixed {
        references: vec![here(0, 3, 6), here(2, 4, 7)],
        references_without_declaration: Some(vec![here(2, 4, 7)]),
        ..Default::default()
    };
    let log = fixed.log.clone();
    let (editor, cx) = setup(fixed, cx);
    set_cursor(&editor, cx, p(0, 4));
    cx.simulate_keystrokes("shift-f12");
    draw(cx);
    assert_eq!(
        *log.borrow(),
        ["references", "references without declaration"]
    );
    assert!(!editor.read_with(cx, |e, _| e.is_peek_visible()));
    assert_eq!(head(&editor, cx), p(2, 4));

    // Peek References shows both.
    log.borrow_mut().clear();
    set_cursor(&editor, cx, p(0, 4));
    editor.update(cx, |e, cx| e.peek(LocationKind::References, cx));
    draw(cx);
    assert_eq!(*log.borrow(), ["references"]);
    let (locations, _) = editor.read_with(cx, |e, cx| e.peek_results(cx)).unwrap();
    assert_eq!(locations.len(), 2);
}

#[gpui::test]
fn compact_references_keep_the_first_answer_unless_it_has_one(cx: &mut TestAppContext) {
    // Two without the declaration too: the first answer stays.
    let fixed = Fixed {
        references: vec![here(0, 3, 6), here(2, 4, 7)],
        ..Default::default()
    };
    let log = fixed.log.clone();
    let (editor, cx) = setup(fixed, cx);
    set_cursor(&editor, cx, p(0, 4));
    cx.simulate_keystrokes("shift-f12");
    draw(cx);
    assert_eq!(
        *log.borrow(),
        ["references", "references without declaration"]
    );
    let (locations, _) = editor.read_with(cx, |e, cx| e.peek_results(cx)).unwrap();
    assert_eq!(locations.len(), 2);
}

#[gpui::test]
fn compact_references_ask_again_only_for_two(cx: &mut TestAppContext) {
    let fixed = Fixed {
        references: vec![here(0, 3, 6), here(2, 4, 7), here(3, 4, 7)],
        references_without_declaration: Some(vec![here(2, 4, 7)]),
        ..Default::default()
    };
    let log = fixed.log.clone();
    let (editor, cx) = setup(fixed, cx);
    set_cursor(&editor, cx, p(0, 4));
    cx.simulate_keystrokes("shift-f12");
    draw(cx);
    assert_eq!(*log.borrow(), ["references"]);
    let (locations, _) = editor.read_with(cx, |e, cx| e.peek_results(cx)).unwrap();
    assert_eq!(locations.len(), 3);
}

/// The alternative runs as its own command (Monaco runs the registered
/// action with its `SymbolNavigationActionConfig`): from Peek Definition,
/// Go to References with one result goes there without a peek view.
#[gpui::test]
fn the_alternative_runs_with_its_own_configuration(cx: &mut TestAppContext) {
    let fixed = Fixed {
        definitions: vec![here(0, 3, 6)],
        references: vec![here(2, 4, 7)],
        ..Default::default()
    };
    let log = fixed.log.clone();
    let (editor, cx) = setup(fixed, cx);
    set_cursor(&editor, cx, p(0, 4));
    editor.update(cx, |e, cx| e.peek(LocationKind::Definition, cx));
    draw(cx);
    assert_eq!(*log.borrow(), ["definition", "references"]);
    assert!(!editor.read_with(cx, |e, _| e.is_peek_visible()));
    assert_eq!(head(&editor, cx), p(2, 4));

    // Naming Peek References opens even one result in a peek view.
    set_goto(&editor, cx, |o| {
        o.alternative_definition_command = Some(NavigationCommand::PeekReferences)
    });
    set_cursor(&editor, cx, p(0, 4));
    cx.simulate_keystrokes("f12");
    draw(cx);
    let (locations, _) = editor.read_with(cx, |e, cx| e.peek_results(cx)).unwrap();
    assert_eq!(locations.len(), 1);
}

/// Monaco `_activeAlternativeCommands`: a command whose alternative is
/// running can't run as an alternative again, so chains end.
#[gpui::test]
fn chains_of_alternatives_end(cx: &mut TestAppContext) {
    let fixed = Fixed {
        definitions: vec![here(0, 3, 6)],
        references: vec![here(0, 3, 6)],
        ..Default::default()
    };
    let log = fixed.log.clone();
    let (editor, cx) = setup(fixed, cx);
    // Definition -> references -> peek definition, whose alternative (Go to
    // References) already ran one.
    set_goto(&editor, cx, |o| {
        o.alternative_reference_command = Some(NavigationCommand::PeekDefinition)
    });
    set_cursor(&editor, cx, p(0, 4));
    cx.simulate_keystrokes("f12");
    draw(cx);
    assert_eq!(*log.borrow(), ["definition", "references", "definition"]);
    assert!(editor.read_with(cx, |e, _| e.is_peek_visible()));

    // Back to the command that started it: it doesn't run again.
    cx.simulate_keystrokes("escape");
    log.borrow_mut().clear();
    set_goto(&editor, cx, |o| {
        o.alternative_reference_command = Some(NavigationCommand::RevealDefinition)
    });
    set_cursor(&editor, cx, p(0, 4));
    cx.simulate_keystrokes("f12");
    draw(cx);
    assert_eq!(*log.borrow(), ["definition", "references"]);
    assert!(!editor.read_with(cx, |e, _| e.is_peek_visible()));
    assert_eq!(head(&editor, cx), p(0, 3));
}

fn visible_rows(editor: &Entity<CodeEditor>, cx: &mut VisualTestContext) -> (u32, u32) {
    editor.read_with(cx, |e, cx| {
        let ranges = e.visible_ranges(cx);
        (ranges[0].start.row, ranges.last().unwrap().end.row)
    })
}

/// Standalone Monaco's `openCodeEditor` centers a jump's target at once
/// (`StandaloneCodeEditorService.doOpenEditor`), even when it's visible and
/// with `smooth_scrolling`.
#[gpui::test]
fn jumps_in_this_model_center_the_target_at_once(cx: &mut TestAppContext) {
    let text: String = (0..400).map(|i| format!("line {i}\n")).collect();
    let fixed = Fixed {
        definitions: vec![here(300, 0, 4)],
        ..Default::default()
    };
    let (editor, cx) = setup_with(&text, fixed, cx);
    editor.update(cx, |e, cx| {
        let mut options = e.options().clone();
        options.smooth_scrolling = true;
        e.update_options(options, cx);
    });
    draw(cx);
    assert_eq!(visible_rows(&editor, cx).0, 0);
    cx.simulate_keystrokes("f12");
    // One frame, no clock: no animation.
    draw(cx);
    assert_eq!(head(&editor, cx), p(300, 0));
    let (top, bottom) = visible_rows(&editor, cx);
    assert!((top + bottom).abs_diff(600) <= 2, "{top}..{bottom}");

    // A visible target is centered too.
    let near = bottom - 2;
    editor.update(cx, |e, cx| {
        e.go_to_locations(p(300, 0), vec![here(near, 0, 4)], None, None, cx)
    });
    draw(cx);
    assert_eq!(head(&editor, cx), p(near, 0));
    let (top, bottom) = visible_rows(&editor, cx);
    assert!((top + bottom).abs_diff(2 * near) <= 2, "{top}..{bottom}");
}
