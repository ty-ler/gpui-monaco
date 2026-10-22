//! The context menu (Monaco `ContextMenuController`): Shift+F10 and
//! right-click, groups, keyboard navigation and host actions.

use gpui::{AppContext as _, Entity, Focusable, TestAppContext, VisualTestContext};
use gpui_monaco::text::PointUtf16;
use gpui_monaco::{CodeEditor, EditorActionDescriptor, TextModel};

fn setup<'a>(
    text: &str,
    cx: &'a mut TestAppContext,
) -> (Entity<CodeEditor>, &'a mut VisualTestContext) {
    cx.update(gpui_monaco::init);
    let text = text.to_string();
    let (editor, cx) = cx.add_window_view(|window, cx| {
        let model = cx.new(|_| TextModel::new(&text));
        CodeEditor::new(model, window, cx)
    });
    cx.update(|window, cx| window.focus(&editor.focus_handle(cx), cx));
    cx.run_until_parked();
    (editor, cx)
}

fn rows(editor: &Entity<CodeEditor>, cx: &mut VisualTestContext) -> Vec<String> {
    cx.run_until_parked();
    editor.read_with(cx, |e, cx| e.context_menu_rows_for_tests(cx))
}

#[gpui::test]
fn shift_f10_shows_the_default_menu(cx: &mut TestAppContext) {
    let (editor, cx) = setup("foo bar foo", cx);
    cx.simulate_keystrokes("shift-f10");
    // No language providers: no go to definition, rename or formatting,
    // and no symbols to go to (Monaco's `hasDocumentSymbolProvider`).
    assert_eq!(
        rows(&editor, cx),
        vec![
            "Change All Occurrences",
            "---",
            "Cut",
            "Copy",
            "Paste",
            "---",
            "Command Palette",
        ]
    );
    // Keyboard: down to "Change All Occurrences", Enter runs it.
    cx.simulate_keystrokes("down");
    assert_eq!(rows(&editor, cx)[0], "> Change All Occurrences");
    cx.simulate_keystrokes("enter");
    cx.run_until_parked();
    assert!(!editor.read_with(cx, |e, _| e.is_context_menu_visible()));
    assert_eq!(editor.read_with(cx, |e, _| e.selections().len()), 2);
}

#[gpui::test]
fn escape_and_options(cx: &mut TestAppContext) {
    let (editor, cx) = setup("foo", cx);
    cx.simulate_keystrokes("shift-f10");
    assert!(editor.read_with(cx, |e, _| e.is_context_menu_visible()));
    cx.simulate_keystrokes("escape");
    cx.run_until_parked();
    assert!(!editor.read_with(cx, |e, _| e.is_context_menu_visible()));

    // Read-only: no editing items.
    editor.update(cx, |e, cx| {
        let mut options = e.options().clone();
        options.read_only = true;
        e.update_options(options, cx);
    });
    cx.simulate_keystrokes("shift-f10");
    assert_eq!(rows(&editor, cx), vec!["Copy", "---", "Command Palette"]);
    cx.simulate_keystrokes("escape");

    // Off: nothing.
    editor.update(cx, |e, cx| {
        let mut options = e.options().clone();
        options.context_menu = false;
        e.update_options(options, cx);
    });
    cx.simulate_keystrokes("shift-f10");
    assert!(rows(&editor, cx).is_empty());
}

#[gpui::test]
fn host_actions_join_their_group(cx: &mut TestAppContext) {
    let (editor, cx) = setup("abc", cx);
    editor.update(cx, |e, cx| {
        let _ = e.add_action(
            EditorActionDescriptor::new("my.upper", "Make Upper", |editor, _, cx| {
                editor.model().update(cx, |m, cx| m.set_value("ABC", cx));
            })
            .context_menu("1_modification", 0.5),
            cx,
        );
        let _ = e.add_action(
            EditorActionDescriptor::new("my.hidden", "Not In Menu", |_, _, _| {}),
            cx,
        );
    });
    cx.simulate_keystrokes("shift-f10");
    let menu = rows(&editor, cx);
    assert_eq!(menu[0], "Make Upper");
    assert!(!menu.iter().any(|r| r.contains("Not In Menu")));
    cx.simulate_keystrokes("down enter");
    cx.run_until_parked();
    assert_eq!(
        editor.read_with(cx, |e, cx| e.model().read(cx).text()),
        "ABC"
    );
}

