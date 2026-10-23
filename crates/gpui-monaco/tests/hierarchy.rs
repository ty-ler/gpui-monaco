//! Peek Call Hierarchy and Peek Type Hierarchy (VS Code's call and type
//! hierarchy peek views).

use std::ops::Range;

use gpui::{App, AppContext as _, Entity, Focusable, Task, TestAppContext, VisualTestContext};
use gpui_monaco::providers::{
    CallHierarchyProvider, HierarchyItem, IncomingCall, LanguageSelector, OutgoingCall,
    ProviderContext, SymbolKind, TypeHierarchyProvider, register_call_hierarchy_provider,
    register_type_hierarchy_provider,
};
use gpui_monaco::text::PointUtf16;
use gpui_monaco::{CodeEditor, HierarchyDirection, TextModel};

// Rows: 0 `fn helper() {}`, 2 `fn main() {`, 3 `helper();`, 6 `fn caller() {`,
// 7 `main();`.
const TEXT: &str =
    "fn helper() {}\n\nfn main() {\n    helper();\n}\n\nfn caller() {\n    main();\n}\n";

fn p(row: u32, column: u32) -> PointUtf16 {
    PointUtf16::new(row, column)
}

fn item(name: &str, kind: SymbolKind, row: u32) -> HierarchyItem {
    let column = 3;
    HierarchyItem {
        name: name.to_string().into(),
        kind,
        range: p(row, 0)..p(row, 20),
        selection_range: p(row, column)..p(row, column + name.len() as u32),
        ..Default::default()
    }
}

fn call(name: &str, ranges: Vec<Range<PointUtf16>>) -> (HierarchyItem, Vec<Range<PointUtf16>>) {
    let row = match name {
        "helper" => 0,
        "main" => 2,
        _ => 6,
    };
    (item(name, SymbolKind::Function, row), ranges)
}

struct Calls;

impl CallHierarchyProvider for Calls {
    fn prepare_call_hierarchy(
        &self,
        _context: &ProviderContext,
        position: PointUtf16,
        _cx: &mut App,
    ) -> Task<anyhow::Result<Vec<HierarchyItem>>> {
        let items = if position.row == 2 {
            vec![item("main", SymbolKind::Function, 2)]
        } else {
            Vec::new()
        };
        Task::ready(Ok(items))
    }

    fn provide_incoming_calls(
        &self,
        item: &HierarchyItem,
        _cx: &mut App,
    ) -> Task<anyhow::Result<Vec<IncomingCall>>> {
        let calls = match item.name.as_ref() {
            "main" => vec![call("caller", vec![p(7, 4)..p(7, 8)])],
            _ => Vec::new(),
        };
        Task::ready(Ok(calls
            .into_iter()
            .map(|(from, from_ranges)| IncomingCall { from, from_ranges })
            .collect()))
    }

    fn provide_outgoing_calls(
        &self,
        item: &HierarchyItem,
        _cx: &mut App,
    ) -> Task<anyhow::Result<Vec<OutgoingCall>>> {
        let calls = match item.name.as_ref() {
            "main" => vec![call("helper", vec![p(3, 4)..p(3, 10)])],
            _ => Vec::new(),
        };
        Task::ready(Ok(calls
            .into_iter()
            .map(|(to, from_ranges)| OutgoingCall { to, from_ranges })
            .collect()))
    }
}

struct Types;

impl TypeHierarchyProvider for Types {
    fn prepare_type_hierarchy(
        &self,
        _context: &ProviderContext,
        _position: PointUtf16,
        _cx: &mut App,
    ) -> Task<anyhow::Result<Vec<HierarchyItem>>> {
        Task::ready(Ok(vec![item("Dog", SymbolKind::Class, 2)]))
    }

    fn provide_supertypes(
        &self,
        item: &HierarchyItem,
        _cx: &mut App,
    ) -> Task<anyhow::Result<Vec<HierarchyItem>>> {
        let types = match item.name.as_ref() {
            "Dog" => vec![self::item("Animal", SymbolKind::Interface, 0)],
            _ => Vec::new(),
        };
        Task::ready(Ok(types))
    }

    fn provide_subtypes(
        &self,
        _item: &HierarchyItem,
        _cx: &mut App,
    ) -> Task<anyhow::Result<Vec<HierarchyItem>>> {
        Task::ready(Ok(Vec::new()))
    }
}

