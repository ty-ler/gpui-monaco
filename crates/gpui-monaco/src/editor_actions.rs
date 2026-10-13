//! The editor's actions (Monaco `IEditorAction`): the built-in ones with a
//! label, and the host's from [`CodeEditor::add_action`] (Monaco
//! `addAction`). The command palette lists the supported ones.
//!
//! Also Monaco's global `editor.addEditorAction` and
//! `editor.addKeybindingRule(s)`, removable with their [`Disposable`].

use std::collections::HashMap;
use std::rc::Rc;

use gpui::{App, Context, Global, KeyBinding, SharedString, Window};

use crate::actions::{KEY_CONTEXT, RunAction, RunCommand};
use crate::editor::CodeEditor;
use crate::providers;
use crate::registry::Disposable;

/// A host action's body, with its argument (Monaco `run(editor, ...args)`):
/// the payload of [`CodeEditor::trigger_with_payload`], the `args` of
/// [`CodeEditor::run_action_with_args`] and [`EditorAction::run`], or a
/// keybinding rule's `command_args`; `None` when run by its keybindings or
/// from the menus.
pub type ActionRun =
    Rc<dyn Fn(&mut CodeEditor, Option<serde_json::Value>, &mut Window, &mut Context<CodeEditor>)>;

/// A host action's precondition.
pub type ActionPrecondition = Rc<dyn Fn(&CodeEditor, &App) -> bool>;

/// An action added with [`CodeEditor::add_action`] (Monaco
/// `IActionDescriptor`).
#[derive(Clone)]
pub struct EditorActionDescriptor {
    /// A unique id; adding another action with it replaces this one.
    pub id: SharedString,
    /// The label in the command palette.
    pub label: SharedString,
    /// Another name the command palette matches.
    pub alias: Option<SharedString>,
    /// Keystrokes (GPUI syntax, e.g. `"cmd-k cmd-m"`) that run it in the
    /// editor's key context.
    pub keybindings: Vec<SharedString>,
    /// Where its keybindings apply, besides the precondition (Monaco
    /// `keybindingContext`): a GPUI predicate over the editor's
    /// [context keys](CodeEditor::context_keys), such as `"!editorReadonly"`
    /// or a key from [`CodeEditor::create_context_key`]. Keybindings whose
    /// predicate doesn't parse aren't bound.
    pub keybinding_context: Option<SharedString>,
    /// Whether it can run; without one it always can.
    pub precondition: Option<ActionPrecondition>,
    /// The context menu group to show it in (Monaco `contextMenuGroupId`,
    /// e.g. `"navigation"` or `"1_modification"`); without one it isn't
    /// in the menu.
    pub context_menu_group_id: Option<SharedString>,
    /// Its order in the group (Monaco `contextMenuOrder`).
    pub context_menu_order: f32,
    /// Runs it.
    pub run: ActionRun,
}

impl EditorActionDescriptor {
    /// An action with `id`, `label` and its body, which ignores arguments.
    pub fn new(
        id: impl Into<SharedString>,
        label: impl Into<SharedString>,
        run: impl Fn(&mut CodeEditor, &mut Window, &mut Context<CodeEditor>) + 'static,
    ) -> Self {
        Self::new_with_args(id, label, move |editor, _, window, cx| {
            run(editor, window, cx)
        })
    }

    /// An action with `id`, `label` and a body taking its argument (Monaco
    /// `run(editor, ...args)`; see [`ActionRun`]).
    pub fn new_with_args(
        id: impl Into<SharedString>,
        label: impl Into<SharedString>,
        run: impl Fn(&mut CodeEditor, Option<serde_json::Value>, &mut Window, &mut Context<CodeEditor>)
        + 'static,
    ) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            alias: None,
            keybindings: Vec::new(),
            keybinding_context: None,
            precondition: None,
            context_menu_group_id: None,
            context_menu_order: 0.,
            run: Rc::new(run),
        }
    }

    /// Sets the alias.
    pub fn alias(mut self, alias: impl Into<SharedString>) -> Self {
        self.alias = Some(alias.into());
        self
    }

    /// Adds a keybinding.
    pub fn keybinding(mut self, keystrokes: impl Into<SharedString>) -> Self {
        self.keybindings.push(keystrokes.into());
        self
    }

    /// Sets where the keybindings apply (Monaco `keybindingContext`).
    pub fn keybinding_context(mut self, when: impl Into<SharedString>) -> Self {
        self.keybinding_context = Some(when.into());
        self
    }

    /// Shows it in the context menu in `group` at `order`.
    pub fn context_menu(mut self, group: impl Into<SharedString>, order: f32) -> Self {
        self.context_menu_group_id = Some(group.into());
        self.context_menu_order = order;
        self
    }

    /// Sets the precondition.
    pub fn precondition(
        mut self,
        precondition: impl Fn(&CodeEditor, &App) -> bool + 'static,
    ) -> Self {
        self.precondition = Some(Rc::new(precondition));
        self
    }
}

/// An action the editor supports (Monaco `IEditorAction`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EditorActionInfo {
    /// Its Monaco id, e.g. `"editor.action.formatDocument"`.
    pub id: SharedString,
    /// Its label.
    pub label: SharedString,
    /// Another name, if any.
    pub alias: Option<SharedString>,
}

/// One of an editor's actions, from [`CodeEditor::get_action`] (Monaco
/// `IEditorAction`): its names, whether it can run now, and a way to run
/// it in that editor. Monaco's `metadata` (a command's argument
/// descriptions) has no counterpart.
#[derive(Clone, Debug)]
pub struct EditorAction {
    /// Its id, label and alias.
    pub info: EditorActionInfo,
    editor: gpui::WeakEntity<CodeEditor>,
}

impl EditorAction {
    /// Its id (Monaco `id`).
    pub fn id(&self) -> &SharedString {
        &self.info.id
    }

    /// Its label (Monaco `label`).
    pub fn label(&self) -> &SharedString {
        &self.info.label
    }

    /// Its alias (Monaco `alias`).
    pub fn alias(&self) -> Option<&SharedString> {
        self.info.alias.as_ref()
    }

    /// Whether its precondition holds now (Monaco `isSupported`); `false`
    /// once the editor is gone.
    pub fn is_supported(&self, cx: &App) -> bool {
        self.editor
            .upgrade()
            .is_some_and(|editor| editor.read(cx).action_supported(&self.info.id, cx))
    }

    /// Runs it in its editor with `args` (Monaco `run(args)`), as
    /// [`CodeEditor::run_action_with_args`] does. Call it outside the
    /// editor's own update (from an editor method, use
    /// [`CodeEditor::run_action_with_args`]). Returns whether it ran.
    pub fn run(&self, args: Option<serde_json::Value>, window: &mut Window, cx: &mut App) -> bool {
        self.editor
            .update(cx, |editor, cx| {
                editor.run_action_with_args(&self.info.id, args, window, cx)
            })
            .unwrap_or(false)
    }
}

/// A built-in action: Monaco id, GPUI action name, label, and Monaco's
/// precondition as a predicate over the editor's
/// [context keys](CodeEditor::context_keys) (empty for none). The command
/// palette lists the action while it holds, [`CodeEditor::trigger`] does
/// nothing while it doesn't, and the default keybindings require it, so the
/// key falls through as in Monaco.
struct Builtin(&'static str, &'static str, &'static str, &'static str);

/// Monaco `EditorContextKeys.writable`.
const WRITABLE: &str = "!editorReadonly";
/// Monaco `CONTEXT_FOLDING_ENABLED`.
const FOLDING: &str = "foldingEnabled";
/// Monaco `EditorContextKeys.hoverFocused`.
const HOVER_FOCUSED: &str = "editorHoverFocused";
/// Monaco `EditorContextKeys.stickyScrollFocused`.
const STICKY_FOCUSED: &str = "stickyScrollFocused";
/// Monaco `SelectionAnchorSet`.
const ANCHOR_SET: &str = "selectionAnchorSet";
/// Monaco `writable && inlineSuggestionVisible`.
const INLINE_WRITABLE: &str = "!editorReadonly && inlineSuggestionVisible";
/// Monaco `inlineSuggestionVisible || inlineEditIsVisible`.
const INLINE_OR_EDIT: &str = "inlineSuggestionVisible || inlineEditIsVisible";
/// Monaco `writable && hasCodeActionsProvider`.
const CODE_ACTIONS: &str = "!editorReadonly && editorHasCodeActionsProvider";
/// Monaco `SuggestContext.Visible`.
const SUGGEST_VISIBLE: &str = "suggestWidgetVisible";
/// Monaco's precondition for moving in the suggest list.
const SUGGEST_NAVIGABLE: &str = "suggestWidgetVisible && (suggestWidgetMultipleSuggestions || !suggestWidgetHasFocusedSuggestion)";

