//! Monaco's context keys (`EditorContextKeys` and the contributions' keys,
//! such as `suggestWidgetVisible` or `inSnippetMode`) as GPUI key context
//! entries, and the host's own keys (Monaco `createContextKey`).
//!
//! The editor sets these on its [`KEY_CONTEXT`] element, so a host's
//! [`KeybindingRule::when`](crate::KeybindingRule::when) can test them with
//! GPUI's predicate syntax: `CodeEditor && editorTextFocus &&
//! !editorReadonly`, `editorLangId == rust`. Boolean keys are present when
//! true; string and number keys are `key = value` entries (GPUI compares
//! them as identifiers, so values with other characters, such as
//! `supportedCodeAction`'s dotted kinds, can only be read with
//! [`CodeEditor::context_keys`]). The built-in actions' preconditions are
//! predicates over the same keys.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use gpui::{
    App, Context, KeyBindingContextPredicate, KeyContext, SharedString, WeakEntity, Window,
};

use crate::actions::KEY_CONTEXT;
use crate::editor::CodeEditor;
use crate::providers::{self, CodeActionKind};

/// A context key's value (Monaco `ContextKeyValue`).
#[derive(Clone, Debug, PartialEq)]
pub enum ContextKeyValue {
    /// A flag: the key is set while it's true.
    Bool(bool),
    /// A string: `key = value` while it isn't empty.
    String(SharedString),
    /// A number: `key = value`.
    Number(f64),
}

impl From<bool> for ContextKeyValue {
    fn from(value: bool) -> Self {
        Self::Bool(value)
    }
}

impl From<&str> for ContextKeyValue {
    fn from(value: &str) -> Self {
        Self::String(value.to_string().into())
    }
}

impl From<String> for ContextKeyValue {
    fn from(value: String) -> Self {
        Self::String(value.into())
    }
}

impl From<SharedString> for ContextKeyValue {
    fn from(value: SharedString) -> Self {
        Self::String(value)
    }
}

impl From<f64> for ContextKeyValue {
    fn from(value: f64) -> Self {
        Self::Number(value)
    }
}

impl From<i32> for ContextKeyValue {
    fn from(value: i32) -> Self {
        Self::Number(value.into())
    }
}

impl ContextKeyValue {
    pub(crate) fn apply(&self, key: &SharedString, context: &mut KeyContext) {
        match self {
            Self::Bool(true) => context.add(key.clone()),
            Self::Bool(false) => {}
            Self::String(value) if value.is_empty() => {}
            Self::String(value) => context.set(key.clone(), value.clone()),
            Self::Number(value) => context.set(key.clone(), value.to_string()),
        }
    }
}

/// A key the host created on an editor (Monaco `IContextKey`, from
/// [`CodeEditor::create_context_key`]). Setting it re-renders the editor,
/// so keybindings whose `when` tests it follow.
#[derive(Clone, Debug)]
pub struct ContextKey {
    editor: WeakEntity<CodeEditor>,
    key: SharedString,
    default: ContextKeyValue,
}

impl ContextKey {
    /// The key's name.
    pub fn key(&self) -> &SharedString {
        &self.key
    }

    /// Sets the value (Monaco `set`).
    pub fn set(&self, value: impl Into<ContextKeyValue>, cx: &mut App) {
        let value = value.into();
        self.editor
            .update(cx, |editor, cx| {
                editor.commands.host_keys.insert(self.key.clone(), value);
                cx.notify();
            })
            .ok();
    }

    /// Goes back to the default value (Monaco `reset`).
    pub fn reset(&self, cx: &mut App) {
        self.set(self.default.clone(), cx);
    }

    /// The value (Monaco `get`); `None` once the editor is gone.
    pub fn get(&self, cx: &App) -> Option<ContextKeyValue> {
        let editor = self.editor.upgrade()?;
        editor.read(cx).commands.host_keys.get(&self.key).cloned()
    }
}