fn draw(cx: &mut VisualTestContext) {
    cx.run_until_parked();
    cx.update(|window, cx| {
        window.refresh();
        window.draw(cx).clear(cx);
    });
    cx.run_until_parked();
}

fn right_click(cx: &mut VisualTestContext, at: gpui::Point<gpui::Pixels>) {
    cx.simulate_mouse_down(at, gpui::MouseButton::Right, gpui::Modifiers::default());
    cx.simulate_mouse_up(at, gpui::MouseButton::Right, gpui::Modifiers::default());
    cx.run_until_parked();
}

/// Monaco's menu sizes to its widest row (no fixed 260px).
#[gpui::test]
fn the_menu_is_as_wide_as_its_rows(cx: &mut TestAppContext) {
    let (editor, cx) = setup("foo", cx);
    cx.simulate_keystrokes("shift-f10");
    let narrow = editor
        .read_with(cx, |e, cx| e.context_menu_width_for_tests(cx))
        .unwrap();
    cx.simulate_keystrokes("escape");
    cx.run_until_parked();
    let long = "A host action with a label much longer than the fixed menu width was";
    editor.update(cx, |editor, cx| {
        let _ = editor.add_action(
            EditorActionDescriptor::new("my.long", long, |_, _, _| {})
                .context_menu("navigation", 1.)
                .keybinding("ctrl-alt-h"),
            cx,
        );
    });
    cx.simulate_keystrokes("shift-f10");
    let wide = editor
        .read_with(cx, |e, cx| e.context_menu_width_for_tests(cx))
        .unwrap();
    assert!(wide > narrow + gpui::px(100.), "{wide:?} {narrow:?}");
    let label = editor.read_with(cx, |e, cx| {
        e.context_menu_rows_for_tests(cx)
            .into_iter()
            .find(|row| row.starts_with("A host action"))
    });
    assert_eq!(label.as_deref(), Some(long));
}

/// Monaco `_doShowContextMenu`: Shift+F10 reveals the cursor first, then
/// anchors the menu below it.
#[gpui::test]
fn shift_f10_reveals_the_cursor(cx: &mut TestAppContext) {
    let (editor, cx) = setup(&"foo\n".repeat(300), cx);
    editor.update(cx, |e, cx| {
        e.set_selections(
            &[gpui_monaco::cursor::Selection::collapsed(PointUtf16::new(
                250, 0,
            ))],
            cx,
        );
    });
    draw(cx);
    editor.update(cx, |e, cx| {
        e.set_scroll_position(gpui::point(gpui::px(0.), gpui::px(0.)), cx)
    });
    draw(cx);
    cx.simulate_keystrokes("shift-f10");
    draw(cx);
    assert!(editor.read_with(cx, |e, _| e.is_context_menu_visible()));
    let top = editor.read_with(cx, |e, _| e.scroll_position().y);
    assert!(top > gpui::px(1000.), "{top:?}");
    // The menu is anchored at the cursor's line, now in view: below it, or
    // flipped above it at the window's bottom.
    let cursor = editor
        .read_with(cx, |e, _| {
            e.scrolled_visible_position(PointUtf16::new(250, 0))
        })
        .unwrap();
    let (_, widgets) = editor.read_with(cx, |e, _| e.widget_bounds_for_tests());
    let menu = widgets.last().unwrap().0;
    assert!(
        menu.top() > cursor.y || menu.bottom() <= cursor.y + gpui::px(30.),
        "{menu:?} {cursor:?}"
    );
}