/// Monaco's labelled editor actions that this editor implements.
#[rustfmt::skip]
static BUILTINS: &[Builtin] = &[
    Builtin("editor.action.quickCommand", "code_editor::QuickCommand", "Command Palette", ""),
    Builtin("editor.action.gotoLine", "code_editor::GotoLine", "Go to Line/Column...", ""),
    // Monaco requires a document symbol provider; the `tags.scm` outline
    // counts as one here.
    Builtin("editor.action.quickOutline", "code_editor::QuickOutline", "Go to Symbol...", "editorHasDocumentSymbolProvider"),
    Builtin("editor.action.gotoOffset", "code_editor::GotoOffset", "Go to Offset...", ""),
    // Developer and theme.
    Builtin("editor.action.forceRetokenize", "code_editor::ForceRetokenize", "Developer: Force Retokenize", ""),
    Builtin("editor.action.inspectTokens", "code_editor::InspectTokens", "Developer: Inspect Tokens", ""),
    Builtin("editor.action.toggleHighContrast", "code_editor::ToggleHighContrast", "Toggle High Contrast Theme", ""),
    // Cursors and selections.
    Builtin("expandLineSelection", "code_editor::ExpandLineSelection", "Expand Line Selection", ""),
    Builtin("editor.action.insertCursorAbove", "code_editor::AddCursorAbove", "Add Cursor Above", ""),
    Builtin("editor.action.insertCursorBelow", "code_editor::AddCursorBelow", "Add Cursor Below", ""),
    Builtin("editor.action.insertCursorAtEndOfEachLineSelected", "code_editor::InsertCursorAtEndOfEachLineSelected", "Add Cursors to Line Ends", ""),
    Builtin("editor.action.addCursorsToBottom", "code_editor::AddCursorsToBottom", "Add Cursors to Bottom", ""),
    Builtin("editor.action.addCursorsToTop", "code_editor::AddCursorsToTop", "Add Cursors to Top", ""),
    Builtin("editor.action.addSelectionToNextFindMatch", "code_editor::AddSelectionToNextFindMatch", "Add Selection to Next Find Match", ""),
    Builtin("editor.action.addSelectionToPreviousFindMatch", "code_editor::AddSelectionToPreviousFindMatch", "Add Selection to Previous Find Match", ""),
    Builtin("editor.action.moveSelectionToNextFindMatch", "code_editor::MoveSelectionToNextFindMatch", "Move Last Selection to Next Find Match", ""),
    Builtin("editor.action.moveSelectionToPreviousFindMatch", "code_editor::MoveSelectionToPreviousFindMatch", "Move Last Selection to Previous Find Match", ""),
    Builtin("editor.action.selectHighlights", "code_editor::SelectAllOccurrences", "Select All Occurrences of Find Match", ""),
    Builtin("editor.action.focusNextCursor", "code_editor::FocusNextCursor", "Focus Next Cursor", ""),
    Builtin("editor.action.focusPreviousCursor", "code_editor::FocusPreviousCursor", "Focus Previous Cursor", ""),
    Builtin("cursorUndo", "code_editor::CursorUndo", "Cursor Undo", ""),
    Builtin("cursorRedo", "code_editor::CursorRedo", "Cursor Redo", ""),
    Builtin("editor.action.setSelectionAnchor", "code_editor::SetSelectionAnchor", "Set Selection Anchor", ""),
    Builtin("editor.action.goToSelectionAnchor", "code_editor::GoToSelectionAnchor", "Go to Selection Anchor", ANCHOR_SET),
    Builtin("editor.action.selectFromAnchorToCursor", "code_editor::SelectFromAnchorToCursor", "Select from Anchor to Cursor", ANCHOR_SET),
    Builtin("editor.action.cancelSelectionAnchor", "code_editor::CancelSelectionAnchor", "Cancel Selection Anchor", ANCHOR_SET),
    Builtin("editor.action.smartSelect.expand", "code_editor::ExpandSelection", "Expand Selection", ""),
    Builtin("editor.action.smartSelect.shrink", "code_editor::ShrinkSelection", "Shrink Selection", ""),
    Builtin("editor.action.toggleOvertypeInsertMode", "code_editor::ToggleOvertypeInsertMode", "Toggle Overtype/Insert Mode", ""),
    // Find.
    Builtin("actions.find", "code_editor::StartFind", "Find", ""),
    Builtin("editor.action.startFindReplaceAction", "code_editor::StartFindReplace", "Replace", ""),
    Builtin("editor.action.nextMatchFindAction", "code_editor::NextMatch", "Find Next", ""),
    Builtin("editor.action.previousMatchFindAction", "code_editor::PreviousMatch", "Find Previous", ""),
    Builtin("editor.actions.findWithArgs", "code_editor::FindWithArgs", "Find with Arguments", ""),
    Builtin("editor.action.goToMatchFindAction", "code_editor::GoToMatch", "Go to Match...", "findWidgetVisible"),
    // Brackets, problems, highlights.
    Builtin("editor.action.jumpToBracket", "code_editor::JumpToBracket", "Go to Bracket", ""),
    Builtin("editor.action.selectToBracket", "code_editor::SelectToBracket", "Select to Bracket", ""),
    Builtin("editor.action.marker.next", "code_editor::GoToNextMarker", "Go to Next Problem (Error, Warning, Info)", ""),
    Builtin("editor.action.marker.prev", "code_editor::GoToPreviousMarker", "Go to Previous Problem (Error, Warning, Info)", ""),
    Builtin("editor.action.marker.nextInFiles", "code_editor::GoToNextMarkerInFiles", "Go to Next Problem in Files (Error, Warning, Info)", ""),
    Builtin("editor.action.marker.prevInFiles", "code_editor::GoToPreviousMarkerInFiles", "Go to Previous Problem in Files (Error, Warning, Info)", ""),
    Builtin("editor.action.wordHighlight.next", "code_editor::WordHighlightNext", "Go to Next Symbol Highlight", "hasWordHighlights"),
    Builtin("editor.action.wordHighlight.prev", "code_editor::WordHighlightPrev", "Go to Previous Symbol Highlight", "hasWordHighlights"),
    Builtin("editor.action.wordHighlight.trigger", "code_editor::WordHighlightTrigger", "Trigger Symbol Highlight", ""),
    Builtin("editor.action.showHover", "code_editor::ShowHover", "Show or Focus Hover", ""),
    Builtin("editor.action.hideHover", "code_editor::HideHover", "Hide Hover", ""),
    Builtin("editor.action.showDefinitionPreviewHover", "code_editor::ShowDefinitionPreviewHover", "Show Definition Preview Hover", ""),
    Builtin("editor.action.scrollUpHover", "code_editor::ScrollUpHover", "Scroll Up Hover", HOVER_FOCUSED),
    Builtin("editor.action.scrollDownHover", "code_editor::ScrollDownHover", "Scroll Down Hover", HOVER_FOCUSED),
    Builtin("editor.action.scrollLeftHover", "code_editor::ScrollLeftHover", "Scroll Left Hover", HOVER_FOCUSED),
    Builtin("editor.action.scrollRightHover", "code_editor::ScrollRightHover", "Scroll Right Hover", HOVER_FOCUSED),
    Builtin("editor.action.pageUpHover", "code_editor::PageUpHover", "Page Up Hover", HOVER_FOCUSED),
    Builtin("editor.action.pageDownHover", "code_editor::PageDownHover", "Page Down Hover", HOVER_FOCUSED),
    Builtin("editor.action.goToTopHover", "code_editor::GoToTopHover", "Go To Top Hover", HOVER_FOCUSED),
    Builtin("editor.action.goToBottomHover", "code_editor::GoToBottomHover", "Go To Bottom Hover", HOVER_FOCUSED),
    Builtin("editor.action.increaseHoverVerbosityLevel", "code_editor::IncreaseHoverVerbosityLevel", "Increase Hover Verbosity Level", "editorHoverVisible"),
    Builtin("editor.action.decreaseHoverVerbosityLevel", "code_editor::DecreaseHoverVerbosityLevel", "Decrease Hover Verbosity Level", "editorHoverVisible"),
    Builtin("editor.action.openLink", "code_editor::OpenLink", "Open Link", ""),
    Builtin("editor.action.fontZoomIn", "code_editor::FontZoomIn", "Increase Editor Font Size", ""),
    Builtin("editor.action.fontZoomOut", "code_editor::FontZoomOut", "Decrease Editor Font Size", ""),
    Builtin("editor.action.fontZoomReset", "code_editor::FontZoomReset", "Reset Editor Font Size", ""),
    Builtin("editor.action.toggleStickyScroll", "code_editor::ToggleStickyScroll", "Toggle Editor Sticky Scroll", ""),
    Builtin("editor.action.focusStickyScroll", "code_editor::FocusStickyScroll", "Focus Editor Sticky Scroll", "stickyScrollVisible"),
    Builtin("editor.action.showContextMenu", "code_editor::ShowContextMenu", "Show Editor Context Menu", ""),
    Builtin("editor.action.showOrFocusStandaloneColorPicker", "code_editor::ShowOrFocusStandaloneColorPicker", "Show or Focus Standalone Color Picker", ""),
    // Editing.
    Builtin("deleteAllLeft", "code_editor::DeleteAllLeft", "Delete All Left", WRITABLE),
    Builtin("deleteAllRight", "code_editor::DeleteAllRight", "Delete All Right", WRITABLE),
    Builtin("editor.action.sortLinesAscending", "code_editor::SortLinesAscending", "Sort Lines Ascending", WRITABLE),
    Builtin("editor.action.sortLinesDescending", "code_editor::SortLinesDescending", "Sort Lines Descending", WRITABLE),
    Builtin("editor.action.removeDuplicateLines", "code_editor::DeleteDuplicateLines", "Delete Duplicate Lines", WRITABLE),
    Builtin("editor.action.reverseLines", "code_editor::ReverseLines", "Reverse lines", WRITABLE),
    Builtin("editor.action.trimTrailingWhitespace", "code_editor::TrimTrailingWhitespace", "Trim Trailing Whitespace", WRITABLE),
    Builtin("editor.action.deleteLines", "code_editor::DeleteLines", "Delete Line", WRITABLE),
    Builtin("editor.action.joinLines", "code_editor::JoinLines", "Join Lines", WRITABLE),
    Builtin("editor.action.transpose", "code_editor::Transpose", "Transpose Characters around the Cursor", WRITABLE),
    Builtin("editor.action.transposeLetters", "code_editor::TransposeLetters", "Transpose Letters", WRITABLE),
    Builtin("editor.action.transformToUppercase", "code_editor::TransformToUppercase", "Transform to Uppercase", WRITABLE),
    Builtin("editor.action.transformToLowercase", "code_editor::TransformToLowercase", "Transform to Lowercase", WRITABLE),
    Builtin("editor.action.transformToTitlecase", "code_editor::TransformToTitlecase", "Transform to Title Case", WRITABLE),
    Builtin("editor.action.transformToSnakecase", "code_editor::TransformToSnakecase", "Transform to Snake Case", WRITABLE),
    Builtin("editor.action.transformToCamelcase", "code_editor::TransformToCamelcase", "Transform to Camel Case", WRITABLE),
    Builtin("editor.action.transformToPascalcase", "code_editor::TransformToPascalcase", "Transform to Pascal Case", WRITABLE),
    Builtin("editor.action.transformToKebabcase", "code_editor::TransformToKebabcase", "Transform to Kebab Case", WRITABLE),
    Builtin("editor.action.duplicateSelection", "code_editor::DuplicateSelection", "Duplicate Selection", WRITABLE),
    Builtin("editor.action.moveCarretLeftAction", "code_editor::MoveCaretLeft", "Move Selected Text Left", WRITABLE),
    Builtin("editor.action.moveCarretRightAction", "code_editor::MoveCaretRight", "Move Selected Text Right", WRITABLE),
    Builtin("editor.action.reindentlines", "code_editor::ReindentLines", "Reindent Lines", WRITABLE),
    Builtin("editor.action.reindentselectedlines", "code_editor::ReindentSelectedLines", "Reindent Selected Lines", WRITABLE),
    Builtin("editor.action.inPlaceReplace.up", "code_editor::InPlaceReplaceUp", "Replace with Previous Value", WRITABLE),
    Builtin("editor.action.inPlaceReplace.down", "code_editor::InPlaceReplaceDown", "Replace with Next Value", WRITABLE),
    // Monaco has no precondition here; read-only editors say they can't
    // be edited.
    Builtin("editor.action.removeBrackets", "code_editor::RemoveBrackets", "Remove Brackets", ""),
    Builtin("editor.action.commentLine", "code_editor::CommentLine", "Toggle Line Comment", WRITABLE),
    Builtin("editor.action.addCommentLine", "code_editor::AddCommentLine", "Add Line Comment", WRITABLE),
    Builtin("editor.action.removeCommentLine", "code_editor::RemoveCommentLine", "Remove Line Comment", WRITABLE),
    Builtin("editor.action.blockComment", "code_editor::BlockComment", "Toggle Block Comment", WRITABLE),
    Builtin("editor.action.indentLines", "code_editor::IndentLines", "Indent Line", WRITABLE),
    Builtin("editor.action.outdentLines", "code_editor::OutdentLines", "Outdent Line", WRITABLE),
    Builtin("editor.action.moveLinesUpAction", "code_editor::MoveLinesUp", "Move Line Up", WRITABLE),
    Builtin("editor.action.moveLinesDownAction", "code_editor::MoveLinesDown", "Move Line Down", WRITABLE),
    Builtin("editor.action.copyLinesUpAction", "code_editor::CopyLinesUp", "Copy Line Up", WRITABLE),
    Builtin("editor.action.copyLinesDownAction", "code_editor::CopyLinesDown", "Copy Line Down", WRITABLE),
    Builtin("editor.action.insertLineAfter", "code_editor::InsertLineAfter", "Insert Line Below", WRITABLE),
    Builtin("editor.action.insertLineBefore", "code_editor::InsertLineBefore", "Insert Line Above", WRITABLE),
    Builtin("editor.action.linkedEditing", "code_editor::StartLinkedEditing", "Start Linked Editing", "!editorReadonly && editorHasRenameProvider"),
    Builtin("editor.action.triggerSuggest", "code_editor::TriggerSuggest", "Trigger Suggest", "!editorReadonly && editorHasCompletionItemProvider && !suggestWidgetVisible"),
    Builtin("editor.action.resetSuggestSize", "code_editor::ResetSuggestSize", "Reset Suggest Widget Size", ""),
    Builtin("editor.action.insertFinalNewLine", "code_editor::InsertFinalNewLine", "Insert Final New Line", WRITABLE),
    Builtin("editor.action.inlineSuggest.trigger", "code_editor::TriggerInlineSuggest", "Trigger Inline Suggestion", WRITABLE),
    Builtin("editor.action.triggerParameterHints", "code_editor::TriggerParameterHints", "Trigger Parameter Hints", "editorHasSignatureHelpProvider"),
    // Folding.
    Builtin("editor.fold", "code_editor::Fold", "Fold", FOLDING),
    Builtin("editor.unfold", "code_editor::Unfold", "Unfold", FOLDING),
    Builtin("editor.toggleFold", "code_editor::ToggleFold", "Toggle Fold", FOLDING),
    Builtin("editor.foldRecursively", "code_editor::FoldRecursively", "Fold Recursively", FOLDING),
    Builtin("editor.unfoldRecursively", "code_editor::UnfoldRecursively", "Unfold Recursively", FOLDING),
    Builtin("editor.toggleFoldRecursively", "code_editor::ToggleFoldRecursively", "Toggle Fold Recursively", FOLDING),
    Builtin("editor.foldAll", "code_editor::FoldAll", "Fold All", FOLDING),
    Builtin("editor.unfoldAll", "code_editor::UnfoldAll", "Unfold All", FOLDING),
    Builtin("editor.foldAllBlockComments", "code_editor::FoldAllBlockComments", "Fold All Block Comments", FOLDING),
    Builtin("editor.foldAllMarkerRegions", "code_editor::FoldAllMarkerRegions", "Fold All Regions", FOLDING),
    Builtin("editor.unfoldAllMarkerRegions", "code_editor::UnfoldAllMarkerRegions", "Unfold All Regions", FOLDING),
    Builtin("editor.foldAllExcept", "code_editor::FoldAllExcept", "Fold All Except Selected", FOLDING),
    Builtin("editor.unfoldAllExcept", "code_editor::UnfoldAllExcept", "Unfold All Except Selected", FOLDING),
    Builtin("editor.foldLevel1", "code_editor::FoldLevel1", "Fold Level 1", FOLDING),
    Builtin("editor.foldLevel2", "code_editor::FoldLevel2", "Fold Level 2", FOLDING),
    Builtin("editor.foldLevel3", "code_editor::FoldLevel3", "Fold Level 3", FOLDING),
    Builtin("editor.foldLevel4", "code_editor::FoldLevel4", "Fold Level 4", FOLDING),
    Builtin("editor.foldLevel5", "code_editor::FoldLevel5", "Fold Level 5", FOLDING),
    Builtin("editor.foldLevel6", "code_editor::FoldLevel6", "Fold Level 6", FOLDING),
    Builtin("editor.foldLevel7", "code_editor::FoldLevel7", "Fold Level 7", FOLDING),
    Builtin("editor.gotoParentFold", "code_editor::GoToParentFold", "Go to Parent Fold", FOLDING),
    Builtin("editor.gotoPreviousFold", "code_editor::GoToPreviousFold", "Go to Previous Folding Range", FOLDING),
    Builtin("editor.gotoNextFold", "code_editor::GoToNextFold", "Go to Next Folding Range", FOLDING),
    Builtin("editor.createFoldingRangeFromSelection", "code_editor::CreateFoldFromSelection", "Create Folding Range from Selection", FOLDING),
    Builtin("editor.removeManualFoldingRanges", "code_editor::RemoveManualFoldingRanges", "Remove Manual Folding Ranges", FOLDING),
    Builtin("editor.toggleImportFold", "code_editor::ToggleImportFold", "Toggle Import Fold", FOLDING),
    // Language features.
    Builtin("editor.action.formatDocument", "code_editor::FormatDocument", "Format Document", "!inCompositeEditor && !editorReadonly && editorHasDocumentFormattingProvider"),
    Builtin("editor.action.formatSelection", "code_editor::FormatSelection", "Format Selection", "!editorReadonly && editorHasDocumentSelectionFormattingProvider"),
    Builtin("editor.action.rename", "code_editor::Rename", "Rename Symbol", "!editorReadonly && editorHasRenameProvider"),
    Builtin("editor.action.toggleTabFocusMode", "code_editor::ToggleTabFocusMode", "Toggle Tab Key Moves Focus", ""),
    Builtin("editor.action.pasteAs", "code_editor::PasteAs", "Paste As...", WRITABLE),
    Builtin("editor.action.pasteAsText", "code_editor::PasteAsText", "Paste as Text", WRITABLE),
    Builtin("editor.action.quickFix", "code_editor::QuickFix", "Quick Fix...", CODE_ACTIONS),
    Builtin("editor.action.refactor", "code_editor::Refactor", "Refactor...", CODE_ACTIONS),
    Builtin("editor.action.sourceAction", "code_editor::SourceAction", "Source Action...", CODE_ACTIONS),
    Builtin("editor.action.organizeImports", "code_editor::OrganizeImports", "Organize Imports", "!editorReadonly && supportedCodeAction_source_organizeImports"),
    Builtin("editor.action.fixAll", "code_editor::FixAll", "Fix All", "!editorReadonly && supportedCodeAction_source_fixAll"),
    Builtin("editor.action.autoFix", "code_editor::AutoFix", "Auto Fix...", "!editorReadonly && supportedCodeAction_quickfix"),
    Builtin("editor.action.revealDefinition", "code_editor::GoToDefinition", "Go to Definition", "editorHasDefinitionProvider"),
    Builtin("editor.action.peekDefinition", "code_editor::PeekDefinition", "Peek Definition", "editorHasDefinitionProvider && !inReferenceSearchEditor && !isInEmbeddedEditor"),
    Builtin("editor.action.revealDefinitionAside", "code_editor::RevealDefinitionAside", "Open Definition to the Side", "editorHasDefinitionProvider && !isInEmbeddedEditor"),
    Builtin("codelens.showLensesInCurrentLine", "code_editor::ShowLensesInCurrentLine", "Show CodeLens Commands for Current Line", "editorHasCodeLensProvider"),
    Builtin("editor.action.revealDeclaration", "code_editor::GoToDeclaration", "Go to Declaration", "editorHasDeclarationProvider && !isInEmbeddedEditor"),
    Builtin("editor.action.peekDeclaration", "code_editor::PeekDeclaration", "Peek Declaration", "editorHasDeclarationProvider && !inReferenceSearchEditor && !isInEmbeddedEditor"),
    Builtin("editor.action.goToTypeDefinition", "code_editor::GoToTypeDefinition", "Go to Type Definition", "editorHasTypeDefinitionProvider"),
    Builtin("editor.action.peekTypeDefinition", "code_editor::PeekTypeDefinition", "Peek Type Definition", "editorHasTypeDefinitionProvider && !inReferenceSearchEditor && !isInEmbeddedEditor"),
    Builtin("editor.action.goToImplementation", "code_editor::GoToImplementation", "Go to Implementations", "editorHasImplementationProvider"),
    Builtin("editor.action.peekImplementation", "code_editor::PeekImplementation", "Peek Implementations", "editorHasImplementationProvider && !inReferenceSearchEditor && !isInEmbeddedEditor"),
    Builtin("editor.action.goToReferences", "code_editor::GoToReferences", "Go to References", "editorHasReferenceProvider && !inReferenceSearchEditor && !isInEmbeddedEditor"),
    Builtin("editor.action.referenceSearch.trigger", "code_editor::PeekReferences", "Peek References", "editorHasReferenceProvider && !inReferenceSearchEditor && !isInEmbeddedEditor"),
    // Indentation.
    Builtin("editor.action.indentationToSpaces", "code_editor::IndentationToSpaces", "Convert Indentation to Spaces", WRITABLE),
    Builtin("editor.action.indentationToTabs", "code_editor::IndentationToTabs", "Convert Indentation to Tabs", WRITABLE),
    Builtin("editor.action.indentUsingTabs", "code_editor::IndentUsingTabs", "Indent Using Tabs", ""),
    Builtin("editor.action.indentUsingSpaces", "code_editor::IndentUsingSpaces", "Indent Using Spaces", ""),
    Builtin("editor.action.changeTabDisplaySize", "code_editor::ChangeTabDisplaySize", "Change Tab Display Size", ""),
    Builtin("editor.action.detectIndentation", "code_editor::DetectIndentation", "Detect Indentation from Content", ""),
    // Find, inline suggestions, the color picker, sticky scroll.
    Builtin("deleteInsideWord", "code_editor::DeleteInsideWord", "Delete Word", WRITABLE),
    Builtin("actions.findWithSelection", "code_editor::StartFindWithSelection", "Find with Selection", ""),
    Builtin("editor.action.nextSelectionMatchFindAction", "code_editor::NextSelectionMatch", "Find Next Selection", ""),
    Builtin("editor.action.previousSelectionMatchFindAction", "code_editor::PreviousSelectionMatch", "Find Previous Selection", ""),
    Builtin("editor.action.selectNextStickyScrollLine", "code_editor::SelectNextStickyScrollLine", "Select the next editor sticky scroll line", STICKY_FOCUSED),
    Builtin("editor.action.selectPreviousStickyScrollLine", "code_editor::SelectPreviousStickyScrollLine", "Select the previous sticky scroll line", STICKY_FOCUSED),
    Builtin("editor.action.goToFocusedStickyScrollLine", "code_editor::GoToFocusedStickyScrollLine", "Go to the focused sticky scroll line", STICKY_FOCUSED),
    Builtin("editor.action.selectEditor", "code_editor::SelectEditor", "Select Editor", STICKY_FOCUSED),
    Builtin("editor.action.hideColorPicker", "code_editor::HideColorPicker", "Hide the Color Picker", "standaloneColorPickerVisible"),
    Builtin("editor.action.insertColorWithStandaloneColorPicker", "code_editor::InsertColorWithStandaloneColorPicker", "Insert Color with Standalone Color Picker", "standaloneColorPickerFocused"),
    Builtin("editor.action.inlineSuggest.showNext", "code_editor::ShowNextInlineSuggestion", "Show Next Inline Suggestion", INLINE_WRITABLE),
    Builtin("editor.action.inlineSuggest.showPrevious", "code_editor::ShowPreviousInlineSuggestion", "Show Previous Inline Suggestion", INLINE_WRITABLE),
    Builtin("editor.action.inlineSuggest.commit", "code_editor::AcceptInlineSuggestion", "Accept Inline Suggestion", INLINE_OR_EDIT),
    Builtin("editor.action.inlineSuggest.jump", "code_editor::JumpToNextInlineEdit", "Jump to next inline edit", "inlineEditIsVisible"),
    Builtin("editor.action.inlineSuggest.commitAlternativeAction", "code_editor::AcceptInlineSuggestionAlternativeAction", "Accept Inline Suggestion Alternative Action", "inlineSuggestionAlternativeActionVisible && inlineEditIsVisible"),
    Builtin("editor.action.inlineSuggest.toggleShowCollapsed", "code_editor::ToggleInlineSuggestionShowCollapsed", "Toggle Inline Suggestions Show Collapsed", ""),
    Builtin("editor.action.inlineSuggest.acceptNextWord", "code_editor::AcceptNextInlineWord", "Accept Next Word Of Inline Suggestion", INLINE_WRITABLE),
    Builtin("editor.action.inlineSuggest.acceptNextLine", "code_editor::AcceptNextInlineLine", "Accept Next Line Of Inline Suggestion", INLINE_WRITABLE),
    Builtin("editor.action.inlineSuggest.hide", "code_editor::HideInlineSuggestion", "Hide Inline Suggestion", INLINE_OR_EDIT),
    Builtin("editor.action.changeAll", "code_editor::SelectAllOccurrences", "Change All Occurrences", WRITABLE),
];