/// What the editor's context keys need a window for, noted each frame.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct FocusKeys {
    /// Monaco `editorTextFocus`.
    pub(crate) text: bool,
    /// Monaco `editorFocus`: the text or one of the editor's widgets.
    pub(crate) widget: bool,
    pub(crate) hover: bool,
    pub(crate) color_picker: bool,
    pub(crate) rename_input: bool,
    pub(crate) find_input: bool,
    pub(crate) replace_input: bool,
    pub(crate) find_widget: bool,
    pub(crate) accessibility: bool,
}

thread_local! {
    /// Preconditions parsed once.
    static PREDICATES: RefCell<HashMap<&'static str, Rc<KeyBindingContextPredicate>>> =
        RefCell::new(HashMap::new());
}

/// `when`, parsed (preconditions are static and valid; the tests check).
pub(crate) fn predicate(when: &'static str) -> Rc<KeyBindingContextPredicate> {
    PREDICATES.with(|predicates| {
        predicates
            .borrow_mut()
            .entry(when)
            .or_insert_with(|| {
                Rc::new(
                    KeyBindingContextPredicate::parse(when)
                        .unwrap_or_else(|err| panic!("bad precondition `{when}`: {err}")),
                )
            })
            .clone()
    })
}

/// Whether the code action kinds `supported` include `kind` (Monaco
/// `contextKeyForSupportedActions`: a provided kind that is `kind`, or
/// starts with it at a word boundary).
fn supports_kind(supported: &[CodeActionKind], kind: &str) -> bool {
    supported.iter().any(|provided| {
        provided.0.strip_prefix(kind).is_some_and(|rest| {
            rest.chars()
                .next()
                .is_none_or(|c| !(c.is_alphanumeric() || c == '_'))
        })
    })
}

impl CodeEditor {
    /// The editor's context keys as GPUI sets them on its element (Monaco's
    /// context key service for this editor): the [`KEY_CONTEXT`]
    /// identifier, Monaco's keys (`editorTextFocus`, `editorReadonly`,
    /// `editorLangId`, `editorHasDefinitionProvider`, `suggestWidgetVisible`,
    /// `inSnippetMode`, `findWidgetVisible`, …), an enclosing diff editor's
    /// keys (see [`DiffEditor::context_keys`](crate::DiffEditor::context_keys)),
    /// the keys the host made with [`CodeEditor::create_context_key`], and
    /// the editor's own snake_case keys (`suggest_widget_visible`,
    /// `single_line`, …). Focus keys are as of the last frame.
    pub fn context_keys(&self, cx: &App) -> KeyContext {
        let mut context = KeyContext::new_with_defaults();
        context.add(KEY_CONTEXT);
        self.add_context_keys(&mut context, cx);
        context
    }

    /// Whether `when` (a precondition over [`CodeEditor::context_keys`])
    /// holds.
    pub(crate) fn when(&self, when: &'static str, cx: &App) -> bool {
        when.is_empty() || predicate(when).eval(&[self.context_keys(cx)])
    }

    /// Notes what the context keys need a window for (each frame).
    pub(crate) fn note_focus_keys(&mut self, window: &Window, cx: &App) {
        let (find_input, replace_input, find_widget) = self.find_widget_focus(window, cx);
        self.commands.focus = FocusKeys {
            text: self.has_focus,
            widget: self.focus_handle.contains_focused(window, cx),
            hover: self.is_hover_focused(window, cx),
            color_picker: self.is_standalone_color_picker_focused(window, cx),
            rename_input: self.is_rename_input_focused(window, cx),
            find_input,
            replace_input,
            find_widget,
            accessibility: self.screen_reader_optimized(window),
        };
    }

