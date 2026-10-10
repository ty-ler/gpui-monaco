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