/// Monaco's sticky scroll has its own menu: Toggle Editor Sticky Scroll,
/// checked, as "Sticky Scroll".
#[gpui::test]
fn sticky_scroll_lines_have_their_own_menu(cx: &mut TestAppContext) {
    let text = format!("fn main() {{\n{}}}\n", "    foo();\n".repeat(200));
    let (editor, cx) = setup(&text, cx);
    editor.update(cx, |e, cx| {
        e.set_scroll_position(gpui::point(gpui::px(0.), gpui::px(400.)), cx)
    });
    draw(cx);
    let (rows, _) = editor.read_with(cx, |e, _| e.sticky_rows_for_tests());
    assert_eq!(rows, [0]);
    let (bounds, _) = editor.read_with(cx, |e, _| e.widget_bounds_for_tests());
    right_click(
        cx,
        bounds.origin + gpui::point(gpui::px(120.), gpui::px(5.)),
    );
    assert_eq!(
        editor.read_with(cx, |e, cx| e.context_menu_rows_for_tests(cx)),
        ["✓ Sticky Scroll"]
    );
    // The menu has the focus.
    cx.simulate_keystrokes("down enter");
    draw(cx);
    assert!(!editor.read_with(cx, |e, _| e.options().sticky_scroll.enabled));
}

/// Monaco leaves right-clicks on injected text (inlay hints) to its own
/// menus: the editor's doesn't open there.
#[gpui::test]
fn no_editor_menu_on_injected_text(cx: &mut TestAppContext) {
    use gpui_monaco::decorations::{DecorationOptions, InjectedText};
    let (editor, cx) = setup("foo bar foo", cx);
    let _collection = editor.update(cx, |e, cx| {
        e.create_decorations_collection(
            vec![(
                PointUtf16::new(0, 4)..PointUtf16::new(0, 7),
                DecorationOptions {
                    before: Some(InjectedText {
                        content: "INJECTED: ".into(),
                        ..Default::default()
                    }),
                    ..Default::default()
                },
            )],
            cx,
        )
    });
    draw(cx);
    let after = editor
        .read_with(cx, |e, _| {
            e.scrolled_visible_position(PointUtf16::new(0, 4))
        })
        .unwrap();
    // On the injected text, just left of the column it's before.
    right_click(cx, after + gpui::point(gpui::px(-10.), gpui::px(5.)));
    assert!(!editor.read_with(cx, |e, _| e.is_context_menu_visible()));
    // On the text itself, it does.
    right_click(cx, after + gpui::point(gpui::px(5.), gpui::px(5.)));
    assert!(editor.read_with(cx, |e, _| e.is_context_menu_visible()));
}

/// A code action provider declaring `kinds` (Monaco
/// `providedCodeActionKinds`).
struct Kinds(Option<Vec<gpui_monaco::providers::CodeActionKind>>);

impl gpui_monaco::providers::CodeActionProvider for Kinds {
    fn provide_code_actions(
        &self,
        _: &gpui_monaco::providers::ProviderContext,
        _: std::ops::Range<PointUtf16>,
        _: &gpui_monaco::providers::CodeActionContext,
        _: &mut gpui::App,
    ) -> gpui::Task<anyhow::Result<Vec<gpui_monaco::providers::CodeAction>>> {
        gpui::Task::ready(Ok(Vec::new()))
    }

    fn provided_kinds(&self) -> Option<Vec<gpui_monaco::providers::CodeActionKind>> {
        self.0.clone()
    }
}

/// Monaco's Refactor... and Source Action... items need a provider that
/// declares those kinds (`contextKeyForSupportedActions`); one without
/// `providedCodeActionKinds` doesn't show them.
#[gpui::test]
fn refactor_and_source_items_need_declared_kinds(cx: &mut TestAppContext) {
    use gpui_monaco::providers::{LanguageSelector, register_code_action_provider};
    let (editor, cx) = setup("foo", cx);
    cx.update(|_, cx| register_code_action_provider(cx, LanguageSelector::All, Kinds(None)));
    draw(cx);
    cx.simulate_keystrokes("shift-f10");
    let menu = rows(&editor, cx);
    assert!(
        !menu
            .iter()
            .any(|r| r == "Refactor..." || r == "Source Action...")
    );
    cx.simulate_keystrokes("escape");
    cx.update(|_, cx| {
        register_code_action_provider(
            cx,
            LanguageSelector::All,
            Kinds(Some(vec!["refactor.extract".into()])),
        )
    });
    draw(cx);
    cx.simulate_keystrokes("shift-f10");
    let menu = rows(&editor, cx);
    assert!(menu.iter().any(|r| r == "Refactor..."), "{menu:?}");
    assert!(!menu.iter().any(|r| r == "Source Action..."));
}