    /// Adds the context keys to `context`.
    pub(crate) fn add_context_keys(&self, context: &mut KeyContext, cx: &App) {
        let focus = self.commands.focus;
        let model = self.model().read(cx);
        let target = self.provider_target(cx);
        let selections = self.cursors.selections();

        // Monaco `EditorContextKeys`.
        flag(context, "editorTextFocus", self.has_focus);
        flag(context, "textInputFocus", self.has_focus);
        flag(context, "editorFocus", self.has_focus || focus.widget);
        flag(context, "editorReadonly", self.options.read_only);
        flag(context, "editorSimpleInput", self.widget_input);
        flag(
            context,
            "editorColumnSelection",
            self.options.column_selection,
        );
        flag(
            context,
            "editorHasSelection",
            selections.iter().any(|s| !s.is_empty()),
        );
        flag(context, "editorHasMultipleSelections", selections.len() > 1);
        flag(context, "editorTabMovesFocus", self.tab_moves_focus(cx));
        flag(context, "isInEmbeddedEditor", self.in_peek);
        flag(context, "canUndo", model.can_undo());
        flag(context, "canRedo", model.can_redo());
        flag(context, "editorHoverVisible", self.is_hover_visible());
        flag(context, "editorHoverFocused", focus.hover);
        flag(
            context,
            "stickyScrollFocused",
            self.sticky.focused.is_some(),
        );
        flag(
            context,
            "stickyScrollVisible",
            self.options.sticky_scroll.enabled
                && self
                    .layout
                    .as_ref()
                    .is_some_and(|l| !l.sticky_lines.is_empty()),
        );
        flag(
            context,
            "standaloneColorPickerVisible",
            self.is_standalone_color_picker_visible(),
        );
        flag(context, "standaloneColorPickerFocused", focus.color_picker);
        flag(
            context,
            "isComposing",
            self.marked_range.is_some() || self.commands.composing,
        );
        flag(context, "accessibilityModeEnabled", focus.accessibility);
        if let Some(language) = model.language_id() {
            context.set("editorLangId", language.clone());
        }

        // Monaco `editorHas*Provider`, for the model's language and uri.
        // Monaco registers its word-based completions and textual
        // highlights for every language.
        has(context, "editorHasCompletionItemProvider", 1);
        has(context, "editorHasDocumentHighlightProvider", 1);
        has(
            context,
            "editorHasCodeActionsProvider",
            providers::code_action_providers(cx, target.clone()).len(),
        );
        has(
            context,
            "editorHasCodeLensProvider",
            providers::code_lens_providers(cx, target.clone()).len(),
        );
        has(
            context,
            "editorHasDefinitionProvider",
            providers::definition_providers(cx, target.clone()).len(),
        );
        has(
            context,
            "editorHasDeclarationProvider",
            providers::declaration_providers(cx, target.clone()).len(),
        );
        has(
            context,
            "editorHasImplementationProvider",
            providers::implementation_providers(cx, target.clone()).len(),
        );
        has(
            context,
            "editorHasTypeDefinitionProvider",
            providers::type_definition_providers(cx, target.clone()).len(),
        );
        has(
            context,
            "editorHasHoverProvider",
            providers::hover_providers(cx, target.clone()).len(),
        );
        // The `tags.scm` outline stands in for a provider.
        let tags = model
            .language()
            .and_then(|l| l.grammar.as_ref())
            .is_some_and(|g| g.tags_query().is_some());
        has(
            context,
            "editorHasDocumentSymbolProvider",
            providers::document_symbol_providers(cx, target.clone()).len() + usize::from(tags),
        );
        has(
            context,
            "editorHasReferenceProvider",
            providers::reference_providers(cx, target.clone()).len(),
        );
        has(
            context,
            "editorHasRenameProvider",
            providers::rename_providers(cx, target.clone()).len(),
        );
        has(
            context,
            "editorHasSignatureHelpProvider",
            providers::signature_help_providers(cx, target.clone()).len(),
        );
        has(
            context,
            "editorHasInlayHintsProvider",
            providers::inlay_hints_providers(cx, target.clone()).len(),
        );
        let document_formatters =
            providers::document_formatting_providers(cx, target.clone()).len();
        let range_formatters =
            providers::document_range_formatting_providers(cx, target.clone()).len();
        // Monaco: a range formatter formats documents too.
        has(
            context,
            "editorHasDocumentFormattingProvider",
            document_formatters + range_formatters,
        );
        has(
            context,
            "editorHasDocumentSelectionFormattingProvider",
            range_formatters,
        );
        flag(
            context,
            "editorHasMultipleDocumentFormattingProvider",
            document_formatters + range_formatters > 1,
        );
        flag(
            context,
            "editorHasMultipleDocumentSelectionFormattingProvider",
            range_formatters > 1,
        );
        // Monaco `MarkerSelectionStatus`: an error, warning or info
        // intersects the primary selection.
        let primary = self.cursors.primary().selection();
        flag(
            context,
            "editorSelectionHasDiagnostics",
            model.has_markers()
                && model.markers().iter().any(|marker| {
                    marker.severity != crate::MarkerSeverity::Hint
                        && marker.range.start < primary.end()
                        && primary.start() < marker.range.end
                }),
        );

        // The contributions' keys.
        let selected = self.selected_suggestion();
        flag(context, "suggestWidgetVisible", self.is_suggest_visible());
        flag(
            context,
            "suggestWidgetMultipleSuggestions",
            self.suggest_item_count() > 1,
        );
        flag(
            context,
            "suggestWidgetHasFocusedSuggestion",
            selected.is_some(),
        );
        flag(
            context,
            "suggestWidgetDetailsVisible",
            self.is_suggestion_details_visible(cx),
        );
        flag(
            context,
            "suggestWidgetDetailsFocused",
            self.is_suggestion_details_focused(),
        );
        flag(
            context,
            "suggestionMakesTextEdit",
            self.selected_suggestion_edit().is_none_or(|(range, text)| {
                let rope = self.snapshot.rope();
                rope.text_for_range(
                    rope.point_utf16_to_offset(range.start)..rope.point_utf16_to_offset(range.end),
                ) != text
            }),
        );
        flag(
            context,
            "acceptSuggestionOnEnter",
            self.options.accept_suggestion_on_enter != crate::options::AcceptSuggestionOnEnter::Off,
        );
        if let Some((_, item)) = &selected {
            flag(
                context,
                "suggestionHasInsertAndReplaceRange",
                item.replace_range.is_some() && item.replace_range != item.range,
            );
            flag(context, "suggestionCanResolve", item.data.is_some());
            context.set(
                "suggestionInsertMode",
                match self.options.suggest.insert_mode {
                    crate::options::SuggestInsertMode::Insert => "insert",
                    crate::options::SuggestInsertMode::Replace => "replace",
                },
            );
        }
        flag(context, "atEndOfWord", self.at_end_of_word(cx));
        flag(context, "hasOtherSuggestions", self.has_other_suggestions());
        flag(context, "inSnippetMode", self.in_snippet_mode());
        flag(context, "hasNextTabstop", self.has_next_tabstop());
        flag(context, "hasPrevTabstop", self.has_prev_tabstop());
        flag(context, "inSnippetChoice", self.in_snippet_choice());
        flag(context, "messageVisible", self.is_message_visible());
        flag(context, "hasWordHighlights", self.has_word_highlights());
        flag(
            context,
            "LinkedEditingInputVisible",
            self.linked_editing_active(),
        );
        flag(context, "renameInputVisible", self.is_rename_visible());
        flag(context, "renameInputFocused", focus.rename_input);
        flag(context, "inReferenceSearchEditor", self.in_peek);
        flag(context, "referenceSearchVisible", self.is_peek_visible());
        flag(
            context,
            "selectionAnchorSet",
            self.selection_anchor(cx).is_some(),
        );
        flag(context, "foldingEnabled", self.options.folding);
        flag(
            context,
            "markersNavigationVisible",
            self.is_marker_zone_visible(),
        );
        flag(
            context,
            "hasSymbols",
            crate::goto::has_symbol_navigation(cx),
        );
        let (paste_widget, drop_widget) = self.post_edit_widget_kind();
        flag(context, "pasteWidgetVisible", paste_widget);
        flag(context, "dropWidgetVisible", drop_widget);
        flag(
            context,
            "cancellableOperation",
            self.has_cancellable_operation(),
        );
        flag(
            context,
            "parameterHintsVisible",
            self.is_signature_help_visible(),
        );
        flag(
            context,
            "parameterHintsMultipleSignatures",
            self.has_multiple_signatures(cx),
        );
        flag(context, "findWidgetVisible", self.is_find_visible());
        flag(context, "findInputFocussed", focus.find_input);
        flag(context, "replaceInputFocussed", focus.replace_input);
        flag(context, "findWidgetFocused", focus.find_widget);
        let inline_visible = self.inline_suggestion_visible(cx);
        let inline_edit = self.inline_edit_visible(cx);
        flag(context, "inlineSuggestionVisible", inline_visible);
        flag(context, "inlineEditIsVisible", inline_edit);
        flag(
            context,
            "cursorBeforeGhostText",
            inline_visible && self.cursor_before_ghost_text(cx),
        );
        flag(
            context,
            "cursorAtInlineEdit",
            inline_edit && self.cursor_at_inline_edit(cx),
        );
        flag(
            context,
            "tabShouldJumpToInlineEdit",
            inline_edit && self.tab_should_jump_to_inline_edit(cx),
        );
        flag(
            context,
            "tabShouldAcceptInlineEdit",
            inline_edit && self.tab_should_accept_inline_edit(cx),
        );
        flag(
            context,
            "inlineSuggestionAlternativeActionVisible",
            inline_edit && self.inline_suggestion_alternative_action_visible(cx),
        );

        // Monaco `supportedCodeAction`: the code action providers' kinds,
        // space-separated. GPUI predicates can't match Monaco's regex on
        // it, so the kinds the built-in actions test are keys of their own.
        let kinds: Vec<CodeActionKind> = providers::code_action_providers(cx, target)
            .iter()
            .flat_map(|p| p.provided_kinds().unwrap_or_default())
            .collect();
        if !kinds.is_empty() {
            let joined: Vec<&str> = kinds.iter().map(|k| k.0.as_ref()).collect();
            context.set("supportedCodeAction", joined.join(" "));
        }
        for (key, kind) in [
            ("supportedCodeAction_quickfix", CodeActionKind::QUICK_FIX),
            ("supportedCodeAction_refactor", CodeActionKind::REFACTOR),
            ("supportedCodeAction_source", CodeActionKind::SOURCE),
            (
                "supportedCodeAction_source_organizeImports",
                CodeActionKind::SOURCE_ORGANIZE_IMPORTS,
            ),
            (
                "supportedCodeAction_source_fixAll",
                CodeActionKind::SOURCE_FIX_ALL,
            ),
        ] {
            if supports_kind(&kinds, kind) {
                context.add(key);
            }
        }

        // An enclosing diff editor's keys (`isInDiffEditor`,
        // `diffEditorHasChanges`, …), which Monaco's editors inherit from
        // the diff editor's scoped context key service.
        for (key, value) in &self.commands.parent_keys {
            value.apply(key, context);
        }
        // The host's keys (Monaco `createContextKey`).
        for (key, value) in &self.commands.host_keys {
            value.apply(key, context);
        }
        // This editor, for its own commands' keybindings
        // ([`CodeEditor::add_command`], [`CodeEditor::add_action`]).
        context.set(EDITOR_ID_KEY, self.commands.id.to_string());
    }

