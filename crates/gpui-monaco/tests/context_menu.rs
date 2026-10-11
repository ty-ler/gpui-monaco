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
