//! [`TextModel`]: the document shared by any number of [`CodeEditor`]s.
//!
//! [`CodeEditor`]: crate::CodeEditor

use std::ops::Range;
use std::sync::Arc;

use gpui::{App, Context, EntityId, EventEmitter, Global, SharedString, Task};

use crate::decorations::{Decoration, DecorationId, DecorationOptions, DecorationSet};
use syntax::{HighlightState, Highlighter, Language, LanguageRegistry, TreeSitterHighlighter};
use text::{
    Buffer, BufferSnapshot, EditError, LineEnding, Patch, TransactionMetadata, UndoOutcome, Version,
};

/// The process-wide language registry used to resolve [`TextModel`] language
/// ids. Set it with [`set_language_registry`] (e.g. to
/// `gpui_monaco_languages::registry()`).
struct Languages(Arc<LanguageRegistry>);

impl Global for Languages {}

pub fn set_language_registry(cx: &mut App, registry: Arc<LanguageRegistry>) {
    cx.set_global(Languages(registry));
}

pub fn language_registry(cx: &App) -> Arc<LanguageRegistry> {
    cx.try_global::<Languages>()
        .map(|languages| languages.0.clone())
        .unwrap_or_default()
}

/// The highlighter and the latest highlight state with the text it describes.
#[derive(Clone)]
pub struct SyntaxSnapshot {
    pub highlighter: Arc<dyn Highlighter>,
    pub state: HighlightState,
    /// The text `state` was computed from; may be older than the model.
    pub text: BufferSnapshot,
}

struct Syntax {
    language: Arc<Language>,
    highlighter: Arc<dyn Highlighter>,
    latest: Option<(HighlightState, BufferSnapshot)>,
    task: Option<Task<()>>,
    dirty: bool,
}

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
    /// New highlights are available (background parse finished).
    SyntaxUpdated,
    /// Decorations were added, removed or changed.
    DecorationsChanged,
}

/// A text document: buffer, undo history, language and options.
pub struct TextModel {
    buffer: Buffer,
    options: ModelOptions,
    language_id: Option<SharedString>,
    uri: Option<SharedString>,
    syntax: Option<Syntax>,
    decorations: DecorationSet,
    /// Set when `language_id` hasn't been resolved against the registry yet.
    language_pending: bool,
}

impl EventEmitter<TextModelEvent> for TextModel {}

impl TextModel {
    pub fn new(text: &str) -> Self {
        Self {
            buffer: Buffer::new(text),
            options: ModelOptions::default(),
            language_id: None,
            uri: None,
            syntax: None,
            decorations: DecorationSet::default(),
            language_pending: false,
        }
    }

    /// Sets the language id, resolved against the registry the first time the
    /// model is shown (see [`TextModel::ensure_syntax`]).
    pub fn with_language(mut self, language_id: impl Into<SharedString>) -> Self {
        self.language_id = Some(language_id.into());
        self.language_pending = true;
        self
    }

    /// Identifies the document for providers (Monaco model uris).
    pub fn with_uri(mut self, uri: impl Into<SharedString>) -> Self {
        self.uri = Some(uri.into());
        self
    }

    pub fn uri(&self) -> Option<&SharedString> {
        self.uri.as_ref()
    }

