//! [`TextModel`]: the document shared by any number of [`CodeEditor`]s.
//!
//! [`CodeEditor`]: crate::CodeEditor

use std::ops::Range;
use std::sync::Arc;

use gpui::{Context, EntityId, EventEmitter, SharedString};
use text::{
    Buffer, BufferSnapshot, EditError, LineEnding, Patch, TransactionMetadata, UndoOutcome, Version,
};

/// Per-model options (Monaco `ITextModelUpdateOptions`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ModelOptions {
    pub tab_size: u32,
    pub indent_size: u32,
    pub insert_spaces: bool,
}

impl Default for ModelOptions {
    fn default() -> Self {
        Self {
            tab_size: 4,
            indent_size: 4,
            insert_spaces: true,
        }
    }
}

/// Why the text changed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EditKind {
    Edit,
    Undo,
    Redo,
    SetValue,
}

#[derive(Clone, Debug)]
pub enum TextModelEvent {
    /// The text changed from `version - 1` (or older, for `SetValue`) to
    /// `version`. `patch` is in byte offsets of the previous snapshot.
    Edited {
        patch: Patch,
        version: Version,
        kind: EditKind,
        /// The editor that made the edit, which has already updated its own
        /// cursors. Other editors should transform theirs through `patch`.
        source: Option<EntityId>,
        /// Cursor state to restore for undo/redo (from the editor that made the
        /// original edit).
        restore: Option<TransactionMetadata>,
    },
    OptionsChanged,
    LanguageChanged,
}

/// A text document: buffer, undo history, language and options.
pub struct TextModel {
    buffer: Buffer,
    options: ModelOptions,
    language_id: Option<SharedString>,
}

impl EventEmitter<TextModelEvent> for TextModel {}

impl TextModel {
    pub fn new(text: &str) -> Self {
        Self {
            buffer: Buffer::new(text),
            options: ModelOptions::default(),
            language_id: None,
        }
    }

    pub fn with_language(mut self, language_id: impl Into<SharedString>) -> Self {
        self.language_id = Some(language_id.into());
        self
    }

    pub fn buffer(&self) -> &Buffer {
        &self.buffer
    }

    pub fn snapshot(&self) -> BufferSnapshot {
        self.buffer.snapshot()
    }

    pub fn version(&self) -> Version {
        self.buffer.version()
    }

    /// The text with the model's line ending (Monaco `getValue()`).
    pub fn text(&self) -> String {
        self.buffer.text()
    }

    pub fn line_ending(&self) -> LineEnding {
        self.buffer.line_ending()
    }

    pub fn set_line_ending(&mut self, line_ending: LineEnding) {
        self.buffer.set_line_ending(line_ending);
    }

    pub fn options(&self) -> &ModelOptions {
        &self.options
    }

    pub fn update_options(&mut self, options: ModelOptions, cx: &mut Context<Self>) {
        if self.options != options {
            self.options = options;
            cx.emit(TextModelEvent::OptionsChanged);
            cx.notify();
        }
    }

    pub fn language_id(&self) -> Option<&SharedString> {
        self.language_id.as_ref()
    }

    pub fn set_language(&mut self, language_id: Option<SharedString>, cx: &mut Context<Self>) {
        if self.language_id != language_id {
            self.language_id = language_id;
            cx.emit(TextModelEvent::LanguageChanged);
            cx.notify();
        }
    }

    /// Replaces the whole text and clears undo history (Monaco `setValue`).
    pub fn set_value(&mut self, text: &str, cx: &mut Context<Self>) {
        let old_len = self.buffer.len();
        let line_ending = self.buffer.line_ending();
        self.buffer = Buffer::with_default_line_ending(text, line_ending);
        let new_len = self.buffer.len();
        cx.emit(TextModelEvent::Edited {
            patch: Patch::new(vec![text::Edit {
                old: 0..old_len,
                new: 0..new_len,
            }]),
            version: self.buffer.version(),
            kind: EditKind::SetValue,
            source: None,
            restore: None,
        });
        cx.notify();
    }

    /// Applies `edits` (byte ranges in the current text) as one undo step.
    pub fn edit<S: AsRef<str>>(
        &mut self,
        edits: impl IntoIterator<Item = (Range<usize>, S)>,
        cx: &mut Context<Self>,
    ) -> Result<Patch, EditError> {
        self.edit_from(edits, None, None, cx)
    }

    /// Applies `edits` on behalf of `source`. With a `group`, consecutive
    /// edits with the same group merge into one undo step until
    /// [`TextModel::push_stack_element`].
    pub fn edit_from<S: AsRef<str>>(
        &mut self,
        edits: impl IntoIterator<Item = (Range<usize>, S)>,
        group: Option<u64>,
        source: Option<EntityId>,
        cx: &mut Context<Self>,
    ) -> Result<Patch, EditError> {
        let patch = match group {
            Some(group) => self.buffer.edit_grouped(edits, group)?,
            None => self.buffer.edit(edits)?,
        };
        if !patch.is_empty() {
            cx.emit(TextModelEvent::Edited {
                patch: patch.clone(),
                version: self.buffer.version(),
                kind: EditKind::Edit,
                source,
                restore: None,
            });
            cx.notify();
        }
        Ok(patch)
    }

    /// Records cursor state with the most recent undo step.
    pub fn set_undo_metadata(
        &mut self,
        before: Option<TransactionMetadata>,
        after: Option<TransactionMetadata>,
    ) {
        self.buffer.set_last_transaction_metadata(before, after);
    }

    pub fn push_stack_element(&mut self) {
        self.buffer.push_stack_element();
    }

    pub fn can_undo(&self) -> bool {
        self.buffer.can_undo()
    }

    pub fn can_redo(&self) -> bool {
        self.buffer.can_redo()
    }

    pub fn undo(&mut self, source: Option<EntityId>, cx: &mut Context<Self>) -> bool {
        let outcome = self.buffer.undo();
        self.emit_undo(outcome, EditKind::Undo, source, cx)
    }

    pub fn redo(&mut self, source: Option<EntityId>, cx: &mut Context<Self>) -> bool {
        let outcome = self.buffer.redo();
        self.emit_undo(outcome, EditKind::Redo, source, cx)
    }

    fn emit_undo(
        &mut self,
        outcome: Option<UndoOutcome>,
        kind: EditKind,
        source: Option<EntityId>,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(outcome) = outcome else {
            return false;
        };
        cx.emit(TextModelEvent::Edited {
            patch: outcome.patch,
            version: self.buffer.version(),
            kind,
            source,
            restore: outcome.metadata,
        });
        cx.notify();
        true
    }
}

/// Wraps a value as undo metadata.
pub(crate) fn metadata<T: std::any::Any + Send + Sync>(value: T) -> TransactionMetadata {
    Arc::new(value)
}