    /// Creates a context key on this editor with `default` as its value
    /// (Monaco `IStandaloneCodeEditor.createContextKey`). The editor sets it
    /// on its key context until [`ContextKey::reset`] or another value, so
    /// keybinding rules can test it (`CodeEditor && myKey`) and
    /// [`CodeEditor::add_command`] can take it as its `when`.
    pub fn create_context_key(
        &mut self,
        key: impl Into<SharedString>,
        default: impl Into<ContextKeyValue>,
        cx: &mut Context<Self>,
    ) -> ContextKey {
        let key = key.into();
        let default = default.into();
        self.commands.host_keys.insert(key.clone(), default.clone());
        cx.notify();
        ContextKey {
            editor: cx.weak_entity(),
            key,
            default,
        }
    }

    /// Monaco's `WordContextKey` (`atEndOfWord`), kept only with
    /// `tab_completion` on: the cursor ends a word.
    fn at_end_of_word(&self, _cx: &App) -> bool {
        if self.options.tab_completion != crate::options::TabCompletion::On {
            return false;
        }
        let selection = self.cursors.primary().selection();
        if !selection.is_empty() {
            return false;
        }
        let position = selection.head;
        let line = self.snapshot.rope().line(position.row);
        let units: Vec<u16> = line.encode_utf16().collect();
        let column = position.column as usize;
        let is_word = |unit: Option<&u16>| {
            unit.and_then(|u| char::from_u32(*u as u32))
                .is_some_and(|c| c.is_alphanumeric() || c == '_')
        };
        column > 0 && is_word(units.get(column - 1)) && !is_word(units.get(column))
    }
}