fn setup(cx: &mut TestAppContext) -> (Entity<CodeEditor>, &mut VisualTestContext) {
    cx.update(|cx| {
        gpui_monaco::init(cx);
        register_call_hierarchy_provider(cx, LanguageSelector::All, Calls);
        register_type_hierarchy_provider(cx, LanguageSelector::All, Types);
    });
    let (editor, cx) = cx.add_window_view(|window, cx| {
        let model = cx.new(|_| TextModel::new(TEXT).with_uri("file:///main.rs"));
        CodeEditor::new(model, window, cx)
    });
    cx.update(|window, cx| window.focus(&editor.focus_handle(cx), cx));
    editor.update(cx, |e, cx| {
        e.set_selections(&[gpui_monaco::cursor::Selection::collapsed(p(2, 4))], cx)
    });
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

fn rows(editor: &Entity<CodeEditor>, cx: &mut VisualTestContext) -> Vec<String> {
    draw(cx);
    editor.read_with(cx, |e, cx| e.hierarchy_rows_for_tests(cx))
}

/// The preview's model text at its first highlighted line, and the
/// preview's cursor.
fn preview_cursor(editor: &Entity<CodeEditor>, cx: &mut VisualTestContext) -> PointUtf16 {
    let preview = editor
        .read_with(cx, |e, cx| e.hierarchy_preview_for_tests(cx))
        .unwrap();
    preview.read_with(cx, |p, _| p.selections()[0].head)
}

#[gpui::test]
fn call_hierarchy_expands_and_switches_direction(cx: &mut TestAppContext) {
    let (editor, cx) = setup(cx);
    editor.update(cx, |e, cx| e.show_call_hierarchy(cx));
    // The root is expanded: its callers.
    assert_eq!(rows(&editor, cx), vec!["> main", "  caller"]);
    assert_eq!(
        editor.read_with(cx, |e, cx| e.hierarchy_direction(cx)),
        Some(HierarchyDirection::IncomingCalls)
    );
    // The preview shows the root's name.
    assert_eq!(preview_cursor(&editor, cx), p(2, 3));

    // Down focuses the caller, whose call site the preview shows; Right
    // asks for its callers, which it has none of.
    cx.simulate_keystrokes("down");
    assert_eq!(rows(&editor, cx), vec!["main", "  > caller"]);
    assert_eq!(preview_cursor(&editor, cx), p(7, 4));
    cx.simulate_keystrokes("right");
    assert_eq!(rows(&editor, cx), vec!["main", "  > caller"]);
    // Left goes back to the parent, then collapses it.
    cx.simulate_keystrokes("left left");
    assert_eq!(rows(&editor, cx), vec!["> main"]);

    // Again: the other direction, from the same root.
    editor.update(cx, |e, cx| e.show_call_hierarchy(cx));
    assert_eq!(rows(&editor, cx), vec!["> main", "  helper"]);
    assert_eq!(
        editor.read_with(cx, |e, cx| e.hierarchy_direction(cx)),
        Some(HierarchyDirection::OutgoingCalls)
    );

    // Enter on the callee goes to it and closes the view.
    cx.simulate_keystrokes("down");
    // An outgoing call's sites are in the caller's model.
    assert_eq!(preview_cursor(&editor, cx), p(3, 4));
    cx.simulate_keystrokes("enter");
    draw(cx);
    editor.read_with(cx, |e, cx| {
        assert_eq!(e.hierarchy_direction(cx), None);
        assert_eq!(e.selections()[0].head, p(0, 3));
    });
}

#[gpui::test]
fn nothing_to_show_and_escape(cx: &mut TestAppContext) {
    let (editor, cx) = setup(cx);
    // No function at the cursor.
    editor.update(cx, |e, cx| {
        e.set_selections(&[gpui_monaco::cursor::Selection::collapsed(p(1, 0))], cx);
        e.show_call_hierarchy(cx);
    });
    assert_eq!(rows(&editor, cx), vec!["[No results]"]);
    cx.simulate_keystrokes("escape");
    draw(cx);
    assert_eq!(
        editor.read_with(cx, |e, cx| e.hierarchy_direction(cx)),
        None
    );

    // A function nobody calls.
    editor.update(cx, |e, cx| {
        e.set_selections(&[gpui_monaco::cursor::Selection::collapsed(p(2, 4))], cx);
        e.show_call_hierarchy(cx);
        e.show_call_hierarchy(cx);
    });
    draw(cx);
    // Toggled before the root came: still opened with the first direction,
    // then switched.
    let rows = rows(&editor, cx);
    assert_eq!(rows[0], "> main");
}

#[gpui::test]
fn type_hierarchy(cx: &mut TestAppContext) {
    let (editor, cx) = setup(cx);
    editor.update(cx, |e, cx| e.show_type_hierarchy(cx));
    assert_eq!(rows(&editor, cx), vec!["> Dog", "  Animal"]);
    editor.update(cx, |e, cx| e.show_type_hierarchy(cx));
    assert_eq!(rows(&editor, cx), vec!["> Dog", "[No subtypes of 'Dog']"]);
    // The call hierarchy replaces it.
    editor.update(cx, |e, cx| e.show_call_hierarchy(cx));
    assert_eq!(rows(&editor, cx), vec!["> main", "  caller"]);
}

#[gpui::test]
fn children_show_their_parents_indent_guide(cx: &mut TestAppContext) {
    let (editor, cx) = setup(cx);
    editor.update(cx, |e, cx| e.show_call_hierarchy(cx));
    assert_eq!(rows(&editor, cx), vec!["> main", "  caller"]);
    // The focused root is expanded, so its guide, beside its child, is
    // active: drawn without hovering the tree.
    let guides = cx.debug_bounds("tree-indent-guides").unwrap();
    let guide = cx.debug_bounds("tree-indent-guide").unwrap();
    assert_eq!(guide.origin, guides.origin);
    assert_eq!(guide.size, gpui::size(gpui::px(8.), gpui::px(22.)));
    assert!(cx.debug_bounds("tree-indent-guide-hidden").is_none());
}
