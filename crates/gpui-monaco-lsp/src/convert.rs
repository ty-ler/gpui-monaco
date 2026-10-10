//! Conversions between LSP types and gpui-monaco's (the role of
//! `monaco-languageclient`'s protocol converters). Positions need no
//! conversion: both use 0-based lines and UTF-16 columns.

use std::ops::Range;

use gpui::SharedString;
use gpui_monaco::providers::{
    CompletionItem, CompletionItemKind, CompletionList, DocumentHighlight, DocumentHighlightKind,
    Hover, Location, MarkdownString, ParameterInformation, ParameterLabel, SignatureHelp,
    SignatureInformation, WorkspaceEdit,
};
use gpui_monaco::text::PointUtf16;
use gpui_monaco::{Marker, MarkerSeverity};
use lsp_types as lsp;

/// An LSP position as a gpui-monaco one.
pub fn point(position: lsp::Position) -> PointUtf16 {
    PointUtf16::new(position.line, position.character)
}

/// A gpui-monaco position as an LSP one.
pub fn position(point: PointUtf16) -> lsp::Position {
    lsp::Position::new(point.row, point.column)
}

/// An LSP range as a gpui-monaco one.
pub fn range(range: lsp::Range) -> Range<PointUtf16> {
    point(range.start)..point(range.end)
}

/// A gpui-monaco range as an LSP one.
pub fn lsp_range(range: &Range<PointUtf16>) -> lsp::Range {
    lsp::Range::new(position(range.start), position(range.end))
}

/// Escapes markdown syntax so plain text renders as written.
pub fn escape_markdown(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        if r"\`*_{}[]()#+-.!|<>~".contains(c) {
            out.push('\\');
        }
        out.push(c);
    }
    // Keep line breaks (markdown joins single newlines).
    out.replace('\n', "  \n")
}

fn markup(content: lsp::MarkupContent) -> MarkdownString {
    match content.kind {
        lsp::MarkupKind::Markdown => content.value.into(),
        lsp::MarkupKind::PlainText => escape_markdown(&content.value).into(),
    }
}

fn marked_string(value: lsp::MarkedString) -> MarkdownString {
    match value {
        lsp::MarkedString::String(markdown) => markdown.into(),
        lsp::MarkedString::LanguageString(code) => {
            format!("```{}\n{}\n```", code.language, code.value).into()
        }
    }
}

fn documentation(documentation: lsp::Documentation) -> MarkdownString {
    match documentation {
        lsp::Documentation::String(text) => escape_markdown(&text).into(),
        lsp::Documentation::MarkupContent(content) => markup(content),
    }
}

/// `textDocument/hover` results.
pub fn hover(hover: lsp::Hover) -> Hover {
    let contents = match hover.contents {
        lsp::HoverContents::Scalar(value) => vec![marked_string(value)],
        lsp::HoverContents::Array(values) => values.into_iter().map(marked_string).collect(),
        lsp::HoverContents::Markup(content) => vec![markup(content)],
    };
    Hover {
        contents: contents
            .into_iter()
            .filter(|c| !c.0.trim().is_empty())
            .collect(),
        range: hover.range.map(range),
    }
}

/// Monaco's `CompletionItemKind` for an LSP one (the protocol converter's
/// `asCompletionItemKind`).
pub fn completion_kind(kind: lsp::CompletionItemKind) -> CompletionItemKind {
    use CompletionItemKind as K;
    match kind {
        lsp::CompletionItemKind::METHOD => K::Method,
        lsp::CompletionItemKind::FUNCTION => K::Function,
        lsp::CompletionItemKind::CONSTRUCTOR => K::Constructor,
        lsp::CompletionItemKind::FIELD => K::Field,
        lsp::CompletionItemKind::VARIABLE => K::Variable,
        lsp::CompletionItemKind::CLASS => K::Class,
        lsp::CompletionItemKind::INTERFACE => K::Interface,
        lsp::CompletionItemKind::MODULE => K::Module,
        lsp::CompletionItemKind::PROPERTY => K::Property,
        lsp::CompletionItemKind::UNIT => K::Unit,
        lsp::CompletionItemKind::VALUE => K::Value,
        lsp::CompletionItemKind::ENUM => K::Enum,
        lsp::CompletionItemKind::KEYWORD => K::Keyword,
        lsp::CompletionItemKind::SNIPPET => K::Snippet,
        lsp::CompletionItemKind::COLOR => K::Color,
        lsp::CompletionItemKind::FILE => K::File,
        lsp::CompletionItemKind::REFERENCE => K::Reference,
        lsp::CompletionItemKind::FOLDER => K::Folder,
        lsp::CompletionItemKind::ENUM_MEMBER => K::EnumMember,
        lsp::CompletionItemKind::CONSTANT => K::Constant,
        lsp::CompletionItemKind::STRUCT => K::Struct,
        lsp::CompletionItemKind::EVENT => K::Event,
        lsp::CompletionItemKind::OPERATOR => K::Operator,
        lsp::CompletionItemKind::TYPE_PARAMETER => K::TypeParameter,
        _ => K::Text,
    }
}