/// An editor command without a label (Monaco `registerEditorCommand`, core
/// commands such as `cursorDown` and their handler commands such as
/// `type`): [`CodeEditor::trigger`] runs it by its Monaco id, with its
/// arguments, but the command palette doesn't list it. Monaco id, GPUI
/// action name, precondition (as for [`Builtin`]).
struct Command(&'static str, &'static str, &'static str);

#[rustfmt::skip]
static COMMANDS: &[Command] = &[
    Command("cursorLeft", "code_editor::CursorLeft", ""),
    Command("cursorLeftSelect", "code_editor::CursorLeftSelect", ""),
    Command("cursorRight", "code_editor::CursorRight", ""),
    Command("cursorRightSelect", "code_editor::CursorRightSelect", ""),
    Command("cursorUp", "code_editor::CursorUp", ""),
    Command("cursorUpSelect", "code_editor::CursorUpSelect", ""),
    Command("cursorDown", "code_editor::CursorDown", ""),
    Command("cursorDownSelect", "code_editor::CursorDownSelect", ""),
    Command("cursorWordLeft", "code_editor::CursorWordLeft", ""),
    Command("cursorWordLeftSelect", "code_editor::CursorWordLeftSelect", ""),
    Command("cursorWordRight", "code_editor::CursorWordRight", ""),
    Command("cursorWordRightSelect", "code_editor::CursorWordRightSelect", ""),
    Command("cursorWordPartLeft", "code_editor::CursorWordPartLeft", ""),
    Command("cursorWordPartLeftSelect", "code_editor::CursorWordPartLeftSelect", ""),
    Command("cursorWordPartRight", "code_editor::CursorWordPartRight", ""),
    Command("cursorWordPartRightSelect", "code_editor::CursorWordPartRightSelect", ""),
    Command("cursorWordAccessibilityLeft", "code_editor::CursorWordAccessibilityLeft", ""),
    Command("cursorWordAccessibilityLeftSelect", "code_editor::CursorWordAccessibilityLeftSelect", ""),
    Command("cursorWordAccessibilityRight", "code_editor::CursorWordAccessibilityRight", ""),
    Command("cursorWordAccessibilityRightSelect", "code_editor::CursorWordAccessibilityRightSelect", ""),
    Command("cursorHome", "code_editor::CursorHome", ""),
    Command("cursorHomeSelect", "code_editor::CursorHomeSelect", ""),
    Command("cursorEnd", "code_editor::CursorEnd", ""),
    Command("cursorEndSelect", "code_editor::CursorEndSelect", ""),
    Command("cursorTop", "code_editor::CursorTop", ""),
    Command("cursorTopSelect", "code_editor::CursorTopSelect", ""),
    Command("cursorBottom", "code_editor::CursorBottom", ""),
    Command("cursorBottomSelect", "code_editor::CursorBottomSelect", ""),
    Command("cursorPageUp", "code_editor::CursorPageUp", ""),
    Command("cursorPageUpSelect", "code_editor::CursorPageUpSelect", ""),
    Command("cursorPageDown", "code_editor::CursorPageDown", ""),
    Command("cursorPageDownSelect", "code_editor::CursorPageDownSelect", ""),
    Command("cursorMove", "code_editor::CursorMove", ""),
    Command("editor.action.selectAll", "code_editor::SelectAll", ""),
    Command("cursorWordStartLeft", "code_editor::CursorWordStartLeft", ""),
    Command("cursorWordStartLeftSelect", "code_editor::CursorWordStartLeftSelect", ""),
    Command("cursorWordStartRight", "code_editor::CursorWordStartRight", ""),
    Command("cursorWordStartRightSelect", "code_editor::CursorWordStartRightSelect", ""),
    Command("cursorWordEndLeft", "code_editor::CursorWordEndLeft", ""),
    Command("cursorWordEndLeftSelect", "code_editor::CursorWordEndLeftSelect", ""),
    Command("cursorWordEndRight", "code_editor::CursorWordEndRight", ""),
    Command("cursorWordEndRightSelect", "code_editor::CursorWordEndRightSelect", ""),
    Command("deleteWordStartLeft", "code_editor::DeleteWordStartLeft", WRITABLE),
    Command("deleteWordStartRight", "code_editor::DeleteWordStartRight", WRITABLE),
    Command("deleteWordEndLeft", "code_editor::DeleteWordEndLeft", WRITABLE),
    Command("deleteWordEndRight", "code_editor::DeleteWordEndRight", WRITABLE),
    Command("deleteWordPartLeft", "code_editor::DeleteWordPartLeft", WRITABLE),
    Command("deleteWordPartRight", "code_editor::DeleteWordPartRight", WRITABLE),
    Command("lineBreakInsert", "code_editor::LineBreakInsert", WRITABLE),
    Command("cursorLineStart", "code_editor::CursorLineStart", ""),
    Command("cursorLineStartSelect", "code_editor::CursorLineStartSelect", ""),
    Command("cursorLineEnd", "code_editor::CursorLineEnd", ""),
    Command("cursorLineEndSelect", "code_editor::CursorLineEndSelect", ""),
    Command("cursorColumnSelectLeft", "code_editor::CursorColumnSelectLeft", ""),
    Command("cursorColumnSelectRight", "code_editor::CursorColumnSelectRight", ""),
    Command("cursorColumnSelectUp", "code_editor::CursorColumnSelectUp", ""),
    Command("cursorColumnSelectDown", "code_editor::CursorColumnSelectDown", ""),
    Command("cursorColumnSelectPageUp", "code_editor::CursorColumnSelectPageUp", ""),
    Command("cursorColumnSelectPageDown", "code_editor::CursorColumnSelectPageDown", ""),
    Command("columnSelect", "code_editor::ColumnSelect", ""),
    Command("createCursor", "code_editor::CreateCursor", ""),
    Command("_moveTo", "code_editor::MoveTo", ""),
    Command("_moveToSelect", "code_editor::MoveToSelect", ""),
    Command("_wordSelect", "code_editor::WordSelect", ""),
    Command("_wordSelectDrag", "code_editor::WordSelectDrag", ""),
    Command("_lineSelect", "code_editor::LineSelect", ""),
    Command("_lineSelectDrag", "code_editor::LineSelectDrag", ""),
    Command("_lastCursorMoveToSelect", "code_editor::LastCursorMoveToSelect", ""),
    Command("lastCursorWordSelect", "code_editor::LastCursorWordSelect", ""),
    Command("lastCursorLineSelect", "code_editor::LastCursorLineSelect", ""),
    Command("lastCursorLineSelectDrag", "code_editor::LastCursorLineSelectDrag", ""),
    Command("editorScroll", "code_editor::EditorScroll", ""),
    Command("scrollLineUp", "code_editor::ScrollLineUp", ""),
    Command("scrollLineDown", "code_editor::ScrollLineDown", ""),
    Command("scrollPageUp", "code_editor::ScrollPageUp", ""),
    Command("scrollPageDown", "code_editor::ScrollPageDown", ""),
    Command("scrollEditorTop", "code_editor::ScrollEditorTop", ""),
    Command("scrollEditorBottom", "code_editor::ScrollEditorBottom", ""),
    Command("scrollLeft", "code_editor::ScrollLeft", ""),
    Command("scrollRight", "code_editor::ScrollRight", ""),
    Command("revealLine", "code_editor::RevealLine", ""),
    Command("setSelection", "code_editor::SetSelection", ""),
    Command("cancelSelection", "code_editor::CancelSelection", "editorHasSelection"),
    // Escape's whole chain: Monaco's own precondition (several cursors)
    // would stop it.
    Command("removeSecondaryCursors", "code_editor::RemoveSecondaryCursors", ""),
    Command("deleteLeft", "code_editor::DeleteLeft", ""),
    Command("deleteRight", "code_editor::DeleteRight", ""),
    Command("deleteWordLeft", "code_editor::DeleteWordLeft", WRITABLE),
    Command("deleteWordRight", "code_editor::DeleteWordRight", WRITABLE),
    // Monaco's `writable` precondition is tested in their handlers:
    // folded into the bindings, GPUI's `!editorReadonly` would test every
    // context up the tree and take Tab from the find widget's inputs in a
    // read-only editor.
    Command("tab", "code_editor::Tab", ""),
    Command("outdent", "code_editor::Outdent", ""),
    Command("undo", "code_editor::Undo", ""),
    Command("redo", "code_editor::Redo", ""),
    // Monaco's overwritable handler commands (`editor.trigger(source,
    // 'type', { text })`), with their `default:` variants as aliases.
    Command("type", "code_editor::Type", ""),
    Command("replacePreviousChar", "code_editor::ReplacePreviousChar", ""),
    Command("compositionType", "code_editor::CompositionType", ""),
    Command("compositionStart", "code_editor::CompositionStart", ""),
    Command("compositionEnd", "code_editor::CompositionEnd", ""),
    Command("paste", "code_editor::PasteText", ""),
    Command("cut", "code_editor::CutText", ""),
    Command("editor.action.clipboardPasteAction", "code_editor::Paste", ""),
    Command("editor.changePasteType", "code_editor::ChangePasteType", "pasteWidgetVisible"),
    Command("editor.changeDropType", "code_editor::ChangeDropType", "dropWidgetVisible"),
    Command("editor.hidePasteWidget", "code_editor::HidePostEditWidget", "pasteWidgetVisible"),
    Command("editor.hideDropWidget", "code_editor::HidePostEditWidget", "dropWidgetVisible"),
    Command("editor.action.replaceOne", "code_editor::ReplaceOne", "findWidgetVisible"),
    Command("editor.action.replaceAll", "code_editor::ReplaceAll", "findWidgetVisible"),
    Command("editor.action.selectAllMatches", "code_editor::SelectAllMatches", "findWidgetVisible"),
    Command("toggleFindCaseSensitive", "code_editor::ToggleCaseSensitive", ""),
    Command("toggleFindRegex", "code_editor::ToggleRegex", ""),
    Command("toggleFindWholeWord", "code_editor::ToggleWholeWord", ""),
    Command("toggleFindInSelection", "code_editor::ToggleFindInSelection", ""),
    Command("togglePreserveCase", "code_editor::TogglePreserveCase", ""),
    Command("closeFindWidget", "code_editor::CloseFindWidget", "findWidgetVisible"),
    Command("history.showPrevious", "code_editor::FindHistoryPrevious", ""),
    Command("history.showNext", "code_editor::FindHistoryNext", ""),
    Command("acceptSelectedSuggestion", "code_editor::AcceptSelectedSuggestion", SUGGEST_VISIBLE),
    Command("acceptAlternativeSelectedSuggestion", "code_editor::AcceptAlternativeSuggestion", SUGGEST_VISIBLE),
    Command("hideSuggestWidget", "code_editor::HideSuggestWidget", SUGGEST_VISIBLE),
    Command("insertBestCompletion", "code_editor::InsertBestCompletion", ""),
    Command("insertNextSuggestion", "code_editor::InsertNextSuggestion", "hasOtherSuggestions"),
    Command("insertPrevSuggestion", "code_editor::InsertPrevSuggestion", "hasOtherSuggestions"),
    Command("toggleSuggestionDetails", "code_editor::ToggleSuggestionDetails", SUGGEST_VISIBLE),
    Command("selectNextSuggestion", "code_editor::SelectNextSuggestion", SUGGEST_NAVIGABLE),
    Command("selectPrevSuggestion", "code_editor::SelectPreviousSuggestion", SUGGEST_NAVIGABLE),
    Command("selectNextPageSuggestion", "code_editor::SelectNextPageSuggestion", SUGGEST_NAVIGABLE),
    Command("selectPrevPageSuggestion", "code_editor::SelectPreviousPageSuggestion", SUGGEST_NAVIGABLE),
    Command("selectFirstSuggestion", "code_editor::SelectFirstSuggestion", SUGGEST_NAVIGABLE),
    Command("selectLastSuggestion", "code_editor::SelectLastSuggestion", SUGGEST_NAVIGABLE),
    Command("focusSuggestion", "code_editor::FocusSuggestion", SUGGEST_VISIBLE),
    Command("focusAndAcceptSuggestion", "code_editor::FocusAndAcceptSuggestion", SUGGEST_VISIBLE),
    Command("toggleExplainMode", "code_editor::ToggleExplainMode", SUGGEST_VISIBLE),
    Command("toggleSuggestionFocus", "code_editor::ToggleSuggestionFocus", SUGGEST_VISIBLE),
    Command("suggestWidgetCopy", "code_editor::SuggestWidgetCopy", "suggestWidgetDetailsFocused"),
    Command("jumpToNextSnippetPlaceholder", "code_editor::JumpToNextSnippetPlaceholder", "inSnippetMode && hasNextTabstop"),
    Command("jumpToPrevSnippetPlaceholder", "code_editor::JumpToPrevSnippetPlaceholder", "inSnippetMode && hasPrevTabstop"),
    Command("leaveSnippet", "code_editor::LeaveSnippet", "inSnippetMode"),
    Command("acceptSnippet", "code_editor::AcceptSnippet", "inSnippetMode"),
    Command("cancelLinkedEditingInput", "code_editor::CancelLinkedEditingInput", "LinkedEditingInputVisible"),
    Command("acceptRenameInput", "code_editor::AcceptRenameInput", "renameInputVisible"),
    Command("acceptRenameInputWithPreview", "code_editor::AcceptRenameInputWithPreview", "renameInputVisible"),
    Command("cancelRenameInput", "code_editor::CancelRenameInput", "renameInputVisible"),
    Command("focusNextRenameSuggestion", "code_editor::FocusNextRenameSuggestion", "renameInputVisible"),
    Command("focusPreviousRenameSuggestion", "code_editor::FocusPreviousRenameSuggestion", "renameInputVisible"),
    Command("leaveEditorMessage", "code_editor::LeaveEditorMessage", "messageVisible"),
    Command("togglePeekWidgetFocus", "code_editor::TogglePeekWidgetFocus", ""),
    Command("editor.action.codeAction", "code_editor::TriggerCodeAction", CODE_ACTIONS),
    Command("closeParameterHints", "code_editor::CloseParameterHints", "parameterHintsVisible"),
    Command("showNextParameterHint", "code_editor::ShowNextParameterHint", "parameterHintsVisible && parameterHintsMultipleSignatures"),
    Command("showPrevParameterHint", "code_editor::ShowPrevParameterHint", "parameterHintsVisible && parameterHintsMultipleSignatures"),
    Command("goToNextReference", "code_editor::GoToNextReference", ""),
    Command("goToPreviousReference", "code_editor::GoToPreviousReference", ""),
    Command("closeReferenceSearch", "code_editor::ClosePeek", ""),
    Command("editor.gotoNextSymbolFromResult", "code_editor::GoToNextSymbolFromResult", "hasSymbols"),
    Command("editor.gotoNextSymbolFromResult.cancel", "code_editor::CancelSymbolNavigation", "hasSymbols"),
    Command("closeMarkersNavigation", "code_editor::CloseMarkersNavigation", "markersNavigationVisible"),
    Command("editor.action.clipboardCutAction", "code_editor::Cut", ""),
    Command("editor.action.clipboardCopyAction", "code_editor::Copy", ""),
    Command("editor.action.hideLongLineWarningHover", "code_editor::HideLongLineWarningHover", ""),
    Command("editor.action.inlineSuggest.toggleAlwaysShowToolbar", "code_editor::ToggleAlwaysShowInlineSuggestionToolbar", ""),
    Command("editor.action.goToLocations", "code_editor::GoToLocations", ""),
    Command("editor.action.peekLocations", "code_editor::PeekLocations", ""),
    Command("editor.action.findReferences", "code_editor::FindReferences", ""),
    Command("editor.action.format", "code_editor::FormatSelectionOrDocument", ""),
    Command("editor.cancelOperation", "code_editor::CancelOperation", "cancellableOperation"),
    Command("editor.action.unicodeHighlight.disableHighlightingOfAmbiguousCharacters", "code_editor::DisableHighlightingOfAmbiguousCharacters", ""),
    Command("editor.action.unicodeHighlight.disableHighlightingOfInvisibleCharacters", "code_editor::DisableHighlightingOfInvisibleCharacters", ""),
    Command("editor.action.unicodeHighlight.disableHighlightingOfNonBasicAsciiCharacters", "code_editor::DisableHighlightingOfNonBasicAsciiCharacters", ""),
    Command("editor.action.unicodeHighlight.showExcludeOptions", "code_editor::ShowUnicodeHighlightExcludeOptions", ""),
];

/// Monaco's command aliases (`CommandsRegistry.registerCommandAlias`, the
/// overwritable handlers' `default:` ids, and ids registered twice): the
/// alias and the id it runs.
#[rustfmt::skip]
static ALIASES: &[(&str, &str)] = &[
    ("acceptSelectedSuggestionOnEnter", "acceptSelectedSuggestion"),
    ("editor.action.smartSelect.grow", "editor.action.smartSelect.expand"),
    ("cursorWordPartStartLeft", "cursorWordPartLeft"),
    ("cursorWordPartStartLeftSelect", "cursorWordPartLeftSelect"),
    ("goToNextReferenceFromEmbeddedEditor", "goToNextReference"),
    ("goToPreviousReferenceFromEmbeddedEditor", "goToPreviousReference"),
    ("closeReferenceSearchEditor", "closeReferenceSearch"),
    ("editor.action.goToDeclaration", "editor.action.revealDefinition"),
    ("editor.action.previewDeclaration", "editor.action.peekDefinition"),
    ("editor.action.openDeclarationToTheSide", "editor.action.revealDefinitionAside"),
    ("editor.action.showReferences", "editor.action.peekLocations"),
    ("default:type", "type"),
    ("default:replacePreviousChar", "replacePreviousChar"),
    ("default:compositionType", "compositionType"),
    ("default:compositionStart", "compositionStart"),
    ("default:compositionEnd", "compositionEnd"),
    ("default:paste", "paste"),
    ("default:cut", "cut"),
    ("default:undo", "undo"),
    ("default:redo", "redo"),
];

/// The id `id` stands for, if it's an alias.
fn resolve_alias(id: &str) -> &str {
    ALIASES
        .iter()
        .find(|(alias, _)| *alias == id)
        .map_or(id, |(_, target)| target)
}

/// The GPUI action name and precondition for the Monaco id (or alias)
/// `id`, labelled or not.
pub(crate) fn action_name(id: &str) -> Option<(&'static str, &'static str)> {
    let id = resolve_alias(id);
    builtin(id)
        .map(|b| (b.1, b.3))
        .or_else(|| COMMANDS.iter().find(|c| c.0 == id).map(|c| (c.1, c.2)))
}

/// The Monaco command id the GPUI action named `name` runs (the first, if
/// several share it), for looking up keybinding rules that name the id.
pub(crate) fn monaco_id(name: &str) -> Option<&'static str> {
    BUILTINS
        .iter()
        .find(|b| b.1 == name)
        .map(|b| b.0)
        .or_else(|| COMMANDS.iter().find(|c| c.1 == name).map(|c| c.0))
}

