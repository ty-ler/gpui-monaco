//! Mock language feature providers for the playground (off with `PLAYGROUND_PLAIN=1`).

use gpui::{App, Entity, Task};
use gpui_monaco::providers::*;
use gpui_monaco::text::PointUtf16;
use gpui_monaco::{CodeEditor, Marker, MarkerSeverity};

fn word_at(context: &ProviderContext, position: PointUtf16) -> String {
    let line = context.snapshot.line(position.row);
    let chars: Vec<char> = line.chars().collect();
    let is_word = |c: &char| c.is_alphanumeric() || *c == '_';
    let col = (position.column as usize).min(chars.len());
    let start = chars[..col]
        .iter()
        .rposition(|c| !is_word(c))
        .map_or(0, |i| i + 1);
    let end = chars[col..]
        .iter()
        .position(|c| !is_word(c))
        .map_or(chars.len(), |i| col + i);
    chars[start..end].iter().collect()
}

struct DemoHover;
impl HoverProvider for DemoHover {
    fn provide_hover(
        &self,
        context: &ProviderContext,
        position: PointUtf16,
        _: &mut App,
    ) -> Task<anyhow::Result<Option<Hover>>> {
        let word = word_at(context, position);
        if word.is_empty() {
            return Task::ready(Ok(None));
        }
        let contents = vec![
            format!("```rust\nfn {word}(&self) -> usize\n```").into(),
            format!("Documentation for **{word}**, rendered as *markdown* with `inline code`.")
                .into(),
        ];
        Task::ready(Ok(Some(Hover {
            contents,
            range: None,
        })))
    }
}

struct DemoSignatures;
impl SignatureHelpProvider for DemoSignatures {
    fn provide_signature_help(
        &self,
        context: &ProviderContext,
        position: PointUtf16,
        _: SignatureHelpTrigger,
        _: &mut App,
    ) -> Task<anyhow::Result<Option<SignatureHelp>>> {
        let line = context.snapshot.line(position.row);
        let before: String = line.chars().take(position.column as usize).collect();
        let Some(open) = before
            .rfind('(')
            .filter(|o| before.rfind(')').is_none_or(|c| c < *o))
        else {
            return Task::ready(Ok(None));
        };
        let parameter = |label: &str, doc: &str| ParameterInformation {
            label: ParameterLabel::Text(label.into()),
            documentation: Some(doc.into()),
        };
        Task::ready(Ok(Some(SignatureHelp {
            signatures: vec![SignatureInformation {
                label: "transform_offset(offset: usize, bias: Bias) -> usize".into(),
                documentation: Some("Maps an offset in the old version to the new version.".into()),
                parameters: vec![
                    parameter("offset: usize", "The offset in the old text."),
                    parameter("bias: Bias", "Which side of an insertion to stay on."),
                ],
                active_parameter: None,
            }],
            active_signature: 0,
            active_parameter: before[open..].matches(',').count(),
        })))
    }
}

struct DemoCompletions;
impl CompletionProvider for DemoCompletions {
    fn trigger_characters(&self) -> Vec<char> {
        vec!['.']
    }

    fn provide_completions(
        &self,
        _: &ProviderContext,
        _: PointUtf16,
        _: CompletionTrigger,
        _: &mut App,
    ) -> Task<anyhow::Result<CompletionList>> {
        let item = |label: &str, kind, detail: &str, docs: &str| CompletionItem {
            detail: Some(detail.to_string().into()),
            documentation: Some(docs.into()),
            ..CompletionItem::new(label.to_string(), kind)
        };
        let items = vec![
            item(
                "self",
                CompletionItemKind::Keyword,
                "keyword",
                "The current value.",
            ),
            item(
                "selections",
                CompletionItemKind::Method,
                "fn(&self) -> Vec<Selection>",
                "All selections; the first is the primary one.\n\n```rust\nlet s = editor.selections();\n```",
            ),
            item(
                "set_selections",
                CompletionItemKind::Method,
                "fn(&mut self, &[Selection])",
                "Replaces the selections.",
            ),
            item(
                "Selection",
                CompletionItemKind::Struct,
                "struct Selection",
                "An anchor and a head.",
            ),
            item(
                "snapshot",
                CompletionItemKind::Field,
                "BufferSnapshot",
                "The text as of the last sync.",
            ),
            item(
                "select_all",
                CompletionItemKind::Function,
                "fn(&mut self)",
                "Selects everything.",
            ),
        ];
        Task::ready(Ok(CompletionList {
            items,
            incomplete: false,
        }))
    }
}

