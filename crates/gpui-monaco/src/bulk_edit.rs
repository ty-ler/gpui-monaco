//! Applying workspace edits (Monaco's `IBulkEditService`), and previewing
//! them first: the hook behind Monaco's refactor preview (Cmd/Ctrl+Enter
//! on a code action) and `acceptRenameInputWithPreview`.
//!
//! Applying follows standalone Monaco's `StandaloneBulkEditService`: text
//! edits go to models found by uri, after checking their versions, and
//! move the cursors and decorations at their edges past their text
//! (`replaceMove`).
//! Standalone Monaco rejects file operations; here they go to the host's
//! [file edit handler](set_workspace_file_edit_handler) when one is set.
//!
//! The preview half (`setPreviewHandler`, `hasPreviewHandler`,
//! `showPreview`): standalone Monaco has no preview handler, so previews
//! apply directly; VS Code's workbench sets one that opens its Refactor
//! Preview panel. A host can do the same with
//! [`set_workspace_edit_preview_handler`].

use std::rc::Rc;

use gpui::{App, Context, Entity, Global, SharedString, Task};
use text::PointUtf16;

use crate::api::SingleEditOperation;
use crate::editor::CodeEditor;
use crate::model::{TextModel, model_for_uri};
use crate::providers::{TextEdit, WorkspaceEdit, WorkspaceFileEdit};
use crate::registry::Disposable;
use crate::snippet::parser::SnippetParser;

type PreviewHandler = Rc<
    dyn Fn(WorkspaceEdit, SharedString, &mut App) -> Task<anyhow::Result<Option<WorkspaceEdit>>>,
>;

type FileEditHandler = Rc<dyn Fn(Vec<WorkspaceFileEdit>, &mut App) -> Task<anyhow::Result<()>>>;

#[derive(Default)]
struct PreviewHandlers {
    next_id: u64,
    handler: Option<(u64, PreviewHandler)>,
    file_handler: Option<(u64, FileEditHandler)>,
}

impl Global for PreviewHandlers {}

/// Sets what previews a workspace edit (Monaco
/// `IBulkEditService.setPreviewHandler`), replacing any earlier handler
/// until the returned [`Disposable`] is disposed.
///
/// The handler gets the edit and a label for it (e.g. `Renaming 'foo' to
/// 'bar'`, or the code action's title) and returns the edits to apply, or
/// `None` if the user cancelled. With a handler set, the rename box says
/// "Enter to Rename, Cmd/Ctrl+Enter to Preview" and Cmd/Ctrl+Enter in it and
/// in the code action menu go through the handler, as do edits whose
/// metadata [needs confirmation](WorkspaceEdit::needs_confirmation);
/// without one, previews apply the edit directly, as in standalone Monaco.
pub fn set_workspace_edit_preview_handler(
    cx: &mut App,
    handler: impl Fn(
        WorkspaceEdit,
        SharedString,
        &mut App,
    ) -> Task<anyhow::Result<Option<WorkspaceEdit>>>
    + 'static,
) -> Disposable {
    let handlers = cx.default_global::<PreviewHandlers>();
    handlers.next_id += 1;
    let id = handlers.next_id;
    handlers.handler = Some((id, Rc::new(handler)));
    Disposable::new(move |cx| {
        let handlers = cx.default_global::<PreviewHandlers>();
        if handlers.handler.as_ref().is_some_and(|(i, _)| *i == id) {
            handlers.handler = None;
        }
    })
}

/// Sets what performs a workspace edit's [file
/// operations](WorkspaceEdit::file_edits) (creating, renaming and deleting
/// files, Monaco `IWorkspaceFileEdit`), replacing any earlier handler until
/// the returned [`Disposable`] is disposed.
///
/// The handler gets the operations in order and resolves once they're
/// done; the text edits are applied after, so they can edit a created or
/// renamed file (make it findable with
/// [`set_model_resolver`](crate::set_model_resolver) or
/// [`register_model`](crate::register_model)). An error fails the whole
/// edit. Without a handler, edits with file operations fail, as standalone
/// Monaco's bulk edit service rejects them.
pub fn set_workspace_file_edit_handler(
    cx: &mut App,
    handler: impl Fn(Vec<WorkspaceFileEdit>, &mut App) -> Task<anyhow::Result<()>> + 'static,
) -> Disposable {
    let handlers = cx.default_global::<PreviewHandlers>();
    handlers.next_id += 1;
    let id = handlers.next_id;
    handlers.file_handler = Some((id, Rc::new(handler)));
    Disposable::new(move |cx| {
        let handlers = cx.default_global::<PreviewHandlers>();
        if handlers
            .file_handler
            .as_ref()
            .is_some_and(|(i, _)| *i == id)
        {
            handlers.file_handler = None;
        }
    })
}

