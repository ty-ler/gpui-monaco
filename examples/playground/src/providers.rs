//! Mock language feature providers for the playground (off with `PLAYGROUND_PLAIN=1`).

use gpui::{App, Entity, Task};
use gpui_monaco::providers::*;
use gpui_monaco::text::PointUtf16;
use gpui_monaco::{CodeEditor, Marker, MarkerSeverity, MarkerTag, RelatedInformation};

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
        cx: &mut App,
    ) -> Task<anyhow::Result<Option<Hover>>> {
        self.provide_hover_with_context(context, position, &HoverContext::default(), cx)
    }

    /// Three levels of detail, for the hover's + and − controls.
    fn provide_hover_with_context(
        &self,
        context: &ProviderContext,
        position: PointUtf16,
        hover_context: &HoverContext,
        _: &mut App,
    ) -> Task<anyhow::Result<Option<Hover>>> {
        let word = word_at(context, position);
        if word.is_empty() {
            return Task::ready(Ok(None));
        }
        let level = hover_context
            .verbosity_request
            .as_ref()
            .map_or(0, |request| {
                let previous = request.previous_hover.contents.len() as i32 - 2;
                (previous + request.verbosity_delta).clamp(0, 2)
            });
        let mut contents = vec![
            format!("```rust\nfn {word}(&self) -> usize\n```").into(),
            format!("Documentation for **{word}**, rendered as *markdown* with `inline code`.")
                .into(),
        ];
        if level >= 1 {
            contents.push(format!("More detail about `{word}`: it returns a count.").into());
        }
        if level >= 2 {
            contents.push("Even more: the count is never negative.".into());
        }
        Task::ready(Ok(Some(Hover {
            contents,
            range: None,
            can_increase_verbosity: level < 2,
            can_decrease_verbosity: level > 0,
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
            CompletionItem {
                label_detail: Some("<T>".into()),
                label_description: Some("gpui_monaco::cursor".into()),
                ..item(
                    "Selection",
                    CompletionItemKind::Struct,
                    "struct Selection",
                    "An anchor and a head.",
                )
            },
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
            CompletionItem {
                tags: vec![CompletionItemTag::Deprecated],
                ..item(
                    "set_selection",
                    CompletionItemKind::Method,
                    "fn(&mut self, Selection)",
                    "Deprecated: use `set_selections`.",
                )
            },
            CompletionItem {
                insert_text: Some("for ${1:item} in ${2:iter} {\n\t$0\n}".into()),
                insert_text_is_snippet: true,
                ..item(
                    "for",
                    CompletionItemKind::Snippet,
                    "for loop",
                    "Tab moves through `item`, `iter` and the body.",
                )
            },
            CompletionItem {
                insert_text: Some("#[derive(${1|Debug,Clone,Default|})]$0".into()),
                insert_text_is_snippet: true,
                ..item(
                    "derive",
                    CompletionItemKind::Snippet,
                    "derive attribute",
                    "A choice of traits.",
                )
            },
            // Resolved when focused: its docs arrive later.
            CompletionItem::new("selection_count", CompletionItemKind::Method),
        ];
        Task::ready(Ok(CompletionList {
            items,
            incomplete: false,
        }))
    }

    fn resolve_completion_item(
        &self,
        item: &CompletionItem,
        cx: &mut App,
    ) -> Option<Task<anyhow::Result<CompletionItem>>> {
        if item.documentation.is_some() {
            return None;
        }
        let item = item.clone();
        let timer = cx
            .background_executor()
            .timer(std::time::Duration::from_millis(300));
        Some(cx.background_executor().spawn(async move {
            timer.await;
            Ok(CompletionItem {
                detail: Some("fn(&self) -> usize".into()),
                documentation: Some("How many selections there are (resolved).".into()),
                ..item
            })
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

/// "N references" above each `fn`, counted when the lens scrolls into view.
struct DemoCodeLens;
impl CodeLensProvider for DemoCodeLens {
    fn provide_code_lenses(
        &self,
        context: &ProviderContext,
        _: &mut App,
    ) -> Task<anyhow::Result<Vec<CodeLens>>> {
        let rows = context.snapshot.rope().max_point().row + 1;
        let lenses = (0..rows)
            .filter_map(|row| {
                let line = context.snapshot.line(row);
                let column = line.find("fn ")? as u32;
                Some(CodeLens {
                    range: PointUtf16::new(row, column + 3)..PointUtf16::new(row, column + 3),
                    command: None,
                })
            })
            .collect();
        Task::ready(Ok(lenses))
    }

    fn resolve_code_lens(
        &self,
        context: &ProviderContext,
        lens: &CodeLens,
        cx: &mut App,
    ) -> Option<Task<anyhow::Result<CodeLens>>> {
        let name = word_at(context, lens.range.start);
        let rope = context.snapshot.rope().clone();
        let lens = lens.clone();
        // Counting scans the document: off the main thread.
        Some(cx.background_executor().spawn(async move {
            let count = rope
                .chunks()
                .collect::<String>()
                .matches(name.as_str())
                .count()
                .saturating_sub(1);
            Ok(CodeLens {
                command: Some(Command {
                    id: "playground.references".into(),
                    title: format!("{count} reference{}", if count == 1 { "" } else { "s" }).into(),
                    arguments: vec![name.into()],
                }),
                ..lens
            })
        }))
    }
}

/// Ghost text completing a few lines from what's typed.
struct DemoInlineCompletions;
impl InlineCompletionsProvider for DemoInlineCompletions {
    fn provide_inline_completions(
        &self,
        context: &ProviderContext,
        position: PointUtf16,
        _: InlineCompletionContext,
        _: &mut App,
    ) -> Task<anyhow::Result<InlineCompletionList>> {
        const LINES: &[&str] = &[
            "println!(\"{}\", total);",
            "let total = items.iter().sum::<usize>();",
            "fn render(&self) -> usize {\n    self.items.len()\n}",
        ];
        let line = context.snapshot.line(position.row);
        let before: String = line.chars().take(position.column as usize).collect();
        let typed = before.trim_start();
        let indent = &before[..before.len() - typed.len()];
        let items = if typed.len() < 2 {
            Vec::new()
        } else {
            LINES
                .iter()
                .filter(|l| l.starts_with(typed))
                .map(|l| InlineCompletion {
                    insert_text: format!("{indent}{}", l.replace('\n', &format!("\n{indent}"))),
                    range: Some(PointUtf16::new(position.row, 0)..position),
                    ..Default::default()
                })
                .collect()
        };
        Task::ready(Ok(InlineCompletionList {
            items,
            suppress_suggestions: None,
            commands: Vec::new(),
        }))
    }
}

/// Offers pasting text as a quoted string literal, from the "Paste
/// options" button after a paste.
struct DemoPasteAsString;

impl DocumentPasteEditProvider for DemoPasteAsString {
    fn paste_mime_types(&self) -> Vec<gpui::SharedString> {
        vec!["text/plain".into()]
    }

    fn provided_paste_edit_kinds(&self) -> Vec<gpui::SharedString> {
        vec!["text.string-literal".into()]
    }

    fn provide_document_paste_edits(
        &self,
        _: &ProviderContext,
        _: &[std::ops::Range<PointUtf16>],
        data: &DataTransfer,
        _: DocumentPasteTriggerKind,
        _: &mut App,
    ) -> Task<anyhow::Result<Vec<DocumentDropOrPasteEdit>>> {
        let Some(text) = data.text("text/plain") else {
            return Task::ready(Ok(Vec::new()));
        };
        Task::ready(Ok(vec![DocumentDropOrPasteEdit {
            title: "Insert as String Literal".into(),
            kind: Some("text.string-literal".into()),
            insert_text: EditInsertText::Text(format!("{text:?}")),
            // Plain text first; this one is in the options.
            yield_to: vec![EditYieldTo::MimeType("text/plain".into())],
            ..Default::default()
        }]))
    }
}

pub fn register(cx: &mut App) {
    register_code_lens_provider(cx, LanguageSelector::All, DemoCodeLens);
    register_document_paste_edit_provider(cx, LanguageSelector::All, DemoPasteAsString);
    gpui_monaco::register_command(cx, "playground.references", |args, _| {
        eprintln!("references to {args:?}");
    });
    register_inline_completions_provider(cx, LanguageSelector::All, DemoInlineCompletions);
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
            related_information: vec![RelatedInformation {
                uri: None,
                range: PointUtf16::new(0, 0)..PointUtf16::new(0, 3),
                message: "expected because of this".into(),
            }],
            ..Default::default()
        })
        .chain([
            // Faded and struck-through code (Monaco `showUnused`,
            // `showDeprecated`).
            Marker {
                severity: MarkerSeverity::Hint,
                message: "unused import".into(),
                range: PointUtf16::new(3, 0)..PointUtf16::new(3, 20),
                tags: vec![MarkerTag::Unnecessary],
                ..Default::default()
            },
            Marker {
                severity: MarkerSeverity::Hint,
                message: "deprecated".into(),
                range: PointUtf16::new(5, 4)..PointUtf16::new(5, 12),
                tags: vec![MarkerTag::Deprecated],
                ..Default::default()
            },
        ])
        .collect();
        model.set_markers("demo", markers, cx);
    });
}