/// Jumps to the first `fn`, `struct`, `enum` or `trait` declaring the word.
/// Every whole-word occurrence of the word at the cursor.
struct DemoReferences;
impl ReferenceProvider for DemoReferences {
    fn provide_references(
        &self,
        context: &ProviderContext,
        position: PointUtf16,
        _: ReferenceContext,
        _: &mut App,
    ) -> Task<anyhow::Result<Vec<Location>>> {
        let word = word_at(context, position);
        if word.is_empty() {
            return Task::ready(Ok(Vec::new()));
        }
        let is_word = |c: char| c.is_alphanumeric() || c == '_';
        let rows = context.snapshot.rope().max_point().row + 1;
        let mut found = Vec::new();
        for row in 0..rows {
            let line = context.snapshot.line(row);
            for (start, _) in line.match_indices(word.as_str()) {
                let end = start + word.len();
                let before = line[..start].chars().next_back();
                let after = line[end..].chars().next();
                if before.is_some_and(is_word) || after.is_some_and(is_word) {
                    continue;
                }
                let column = line[..start].encode_utf16().count() as u32;
                let len = word.encode_utf16().count() as u32;
                found.push(Location {
                    uri: None,
                    range: PointUtf16::new(row, column)..PointUtf16::new(row, column + len),
                });
            }
        }
        Task::ready(Ok(found))
    }
}

/// Renames every whole-word occurrence (the demo references).
struct DemoRename;
impl RenameProvider for DemoRename {
    fn provide_rename_edits(
        &self,
        context: &ProviderContext,
        position: PointUtf16,
        new_name: &str,
        cx: &mut App,
    ) -> Task<anyhow::Result<Option<WorkspaceEdit>>> {
        let references = DemoReferences.provide_references(
            context,
            position,
            ReferenceContext {
                include_declaration: true,
            },
            cx,
        );
        let new_name = new_name.to_string();
        cx.background_executor().spawn(async move {
            let edits = references
                .await?
                .into_iter()
                .map(|location| (location.range, new_name.clone()))
                .collect();
            Ok(Some(WorkspaceEdit {
                edits: vec![(None, edits)],
            }))
        })
    }
}

/// A quick fix per marker and an uppercase rewrite of the word.
struct DemoCodeActions;
impl CodeActionProvider for DemoCodeActions {
    fn provide_code_actions(
        &self,
        context: &ProviderContext,
        range: std::ops::Range<PointUtf16>,
        action_context: &CodeActionContext,
        _: &mut App,
    ) -> Task<anyhow::Result<Vec<CodeAction>>> {
        let mut actions: Vec<CodeAction> = action_context
            .markers
            .iter()
            .map(|marker| CodeAction {
                title: format!("Silence '{}'", marker.message).into(),
                kind: Some(CodeActionKind::QUICK_FIX.into()),
                diagnostics: vec![marker.clone()],
                is_preferred: true,
                edit: Some(WorkspaceEdit {
                    edits: vec![(
                        None,
                        vec![(marker.range.start..marker.range.start, "_".to_string())],
                    )],
                }),
                ..Default::default()
            })
            .collect();
        let word = word_at(context, range.start);
        if !word.is_empty() && word != word.to_uppercase() {
            let line = context.snapshot.line(range.start.row);
            let column = range.start.column as usize;
            let start = line[..column.min(line.len())]
                .rfind(|c: char| !(c.is_alphanumeric() || c == '_'))
                .map_or(0, |i| i + 1) as u32;
            let end = start + word.encode_utf16().count() as u32;
            actions.push(CodeAction {
                title: format!("Convert '{word}' to UPPERCASE").into(),
                kind: Some(CodeActionKind::REFACTOR_REWRITE.into()),
                edit: Some(WorkspaceEdit {
                    edits: vec![(
                        None,
                        vec![(
                            PointUtf16::new(range.start.row, start)
                                ..PointUtf16::new(range.start.row, end),
                            word.to_uppercase(),
                        )],
                    )],
                }),
                ..Default::default()
            });
        }
        Task::ready(Ok(actions))
    }
}

