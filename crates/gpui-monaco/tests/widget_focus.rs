//! Keys in the editor's focused widgets (#400): Monaco's text keybindings
//! need `editorTextFocus`, so editing keys pressed in a focused hover, find
//! widget button, menu, peek list, marker zone or color picker don't
//! reach the document; Tab moves among a widget's own controls; and the
//! hover's copy buttons and status bar actions are tab stops that Enter and
//! Space press.

use std::ops::Range;
use std::time::Duration;

use gpui::{AppContext as _, Entity, Focusable as _, TestAppContext, VisualTestContext};
use gpui_monaco::actions::*;
use gpui_monaco::cursor::Selection;
use gpui_monaco::providers::Location;
use gpui_monaco::text::PointUtf16;
use gpui_monaco::{CodeEditor, DiffEditor, Marker, MarkerSeverity, TextModel};

/// The keys that edited the document from a focused widget, and typing.
const EDITING_KEYS: &str = "enter backspace delete secondary-v secondary-x a";

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
        let model = cx.new(|_| TextModel::new(&text).with_uri("file:///src/main.rs"));
        CodeEditor::new(model, window, cx)
    });
    cx.update(|window, _| window.activate_window());
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

fn text(editor: &Entity<CodeEditor>, cx: &mut VisualTestContext) -> String {
    editor.read_with(cx, |e, cx| e.model().read(cx).text())
}

/// Whether one of the editor's widgets has the focus, rather than its text.
fn widget_focused(editor: &Entity<CodeEditor>, cx: &mut VisualTestContext) -> bool {
    cx.update(|window, cx| {
        let focus = editor.focus_handle(cx);
        !focus.is_focused(window) && focus.contains_focused(window, cx)
    })
}

fn editor_focused(editor: &Entity<CodeEditor>, cx: &mut VisualTestContext) -> bool {
    cx.update(|window, cx| editor.focus_handle(cx).is_focused(window))
}

fn announced(editor: &Entity<CodeEditor>, cx: &mut VisualTestContext) -> String {
    editor
        .read_with(cx, |e, _| e.last_announcement_for_tests())
        .map(|a| a.to_string())
        .unwrap_or_default()
}

fn marker(range: Range<PointUtf16>) -> Marker {
    Marker {
        severity: MarkerSeverity::Error,
        message: "Error here".into(),
        range,
        ..Default::default()
    }
}

fn set_markers(editor: &Entity<CodeEditor>, cx: &mut VisualTestContext, markers: Vec<Marker>) {
    editor.update(cx, |e, cx| {
        e.model()
            .update(cx, |m, cx| m.set_markers("test", markers, cx))
    });
    draw(cx);
}

/// Shows the hover at `at` from the keyboard, then focuses it (Monaco's
/// second Show or Focus Hover).
fn focus_hover(editor: &Entity<CodeEditor>, cx: &mut VisualTestContext, at: PointUtf16) {
    editor.update(cx, |e, cx| {
        e.set_selections(&[Selection::collapsed(at)], cx);
        e.show_hover_at_cursor(cx);
    });
    draw(cx);
    cx.dispatch_action(ShowHover::default());
    draw(cx);
    assert!(cx.update(|window, cx| editor.read(cx).is_hover_focused(window, cx)));
}

fn press(keys: &str, cx: &mut VisualTestContext) {
    for key in keys.split(' ') {
        cx.simulate_keystrokes(key);
        draw(cx);
    }
}

/// §C.1: Enter in a focused hover split the line (`Newline` matched in the
/// editor's key context, which the hover draws in); Backspace, Delete,
/// Cmd/Ctrl+V and Cmd/Ctrl+X edited too.
#[gpui::test]
fn editing_keys_in_a_focused_hover_leave_the_text(cx: &mut TestAppContext) {
    let (editor, cx) = setup("let e = Edit::new();\n", cx);
    cx.write_to_clipboard(gpui::ClipboardItem::new_string("pasted".into()));
    set_markers(&editor, cx, vec![marker(p(0, 8)..p(0, 12))]);
    focus_hover(&editor, cx, p(0, 10));
    for key in EDITING_KEYS.split(' ') {
        cx.simulate_keystrokes(key);
        draw(cx);
        assert_eq!(text(&editor, cx), "let e = Edit::new();\n", "after {key}");
        assert!(
            editor.read_with(cx, |e, _| e.is_hover_visible()),
            "after {key}"
        );
    }
    assert_eq!(
        editor.read_with(cx, |e, _| e.selections()[0].head),
        p(0, 10)
    );
    // The text's own keys from a focused row too.
    assert!(cx.update(|window, cx| editor.update(cx, |e, cx| e.focus_hover_part(0, window, cx))));
    draw(cx);
    press("backspace enter", cx);
    assert_eq!(text(&editor, cx), "let e = Edit::new();\n");
}

