//! The editor's actions (Monaco `IEditorAction`): the built-in ones with a
//! label, and the host's from [`CodeEditor::add_action`] (Monaco
//! `addAction`). The command palette lists the supported ones.

use std::collections::HashMap;
use std::rc::Rc;

use gpui::{App, Context, Global, KeyBinding, SharedString, Window};

use crate::actions::{KEY_CONTEXT, RunAction};
use crate::editor::CodeEditor;
use crate::providers;

/// Whether an action can run in an editor (Monaco's precondition).
type Precondition = fn(&CodeEditor, &App) -> bool;

/// A host action's body.
pub type ActionRun = Rc<dyn Fn(&mut CodeEditor, &mut Window, &mut Context<CodeEditor>)>;

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
    /// An action with `id`, `label` and its body.
    pub fn new(
        id: impl Into<SharedString>,
        label: impl Into<SharedString>,
        run: impl Fn(&mut CodeEditor, &mut Window, &mut Context<CodeEditor>) + 'static,
    ) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            alias: None,
            keybindings: Vec::new(),
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

/// A built-in action: Monaco id, GPUI action name, label, precondition.
struct Builtin(&'static str, &'static str, &'static str, Precondition);

fn always(_: &CodeEditor, _: &App) -> bool {
    true
}

fn sticky_visible(editor: &CodeEditor, _: &App) -> bool {
    editor.options.sticky_scroll.enabled
        && editor
            .layout
            .as_ref()
            .is_some_and(|l| !l.sticky_lines.is_empty())
}

fn writable(editor: &CodeEditor, _: &App) -> bool {
    !editor.options.read_only
}

/// Monaco `inlineSuggestionVisible`.
fn inline_visible(editor: &CodeEditor, cx: &App) -> bool {
    editor.inline_suggestion_visible(cx)
}

fn inline_visible_writable(editor: &CodeEditor, cx: &App) -> bool {
    writable(editor, cx) && inline_visible(editor, cx)
}

/// Monaco `standaloneColorPickerVisible`.
fn color_picker_visible(editor: &CodeEditor, _: &App) -> bool {
    editor.is_standalone_color_picker_visible()
}

/// Monaco `hoverVisible` (Monaco requires `hoverFocused` for scrolling,
/// which preconditions here can't see).
fn hover_visible(editor: &CodeEditor, _: &App) -> bool {
    editor.is_hover_visible()
}

/// Monaco `stickyScrollFocused`.
fn sticky_focused(editor: &CodeEditor, _: &App) -> bool {
    editor.sticky.focused.is_some()
}

/// Whether some `providers::$lookup` serves the editor's language.
macro_rules! has_provider {
    ($name:ident, $lookup:ident) => {
        fn $name(editor: &CodeEditor, cx: &App) -> bool {
            let language = editor.model().read(cx).language_id().map(|l| l.to_string());
            !providers::$lookup(cx, language.as_deref()).is_empty()
        }
    };
    ($name:ident, writable, $lookup:ident) => {
        fn $name(editor: &CodeEditor, cx: &App) -> bool {
            let language = editor.model().read(cx).language_id().map(|l| l.to_string());
            writable(editor, cx) && !providers::$lookup(cx, language.as_deref()).is_empty()
        }
    };
}

has_provider!(has_definition, definition_providers);
has_provider!(has_declaration, declaration_providers);
has_provider!(has_type_definition, type_definition_providers);
has_provider!(has_implementation, implementation_providers);
has_provider!(has_references, reference_providers);
has_provider!(has_signature_help, signature_help_providers);
has_provider!(can_format, writable, document_formatting_providers);
has_provider!(
    can_format_selection,
    writable,
    document_range_formatting_providers
);
has_provider!(can_rename, writable, rename_providers);
has_provider!(can_code_action, writable, code_action_providers);

/// Monaco's labelled editor actions that this editor implements.
#[rustfmt::skip]
static BUILTINS: &[Builtin] = &[
    Builtin("editor.action.quickCommand", "code_editor::QuickCommand", "Command Palette", always),
    Builtin("editor.action.gotoLine", "code_editor::GotoLine", "Go to Line/Column...", always),
    Builtin("editor.action.quickOutline", "code_editor::QuickOutline", "Go to Symbol...", always),
    // Cursors and selections.
    Builtin("expandLineSelection", "code_editor::ExpandLineSelection", "Expand Line Selection", always),
    Builtin("editor.action.insertCursorAbove", "code_editor::AddCursorAbove", "Add Cursor Above", always),
    Builtin("editor.action.insertCursorBelow", "code_editor::AddCursorBelow", "Add Cursor Below", always),
    Builtin("editor.action.insertCursorAtEndOfEachLineSelected", "code_editor::InsertCursorAtEndOfEachLineSelected", "Add Cursors to Line Ends", always),
    Builtin("editor.action.addCursorsToBottom", "code_editor::AddCursorsToBottom", "Add Cursors to Bottom", always),
    Builtin("editor.action.addCursorsToTop", "code_editor::AddCursorsToTop", "Add Cursors to Top", always),
    Builtin("editor.action.addSelectionToNextFindMatch", "code_editor::AddSelectionToNextFindMatch", "Add Selection to Next Find Match", always),
    Builtin("editor.action.addSelectionToPreviousFindMatch", "code_editor::AddSelectionToPreviousFindMatch", "Add Selection to Previous Find Match", always),
    Builtin("editor.action.moveSelectionToNextFindMatch", "code_editor::MoveSelectionToNextFindMatch", "Move Last Selection to Next Find Match", always),
    Builtin("editor.action.moveSelectionToPreviousFindMatch", "code_editor::MoveSelectionToPreviousFindMatch", "Move Last Selection to Previous Find Match", always),
    Builtin("editor.action.selectHighlights", "code_editor::SelectAllOccurrences", "Select All Occurrences of Find Match", always),
    Builtin("editor.action.focusNextCursor", "code_editor::FocusNextCursor", "Focus Next Cursor", always),
    Builtin("editor.action.focusPreviousCursor", "code_editor::FocusPreviousCursor", "Focus Previous Cursor", always),
    Builtin("cursorUndo", "code_editor::CursorUndo", "Cursor Undo", always),
    Builtin("cursorRedo", "code_editor::CursorRedo", "Cursor Redo", always),
    Builtin("editor.action.setSelectionAnchor", "code_editor::SetSelectionAnchor", "Set Selection Anchor", always),
    Builtin("editor.action.goToSelectionAnchor", "code_editor::GoToSelectionAnchor", "Go to Selection Anchor", always),
    Builtin("editor.action.selectFromAnchorToCursor", "code_editor::SelectFromAnchorToCursor", "Select from Anchor to Cursor", always),
    Builtin("editor.action.cancelSelectionAnchor", "code_editor::CancelSelectionAnchor", "Cancel Selection Anchor", always),
    Builtin("editor.action.smartSelect.expand", "code_editor::ExpandSelection", "Expand Selection", always),
    Builtin("editor.action.smartSelect.shrink", "code_editor::ShrinkSelection", "Shrink Selection", always),
    Builtin("editor.action.toggleOvertypeInsertMode", "code_editor::ToggleOvertypeInsertMode", "Toggle Overtype/Insert Mode", always),
    // Find.
    Builtin("actions.find", "code_editor::StartFind", "Find", always),
    Builtin("editor.action.startFindReplaceAction", "code_editor::StartFindReplace", "Replace", always),
    Builtin("editor.action.nextMatchFindAction", "code_editor::NextMatch", "Find Next", always),
    Builtin("editor.action.previousMatchFindAction", "code_editor::PreviousMatch", "Find Previous", always),
    // Brackets, problems, highlights.
    Builtin("editor.action.jumpToBracket", "code_editor::JumpToBracket", "Go to Bracket", always),
    Builtin("editor.action.selectToBracket", "code_editor::SelectToBracket", "Select to Bracket", always),
    Builtin("editor.action.marker.next", "code_editor::GoToNextMarker", "Go to Next Problem (Error, Warning, Info)", always),
    Builtin("editor.action.marker.prev", "code_editor::GoToPreviousMarker", "Go to Previous Problem (Error, Warning, Info)", always),
    Builtin("editor.action.marker.nextInFiles", "code_editor::GoToNextMarkerInFiles", "Go to Next Problem in Files (Error, Warning, Info)", always),
    Builtin("editor.action.marker.prevInFiles", "code_editor::GoToPreviousMarkerInFiles", "Go to Previous Problem in Files (Error, Warning, Info)", always),
    Builtin("editor.action.wordHighlight.next", "code_editor::WordHighlightNext", "Go to Next Symbol Highlight", always),
    Builtin("editor.action.wordHighlight.prev", "code_editor::WordHighlightPrev", "Go to Previous Symbol Highlight", always),
    Builtin("editor.action.wordHighlight.trigger", "code_editor::WordHighlightTrigger", "Trigger Symbol Highlight", always),
    Builtin("editor.action.showHover", "code_editor::ShowHover", "Show or Focus Hover", always),
    Builtin("editor.action.hideHover", "code_editor::HideHover", "Hide Hover", always),
    Builtin("editor.action.scrollUpHover", "code_editor::ScrollUpHover", "Scroll Up Hover", hover_visible),
    Builtin("editor.action.scrollDownHover", "code_editor::ScrollDownHover", "Scroll Down Hover", hover_visible),
    Builtin("editor.action.scrollLeftHover", "code_editor::ScrollLeftHover", "Scroll Left Hover", hover_visible),
    Builtin("editor.action.scrollRightHover", "code_editor::ScrollRightHover", "Scroll Right Hover", hover_visible),
    Builtin("editor.action.pageUpHover", "code_editor::PageUpHover", "Page Up Hover", hover_visible),
    Builtin("editor.action.pageDownHover", "code_editor::PageDownHover", "Page Down Hover", hover_visible),
    Builtin("editor.action.goToTopHover", "code_editor::GoToTopHover", "Go To Top Hover", hover_visible),
    Builtin("editor.action.goToBottomHover", "code_editor::GoToBottomHover", "Go To Bottom Hover", hover_visible),
    Builtin("editor.action.increaseHoverVerbosityLevel", "code_editor::IncreaseHoverVerbosityLevel", "Increase Hover Verbosity Level", hover_visible),
    Builtin("editor.action.decreaseHoverVerbosityLevel", "code_editor::DecreaseHoverVerbosityLevel", "Decrease Hover Verbosity Level", hover_visible),
    Builtin("editor.action.openLink", "code_editor::OpenLink", "Open Link", always),
    Builtin("editor.action.fontZoomIn", "code_editor::FontZoomIn", "Increase Editor Font Size", always),
    Builtin("editor.action.fontZoomOut", "code_editor::FontZoomOut", "Decrease Editor Font Size", always),
    Builtin("editor.action.fontZoomReset", "code_editor::FontZoomReset", "Reset Editor Font Size", always),
    Builtin("editor.action.toggleStickyScroll", "code_editor::ToggleStickyScroll", "Toggle Editor Sticky Scroll", always),
    Builtin("editor.action.focusStickyScroll", "code_editor::FocusStickyScroll", "Focus Editor Sticky Scroll", sticky_visible),
    Builtin("editor.action.showContextMenu", "code_editor::ShowContextMenu", "Show Editor Context Menu", always),
    Builtin("editor.action.showOrFocusStandaloneColorPicker", "code_editor::ShowOrFocusStandaloneColorPicker", "Show or Focus Standalone Color Picker", always),
    // Editing.
    Builtin("deleteAllLeft", "code_editor::DeleteAllLeft", "Delete All Left", writable),
    Builtin("deleteAllRight", "code_editor::DeleteAllRight", "Delete All Right", writable),
    Builtin("editor.action.sortLinesAscending", "code_editor::SortLinesAscending", "Sort Lines Ascending", writable),
    Builtin("editor.action.sortLinesDescending", "code_editor::SortLinesDescending", "Sort Lines Descending", writable),
    Builtin("editor.action.removeDuplicateLines", "code_editor::DeleteDuplicateLines", "Delete Duplicate Lines", writable),
    Builtin("editor.action.reverseLines", "code_editor::ReverseLines", "Reverse lines", writable),
    Builtin("editor.action.trimTrailingWhitespace", "code_editor::TrimTrailingWhitespace", "Trim Trailing Whitespace", writable),
    Builtin("editor.action.deleteLines", "code_editor::DeleteLines", "Delete Line", writable),
    Builtin("editor.action.joinLines", "code_editor::JoinLines", "Join Lines", writable),
    Builtin("editor.action.transpose", "code_editor::Transpose", "Transpose Characters around the Cursor", writable),
    Builtin("editor.action.transposeLetters", "code_editor::TransposeLetters", "Transpose Letters", writable),
    Builtin("editor.action.transformToUppercase", "code_editor::TransformToUppercase", "Transform to Uppercase", writable),
    Builtin("editor.action.transformToLowercase", "code_editor::TransformToLowercase", "Transform to Lowercase", writable),
    Builtin("editor.action.transformToTitlecase", "code_editor::TransformToTitlecase", "Transform to Title Case", writable),
    Builtin("editor.action.transformToSnakecase", "code_editor::TransformToSnakecase", "Transform to Snake Case", writable),
    Builtin("editor.action.transformToCamelcase", "code_editor::TransformToCamelcase", "Transform to Camel Case", writable),
    Builtin("editor.action.transformToPascalcase", "code_editor::TransformToPascalcase", "Transform to Pascal Case", writable),
    Builtin("editor.action.transformToKebabcase", "code_editor::TransformToKebabcase", "Transform to Kebab Case", writable),
    Builtin("editor.action.duplicateSelection", "code_editor::DuplicateSelection", "Duplicate Selection", writable),
    Builtin("editor.action.moveCarretLeftAction", "code_editor::MoveCaretLeft", "Move Selected Text Left", writable),
    Builtin("editor.action.moveCarretRightAction", "code_editor::MoveCaretRight", "Move Selected Text Right", writable),
    Builtin("editor.action.reindentlines", "code_editor::ReindentLines", "Reindent Lines", writable),
    Builtin("editor.action.reindentselectedlines", "code_editor::ReindentSelectedLines", "Reindent Selected Lines", writable),
    Builtin("editor.action.inPlaceReplace.up", "code_editor::InPlaceReplaceUp", "Replace with Previous Value", writable),
    Builtin("editor.action.inPlaceReplace.down", "code_editor::InPlaceReplaceDown", "Replace with Next Value", writable),
    Builtin("editor.action.removeBrackets", "code_editor::RemoveBrackets", "Remove Brackets", writable),
    Builtin("editor.action.commentLine", "code_editor::CommentLine", "Toggle Line Comment", writable),
    Builtin("editor.action.addCommentLine", "code_editor::AddCommentLine", "Add Line Comment", writable),
    Builtin("editor.action.removeCommentLine", "code_editor::RemoveCommentLine", "Remove Line Comment", writable),
    Builtin("editor.action.blockComment", "code_editor::BlockComment", "Toggle Block Comment", writable),
    Builtin("editor.action.indentLines", "code_editor::IndentLines", "Indent Line", writable),
    Builtin("editor.action.outdentLines", "code_editor::OutdentLines", "Outdent Line", writable),
    Builtin("editor.action.moveLinesUpAction", "code_editor::MoveLinesUp", "Move Line Up", writable),
    Builtin("editor.action.moveLinesDownAction", "code_editor::MoveLinesDown", "Move Line Down", writable),
    Builtin("editor.action.copyLinesUpAction", "code_editor::CopyLinesUp", "Copy Line Up", writable),
    Builtin("editor.action.copyLinesDownAction", "code_editor::CopyLinesDown", "Copy Line Down", writable),
    Builtin("editor.action.insertLineAfter", "code_editor::InsertLineAfter", "Insert Line Below", writable),
    Builtin("editor.action.insertLineBefore", "code_editor::InsertLineBefore", "Insert Line Above", writable),
    Builtin("editor.action.linkedEditing", "code_editor::StartLinkedEditing", "Start Linked Editing", writable),
    Builtin("editor.action.triggerSuggest", "code_editor::TriggerSuggest", "Trigger Suggest", writable),
    Builtin("editor.action.inlineSuggest.trigger", "code_editor::TriggerInlineSuggest", "Trigger Inline Suggestion", writable),
    Builtin("editor.action.triggerParameterHints", "code_editor::TriggerParameterHints", "Trigger Parameter Hints", has_signature_help),
    // Folding.
    Builtin("editor.fold", "code_editor::Fold", "Fold", always),
    Builtin("editor.unfold", "code_editor::Unfold", "Unfold", always),
    Builtin("editor.toggleFold", "code_editor::ToggleFold", "Toggle Fold", always),
    Builtin("editor.foldRecursively", "code_editor::FoldRecursively", "Fold Recursively", always),
    Builtin("editor.unfoldRecursively", "code_editor::UnfoldRecursively", "Unfold Recursively", always),
    Builtin("editor.toggleFoldRecursively", "code_editor::ToggleFoldRecursively", "Toggle Fold Recursively", always),
    Builtin("editor.foldAll", "code_editor::FoldAll", "Fold All", always),
    Builtin("editor.unfoldAll", "code_editor::UnfoldAll", "Unfold All", always),
    Builtin("editor.foldAllBlockComments", "code_editor::FoldAllBlockComments", "Fold All Block Comments", always),
    Builtin("editor.foldAllMarkerRegions", "code_editor::FoldAllMarkerRegions", "Fold All Regions", always),
    Builtin("editor.unfoldAllMarkerRegions", "code_editor::UnfoldAllMarkerRegions", "Unfold All Regions", always),
    Builtin("editor.foldAllExcept", "code_editor::FoldAllExcept", "Fold All Except Selected", always),
    Builtin("editor.unfoldAllExcept", "code_editor::UnfoldAllExcept", "Unfold All Except Selected", always),
    Builtin("editor.foldLevel1", "code_editor::FoldLevel1", "Fold Level 1", always),
    Builtin("editor.foldLevel2", "code_editor::FoldLevel2", "Fold Level 2", always),
    Builtin("editor.foldLevel3", "code_editor::FoldLevel3", "Fold Level 3", always),
    Builtin("editor.foldLevel4", "code_editor::FoldLevel4", "Fold Level 4", always),
    Builtin("editor.foldLevel5", "code_editor::FoldLevel5", "Fold Level 5", always),
    Builtin("editor.foldLevel6", "code_editor::FoldLevel6", "Fold Level 6", always),
    Builtin("editor.foldLevel7", "code_editor::FoldLevel7", "Fold Level 7", always),
    Builtin("editor.gotoParentFold", "code_editor::GoToParentFold", "Go to Parent Fold", always),
    Builtin("editor.gotoPreviousFold", "code_editor::GoToPreviousFold", "Go to Previous Folding Range", always),
    Builtin("editor.gotoNextFold", "code_editor::GoToNextFold", "Go to Next Folding Range", always),
    Builtin("editor.createFoldingRangeFromSelection", "code_editor::CreateFoldFromSelection", "Create Folding Range from Selection", always),
    Builtin("editor.removeManualFoldingRanges", "code_editor::RemoveManualFoldingRanges", "Remove Manual Folding Ranges", always),
    Builtin("editor.toggleImportFold", "code_editor::ToggleImportFold", "Toggle Import Fold", always),
    // Language features.
    Builtin("editor.action.formatDocument", "code_editor::FormatDocument", "Format Document", can_format),
    Builtin("editor.action.formatSelection", "code_editor::FormatSelection", "Format Selection", can_format_selection),
    Builtin("editor.action.rename", "code_editor::Rename", "Rename Symbol", can_rename),
    Builtin("editor.action.toggleTabFocusMode", "code_editor::ToggleTabFocusMode", "Toggle Tab Key Moves Focus", always),
    Builtin("editor.action.pasteAs", "code_editor::PasteAs", "Paste As...", writable),
    Builtin("editor.action.pasteAsText", "code_editor::PasteAsText", "Paste as Text", writable),
    Builtin("editor.action.quickFix", "code_editor::QuickFix", "Quick Fix...", can_code_action),
    Builtin("editor.action.refactor", "code_editor::Refactor", "Refactor...", can_code_action),
    Builtin("editor.action.sourceAction", "code_editor::SourceAction", "Source Action...", can_code_action),
    Builtin("editor.action.organizeImports", "code_editor::OrganizeImports", "Organize Imports", can_code_action),
    Builtin("editor.action.fixAll", "code_editor::FixAll", "Fix All", can_code_action),
    Builtin("editor.action.autoFix", "code_editor::AutoFix", "Auto Fix...", can_code_action),
    Builtin("editor.action.revealDefinition", "code_editor::GoToDefinition", "Go to Definition", has_definition),
    Builtin("editor.action.peekDefinition", "code_editor::PeekDefinition", "Peek Definition", has_definition),
    Builtin("editor.action.revealDeclaration", "code_editor::GoToDeclaration", "Go to Declaration", has_declaration),
    Builtin("editor.action.peekDeclaration", "code_editor::PeekDeclaration", "Peek Declaration", has_declaration),
    Builtin("editor.action.goToTypeDefinition", "code_editor::GoToTypeDefinition", "Go to Type Definition", has_type_definition),
    Builtin("editor.action.peekTypeDefinition", "code_editor::PeekTypeDefinition", "Peek Type Definition", has_type_definition),
    Builtin("editor.action.goToImplementation", "code_editor::GoToImplementation", "Go to Implementations", has_implementation),
    Builtin("editor.action.peekImplementation", "code_editor::PeekImplementation", "Peek Implementations", has_implementation),
    Builtin("editor.action.goToReferences", "code_editor::GoToReferences", "Go to References", has_references),
    Builtin("editor.action.referenceSearch.trigger", "code_editor::PeekReferences", "Peek References", has_references),
    // Indentation.
    Builtin("editor.action.indentationToSpaces", "code_editor::IndentationToSpaces", "Convert Indentation to Spaces", writable),
    Builtin("editor.action.indentationToTabs", "code_editor::IndentationToTabs", "Convert Indentation to Tabs", writable),
    Builtin("editor.action.indentUsingTabs", "code_editor::IndentUsingTabs", "Indent Using Tabs", always),
    Builtin("editor.action.indentUsingSpaces", "code_editor::IndentUsingSpaces", "Indent Using Spaces", always),
    Builtin("editor.action.changeTabDisplaySize", "code_editor::ChangeTabDisplaySize", "Change Tab Display Size", always),
    Builtin("editor.action.detectIndentation", "code_editor::DetectIndentation", "Detect Indentation from Content", always),
    // Find, inline suggestions, the color picker, sticky scroll.
    Builtin("deleteInsideWord", "code_editor::DeleteInsideWord", "Delete Word", writable),
    Builtin("actions.findWithSelection", "code_editor::StartFindWithSelection", "Find with Selection", always),
    Builtin("editor.action.nextSelectionMatchFindAction", "code_editor::NextSelectionMatch", "Find Next Selection", always),
    Builtin("editor.action.previousSelectionMatchFindAction", "code_editor::PreviousSelectionMatch", "Find Previous Selection", always),
    Builtin("editor.action.selectNextStickyScrollLine", "code_editor::SelectNextStickyScrollLine", "Select the next editor sticky scroll line", sticky_focused),
    Builtin("editor.action.selectPreviousStickyScrollLine", "code_editor::SelectPreviousStickyScrollLine", "Select the previous sticky scroll line", sticky_focused),
    Builtin("editor.action.goToFocusedStickyScrollLine", "code_editor::GoToFocusedStickyScrollLine", "Go to the focused sticky scroll line", sticky_focused),
    Builtin("editor.action.selectEditor", "code_editor::SelectEditor", "Select Editor", sticky_focused),
    Builtin("editor.action.hideColorPicker", "code_editor::HideColorPicker", "Hide the Color Picker", color_picker_visible),
    Builtin("editor.action.insertColorWithStandaloneColorPicker", "code_editor::InsertColorWithStandaloneColorPicker", "Insert Color with Standalone Color Picker", color_picker_visible),
    Builtin("editor.action.inlineSuggest.showNext", "code_editor::ShowNextInlineSuggestion", "Show Next Inline Suggestion", inline_visible_writable),
    Builtin("editor.action.inlineSuggest.showPrevious", "code_editor::ShowPreviousInlineSuggestion", "Show Previous Inline Suggestion", inline_visible_writable),
    Builtin("editor.action.inlineSuggest.commit", "code_editor::AcceptInlineSuggestion", "Accept Inline Suggestion", inline_visible),
    Builtin("editor.action.inlineSuggest.acceptNextWord", "code_editor::AcceptNextInlineWord", "Accept Next Word Of Inline Suggestion", inline_visible_writable),
    Builtin("editor.action.inlineSuggest.acceptNextLine", "code_editor::AcceptNextInlineLine", "Accept Next Line Of Inline Suggestion", inline_visible_writable),
    Builtin("editor.action.inlineSuggest.hide", "code_editor::HideInlineSuggestion", "Hide Inline Suggestion", inline_visible),
    Builtin("editor.action.changeAll", "code_editor::SelectAllOccurrences", "Change All Occurrences", writable),
];

/// An editor command without a label (Monaco `registerEditorCommand` and
/// core commands such as `cursorDown`): [`CodeEditor::trigger`] runs it by
/// its Monaco id, but the command palette doesn't list it.
struct Command(&'static str, &'static str);

#[rustfmt::skip]
static COMMANDS: &[Command] = &[
    Command("cursorLeft", "code_editor::CursorLeft"),
    Command("cursorLeftSelect", "code_editor::CursorLeftSelect"),
    Command("cursorRight", "code_editor::CursorRight"),
    Command("cursorRightSelect", "code_editor::CursorRightSelect"),
    Command("cursorUp", "code_editor::CursorUp"),
    Command("cursorUpSelect", "code_editor::CursorUpSelect"),
    Command("cursorDown", "code_editor::CursorDown"),
    Command("cursorDownSelect", "code_editor::CursorDownSelect"),
    Command("cursorWordLeft", "code_editor::CursorWordLeft"),
    Command("cursorWordLeftSelect", "code_editor::CursorWordLeftSelect"),
    Command("cursorWordRight", "code_editor::CursorWordRight"),
    Command("cursorWordRightSelect", "code_editor::CursorWordRightSelect"),
    Command("cursorWordPartLeft", "code_editor::CursorWordPartLeft"),
    Command("cursorWordPartLeftSelect", "code_editor::CursorWordPartLeftSelect"),
    Command("cursorWordPartRight", "code_editor::CursorWordPartRight"),
    Command("cursorWordPartRightSelect", "code_editor::CursorWordPartRightSelect"),
    Command("cursorHome", "code_editor::CursorHome"),
    Command("cursorHomeSelect", "code_editor::CursorHomeSelect"),
    Command("cursorEnd", "code_editor::CursorEnd"),
    Command("cursorEndSelect", "code_editor::CursorEndSelect"),
    Command("cursorTop", "code_editor::CursorTop"),
    Command("cursorTopSelect", "code_editor::CursorTopSelect"),
    Command("cursorBottom", "code_editor::CursorBottom"),
    Command("cursorBottomSelect", "code_editor::CursorBottomSelect"),
    Command("cursorPageUp", "code_editor::CursorPageUp"),
    Command("cursorPageUpSelect", "code_editor::CursorPageUpSelect"),
    Command("cursorPageDown", "code_editor::CursorPageDown"),
    Command("cursorPageDownSelect", "code_editor::CursorPageDownSelect"),
    Command("editor.action.selectAll", "code_editor::SelectAll"),
    Command("cursorWordStartLeft", "code_editor::CursorWordStartLeft"),
    Command("cursorWordStartLeftSelect", "code_editor::CursorWordStartLeftSelect"),
    Command("cursorWordStartRight", "code_editor::CursorWordStartRight"),
    Command("cursorWordStartRightSelect", "code_editor::CursorWordStartRightSelect"),
    Command("cursorWordEndLeft", "code_editor::CursorWordEndLeft"),
    Command("cursorWordEndLeftSelect", "code_editor::CursorWordEndLeftSelect"),
    Command("cursorWordEndRight", "code_editor::CursorWordEndRight"),
    Command("cursorWordEndRightSelect", "code_editor::CursorWordEndRightSelect"),
    Command("deleteWordStartLeft", "code_editor::DeleteWordStartLeft"),
    Command("deleteWordStartRight", "code_editor::DeleteWordStartRight"),
    Command("deleteWordEndLeft", "code_editor::DeleteWordEndLeft"),
    Command("deleteWordEndRight", "code_editor::DeleteWordEndRight"),
    Command("deleteWordPartLeft", "code_editor::DeleteWordPartLeft"),
    Command("deleteWordPartRight", "code_editor::DeleteWordPartRight"),
    Command("lineBreakInsert", "code_editor::LineBreakInsert"),
    Command("cursorLineStart", "code_editor::CursorLineStart"),
    Command("cursorLineStartSelect", "code_editor::CursorLineStartSelect"),
    Command("cursorLineEnd", "code_editor::CursorLineEnd"),
    Command("cursorLineEndSelect", "code_editor::CursorLineEndSelect"),
    Command("cursorColumnSelectLeft", "code_editor::CursorColumnSelectLeft"),
    Command("cursorColumnSelectRight", "code_editor::CursorColumnSelectRight"),
    Command("cursorColumnSelectUp", "code_editor::CursorColumnSelectUp"),
    Command("cursorColumnSelectDown", "code_editor::CursorColumnSelectDown"),
    Command("cursorColumnSelectPageUp", "code_editor::CursorColumnSelectPageUp"),
    Command("cursorColumnSelectPageDown", "code_editor::CursorColumnSelectPageDown"),
    Command("scrollLineUp", "code_editor::ScrollLineUp"),
    Command("scrollLineDown", "code_editor::ScrollLineDown"),
    Command("scrollPageUp", "code_editor::ScrollPageUp"),
    Command("scrollPageDown", "code_editor::ScrollPageDown"),
    Command("scrollEditorTop", "code_editor::ScrollEditorTop"),
    Command("scrollEditorBottom", "code_editor::ScrollEditorBottom"),
    Command("cancelSelection", "code_editor::CancelSelection"),
    Command("removeSecondaryCursors", "code_editor::RemoveSecondaryCursors"),
    Command("deleteLeft", "code_editor::DeleteLeft"),
    Command("deleteRight", "code_editor::DeleteRight"),
    Command("deleteWordLeft", "code_editor::DeleteWordLeft"),
    Command("deleteWordRight", "code_editor::DeleteWordRight"),
    Command("tab", "code_editor::Tab"),
    Command("outdent", "code_editor::Outdent"),
    Command("undo", "code_editor::Undo"),
    Command("redo", "code_editor::Redo"),
    Command("editor.action.clipboardPasteAction", "code_editor::Paste"),
    Command("editor.changePasteType", "code_editor::ChangePasteType"),
    Command("editor.changeDropType", "code_editor::ChangeDropType"),
    Command("editor.hidePasteWidget", "code_editor::HidePostEditWidget"),
    Command("editor.action.replaceOne", "code_editor::ReplaceOne"),
    Command("editor.action.replaceAll", "code_editor::ReplaceAll"),
    Command("editor.action.selectAllMatches", "code_editor::SelectAllMatches"),
    Command("toggleFindCaseSensitive", "code_editor::ToggleCaseSensitive"),
    Command("toggleFindRegex", "code_editor::ToggleRegex"),
    Command("toggleFindWholeWord", "code_editor::ToggleWholeWord"),
    Command("toggleFindInSelection", "code_editor::ToggleFindInSelection"),
    Command("togglePreserveCase", "code_editor::TogglePreserveCase"),
    Command("history.showPrevious", "code_editor::FindHistoryPrevious"),
    Command("history.showNext", "code_editor::FindHistoryNext"),
    Command("acceptAlternativeSelectedSuggestion", "code_editor::AcceptAlternativeSuggestion"),
    Command("toggleSuggestionDetails", "code_editor::ToggleSuggestionDetails"),
    Command("selectNextSuggestion", "code_editor::SelectNextSuggestion"),
    Command("selectPrevSuggestion", "code_editor::SelectPreviousSuggestion"),
    Command("selectNextPageSuggestion", "code_editor::SelectNextPageSuggestion"),
    Command("selectPrevPageSuggestion", "code_editor::SelectPreviousPageSuggestion"),
    Command("selectFirstSuggestion", "code_editor::SelectFirstSuggestion"),
    Command("selectLastSuggestion", "code_editor::SelectLastSuggestion"),
    Command("closeParameterHints", "code_editor::CloseParameterHints"),
    Command("showNextParameterHint", "code_editor::ShowNextParameterHint"),
    Command("showPrevParameterHint", "code_editor::ShowPrevParameterHint"),
    Command("goToNextReference", "code_editor::GoToNextReference"),
    Command("goToPreviousReference", "code_editor::GoToPreviousReference"),
    Command("closeReferenceSearch", "code_editor::ClosePeek"),
    Command("editor.action.clipboardCutAction", "code_editor::Cut"),
    Command("editor.action.clipboardCopyAction", "code_editor::Copy"),
    Command("editor.action.hideLongLineWarningHover", "code_editor::HideLongLineWarningHover"),
    Command("editor.action.inlineSuggest.toggleAlwaysShowToolbar", "code_editor::ToggleAlwaysShowInlineSuggestionToolbar"),
];

/// The GPUI action name for the unlabelled Monaco command `id`.
pub(crate) fn command_action_name(id: &str) -> Option<&'static str> {
    COMMANDS.iter().find(|c| c.0 == id).map(|c| c.1)
}