struct DemoDefinitions;
impl DefinitionProvider for DemoDefinitions {
    fn provide_definition(
        &self,
        context: &ProviderContext,
        position: PointUtf16,
        _: &mut App,
    ) -> Task<anyhow::Result<Vec<Location>>> {
        let word = word_at(context, position);
        if word.is_empty() {
            return Task::ready(Ok(Vec::new()));
        }
        let rows = context.snapshot.rope().max_point().row + 1;
        let found = (0..rows).find_map(|row| {
            let line = context.snapshot.line(row);
            ["fn ", "struct ", "enum ", "trait "]
                .iter()
                .find_map(|keyword| {
                    let declaration = format!("{keyword}{word}");
                    let column = line.find(&declaration)? + keyword.len();
                    let after = line[column + word.len()..].chars().next();
                    if after.is_some_and(|c| c.is_alphanumeric() || c == '_') {
                        return None;
                    }
                    let column = line[..column].encode_utf16().count() as u32;
                    Some(Location {
                        uri: None,
                        range: PointUtf16::new(row, column)
                            ..PointUtf16::new(row, column + word.encode_utf16().count() as u32),
                    })
                })
        });
        Task::ready(Ok(found.into_iter().collect()))
    }
}

/// The demo formatter's rule: trailing whitespace goes, and runs of spaces
/// after the indentation become one.
fn tidy_line(line: &str) -> String {
    let indent = line.len() - line.trim_start().len();
    let mut out = line[..indent].to_string();
    let mut last_space = false;
    for c in line[indent..].trim_end().chars() {
        if !(c == ' ' && last_space) {
            out.push(c);
        }
        last_space = c == ' ';
    }
    out
}

/// Tidies `rows`, one edit per changed line.
fn tidy_rows(context: &ProviderContext, rows: std::ops::RangeInclusive<u32>) -> Vec<TextEdit> {
    rows.filter(|row| *row <= context.snapshot.max_point().row)
        .filter_map(|row| {
            let line = context.snapshot.line(row);
            let tidy = tidy_line(&line);
            (tidy != line).then(|| {
                let end = PointUtf16::new(row, line.encode_utf16().count() as u32);
                (PointUtf16::new(row, 0)..end, tidy)
            })
        })
        .collect()
}

/// Formats the document, selections, pasted text and the line where `;`
/// or `}` is typed.
struct DemoFormatter;

impl DocumentFormattingProvider for DemoFormatter {
    fn provide_document_formatting_edits(
        &self,
        context: &ProviderContext,
        _: FormattingOptions,
        _: &mut App,
    ) -> Task<anyhow::Result<Vec<TextEdit>>> {
        let last = context.snapshot.max_point().row;
        Task::ready(Ok(tidy_rows(context, 0..=last)))
    }
}

impl DocumentRangeFormattingProvider for DemoFormatter {
    fn provide_document_range_formatting_edits(
        &self,
        context: &ProviderContext,
        range: std::ops::Range<PointUtf16>,
        _: FormattingOptions,
        _: &mut App,
    ) -> Task<anyhow::Result<Vec<TextEdit>>> {
        Task::ready(Ok(tidy_rows(context, range.start.row..=range.end.row)))
    }
}

impl OnTypeFormattingProvider for DemoFormatter {
    fn auto_format_trigger_characters(&self) -> Vec<char> {
        vec![';', '}']
    }

    fn provide_on_type_formatting_edits(
        &self,
        context: &ProviderContext,
        position: PointUtf16,
        _: char,
        _: FormattingOptions,
        _: &mut App,
    ) -> Task<anyhow::Result<Vec<TextEdit>>> {
        Task::ready(Ok(tidy_rows(context, position.row..=position.row)))
    }
}

pub fn register(cx: &mut App) {
    register_document_formatting_provider(cx, LanguageSelector::All, DemoFormatter);
    register_document_range_formatting_provider(cx, LanguageSelector::All, DemoFormatter);
    register_on_type_formatting_provider(cx, LanguageSelector::All, DemoFormatter);
    register_definition_provider(cx, LanguageSelector::All, DemoDefinitions);
    register_reference_provider(cx, LanguageSelector::All, DemoReferences);
    register_rename_provider(cx, LanguageSelector::All, DemoRename);
    register_code_action_provider(cx, LanguageSelector::All, DemoCodeActions);
    register_completion_provider(cx, LanguageSelector::All, DemoCompletions);
    register_hover_provider(cx, LanguageSelector::All, DemoHover);
    register_signature_help_provider(cx, LanguageSelector::All, DemoSignatures);
}

pub fn add_markers(editor: &Entity<CodeEditor>, cx: &mut App) {
    let model = editor.read(cx).model().clone();
    model.update(cx, |model, cx| {
        let markers = [
            (10, MarkerSeverity::Error, "mismatched types"),
            (24, MarkerSeverity::Warning, "unused variable"),
        ]
        .into_iter()
        .map(|(row, severity, message)| Marker {
            severity,
            message: message.into(),
            range: PointUtf16::new(row, 4)..PointUtf16::new(row, 14),
            source: Some("demo".into()),
            code: Some("E0308".into()),
        })
        .collect();
        model.set_markers("demo", markers, cx);
    });
}