fn builtin(id: &str) -> Option<&'static Builtin> {
    BUILTINS.iter().find(|b| b.0 == id)
}

/// The precondition of the action named `name` (a GPUI action name), if
/// it's one of Monaco's: what its default keybindings require. An action
/// that runs several Monaco ids needs one of their preconditions.
fn action_precondition(name: &str) -> Option<gpui::KeyBindingContextPredicate> {
    let whens: Vec<&'static str> = BUILTINS
        .iter()
        .filter(|b| b.1 == name)
        .map(|b| b.3)
        .chain(COMMANDS.iter().filter(|c| c.1 == name).map(|c| c.2))
        .collect();
    if whens.is_empty() || whens.iter().any(|when| when.is_empty()) {
        return None;
    }
    whens
        .into_iter()
        .map(|when| (*crate::context_keys::predicate(when)).clone())
        .reduce(|a, b| gpui::KeyBindingContextPredicate::Or(Box::new(a), Box::new(b)))
}

/// Whether `predicate` is evaluated in an editor's own key context (where
/// its context keys are), not a widget's or a parent's.
fn in_editor_context(predicate: &gpui::KeyBindingContextPredicate) -> bool {
    use gpui::KeyBindingContextPredicate as P;
    match predicate {
        P::Identifier(name) => name.as_ref() == KEY_CONTEXT,
        P::And(a, b) => in_editor_context(a) || in_editor_context(b),
        _ => false,
    }
}