/// Whether a preview handler is set (Monaco
/// `IBulkEditService.hasPreviewHandler`).
pub fn has_workspace_edit_preview_handler(cx: &App) -> bool {
    cx.try_global::<PreviewHandlers>()
        .is_some_and(|handlers| handlers.handler.is_some())
}

fn preview_handler(cx: &App) -> Option<PreviewHandler> {
    cx.try_global::<PreviewHandlers>()?
        .handler
        .as_ref()
        .map(|(_, handler)| handler.clone())
}

fn file_edit_handler(cx: &App) -> Option<FileEditHandler> {
    cx.try_global::<PreviewHandlers>()?
        .file_handler
        .as_ref()
        .map(|(_, handler)| handler.clone())
}

/// The plain text of a snippet (Monaco `SnippetParser.asInsertText`).
fn snippet_text(snippet: &str) -> String {
    SnippetParser::new()
        .parse(snippet, false, false)
        .to_string()
}

/// Where `offset` (in the text before `edits`) ends up after them; `edits`
/// are sorted and don't overlap.
fn offset_after(offset: usize, edits: &[(std::ops::Range<usize>, usize)]) -> usize {
    let mut shifted = offset as isize;
    for (range, new_len) in edits {
        if range.end <= offset {
            shifted += *new_len as isize - range.len() as isize;
        }
    }
    shifted.max(0) as usize
}

/// Applies `edits` to `model` as one undo step, moving the decorations at
/// their edges past their text (Monaco `pushEditOperations` with
/// `EditOperation.replaceMove`).
fn edit_model(model: &Entity<TextModel>, edits: Vec<TextEdit>, cx: &mut App) -> anyhow::Result<()> {
    model
        .update(cx, |model, cx| {
            let mut edits: Vec<_> = edits
                .into_iter()
                .map(|(range, text)| {
                    (
                        model.offset_at(range.start)..model.offset_at(range.end),
                        text,
                    )
                })
                .collect();
            edits.sort_by_key(|(range, _)| range.start);
            model.force_move_markers_on_next_edit(
                edits.iter().map(|(range, _)| range.clone()).collect(),
            );
            let result = model.edit(edits, cx).map(|_| ());
            model.force_move_markers_on_next_edit(Vec::new());
            result
        })
        .map_err(|err| anyhow::anyhow!("bad edit - {err}"))
}

/// Applies `edit` without an editor (Monaco `IBulkEditService.apply`, as
/// `workspace/applyEdit` reaches it): an edit whose metadata [needs
/// confirmation](WorkspaceEdit::needs_confirmation) goes through the
/// [preview handler](set_workspace_edit_preview_handler) first, if one is
/// set; then its [file operations](WorkspaceEdit::file_edits) go through the
/// [file edit handler](set_workspace_file_edit_handler), and its text edits
/// to the documents [`model_for_uri`](crate::model_for_uri) finds, each as
/// one undo step (snippet edits insert their plain text). The task says
/// whether the edit was applied (`false` if the preview was cancelled).
///
/// Fails without applying text edits if a document can't be found (Monaco's
/// "bad edit - model not found"; every edit needs a uri here) or changed
/// since its [version](WorkspaceEdit::versions), or if there are file
/// operations and no handler.
pub fn apply_workspace_edit(cx: &mut App, edit: WorkspaceEdit) -> Task<anyhow::Result<bool>> {
    let handler = edit
        .needs_confirmation()
        .then(|| preview_handler(cx))
        .flatten();
    let Some(handler) = handler else {
        let apply = apply_to_models(cx, edit);
        return cx.spawn(async move |_| apply.await.map(|_| true));
    };
    let label = edit
        .metadata
        .as_ref()
        .map(|metadata| metadata.label.clone())
        .unwrap_or_default();
    let previewed = handler(edit, label, cx);
    cx.spawn(async move |cx| {
        let Some(edit) = previewed.await? else {
            return Ok(false);
        };
        cx.update(|cx| apply_to_models(cx, edit)).await?;
        Ok(true)
    })
}

/// [`apply_workspace_edit`] after the preview.
fn apply_to_models(cx: &mut App, mut edit: WorkspaceEdit) -> Task<anyhow::Result<()>> {
    let file_edits = std::mem::take(&mut edit.file_edits);
    if file_edits.is_empty() {
        return Task::ready(apply_text_edits_to_models(cx, edit));
    }
    let Some(handler) = file_edit_handler(cx) else {
        return Task::ready(Err(anyhow::anyhow!(
            "bad edit - file operations need a file edit handler"
        )));
    };
    let done = handler(file_edits, cx);
    cx.spawn(async move |cx| {
        done.await?;
        cx.update(|cx| apply_text_edits_to_models(cx, edit))
    })
}

