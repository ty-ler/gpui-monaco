//! Go to Symbol in Workspace: quick access's `#` mode over the workspace
//! symbol providers (VS Code's `SymbolsQuickAccessProvider`).

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::{Arc, Mutex};

use gpui::{App, AppContext as _, Entity, Focusable, Task, TestAppContext, VisualTestContext};
use gpui_monaco::providers::{
    Location, SymbolKind, WorkspaceSymbol, WorkspaceSymbolProvider,
    register_workspace_symbol_provider, unregister_provider,
};
use gpui_monaco::text::PointUtf16;
use gpui_monaco::{CodeEditor, CodeEditorEvent, TextModel};

const URI: &str = "file:///src/main.rs";

fn p(row: u32, column: u32) -> PointUtf16 {
    PointUtf16::new(row, column)
}

/// Returns its symbols for any query, and records the queries; resolving
/// gives a symbol without a range row 3.
struct Symbols {
    queries: Arc<Mutex<Vec<String>>>,
}

impl WorkspaceSymbolProvider for Symbols {
    fn provide_workspace_symbols(
        &self,
        query: &str,
        _cx: &mut App,
    ) -> Task<anyhow::Result<Vec<WorkspaceSymbol>>> {
        self.queries.lock().unwrap().push(query.to_string());
        let symbol = |name: &str, kind, container: Option<&str>, uri: &str, row: Option<u32>| {
            WorkspaceSymbol {
                name: name.to_string().into(),
                kind,
                container_name: container.map(|c| c.to_string().into()),
                uri: uri.to_string().into(),
                range: row.map(|row| p(row, 3)..p(row, 3 + name.len() as u32)),
                ..Default::default()
            }
        };
        Task::ready(Ok(vec![
            symbol("Point", SymbolKind::Struct, None, URI, Some(1)),
            symbol("scale", SymbolKind::Method, Some("Point"), URI, None),
            symbol(
                "parse",
                SymbolKind::Function,
                None,
                "file:///src/lib.rs",
                Some(7),
            ),
        ]))
    }

    fn resolve_workspace_symbol(
        &self,
        symbol: &WorkspaceSymbol,
        _cx: &mut App,
    ) -> Option<Task<anyhow::Result<WorkspaceSymbol>>> {
        let mut symbol = symbol.clone();
        symbol.range = Some(p(3, 7)..p(3, 12));
        Some(Task::ready(Ok(symbol)))
    }
}

fn setup(
    cx: &mut TestAppContext,
) -> (
    Entity<CodeEditor>,
    Arc<Mutex<Vec<String>>>,
    &mut VisualTestContext,
) {
    cx.update(gpui_monaco::init);
    let queries = Arc::new(Mutex::new(Vec::new()));
    let provider = Symbols {
        queries: queries.clone(),
    };
    cx.update(|cx| register_workspace_symbol_provider(cx, provider));
    let (editor, cx) = cx.add_window_view(|window, cx| {
        let model = cx.new(|_| TextModel::new(&"fn main() {}\n".repeat(10)).with_uri(URI));
        CodeEditor::new(model, window, cx)
    });
    cx.update(|window, cx| window.focus(&editor.focus_handle(cx), cx));
    cx.run_until_parked();
    (editor, queries, cx)
}

fn rows(editor: &Entity<CodeEditor>, cx: &mut VisualTestContext) -> Vec<String> {
    cx.run_until_parked();
    editor.read_with(cx, |e, cx| e.quick_access_rows_for_tests(cx))
}

fn open_workspace_symbols(editor: &Entity<CodeEditor>, cx: &mut VisualTestContext) {
    editor.update_in(cx, |e, window, cx| e.open_quick_access("#", window, cx));
}

#[gpui::test]
fn lists_and_filters_workspace_symbols(cx: &mut TestAppContext) {
    let (editor, queries, cx) = setup(cx);
    open_workspace_symbols(&editor, cx);
    assert_eq!(
        rows(&editor, cx),
        vec![
            "> Point (main.rs)",
            "scale (Point · main.rs)",
            "parse (lib.rs)",
        ]
    );
    cx.simulate_input("sc");
    assert_eq!(rows(&editor, cx), vec!["> scale (Point · main.rs)"]);
    // Each query asks the providers again.
    assert_eq!(*queries.lock().unwrap(), vec!["", "s", "sc"]);
    cx.simulate_input("zz");
    assert_eq!(rows(&editor, cx), vec!["> No matching workspace symbols"]);
}

#[gpui::test]
fn opens_symbols_here_and_elsewhere(cx: &mut TestAppContext) {
    let (editor, _, cx) = setup(cx);
    let opened = Rc::new(RefCell::new(None::<Location>));
    let sink = opened.clone();
    cx.update(|_, cx| {
        cx.subscribe(&editor, move |_, event: &CodeEditorEvent, _| {
            if let CodeEditorEvent::OpenLocation(location) = event {
                *sink.borrow_mut() = Some(location.clone());
            }
        })
        .detach();
    });

    // In this model, without a range: resolved, then selected.
    open_workspace_symbols(&editor, cx);
    cx.simulate_input("scale");
    assert_eq!(rows(&editor, cx), vec!["> scale (Point · main.rs)"]);
    cx.simulate_keystrokes("enter");
    cx.run_until_parked();
    editor.read_with(cx, |e, _| {
        assert!(!e.is_quick_access_open());
        assert_eq!(e.selections()[0].head, p(3, 7));
    });
    assert!(opened.borrow().is_none());

    // In another model: offered to the host.
    open_workspace_symbols(&editor, cx);
    cx.simulate_input("parse");
    cx.simulate_keystrokes("enter");
    cx.run_until_parked();
    let location = opened.borrow().clone().unwrap();
    assert_eq!(location.uri.as_deref(), Some("file:///src/lib.rs"));
    assert_eq!(location.range, p(7, 3)..p(7, 8));
}

/// The help lists `#` while a provider is registered (the quick access
/// tests check it's left out without one).
#[gpui::test]
fn help_lists_workspace_symbols_with_a_provider(cx: &mut TestAppContext) {
    let (editor, _, cx) = setup(cx);
    editor.update_in(cx, |e, window, cx| e.open_quick_access("", window, cx));
    assert!(rows(&editor, cx).contains(&"# (Go to Symbol in Workspace...)".to_string()));
    cx.simulate_keystrokes("escape");
    cx.run_until_parked();
    let registration = cx.update(|_, cx| register_workspace_symbol_provider(cx, Empty));
    cx.update(|_, cx| unregister_provider(cx, registration));
    assert_eq!(
        cx.update(|_, cx| gpui_monaco::providers::workspace_symbol_providers(cx).len()),
        1
    );
}

struct Empty;

impl WorkspaceSymbolProvider for Empty {
    fn provide_workspace_symbols(
        &self,
        _query: &str,
        _cx: &mut App,
    ) -> Task<anyhow::Result<Vec<WorkspaceSymbol>>> {
        Task::ready(Ok(Vec::new()))
    }
}