/// `predicate` with `when` required where it names one of
/// [`crate::context_keys::WIDGET_KEY_CONTEXTS`] (the outermost, for `A > B`),
/// so a widget's keybinding tests its editor's keys in the widget's own
/// context; `None` if it names none.
fn fold_into_widget_context(
    predicate: &gpui::KeyBindingContextPredicate,
    when: &gpui::KeyBindingContextPredicate,
) -> Option<gpui::KeyBindingContextPredicate> {
    use gpui::KeyBindingContextPredicate as P;
    match predicate {
        P::Identifier(name)
            if crate::context_keys::WIDGET_KEY_CONTEXTS.contains(&name.as_ref()) =>
        {
            Some(P::And(Box::new(predicate.clone()), Box::new(when.clone())))
        }
        P::And(a, b) => fold_into_widget_context(a, when)
            .map(|a| P::And(Box::new(a), b.clone()))
            .or_else(|| fold_into_widget_context(b, when).map(|b| P::And(a.clone(), Box::new(b)))),
        P::Descendant(a, b) => fold_into_widget_context(a, when)
            .map(|a| P::Descendant(Box::new(a), b.clone()))
            .or_else(|| {
                fold_into_widget_context(b, when).map(|b| P::Descendant(a.clone(), Box::new(b)))
            }),
        _ => None,
    }
}