/// Monaco's `undo` has no `kbExpr`: from a widget it focuses the editor
/// and undoes there (`EditorOrNativeTextInputCommand`).
#[gpui::test]
fn undo_from_a_focused_hover_focuses_the_editor(cx: &mut TestAppContext) {
    let (editor, cx) = setup("let e = Edit::new();\n", cx);
    editor.update(cx, |e, cx| {
        e.set_selections(&[Selection::collapsed(p(0, 0))], cx)
    });
    cx.simulate_input("x");
    draw(cx);
    assert_eq!(text(&editor, cx), "xlet e = Edit::new();\n");
    set_markers(&editor, cx, vec![marker(p(0, 9)..p(0, 13))]);
    focus_hover(&editor, cx, p(0, 10));
    press("secondary-z", cx);
    assert_eq!(text(&editor, cx), "let e = Edit::new();\n");
    assert!(editor_focused(&editor, cx));
}

/// §C.2: Monaco's copy button (a `SimpleButton`) and status bar actions
/// (`HoverAction`) are tab stops after their row, and Enter or Space
/// presses them.
#[gpui::test]
fn hover_buttons_take_the_focus_and_enter_or_space_press_them(cx: &mut TestAppContext) {
    let (editor, cx) = setup("let x = 1;\n", cx);
    set_markers(&editor, cx, vec![marker(p(0, 4)..p(0, 5))]);
    focus_hover(&editor, cx, p(0, 4));
    let focused = |cx: &mut VisualTestContext| {
        cx.update(|window, cx| editor.read(cx).focused_hover_part_index(window, cx))
    };
    // The marker row, then its copy button (the focus stays in the row).
    press("tab", cx);
    assert_eq!(focused(cx), Some(0));
    assert_eq!(announced(&editor, cx), "Error here");
    press("tab", cx);
    assert_eq!(focused(cx), Some(0));
    assert_eq!(announced(&editor, cx), "Copy");
    press("enter", cx);
    assert_eq!(announced(&editor, cx), "Copied to clipboard");
    assert!(
        cx.read_from_clipboard()
            .and_then(|item| item.text())
            .is_some_and(|text| text.contains("Error here"))
    );
    assert_eq!(text(&editor, cx), "let x = 1;\n");
    // Shift+Tab goes back to the row, Tab again to the button.
    press("shift-tab", cx);
    assert_eq!(announced(&editor, cx), "Error here");
    press("tab", cx);
    assert_eq!(announced(&editor, cx), "Copy");
    // The status bar, then its "View Problem" action, which Space runs:
    // the hover hides and the marker zone opens.
    press("tab", cx);
    assert_eq!(focused(cx), Some(1));
    press("tab", cx);
    assert_eq!(focused(cx), Some(1));
    assert!(
        announced(&editor, cx).starts_with("View Problem"),
        "{}",
        announced(&editor, cx)
    );
    press("space", cx);
    assert!(!editor.read_with(cx, |e, _| e.is_hover_visible()));
    assert!(
        editor
            .read_with(cx, |e, cx| e.marker_zone_state_for_tests(cx))
            .is_some()
    );
    assert_eq!(text(&editor, cx), "let x = 1;\n");
}

/// The status bar's keybinding labels don't depend on the focus (Monaco
/// `lookupKeybinding` without a context), though the Quick Fix key needs
/// the text focus.
#[gpui::test]
fn hover_status_bar_keys_show_while_the_hover_has_the_focus(cx: &mut TestAppContext) {
    let (editor, cx) = setup("let x = 1;\n", cx);
    set_markers(&editor, cx, vec![marker(p(0, 4)..p(0, 5))]);
    focus_hover(&editor, cx, p(0, 4));
    let status = editor.read_with(cx, |e, cx| e.hover_status_bar_for_tests(cx));
    assert!(status[0].starts_with("View Problem ("), "{status:?}");
}