/// `textDocument/completion` results.
pub fn completion_list(response: lsp::CompletionResponse) -> CompletionList {
    let (items, incomplete) = match response {
        lsp::CompletionResponse::Array(items) => (items, false),
        lsp::CompletionResponse::List(list) => (list.items, list.is_incomplete),
    };
    CompletionList {
        items: items.into_iter().map(completion_item).collect(),
        incomplete,
    }
}

/// One completion item. Snippets are inserted as plain text until the
/// editor supports them.
pub fn completion_item(item: lsp::CompletionItem) -> CompletionItem {
    let snippet = item.insert_text_format == Some(lsp::InsertTextFormat::SNIPPET);
    let as_text = |text: String| -> SharedString {
        if snippet {
            snippet_to_text(&text).into()
        } else {
            text.into()
        }
    };
    let (edit_range, edit_text) = match item.text_edit {
        Some(lsp::CompletionTextEdit::Edit(edit)) => (Some(range(edit.range)), Some(edit.new_text)),
        // Monaco's default `insertMode` is `insert`.
        Some(lsp::CompletionTextEdit::InsertAndReplace(edit)) => {
            (Some(range(edit.insert)), Some(edit.new_text))
        }
        None => (None, None),
    };
    let insert_text = edit_text.or(item.insert_text).map(as_text);
    CompletionItem {
        kind: item.kind.map(completion_kind).unwrap_or_default(),
        detail: item
            .detail
            .or_else(|| {
                item.label_details
                    .as_ref()
                    .and_then(|d| d.description.clone())
            })
            .map(Into::into),
        documentation: item.documentation.map(documentation),
        sort_text: item.sort_text.map(Into::into),
        filter_text: item.filter_text.map(Into::into),
        insert_text,
        range: edit_range,
        preselect: item.preselect.unwrap_or(false),
        commit_characters: item
            .commit_characters
            .unwrap_or_default()
            .iter()
            .filter_map(|c| c.chars().next())
            .collect(),
        additional_text_edits: item
            .additional_text_edits
            .unwrap_or_default()
            .into_iter()
            .map(|edit| (range(edit.range), edit.new_text))
            .collect(),
        label: item.label.into(),
    }
}

/// The text a snippet inserts with every placeholder at its default:
/// `$1` and `$0` become nothing, `${1:name}` becomes `name`, `${1|a,b|}`
/// becomes `a`, and variables become their default or nothing.
pub fn snippet_to_text(snippet: &str) -> String {
    let chars: Vec<char> = snippet.chars().collect();
    let mut out = String::new();
    parse_snippet(&chars, &mut 0, &mut out, false);
    out
}

/// Parses until the end, or a `}` closing a placeholder when `nested`.
fn parse_snippet(chars: &[char], i: &mut usize, out: &mut String, nested: bool) {
    while let Some(&c) = chars.get(*i) {
        match c {
            '\\' => {
                // `\$`, `\}` and `\\` are escapes; other backslashes are text.
                match chars.get(*i + 1) {
                    Some(&next @ ('$' | '}' | '\\' | ',' | '|')) => {
                        out.push(next);
                        *i += 2;
                    }
                    _ => {
                        out.push(c);
                        *i += 1;
                    }
                }
            }
            '}' if nested => {
                *i += 1;
                return;
            }
            '$' => {
                *i += 1;
                parse_dollar(chars, i, out);
            }
            _ => {
                out.push(c);
                *i += 1;
            }
        }
    }
}

