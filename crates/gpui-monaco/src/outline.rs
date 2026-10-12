//! Document symbols (Monaco's `documentSymbols` / `OutlineModel`): from
//! providers, or from the language's `tags.scm` query.

use gpui::{App, Task};
use text::Rope;

use crate::editor::CodeEditor;
use crate::providers::{DocumentSymbol, SymbolKind, document_symbol_providers};

/// The symbol kind for a `tags.scm` `@definition.<kind>`.
fn tag_kind(kind: &str) -> SymbolKind {
    match kind {
        "class" => SymbolKind::Class,
        "struct" => SymbolKind::Struct,
        "enum" => SymbolKind::Enum,
        "interface" | "trait" => SymbolKind::Interface,
        "module" | "namespace" => SymbolKind::Module,
        "method" => SymbolKind::Method,
        "constant" => SymbolKind::Constant,
        "field" => SymbolKind::Field,
        "property" => SymbolKind::Property,
        "variable" => SymbolKind::Variable,
        "type" => SymbolKind::TypeParameter,
        "object" => SymbolKind::Object,
        "constructor" => SymbolKind::Constructor,
        // VS Code's Markdown outline uses strings for headings and its CSS
        // outline classes for selectors.
        "heading" => SymbolKind::String,
        "selector" => SymbolKind::Class,
        _ => SymbolKind::Function,
    }
}

/// Nests flat definitions (sorted by start, longest first) by range
/// containment.
fn nest(mut flat: Vec<DocumentSymbol>) -> Vec<DocumentSymbol> {
    flat.sort_by_key(|s| (s.range.start, std::cmp::Reverse(s.range.end)));
    let mut roots: Vec<DocumentSymbol> = Vec::new();
    // The path of open symbols, as indices from the roots down.
    let mut stack: Vec<DocumentSymbol> = Vec::new();
    for symbol in flat {
        while let Some(top) = stack.last() {
            if symbol.range.start >= top.range.start && symbol.range.end <= top.range.end {
                break;
            }
            let done = stack.pop().expect("non-empty");
            match stack.last_mut() {
                Some(parent) => parent.children.push(done),
                None => roots.push(done),
            }
        }
        stack.push(symbol);
    }
    while let Some(done) = stack.pop() {
        match stack.last_mut() {
            Some(parent) => parent.children.push(done),
            None => roots.push(done),
        }
    }
    roots
}

/// Symbols from tags: the name is the `@name` text.
pub(crate) fn symbols_from_tags(tags: Vec<syntax::Tag>, rope: &Rope) -> Vec<DocumentSymbol> {
    let point = |offset: usize| rope.offset_to_point_utf16(offset.min(rope.len()));
    let flat = tags
        .into_iter()
        .filter_map(|tag| {
            let name: String = rope.text_for_range(tag.name_range.clone());
            let name = name.lines().next().unwrap_or("").trim().to_string();
            (!name.is_empty()).then(|| DocumentSymbol {
                name: name.into(),
                kind: tag_kind(&tag.kind),
                range: point(tag.range.start)..point(tag.range.end),
                selection_range: point(tag.name_range.start)..point(tag.name_range.end),
                ..Default::default()
            })
        })
        .collect();
    nest(flat)
}

/// Every symbol, depth first, with its depth and container's name: its own
/// `container_name`, else its parent's name (Monaco
/// `OutlineModel._flattenDocumentSymbols`).
pub fn flatten_symbols(
    symbols: &[DocumentSymbol],
) -> Vec<(usize, Option<String>, &DocumentSymbol)> {
    fn walk<'a>(
        symbols: &'a [DocumentSymbol],
        depth: usize,
        container: Option<&str>,
        out: &mut Vec<(usize, Option<String>, &'a DocumentSymbol)>,
    ) {
        for symbol in symbols {
            let own = symbol.container_name.as_deref().filter(|c| !c.is_empty());
            out.push((depth, own.or(container).map(str::to_string), symbol));
            walk(&symbol.children, depth + 1, Some(&symbol.name), out);
        }
    }
    let mut out = Vec::new();
    walk(symbols, 0, None, &mut out);
    out
}

impl CodeEditor {
    /// The document's symbols (Monaco `OutlineModel.create`): every
    /// provider's, in registration order, or the `tags.scm` ones without
    /// providers.
    pub fn document_symbols(&self, cx: &mut App) -> Task<Vec<DocumentSymbol>> {
        let model = self.model().read(cx);
        let providers = document_symbol_providers(cx, self.provider_target(cx));
        if providers.is_empty() {
            let Some(syntax) = model.syntax_snapshot() else {
                return Task::ready(Vec::new());
            };
            // The query walks the whole tree. Its offsets are into the
            // parsed text.
            return cx.background_executor().spawn(async move {
                let rope = syntax.text.rope();
                syntax
                    .highlighter
                    .tags(&syntax.state, rope)
                    .map(|tags| symbols_from_tags(tags, rope))
                    .unwrap_or_default()
            });
        }
        let context = model.provider_context();
        let requests: Vec<_> = providers
            .iter()
            .map(|provider| provider.provide_document_symbols(&context, cx))
            .collect();
        cx.background_executor().spawn(async move {
            let mut symbols = Vec::new();
            for request in requests {
                match request.await {
                    Ok(found) => symbols.extend(found),
                    Err(err) => log::warn!("document symbols failed: {err:#}"),
                }
            }
            symbols
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use text::PointUtf16;

    fn symbol(name: &str, start: u32, end: u32) -> DocumentSymbol {
        DocumentSymbol {
            name: name.to_string().into(),
            range: PointUtf16::new(start, 0)..PointUtf16::new(end, 0),
            ..Default::default()
        }
    }

    #[test]
    fn nests_by_containment() {
        let nested = nest(vec![
            symbol("b", 2, 3),
            symbol("impl", 1, 6),
            symbol("c", 4, 5),
            symbol("d", 7, 8),
        ]);
        assert_eq!(nested.len(), 2);
        assert_eq!(nested[0].name.as_ref(), "impl");
        let children: Vec<_> = nested[0]
            .children
            .iter()
            .map(|c| c.name.to_string())
            .collect();
        assert_eq!(children, vec!["b", "c"]);
        let flat = flatten_symbols(&nested);
        assert_eq!(flat[1].0, 1);
        assert_eq!(flat[1].1.as_deref(), Some("impl"));
    }
}