/// §C.3: Tab on a focused find widget control moves to the next control in
/// read-only editors and tab focus mode too, where the editor's Tab moves
/// the focus out (`tabMovesFocus` needs `editorTextFocus` in Monaco).
#[gpui::test]
fn tab_cycles_the_find_widget_in_read_only_editors_and_tab_focus_mode(cx: &mut TestAppContext) {
    let (editor, cx) = setup("foo foo", cx);
    let focused_control = |cx: &mut VisualTestContext| {
        cx.update(|window, cx| {
            editor
                .read(cx)
                .find_widget_focused_control_for_tests(window, cx)
        })
    };
    editor.update(cx, |e, cx| {
        e.update_options_with(|o| o.read_only = true, cx)
    });
    cx.simulate_keystrokes("secondary-f");
    draw(cx);
    press("tab", cx);
    assert_eq!(focused_control(cx), Some("match-case"));
    press("tab", cx);
    assert_eq!(focused_control(cx), Some("whole-word"));
    press("shift-tab", cx);
    assert_eq!(focused_control(cx), Some("match-case"));

    // Tab focus mode, writable.
    editor.update(cx, |e, cx| {
        e.update_options_with(|o| o.read_only = false, cx)
    });
    cx.dispatch_action(ToggleTabFocusMode);
    draw(cx);
    press("tab", cx);
    assert_eq!(focused_control(cx), Some("whole-word"));
    press("tab", cx);
    assert_eq!(focused_control(cx), Some("regex"));
    // Editing keys on a focused control leave the text.
    press("backspace delete secondary-x secondary-v", cx);
    assert_eq!(text(&editor, cx), "foo foo");
    assert_eq!(focused_control(cx), Some("regex"));
    cx.dispatch_action(ToggleTabFocusMode);
}

/// The context menu takes the focus; its keys are its own.
#[gpui::test]
fn editing_keys_in_the_context_menu_leave_the_text(cx: &mut TestAppContext) {
    let (editor, cx) = setup("fn main() {}\n", cx);
    cx.write_to_clipboard(gpui::ClipboardItem::new_string("pasted".into()));
    cx.simulate_keystrokes("shift-f10");
    draw(cx);
    assert!(editor.read_with(cx, |e, _| e.is_context_menu_visible()));
    assert!(widget_focused(&editor, cx));
    press("backspace delete secondary-x secondary-v a tab", cx);
    assert_eq!(text(&editor, cx), "fn main() {}\n");
}

/// The peek view's results list.
#[gpui::test]
fn editing_keys_in_the_peek_list_leave_the_text(cx: &mut TestAppContext) {
    let (editor, cx) = setup("fn main() {\n    main();\n}\n", cx);
    cx.write_to_clipboard(gpui::ClipboardItem::new_string("pasted".into()));
    let location = Location {
        uri: None,
        range: p(1, 4)..p(1, 8),
        ..Default::default()
    };
    editor.update(cx, |e, cx| {
        e.peek_locations(p(0, 3), vec![location.clone(), location], None, cx)
    });
    draw(cx);
    draw(cx);
    assert!(editor.read_with(cx, |e, _| e.is_peek_visible()));
    assert!(widget_focused(&editor, cx));
    press("backspace delete secondary-x secondary-v a", cx);
    assert_eq!(text(&editor, cx), "fn main() {\n    main();\n}\n");
}

/// The marker zone's body takes the focus when clicked; Escape still
/// closes it (Monaco `closeMarkersNavigation` needs `editorFocus` only).
#[gpui::test]
fn editing_keys_in_the_marker_zone_leave_the_text(cx: &mut TestAppContext) {
    let (editor, cx) = setup("let x = 1;\nlet y = 2;\n", cx);
    cx.write_to_clipboard(gpui::ClipboardItem::new_string("pasted".into()));
    set_markers(&editor, cx, vec![marker(p(0, 4)..p(0, 5))]);
    editor.update(cx, |e, cx| e.go_to_marker(true, cx));
    draw(cx);
    let zone = editor
        .read_with(cx, |e, cx| e.marker_zone_state_for_tests(cx))
        .expect("a marker zone");
    cx.update(|window, cx| window.focus(&zone.focus_handle, cx));
    draw(cx);
    assert!(widget_focused(&editor, cx));
    press(EDITING_KEYS, cx);
    assert_eq!(text(&editor, cx), "let x = 1;\nlet y = 2;\n");
    press("escape", cx);
    assert!(
        editor
            .read_with(cx, |e, cx| e.marker_zone_state_for_tests(cx))
            .is_none()
    );
}

