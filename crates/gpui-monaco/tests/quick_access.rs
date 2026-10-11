//! Quick access (Monaco's standalone quick access): go to line, go to
//! symbol, the command palette and the help list.

use gpui::{AppContext as _, Entity, Focusable, TestAppContext, VisualTestContext};
use gpui_monaco::text::PointUtf16;
use gpui_monaco::{CodeEditor, TextModel};

const RUST: &str = "struct Point {\n    x: i32,\n}\n\nimpl Point {\n    fn len(&self) -> i32 {\n        self.x\n    }\n\n    fn scale(&mut self) {}\n}\n\nfn main() {}\n";

fn setup<'a>(
    text: &str,
    cx: &'a mut TestAppContext,
) -> (Entity<CodeEditor>, &'a mut VisualTestContext) {
    cx.update(|cx| {
        gpui_monaco::init(cx);
        gpui_monaco::set_language_registry(cx, gpui_monaco_languages::registry());
    });
    let text = text.to_string();
    let (editor, cx) = cx.add_window_view(|window, cx| {
        let model = cx.new(|_| TextModel::new(&text).with_language("rust"));
        CodeEditor::new(model, window, cx)
    });
    cx.update(|window, cx| window.focus(&editor.focus_handle(cx), cx));
    cx.run_until_parked();
    (editor, cx)
}

fn rows(editor: &Entity<CodeEditor>, cx: &mut VisualTestContext) -> Vec<String> {
    cx.run_until_parked();
    editor.read_with(cx, |e, cx| e.quick_access_rows_for_tests(cx))
}

fn cursor(editor: &Entity<CodeEditor>, cx: &mut VisualTestContext) -> PointUtf16 {
    editor.read_with(cx, |e, _| e.selections()[0].head)
}

fn open(editor: &Entity<CodeEditor>, cx: &mut VisualTestContext) -> bool {
    editor.read_with(cx, |e, _| e.is_quick_access_open())
}

#[gpui::test]
fn go_to_line(cx: &mut TestAppContext) {
    let (editor, cx) = setup(RUST, cx);
    cx.simulate_keystrokes("ctrl-g");
    assert!(open(&editor, cx));
    assert_eq!(
        rows(&editor, cx),
        vec!["> Type a line number to go to (from 1 to 14)."]
    );
    cx.simulate_input("6");
    assert_eq!(
        rows(&editor, cx),
        vec!["> Press 'Enter' to go to line 6 or enter colon : to add a column number."]
    );
    cx.simulate_input(":5");
    assert_eq!(
        rows(&editor, cx),
        vec!["> Press 'Enter' to go to line 6 at column 5."]
    );
    cx.simulate_keystrokes("enter");
    assert!(!open(&editor, cx));
    assert_eq!(cursor(&editor, cx), PointUtf16::new(5, 4));
}

#[gpui::test]
fn negative_lines_and_cancel_restores(cx: &mut TestAppContext) {
    let (editor, cx) = setup(RUST, cx);
    cx.simulate_keystrokes("ctrl-g");
    cx.simulate_input("-1");
    assert_eq!(
        rows(&editor, cx),
        vec!["> Press 'Enter' to go to line 14 or enter colon : to add a column number."]
    );
    // Escape: back where it was.
    cx.simulate_keystrokes("escape");
    cx.run_until_parked();
    assert!(!open(&editor, cx));
    assert_eq!(cursor(&editor, cx), PointUtf16::new(0, 0));
}

#[gpui::test]
fn go_to_symbol(cx: &mut TestAppContext) {
    let (editor, cx) = setup(RUST, cx);
    // Inside `len`: it's active.
    editor.update(cx, |e, cx| {
        e.set_selections(
            &[gpui_monaco::cursor::Selection::collapsed(PointUtf16::new(
                6, 8,
            ))],
            cx,
        )
    });
    cx.simulate_keystrokes("cmd-shift-o");
    assert_eq!(
        rows(&editor, cx),
        vec![
            // (tree-sitter-rust's tags don't include fields.)
            "[symbols (5)]",
            "Point",
            "Point",
            "> len (Point)",
            "scale (Point)",
            "main"
        ]
    );
    cx.simulate_input("sc");
    assert_eq!(rows(&editor, cx), vec!["[symbols (1)]", "> scale (Point)"]);
    cx.simulate_keystrokes("enter");
    assert_eq!(cursor(&editor, cx), PointUtf16::new(9, 7));
}

