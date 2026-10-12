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
    /// `provided_kinds`, which the kind actions' keybindings need.
    kinds: Option<Vec<CodeActionKind>>,
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

    fn provided_kinds(&self) -> Option<Vec<CodeActionKind>> {
        self.kinds.clone()
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
        let _ = gpui_monaco::register_command(cx, "test.after", move |arguments, _| {
            log.borrow_mut().push(arguments.to_vec());
        });
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

/// Monaco runs a code action's command through `ICommandService`, where
/// editor actions are commands; one nothing handles goes to the host, as
/// a code lens's does.
#[gpui::test]
fn code_action_commands_run_editor_actions(cx: &mut TestAppContext) {
    let command = |id: &str| Command {
        id: id.to_string().into(),
        title: "".into(),
        ..Default::default()
    };
    let provider = Actions {
        actions: vec![
            CodeAction {
                edit: Some(replace(p(0, 4)..p(0, 5), "renamed")),
                command: Some(command("editor.action.selectAll")),
                ..action("Rename and select all", "quickfix")
            },
            CodeAction {
                command: Some(command("host.command")),
                ..action("Host", "quickfix")
            },
        ],
        ..Default::default()
    };
    let (editor, cx) = setup(provider, cx);
    let unhandled = Rc::new(RefCell::new(Vec::new()));
    cx.update(|_, cx| {
        let sink = unhandled.clone();
        cx.subscribe(&editor, move |_, event, _| {
            if let gpui_monaco::CodeEditorEvent::ExecuteCommand(command) = event {
                sink.borrow_mut().push(command.id.to_string());
            }
        })
        .detach();
    });
    editor.update(cx, |e, cx| {
        e.set_selections(&[Selection::collapsed(p(0, 4))], cx)
    });
    cx.simulate_keystrokes("secondary-.");
    draw(cx);
    cx.simulate_keystrokes("enter");
    draw(cx);
    assert_eq!(text(&editor, cx), "let renamed = 1;\nlet y = 2;\n");
    let selection = editor.read_with(cx, |e, _| e.selections()[0]);
    assert_eq!(
        (selection.anchor, selection.head),
        (p(0, 0), p(2, 0)),
        "selectAll ran"
    );
    assert!(unhandled.borrow().is_empty());
    assert!(!editor.read_with(cx, |e, _| e.is_message_visible()));

    cx.simulate_keystrokes("secondary-.");
    draw(cx);
    cx.simulate_keystrokes("down enter");
    draw(cx);
    assert_eq!(*unhandled.borrow(), ["host.command"]);
    assert!(!editor.read_with(cx, |e, _| e.is_message_visible()));
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
        // Monaco's Auto Fix and Organize Imports need the provider to
        // declare these kinds.
        kinds: Some(vec!["quickfix".into(), "source.organizeImports".into()]),
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
        let _ = gpui_monaco::register_command(cx, "docs.refactor", move |_, _| {
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
        let _ =
            gpui_monaco::register_command(cx, "test.after", move |_, _| *count.borrow_mut() += 1);
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

fn menu_layout(
    editor: &Entity<CodeEditor>,
    cx: &mut VisualTestContext,
) -> (gpui::Size<gpui::Pixels>, bool, Vec<Option<String>>) {
    editor
        .read_with(cx, |e, cx| e.code_action_menu_layout_for_tests(cx))
        .unwrap()
}

/// Monaco's `ActionList` reveals the focused row: Down past the rows that
/// fit keeps it in view, and the list is capped and scrolls.
#[gpui::test]
fn up_and_down_keep_the_focused_action_in_view(cx: &mut TestAppContext) {
    let provider = Actions {
        actions: (0..60)
            .map(|i| action(&format!("Fix number {i}"), "quickfix"))
            .collect(),
        ..Default::default()
    };
    let (editor, cx) = setup(provider, cx);
    editor.update(cx, |e, cx| e.quick_fix(cx));
    draw(cx);
    let (size, visible, _) = menu_layout(&editor, cx);
    assert!(visible);
    // 60 rows of 24px don't fit in 70% of the window.
    assert!(size.height < gpui::px(24. * 60.), "{size:?}");
    for _ in 0..45 {
        cx.simulate_keystrokes("down");
    }
    draw(cx);
    draw(cx);
    let (_, visible, _) = menu_layout(&editor, cx);
    assert!(visible, "the focused row scrolled out of view");
    // Up wraps to the last row, also revealed.
    for _ in 0..46 {
        cx.simulate_keystrokes("up");
    }
    draw(cx);
    draw(cx);
    assert!(menu_layout(&editor, cx).1);
}

/// The list is as wide as its widest row (measured, one line each), and
/// rows show their keybinding as Monaco labels it.
#[gpui::test]
fn the_menu_is_measured_and_shows_keybindings(cx: &mut TestAppContext) {
    let long = "Replace this expression with a considerably longer and more descriptive name";
    let provider = Actions {
        actions: vec![
            action("Short", "source"),
            action(long, "source"),
            action("Organize imports", "source.organizeImports"),
        ],
        kinds: Some(vec!["source".into(), "source.organizeImports".into()]),
        ..Default::default()
    };
    let (editor, cx) = setup(provider, cx);
    editor.update(cx, |e, cx| e.source_action(cx));
    draw(cx);
    let (size, _, keys) = menu_layout(&editor, cx);
    // Wider than the old fixed 200-500px guess would allow for one line.
    assert!(size.width > gpui::px(400.), "{size:?}");
    // One 24px row per action and a 26px header.
    assert_eq!(size.height, gpui::px(26. + 3. * 24.));
    let organize = if cfg!(target_os = "macos") {
        "⇧⌥O"
    } else {
        "Shift+Alt+O"
    };
    assert_eq!(keys, [None, None, None, Some(organize.to_string())]);
}

/// Monaco `computeLightBulbInfo`'s titles, with the Quick Fix and Auto Fix
/// keys.
#[gpui::test]
fn the_lightbulb_title_names_its_keybinding(cx: &mut TestAppContext) {
    let provider = Actions {
        actions: vec![action("Inline variable", "refactor.inline")],
        ..Default::default()
    };
    let (editor, cx) = setup(provider, cx);
    editor.update(cx, |e, cx| {
        e.set_selections(&[Selection::collapsed(p(0, 4))], cx)
    });
    wait(cx);
    let title = cx.update(|window, cx| editor.read(cx).lightbulb_title_for_tests(window));
    let quick_fix = if cfg!(target_os = "macos") {
        "⌘."
    } else {
        "Ctrl+."
    };
    assert_eq!(title, Some(format!("Show Code Actions ({quick_fix})")));
}

#[gpui::test]
fn a_preferred_fix_names_the_auto_fix_keys(cx: &mut TestAppContext) {
    let provider = Actions {
        actions: vec![CodeAction {
            is_preferred: true,
            ..action("Add semicolon", "quickfix")
        }],
        kinds: Some(vec!["quickfix".into()]),
        ..Default::default()
    };
    let (editor, cx) = setup(provider, cx);
    editor.update(cx, |e, cx| {
        e.set_selections(&[Selection::collapsed(p(0, 4))], cx)
    });
    wait(cx);
    let title = cx.update(|window, cx| editor.read(cx).lightbulb_title_for_tests(window));
    let auto_fix = if cfg!(target_os = "macos") {
        "⌥⌘."
    } else {
        "Shift+Alt+."
    };
    assert_eq!(
        title,
        Some(format!(
            "Show Code Actions. Preferred Quick Fix Available ({auto_fix})"
        ))
    );
}

fn set_text(editor: &Entity<CodeEditor>, cx: &mut VisualTestContext, value: &str) {
    editor.update(cx, |e, cx| {
        e.model().update(cx, |m, cx| m.set_value(value, cx));
    });
}

fn primary(key: &str) -> String {
    if cfg!(target_os = "macos") {
        format!("cmd-{key}")
    } else {
        format!("ctrl-{key}")
    }
}

/// Monaco `toCoords` reveals the position before anchoring the menu: Cmd+.
/// with the cursor scrolled out of view scrolls back to it, and the menu
/// shows rather than taking the keys unseen.
#[gpui::test]
fn the_menu_reveals_an_off_screen_cursor(cx: &mut TestAppContext) {
    let provider = Actions {
        actions: vec![CodeAction {
            edit: Some(replace(p(0, 4)..p(0, 5), "z")),
            ..action("Rename to z", "quickfix")
        }],
        ..Default::default()
    };
    let (editor, cx) = setup(provider, cx);
    set_text(&editor, cx, &format!("let x = 1;{}", "\n".repeat(300)));
    editor.update(cx, |e, cx| {
        e.set_selections(&[Selection::collapsed(p(0, 4))], cx);
        e.set_scroll_position(gpui::point(gpui::px(0.), gpui::px(3000.)), cx)
    });
    draw(cx);
    assert!(editor.read_with(cx, |e, cx| e.visible_ranges(cx)[0].start.row > 0));
    cx.simulate_keystrokes(&primary("."));
    draw(cx);
    draw(cx);
    assert!(editor.read_with(cx, |e, _| e.is_code_action_menu_visible()));
    assert_eq!(
        editor.read_with(cx, |e, cx| e.visible_ranges(cx)[0].start.row),
        0
    );
    // Laid out and rendered.
    let (size, _, _) = menu_layout(&editor, cx);
    assert!(size.width > gpui::px(0.), "{size:?}");
    cx.simulate_keystrokes("enter");
    draw(cx);
    assert!(text(&editor, cx).starts_with("let z = 1;"));
}

/// The lightbulb's bounds and the window position of `row`'s start.
fn bulb_and_line(
    editor: &Entity<CodeEditor>,
    cx: &mut VisualTestContext,
    row: u32,
) -> (gpui::Bounds<gpui::Pixels>, gpui::Point<gpui::Pixels>) {
    editor.read_with(cx, |e, _| {
        (
            e.code_action_menu_anchor_for_tests()
                .1
                .expect("no lightbulb"),
            e.scrolled_visible_position(p(row, 0)).unwrap(),
        )
    })
}

/// Monaco `LightBulbWidget.update`: on the last line, after code, the bulb
/// goes in the gutter, not over the line above.
#[gpui::test]
fn the_lightbulb_on_the_last_line_goes_in_the_gutter(cx: &mut TestAppContext) {
    let provider = Actions {
        actions: vec![action("Fix", "quickfix")],
        ..Default::default()
    };
    let (editor, cx) = setup(provider, cx);
    editor.update(cx, |e, cx| {
        e.update_options_with(|o| o.glyph_margin = true, cx)
    });
    set_text(&editor, cx, "let a = 1;\nlet b = 2;");
    editor.update(cx, |e, cx| {
        e.set_selections(&[Selection::collapsed(p(1, 4))], cx)
    });
    wait(cx);
    let (bulb, line) = bulb_and_line(&editor, cx, 1);
    assert!(bulb.right() <= line.x, "{bulb:?} over the text at {line:?}");
    assert!(bulb.top() >= line.y, "{bulb:?} above line 2 at {line:?}");
}

/// Monaco `LightBulbWidget.update` hides the bulb with `lightbulb: Off`,
/// even after Cmd+. asked for actions.
#[gpui::test]
fn lightbulb_off_shows_no_bulb_after_quick_fix(cx: &mut TestAppContext) {
    let provider = Actions {
        actions: vec![action("Fix", "quickfix")],
        ..Default::default()
    };
    let (editor, cx) = setup(provider, cx);
    editor.update(cx, |e, cx| {
        e.update_options_with(
            |o| o.lightbulb = gpui_monaco::options::LightbulbMode::Off,
            cx,
        );
        e.set_selections(&[Selection::collapsed(p(0, 4))], cx)
    });
    cx.simulate_keystrokes(&primary("."));
    draw(cx);
    assert!(editor.read_with(cx, |e, _| e.is_code_action_menu_visible()));
    cx.simulate_keystrokes("escape");
    wait(cx);
    assert!(!editor.read_with(cx, |e, _| e.is_lightbulb_visible()));
    assert!(
        editor
            .read_with(cx, |e, _| e.code_action_menu_anchor_for_tests().1)
            .is_none()
    );
}

/// The ranges of the code action highlight (`background` decorations).
fn highlights(editor: &Entity<CodeEditor>, cx: &mut VisualTestContext) -> Vec<Range<PointUtf16>> {
    editor.update(cx, |e, cx| {
        let mut ranges: Vec<_> = e
            .decorations_in_range(p(0, 0)..p(10, 0), cx)
            .into_iter()
            .filter(|d| d.options.background.is_some())
            .map(|d| d.range)
            .collect();
        ranges.sort_by_key(|r| r.start);
        ranges
    })
}

/// Monaco's `onFocus`: the focused action's `ranges`, its diagnostics when
/// it fixes several, else its diagnostics with a "Context: …"
/// announcement.
#[gpui::test]
fn focusing_an_action_highlights_its_ranges(cx: &mut TestAppContext) {
    let marker = |range: Range<PointUtf16>| Marker {
        severity: MarkerSeverity::Warning,
        message: "unused".into(),
        range,
        ..Default::default()
    };
    let provider = Actions {
        actions: vec![
            CodeAction {
                diagnostics: vec![marker(p(0, 4)..p(0, 5))],
                ..action("Fix x", "quickfix")
            },
            CodeAction {
                diagnostics: vec![marker(p(0, 4)..p(0, 5)), marker(p(1, 4)..p(1, 5))],
                ranges: vec![p(0, 0)..p(1, 10)],
                ..action("Fix all", "quickfix")
            },
            CodeAction {
                ranges: vec![p(1, 8)..p(1, 9)],
                ..action("Change 2", "quickfix")
            },
        ],
        ..Default::default()
    };
    let (editor, cx) = setup(provider, cx);
    editor.update(cx, |e, cx| {
        e.set_selections(&[Selection::collapsed(p(0, 4))], cx)
    });
    editor.update(cx, |e, cx| e.quick_fix(cx));
    draw(cx);
    // The first action is focused as the menu opens.
    assert_eq!(highlights(&editor, cx), [p(0, 4)..p(0, 5)]);
    assert_eq!(
        editor
            .read_with(cx, |e, _| e.last_announcement_for_tests())
            .as_deref(),
        Some("Context: x at line 1 and column 5.")
    );
    // Monaco `aria.status`: polite.
    assert!(editor.read_with(cx, |e, _| e.last_announcement_is_status_for_tests()));
    cx.simulate_keystrokes("down");
    draw(cx);
    assert_eq!(
        highlights(&editor, cx),
        [p(0, 4)..p(0, 5), p(1, 4)..p(1, 5)]
    );
    cx.simulate_keystrokes("down");
    draw(cx);
    assert_eq!(highlights(&editor, cx), [p(1, 8)..p(1, 9)]);
    cx.simulate_keystrokes("escape");
    draw(cx);
    assert!(highlights(&editor, cx).is_empty());
}

/// Monaco's comparator puts AI actions after the others, so `apply: first`
/// doesn't pick one listed first by its provider.
#[gpui::test]
fn ai_actions_come_last(cx: &mut TestAppContext) {
    let provider = Actions {
        actions: vec![
            CodeAction {
                is_ai: true,
                edit: Some(replace(p(0, 4)..p(0, 5), "ai")),
                ..action("Ask AI", "quickfix")
            },
            CodeAction {
                edit: Some(replace(p(0, 4)..p(0, 5), "z")),
                ..action("Rename to z", "quickfix")
            },
        ],
        ..Default::default()
    };
    let (editor, cx) = setup(provider, cx);
    editor.update(cx, |e, cx| {
        e.set_selections(&[Selection::collapsed(p(0, 4))], cx)
    });
    editor.update(cx, |e, cx| e.quick_fix(cx));
    draw(cx);
    assert_eq!(rows(&editor, cx), ["# Quick Fix", "Rename to z", "Ask AI"]);
    cx.simulate_keystrokes("escape");
    draw(cx);
    code_action(
        &editor,
        cx,
        serde_json::json!({ "kind": "quickfix", "apply": "first" }),
    );
    draw(cx);
    assert_eq!(text(&editor, cx), "let z = 1;\nlet y = 2;\n");
}

/// `editor.codeActionWidget.showHeaders: false` (Monaco `toMenuItems`):
/// no headers, the actions in order, without keybindings, and a disabled
/// action shows why.
#[gpui::test]
fn show_headers_off_lists_actions_without_groups(cx: &mut TestAppContext) {
    let provider = Actions {
        actions: vec![
            action("Extract", "refactor.extract"),
            action("Organize imports", "source.organizeImports"),
            CodeAction {
                disabled: Some("Select a variable".into()),
                ..action("Inline variable", "refactor.inline")
            },
            action("Fix", "quickfix"),
        ],
        kinds: Some(vec![
            "refactor".into(),
            "source.organizeImports".into(),
            "quickfix".into(),
        ]),
        ..Default::default()
    };
    let (editor, cx) = setup(provider, cx);
    editor.update(cx, |e, cx| {
        e.update_options_with(|o| o.code_action_widget.show_headers = false, cx);
        e.set_selections(&[Selection::collapsed(p(0, 4))], cx)
    });
    editor.update(cx, |e, cx| e.quick_fix(cx));
    draw(cx);
    assert_eq!(rows(&editor, cx), ["Extract", "Fix"]);
    cx.simulate_keystrokes("escape");
    draw(cx);
    // Source actions: no Organize Imports keys.
    editor.update(cx, |e, cx| e.source_action(cx));
    draw(cx);
    assert_eq!(rows(&editor, cx), ["Organize imports"]);
    assert_eq!(menu_layout(&editor, cx).2, [None]);
    cx.simulate_keystrokes("escape");
    draw(cx);
    // Disabled ones by their reason.
    editor.update(cx, |e, cx| e.refactor(cx));
    draw(cx);
    click_bar(&editor, cx, "Show Disabled");
    assert_eq!(rows(&editor, cx), ["Extract", "Select a variable"]);
}

/// Monaco's action list secondary keys: Cmd/Ctrl+Up/Down move, and
/// Cmd/Ctrl+. accepts, so Cmd+. twice applies the first fix.
#[gpui::test]
fn the_menu_takes_its_secondary_keys(cx: &mut TestAppContext) {
    let provider = Actions {
        actions: vec![
            CodeAction {
                edit: Some(replace(p(0, 4)..p(0, 5), "a")),
                ..action("Rename to a", "quickfix")
            },
            CodeAction {
                edit: Some(replace(p(0, 4)..p(0, 5), "b")),
                ..action("Rename to b", "quickfix")
            },
        ],
        ..Default::default()
    };
    let (editor, cx) = setup(provider, cx);
    editor.update(cx, |e, cx| {
        e.set_selections(&[Selection::collapsed(p(0, 4))], cx)
    });
    cx.simulate_keystrokes(&primary("."));
    draw(cx);
    cx.simulate_keystrokes(&format!(
        "{} {} {}",
        primary("down"),
        primary("up"),
        primary(".")
    ));
    draw(cx);
    assert!(!editor.read_with(cx, |e, _| e.is_code_action_menu_visible()));
    assert_eq!(text(&editor, cx), "let a = 1;\nlet y = 2;\n");
    cx.simulate_keystrokes(&primary("."));
    draw(cx);
    cx.simulate_keystrokes(&format!("{} {}", primary("down"), primary(".")));
    draw(cx);
    assert_eq!(text(&editor, cx), "let b = 1;\nlet y = 2;\n");
}

/// Monaco `LightBulbWidget`'s click: the menu at the click's x, a third of
/// a line below the bulb, and a line further when the bulb is on the line
/// above the cursor's, so it covers neither.
#[gpui::test]
fn the_menu_from_the_lightbulb_goes_below_it(cx: &mut TestAppContext) {
    let provider = Actions {
        actions: vec![action("Fix", "quickfix")],
        ..Default::default()
    };
    let (editor, cx) = setup(provider, cx);
    let line_height = editor.read_with(cx, |e, _| e.options().resolved_line_height());
    let pad = (line_height / 3.).floor();

    // Below the cursor's line (the next line is empty).
    editor.update(cx, |e, cx| {
        e.set_selections(&[Selection::collapsed(p(1, 4))], cx)
    });
    wait(cx);
    let (bulb, line) = bulb_and_line(&editor, cx, 2);
    assert!(bulb.top() >= line.y, "{bulb:?} not on line 3");
    let at = bulb.center() + gpui::point(gpui::px(3.), gpui::px(0.));
    cx.simulate_click(at, gpui::Modifiers::none());
    draw(cx);
    assert!(editor.read_with(cx, |e, _| e.is_code_action_menu_visible()));
    let anchor = editor
        .read_with(cx, |e, _| e.code_action_menu_anchor_for_tests().0)
        .unwrap();
    assert_eq!(anchor.origin.x, at.x);
    assert_eq!(anchor.bottom(), bulb.bottom() + pad);
    cx.simulate_keystrokes("escape");
    draw(cx);

    // Above the cursor's line (the line above is indented).
    set_text(&editor, cx, "    a\nlet b = 2;\nlet c = 3;");
    editor.update(cx, |e, cx| {
        e.set_selections(&[Selection::collapsed(p(1, 4))], cx)
    });
    wait(cx);
    let (bulb, line) = bulb_and_line(&editor, cx, 0);
    assert!(
        bulb.bottom() <= line.y + line_height,
        "{bulb:?} not on line 1"
    );
    editor.update(cx, |e, cx| e.show_lightbulb_menu_for_tests(cx));
    draw(cx);
    let anchor = editor
        .read_with(cx, |e, _| e.code_action_menu_anchor_for_tests().0)
        .unwrap();
    assert_eq!(anchor.origin.x, bulb.center().x);
    assert_eq!(anchor.bottom(), bulb.bottom() + pad + line_height);
}

/// Monaco `toCoords` fixes the menu's page coordinates once: scrolling its
/// line out of view leaves the menu where it was, visible, and Enter
/// applies the action shown.
#[gpui::test]
fn the_menu_stays_put_when_its_line_scrolls_away(cx: &mut TestAppContext) {
    let provider = Actions {
        actions: vec![CodeAction {
            edit: Some(replace(p(0, 4)..p(0, 5), "z")),
            ..action("Rename to z", "quickfix")
        }],
        ..Default::default()
    };
    let (editor, cx) = setup(provider, cx);
    set_text(&editor, cx, &format!("let x = 1;{}", "\n".repeat(300)));
    editor.update(cx, |e, cx| {
        e.set_selections(&[Selection::collapsed(p(0, 4))], cx)
    });
    cx.simulate_keystrokes(&primary("."));
    draw(cx);
    let line = editor.read_with(cx, |e, _| e.scrolled_visible_position(p(0, 4)).unwrap());
    let anchor = editor
        .read_with(cx, |e, _| e.code_action_menu_anchor_for_tests().0)
        .expect("pinned where its line was laid out");
    assert_eq!(anchor.origin, line);
    editor.update(cx, |e, cx| {
        e.set_scroll_position(gpui::point(gpui::px(0.), gpui::px(3000.)), cx)
    });
    draw(cx);
    assert!(editor.read_with(cx, |e, cx| e.visible_ranges(cx)[0].start.row > 0));
    assert!(editor.read_with(cx, |e, _| e.is_code_action_menu_visible()));
    assert_eq!(
        editor.read_with(cx, |e, _| e.code_action_menu_anchor_for_tests().0),
        Some(anchor)
    );
    let (size, _, _) = menu_layout(&editor, cx);
    assert!(size.width > gpui::px(0.), "{size:?}");
    cx.simulate_keystrokes("enter");
    draw(cx);
    assert!(text(&editor, cx).starts_with("let z = 1;"));
}

/// Monaco `CodeActionController`: `onDidLayoutChange` hides the menu.
#[gpui::test]
fn a_layout_change_hides_the_menu(cx: &mut TestAppContext) {
    let provider = Actions {
        actions: vec![action("Fix", "quickfix")],
        ..Default::default()
    };
    let (editor, cx) = setup(provider, cx);
    editor.update(cx, |e, cx| {
        e.set_selections(&[Selection::collapsed(p(0, 4))], cx)
    });
    cx.simulate_keystrokes(&primary("."));
    draw(cx);
    assert!(editor.read_with(cx, |e, _| e.is_code_action_menu_visible()));
    // Scrolling isn't a layout change.
    editor.update(cx, |e, cx| {
        e.set_scroll_position(gpui::point(gpui::px(0.), gpui::px(10.)), cx)
    });
    draw(cx);
    assert!(editor.read_with(cx, |e, _| e.is_code_action_menu_visible()));
    cx.simulate_resize(gpui::size(gpui::px(500.), gpui::px(300.)));
    draw(cx);
    assert!(!editor.read_with(cx, |e, _| e.is_code_action_menu_visible()));
}

/// Monaco's `CodeActionsState` is at the selection's start: selecting
/// lines 2 to 4 downward puts the lightbulb and the Cmd+. menu on line 2.
#[gpui::test]
fn the_lightbulb_and_menu_go_at_the_selection_start(cx: &mut TestAppContext) {
    let provider = Actions {
        actions: vec![action("Fix", "quickfix")],
        ..Default::default()
    };
    let (editor, cx) = setup(provider, cx);
    set_text(
        &editor,
        cx,
        "let a = 1;\nlet b = 2;\nlet c = 3;\nlet d = 4;\nlet e = 5;",
    );
    editor.update(cx, |e, cx| {
        e.set_selections(&[Selection::new(p(1, 4), p(3, 4))], cx)
    });
    wait(cx);
    assert_eq!(
        editor.read_with(cx, |e, _| e.lightbulb_position_for_tests()),
        Some(p(1, 4))
    );
    cx.simulate_keystrokes(&primary("."));
    draw(cx);
    assert!(editor.read_with(cx, |e, _| e.is_code_action_menu_visible()));
    let line = editor.read_with(cx, |e, _| e.scrolled_visible_position(p(1, 4)).unwrap());
    let anchor = editor
        .read_with(cx, |e, _| e.code_action_menu_anchor_for_tests().0)
        .unwrap();
    assert_eq!(anchor.origin, line);
    // Selected upward, the start is still line 2.
    cx.simulate_keystrokes("escape");
    editor.update(cx, |e, cx| {
        e.set_selections(&[Selection::new(p(3, 4), p(1, 4))], cx)
    });
    wait(cx);
    assert_eq!(
        editor.read_with(cx, |e, _| e.lightbulb_position_for_tests()),
        Some(p(1, 4))
    );
}

/// Monaco's gutter lightbulb is a glyph margin decoration: without a
/// glyph margin nothing draws, rather than a bulb over the code.
#[gpui::test]
fn no_gutter_lightbulb_without_a_glyph_margin(cx: &mut TestAppContext) {
    let provider = Actions {
        actions: vec![action("Fix", "quickfix")],
        ..Default::default()
    };
    let (editor, cx) = setup(provider, cx);
    editor.update(cx, |e, cx| {
        e.update_options_with(|o| o.glyph_margin = false, cx)
    });
    set_text(&editor, cx, "let a = 1;\nlet b = 2;\nlet c = 3;");
    editor.update(cx, |e, cx| {
        e.set_selections(&[Selection::collapsed(p(1, 4))], cx)
    });
    wait(cx);
    assert!(editor.read_with(cx, |e, _| e.is_lightbulb_visible()));
    assert_eq!(
        editor.read_with(cx, |e, _| e.code_action_menu_anchor_for_tests().1),
        None
    );
    // With the glyph margin, it's in the gutter.
    editor.update(cx, |e, cx| {
        e.update_options_with(|o| o.glyph_margin = true, cx)
    });
    draw(cx);
    let (bulb, line) = bulb_and_line(&editor, cx, 1);
    assert!(bulb.right() <= line.x, "{bulb:?} over the text at {line:?}");
}