/// Monaco `MouseHandler._onMouseDown`: on macOS, Ctrl+click isn't a click
/// (no selection drag, whatever the `multiCursorModifier`); it opens the
/// context menu, as a right-click does. Elsewhere Ctrl+click stays a click.
#[gpui::test]
fn ctrl_click_opens_the_menu_on_macos(cx: &mut TestAppContext) {
    use gpui::{Modifiers, MouseButton, point, px};
    use gpui_monaco::options::MultiCursorModifier;
    let (editor, cx) = setup("foo bar foo\nbaz qux\n", cx);
    draw(cx);
    let at = |column: u32, cx: &mut VisualTestContext| {
        editor
            .read_with(cx, |e, _| {
                e.scrolled_visible_position(PointUtf16::new(0, column))
            })
            .unwrap()
            + point(px(1.), px(5.))
    };
    let ctrl = Modifiers {
        control: true,
        ..Default::default()
    };
    for modifier in [MultiCursorModifier::Alt, MultiCursorModifier::CtrlCmd] {
        editor.update(cx, |e, cx| {
            let mut options = e.options().clone();
            options.multi_cursor_modifier = modifier;
            e.update_options(options, cx);
            e.set_selections(
                &[gpui_monaco::cursor::Selection::collapsed(PointUtf16::new(
                    1, 0,
                ))],
                cx,
            );
        });
        draw(cx);
        let (from, to) = (at(4, cx), at(9, cx));
        cx.simulate_mouse_down(from, MouseButton::Left, ctrl);
        cx.simulate_mouse_move(to, Some(MouseButton::Left), ctrl);
        cx.simulate_mouse_up(to, MouseButton::Left, ctrl);
        draw(cx);
        let selections = editor.read_with(cx, |e, _| e.selections().to_vec());
        assert_eq!(selections.len(), 1, "{modifier:?}");
        assert!(selections[0].is_empty(), "{modifier:?}: {selections:?}");
        if cfg!(target_os = "macos") {
            assert!(editor.read_with(cx, |e, _| e.is_context_menu_visible()));
            // The menu's cursor move: to the click, outside the selection.
            assert_eq!(selections[0].head, PointUtf16::new(0, 4));
            cx.simulate_keystrokes("escape");
            draw(cx);
        } else {
            assert!(!editor.read_with(cx, |e, _| e.is_context_menu_visible()));
        }
    }
}

fn wheel(cx: &mut VisualTestContext, at: gpui::Point<gpui::Pixels>) {
    cx.simulate_event(gpui::ScrollWheelEvent {
        position: at,
        delta: gpui::ScrollDelta::Pixels(gpui::point(gpui::px(0.), gpui::px(-40.))),
        ..Default::default()
    });
    draw(cx);
}

/// Monaco `ContextMenuController`: the wheel in the editor hides the menu;
/// over the menu it doesn't.
#[gpui::test]
fn the_wheel_over_the_text_hides_the_menu(cx: &mut TestAppContext) {
    let (editor, cx) = setup(&"foo bar\n".repeat(200), cx);
    cx.simulate_keystrokes("shift-f10");
    draw(cx);
    let row = cx.debug_bounds("context-menu-0").expect("a menu row");
    wheel(cx, row.center());
    assert!(editor.read_with(cx, |e, _| e.is_context_menu_visible()));
    let (bounds, _) = editor.read_with(cx, |e, _| e.widget_bounds_for_tests());
    wheel(
        cx,
        gpui::point(
            bounds.right() - gpui::px(200.),
            bounds.bottom() - gpui::px(20.),
        ),
    );
    assert!(!editor.read_with(cx, |e, _| e.is_context_menu_visible()));
    // And the text still scrolled.
    assert!(editor.read_with(cx, |e, _| e.scroll_position().y) > gpui::px(0.));
}