/// Folds each binding's action's precondition into its context, as Monaco
/// does for an action's keybinding (`editorExtensions.ts`): when the
/// precondition fails, the key falls through, so Cmd+/ in a read-only
/// editor does nothing (and reaches the host). Bindings in a widget's key
/// context (the hover, the rename input, the peek list, …) get it in that
/// context, which carries the editor's keys
/// ([`crate::context_keys::widget_key_context`]): Up in a focused hover
/// (`editor.action.scrollUpHover`) tests `editorHoverFocused` there, and
/// Enter in the rename input `renameInputVisible`.
pub(crate) fn with_preconditions(bindings: Vec<KeyBinding>) -> Vec<KeyBinding> {
    bindings
        .into_iter()
        .map(|binding| {
            let Some(when) = action_precondition(binding.action().name()) else {
                return binding;
            };
            let Some(context) = binding.predicate() else {
                return binding;
            };
            let predicate = if in_editor_context(&context) {
                gpui::KeyBindingContextPredicate::And(Box::new((*context).clone()), Box::new(when))
            } else if let Some(folded) = fold_into_widget_context(&context, &when) {
                folded
            } else {
                return binding;
            };
            let keystrokes: Vec<String> = binding
                .keystrokes()
                .iter()
                .map(|keystroke| keystroke.inner().unparse())
                .collect();
            KeyBinding::load(
                &keystrokes.join(" "),
                binding.action().boxed_clone(),
                Some(Rc::new(predicate)),
                false,
                None,
                &gpui::DummyKeyboardMapper,
            )
            .unwrap_or(binding)
        })
        .collect()
}

/// The command palette's history (Monaco `CommandsHistory`): recently run
/// action ids, shared by every editor.
#[derive(Default)]
struct CommandsHistory {
    counter: u64,
    entries: HashMap<SharedString, u64>,
}

impl Global for CommandsHistory {}

/// Monaco's default `workbench.commandPalette.history`.
const HISTORY_SIZE: usize = 50;

/// Records that the palette ran `id`.
pub(crate) fn push_command_history(id: SharedString, cx: &mut App) {
    let history = cx.default_global::<CommandsHistory>();
    history.counter += 1;
    let counter = history.counter;
    history.entries.insert(id, counter);
    if history.entries.len() > HISTORY_SIZE {
        let oldest = history
            .entries
            .iter()
            .min_by_key(|(_, counter)| **counter)
            .map(|(id, _)| id.clone());
        if let Some(oldest) = oldest {
            history.entries.remove(&oldest);
        }
    }
}

/// When the palette last ran `id` (higher is more recent).
pub(crate) fn command_history(id: &str, cx: &App) -> Option<u64> {
    cx.try_global::<CommandsHistory>()?.entries.get(id).copied()
}

/// The actions added to every editor with [`add_editor_action`], oldest
/// first.
#[derive(Default)]
struct GlobalActions {
    next_id: u64,
    actions: Vec<(u64, EditorActionDescriptor)>,
}

impl Global for GlobalActions {}

/// The actions from [`add_editor_action`], newest first (the newest with an
/// id wins).
pub(crate) fn global_actions(cx: &App) -> Vec<EditorActionDescriptor> {
    let Some(global) = cx.try_global::<GlobalActions>() else {
        return Vec::new();
    };
    let mut actions: Vec<EditorActionDescriptor> = Vec::new();
    for (_, action) in global.actions.iter().rev() {
        if !actions.iter().any(|a| a.id == action.id) {
            actions.push(action.clone());
        }
    }
    actions
}

/// Adds an action to every editor, now and later (Monaco
/// `editor.addEditorAction`), until the returned [`Disposable`] is
/// disposed. [`CodeEditor::run_action`], [`CodeEditor::trigger`] and
/// [`RunAction`] run it (in the focused editor for its keybindings), and
/// with a group it's in the context menu. As in Monaco, where it's a
/// command rather than an editor action, it isn't in
/// [`CodeEditor::supported_actions`] (the command palette). An editor's own
/// action with the same id ([`CodeEditor::add_action`]) wins.
pub fn add_editor_action(cx: &mut App, descriptor: EditorActionDescriptor) -> Disposable {
    let bindings = action_key_bindings(&descriptor, KEY_CONTEXT);
    let keybindings = bind_removable(cx, bindings);
    let global = cx.default_global::<GlobalActions>();
    global.next_id += 1;
    let entry_id = global.next_id;
    global.actions.push((entry_id, descriptor));
    cx.refresh_windows();
    Disposable::new(move |cx| {
        keybindings.dispose(cx);
        cx.default_global::<GlobalActions>()
            .actions
            .retain(|(id, _)| *id != entry_id);
        cx.refresh_windows();
    })
}

/// The keybindings of a host action, in key context `context` and the
/// action's `keybinding_context` (Monaco's `keybindingsWhen`, the
/// precondition and `keybindingContext`). Those that don't parse are
/// left out.
fn action_key_bindings(descriptor: &EditorActionDescriptor, context: &str) -> Vec<KeyBinding> {
    let when = match &descriptor.keybinding_context {
        Some(when) => format!("{context} && ({when})"),
        None => context.to_string(),
    };
    let Ok(predicate) = gpui::KeyBindingContextPredicate::parse(&when) else {
        return Vec::new();
    };
    let predicate = Rc::new(predicate);
    descriptor
        .keybindings
        .iter()
        .filter_map(|keystrokes| {
            KeyBinding::load(
                keystrokes,
                Box::new(RunAction {
                    id: descriptor.id.to_string(),
                }),
                Some(predicate.clone()),
                false,
                None,
                &gpui::DummyKeyboardMapper,
            )
            .ok()
        })
        .collect()
}

/// Monaco `IKeybindingRule`: keystrokes that run a command, for
/// [`add_keybinding_rules`].
#[derive(Clone, Debug, PartialEq)]
pub struct KeybindingRule {
    /// The keystrokes, in GPUI syntax, where Monaco takes a
    /// `KeyMod | KeyCode` number: `secondary-` is `KeyMod.CtrlCmd` (Cmd on
    /// macOS, Ctrl elsewhere), `ctrl-` is `KeyMod.WinCtrl`, and a chord is
    /// space-separated keystrokes (e.g. `"secondary-k secondary-m"`).
    pub keystrokes: SharedString,
    /// The command: an editor action id (built-in or added), a Monaco
    /// command such as `"cursorDown"`, a command from [`crate::add_command`]
    /// or [`crate::register_command`], or a GPUI action name. `None`
    /// unbinds the keystrokes (Monaco `command: null`). A `-` before the
    /// id removes the command's default bindings instead; see
    /// [`KeybindingRule::remove`].
    pub command: Option<SharedString>,
    /// The argument for a registered command (Monaco `commandArgs`).
    pub command_args: Option<serde_json::Value>,
    /// Where it applies, as a GPUI key context predicate (Monaco `when`,
    /// a context key expression); `None` for the editor's key context.
    pub when: Option<SharedString>,
}

impl KeybindingRule {
    /// A rule binding `keystrokes` to `command` in the editor.
    pub fn new(keystrokes: impl Into<SharedString>, command: impl Into<SharedString>) -> Self {
        Self {
            keystrokes: keystrokes.into(),
            command: Some(command.into()),
            command_args: None,
            when: None,
        }
    }

    /// A rule unbinding `keystrokes` in the editor (Monaco `command: null`).
    pub fn unbind(keystrokes: impl Into<SharedString>) -> Self {
        Self {
            keystrokes: keystrokes.into(),
            command: None,
            command_args: None,
            when: None,
        }
    }

    /// A rule removing `command`'s default bindings of `keystrokes`, or
    /// of every keystroke if `keystrokes` is empty (Monaco's `-command`,
    /// e.g. `-actions.find`). Other commands' bindings of the keystrokes
    /// still apply. With [`KeybindingRule::when`], only where it holds.
    pub fn remove(keystrokes: impl Into<SharedString>, command: &str) -> Self {
        Self {
            keystrokes: keystrokes.into(),
            command: Some(format!("-{command}").into()),
            command_args: None,
            when: None,
        }
    }

