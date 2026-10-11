//! Code actions (Monaco `contrib/codeAction`) with a test provider.

use std::cell::RefCell;
use std::ops::Range;
use std::rc::Rc;
use std::time::Duration;

use gpui::{App, AppContext as _, Entity, Focusable, Task, TestAppContext, VisualTestContext};
use gpui_monaco::cursor::Selection;
use gpui_monaco::providers::*;
use gpui_monaco::text::PointUtf16;
use gpui_monaco::{CodeEditor, Marker, MarkerSeverity, TextModel};

fn p(row: u32, column: u32) -> PointUtf16 {
    PointUtf16::new(row, column)
}

/// Offers fixed actions and records the contexts it was asked with.
#[derive(Clone, Default)]
struct Actions {
    actions: Vec<CodeAction>,
    contexts: Rc<RefCell<Vec<CodeActionContext>>>,
    /// Edits filled in by `resolve_code_action`.
    resolved: Option<WorkspaceEdit>,
}

impl CodeActionProvider for Actions {
    fn provide_code_actions(
        &self,
        _: &ProviderContext,
        _: Range<PointUtf16>,
        context: &CodeActionContext,
        _: &mut App,
    ) -> Task<anyhow::Result<Vec<CodeAction>>> {
        self.contexts.borrow_mut().push(context.clone());
        Task::ready(Ok(self.actions.clone()))
    }

    fn resolve_code_action(
        &self,
        action: &CodeAction,
        _: &mut App,
    ) -> Option<Task<anyhow::Result<CodeAction>>> {
        let edit = self.resolved.clone()?;
        let mut action = action.clone();
        action.edit = Some(edit);
        Some(Task::ready(Ok(action)))
    }
}

fn replace(range: Range<PointUtf16>, text: &str) -> WorkspaceEdit {
    WorkspaceEdit {
        edits: vec![(None, vec![(range, text.to_string())])],
        ..Default::default()
    }
}

fn action(title: &str, kind: &str) -> CodeAction {
    CodeAction {
        title: title.to_string().into(),
        kind: (!kind.is_empty()).then(|| kind.into()),
        ..Default::default()
    }
}

const TEXT: &str = "let x = 1;\nlet y = 2;\n";