/// VS Code's menu runs an item on mouse up: a press dragged off the item
/// and released elsewhere runs nothing.
#[gpui::test]
fn items_run_on_mouse_up(cx: &mut TestAppContext) {
    use gpui::{Modifiers, MouseButton};
    let (editor, cx) = setup("foo bar foo", cx);
    cx.simulate_keystrokes("shift-f10");
    draw(cx);
    let item = cx
        .debug_bounds("context-menu-0")
        .expect("Change All Occurrences")
        .center();
    cx.simulate_mouse_down(item, MouseButton::Left, Modifiers::default());
    draw(cx);
    assert!(editor.read_with(cx, |e, _| e.is_context_menu_visible()));
    assert_eq!(editor.read_with(cx, |e, _| e.selections().len()), 1);
    // Dragged off and released over a separator: nothing runs.
    let separator = gpui::point(item.x, item.y + gpui::px(ROW_PITCH));
    cx.simulate_mouse_move(separator, MouseButton::Left, Modifiers::default());
    cx.simulate_mouse_up(separator, MouseButton::Left, Modifiers::default());
    draw(cx);
    assert!(editor.read_with(cx, |e, _| e.is_context_menu_visible()));
    assert_eq!(editor.read_with(cx, |e, _| e.selections().len()), 1);
    // A click runs it.
    cx.simulate_mouse_down(item, MouseButton::Left, Modifiers::default());
    cx.simulate_mouse_up(item, MouseButton::Left, Modifiers::default());
    draw(cx);
    assert!(!editor.read_with(cx, |e, _| e.is_context_menu_visible()));
    assert_eq!(editor.read_with(cx, |e, _| e.selections().len()), 2);
}

/// From a row's center to the separator after it.
const ROW_PITCH: f32 = 15.;

/// Monaco `_onContextMenu` with `contextmenu: false` keeps the cursors only
/// when the *primary* selection contains the click (`getSelection`); with
/// the menu, any selection does (#491).
#[gpui::test]
fn without_the_menu_only_the_primary_selection_keeps_the_cursors(cx: &mut TestAppContext) {
    use gpui::{point, px};
    use gpui_monaco::cursor::Selection;
    let (editor, cx) = setup("one two\nthree four\n", cx);
    draw(cx);
    let at = |row: u32, column: u32, cx: &mut VisualTestContext| {
        editor
            .read_with(cx, |e, _| {
                e.scrolled_visible_position(PointUtf16::new(row, column))
            })
            .unwrap()
            + point(px(1.), px(5.))
    };
    let two = vec![
        Selection::new(PointUtf16::new(0, 0), PointUtf16::new(0, 3)),
        Selection::new(PointUtf16::new(1, 0), PointUtf16::new(1, 5)),
    ];
    let set = |cx: &mut VisualTestContext| {
        editor.update(cx, |e, cx| e.set_selections(&two, cx));
        draw(cx);
    };
    // With the menu: the click in the secondary selection keeps both.
    set(cx);
    let p = at(1, 2, cx);
    right_click(cx, p);
    assert_eq!(editor.read_with(cx, |e, _| e.selections()), two);
    cx.simulate_keystrokes("escape");
    draw(cx);

    editor.update(cx, |e, cx| {
        let mut options = e.options().clone();
        options.context_menu = false;
        e.update_options(options, cx);
    });
    // Without: one cursor at the click.
    set(cx);
    let p = at(1, 2, cx);
    right_click(cx, p);
    assert_eq!(
        editor.read_with(cx, |e, _| e.selections()),
        vec![Selection::collapsed(PointUtf16::new(1, 2))]
    );
    // In the primary one, both stay.
    set(cx);
    let p = at(0, 2, cx);
    right_click(cx, p);
    assert_eq!(editor.read_with(cx, |e, _| e.selections()), two);
}

