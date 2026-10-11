//! [`TextModel`]: the document shared by any number of [`CodeEditor`]s.
//!
//! [`CodeEditor`]: crate::CodeEditor

use std::ops::{Range, RangeInclusive};
use std::sync::Arc;

use gpui::{App, Context, EntityId, EventEmitter, Global, SharedString, Task};

use crate::decorations::{Decoration, DecorationId, DecorationOptions, DecorationSet};
use crate::events::{ContentChange, content_changes};
use syntax::{HighlightState, Highlighter, Language, LanguageRegistry, TreeSitterHighlighter};
use text::{
    Buffer, BufferSnapshot, EditError, LineEnding, Patch, Rope, TransactionMetadata, UndoOutcome,
    Version,
};

/// The process-wide language registry used to resolve [`TextModel`] language
/// ids. Set it with [`set_language_registry`] (e.g. to
/// `gpui_monaco_languages::registry()`).
struct Languages(Arc<LanguageRegistry>);

impl Global for Languages {}

/// Sets the process-wide language registry that [`TextModel`] language ids are
/// resolved against (e.g. `gpui_monaco_languages::registry()`).
///
/// Models that already resolved their language keep it; set the registry before
/// showing any model.
pub fn set_language_registry(cx: &mut App, registry: Arc<LanguageRegistry>) {
    cx.set_global(Languages(registry));
}

/// The registry set with [`set_language_registry`], or an empty one.
pub fn language_registry(cx: &App) -> Arc<LanguageRegistry> {
    cx.try_global::<Languages>()
        .map(|languages| languages.0.clone())
        .unwrap_or_default()
}

/// Models by uri (Monaco `editor.getModel(uri)`), held weakly, plus the
/// host's resolver for documents that aren't open.
#[derive(Default)]
struct Models {
    by_uri: std::collections::HashMap<SharedString, gpui::WeakEntity<TextModel>>,
    resolver: Option<std::rc::Rc<ModelResolver>>,
}

type ModelResolver = dyn Fn(&str, &mut App) -> Option<gpui::Entity<TextModel>>;

impl Global for Models {}

/// Makes `model` findable by its uri with [`model_for_uri`] while it's
/// alive. Editors register their models; register others (e.g. files a
/// language server talks about) yourself.
pub fn register_model(cx: &mut App, model: &gpui::Entity<TextModel>) {
    if let Some(uri) = model.read(cx).uri().cloned() {
        let models = cx.default_global::<Models>();
        models.by_uri.retain(|_, model| model.upgrade().is_some());
        models.by_uri.insert(uri, model.downgrade());
    }
}

/// Supplies models for uris that aren't registered, e.g. by reading files,
/// so peek views can show other documents (Monaco's text model resolver).
pub fn set_model_resolver(
    cx: &mut App,
    resolver: impl Fn(&str, &mut App) -> Option<gpui::Entity<TextModel>> + 'static,
) {
    cx.default_global::<Models>().resolver = Some(std::rc::Rc::new(resolver));
}

/// Every live model registered with [`register_model`].
pub(crate) fn registered_models(cx: &App) -> Vec<gpui::Entity<TextModel>> {
    cx.try_global::<Models>()
        .map(|models| models.by_uri.values().filter_map(|m| m.upgrade()).collect())
        .unwrap_or_default()
}

/// The live model with `uri`, or one from the resolver (Monaco
/// `getModel`, then `createModelReference`).
pub fn model_for_uri(cx: &mut App, uri: &str) -> Option<gpui::Entity<TextModel>> {
    let models = cx.default_global::<Models>();
    if let Some(model) = models.by_uri.get(uri).and_then(|model| model.upgrade()) {
        return Some(model);
    }
    let resolver = models.resolver.clone()?;
    let model = resolver(uri, cx)?;
    register_model(cx, &model);
    Some(model)
}