/// After a `$`: a tabstop, placeholder, choice or variable.
fn parse_dollar(chars: &[char], i: &mut usize, out: &mut String) {
    let is_name = |c: char| c.is_ascii_alphanumeric() || c == '_';
    match chars.get(*i) {
        // `$1`, `$TM_FILENAME`: nothing.
        Some(c) if is_name(*c) => {
            while chars.get(*i).is_some_and(|c| is_name(*c)) {
                *i += 1;
            }
        }
        Some('{') => {
            *i += 1;
            while chars.get(*i).is_some_and(|c| is_name(*c)) {
                *i += 1;
            }
            match chars.get(*i) {
                // `${1:default}`, `${VAR:default}`.
                Some(':') => {
                    *i += 1;
                    parse_snippet(chars, i, out, true);
                }
                // `${1|one,two|}`: the first choice.
                Some('|') => {
                    *i += 1;
                    let mut first = true;
                    while let Some(&c) = chars.get(*i) {
                        *i += 1;
                        match c {
                            '\\' => {
                                if let Some(&next) = chars.get(*i) {
                                    if first {
                                        out.push(next);
                                    }
                                    *i += 1;
                                }
                            }
                            ',' => first = false,
                            '|' => break,
                            _ if first => out.push(c),
                            _ => {}
                        }
                    }
                    if chars.get(*i) == Some(&'}') {
                        *i += 1;
                    }
                }
                // `${1}`, or a transform (`${1/a/b/}`), which has no text.
                _ => {
                    let mut depth = 1;
                    while let Some(&c) = chars.get(*i) {
                        *i += 1;
                        match c {
                            '\\' => *i += 1,
                            '{' => depth += 1,
                            '}' => {
                                depth -= 1;
                                if depth == 0 {
                                    break;
                                }
                            }
                            _ => {}
                        }
                    }
                }
            }
        }
        // A lone `$` is text.
        _ => out.push('$'),
    }
}

/// Definition results: `None` for locations in the queried document.
pub fn locations(response: lsp::GotoDefinitionResponse, queried: &str) -> Vec<Location> {
    let location = |uri: lsp::Uri, target: lsp::Range| Location {
        uri: (uri.as_str() != queried).then(|| uri.as_str().to_string().into()),
        range: range(target),
    };
    match response {
        lsp::GotoDefinitionResponse::Scalar(l) => vec![location(l.uri, l.range)],
        lsp::GotoDefinitionResponse::Array(ls) => {
            ls.into_iter().map(|l| location(l.uri, l.range)).collect()
        }
        lsp::GotoDefinitionResponse::Link(links) => links
            .into_iter()
            .map(|l| location(l.target_uri, l.target_selection_range))
            .collect(),
    }
}

/// `textDocument/documentHighlight` results.
pub fn document_highlight(highlight: lsp::DocumentHighlight) -> DocumentHighlight {
    DocumentHighlight {
        range: range(highlight.range),
        kind: match highlight.kind {
            Some(lsp::DocumentHighlightKind::READ) => DocumentHighlightKind::Read,
            Some(lsp::DocumentHighlightKind::WRITE) => DocumentHighlightKind::Write,
            _ => DocumentHighlightKind::Text,
        },
    }
}

/// `textDocument/signatureHelp` results.
pub fn signature_help(help: lsp::SignatureHelp) -> SignatureHelp {
    let active_signature = help.active_signature.unwrap_or(0) as usize;
    SignatureHelp {
        active_parameter: help.active_parameter.unwrap_or(0) as usize,
        active_signature: active_signature.min(help.signatures.len().saturating_sub(1)),
        signatures: help
            .signatures
            .into_iter()
            .map(|signature| SignatureInformation {
                label: signature.label.into(),
                documentation: signature.documentation.map(documentation),
                parameters: signature
                    .parameters
                    .unwrap_or_default()
                    .into_iter()
                    .map(|parameter| ParameterInformation {
                        label: match parameter.label {
                            lsp::ParameterLabel::Simple(text) => ParameterLabel::Text(text.into()),
                            lsp::ParameterLabel::LabelOffsets([start, end]) => {
                                ParameterLabel::Range(start as usize..end as usize)
                            }
                        },
                        documentation: parameter.documentation.map(documentation),
                    })
                    .collect(),
                active_parameter: signature.active_parameter.map(|p| p as usize),
            })
            .collect(),
    }
}

/// A diagnostic as a marker.
pub fn marker(diagnostic: lsp::Diagnostic) -> Marker {
    Marker {
        severity: match diagnostic.severity {
            Some(lsp::DiagnosticSeverity::WARNING) => MarkerSeverity::Warning,
            Some(lsp::DiagnosticSeverity::INFORMATION) => MarkerSeverity::Info,
            Some(lsp::DiagnosticSeverity::HINT) => MarkerSeverity::Hint,
            _ => MarkerSeverity::Error,
        },
        message: diagnostic.message.into(),
        range: range(diagnostic.range),
        source: diagnostic.source.map(Into::into),
        code: diagnostic.code.map(|code| match code {
            lsp::NumberOrString::Number(n) => n.to_string().into(),
            lsp::NumberOrString::String(s) => s.into(),
        }),
    }
}