/// Runs logged by the alternative action tests' host actions.
type Runs = std::rc::Rc<std::cell::RefCell<Vec<&'static str>>>;

/// Adds "Primary" to the menu, with "Alternative" (as `alternative`
/// makes it) as its alternative, each with a key and logging its runs.
fn add_alternative(
    editor: &Entity<CodeEditor>,
    alternative: impl FnOnce(EditorActionDescriptor) -> EditorActionDescriptor,
    cx: &mut VisualTestContext,
) -> Runs {
    let runs = Runs::default();
    let log = |id: &'static str| {
        let runs = runs.clone();
        move |_: &mut CodeEditor, _: &mut gpui::Window, _: &mut gpui::Context<CodeEditor>| {
            runs.borrow_mut().push(id)
        }
    };
    editor.update(cx, |e, cx| {
        let _ = e.add_action(
            EditorActionDescriptor::new("my.primary", "Primary", log("primary"))
                .keybinding("ctrl-alt-p")
                .context_menu("navigation", 1.)
                .context_menu_alt("my.alternative"),
            cx,
        );
        let _ = e.add_action(
            alternative(
                EditorActionDescriptor::new("my.alternative", "Alternative", log("alternative"))
                    .keybinding("ctrl-alt-a"),
            ),
            cx,
        );
    });
    runs
}

/// The menu items a fresh frame shows: label and keybinding.
fn shown_items(cx: &mut VisualTestContext) -> Vec<(String, Option<String>)> {
    cx.run_until_parked();
    gpui_monaco::record_aria_for_tests();
    cx.update(|window, cx| {
        window.refresh();
        window.draw(cx).clear(cx);
    });
    gpui_monaco::recorded_aria_for_tests()
        .into_iter()
        .filter(|n| matches!(n.role, gpui::Role::MenuItem | gpui::Role::MenuItemCheckBox))
        .map(|n| {
            let label = n.label.unwrap_or_default().to_string();
            (label, n.keyshortcuts.map(|k| k.to_string()))
        })
        .collect()
}

/// `keystroke`'s label, as the menu shows a keybinding.
fn key_label(keystroke: &str) -> Option<String> {
    use gpui_monaco::keybinding_label::{OperatingSystem, ui_label};
    ui_label(
        &[gpui::Keystroke::parse(keystroke).unwrap()],
        OperatingSystem::current(),
    )
}

/// Whether Shift shows alternatives too (Monaco `isWindows || isLinux`).
const SHIFT_IS_ALT: bool = cfg!(any(target_os = "windows", target_os = "linux"));

/// Monaco `menu.test.ts` "Alt updates menu labels, mnemonics and state
/// without hover": while Alt (or Shift on Windows and Linux) is held, the
/// item shows its alternative, with the alternative's keybinding, and
/// shows itself again on release. Mnemonics aren't ported: the editor's
/// menu has none.
#[gpui::test]
fn alternatives_show_while_alt_is_held(cx: &mut TestAppContext) {
    use gpui::Modifiers;
    let (editor, cx) = setup("abc", cx);
    let _runs = add_alternative(&editor, |a| a, cx);
    let primary = ("Primary".to_string(), key_label("ctrl-alt-p"));
    let alternative = ("Alternative".to_string(), key_label("ctrl-alt-a"));
    for (modifiers, uses_alternative) in
        [(Modifiers::alt(), true), (Modifiers::shift(), SHIFT_IS_ALT)]
    {
        cx.simulate_modifiers_change(Modifiers::none());
        cx.simulate_keystrokes("shift-f10");
        assert_eq!(shown_items(cx)[0], primary);
        cx.simulate_modifiers_change(modifiers);
        let selected = if uses_alternative {
            &alternative
        } else {
            &primary
        };
        assert_eq!(&shown_items(cx)[0], selected, "{modifiers:?}");
        assert_eq!(rows(&editor, cx)[0], selected.0);
        // Released: the item again.
        cx.simulate_modifiers_change(Modifiers::none());
        assert_eq!(shown_items(cx)[0], primary);
        cx.simulate_modifiers_change(modifiers);
        assert_eq!(&shown_items(cx)[0], selected);
        cx.simulate_keystrokes("escape");
        cx.run_until_parked();
    }
    // Held when the menu opens (Monaco keeps the key status until the
    // menu hides): a right-click with Alt shows the alternative at once.
    cx.simulate_modifiers_change(Modifiers::none());
    draw(cx);
    let (bounds, _) = editor.read_with(cx, |e, _| e.widget_bounds_for_tests());
    let at = bounds.origin + gpui::point(gpui::px(100.), gpui::px(5.));
    cx.simulate_mouse_down(at, gpui::MouseButton::Right, Modifiers::alt());
    cx.simulate_mouse_up(at, gpui::MouseButton::Right, Modifiers::alt());
    assert_eq!(shown_items(cx)[0], alternative);
    cx.simulate_modifiers_change(Modifiers::none());
    assert_eq!(shown_items(cx)[0], primary);
}

