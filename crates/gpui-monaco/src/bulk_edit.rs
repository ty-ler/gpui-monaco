//! Previewing workspace edits before they're applied: the hook behind
//! Monaco's refactor preview (Cmd/Ctrl+Enter on a code action) and
//! `acceptRenameInputWithPreview`.
//!
//! Ports the preview half of Monaco's `IBulkEditService`
//! (`setPreviewHandler`, `hasPreviewHandler`, `showPreview`). Standalone
//! Monaco has no preview handler, so previews apply directly; VS Code's
//! workbench sets one that opens its Refactor Preview panel. A host can do
//! the same with [`set_workspace_edit_preview_handler`].

use std::rc::Rc;

use gpui::{App, Context, Global, SharedString, Task};

use crate::editor::CodeEditor;
use crate::providers::WorkspaceEdit;
use crate::registry::Disposable;

type PreviewHandler = Rc<
    dyn Fn(WorkspaceEdit, SharedString, &mut App) -> Task<anyhow::Result<Option<WorkspaceEdit>>>,
>;

#[derive(Default)]
struct PreviewHandlers {
    next_id: u64,
    handler: Option<(u64, PreviewHandler)>,
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
/// in the code action menu go through the handler; without one, previews
/// apply the edit directly, as in standalone Monaco.
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

impl CodeEditor {
    /// Monaco `IBulkEditService.apply` with `showPreview`: with `preview`
    /// and a [preview handler](set_workspace_edit_preview_handler), the
    /// handler decides what's applied; otherwise `edit` is applied with
    /// [`CodeEditor::apply_workspace_edit`]. The task says whether edits
    /// were applied (Monaco `isApplied`), or fails as applying does.
    pub(crate) fn apply_workspace_edit_with_preview(
        &mut self,
        edit: WorkspaceEdit,
        label: SharedString,
        preview: bool,
        cx: &mut Context<Self>,
    ) -> Task<anyhow::Result<bool>> {
        let handler = preview.then(|| preview_handler(cx)).flatten();
        let Some(handler) = handler else {
            return Task::ready(self.apply_workspace_edit(edit, cx).map(|_| true));
        };
        let previewed = handler(edit, label, cx);
        cx.spawn(async move |this, cx| {
            let Some(edit) = previewed.await? else {
                return Ok(false);
            };
            this.update(cx, |this, cx| this.apply_workspace_edit(edit, cx))??;
            Ok(true)
        })
    }
}