    /// What language feature providers see about this model.
    pub fn provider_context(&self) -> crate::providers::ProviderContext {
        crate::providers::ProviderContext {
            uri: self.uri.clone(),
            language_id: self.language_id.clone(),
            snapshot: self.buffer.snapshot(),
        }
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

    /// The resolved language, if it has a registered definition.
    pub fn language(&self) -> Option<&Arc<Language>> {
        self.syntax.as_ref().map(|syntax| &syntax.language)
    }

    pub fn set_language(&mut self, language_id: Option<SharedString>, cx: &mut Context<Self>) {
        if self.language_id != language_id {
            self.language_id = language_id;
            self.language_pending = true;
            self.ensure_syntax(cx);
            cx.emit(TextModelEvent::LanguageChanged);
            cx.notify();
        }
    }

    /// Resolves a pending language id against the global registry and starts
    /// parsing. Editors call this when they start showing the model.
    pub fn ensure_syntax(&mut self, cx: &mut Context<Self>) {
        if !self.language_pending {
            return;
        }
        self.language_pending = false;
        let registry = language_registry(cx);
        let language = self.language_id.as_ref().and_then(|id| {
            registry
                .language_for_id(id)
                .or_else(|| registry.language_for_name(id))
        });
        self.syntax = language
            .filter(|l| l.grammar.is_some())
            .map(|language| Syntax {
                highlighter: Arc::new(TreeSitterHighlighter::new(language.clone(), registry)),
                language,
                latest: None,
                task: None,
                dirty: false,
            });
        self.reparse(cx);
    }

    /// The latest highlight state, if the model has a grammar and a parse has
    /// finished. Its text may lag behind the model while a reparse runs.
    pub fn syntax_snapshot(&self) -> Option<SyntaxSnapshot> {
        let syntax = self.syntax.as_ref()?;
        let (state, text) = syntax.latest.clone()?;
        Some(SyntaxSnapshot {
            highlighter: syntax.highlighter.clone(),
            state,
            text,
        })
    }

    /// The innermost highlight capture name at byte `offset` of the current
    /// text (e.g. `string`, `comment.documentation`), using the latest syntax
    /// state mapped through any pending edits.
    pub fn highlight_name_at(&self, offset: usize) -> Option<Arc<str>> {
        let syntax = self.syntax_snapshot()?;
        let offset = if syntax.text.version() == self.buffer.version() {
            offset
        } else {
            self.buffer
                .edits_since(syntax.text.version())
                .invert()
                .transform_offset(offset, text::Bias::Left)
        };
        let end = (offset + 1).min(syntax.text.len());
        syntax
            .highlighter
            .highlights(&syntax.state, syntax.text.rope(), offset..end)
            .into_iter()
            .find(|span| span.range.start <= offset && offset < span.range.end)
            .map(|span| span.highlight.name())
    }

    /// Monaco's standard token type of the character at byte `offset`.
    pub fn token_type_at(&self, offset: usize) -> crate::cursor::StandardTokenType {
        use crate::cursor::StandardTokenType;
        match self.highlight_name_at(offset) {
            Some(name) if name.starts_with("comment") => StandardTokenType::Comment,
            Some(name) if name.contains("regex") || name.contains("regexp") => {
                StandardTokenType::RegEx
            }
            Some(name) if name.starts_with("string") || name.starts_with("character") => {
                StandardTokenType::String
            }
            _ => StandardTokenType::Other,
        }
    }

    /// Whether a background parse is in flight.
    pub fn is_parsing(&self) -> bool {
        self.syntax
            .as_ref()
            .is_some_and(|syntax| syntax.task.is_some())
    }

    /// Starts (or queues) a background parse of the current text.
    fn reparse(&mut self, cx: &mut Context<Self>) {
        let Some(syntax) = self.syntax.as_mut() else {
            return;
        };
        if syntax.task.is_some() {
            syntax.dirty = true;
            return;
        }
        let snapshot = self.buffer.snapshot();
        let highlighter = syntax.highlighter.clone();
        let previous = syntax.latest.clone().map(|(state, text)| {
            let patch = self.buffer.edits_since(text.version());
            (state, text, patch)
        });
        syntax.task = Some(cx.spawn(async move |this, cx| {
            let parse_snapshot = snapshot.clone();
            let state = cx
                .background_executor()
                .spawn(async move {
                    let previous = previous
                        .as_ref()
                        .map(|(state, text, patch)| (state, text.rope(), patch));
                    highlighter.update(previous, &parse_snapshot)
                })
                .await;
            this.update(cx, |this, cx| {
                let Some(syntax) = this.syntax.as_mut() else {
                    return;
                };
                syntax.latest = Some((state, snapshot));
                syntax.task = None;
                let dirty = std::mem::take(&mut syntax.dirty);
                cx.emit(TextModelEvent::SyntaxUpdated);
                cx.notify();
                if dirty {
                    this.reparse(cx);
                }
            })
            .ok();
        }));
    }

    /// Replaces the whole text and clears undo history (Monaco `setValue`).
    /// It's a whole-text edit, so versions keep increasing and anchors,
    /// decorations and syntax update normally.
    pub fn set_value(&mut self, text: &str, cx: &mut Context<Self>) {
        let text = text.strip_prefix('\u{feff}').unwrap_or(text);
        let len = self.buffer.len();
        let Ok(patch) = self.buffer.edit([(0..len, text)]) else {
            return;
        };
        self.buffer.clear_history();
        self.decorations.apply_patch(&patch);
        self.reparse(cx);
        cx.emit(TextModelEvent::Edited {
            patch,
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
            self.decorations.apply_patch(&patch);
            self.reparse(cx);
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

    /// Replaces decorations `old` with `new` (byte ranges), returning the new
    /// ids (Monaco `deltaDecorations`). `owner` restricts them to one editor.
    pub fn delta_decorations(
        &mut self,
        owner: Option<EntityId>,
        old: &[DecorationId],
        new: Vec<(Range<usize>, DecorationOptions)>,
        cx: &mut Context<Self>,
    ) -> Vec<DecorationId> {
        let len = self.buffer.len();
        let new = new.into_iter().map(|(range, options)| {
            let start = self
                .buffer
                .clip_offset(range.start.min(len), text::Bias::Left);
            let end = self
                .buffer
                .clip_offset(range.end.min(len), text::Bias::Right);
            (start..end, options)
        });
        let ids = self.decorations.delta(owner, old, new);
        cx.emit(TextModelEvent::DecorationsChanged);
        cx.notify();
        ids
    }

    /// Decorations intersecting byte `range`, as seen by `editor`.
    pub fn decorations_in_range(
        &mut self,
        range: Range<usize>,
        editor: Option<EntityId>,
    ) -> Vec<Decoration> {
        self.decorations.in_range(range, editor)
    }

    /// All decorations visible to `editor`.
    pub fn all_decorations(&mut self, editor: Option<EntityId>) -> Vec<Decoration> {
        self.decorations.all(editor)
    }

    pub fn decoration(&self, id: DecorationId) -> Option<&Decoration> {
        self.decorations.get(id)
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
        self.decorations.apply_patch(&outcome.patch);
        self.reparse(cx);
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