/// Monaco `menu.test.ts` "selects alternatives at keyboard activation
/// time": Enter runs the item; with Alt (Shift too on Windows and Linux)
/// it runs the alternative. On macOS, Shift+Enter isn't a trigger key, so
/// nothing runs.
#[gpui::test]
fn enter_with_alt_runs_the_alternative(cx: &mut TestAppContext) {
    use gpui::Modifiers;
    let (editor, cx) = setup("abc", cx);
    let runs = add_alternative(&editor, |a| a, cx);
    let alt_shift = Modifiers {
        alt: true,
        shift: true,
        ..Default::default()
    };
    for (modifiers, enter) in [
        (Modifiers::none(), "enter"),
        (Modifiers::alt(), "alt-enter"),
        (Modifiers::shift(), "shift-enter"),
        (alt_shift, "alt-shift-enter"),
        (Modifiers::none(), "enter"),
    ] {
        cx.simulate_modifiers_change(Modifiers::none());
        if !editor.read_with(cx, |e, _| e.is_context_menu_visible()) {
            cx.simulate_keystrokes("shift-f10 down");
        }
        assert_eq!(rows(&editor, cx)[0], "> Primary");
        cx.simulate_modifiers_change(modifiers);
        cx.simulate_keystrokes(enter);
        cx.run_until_parked();
    }
    let shift: &[&str] = if SHIFT_IS_ALT { &["alternative"] } else { &[] };
    let expected: Vec<&str> = ["primary", "alternative"]
        .into_iter()
        .chain(shift.iter().copied())
        .chain(["alternative", "primary"])
        .collect();
    assert_eq!(*runs.borrow(), expected);
    assert_eq!(
        editor.read_with(cx, |e, cx| e.model().read(cx).text()),
        "abc"
    );
}

/// Monaco `menu.test.ts` "selects alternatives at mouse activation time":
/// a click runs the item, and with Alt (Shift too on Windows and Linux)
/// the alternative.
#[gpui::test]
fn clicking_with_alt_runs_the_alternative(cx: &mut TestAppContext) {
    use gpui::{Modifiers, MouseButton};
    let (editor, cx) = setup("abc", cx);
    let runs = add_alternative(&editor, |a| a, cx);
    let alt_shift = Modifiers {
        alt: true,
        shift: true,
        ..Default::default()
    };
    for modifiers in [
        Modifiers::none(),
        Modifiers::alt(),
        Modifiers::shift(),
        alt_shift,
        Modifiers::none(),
    ] {
        cx.simulate_modifiers_change(Modifiers::none());
        cx.simulate_keystrokes("shift-f10");
        draw(cx);
        let item = cx.debug_bounds("context-menu-0").expect("Primary").center();
        cx.simulate_mouse_down(item, MouseButton::Left, modifiers);
        cx.simulate_mouse_up(item, MouseButton::Left, modifiers);
        draw(cx);
        assert!(!editor.read_with(cx, |e, _| e.is_context_menu_visible()));
    }
    let shift = if SHIFT_IS_ALT {
        "alternative"
    } else {
        "primary"
    };
    assert_eq!(
        *runs.borrow(),
        ["primary", "alternative", shift, "alternative", "primary"]
    );
}