/// Monaco `StandaloneBulkEditService.apply`'s text edits.
fn apply_text_edits_to_models(cx: &mut App, edit: WorkspaceEdit) -> anyhow::Result<()> {
    let find = |cx: &mut App, target: Option<&SharedString>| {
        let target = target.ok_or_else(|| anyhow::anyhow!("bad edit - model not found"))?;
        model_for_uri(cx, target)
            .ok_or_else(|| anyhow::anyhow!("bad edit - model not found: {target}"))
    };
    for (target, version) in &edit.versions {
        if find(cx, target.as_ref())?.read(cx).version() != *version {
            anyhow::bail!("bad state - model changed in the meantime");
        }
    }
    let mut by_model: Vec<(Entity<TextModel>, Vec<TextEdit>)> = Vec::new();
    let snippets = edit.snippet_edits.into_iter().map(|snippet| {
        let text = snippet_text(&snippet.snippet);
        (snippet.uri, vec![(snippet.range, text)])
    });
    for (target, edits) in edit.edits.into_iter().chain(snippets) {
        let model = find(cx, target.as_ref())?;
        match by_model.iter_mut().find(|(m, _)| *m == model) {
            Some((_, existing)) => existing.extend(edits),
            None => by_model.push((model, edits)),
        }
    }
    for (model, edits) in by_model {
        edit_model(&model, edits, cx)?;
    }
    Ok(())
}

impl CodeEditor {
    /// Monaco `IBulkEditService.apply` with `showPreview`: with `preview`
    /// (or an edit that [needs
    /// confirmation](WorkspaceEdit::needs_confirmation)) and a [preview
    /// handler](set_workspace_edit_preview_handler), the handler decides
    /// what's applied; otherwise `edit` is applied with
    /// [`CodeEditor::apply_workspace_edit`]. The task says whether edits
    /// were applied (Monaco `isApplied`), or fails as applying does.
    pub(crate) fn apply_workspace_edit_with_preview(
        &mut self,
        edit: WorkspaceEdit,
        label: SharedString,
        preview: bool,
        cx: &mut Context<Self>,
    ) -> Task<anyhow::Result<bool>> {
        let preview = preview || edit.needs_confirmation();
        let handler = preview.then(|| preview_handler(cx)).flatten();
        let Some(handler) = handler else {
            let apply = self.apply_workspace_edit(edit, cx);
            return cx.spawn(async move |_, _| apply.await.map(|_| true));
        };
        let previewed = handler(edit, label, cx);
        cx.spawn(async move |this, cx| {
            let Some(edit) = previewed.await? else {
                return Ok(false);
            };
            this.update(cx, |this, cx| this.apply_workspace_edit(edit, cx))?
                .await?;
            Ok(true)
        })
    }

    /// Applies `edit` (Monaco `IBulkEditService.apply`): its [file
    /// operations](WorkspaceEdit::file_edits) through the [file edit
    /// handler](set_workspace_file_edit_handler), then its text edits: to
    /// this editor's model as one undo step with the cursors following
    /// (a single [snippet edit](WorkspaceEdit::snippet_edits) here starts a
    /// snippet session), and to other documents found with
    /// [`model_for_uri`](crate::model_for_uri).
    ///
    /// Fails without applying text edits if a document can't be found or
    /// changed since its [version](WorkspaceEdit::versions) (Monaco's "bad
    /// state - model changed in the meantime"), or if there are file
    /// operations and no handler.
    pub fn apply_workspace_edit(
        &mut self,
        mut edit: WorkspaceEdit,
        cx: &mut Context<Self>,
    ) -> Task<anyhow::Result<()>> {
        let file_edits = std::mem::take(&mut edit.file_edits);
        if file_edits.is_empty() {
            return Task::ready(self.apply_text_edits(edit, cx));
        }
        let Some(handler) = file_edit_handler(cx) else {
            return Task::ready(Err(anyhow::anyhow!(
                "bad edit - file operations need a file edit handler"
            )));
        };
        let done = handler(file_edits, cx);
        cx.spawn(async move |this, cx| {
            done.await?;
            this.update(cx, |this, cx| this.apply_text_edits(edit, cx))?
        })
    }

