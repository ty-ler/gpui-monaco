//! Escape cancels pending requests (Monaco's `editor.cancelOperation`,
//! `contrib/editorState/browser/keybindingCancellation.ts`, MIT): the
//! requests Monaco runs with an `EditorKeybindingCancellationTokenSource`
//! (go to and peek, format, rename, paste and drop) register here, and
//! Escape cancels the newest one still pending.

use gpui::Context;

use crate::editor::CodeEditor;

/// A request Escape can cancel.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum CancellableOperation {
    /// A go to or peek command (Monaco `SymbolNavigationAction`).
    Navigation,
    /// Format document, selection or pasted text (Monaco
    /// `formatDocumentWithProvider` and `formatDocumentRangesWithProvider`).
    Format,
    /// Resolving the rename location or computing the edits (Monaco
    /// `RenameController.run`).
    Rename,
    /// Paste, "Paste As..." or drop providers (Monaco
    /// `CopyPasteController` and `DropIntoEditorController`).
    DropOrPaste,
}

/// The editor's cancellable requests, oldest first (Monaco's
/// `IEditorCancellationTokens`, a stack per editor).
#[derive(Default)]
pub(crate) struct CancellableOperations {
    stack: Vec<CancellableOperation>,
}

impl CodeEditor {
    /// Records that `operation` started a request, which Escape may now
    /// cancel (Monaco `EditorKeybindingCancellationTokenSource`).
    pub(crate) fn start_cancellable(&mut self, operation: CancellableOperation) {
        self.cancellable.stack.retain(|op| *op != operation);
        self.cancellable.stack.push(operation);
    }

    fn is_pending(&self, operation: CancellableOperation) -> bool {
        let task = match operation {
            CancellableOperation::Navigation => &self.goto.task,
            CancellableOperation::Format => &self.format_task,
            CancellableOperation::Rename => &self.rename.task,
            CancellableOperation::DropOrPaste => &self.drop_paste.task,
        };
        // Some requests keep their finished task around.
        task.as_ref().is_some_and(|task| !task.is_ready())
    }

    /// Whether a request that Escape cancels is pending (Monaco's
    /// `cancellableOperation` context key).
    pub fn has_cancellable_operation(&self) -> bool {
        self.cancellable
            .stack
            .iter()
            .any(|operation| self.is_pending(*operation))
    }

    /// Cancels the newest pending request that Escape can cancel (Monaco
    /// `editor.cancelOperation`): a go to or peek command, a format, a
    /// rename, or paste or drop providers. Returns whether there was one.
    pub fn cancel_operation(&mut self, cx: &mut Context<Self>) -> bool {
        // Finished requests leave the stack as Monaco's token sources are
        // disposed.
        let pending: Vec<_> = self
            .cancellable
            .stack
            .iter()
            .copied()
            .filter(|operation| self.is_pending(*operation))
            .collect();
        self.cancellable.stack = pending;
        let Some(operation) = self.cancellable.stack.pop() else {
            return false;
        };
        // Dropping a task cancels it.
        match operation {
            CancellableOperation::Navigation => self.goto.task = None,
            CancellableOperation::Format => self.format_task = None,
            CancellableOperation::Rename => self.rename.task = None,
            CancellableOperation::DropOrPaste => {
                self.drop_paste.task = None;
                // Their spinners go with them.
                self.clear_inline_progress(crate::inline_progress::ProgressKind::Paste, cx);
                self.clear_inline_progress(crate::inline_progress::ProgressKind::Drop, cx);
            }
        }
        cx.notify();
        true
    }
}