/// Sets `key` when `on`.
fn flag(context: &mut KeyContext, key: &'static str, on: bool) {
    if on {
        context.add(key);
    }
}

/// Sets `key` when there are providers.
fn has(context: &mut KeyContext, key: &'static str, count: usize) {
    flag(context, key, count > 0);
}

/// The key context entry naming an editor, which its own keybindings test.
pub(crate) const EDITOR_ID_KEY: &str = "code_editor_id";

/// The editor's widgets whose key contexts carry the editor's context keys
/// ([`widget_key_context`]), so their keybindings can test them: Monaco's
/// widgets live in their editor's context key service.
pub(crate) const WIDGET_KEY_CONTEXTS: &[&str] = &[
    crate::hover::HOVER_KEY_CONTEXT,
    crate::rename::RENAME_INPUT_KEY_CONTEXT,
    crate::peek::PEEK_KEY_CONTEXT,
    crate::color_picker::STANDALONE_COLOR_PICKER_KEY_CONTEXT,
    crate::find_widget::FIND_WIDGET_KEY_CONTEXT,
    crate::find_widget::FIND_INPUT_KEY_CONTEXT,
    crate::find_widget::REPLACE_INPUT_KEY_CONTEXT,
];

/// `editor`'s context keys for its widgets' key contexts (see
/// [`widget_key_context`]); empty once the editor is gone.
pub(crate) fn editor_keys_for_widgets(editor: &WeakEntity<CodeEditor>, cx: &App) -> KeyContext {
    let mut keys = KeyContext::default();
    if let Some(editor) = editor.upgrade() {
        editor.read(cx).add_context_keys(&mut keys, cx);
    }
    keys
}