#[gpui::test]
fn symbols_by_category_and_help(cx: &mut TestAppContext) {
    let (editor, cx) = setup(RUST, cx);
    cx.simulate_keystrokes("cmd-shift-o");
    cx.simulate_input(":");
    let grouped = rows(&editor, cx);
    assert!(
        grouped.contains(&"[methods (2)]".to_string()),
        "{grouped:?}"
    );
    assert!(
        grouped.contains(&"[functions (1)]".to_string()),
        "{grouped:?}"
    );
    // Deleting the prefix shows the help.
    cx.simulate_keystrokes("backspace backspace");
    assert_eq!(
        rows(&editor, cx),
        vec![
            "> : (Go to Line/Column...)",
            ":: (Go to Offset...)",
            "@ (Go to Symbol...)",
            "@: (Go to Symbol by Category...)",
            "> (Show And Run Commands)",
        ]
    );
    // Choosing one switches to it.
    cx.simulate_keystrokes("enter");
    assert_eq!(
        rows(&editor, cx),
        vec!["> Type a line number to go to (from 1 to 14)."]
    );
}

fn text(editor: &Entity<CodeEditor>, cx: &mut VisualTestContext) -> String {
    editor.read_with(cx, |e, cx| e.model().read(cx).text())
}

/// The rows without their keybindings (they differ per platform).
fn labels(editor: &Entity<CodeEditor>, cx: &mut VisualTestContext) -> Vec<String> {
    rows(editor, cx)
        .into_iter()
        .map(|row| match row.find(" [") {
            Some(ix) if !row.starts_with('[') => row[..ix].to_string(),
            _ => row,
        })
        .collect()
}

#[gpui::test]
fn command_palette_runs_actions_and_remembers_them(cx: &mut TestAppContext) {
    let (editor, cx) = setup(RUST, cx);
    cx.simulate_keystrokes("f1");
    assert!(open(&editor, cx));
    let all = rows(&editor, cx);
    // Alphabetical, with keybindings.
    assert_eq!(labels(&editor, cx)[0], "> Add Cursor Above");
    assert!(
        all.iter().any(|row| row.starts_with("Fold All [")),
        "{all:?}"
    );

    // Word starts match: "delete line" matches "Delete Duplicate Lines" too.
    cx.simulate_input("delete line");
    assert_eq!(
        labels(&editor, cx),
        vec!["> Delete Duplicate Lines", "Delete Line"]
    );
    cx.simulate_keystrokes("down enter");
    cx.run_until_parked();
    assert!(!open(&editor, cx));
    assert!(text(&editor, cx).starts_with("    x: i32,\n"));

    // Recently used comes first.
    cx.simulate_keystrokes("f1");
    let rows = labels(&editor, cx);
    assert_eq!(
        rows[..3],
        ["[recently used]", "> Delete Line", "[other commands]"]
    );
    assert_eq!(rows[3], "Add Cursor Above");
    cx.simulate_input("zzz");
    assert_eq!(labels(&editor, cx), vec!["> No matching commands"]);
    cx.simulate_keystrokes("escape");
}

#[gpui::test]
fn host_actions_and_preconditions(cx: &mut TestAppContext) {
    let (editor, cx) = setup(RUST, cx);
    editor.update(cx, |editor, cx| {
        let _ = editor.add_action(
            gpui_monaco::EditorActionDescriptor::new("my.hello", "Say Hello", |editor, _, cx| {
                editor
                    .model()
                    .update(cx, |model, cx| model.set_value("hello", cx));
            })
            .alias("Greet")
            .keybinding("ctrl-alt-h"),
            cx,
        );
        editor.update_options(
            gpui_monaco::options::EditorOptions {
                read_only: true,
                ..editor.options().clone()
            },
            cx,
        );
    });
    cx.simulate_keystrokes("f1");
    cx.simulate_input("greet");
    assert_eq!(labels(&editor, cx), vec!["> Say Hello"]);
    // Read-only editors don't offer editing actions.
    cx.simulate_keystrokes("escape");
    cx.simulate_keystrokes("f1");
    cx.simulate_input("delete line");
    assert_eq!(labels(&editor, cx), vec!["> No matching commands"]);
    cx.simulate_keystrokes("escape");
    cx.run_until_parked();
    // Its keybinding runs it.
    cx.simulate_keystrokes("ctrl-alt-h");
    cx.run_until_parked();
    assert_eq!(text(&editor, cx), "hello");
    editor.update(cx, |editor, _| assert!(editor.remove_action("my.hello")));
    let ids: Vec<_> = editor.read_with(cx, |e, cx| {
        e.supported_actions(cx)
            .into_iter()
            .map(|a| a.id.to_string())
            .collect()
    });
    assert!(!ids.contains(&"my.hello".to_string()));
    assert!(ids.contains(&"editor.action.quickCommand".to_string()));
    assert!(!ids.contains(&"editor.action.formatDocument".to_string()));
}