/// The standalone color picker, focused by its second Show or Focus.
#[gpui::test]
fn editing_keys_in_the_color_picker_leave_the_text(cx: &mut TestAppContext) {
    cx.update(gpui_monaco::init);
    let (editor, cx) = cx.add_window_view(|window, cx| {
        let model = cx.new(|_| TextModel::new("a { color: ; }").with_language("css"));
        CodeEditor::new(model, window, cx)
    });
    cx.update(|window, cx| window.focus(&editor.focus_handle(cx), cx));
    draw(cx);
    cx.write_to_clipboard(gpui::ClipboardItem::new_string("pasted".into()));
    editor.update(cx, |e, cx| {
        e.set_selections(&[Selection::collapsed(p(0, 11))], cx)
    });
    cx.dispatch_action(ShowOrFocusStandaloneColorPicker);
    draw(cx);
    cx.dispatch_action(ShowOrFocusStandaloneColorPicker);
    draw(cx);
    assert!(widget_focused(&editor, cx));
    press("backspace delete secondary-x secondary-v a tab", cx);
    assert_eq!(text(&editor, cx), "a { color: ; }");
    assert!(editor.read_with(cx, |e, _| e.is_standalone_color_picker_visible()));
}

/// The quick input's box is an editor of its own: keys edit it, not the
/// document.
#[gpui::test]
fn keys_in_the_quick_input_edit_its_box(cx: &mut TestAppContext) {
    let (editor, cx) = setup("fn main() {}\n", cx);
    cx.simulate_keystrokes("f1");
    draw(cx);
    cx.simulate_input("xy");
    press("backspace tab", cx);
    let input = editor
        .read_with(cx, |e, cx| e.quick_access_input_for_tests(cx))
        .expect("the quick input");
    assert_eq!(input.read_with(cx, |i, cx| i.model().read(cx).text()), ">x");
    assert_eq!(text(&editor, cx), "fn main() {}\n");
}

/// The accessible diff viewer sits over the diff editor's editors.
#[gpui::test]
fn editing_keys_in_the_accessible_diff_viewer_leave_the_text(cx: &mut TestAppContext) {
    cx.update(gpui_monaco::init);
    let (diff, cx) = cx.add_window_view(|window, cx| {
        let original = cx.new(|_| TextModel::new("a\nb\nc"));
        let modified = cx.new(|_| TextModel::new("a\nx\nc"));
        DiffEditor::new(original, modified, window, cx)
    });
    draw(cx);
    cx.executor().advance_clock(Duration::from_millis(100));
    draw(cx);
    let editor = diff.read_with(cx, |d, _| d.modified_editor().clone());
    cx.update(|window, cx| window.focus(&editor.read(cx).focus_handle(cx), cx));
    cx.simulate_keystrokes("f7");
    draw(cx);
    assert!(diff.read_with(cx, |d, _| d.is_accessible_diff_viewer_visible()));
    press("backspace delete secondary-x tab a", cx);
    assert_eq!(text(&editor, cx), "a\nx\nc");
}

// ----- #472: host actions and keybinding rules from a widget's input -----

/// What the find input holds (the find widget's search string).
fn find_query(editor: &Entity<CodeEditor>, cx: &mut VisualTestContext) -> String {
    cx.run_until_parked();
    editor.read_with(cx, |e, _| e.find_options().query.clone())
}

/// Opens the find widget with the document's `abc` selected, and types
/// `abc` into its input.
fn find_abc(editor: &Entity<CodeEditor>, cx: &mut VisualTestContext) {
    editor.update(cx, |e, cx| {
        e.set_selections(&[Selection::new(p(0, 0), p(0, 3))], cx)
    });
    press("secondary-f", cx);
    cx.simulate_keystrokes("secondary-a");
    cx.simulate_input("abc");
    draw(cx);
    assert_eq!(find_query(editor, cx), "abc");
    assert!(widget_focused(editor, cx));
}

/// A host action that replaces the text of the editor it runs on.
fn ran_action(id: &'static str) -> gpui_monaco::EditorActionDescriptor {
    gpui_monaco::EditorActionDescriptor::new(id, "Ran", |editor, _, cx| {
        editor.model().update(cx, |m, cx| m.set_value("ran", cx))
    })
}