/// The key context of one of the editor's widgets (one of
/// [`WIDGET_KEY_CONTEXTS`]): `widget`, then the editor's context keys
/// (from [`editor_keys_for_widgets`]) but not [`KEY_CONTEXT`], so the
/// editor's preconditions folded into the widget's keybindings (and hosts'
/// rules such as `HoverWidget && editorLangId == rust`) see them, while the
/// editor's own keybindings don't match in the widget.
pub(crate) fn widget_key_context(widget: &'static str, editor_keys: &KeyContext) -> KeyContext {
    debug_assert!(WIDGET_KEY_CONTEXTS.contains(&widget));
    let mut context = KeyContext::default();
    context.add(widget);
    context.extend(editor_keys);
    context
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn supported_kinds_match_at_word_boundaries() {
        let kinds = [
            CodeActionKind::new("refactor.extract"),
            CodeActionKind::new("source"),
        ];
        assert!(supports_kind(&kinds, "refactor"));
        assert!(supports_kind(&kinds, "refactor.extract"));
        assert!(supports_kind(&kinds, "source"));
        assert!(!supports_kind(&kinds, "source.organizeImports"));
        assert!(!supports_kind(&kinds, "quickfix"));
        assert!(!supports_kind(
            &[CodeActionKind::new("refactoring")],
            "refactor"
        ));
    }
}
