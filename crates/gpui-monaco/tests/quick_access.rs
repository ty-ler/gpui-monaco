//! Quick access (Monaco's standalone quick access): go to line, go to
//! symbol and the help list.

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
        ]
    );
    // Choosing one switches to it.
    cx.simulate_keystrokes("enter");
    assert_eq!(
        rows(&editor, cx),
        vec!["> Type a line number to go to (from 1 to 14)."]
    );
}
