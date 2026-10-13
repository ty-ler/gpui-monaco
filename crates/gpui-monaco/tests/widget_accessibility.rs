//! What the editor and its widgets tell screen readers (#413): Monaco's
//! roles, `aria-label`s and ARIA states on the find widget, the rename and
//! quick inputs, the context and code action menus, the references tree
//! the sticky scroll and the suggest list, and the editor's
//! `aria-roledescription`, `aria-autocomplete` and read-only state. GPUI's test platform builds no accessibility tree,
//! so the tests read the nodes the widgets rendered.

use std::ops::Range;

use gpui::accesskit::{AutoComplete, NodeId};
use gpui::{
    App, AppContext as _, Entity, Focusable, Role, Task, TestAppContext, VisualTestContext, px,
};
use gpui_monaco::actions::{StartFindReplace, ToggleCaseSensitive};
use gpui_monaco::cursor::Selection;
use gpui_monaco::options::EditorOptions;
use gpui_monaco::providers::*;
use gpui_monaco::text::PointUtf16;
use gpui_monaco::{AriaNode, AriaOptions, CodeEditor, TextModel};

fn p(row: u32, column: u32) -> PointUtf16 {
    PointUtf16::new(row, column)
}

fn setup<'a>(
    text: &str,
    cx: &'a mut TestAppContext,
) -> (Entity<CodeEditor>, &'a mut VisualTestContext) {
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

/// The accessibility nodes of a fresh frame.
fn nodes(cx: &mut VisualTestContext) -> Vec<AriaNode> {
    cx.run_until_parked();
    gpui_monaco::record_aria_for_tests();
    cx.update(|window, cx| {
        window.refresh();
        window.draw(cx).clear(cx);
    });
    gpui_monaco::recorded_aria_for_tests()
}

/// The node with `role` whose label starts with `label`.
fn find<'a>(nodes: &'a [AriaNode], role: Role, label: &str) -> &'a AriaNode {
    nodes
        .iter()
        .find(|n| n.role == role && n.label.as_deref().is_some_and(|l| l.starts_with(label)))
        .unwrap_or_else(|| panic!("no {role:?} {label:?} in {nodes:#?}"))
}

fn labels(nodes: &[AriaNode], role: Role) -> Vec<String> {
    nodes
        .iter()
        .filter(|n| n.role == role)
        .map(|n| n.label.as_deref().unwrap_or_default().to_string())
        .collect()
}

/// The labels of the nodes reported as the focused container's active
/// descendant (at most one per frame).
fn active(nodes: &[AriaNode]) -> Vec<String> {
    nodes
        .iter()
        .filter(|n| n.active_descendant)
        .map(|n| n.label.as_deref().unwrap_or_default().to_string())
        .collect()
}

/// Monaco `TextAreaEditContext`: `aria-roledescription` "editor", and
/// `aria-autocomplete` "both", "none" when read-only, "list" while
/// `setAriaOptions` names an active descendant.
#[gpui::test]
fn the_editor_is_an_editor_with_completions(cx: &mut TestAppContext) {
    cx.update(gpui_monaco::init);
    let (editor, cx) = setup("fn main() {}", cx);
    let root = find(&nodes(cx), Role::MultilineTextInput, "Editor content").clone();
    assert_eq!(root.role_description.as_deref(), Some("editor"));
    assert_eq!(root.auto_complete, Some(AutoComplete::Both));
    assert_eq!(editor.read_with(cx, |e, _| e.aria_for_tests()), root);

    editor.update(cx, |e, cx| {
        e.set_aria_options(
            AriaOptions {
                active_descendant: Some(NodeId(7)),
                ..Default::default()
            },
            cx,
        )
    });
    let aria = editor.read_with(cx, |e, _| e.aria_for_tests());
    assert_eq!(aria.auto_complete, Some(AutoComplete::List));

    editor.update(cx, |e, cx| {
        e.set_aria_options(AriaOptions::default(), cx);
        e.update_options_with(|o| o.read_only = true, cx);
    });
    let aria = editor.read_with(cx, |e, _| e.aria_for_tests());
    assert_eq!(aria.auto_complete, None);
    assert_eq!(aria.role_description.as_deref(), Some("editor"));
    // Monaco `_ensureReadOnlyAttribute`: a read-only editor is still an
    // editable text box to screen readers, unless `domReadOnly` is on too
    // (#492).
    assert!(!aria.read_only);
    editor.update(cx, |e, cx| {
        e.update_options_with(|o| o.dom_read_only = true, cx)
    });
    assert!(editor.read_with(cx, |e, _| e.aria_for_tests().read_only));
    editor.update(cx, |e, cx| {
        e.update_options_with(|o| o.read_only = false, cx)
    });
    assert!(!editor.read_with(cx, |e, _| e.aria_for_tests().read_only));
}