fn builtin(id: &str) -> Option<&'static Builtin> {
    BUILTINS.iter().find(|b| b.0 == id)
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

impl CodeEditor {
    /// Adds an action (Monaco `addAction`): the command palette lists it,
    /// [`CodeEditor::run_action`] and [`RunAction`] run it, and its
    /// keybindings are bound to [`RunAction`] in the editor's key context.
    pub fn add_action(&mut self, descriptor: EditorActionDescriptor, cx: &mut Context<Self>) {
        let bindings: Vec<KeyBinding> = descriptor
            .keybindings
            .iter()
            .map(|keystrokes| {
                KeyBinding::new(
                    keystrokes,
                    RunAction {
                        id: descriptor.id.to_string(),
                    },
                    Some(KEY_CONTEXT),
                )
            })
            .collect();
        if !bindings.is_empty() {
            cx.bind_keys(bindings);
        }
        self.added_actions.retain(|a| a.id != descriptor.id);
        self.added_actions.push(descriptor);
    }

    /// Removes an action added with [`CodeEditor::add_action`]. Its
    /// keybindings stay bound but do nothing in this editor.
    pub fn remove_action(&mut self, id: &str) -> bool {
        let len = self.added_actions.len();
        self.added_actions.retain(|a| a.id.as_ref() != id);
        self.added_actions.len() != len
    }