/// A workspace edit's text edits, by document. File operations (create,
/// rename, delete) aren't supported.
pub fn workspace_edit(edit: lsp::WorkspaceEdit) -> anyhow::Result<WorkspaceEdit> {
    let text_edits = |edits: Vec<lsp::TextEdit>| -> Vec<_> {
        edits
            .into_iter()
            .map(|edit| (range(edit.range), edit.new_text))
            .collect()
    };
    let mut result = WorkspaceEdit::default();
    if let Some(changes) = edit.document_changes {
        let edits = match changes {
            lsp::DocumentChanges::Edits(edits) => edits,
            lsp::DocumentChanges::Operations(operations) => operations
                .into_iter()
                .map(|operation| match operation {
                    lsp::DocumentChangeOperation::Edit(edit) => Ok(edit),
                    lsp::DocumentChangeOperation::Op(_) => {
                        Err(anyhow::anyhow!("file operations aren't supported"))
                    }
                })
                .collect::<anyhow::Result<_>>()?,
        };
        for edit in edits {
            let edits = edit
                .edits
                .into_iter()
                .map(|edit| match edit {
                    lsp::OneOf::Left(edit) => edit,
                    lsp::OneOf::Right(annotated) => annotated.text_edit,
                })
                .collect();
            result.edits.push((
                Some(edit.text_document.uri.as_str().to_string().into()),
                text_edits(edits),
            ));
        }
    } else if let Some(changes) = edit.changes {
        for (uri, edits) in changes {
            result
                .edits
                .push((Some(uri.as_str().to_string().into()), text_edits(edits)));
        }
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snippets_as_text() {
        assert_eq!(snippet_to_text("foo($1)$0"), "foo()");
        assert_eq!(snippet_to_text("foo(${1:a}, ${2:b})"), "foo(a, b)");
        assert_eq!(snippet_to_text("${1:outer ${2:inner}}"), "outer inner");
        assert_eq!(snippet_to_text("${1|one,two|}"), "one");
        assert_eq!(snippet_to_text("${TM_FILENAME:file}.rs"), "file.rs");
        assert_eq!(snippet_to_text(r"\$1 \} \\ x"), r"$1 } \ x");
        assert_eq!(snippet_to_text("cost: $"), "cost: $");
        assert_eq!(snippet_to_text("${1/(.*)/$1/}x"), "x");
    }

    #[test]
    fn plain_text_is_escaped() {
        assert_eq!(escape_markdown("a_b *c*"), r"a\_b \*c\*");
        assert_eq!(escape_markdown("one\ntwo"), "one  \ntwo");
    }

    #[test]
    fn completion_items() {
        let item: lsp::CompletionItem = serde_json::from_value(serde_json::json!({
            "label": "push",
            "kind": 2,
            "detail": "fn(&mut self, T)",
            "documentation": { "kind": "markdown", "value": "Appends." },
            "insertTextFormat": 2,
            "textEdit": {
                "insert": { "start": { "line": 0, "character": 2 }, "end": { "line": 0, "character": 4 } },
                "replace": { "start": { "line": 0, "character": 2 }, "end": { "line": 0, "character": 6 } },
                "newText": "push(${1:value})$0"
            },
            "commitCharacters": ["("],
            "additionalTextEdits": [{
                "range": { "start": { "line": 0, "character": 0 }, "end": { "line": 0, "character": 0 } },
                "newText": "use x;\n"
            }]
        }))
        .unwrap();
        let item = completion_item(item);
        assert_eq!(item.kind, CompletionItemKind::Method);
        assert_eq!(item.insert_text(), "push(value)");
        assert_eq!(
            item.range,
            Some(PointUtf16::new(0, 2)..PointUtf16::new(0, 4))
        );
        assert_eq!(item.commit_characters, vec!['(']);
        assert_eq!(item.documentation, Some("Appends.".into()));
        assert_eq!(item.additional_text_edits[0].1, "use x;\n");
    }

    #[test]
    fn hovers_and_diagnostics() {
        let hover: lsp::Hover = serde_json::from_value(serde_json::json!({
            "contents": [{ "language": "rust", "value": "fn main()" }, "Docs"],
        }))
        .unwrap();
        assert_eq!(
            super::hover(hover).contents,
            vec!["```rust\nfn main()\n```".into(), "Docs".into()]
        );
        let diagnostic: lsp::Diagnostic = serde_json::from_value(serde_json::json!({
            "range": { "start": { "line": 1, "character": 0 }, "end": { "line": 1, "character": 3 } },
            "severity": 2,
            "code": "unused",
            "source": "rustc",
            "message": "unused variable",
        }))
        .unwrap();
        let marker = marker(diagnostic);
        assert_eq!(marker.severity, MarkerSeverity::Warning);
        assert_eq!(marker.code.as_deref(), Some("unused"));
        assert_eq!(marker.range, PointUtf16::new(1, 0)..PointUtf16::new(1, 3));
    }
}