/// Monaco's find widget: a "Find / Replace" dialog, "Find" and "Replace"
/// inputs (plain inputs, no role description), checkbox toggles and
/// buttons named by their titles, and an expandable replace toggle.
#[gpui::test]
fn the_find_widget_names_its_inputs_and_controls(cx: &mut TestAppContext) {
    cx.update(gpui_monaco::init);
    let (_editor, cx) = setup("foo bar foo", cx);
    cx.simulate_keystrokes("secondary-f");
    let found = nodes(cx);
    find(&found, Role::Dialog, "Find / Replace");
    let input = find(&found, Role::TextInput, "Find");
    assert_eq!(input.label.as_deref(), Some("Find"));
    assert_eq!(input.role_description, None);
    assert_eq!(input.auto_complete, None);
    let match_case = find(&found, Role::CheckBox, "Match Case (");
    assert_eq!(match_case.checked, Some(false));
    find(&found, Role::CheckBox, "Match Whole Word (");
    find(&found, Role::CheckBox, "Use Regular Expression (");
    find(&found, Role::CheckBox, "Find in Selection (");
    find(&found, Role::Button, "Previous Match (");
    find(&found, Role::Button, "Next Match (");
    find(&found, Role::Button, "Close");
    let toggle = find(&found, Role::Button, "Toggle Replace");
    assert_eq!(toggle.expanded, Some(false));
    assert!(
        labels(&found, Role::TextInput)
            .iter()
            .all(|l| l != "Replace")
    );

    cx.dispatch_action(ToggleCaseSensitive);
    cx.dispatch_action(StartFindReplace);
    let found = nodes(cx);
    assert_eq!(
        find(&found, Role::CheckBox, "Match Case (").checked,
        Some(true)
    );
    assert_eq!(
        find(&found, Role::Button, "Toggle Replace").expanded,
        Some(true)
    );
    find(&found, Role::TextInput, "Replace");
    find(&found, Role::CheckBox, "Preserve Case (");
    find(&found, Role::Button, "Replace (");
    find(&found, Role::Button, "Replace All (");
}

struct Words;

impl RenameProvider for Words {
    fn provide_rename_edits(
        &self,
        _: &ProviderContext,
        _: PointUtf16,
        _: &str,
        _: &mut App,
    ) -> Task<anyhow::Result<Option<WorkspaceEdit>>> {
        Task::ready(Ok(None))
    }
}

/// Monaco `renameAriaLabel`.
#[gpui::test]
fn the_rename_input_says_what_it_is(cx: &mut TestAppContext) {
    cx.update(|cx| {
        gpui_monaco::init(cx);
        register_rename_provider(cx, LanguageSelector::All, Words);
    });
    let (editor, cx) = setup("fn foo() {}", cx);
    editor.update(cx, |e, cx| {
        e.set_selections(&[Selection::collapsed(p(0, 4))], cx)
    });
    cx.simulate_keystrokes("f2");
    draw(cx);
    assert!(editor.read_with(cx, |e, _| e.is_rename_visible()));
    find(
        &nodes(cx),
        Role::TextInput,
        "Rename input. Type new name and press Enter to commit.",
    );
}