    /// The actions that can run now (Monaco `getSupportedActions`): the
    /// built-in ones whose precondition holds, then the added ones.
    pub fn supported_actions(&self, cx: &App) -> Vec<EditorActionInfo> {
        let builtins = BUILTINS
            .iter()
            .filter(|b| (b.3)(self, cx))
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

    /// Runs the action `id` if it's supported (Monaco
    /// `getAction(id).run()`); built-in ones run as if their key were
    /// pressed.
    pub fn run_action(&mut self, id: &str, window: &mut Window, cx: &mut Context<Self>) -> bool {
        if let Some(action) = self.added_actions.iter().find(|a| a.id.as_ref() == id) {
            if !action.precondition.as_ref().is_none_or(|p| p(self, cx)) {
                return false;
            }
            let run = action.run.clone();
            run(self, window, cx);
            return true;
        }
        let Some(builtin) = builtin(id) else {
            return false;
        };
        if !(builtin.3)(self, cx) {
            return false;
        }
        let Ok(action) = cx.build_action(builtin.1, None) else {
            return false;
        };
        window.focus(&self.focus_handle, cx);
        window.dispatch_action(action, cx);
        true
    }

    /// The keystrokes bound to each supported action in this editor, for
    /// display.
    pub(crate) fn action_keybindings(
        &self,
        window: &Window,
        cx: &App,
    ) -> HashMap<SharedString, SharedString> {
        let text = |binding: KeyBinding| -> SharedString {
            binding
                .keystrokes()
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join(" ")
                .into()
        };
        let mut keys = HashMap::new();
        for b in BUILTINS {
            let Ok(action) = cx.build_action(b.1, None) else {
                continue;
            };
            if let Some(binding) =
                window.highest_precedence_binding_for_action_in(action.as_ref(), &self.focus_handle)
            {
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