    /// Sets the command's argument.
    pub fn args(mut self, args: serde_json::Value) -> Self {
        self.command_args = Some(args);
        self
    }

    /// Sets where it applies.
    pub fn when(mut self, when: impl Into<SharedString>) -> Self {
        self.when = Some(when.into());
        self
    }
}

/// Adds a keybinding rule (Monaco `editor.addKeybindingRule`); see
/// [`add_keybinding_rules`].
pub fn add_keybinding_rule(cx: &mut App, rule: KeybindingRule) -> anyhow::Result<Disposable> {
    add_keybinding_rules(cx, vec![rule])
}

/// Adds keybinding rules (Monaco `editor.addKeybindingRules`), after (so
/// above) the existing bindings, until the returned [`Disposable`] is
/// disposed; the bindings they shadowed then apply again. A rule's
/// command runs in the focused editor with [`RunCommand`]. Fails, adding
/// nothing, if some keystrokes or `when` don't parse.
pub fn add_keybinding_rules(
    cx: &mut App,
    rules: Vec<KeybindingRule>,
) -> anyhow::Result<Disposable> {
    let mut bindings = Vec::new();
    for rule in rules {
        let when = rule.when.as_deref().unwrap_or(KEY_CONTEXT);
        let predicate = Rc::new(gpui::KeyBindingContextPredicate::parse(when)?);
        if let Some(removed) = rule.command.as_deref().and_then(|c| c.strip_prefix('-')) {
            bindings.extend(removal_bindings(cx, removed, &rule.keystrokes, predicate)?);
            continue;
        }
        let action: Box<dyn gpui::Action> = match rule.command {
            Some(command) => Box::new(RunCommand {
                id: command.to_string(),
                args: rule.command_args,
            }),
            None => Box::new(gpui::NoAction),
        };
        bindings.push(KeyBinding::load(
            &rule.keystrokes,
            action,
            Some(predicate),
            false,
            None,
            &gpui::DummyKeyboardMapper,
        )?);
    }
    Ok(bind_removable(cx, bindings))
}

/// A `-command` rule's bindings (Monaco `KeybindingResolver.handleRemovals`):
/// [`gpui::Unbind`] markers that take the bindings of command `id`'s
/// action off `keystrokes`, or off every keystroke it's bound to if
/// `keystrokes` is empty, where `predicate` holds. Other bindings of the
/// keystrokes still apply.
fn removal_bindings(
    cx: &App,
    id: &str,
    keystrokes: &str,
    predicate: Rc<gpui::KeyBindingContextPredicate>,
) -> anyhow::Result<Vec<KeyBinding>> {
    let name = action_name(id).map_or(id, |(name, _)| name);
    let keystrokes = if keystrokes.trim().is_empty() {
        let keymap = cx.key_bindings();
        let keymap = keymap.borrow();
        let mut bound: Vec<String> = keymap
            .bindings()
            .filter(|binding| binding.action().name() == name)
            .map(|binding| {
                binding
                    .keystrokes()
                    .iter()
                    .map(|keystroke| keystroke.inner().unparse())
                    .collect::<Vec<_>>()
                    .join(" ")
            })
            .collect();
        bound.sort();
        bound.dedup();
        bound
    } else {
        vec![keystrokes.to_string()]
    };
    keystrokes
        .iter()
        .map(|keystrokes| {
            Ok(KeyBinding::load(
                keystrokes,
                Box::new(gpui::Unbind(name.to_string().into())),
                Some(predicate.clone()),
                false,
                None,
                &gpui::DummyKeyboardMapper,
            )?)
        })
        .collect()
}

/// Binds `bindings`, each with a context, until the returned disposable
/// removes exactly them (told apart by their predicates' identity), so
/// GPUI's keymap, which can't remove bindings, works like Monaco's dynamic
/// keybindings.
pub(crate) fn bind_removable(cx: &mut App, bindings: Vec<KeyBinding>) -> Disposable {
    if bindings.is_empty() {
        return Disposable::none();
    }
    let predicates: Vec<_> = bindings.iter().filter_map(KeyBinding::predicate).collect();
    debug_assert_eq!(predicates.len(), bindings.len());
    cx.bind_keys(bindings);
    Disposable::new(move |cx| {
        let keymap = cx.key_bindings();
        let kept: Vec<KeyBinding> = keymap
            .borrow()
            .bindings()
            .filter(|binding| {
                !binding
                    .predicate()
                    .is_some_and(|p| predicates.iter().any(|ours| Rc::ptr_eq(ours, &p)))
            })
            .cloned()
            .collect();
        cx.clear_key_bindings();
        cx.bind_keys(kept);
    })
}

impl CodeEditor {
    /// The key context predicate for this editor's own keybindings, with
    /// `when` (a GPUI predicate over its [context keys](Self::context_keys))
    /// if given.
    fn own_keybinding_context(&self, when: Option<&str>) -> String {
        let editor = format!(
            "{KEY_CONTEXT} && {} == {}",
            crate::context_keys::EDITOR_ID_KEY,
            self.commands.id
        );
        match when {
            Some(when) => format!("{editor} && ({when})"),
            None => editor,
        }
    }

    /// Adds an action (Monaco `addAction`) until the returned
    /// [`Disposable`] is disposed (or [`CodeEditor::remove_action`]): the
    /// command palette lists it, [`CodeEditor::run_action`] and
    /// [`RunAction`] run it, and its keybindings run it in this editor only.
    pub fn add_action(
        &mut self,
        descriptor: EditorActionDescriptor,
        cx: &mut Context<Self>,
    ) -> Disposable {
        let context = self.own_keybinding_context(None);
        let bindings = action_key_bindings(&descriptor, &context);
        let id = descriptor.id.clone();
        self.remove_action_bindings(&id, cx);
        self.commands
            .action_bindings
            .insert(id.clone(), bind_removable(cx, bindings));
        self.added_actions.retain(|a| a.id != descriptor.id);
        self.added_actions.push(descriptor);
        let editor = cx.weak_entity();
        Disposable::new(move |cx| {
            editor
                .update(cx, |editor, cx| editor.remove_action_now(&id, cx))
                .ok();
        })
    }

    /// Removes an action added with [`CodeEditor::add_action`], with its
    /// keybindings.
    pub fn remove_action(&mut self, id: &str) -> bool {
        let len = self.added_actions.len();
        self.added_actions.retain(|a| a.id.as_ref() != id);
        // Unbinding needs the app: on the next frame.
        if let Some(bindings) = self.commands.action_bindings.remove(id) {
            self.commands.pending_dispose.push(bindings);
        }
        self.added_actions.len() != len
    }

    fn remove_action_now(&mut self, id: &str, cx: &mut Context<Self>) {
        self.added_actions.retain(|a| a.id.as_ref() != id);
        self.remove_action_bindings(id, cx);
        cx.notify();
    }

    fn remove_action_bindings(&mut self, id: &str, cx: &mut Context<Self>) {
        if let Some(bindings) = self.commands.action_bindings.remove(id) {
            bindings.dispose(cx);
        }
    }

    /// Adds a command run by `keystrokes` (GPUI syntax, e.g. `"cmd-k
    /// cmd-m"`) in this editor (Monaco `IStandaloneCodeEditor.addCommand`),
    /// while `when` (a GPUI predicate over the editor's
    /// [context keys](Self::context_keys), e.g. one from
    /// [`CodeEditor::create_context_key`]) holds. Returns the command's
    /// generated id, which [`CodeEditor::trigger`] and keybinding rules can
    /// run too, and the [`Disposable`] that removes it. Fails if the
    /// keystrokes or `when` don't parse.
    pub fn add_command(
        &mut self,
        keystrokes: &str,
        handler: impl Fn(&mut CodeEditor, &mut Window, &mut Context<CodeEditor>) + 'static,
        when: Option<&str>,
        cx: &mut Context<Self>,
    ) -> anyhow::Result<(SharedString, Disposable)> {
        let id: SharedString = crate::editor_commands::next_dynamic_command_id().into();
        let predicate =
            gpui::KeyBindingContextPredicate::parse(&self.own_keybinding_context(when))?;
        let binding = KeyBinding::load(
            keystrokes,
            Box::new(RunCommand {
                id: id.to_string(),
                args: None,
            }),
            Some(Rc::new(predicate)),
            false,
            None,
            &gpui::DummyKeyboardMapper,
        )?;
        let bindings = bind_removable(cx, vec![binding]);
        self.commands.handlers.insert(id.clone(), Rc::new(handler));
        let editor = cx.weak_entity();
        let handler_id = id.clone();
        let disposable = Disposable::new(move |cx| {
            bindings.dispose(cx);
            editor
                .update(cx, |editor, _| editor.commands.handlers.remove(&handler_id))
                .ok();
        });
        Ok((id, disposable))
    }

    /// The actions that can run now (Monaco `getSupportedActions`): the
    /// built-in ones whose precondition holds, then the added ones.
    pub fn supported_actions(&self, cx: &App) -> Vec<EditorActionInfo> {
        let keys = [self.context_keys(cx)];
        let builtins = BUILTINS
            .iter()
            .filter(|b| b.3.is_empty() || crate::context_keys::predicate(b.3).eval(&keys))
            .map(|b| EditorActionInfo {
                id: b.0.into(),
                label: b.2.into(),
                alias: None,
            });
        let added = self
            .added_actions
            .iter()
            .filter(|a| a.precondition.as_ref().is_none_or(|p| p(self, cx)))
            .map(|a| EditorActionInfo {
                id: a.id.clone(),
                label: a.label.clone(),
                alias: a.alias.clone(),
            });
        builtins.chain(added).collect()
    }

    /// Whether the built-in action `id` can run now (its precondition holds).
    pub(crate) fn action_enabled(&self, id: &str, cx: &App) -> bool {
        builtin(resolve_alias(id)).is_some_and(|b| self.when(b.3, cx))
    }

    /// Runs the action `id` if it's supported (Monaco
    /// `getAction(id).run()`); built-in ones run as if their key were
    /// pressed.
    pub fn run_action(&mut self, id: &str, window: &mut Window, cx: &mut Context<Self>) -> bool {
        self.run_action_with_args(id, None, window, cx)
    }

    /// [`CodeEditor::run_action`] with an argument (Monaco
    /// `getAction(id).run(args)`): a host action's body gets it, and a
    /// built-in one is built from it as [`CodeEditor::trigger_with_payload`]
    /// builds its payload (ignoring an argument it can't be built from).
    pub fn run_action_with_args(
        &mut self,
        id: &str,
        args: Option<serde_json::Value>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        if let Some(ran) = self.run_host_action(id, args.clone(), window, cx) {
            return ran;
        }
        let Some(builtin) = builtin(resolve_alias(id)) else {
            return false;
        };
        if !self.when(builtin.3, cx) {
            return false;
        }
        let Ok(action) = args
            .and_then(|args| cx.build_action(builtin.1, Some(args)).ok())
            .map_or_else(|| cx.build_action(builtin.1, None), Ok)
        else {
            return false;
        };
        self.dispatch_to_editor(action, true, window, cx);
        true
    }

