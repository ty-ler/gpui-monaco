//! Code lenses (Monaco `contrib/codelens`) with a mock provider.

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::Duration;

use gpui::{App, AppContext as _, Entity, Focusable, Task, TestAppContext, VisualTestContext};
use gpui_monaco::providers::*;
use gpui_monaco::text::PointUtf16;
use gpui_monaco::{CodeEditor, CodeEditorEvent, TextModel, register_command};

/// A lens above every `fn` line: resolved later for the second one.
struct Functions {
    requests: Rc<Cell<usize>>,
}

impl CodeLensProvider for Functions {
    fn provide_code_lenses(
        &self,
        context: &ProviderContext,
        _: &mut App,
    ) -> Task<anyhow::Result<Vec<CodeLens>>> {
        self.requests.set(self.requests.get() + 1);
        let rope = context.snapshot.rope().to_string();
        let lenses = rope
            .lines()
            .enumerate()
            .filter(|(_, line)| line.trim_start().starts_with("fn "))
            .enumerate()
            .map(|(ix, (row, _))| CodeLens {
                range: PointUtf16::new(row as u32, 0)..PointUtf16::new(row as u32, 2),
                command: (ix == 0).then(|| Command {
                    id: "test.references".into(),
                    title: "2 references".into(),
                    arguments: vec![serde_json::json!(row)],
                    ..Default::default()
                }),
                ..Default::default()
            })
            .collect();
        Task::ready(Ok(lenses))
    }

    fn resolve_code_lens(
        &self,
        _: &ProviderContext,
        lens: &CodeLens,
        _: &mut App,
    ) -> Option<Task<anyhow::Result<CodeLens>>> {
        Some(Task::ready(Ok(CodeLens {
            command: Some(Command {
                id: "test.unregistered".into(),
                title: "Run test".into(),
                arguments: Vec::new(),
                ..Default::default()
            }),
            ..lens.clone()
        })))
    }
}

fn setup<'a>(
    text: &str,
    cx: &'a mut TestAppContext,
) -> (
    Entity<CodeEditor>,
    &'a mut VisualTestContext,
    Rc<Cell<usize>>,
) {
    let requests = Rc::new(Cell::new(0));
    let provider = Functions {
        requests: requests.clone(),
    };
    cx.update(|cx| {
        gpui_monaco::init(cx);
        register_code_lens_provider(cx, LanguageSelector::All, provider);
    });
    let text = text.to_string();
    let (editor, cx) = cx.add_window_view(|window, cx| {
        let model = cx.new(|_| TextModel::new(&text));
        CodeEditor::new(model, window, cx)
    });
    cx.update(|window, cx| window.focus(&editor.focus_handle(cx), cx));
    cx.run_until_parked();
    (editor, cx, requests)
}

fn lenses(
    editor: &Entity<CodeEditor>,
    cx: &mut VisualTestContext,
) -> Vec<(u32, Vec<Option<String>>)> {
    cx.run_until_parked();
    editor.read_with(cx, |e, _| e.code_lenses_for_tests())
}

#[gpui::test]
fn lenses_show_above_their_lines_and_resolve(cx: &mut TestAppContext) {
    let (editor, cx, requests) = setup("fn a() {}\n\nfn b() {}\n", cx);
    assert_eq!(
        lenses(&editor, cx),
        vec![
            (0, vec![Some("2 references".into())]),
            // Resolved once visible.
            (2, vec![Some("Run test".into())]),
        ]
    );
    assert_eq!(requests.get(), 1);
}

#[gpui::test]
fn clicking_runs_the_command(cx: &mut TestAppContext) {
    let (editor, cx, _) = setup("fn a() {}\nfn b() {}\n", cx);
    let ran = Rc::new(RefCell::new(Vec::new()));
    let unhandled = Rc::new(RefCell::new(Vec::new()));
    cx.update(|_, cx| {
        let ran = ran.clone();
        register_command(cx, "test.references", move |args, _| {
            ran.borrow_mut().push(args.to_vec())
        });
    });
    let sink = unhandled.clone();
    cx.update(|_, cx| {
        cx.subscribe(&editor, move |_, event, _| {
            if let CodeEditorEvent::ExecuteCommand(command) = event {
                sink.borrow_mut().push(command.id.to_string());
            }
        })
        .detach()
    });
    lenses(&editor, cx);
    editor.update(cx, |e, cx| e.click_code_lens_for_tests(0, 0, cx));
    assert_eq!(*ran.borrow(), vec![vec![serde_json::json!(0)]]);
    // No handler: the host gets it.
    editor.update(cx, |e, cx| e.click_code_lens_for_tests(1, 0, cx));
    assert_eq!(*unhandled.borrow(), vec!["test.unregistered".to_string()]);
}

#[gpui::test]
fn refreshed_after_edits(cx: &mut TestAppContext) {
    let (editor, cx, requests) = setup("fn a() {}\n", cx);
    assert_eq!(lenses(&editor, cx).len(), 1);
    editor.update(cx, |e, cx| {
        e.model()
            .update(cx, |m, cx| m.edit([(0..0, "fn z() {}\n")], cx).unwrap())
    });
    // The zones follow the edit until new lenses arrive after a pause.
    cx.run_until_parked();
    assert_eq!(requests.get(), 1);
    cx.executor().advance_clock(Duration::from_millis(500));
    assert_eq!(lenses(&editor, cx).len(), 2);
    assert_eq!(requests.get(), 2);
}

#[gpui::test]
fn code_lens_option(cx: &mut TestAppContext) {
    let (editor, cx, _) = setup("fn a() {}\n", cx);
    assert_eq!(lenses(&editor, cx).len(), 1);
    editor.update(cx, |e, cx| {
        let mut options = e.options().clone();
        options.code_lens = false;
        e.update_options(options, cx);
    });
    assert!(lenses(&editor, cx).is_empty());
}

#[gpui::test]
fn show_lenses_in_current_line(cx: &mut TestAppContext) {
    let (editor, cx, _) = setup("fn a() {}\nlet x;\n", cx);
    let ran = Rc::new(RefCell::new(Vec::new()));
    cx.update(|_, cx| {
        let ran = ran.clone();
        register_command(cx, "test.references", move |args, _| {
            ran.borrow_mut().push(args.to_vec())
        });
    });
    lenses(&editor, cx);
    let run = |editor: &Entity<CodeEditor>, cx: &mut VisualTestContext| {
        cx.update(|window, cx| {
            editor.update(cx, |e, cx| {
                e.run_action("codelens.showLensesInCurrentLine", window, cx)
            })
        });
        cx.run_until_parked();
    };
    // A line without lenses: nothing shows.
    editor.update(cx, |e, cx| {
        e.set_selections(
            &[gpui_monaco::cursor::Selection::collapsed(PointUtf16::new(
                1, 0,
            ))],
            cx,
        )
    });
    run(&editor, cx);
    assert!(!editor.read_with(cx, |e, _| e.is_quick_access_open()));
    // The lens's line: its commands in the quick input; choosing runs one.
    editor.update(cx, |e, cx| {
        e.set_selections(
            &[gpui_monaco::cursor::Selection::collapsed(PointUtf16::new(
                0, 3,
            ))],
            cx,
        )
    });
    run(&editor, cx);
    assert_eq!(
        editor.read_with(cx, |e, cx| e.quick_access_rows_for_tests(cx)),
        ["> 2 references"]
    );
    cx.simulate_keystrokes("enter");
    cx.run_until_parked();
    assert_eq!(*ran.borrow(), vec![vec![serde_json::json!(0)]]);
}