/// The highlighter and the latest highlight state with the text it describes.
#[derive(Clone)]
pub struct SyntaxSnapshot {
    /// Computes highlights from `state`.
    pub highlighter: Arc<dyn Highlighter>,
    /// The parse state for `text`.
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
    /// Monaco `tabSize`: columns per tab character (default 4).
    pub tab_size: u32,
    /// Monaco `indentSize`: columns per indentation level (default 4).
    pub indent_size: u32,
    /// Monaco `insertSpaces`: indent with spaces instead of tabs (default `true`).
    pub insert_spaces: bool,
    /// Monaco `maxTokenizationLineLength`: longer lines aren't highlighted,
    /// and count as plain text for brackets and indentation (default
    /// 20 000 UTF-16 units).
    pub max_tokenization_line_length: u32,
    /// Monaco `detectIndentation`: when the model is created, `tab_size`,
    /// `indent_size` and `insert_spaces` are guessed from its text, with the
    /// values here as the defaults (default `true`).
    pub detect_indentation: bool,
    /// Monaco `trimAutoWhitespace`: indentation that Enter or Tab inserted
    /// is removed by the next edit if it is still all that's on its line
    /// and that edit is near the cursors (default `true`).
    pub trim_auto_whitespace: bool,
}

impl Default for ModelOptions {
    fn default() -> Self {
        Self {
            tab_size: 4,
            indent_size: 4,
            insert_spaces: true,
            max_tokenization_line_length: 20_000,
            detect_indentation: true,
            trim_auto_whitespace: true,
        }
    }
}

/// Why the text changed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EditKind {
    /// An edit through [`TextModel::edit`] or [`TextModel::edit_from`], including
    /// editing commands.
    Edit,
    /// [`TextModel::undo`].
    Undo,
    /// [`TextModel::redo`].
    Redo,
    /// [`TextModel::set_value`]; undo history was cleared.
    SetValue,
}

/// Events emitted by a [`TextModel`].
#[derive(Clone, Debug)]
pub enum TextModelEvent {
    /// The text changed from `version - 1` (or older, for `SetValue`) to
    /// `version`. `patch` is in byte offsets of the previous snapshot.
    Edited {
        /// The changes, in byte offsets of the previous text.
        patch: Patch,
        /// The changes with positions and inserted text (Monaco
        /// `IModelContentChangedEvent.changes`).
        changes: Arc<[ContentChange]>,
        /// The model's version after the change.
        version: Version,
        /// What caused the change.
        kind: EditKind,
        /// The editor that made the edit, which has already updated its own
        /// cursors. Other editors should transform theirs through `patch`.
        source: Option<EntityId>,
        /// Cursor state to restore for undo/redo (from the editor that made the
        /// original edit).
        restore: Option<TransactionMetadata>,
    },
    /// [`TextModel::update_options`] changed the options.
    OptionsChanged,
    /// [`TextModel::set_language`] changed the language id.
    LanguageChanged,
    /// [`TextModel::set_line_ending`] changed the line ending.
    LineEndingChanged,
    /// [`TextModel::set_markers`] changed the markers.
    MarkersChanged,
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
    /// The user chose to keep this model's unusual line terminators
    /// (Monaco's `ignoreUnusualLineTerminators` model property).
    pub(crate) ignore_unusual_line_terminators: bool,
    pub(crate) markers: crate::markers::MarkerSet,
    /// Set when `language_id` hasn't been resolved against the registry yet.
    language_pending: bool,
    auto_whitespace: AutoWhitespaceLines,
}

impl EventEmitter<TextModelEvent> for TextModel {}

impl TextModel {
    /// A model holding `text`, with default options and no language or uri. Line
    /// breaks are normalized to `\n` and the original line ending is detected
    /// (LF when there are none); a leading byte order mark is stripped.
    pub fn new(text: &str) -> Self {
        Self {
            buffer: Buffer::new(text),
            options: ModelOptions::default(),
            language_id: None,
            uri: None,
            syntax: None,
            decorations: DecorationSet::default(),
            ignore_unusual_line_terminators: false,
            markers: Default::default(),
            language_pending: false,
            auto_whitespace: AutoWhitespaceLines::default(),
        }
        .with_options(ModelOptions::default())
    }

    /// Sets the creation options (Monaco `createModel`'s options): with
    /// `detect_indentation`, the indentation is guessed from the text, with
    /// `options` as the defaults.
    pub fn with_options(mut self, options: ModelOptions) -> Self {
        self.options = options;
        if self.options.detect_indentation {
            let guessed = text::guess_indentation(
                self.buffer.rope(),
                self.options.tab_size,
                self.options.insert_spaces,
            );
            self.options.insert_spaces = guessed.insert_spaces;
            self.options.tab_size = guessed.tab_size;
            self.options.indent_size = guessed.tab_size;
        }
        self
    }