    /// Dispatches `action` as if its key were pressed, focusing the editor
    /// first if `focus`. While one of the editor's widgets (the find input,
    /// a focused hover, …) has the focus rather than its text, it goes to
    /// the editor itself and the focus stays: Monaco runs a command on the
    /// editor it resolved (with widget focus, `getFocusedCodeEditor`),
    /// while dispatching to the focused element would run it in the
    /// widget's input.
    fn dispatch_to_editor(
        &self,
        action: Box<dyn gpui::Action>,
        focus: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.focus_handle.is_focused(window) && self.focus_handle.contains_focused(window, cx) {
            let focus_handle = self.focus_handle.clone();
            window.defer(cx, move |window, cx| {
                focus_handle.dispatch_action(action.as_ref(), window, cx)
            });
            return;
        }
        if focus {
            window.focus(&self.focus_handle, cx);
        }
        window.dispatch_action(action, cx);
    }

    /// Runs the host action `id` ([`CodeEditor::add_action`] or
    /// [`add_editor_action`]) with `args`: `None` if there's none, else
    /// whether its precondition let it run.
    fn run_host_action(
        &mut self,
        id: &str,
        args: Option<serde_json::Value>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<bool> {
        let action = self
            .added_actions
            .iter()
            .find(|a| a.id.as_ref() == id)
            .cloned()
            .or_else(|| global_actions(cx).into_iter().find(|a| a.id.as_ref() == id))?;
        if !action.precondition.as_ref().is_none_or(|p| p(self, cx)) {
            return Some(false);
        }
        (action.run)(self, args, window, cx);
        Some(true)
    }

    /// Whether action `id` (built-in or the host's) exists and its
    /// precondition holds.
    fn action_supported(&self, id: &str, cx: &App) -> bool {
        let added = self
            .added_actions
            .iter()
            .find(|a| a.id.as_ref() == id)
            .cloned()
            .or_else(|| global_actions(cx).into_iter().find(|a| a.id.as_ref() == id));
        match added {
            Some(action) => action.precondition.as_ref().is_none_or(|p| p(self, cx)),
            None => builtin(resolve_alias(id)).is_some_and(|b| self.when(b.3, cx)),
        }
    }

    /// Every action of this editor, supported now or not (Monaco
    /// `getActions`): the built-in ones, then the added ones.
    pub fn actions(&self) -> Vec<EditorActionInfo> {
        let builtins = BUILTINS.iter().map(|b| EditorActionInfo {
            id: b.0.into(),
            label: b.2.into(),
            alias: None,
        });
        let added = self.added_actions.iter().map(|a| EditorActionInfo {
            id: a.id.clone(),
            label: a.label.clone(),
            alias: a.alias.clone(),
        });
        builtins.chain(added).collect()
    }

    /// The action with id `id` (Monaco `getAction`): one of
    /// [`CodeEditor::actions`], supported now or not, or an action from
    /// [`add_editor_action`] (which Monaco registers as a command, so its
    /// `getAction` doesn't find it). Aliases of built-in ids resolve.
    pub fn get_action(&self, id: &str, cx: &Context<Self>) -> Option<EditorAction> {
        let info = match self.added_actions.iter().find(|a| a.id.as_ref() == id) {
            Some(a) => EditorActionInfo {
                id: a.id.clone(),
                label: a.label.clone(),
                alias: a.alias.clone(),
            },
            None => match builtin(resolve_alias(id)) {
                Some(b) => EditorActionInfo {
                    id: b.0.into(),
                    label: b.2.into(),
                    alias: None,
                },
                None => {
                    let a = global_actions(cx)
                        .into_iter()
                        .find(|a| a.id.as_ref() == id)?;
                    EditorActionInfo {
                        id: a.id,
                        label: a.label,
                        alias: a.alias,
                    }
                }
            },
        };
        Some(EditorAction {
            info,
            editor: cx.weak_entity(),
        })
    }

    /// Runs command `id` as a keybinding rule's [`RunCommand`] does
    /// (Monaco's `commandService.executeCommand`): one of this editor's
    /// commands ([`CodeEditor::add_command`]), an editor action, a Monaco
    /// command (with `args`), a registered command (with `args`), or a GPUI
    /// action. Returns whether one was found.
    pub(crate) fn run_command(
        &mut self,
        id: &str,
        args: Option<serde_json::Value>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        if let Some(handler) = self.commands.handlers.get(id).cloned() {
            handler(self, window, cx);
            return true;
        }
        // Host actions take the rule's `commandArgs` (Monaco `run(accessor,
        // ...args)`).
        if self.run_host_action(id, args.clone(), window, cx) == Some(true) {
            return true;
        }
        if args.is_none() && self.run_action(id, window, cx) {
            return true;
        }
        if let Some((name, when)) = action_name(id) {
            if !self.when(when, cx) {
                // Monaco runs nothing, as the precondition fails.
                return true;
            }
            // Commands with a payload (`cursorMove`, `editor.fold`, …) take
            // `args` when they fit it.
            if let Ok(action) = args
                .clone()
                .and_then(|args| cx.build_action(name, Some(args)).ok())
                .map_or_else(|| cx.build_action(name, None), Ok)
            {
                self.dispatch_to_editor(action, false, window, cx);
                return true;
            }
        }
        if crate::commands::has_command(cx, id) {
            let command = providers::Command {
                id: id.to_string().into(),
                title: SharedString::default(),
                arguments: args.into_iter().collect(),
                ..Default::default()
            };
            return crate::commands::execute_command(cx, &command).is_ok();
        }
        if self.run_action(id, window, cx) {
            return true;
        }
        match cx.build_action(id, None) {
            Ok(action) => {
                self.dispatch_to_editor(action, false, window, cx);
                true
            }
            Err(_) => false,
        }
    }

    /// Runs a provider's command (a completion item's or a code action's)
    /// as Monaco's `commandService.executeCommand` does, where editor
    /// actions are commands too: a registered command with all its
    /// arguments; else, once the current update is done (to get the
    /// editor's window), whatever [`CodeEditor::run_command`] finds with
    /// the first argument (`editor.action.triggerParameterHints`,
    /// `editor.action.rename`, …); else [`CodeEditorEvent::ExecuteCommand`]
    /// for the host, as for code lenses.
    ///
    /// [`CodeEditorEvent::ExecuteCommand`]: crate::CodeEditorEvent::ExecuteCommand
    pub(crate) fn run_provider_command(
        &mut self,
        command: providers::Command,
        cx: &mut Context<Self>,
    ) {
        if crate::commands::execute_command(cx, &command).is_ok() {
            return;
        }
        let window = self.window_id.and_then(|id| {
            cx.windows()
                .into_iter()
                .find(|window| window.window_id() == id)
        });
        let Some(window) = window else {
            cx.emit(crate::CodeEditorEvent::ExecuteCommand(command));
            return;
        };
        let editor = cx.weak_entity();
        cx.defer(move |cx| {
            let ran = window.update(cx, |_, window, cx| {
                editor.update(cx, |editor, cx| {
                    let args = command.arguments.first().cloned();
                    if !editor.run_command(&command.id, args, window, cx) {
                        cx.emit(crate::CodeEditorEvent::ExecuteCommand(command));
                    }
                })
            });
            if let Err(err) = ran {
                log::warn!("{err:#}");
            }
        });
    }

    /// The keystrokes bound to each supported action in this editor, as
    /// Monaco labels them (`ResolvedKeybinding.getLabel()`).
    pub(crate) fn action_keybindings(
        &self,
        window: &Window,
        cx: &App,
    ) -> HashMap<SharedString, crate::keybinding_label::KeyLabel> {
        let text = |binding: KeyBinding| crate::keybinding_label::KeyLabel::for_binding(&binding);
        let mut keys = HashMap::new();
        for b in BUILTINS {
            let Ok(action) = cx.build_action(b.1, None) else {
                continue;
            };
            if let Some(binding) = crate::keybinding_label::command_binding(
                action.as_ref(),
                Some(b.0),
                &self.focus_handle,
                window,
            ) {
                keys.insert(SharedString::from(b.0), text(binding));
            }
        }
        for a in &self.added_actions {
            let action = RunAction {
                id: a.id.to_string(),
            };
            if let Some(binding) =
                window.highest_precedence_binding_for_action_in(&action, &self.focus_handle)
            {
                keys.insert(a.id.clone(), text(binding));
            }
        }
        keys
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[gpui::test]
    fn builtins_name_real_actions(cx: &mut gpui::TestAppContext) {
        cx.update(|cx| {
            for b in BUILTINS {
                assert!(
                    cx.build_action(b.1, None).is_ok(),
                    "{} is not an action",
                    b.1
                );
            }
        });
        let mut ids: Vec<_> = BUILTINS.iter().map(|b| b.0).collect();
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), BUILTINS.len());
    }

    #[gpui::test]
    fn commands_name_real_actions(cx: &mut gpui::TestAppContext) {
        cx.update(|cx| {
            for c in COMMANDS {
                assert!(
                    cx.build_action(c.1, None).is_ok(),
                    "{} is not an action",
                    c.1
                );
            }
        });
        // Each id once, and never also a labelled action.
        let mut ids: Vec<_> = COMMANDS.iter().map(|c| c.0).collect();
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), COMMANDS.len());
        assert!(COMMANDS.iter().all(|c| builtin(c.0).is_none()));
    }

    #[test]
    fn preconditions_parse_and_aliases_resolve() {
        for when in BUILTINS
            .iter()
            .map(|b| b.3)
            .chain(COMMANDS.iter().map(|c| c.2))
        {
            if !when.is_empty() {
                assert!(
                    gpui::KeyBindingContextPredicate::parse(when).is_ok(),
                    "bad precondition {when}"
                );
            }
        }
        for (alias, target) in ALIASES {
            assert!(action_name(target).is_some(), "{alias} -> {target}");
            assert!(
                builtin(alias).is_none() && !COMMANDS.iter().any(|c| c.0 == *alias),
                "{alias} is also an id"
            );
        }
        assert_eq!(
            action_name("editor.action.goToDeclaration").map(|a| a.0),
            Some("code_editor::GoToDefinition")
        );
    }

    #[gpui::test]
    fn history_keeps_the_most_recent(cx: &mut gpui::TestAppContext) {
        cx.update(|cx| {
            for i in 0..=HISTORY_SIZE {
                push_command_history(format!("a{i}").into(), cx);
            }
            assert_eq!(command_history("a0", cx), None);
            assert!(command_history("a50", cx) > command_history("a49", cx));
            push_command_history("a1".into(), cx);
            assert!(command_history("a1", cx) > command_history("a50", cx));
        });
    }
}