/// Monaco's quick input: the input named by the provider's `ariaLabel`
/// (else "Type to narrow down results."), and a "Quick Input" listbox of
/// options named by their `ariaLabel`s, the active one selected.
#[gpui::test]
fn the_quick_input_names_its_input_and_options(cx: &mut TestAppContext) {
    cx.update(gpui_monaco::init);
    let (_editor, cx) = setup("one\ntwo\nthree", cx);
    cx.simulate_keystrokes("ctrl-g");
    let found = nodes(cx);
    find(
        &found,
        Role::TextInput,
        "Go to line. Type a line number, optionally followed by colon and column number.",
    );
    find(&found, Role::ListBox, "Quick Input");
    assert_eq!(
        labels(&found, Role::ListBoxOption),
        ["Type a line number to go to (from 1 to 3)."]
    );

    cx.simulate_input("2");
    let found = nodes(cx);
    let option = find(&found, Role::ListBoxOption, "Go to line 2");
    assert_eq!(
        option.label.as_deref(),
        Some("Go to line 2, column 1. Press Enter to navigate.")
    );
    assert_eq!(option.selected, Some(true));
    cx.simulate_input(":3");
    find(
        &nodes(cx),
        Role::ListBoxOption,
        "Go to line 2, column 3. Press Enter to navigate.",
    );

    // Help: the default label, and each option's label and description.
    cx.simulate_keystrokes("escape f1 backspace");
    let found = nodes(cx);
    find(&found, Role::TextInput, "Type to narrow down results.");
    find(&found, Role::ListBoxOption, ":, Go to Line/Column...");
}

/// Monaco's menu: `menuitem`s (with their keys), the focused one the
/// menu's active descendant, a submenu row expanded while it's open.
#[gpui::test]
fn the_context_menu_is_a_menu(cx: &mut TestAppContext) {
    cx.update(gpui_monaco::init);
    let (_editor, cx) = setup("foo bar foo", cx);
    cx.simulate_keystrokes("shift-f10");
    let found = nodes(cx);
    assert_eq!(
        found.iter().filter(|n| n.role == Role::Menu).count(),
        1,
        "{found:#?}"
    );
    assert_eq!(
        labels(&found, Role::MenuItem),
        [
            "Change All Occurrences",
            "Cut",
            "Copy",
            "Paste",
            "Command Palette"
        ]
    );
    assert!(find(&found, Role::MenuItem, "Copy").keyshortcuts.is_some());
    assert!(active(&found).is_empty());
    cx.simulate_keystrokes("down down");
    assert_eq!(active(&nodes(cx)), ["Cut"]);
}

#[derive(Clone)]
struct Actions(Vec<CodeAction>);

impl CodeActionProvider for Actions {
    fn provide_code_actions(
        &self,
        _: &ProviderContext,
        _: Range<PointUtf16>,
        _: &CodeActionContext,
        _: &mut App,
    ) -> Task<anyhow::Result<Vec<CodeAction>>> {
        Task::ready(Ok(self.0.clone()))
    }
}

fn action(title: &str, disabled: Option<&str>) -> CodeAction {
    CodeAction {
        title: title.to_string().into(),
        kind: Some("refactor.extract".into()),
        disabled: disabled.map(Into::into),
        ..Default::default()
    }
}

fn code_action_menu(actions: Vec<CodeAction>, cx: &mut TestAppContext) -> &mut VisualTestContext {
    cx.update(|cx| {
        gpui_monaco::init(cx);
        register_code_action_provider(cx, LanguageSelector::All, Actions(actions));
    });
    let (editor, cx) = setup("let x = 1;", cx);
    editor.update(cx, |e, cx| e.refactor(cx));
    draw(cx);
    assert!(editor.read_with(cx, |e, _| e.is_code_action_menu_visible()));
    cx
}

/// Monaco `ActionList`: an "Action Widget" listbox, the focused option its
/// active descendant.
#[gpui::test]
fn the_code_action_menu_is_a_listbox(cx: &mut TestAppContext) {
    let cx = code_action_menu(
        vec![
            action("Extract constant", None),
            action("Extract method", None),
        ],
        cx,
    );
    let found = nodes(cx);
    find(&found, Role::ListBox, "Action Widget");
    assert_eq!(
        labels(&found, Role::ListBoxOption),
        ["Extract constant", "Extract method"]
    );
    assert_eq!(active(&found), ["Extract constant"]);
    cx.simulate_keystrokes("down");
    assert_eq!(active(&nodes(cx)), ["Extract method"]);
}