    /// Guesses the indentation from the text and applies it (Monaco
    /// `ITextModel.detectIndentation`), falling back to the given defaults.
    pub fn detect_indentation(
        &mut self,
        default_insert_spaces: bool,
        default_tab_size: u32,
        cx: &mut Context<Self>,
    ) {
        let guessed =
            text::guess_indentation(self.buffer.rope(), default_tab_size, default_insert_spaces);
        let options = ModelOptions {
            insert_spaces: guessed.insert_spaces,
            tab_size: guessed.tab_size,
            // Monaco: "guess indentSize independent of tabSize" is a TODO.
            indent_size: guessed.tab_size,
            ..self.options.clone()
        };
        self.update_options(options, cx);
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

    /// The uri set with [`TextModel::with_uri`].
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

    /// The underlying text buffer.
    pub fn buffer(&self) -> &Buffer {
        &self.buffer
    }

    /// An immutable snapshot of the current text.
    pub fn snapshot(&self) -> BufferSnapshot {
        self.buffer.snapshot()
    }

    /// The current version; it changes with every edit, undo and redo.
    pub fn version(&self) -> Version {
        self.buffer.version()
    }

    /// The text with the model's line ending (Monaco `getValue()`).
    pub fn text(&self) -> String {
        self.buffer.text()
    }

    /// The line ending [`TextModel::text`] joins lines with.
    pub fn line_ending(&self) -> LineEnding {
        self.buffer.line_ending()
    }

    /// Sets the line ending [`TextModel::text`] joins lines with (Monaco
    /// `setEOL`), emitting [`TextModelEvent::LineEndingChanged`] if it
    /// differs. The stored text is unchanged, so this isn't an undoable edit.
    pub fn set_line_ending(&mut self, line_ending: LineEnding, cx: &mut Context<Self>) {
        if self.buffer.line_ending() != line_ending {
            self.buffer.set_line_ending(line_ending);
            cx.emit(TextModelEvent::LineEndingChanged);
            cx.notify();
        }
    }

    /// The model options.
    pub fn options(&self) -> &ModelOptions {
        &self.options
    }

    /// Replaces the options, emitting [`TextModelEvent::OptionsChanged`] if they
    /// differ.
    pub fn update_options(&mut self, options: ModelOptions, cx: &mut Context<Self>) {
        if self.options != options {
            self.options = options;
            cx.emit(TextModelEvent::OptionsChanged);
            cx.notify();
        }
    }

    /// The language id set with [`TextModel::with_language`] or
    /// [`TextModel::set_language`].
    pub fn language_id(&self) -> Option<&SharedString> {
        self.language_id.as_ref()
    }

    /// The resolved language, if it has a registered definition.
    pub fn language(&self) -> Option<&Arc<Language>> {
        self.syntax.as_ref().map(|syntax| &syntax.language)
    }

    /// Changes the language id (Monaco `setModelLanguage`), resolving it right away
    /// and emitting [`TextModelEvent::LanguageChanged`] if it differs.
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
            .filter(|l| l.has_highlighter())
            .map(|language| Syntax {
                highlighter: match &language.highlighter {
                    Some(factory) => factory(&language, &registry),
                    None => Arc::new(TreeSitterHighlighter::new(language.clone(), registry)),
                },
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
        // Monaco leaves long lines untokenized.
        let row = self
            .buffer
            .rope()
            .offset_to_point(offset.min(self.buffer.len()))
            .row;
        if self.buffer.rope().line_len_utf16(row) >= self.options.max_tokenization_line_length {
            return None;
        }
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

    /// Byte ranges of strings and comments intersecting `range` (Monaco's
    /// tokens that bracket matching skips), from one highlights query.
    pub fn string_and_comment_ranges(&self, range: Range<usize>) -> Vec<Range<usize>> {
        let Some(syntax) = self.syntax_snapshot() else {
            return Vec::new();
        };
        let patch = (syntax.text.version() != self.buffer.version())
            .then(|| self.buffer.edits_since(syntax.text.version()));
        let old_range = match &patch {
            Some(patch) => {
                let inverse = patch.invert();
                inverse.transform_offset(range.start, text::Bias::Left)
                    ..inverse.transform_offset(range.end, text::Bias::Right)
            }
            None => range,
        };
        // Long lines are untokenized (Monaco `maxTokenizationLineLength`).
        tokenized_ranges(
            syntax.text.rope(),
            old_range,
            self.options.max_tokenization_line_length,
        )
        .into_iter()
        .flat_map(|old_range| {
            syntax
                .highlighter
                .highlights(&syntax.state, syntax.text.rope(), old_range)
        })
        .filter(|span| {
            let name = span.highlight.name();
            name.starts_with("comment")
                || name.starts_with("string")
                || name.starts_with("character")
        })
        .filter_map(|span| match &patch {
            Some(patch) => {
                let start = patch.transform_offset(span.range.start, text::Bias::Right);
                let end = patch.transform_offset(span.range.end, text::Bias::Left);
                (start < end).then_some(start..end)
            }
            None => Some(span.range),
        })
        .collect()
    }

    /// The language at byte `offset` (Monaco `getLanguageIdAtPosition`): the
    /// innermost injected language there (e.g. Rust in a Markdown code fence,
    /// JavaScript in an HTML `<script>`), or the model's language. Hidden
    /// helper languages such as `markdown_inline` are skipped.
    pub fn language_at(&self, offset: usize) -> Option<Arc<Language>> {
        let model_language = self.language().cloned();
        let Some(syntax) = self.syntax_snapshot() else {
            return model_language;
        };
        let Some(tree) = syntax.state.syntax() else {
            return model_language;
        };
        let offset = if syntax.text.version() == self.buffer.version() {
            offset
        } else {
            self.buffer
                .edits_since(syntax.text.version())
                .invert()
                .transform_offset(offset, text::Bias::Left)
        };
        tree.layers()
            .iter()
            .filter(|layer| {
                !layer.language.hidden
                    && layer
                        .ranges
                        .iter()
                        .any(|r| r.start <= offset && offset <= r.end)
            })
            .max_by_key(|layer| layer.depth)
            .map(|layer| layer.language.clone())
            .or(model_language)
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
        let old = self.buffer.rope().clone();
        let Ok(patch) = self.buffer.edit([(0..len, text)]) else {
            return;
        };
        self.auto_whitespace.clear();
        self.buffer.clear_history();
        self.decorations.apply_patch(&patch);
        self.reparse(cx);
        cx.emit(TextModelEvent::Edited {
            changes: content_changes(&old, self.buffer.rope(), &patch).into(),
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
    /// [`TextModel::push_stack_element`]. Like Monaco's `applyEdits`, this
    /// doesn't trim auto-inserted whitespace (see
    /// [`ModelOptions::trim_auto_whitespace`]); editing commands do.
    pub fn edit_from<S: AsRef<str>>(
        &mut self,
        edits: impl IntoIterator<Item = (Range<usize>, S)>,
        group: Option<u64>,
        source: Option<EntityId>,
        cx: &mut Context<Self>,
    ) -> Result<Patch, EditError> {
        let old = self.buffer.rope().clone();
        self.auto_whitespace.clear();
        let patch = match group {
            Some(group) => self.buffer.edit_grouped(edits, group)?,
            None => self.buffer.edit(edits)?,
        };
        self.emit_edit(&old, &patch, source, cx);
        Ok(patch)
    }

    /// Monaco `pushEditOperations`: like [`TextModel::edit_from`], but with
    /// [`ModelOptions::trim_auto_whitespace`] it also removes the whitespace
    /// the previous edit auto-inserted (see [`AutoWhitespaceLines::edit`]).
    /// `edits` are flagged when they insert auto whitespace, and
    /// `cursor_rows` are the rows each cursor's selection spans.
    pub(crate) fn push_edit_operations(
        &mut self,
        cursor_rows: Option<&[RangeInclusive<u32>]>,
        edits: Vec<(Range<usize>, String, bool)>,
        group: Option<u64>,
        source: Option<EntityId>,
        cx: &mut Context<Self>,
    ) -> Result<AppliedEdits, EditError> {
        let old = self.buffer.rope().clone();
        let applied = self.auto_whitespace.edit(
            &mut self.buffer,
            self.options.trim_auto_whitespace,
            cursor_rows,
            edits,
            group,
        )?;
        self.emit_edit(&old, &applied.patch, source, cx);
        Ok(applied)
    }

    fn emit_edit(
        &mut self,
        old: &Rope,
        patch: &Patch,
        source: Option<EntityId>,
        cx: &mut Context<Self>,
    ) {
        if !patch.is_empty() {
            self.decorations.apply_patch(patch);
            self.reparse(cx);
            cx.emit(TextModelEvent::Edited {
                changes: content_changes(old, self.buffer.rope(), patch).into(),
                patch: patch.clone(),
                version: self.buffer.version(),
                kind: EditKind::Edit,
                source,
                restore: None,
            });
            cx.notify();
        }
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

    /// Starts an explicit transaction: every edit until the matching
    /// [`TextModel::end_transaction`] becomes one undo step. Nests.
    pub fn start_transaction(&mut self) {
        self.buffer.start_transaction(None);
    }

    /// Ends the transaction [`TextModel::start_transaction`] started.
    pub fn end_transaction(&mut self) -> Option<text::TransactionId> {
        self.buffer.end_transaction(None)
    }

    /// The id of the undo step being recorded or the most recent one (ids
    /// increase), e.g. to tell whether an edit was undone.
    pub fn last_transaction_id(&self) -> Option<text::TransactionId> {
        self.buffer.current_transaction_id()
    }

    /// Replaces the options of decoration `id`, keeping its range and id
    /// (Monaco `changeDecorationOptions`), e.g. to change its stickiness.
    pub fn change_decoration_options(
        &mut self,
        id: DecorationId,
        options: DecorationOptions,
        cx: &mut Context<Self>,
    ) {
        if self.decorations.change_options(id, options) {
            cx.emit(TextModelEvent::DecorationsChanged);
            cx.notify();
        }
    }

    /// Decorations intersecting byte `range`, as seen by `editor`.
    pub fn decorations_in_range(
        &mut self,
        range: Range<usize>,
        editor: Option<EntityId>,
    ) -> Vec<Decoration> {
        self.decorations.in_range(range, editor)
    }

    /// The decorations' line heights (Monaco `lineHeight`), as seen by
    /// `editor`.
    pub fn decoration_line_heights(&self, editor: Option<EntityId>) -> Vec<(Range<usize>, f32)> {
        self.decorations.line_heights(editor)
    }

    /// All decorations visible to `editor`.
    pub fn all_decorations(&mut self, editor: Option<EntityId>) -> Vec<Decoration> {
        self.decorations.all(editor)
    }

    /// The decoration with `id`, if it still exists.
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

    /// Ends the current undo step, so the next grouped edit starts a new one (Monaco
    /// `pushStackElement`).
    pub fn push_stack_element(&mut self) {
        self.buffer.push_stack_element();
    }

    /// Whether there's an undo step to undo.
    pub fn can_undo(&self) -> bool {
        self.buffer.can_undo()
    }

    /// Whether there's an undone step to redo.
    pub fn can_redo(&self) -> bool {
        self.buffer.can_redo()
    }

    /// Undoes the last undo step on behalf of `source`, emitting
    /// [`TextModelEvent::Edited`] with [`EditKind::Undo`]. Returns whether anything
    /// was undone.
    pub fn undo(&mut self, source: Option<EntityId>, cx: &mut Context<Self>) -> bool {
        let old = self.buffer.rope().clone();
        let outcome = self.buffer.undo();
        self.emit_undo(&old, outcome, EditKind::Undo, source, cx)
    }

    /// Redoes the last undone step on behalf of `source`, emitting
    /// [`TextModelEvent::Edited`] with [`EditKind::Redo`]. Returns whether anything
    /// was redone.
    pub fn redo(&mut self, source: Option<EntityId>, cx: &mut Context<Self>) -> bool {
        let old = self.buffer.rope().clone();
        let outcome = self.buffer.redo();
        self.emit_undo(&old, outcome, EditKind::Redo, source, cx)
    }

    fn emit_undo(
        &mut self,
        old: &Rope,
        outcome: Option<UndoOutcome>,
        kind: EditKind,
        source: Option<EntityId>,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(outcome) = outcome else {
            return false;
        };
        self.auto_whitespace.clear();
        self.decorations.apply_patch(&outcome.patch);
        self.reparse(cx);
        cx.emit(TextModelEvent::Edited {
            changes: content_changes(old, self.buffer.rope(), &outcome.patch).into(),
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

/// Monaco `TextModel._trimAutoWhitespaceLines`: the rows whose content is
/// whitespace that the last edit auto-inserted, in descending order.
#[derive(Clone, Debug, Default)]
pub(crate) struct AutoWhitespaceLines(Option<Vec<u32>>);

impl AutoWhitespaceLines {
    /// Forgets the recorded rows, as any edit that doesn't record new ones
    /// does.
    pub(crate) fn clear(&mut self) {
        self.0 = None;
    }

    /// Applies `edits` (byte ranges in the current text, each flagged when it
    /// inserts auto whitespace) to `buffer` as one batch, like Monaco's
    /// `TextModel._pushEditOperations` followed by
    /// `PieceTreeTextBuffer.applyEdits`. With `trim`, it first adds
    /// deletions of the recorded rows that don't conflict with `edits`
    /// (unless some cursor in `cursor_rows` has no edit on its rows), then
    /// records the rows the flagged edits left holding only auto-inserted
    /// whitespace.
    pub(crate) fn edit(
        &mut self,
        buffer: &mut Buffer,
        trim: bool,
        cursor_rows: Option<&[RangeInclusive<u32>]>,
        edits: Vec<(Range<usize>, String, bool)>,
        group: Option<u64>,
    ) -> Result<AppliedEdits, EditError> {
        let old = buffer.rope().clone();
        let mut edits: Vec<(Range<usize>, String, bool)> = edits
            .into_iter()
            .map(|(range, text, auto)| {
                let text = text::normalize_line_endings(&text).into_owned();
                (
                    range.start.min(range.end)..range.end.max(range.start),
                    text,
                    auto,
                )
            })
            .collect();
        let incoming = edits.len();
        if let Some(rows) = self.0.take()
            && trim
        {
            for row in trim_rows(&old, &rows, cursor_rows, &edits) {
                edits.push((old.line_range(row), String::new(), false));
            }
        }
        let batch = edits.iter().map(|(range, text, _)| (range.clone(), text));
        let patch = match group {
            Some(group) => buffer.edit_grouped(batch, group)?,
            None => buffer.edit(batch)?,
        };
        let new = buffer.rope();

        // Monaco `_getInverseEditRanges`: where each edit's text ended up.
        let mut order: Vec<usize> = (0..edits.len()).collect();
        order.sort_by_key(|&i| (edits[i].0.start, edits[i].0.end));
        let mut inverse = vec![0..0; edits.len()];
        let mut delta = 0isize;
        for i in order {
            let (range, text, _) = &edits[i];
            let start = range.start.saturating_add_signed(delta);
            inverse[i] = start..start + text.len();
            delta += text.len() as isize - range.len() as isize;
        }

        let trimmed = (incoming..edits.len())
            .map(|i| {
                let row = new.offset_to_point(inverse[i].start).row;
                let len = old
                    .text_for_range(edits[i].0.clone())
                    .encode_utf16()
                    .count();
                (row, len as u32)
            })
            .collect();

        if trim {
            let mut candidates: Vec<(u32, String)> = Vec::new();
            for (i, (range, _, auto)) in edits.iter().enumerate() {
                if !*auto || !range.is_empty() {
                    continue;
                }
                let first = new.offset_to_point(inverse[i].start).row;
                let last = new.offset_to_point(inverse[i].end).row;
                for row in first..=last {
                    let mut old_content = String::new();
                    if row == first {
                        old_content = old.line(old.offset_to_point(range.start).row);
                        if !is_blank(&old_content) {
                            continue;
                        }
                    }
                    candidates.push((row, old_content));
                }
            }
            candidates.sort_by_key(|(row, _)| std::cmp::Reverse(*row));
            candidates.dedup_by_key(|(row, _)| *row);
            let rows: Vec<u32> = candidates
                .into_iter()
                .filter(|(row, old_content)| {
                    let content = new.line(*row);
                    !content.is_empty() && content != *old_content && is_blank(&content)
                })
                .map(|(row, _)| row)
                .collect();
            self.0 = (!rows.is_empty()).then_some(rows);
        }
        inverse.truncate(incoming);
        Ok(AppliedEdits {
            patch,
            ranges: inverse,
            trimmed,
        })
    }
}

/// What [`AutoWhitespaceLines::edit`] did.
pub(crate) struct AppliedEdits {
    pub patch: Patch,
    /// Where the text of each given edit ended up, in byte offsets of the new
    /// text (Monaco's inverse edit ranges).
    pub ranges: Vec<Range<usize>>,
    /// Each trimmed row, in the new text, with the UTF-16 length of the
    /// whitespace removed from its start.
    pub trimmed: Vec<(u32, u32)>,
}

/// The recorded `rows` that Monaco's `_pushEditOperations` trims before
/// applying `edits`: none unless every cursor has an edit on its rows, and
/// only rows that no edit touches except by inserting a line break right
/// after or before them.
fn trim_rows(
    rope: &Rope,
    rows: &[u32],
    cursor_rows: Option<&[RangeInclusive<u32>]>,
    edits: &[(Range<usize>, String, bool)],
) -> Vec<u32> {
    let incoming: Vec<(text::Point, text::Point, &str)> = edits
        .iter()
        .map(|(range, text, _)| {
            (
                rope.offset_to_point(range.start),
                rope.offset_to_point(range.end),
                text.as_str(),
            )
        })
        .collect();
    // Auto-formatters can edit away from the cursors, which shouldn't trim.
    let near_cursors = cursor_rows.unwrap_or_default().iter().all(|rows| {
        incoming
            .iter()
            .any(|(start, end, _)| start.row <= *rows.end() && *rows.start() <= end.row)
    });
    if !near_cursors {
        return Vec::new();
    }
    rows.iter()
        .copied()
        .filter(|&row| row < rope.line_count())
        .filter(|&row| {
            let max_column = rope.line_len(row);
            incoming.iter().all(|&(start, end, text)| {
                if row < start.row || row > end.row {
                    return true;
                }
                start == end
                    && start.row == row
                    && ((start.column == max_column && text.starts_with('\n'))
                        || (start.column == 0 && text.ends_with('\n')))
            })
        })
        .collect()
}

/// Monaco `firstNonWhitespaceIndex(line) === -1`.
fn is_blank(line: &str) -> bool {
    line.chars().all(|c| c == ' ' || c == '\t')
}

/// Moves `selections` (in the text after an edit) left past the whitespace
/// [`AutoWhitespaceLines::edit`] trimmed from the start of their rows, as
/// Monaco's cursor state computation sees the trimming.
pub(crate) fn shift_past_trimmed(
    selections: &mut [crate::cursor::Selection],
    trimmed: &[(u32, u32)],
) {
    for &(row, len) in trimmed {
        for selection in selections.iter_mut() {
            for p in [&mut selection.anchor, &mut selection.head] {
                if p.row == row {
                    p.column = p.column.saturating_sub(len);
                }
            }
        }
    }
}

/// Wraps a value as undo metadata.
pub(crate) fn metadata<T: std::any::Any + Send + Sync>(value: T) -> TransactionMetadata {
    Arc::new(value)
}

/// The parts of `range` on lines shorter than `max_line_length`.
pub(crate) fn tokenized_ranges(
    rope: &Rope,
    range: Range<usize>,
    max_line_length: u32,
) -> Vec<Range<usize>> {
    let range = range.start.min(rope.len())..range.end.min(rope.len());
    let (first, last) = (
        rope.offset_to_point(range.start).row,
        rope.offset_to_point(range.end).row,
    );
    let mut ranges: Vec<Range<usize>> = Vec::new();
    for row in first..=last {
        if rope.line_len_utf16(row) >= max_line_length {
            continue;
        }
        let start = rope
            .point_to_offset(text::Point::new(row, 0))
            .max(range.start);
        let end = if row + 1 < rope.line_count() {
            rope.point_to_offset(text::Point::new(row + 1, 0))
        } else {
            rope.len()
        }
        .min(range.end);
        match ranges.last_mut() {
            Some(last) if last.end == start => last.end = end,
            _ => ranges.push(start..end),
        }
    }
    ranges
}
