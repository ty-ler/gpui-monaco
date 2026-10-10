//! Document symbols from tags queries and providers.

use gpui::{App, AppContext as _, Entity, Task, TestAppContext, VisualTestContext};
use gpui_monaco::outline::flatten_symbols;
use gpui_monaco::providers::*;
use gpui_monaco::text::PointUtf16;
use gpui_monaco::{CodeEditor, TextModel};

fn setup<'a>(
    text: &str,
    language: &str,
    cx: &'a mut TestAppContext,
) -> (Entity<CodeEditor>, &'a mut VisualTestContext) {
    cx.update(|cx| {
        gpui_monaco::init(cx);
        gpui_monaco::set_language_registry(cx, gpui_monaco_languages::registry());
    });
    let text = text.to_string();
    let language = language.to_string();
    let (editor, cx) = cx.add_window_view(|window, cx| {
        let model = cx.new(|_| TextModel::new(&text).with_language(language));
        CodeEditor::new(model, window, cx)
    });
    cx.run_until_parked();
    (editor, cx)
}

fn symbols(
    editor: &Entity<CodeEditor>,
    cx: &mut VisualTestContext,
) -> Vec<(usize, String, SymbolKind)> {
    let result = std::rc::Rc::new(std::cell::RefCell::new(None));
    let sink = result.clone();
    editor.update(cx, |e, cx| {
        let task = e.document_symbols(cx);
        cx.spawn(async move |_, _| *sink.borrow_mut() = Some(task.await))
            .detach();
    });
    cx.run_until_parked();
    let symbols = result.borrow_mut().take().expect("symbols");
    flatten_symbols(&symbols)
        .into_iter()
        .map(|(depth, _, s)| (depth, s.name.to_string(), s.kind))
        .collect()
}

#[gpui::test]
fn rust_symbols_from_tags(cx: &mut TestAppContext) {
    let (editor, cx) = setup(
        "struct Point { x: i32 }\n\nimpl Point {\n    fn len(&self) -> i32 { 0 }\n}\n\nfn main() {}\n\nmacro_rules! m { () => {} }\n",
        "rust",
        cx,
    );
    let found = symbols(&editor, cx);
    assert!(
        found.contains(&(0, "Point".into(), SymbolKind::Class)),
        "{found:?}"
    );
    assert!(
        found.contains(&(1, "len".into(), SymbolKind::Method)),
        "{found:?}"
    );
    assert!(
        found.contains(&(0, "main".into(), SymbolKind::Function)),
        "{found:?}"
    );
}

#[gpui::test]
fn markdown_headings(cx: &mut TestAppContext) {
    let (editor, cx) = setup("# Title\n\ntext\n\n## Part\n", "markdown", cx);
    let names: Vec<String> = symbols(&editor, cx)
        .into_iter()
        .map(|(_, n, _)| n)
        .collect();
    assert_eq!(names, vec!["Title", "Part"]);
}

struct Fixed;
impl DocumentSymbolProvider for Fixed {
    fn provide_document_symbols(
        &self,
        _: &ProviderContext,
        _: &mut App,
    ) -> Task<anyhow::Result<Vec<DocumentSymbol>>> {
        Task::ready(Ok(vec![DocumentSymbol {
            name: "Outer".into(),
            kind: SymbolKind::Namespace,
            range: PointUtf16::new(0, 0)..PointUtf16::new(2, 0),
            selection_range: PointUtf16::new(0, 0)..PointUtf16::new(0, 5),
            children: vec![DocumentSymbol {
                name: "inner".into(),
                kind: SymbolKind::Variable,
                ..Default::default()
            }],
            ..Default::default()
        }]))
    }
}

#[gpui::test]
fn providers_replace_tags(cx: &mut TestAppContext) {
    cx.update(|cx| {
        gpui_monaco::init(cx);
        register_document_symbol_provider(cx, LanguageSelector::All, Fixed);
    });
    let (editor, cx) = setup("fn main() {}\n", "rust", cx);
    assert_eq!(
        symbols(&editor, cx),
        vec![
            (0, "Outer".into(), SymbolKind::Namespace),
            (1, "inner".into(), SymbolKind::Variable)
        ]
    );
}