    /// The text half of [`CodeEditor::apply_workspace_edit`].
    fn apply_text_edits(
        &mut self,
        edit: WorkspaceEdit,
        cx: &mut Context<Self>,
    ) -> anyhow::Result<()> {
        // Monaco compares `URI.toString()`s: `file:///a%20b` is the model of
        // `file:///a b`.
        let resource = self.model().read(cx).resource().clone();
        let is_here = |target: &Option<SharedString>| {
            target.as_deref().is_none_or(|target| {
                target == resource.as_ref() || crate::normalize_uri(target) == resource
            })
        };
        // Monaco checks every version before applying anything.
        for (target, version) in &edit.versions {
            let model = if is_here(target) {
                self.model().clone()
            } else {
                let target = target.as_ref().expect("not here");
                model_for_uri(cx, target).ok_or_else(|| anyhow::anyhow!("{target} isn't open"))?
            };
            if model.read(cx).version() != *version {
                anyhow::bail!("bad state - model changed in the meantime");
            }
        }
        let mut here: Vec<TextEdit> = Vec::new();
        let mut elsewhere: Vec<(Entity<TextModel>, Vec<TextEdit>)> = Vec::new();
        let mut add = |cx: &mut Context<Self>,
                       target: Option<SharedString>,
                       edits: Vec<TextEdit>|
         -> anyhow::Result<()> {
            if is_here(&target) {
                here.extend(edits);
                return Ok(());
            }
            let target = target.expect("not here");
            let model =
                model_for_uri(cx, &target).ok_or_else(|| anyhow::anyhow!("{target} isn't open"))?;
            match elsewhere.iter_mut().find(|(m, _)| *m == model) {
                Some((_, existing)) => existing.extend(edits),
                None => elsewhere.push((model, edits)),
            }
            Ok(())
        };
        for (target, edits) in edit.edits {
            add(cx, target, edits)?;
        }
        // A single snippet edit in this editor becomes a snippet session
        // (Monaco's `SnippetController2.apply`); the others insert their
        // text.
        let snippets_here = edit
            .snippet_edits
            .iter()
            .filter(|snippet| is_here(&snippet.uri))
            .count();
        let mut session = None;
        for snippet in edit.snippet_edits {
            if snippets_here == 1 && is_here(&snippet.uri) && !self.options.read_only {
                session = Some(snippet);
                continue;
            }
            let text = snippet_text(&snippet.snippet);
            add(cx, snippet.uri, vec![(snippet.range, text)])?;
        }
        for (model, edits) in elsewhere {
            edit_model(&model, edits, cx)?;
        }
        if here.is_empty() && session.is_none() {
            return Ok(());
        }
        here.sort_by_key(|(range, _)| range.start);
        self.push_undo_stop(cx);
        // Where the snippet's range lands after the other edits.
        let snippet_range = session.as_ref().map(|snippet| {
            let rope = self.snapshot.rope();
            let offsets: Vec<_> = here
                .iter()
                .map(|(range, text)| {
                    (
                        rope.point_utf16_to_offset(range.start)
                            ..rope.point_utf16_to_offset(range.end),
                        text.len(),
                    )
                })
                .collect();
            let start = rope.point_utf16_to_offset(snippet.range.start);
            let end = rope.point_utf16_to_offset(snippet.range.end);
            offset_after(start, &offsets)..offset_after(end, &offsets)
        });
        let with_snippet = session.is_some();
        if with_snippet {
            // One undo step for the edits and the snippet, as Monaco's
            // `SnippetController2.apply` makes them.
            self.model()
                .update(cx, |model, _| model.start_transaction());
        }
        if !here.is_empty() {
            // Monaco's `StandaloneBulkEditService` uses
            // `EditOperation.replaceMove`.
            let here = here
                .into_iter()
                .map(|(range, text)| SingleEditOperation::new(range, text).force_move_markers())
                .collect();
            self.execute_edit_operations(here, None, cx);
        }
        if let (Some(snippet), Some(range)) = (session, snippet_range) {
            let rope = self.snapshot.rope();
            let range: std::ops::Range<PointUtf16> =
                rope.offset_to_point_utf16(range.start)..rope.offset_to_point_utf16(range.end);
            self.set_selections(&[crate::cursor::Selection::from_range(range, false)], cx);
            self.insert_snippet(
                &snippet.snippet,
                crate::SnippetInsertOptions {
                    adjust_whitespace: !snippet.keep_whitespace,
                    undo_stop_before: false,
                    undo_stop_after: false,
                    ..Default::default()
                },
                cx,
            );
        }
        if with_snippet {
            self.model().update(cx, |model, _| model.end_transaction());
        }
        self.push_undo_stop(cx);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::offset_after;

    #[test]
    fn offsets_move_through_earlier_edits() {
        // "abc def": "abc" -> "x", then "def" starts at 2.
        let edits = [(0..3, 1)];
        assert_eq!(offset_after(4, &edits), 2);
        assert_eq!(offset_after(0, &edits), 0);
        // Edits after the offset don't move it.
        assert_eq!(offset_after(2, &[(4..7, 10)]), 2);
    }
}