/// Monaco `customQuickFixWidget.labels`: a disabled option says why.
#[gpui::test]
fn a_disabled_code_action_says_why(cx: &mut TestAppContext) {
    let cx = code_action_menu(
        vec![action("Extract method", Some("Select a statement"))],
        cx,
    );
    let found = nodes(cx);
    assert_eq!(
        labels(&found, Role::ListBoxOption),
        ["Extract method, Disabled Reason: Select a statement"]
    );
    assert!(active(&found).is_empty());
}

/// Monaco's references tree: "References", each row its `ariaMessage`,
/// the focused row its active descendant.
#[gpui::test]
fn the_references_tree_reads_each_result(cx: &mut TestAppContext) {
    cx.update(|cx| {
        gpui_monaco::init(cx);
        gpui_monaco::set_model_resolver(cx, |uri, cx| {
            (uri == "file:///src/lib.rs").then(|| {
                cx.new(|_| TextModel::new("pub fn alpha() {}\npub fn beta() {}").with_uri(uri))
            })
        });
    });
    let (editor, cx) = setup("fn main() {\n    alpha();\n}", cx);
    let elsewhere = |row: u32, start: u32, end: u32| Location {
        uri: Some("file:///src/lib.rs".into()),
        range: p(row, start)..p(row, end),
        ..Default::default()
    };
    let here = Location {
        uri: None,
        range: p(1, 4)..p(1, 9),
        ..Default::default()
    };
    editor.update(cx, |e, cx| {
        e.peek_locations(
            p(1, 4),
            vec![here, elsewhere(0, 7, 12), elsewhere(1, 7, 11)],
            None,
            cx,
        )
    });
    draw(cx);
    draw(cx);
    let found = nodes(cx);
    find(&found, Role::Tree, "References");
    let file = find(&found, Role::TreeItem, "2 symbols in lib.rs");
    assert_eq!(
        file.label.as_deref(),
        Some("2 symbols in lib.rs, full path /src/lib.rs")
    );
    assert_eq!((file.level, file.expanded), (Some(1), Some(true)));
    let reference = find(&found, Role::TreeItem, "pub fn alpha() {} in lib.rs");
    assert_eq!(
        reference.label.as_deref(),
        Some("pub fn alpha() {} in lib.rs on line 1 at column 8")
    );
    assert_eq!(reference.level, Some(2));
    find(
        &found,
        Role::TreeItem,
        "1 symbol in main.rs, full path /src/main.rs",
    );
    find(
        &found,
        Role::TreeItem,
        "alpha(); in main.rs on line 2 at column 5",
    );
    assert_eq!(active(&found).len(), 1);
    // Up moves the active descendant.
    let before = active(&found);
    cx.simulate_keystrokes("up");
    let after = active(&nodes(cx));
    assert_eq!(after.len(), 1);
    assert_ne!(after, before);
}

/// Monaco's sticky scroll: a `list` of `listitem`s, one per line.
#[gpui::test]
fn sticky_lines_are_a_list(cx: &mut TestAppContext) {
    cx.update(gpui_monaco::init);
    let mut text = String::from("fn main() {\n    if true {\n");
    for i in 0..200 {
        text.push_str(&format!("        let v{i} = {i};\n"));
    }
    text.push_str("    }\n}\n");
    let (editor, cx) = setup(&text, cx);
    editor.update(cx, |e, cx| {
        e.update_options(
            EditorOptions {
                minimap: Default::default(),
                ..e.options().clone()
            },
            cx,
        );
        e.set_scroll_position(gpui::point(px(0.), px(1000.)), cx)
    });
    for _ in 0..3 {
        draw(cx);
        cx.executor()
            .advance_clock(std::time::Duration::from_millis(500));
    }
    let rows = editor.read_with(cx, |e, _| e.sticky_rows_for_tests().0);
    assert_eq!(rows, [0, 1]);
    let found = nodes(cx);
    assert_eq!(
        found.iter().filter(|n| n.role == Role::List).count(),
        1,
        "{found:#?}"
    );
    assert_eq!(
        labels(&found, Role::ListItem),
        ["fn main() {", "    if true {"]
    );
}