/// §H.1: Monaco's keybindings run on the editor with widget focus
/// (`getFocusedCodeEditor`), as its find input is a plain `<input>`: an
/// `addEditorAction` key pressed in the find input acts on the editor, and
/// the find input keeps its text and the focus.
#[gpui::test]
fn host_actions_from_the_find_input_run_on_the_editor(cx: &mut TestAppContext) {
    let (editor, cx) = setup("abc def\n", cx);
    let _action = cx.update(|_, cx| {
        gpui_monaco::add_editor_action(cx, ran_action("test.ran").keybinding("ctrl-alt-u"))
    });
    find_abc(&editor, cx);
    press("ctrl-alt-u", cx);
    assert_eq!(text(&editor, cx), "ran");
    assert_eq!(find_query(&editor, cx), "abc");
    assert!(widget_focused(&editor, cx));
}

/// §H.1: a keybinding rule's command runs on the editor too, and the
/// focus stays in the find input.
#[gpui::test]
fn keybinding_rules_from_the_find_input_run_on_the_editor(cx: &mut TestAppContext) {
    let (editor, cx) = setup("abc def\n", cx);
    let _rule = cx.update(|_, cx| {
        gpui_monaco::add_keybinding_rule(
            cx,
            gpui_monaco::KeybindingRule::new("ctrl-shift-u", "editor.action.transformToUppercase"),
        )
        .unwrap()
    });
    find_abc(&editor, cx);
    press("ctrl-shift-u", cx);
    assert_eq!(text(&editor, cx), "ABC def\n");
    assert_eq!(find_query(&editor, cx), "abc");
    assert!(widget_focused(&editor, cx));
}

/// §H.1: the find input has Monaco's `editorFocus` but not
/// `editorTextFocus`, so a rule for the text doesn't apply in it.
#[gpui::test]
fn editor_text_focus_rules_skip_the_find_input(cx: &mut TestAppContext) {
    let (editor, cx) = setup("abc def\n", cx);
    let _rules = cx.update(|_, cx| {
        gpui_monaco::add_keybinding_rules(
            cx,
            vec![
                gpui_monaco::KeybindingRule::new(
                    "ctrl-shift-u",
                    "editor.action.transformToUppercase",
                )
                .when("CodeEditor && editorTextFocus"),
                gpui_monaco::KeybindingRule::new(
                    "ctrl-shift-y",
                    "editor.action.transformToUppercase",
                )
                .when("CodeEditor && editorFocus"),
            ],
        )
        .unwrap()
    });
    find_abc(&editor, cx);
    press("ctrl-shift-u", cx);
    assert_eq!(text(&editor, cx), "abc def\n");
    assert_eq!(find_query(&editor, cx), "abc");
    press("ctrl-shift-y", cx);
    assert_eq!(text(&editor, cx), "ABC def\n");
    assert_eq!(find_query(&editor, cx), "abc");
    // From the text, the `editorTextFocus` rule applies.
    editor.update(cx, |e, cx| {
        e.set_selections(&[Selection::new(p(0, 4), p(0, 7))], cx)
    });
    cx.update(|window, cx| window.focus(&editor.focus_handle(cx), cx));
    draw(cx);
    press("ctrl-shift-u", cx);
    assert_eq!(text(&editor, cx), "ABC DEF\n");
}

/// §H.1: the quick input's box hands host actions to the editor too.
#[gpui::test]
fn host_actions_from_the_quick_input_run_on_the_editor(cx: &mut TestAppContext) {
    let (editor, cx) = setup("fn main() {}\n", cx);
    let _action = cx.update(|_, cx| {
        gpui_monaco::add_editor_action(cx, ran_action("test.ran").keybinding("ctrl-alt-u"))
    });
    press("f1", cx);
    let input = editor
        .read_with(cx, |e, cx| e.quick_access_input_for_tests(cx))
        .expect("the quick input");
    press("ctrl-alt-u", cx);
    assert_eq!(text(&editor, cx), "ran");
    assert_eq!(input.read_with(cx, |i, cx| i.model().read(cx).text()), ">");
}

/// §H.1: the text's keys still edit a widget's input (`textInputFocus`),
/// while its editor has `editorFocus` without `editorTextFocus`.
#[gpui::test]
fn widget_inputs_keep_their_text_keys(cx: &mut TestAppContext) {
    let (editor, cx) = setup("abc def\n", cx);
    find_abc(&editor, cx);
    press("backspace", cx);
    assert_eq!(find_query(&editor, cx), "ab");
    press("left delete", cx);
    assert_eq!(find_query(&editor, cx), "a");
    assert_eq!(text(&editor, cx), "abc def\n");
    let keys = editor.read_with(cx, |e, cx| e.context_keys(cx));
    assert!(keys.contains("editorFocus"));
    assert!(!keys.contains("editorTextFocus"));
}