fn setup(
    provider: Actions,
    cx: &mut TestAppContext,
) -> (Entity<CodeEditor>, &mut VisualTestContext) {
    cx.update(|cx| {
        gpui_monaco::init(cx);
        register_code_action_provider(cx, LanguageSelector::All, provider);
    });
    let (editor, cx) = cx.add_window_view(|window, cx| {
        let model = cx.new(|_| TextModel::new(TEXT));
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

fn text(editor: &Entity<CodeEditor>, cx: &mut VisualTestContext) -> String {
    editor.read_with(cx, |e, cx| e.model().read(cx).text())
}

fn wait(cx: &mut VisualTestContext) {
    cx.executor().advance_clock(Duration::from_millis(300));
    draw(cx);
}

#[gpui::test]
fn the_lightbulb_follows_the_cursor(cx: &mut TestAppContext) {
    let provider = Actions {
        actions: vec![action("Inline variable", "refactor.inline")],
        ..Default::default()
    };
    let contexts = provider.contexts.clone();
    let (editor, cx) = setup(provider, cx);
    editor.update(cx, |e, cx| {
        e.set_selections(&[Selection::collapsed(p(0, 4))], cx)
    });
    // Not until the cursor rests.
    assert!(!editor.read_with(cx, |e, _| e.is_lightbulb_visible()));
    wait(cx);
    assert!(editor.read_with(cx, |e, _| e.is_lightbulb_visible()));
    let context = contexts.borrow().last().cloned().unwrap();
    assert_eq!(
        (context.trigger, context.only),
        (CodeActionTrigger::Auto, None)
    );
    // Between spaces (Monaco's `onCode`): no request, no bulb.
    let before = contexts.borrow().len();
    editor.update(cx, |e, cx| {
        e.set_selections(&[Selection::collapsed(p(2, 0))], cx)
    });
    wait(cx);
    assert!(!editor.read_with(cx, |e, _| e.is_lightbulb_visible()));
    assert_eq!(contexts.borrow().len(), before);
}

#[gpui::test]
fn quick_fix_menu_applies_edits_and_commands(cx: &mut TestAppContext) {
    let ran = Rc::new(RefCell::new(Vec::new()));
    let log = ran.clone();
    let provider = Actions {
        actions: vec![
            CodeAction {
                edit: Some(replace(p(0, 4)..p(0, 5), "renamed")),
                command: Some(Command {
                    id: "test.after".into(),
                    title: "After".into(),
                    arguments: vec![serde_json::json!(7)],
                    ..Default::default()
                }),
                ..action("Extract to function", "refactor.extract")
            },
            CodeAction {
                diagnostics: vec![Marker {
                    severity: MarkerSeverity::Warning,
                    message: "unused".into(),
                    range: p(0, 4)..p(0, 5),
                    source: None,
                    code: None,
                    ..Default::default()
                }],
                edit: Some(replace(p(0, 4)..p(0, 4), "_")),
                ..action("Prefix with underscore", "quickfix")
            },
            CodeAction {
                disabled: Some("Not here".into()),
                ..action("Disabled", "quickfix")
            },
            action("Organize imports", "source.organizeImports"),
        ],
        ..Default::default()
    };
    let (editor, cx) = setup(provider, cx);
    cx.update(|_, cx| {
        gpui_monaco::register_command(cx, "test.after", move |arguments, _| {
            log.borrow_mut().push(arguments.to_vec());
        })
    });
    editor.update(cx, |e, cx| {
        e.set_selections(&[Selection::collapsed(p(0, 4))], cx)
    });
    let keys = if cfg!(target_os = "macos") {
        "cmd-."
    } else {
        "ctrl-."
    };
    cx.simulate_keystrokes(keys);
    draw(cx);
    // Grouped, fixes first; no disabled or source actions.
    assert_eq!(
        editor.read_with(cx, |e, cx| e.code_action_menu_rows(cx)),
        [
            "# Quick Fix",
            "Prefix with underscore",
            "# Extract",
            "Extract to function"
        ]
    );
    cx.simulate_keystrokes("down enter");
    draw(cx);
    assert!(!editor.read_with(cx, |e, _| e.is_code_action_menu_visible()));
    assert_eq!(text(&editor, cx), "let renamed = 1;\nlet y = 2;\n");
    assert_eq!(*ran.borrow(), vec![vec![serde_json::json!(7)]]);
    assert!(cx.update(|window, cx| editor.focus_handle(cx).is_focused(window)));
}

#[gpui::test]
fn commands_with_a_kind_apply_a_single_action(cx: &mut TestAppContext) {
    let provider = Actions {
        actions: vec![
            CodeAction {
                is_preferred: true,
                edit: Some(replace(p(1, 4)..p(1, 5), "z")),
                ..action("Fix it", "quickfix")
            },
            action("Organize imports", "source.organizeImports"),
            action("Other fix", "quickfix"),
        ],
        resolved: Some(replace(p(0, 0)..p(0, 0), "use std;\n")),
        ..Default::default()
    };
    let contexts = provider.contexts.clone();
    let (editor, cx) = setup(provider, cx);
    editor.update(cx, |e, cx| {
        e.set_selections(&[Selection::collapsed(p(1, 4))], cx)
    });
    // Auto fix: only preferred quick fixes, applied without a menu.
    let keys = if cfg!(target_os = "macos") {
        "cmd-alt-."
    } else {
        "shift-alt-."
    };
    cx.simulate_keystrokes(keys);
    draw(cx);
    assert_eq!(text(&editor, cx), "let x = 1;\nlet z = 2;\n");
    let only = contexts.borrow().last().unwrap().only.clone();
    assert_eq!(only, Some("quickfix".into()));
    // Organize imports has no edit: it's resolved first.
    cx.simulate_keystrokes("shift-alt-o");
    draw(cx);
    assert_eq!(text(&editor, cx), "use std;\nlet x = 1;\nlet z = 2;\n");
    assert!(!editor.read_with(cx, |e, _| e.is_code_action_menu_visible()));
}

#[gpui::test]
fn no_actions_and_disabled_ones_say_why(cx: &mut TestAppContext) {
    let provider = Actions {
        actions: vec![CodeAction {
            disabled: Some("Select an expression".into()),
            ..action("Extract", "refactor.extract")
        }],
        ..Default::default()
    };
    let (editor, cx) = setup(provider, cx);
    editor.update(cx, |e, cx| {
        e.set_selections(&[Selection::collapsed(p(0, 4))], cx)
    });
    // Source actions: nothing matches.
    editor.update(cx, |e, cx| e.source_action(cx));
    draw(cx);
    assert!(editor.read_with(cx, |e, _| e.is_message_visible()));
    // Refactor with only a disabled action shows it, greyed out.
    editor.update(cx, |e, cx| e.refactor(cx));
    draw(cx);
    assert_eq!(
        editor.read_with(cx, |e, cx| e.code_action_menu_rows(cx)),
        ["# Extract", "Extract"]
    );
    // It can't be applied.
    cx.simulate_keystrokes("enter");
    draw(cx);
    assert!(editor.read_with(cx, |e, _| e.is_code_action_menu_visible()));
    cx.simulate_keystrokes("escape");
    draw(cx);
    assert!(!editor.read_with(cx, |e, _| e.is_code_action_menu_visible()));
    // Quick fix ignores disabled actions: none available.
    editor.update(cx, |e, cx| e.quick_fix(cx));
    draw(cx);
    assert!(!editor.read_with(cx, |e, _| e.is_code_action_menu_visible()));
    assert!(editor.read_with(cx, |e, _| e.is_message_visible()));
}

fn code_action(editor: &Entity<CodeEditor>, cx: &mut VisualTestContext, args: serde_json::Value) {
    cx.update(|window, cx| {
        editor.update(cx, |e, cx| {
            e.trigger_with_payload("editor.action.codeAction", args, window, cx)
        })
    })
    .unwrap();
    draw(cx);
}

#[gpui::test]
fn the_code_action_command_takes_arguments(cx: &mut TestAppContext) {
    let provider = Actions {
        actions: vec![
            CodeAction {
                edit: Some(replace(p(1, 4)..p(1, 5), "z")),
                ..action("Fix it", "quickfix")
            },
            CodeAction {
                is_preferred: true,
                edit: Some(replace(p(1, 4)..p(1, 5), "w")),
                ..action("Preferred fix", "quickfix")
            },
            CodeAction {
                edit: Some(replace(p(0, 0)..p(0, 0), "use std;\n")),
                ..action("Organize imports", "source.organizeImports")
            },
            action("No kind", ""),
        ],
        ..Default::default()
    };
    let contexts = provider.contexts.clone();
    let (editor, cx) = setup(provider, cx);
    editor.update(cx, |e, cx| {
        e.set_selections(&[Selection::collapsed(p(1, 4))], cx)
    });
    let menu =
        |cx: &mut VisualTestContext| editor.read_with(cx, |e, _| e.is_code_action_menu_visible());
    // Without arguments: every action with a kind, source actions too, in
    // the menu since there are several (Monaco's default `ifSingle`).
    code_action(&editor, cx, serde_json::json!({}));
    assert!(menu(cx));
    let rows = editor.read_with(cx, |e, cx| e.code_action_menu_rows(cx));
    assert!(
        rows.iter().any(|row| row.contains("Organize imports")),
        "{rows:?}"
    );
    assert!(!rows.iter().any(|row| row.contains("No kind")), "{rows:?}");
    cx.simulate_keystrokes("escape");
    draw(cx);

    // Two quick fixes: the menu.
    code_action(&editor, cx, serde_json::json!({ "kind": "quickfix" }));
    assert!(menu(cx));
    cx.simulate_keystrokes("escape");
    draw(cx);
    // `apply: first` applies the first (preferred ones sort first); case
    // doesn't matter.
    code_action(
        &editor,
        cx,
        serde_json::json!({ "kind": "quickfix", "apply": "First" }),
    );
    assert!(!menu(cx));
    assert_eq!(text(&editor, cx), "let x = 1;\nlet w = 2;\n");
    assert_eq!(
        contexts.borrow().last().unwrap().only,
        Some("quickfix".into())
    );

    // `preferred` keeps only preferred ones.
    code_action(
        &editor,
        cx,
        serde_json::json!({ "kind": "quickfix", "preferred": true, "apply": "never" }),
    );
    let rows = editor.read_with(cx, |e, cx| e.code_action_menu_rows(cx));
    assert!(
        rows.iter().any(|row| row.contains("Preferred fix")),
        "{rows:?}"
    );
    assert!(!rows.iter().any(|row| row.contains("Fix it")), "{rows:?}");
    cx.simulate_keystrokes("escape");
    draw(cx);

    // A single one is applied.
    code_action(
        &editor,
        cx,
        serde_json::json!({ "kind": "source.organizeImports" }),
    );
    assert_eq!(text(&editor, cx), "use std;\nlet x = 1;\nlet w = 2;\n");

    // `apply: never` always shows the menu.
    code_action(
        &editor,
        cx,
        serde_json::json!({ "kind": "source", "apply": "never" }),
    );
    assert!(menu(cx));
    cx.simulate_keystrokes("escape");
    draw(cx);

    // Nothing of the kind: a message.
    code_action(&editor, cx, serde_json::json!({ "kind": "refactor" }));
    assert!(!menu(cx));
    assert!(editor.read_with(cx, |e, _| e.is_message_visible()));
}

/// Offers actions with documentation (Monaco
/// `CodeActionProviderMetadata.documentation`).
#[derive(Clone, Default)]
struct Documented(Actions);

impl CodeActionProvider for Documented {
    fn provide_code_actions(
        &self,
        context: &ProviderContext,
        range: Range<PointUtf16>,
        action_context: &CodeActionContext,
        cx: &mut App,
    ) -> Task<anyhow::Result<Vec<CodeAction>>> {
        self.0
            .provide_code_actions(context, range, action_context, cx)
    }

    fn documentation(&self) -> Vec<(CodeActionKind, Command)> {
        let command = |id: &str, title: &str| Command {
            id: id.to_string().into(),
            title: title.to_string().into(),
            arguments: Vec::new(),
            ..Default::default()
        };
        vec![
            (
                "refactor".into(),
                command("docs.refactor", "Learn more about refactorings"),
            ),
            (
                "refactor.extract".into(),
                command("docs.extract", "Learn more about extracting"),
            ),
            (
                "quickfix".into(),
                command("docs.quickfix", "Learn more about fixes"),
            ),
        ]
    }
}

fn bar(editor: &Entity<CodeEditor>, cx: &mut VisualTestContext) -> Vec<String> {
    editor.read_with(cx, |e, cx| {
        e.code_action_menu_bar_actions(cx)
            .into_iter()
            .map(|label| label.to_string())
            .collect()
    })
}

fn rows(editor: &Entity<CodeEditor>, cx: &mut VisualTestContext) -> Vec<String> {
    editor.read_with(cx, |e, cx| e.code_action_menu_rows(cx))
}

fn click_bar(editor: &Entity<CodeEditor>, cx: &mut VisualTestContext, label: &str) {
    assert!(editor.update(cx, |e, cx| e.run_code_action_menu_bar_action(label, cx)));
    draw(cx);
}

#[gpui::test]
fn show_disabled_lists_disabled_actions_until_hidden(cx: &mut TestAppContext) {
    let provider = Actions {
        actions: vec![
            CodeAction {
                edit: Some(replace(p(0, 4)..p(0, 5), "z")),
                ..action("Extract constant", "refactor.extract")
            },
            CodeAction {
                disabled: Some("Select a variable".into()),
                ..action("Inline variable", "refactor.inline")
            },
        ],
        ..Default::default()
    };
    let (editor, cx) = setup(provider, cx);
    editor.update(cx, |e, cx| {
        e.set_selections(&[Selection::collapsed(p(0, 4))], cx)
    });
    editor.update(cx, |e, cx| e.refactor(cx));
    draw(cx);
    assert_eq!(rows(&editor, cx), ["# Extract", "Extract constant"]);
    assert_eq!(bar(&editor, cx), ["Show Disabled"]);
    click_bar(&editor, cx, "Show Disabled");
    assert_eq!(
        rows(&editor, cx),
        [
            "# Extract",
            "Extract constant",
            "# Inline",
            "Inline variable"
        ]
    );
    assert_eq!(bar(&editor, cx), ["Hide Disabled"]);
    // The menu still has focus, and disabled actions are skipped.
    cx.simulate_keystrokes("down down enter");
    draw(cx);
    assert!(!editor.read_with(cx, |e, _| e.is_code_action_menu_visible()));
    assert_eq!(text(&editor, cx), "let z = 1;\nlet y = 2;\n");

    // Monaco keeps the choice for the next menu.
    editor.update(cx, |e, cx| e.refactor(cx));
    draw(cx);
    assert_eq!(rows(&editor, cx).len(), 4);
    click_bar(&editor, cx, "Hide Disabled");
    assert_eq!(rows(&editor, cx).len(), 2);
    assert_eq!(bar(&editor, cx), ["Show Disabled"]);
    cx.simulate_keystrokes("escape");
    draw(cx);

    // Quick fix never lists disabled actions, so it has no toggle.
    editor.update(cx, |e, cx| e.quick_fix(cx));
    draw(cx);
    assert_eq!(rows(&editor, cx), ["# Extract", "Extract constant"]);
    assert!(bar(&editor, cx).is_empty());
}

#[gpui::test]
fn documentation_is_offered_in_the_action_bar(cx: &mut TestAppContext) {
    cx.update(gpui_monaco::init);
    let ran = Rc::new(RefCell::new(Vec::new()));
    let log = ran.clone();
    let provider = Documented(Actions {
        actions: vec![
            action("Extract constant", "refactor.extract"),
            action("Add missing import", "quickfix"),
        ],
        ..Default::default()
    });
    cx.update(|cx| {
        register_code_action_provider(cx, LanguageSelector::All, provider);
        gpui_monaco::register_command(cx, "docs.refactor", move |_, _| {
            log.borrow_mut().push("docs.refactor");
        });
    });
    let (editor, cx) = cx.add_window_view(|window, cx| {
        let model = cx.new(|_| TextModel::new(TEXT));
        CodeEditor::new(model, window, cx)
    });
    cx.update(|window, cx| window.focus(&editor.focus_handle(cx), cx));
    draw(cx);
    editor.update(cx, |e, cx| {
        e.set_selections(&[Selection::collapsed(p(0, 4))], cx)
    });
    // Asked for refactorings: the entry for `refactor` itself, not the
    // more specific one.
    editor.update(cx, |e, cx| e.refactor(cx));
    draw(cx);
    assert_eq!(bar(&editor, cx), ["Learn more about refactorings"]);
    click_bar(&editor, cx, "Learn more about refactorings");
    assert_eq!(*ran.borrow(), ["docs.refactor"]);
    assert!(!editor.read_with(cx, |e, _| e.is_code_action_menu_visible()));

    // Otherwise the first entry matching an action's kind.
    editor.update(cx, |e, cx| e.quick_fix(cx));
    draw(cx);
    assert_eq!(bar(&editor, cx), ["Learn more about refactorings"]);
    cx.simulate_keystrokes("escape");
    draw(cx);

    // The lightbulb's menu has no action bar.
    cx.executor().advance_clock(Duration::from_millis(300));
    draw(cx);
    assert!(editor.read_with(cx, |e, _| e.is_lightbulb_visible()));
    editor.update(cx, |e, cx| e.show_lightbulb_menu_for_tests(cx));
    draw(cx);
    assert!(editor.read_with(cx, |e, _| e.is_code_action_menu_visible()));
    assert!(bar(&editor, cx).is_empty());
}

#[gpui::test]
fn previewing_an_action_goes_through_the_preview_handler(cx: &mut TestAppContext) {
    let ran = Rc::new(RefCell::new(0));
    let count = ran.clone();
    let provider = Actions {
        actions: vec![CodeAction {
            edit: Some(replace(p(0, 4)..p(0, 5), "renamed")),
            command: Some(Command {
                id: "test.after".into(),
                title: "After".into(),
                arguments: Vec::new(),
                ..Default::default()
            }),
            ..action("Extract to function", "refactor.extract")
        }],
        ..Default::default()
    };
    let (editor, cx) = setup(provider, cx);
    cx.update(|_, cx| {
        gpui_monaco::register_command(cx, "test.after", move |_, _| *count.borrow_mut() += 1)
    });
    editor.update(cx, |e, cx| {
        e.set_selections(&[Selection::collapsed(p(0, 4))], cx)
    });
    // The handler cancels the first preview: nothing is applied and the
    // command doesn't run.
    let previews = Rc::new(RefCell::new(Vec::new()));
    let seen = previews.clone();
    let handler = cx.update(|_, cx| {
        gpui_monaco::set_workspace_edit_preview_handler(cx, move |edit, label, _| {
            let first = seen.borrow().is_empty();
            seen.borrow_mut().push(label.to_string());
            Task::ready(Ok((!first).then_some(edit)))
        })
    });
    editor.update(cx, |e, cx| e.refactor(cx));
    draw(cx);
    cx.simulate_keystrokes("secondary-enter");
    draw(cx);
    assert_eq!(*previews.borrow(), ["Extract to function"]);
    assert_eq!(text(&editor, cx), TEXT);
    assert_eq!(*ran.borrow(), 0);
    // The second is accepted.
    editor.update(cx, |e, cx| e.refactor(cx));
    draw(cx);
    cx.simulate_keystrokes("secondary-enter");
    draw(cx);
    assert_eq!(previews.borrow().len(), 2);
    assert_eq!(text(&editor, cx), "let renamed = 1;\nlet y = 2;\n");
    assert_eq!(*ran.borrow(), 1);
    // Enter applies without a preview.
    editor.update(cx, |e, cx| e.undo(cx));
    editor.update(cx, |e, cx| e.refactor(cx));
    draw(cx);
    cx.simulate_keystrokes("enter");
    draw(cx);
    assert_eq!(previews.borrow().len(), 2);
    assert_eq!(text(&editor, cx), "let renamed = 1;\nlet y = 2;\n");
    cx.update(|_, cx| handler.dispose(cx));
    assert!(cx.update(|_, cx| !gpui_monaco::has_workspace_edit_preview_handler(cx)));
}