/// Items whose docs arrive from a 300ms resolve.
struct DocsOnResolve;

impl CompletionProvider for DocsOnResolve {
    fn provide_completions(
        &self,
        _: &ProviderContext,
        _: PointUtf16,
        _: CompletionTrigger,
        _: &mut App,
    ) -> Task<anyhow::Result<CompletionList>> {
        Task::ready(Ok(CompletionList {
            items: ["alpha", "beta"]
                .into_iter()
                .map(|label| CompletionItem::new(label, CompletionItemKind::Function))
                .collect(),
            incomplete: false,
        }))
    }

    fn resolve_completion_item(
        &self,
        item: &CompletionItem,
        cx: &mut App,
    ) -> Option<Task<anyhow::Result<CompletionItem>>> {
        let item = item.clone();
        let timer = cx
            .background_executor()
            .timer(std::time::Duration::from_millis(300));
        Some(cx.background_executor().spawn(async move {
            timer.await;
            Ok(CompletionItem {
                documentation: Some(format!("Docs for {}.", item.label).into()),
                ..item
            })
        }))
    }
}

/// Monaco's suggest list: a "Suggest" listbox of options named by
/// `getAriaLabel`; the focused one is the editor's active descendant once
/// it resolves (making the editor's `aria-autocomplete` "list"), not an
/// alert (#492).
#[gpui::test]
fn the_suggest_list_is_a_listbox(cx: &mut TestAppContext) {
    cx.update(|cx| {
        gpui_monaco::init(cx);
        register_completion_provider(cx, LanguageSelector::All, DocsOnResolve);
    });
    let (editor, cx) = setup("", cx);
    let auto_complete =
        |cx: &mut VisualTestContext| editor.read_with(cx, |e, _| e.aria_for_tests().auto_complete);
    cx.simulate_keystrokes("ctrl-space");
    let found = nodes(cx);
    find(&found, Role::ListBox, "Suggest");
    let option_role = if cfg!(windows) {
        Role::ListItem
    } else {
        Role::ListBoxOption
    };
    assert_eq!(
        labels(&found, option_role),
        ["alpha, Function", "beta, Function"]
    );
    assert_eq!(find(&found, option_role, "alpha").selected, Some(true));
    assert_eq!(find(&found, option_role, "beta").selected, Some(false));
    // Not until it resolves (Monaco `_onListFocus`).
    assert!(active(&found).is_empty());
    assert_eq!(auto_complete(cx), Some(AutoComplete::Both));

    cx.executor()
        .advance_clock(std::time::Duration::from_millis(300));
    assert_eq!(active(&nodes(cx)), ["alpha, Function"]);
    assert_eq!(auto_complete(cx), Some(AutoComplete::List));
    // The focused row isn't alerted.
    assert_eq!(
        editor.read_with(cx, |e, _| e.last_announcement_for_tests()),
        None
    );

    // With the details panel, a resolved row reads its docs.
    cx.simulate_keystrokes("ctrl-space");
    assert_eq!(
        active(&nodes(cx)),
        ["alpha, Function, docs: Docs for alpha."]
    );
    cx.simulate_keystrokes("down");
    assert!(active(&nodes(cx)).is_empty());
    cx.executor()
        .advance_clock(std::time::Duration::from_millis(300));
    assert_eq!(active(&nodes(cx)), ["beta, Function, docs: Docs for beta."]);

    cx.simulate_keystrokes("escape");
    draw(cx);
    assert_eq!(auto_complete(cx), Some(AutoComplete::Both));
    assert!(labels(&nodes(cx), Role::ListBox).is_empty());
}