/// Monaco `menu.test.ts` "disabled alternatives fall back to the primary
/// action": an alternative that can't run isn't shown, Alt+click runs the
/// item, and Alt+Enter isn't a trigger key. An unknown id is no
/// alternative either.
#[gpui::test]
fn a_disabled_alternative_is_not_used(cx: &mut TestAppContext) {
    use gpui::{Modifiers, MouseButton};
    let (editor, cx) = setup("abc", cx);
    let runs = add_alternative(&editor, |a| a.precondition(|_, _| false), cx);
    cx.simulate_keystrokes("shift-f10 down");
    cx.simulate_modifiers_change(Modifiers::alt());
    assert_eq!(rows(&editor, cx)[0], "> Primary");
    cx.simulate_keystrokes("alt-enter");
    cx.run_until_parked();
    assert!(editor.read_with(cx, |e, _| e.is_context_menu_visible()));
    assert!(runs.borrow().is_empty());
    draw(cx);
    let item = cx.debug_bounds("context-menu-0").expect("Primary").center();
    cx.simulate_mouse_down(item, MouseButton::Left, Modifiers::alt());
    cx.simulate_mouse_up(item, MouseButton::Left, Modifiers::alt());
    draw(cx);
    assert_eq!(*runs.borrow(), ["primary"]);

    editor.update(cx, |e, cx| {
        let _ = e.add_action(
            EditorActionDescriptor::new("my.primary", "Primary", |_, _, _| {})
                .context_menu("navigation", 1.)
                .context_menu_alt("no.such.action"),
            cx,
        );
    });
    cx.simulate_keystrokes("shift-f10");
    assert_eq!(rows(&editor, cx)[0], "Primary");
}

/// An alternative can be a built-in action, shown with its label and key.
/// The menu is measured with the alternatives, so holding Alt doesn't
/// resize it.
#[gpui::test]
fn built_in_alternatives(cx: &mut TestAppContext) {
    use gpui::Modifiers;
    let (editor, cx) = setup("foo bar foo", cx);
    let short = || {
        EditorActionDescriptor::new("my.short", "Short", |_, _, _| {})
            .context_menu("navigation", 1.)
    };
    editor.update(cx, |e, cx| {
        let _ = e.add_action(short(), cx);
    });
    cx.simulate_keystrokes("shift-f10");
    let narrow = editor
        .read_with(cx, |e, cx| e.context_menu_width_for_tests(cx))
        .unwrap();
    cx.simulate_keystrokes("escape");
    cx.run_until_parked();
    editor.update(cx, |e, cx| {
        let _ = e.add_action(
            short().context_menu_alt("editor.action.selectHighlights"),
            cx,
        );
    });
    cx.simulate_keystrokes("shift-f10");
    let wide = editor
        .read_with(cx, |e, cx| e.context_menu_width_for_tests(cx))
        .unwrap();
    assert!(wide > narrow, "{wide:?} {narrow:?}");
    cx.simulate_modifiers_change(Modifiers::alt());
    let (label, keys) = shown_items(cx).remove(0);
    assert_eq!(label, "Select All Occurrences of Find Match");
    assert!(keys.is_some());
    assert_eq!(
        editor.read_with(cx, |e, cx| e.context_menu_width_for_tests(cx)),
        Some(wide)
    );
    cx.simulate_modifiers_change(Modifiers::none());
    cx.simulate_keystrokes("down");
    cx.simulate_modifiers_change(Modifiers::alt());
    cx.simulate_keystrokes("alt-enter");
    cx.run_until_parked();
    assert!(!editor.read_with(cx, |e, _| e.is_context_menu_visible()));
    assert_eq!(editor.read_with(cx, |e, _| e.selections().len()), 2);
}
