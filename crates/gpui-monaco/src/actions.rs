//! Editor commands as GPUI actions, and Monaco's default keybindings.
//!
//! Every binding lives in the [`KEY_CONTEXT`] key context, so hosts can
//! override any of them by binding the same keystroke in that context after
//! [`crate::init`], or skip [`default_key_bindings`] entirely and bind their
//! own. As in Monaco, the text's own keys (typing, deleting, moving the
//! cursor, …) also need `textInputFocus`, so they don't act while one of
//! the editor's widgets (a hover, the find widget's buttons, a menu, the
//! peek list, …) has the focus.

use gpui::{Action, App, KeyBinding, actions};

/// The key context the editor element establishes.
pub const KEY_CONTEXT: &str = "CodeEditor";

/// The editor while the suggest widget is open (Monaco's
/// `suggestWidgetVisible`; its keys need `textInputFocus`).
const SUGGEST: Option<&str> = Some("CodeEditor && editorTextFocus && suggest_widget_visible");
/// While the suggest list's keys move in it (Monaco's `selectNextSuggestion`
/// precondition: `suggestWidgetMultipleSuggestions ||
/// !suggestWidgetHasFocusedSuggestion`); with a single focused suggestion
/// they move the cursor instead.
const SUGGEST_NAVIGABLE: Option<&str> = Some(
    "CodeEditor && editorTextFocus && suggest_widget_visible && (suggest_widget_multiple_suggestions || !suggest_widget_has_focused_suggestion)",
);
/// While the suggest widget's details panel has the focus (Monaco's
/// `suggestWidgetDetailsFocused`).
const SUGGEST_DETAILS_FOCUSED: Option<&str> = Some("CodeEditor && suggest_widget_details_focused");
/// The quick access input (inside the quick input widget).
const QUICK_INPUT: Option<&str> = Some("QuickInput > CodeEditor");
/// The editor while a sticky line has keyboard focus.
const STICKY_FOCUSED: Option<&str> = Some("CodeEditor && sticky_scroll_focused");
/// Monaco `ctxReferenceSearchVisible || PeekContext.inPeekEditor`: a peek
/// view is open in this editor, or this editor is a peek view's preview.
const REFERENCE_SEARCH: Option<&str> =
    Some("CodeEditor && (referenceSearchVisible || inReferenceSearchEditor)");
/// While the drop or paste options button shows.
const POST_EDIT_WIDGET: Option<&str> = Some("CodeEditor && post_edit_widget");
/// Where Monaco's `tab` and `outdent` don't apply, so Tab falls through to
/// the browser, which moves the focus: they need `tabDoesNotMoveFocus` and,
/// as their precondition, `writable`. (Single-line inputs leave Tab to
/// their widget.) Like Monaco's `tab` and `outdent`, they need
/// `editorTextFocus`: in a widget, Tab moves among its own controls
/// ([`FocusNextElement`]).
const TAB_MOVES_FOCUS: Option<&str> =
    Some("CodeEditor && editorTextFocus && !single_line && (tab_moves_focus || editorReadonly)");
/// The Tab rules that outweigh [`TAB_MOVES_FOCUS`], as they don't test
/// `tabMovesFocus` or `writable`: `acceptSelectedSuggestion`,
/// `jumpToNextSnippetPlaceholder`, and the tab completion's
/// `insertNextSuggestion` and `insertBestCompletion`. (Ghost text's
/// `inlineSuggest.commit` and `inlineSuggest.jump` require
/// `!tabMovesFocus`.) [`Tab`]'s handler picks among them.
const TAB_OUTWEIGHS_FOCUS: Option<&str> = Some(
    "CodeEditor && editorTextFocus && (suggest_widget_has_focused_suggestion || hasNextTabstop || ((hasOtherSuggestions || atEndOfWord) && !suggest_widget_visible && !inSnippetMode))",
);
/// The Shift+Tab rules that outweigh [`TAB_MOVES_FOCUS`]:
/// `jumpToPrevSnippetPlaceholder`, `insertPrevSuggestion` and
/// `inlineSuggest.commitAlternativeAction`. [`Outdent`]'s handler picks
/// among them.
const SHIFT_TAB_OUTWEIGHS_FOCUS: Option<&str> = Some(
    "CodeEditor && editorTextFocus && (hasPrevTabstop || inline_suggestion_alternative_action_visible || (hasOtherSuggestions && !suggest_widget_visible && !inSnippetMode))",
);
/// The context menu.
const CONTEXT_MENU: Option<&str> = Some("EditorContextMenu");
/// The find widget's inputs.
const FIND_INPUT: Option<&str> = Some("FindWidget > CodeEditor");
/// The find widget's find input (Monaco `findInputFocussed`).
const FIND_INPUT_ONLY: Option<&str> = Some("FindInput > CodeEditor");
/// The find widget's replace input (Monaco `replaceInputFocussed`).
const REPLACE_INPUT_ONLY: Option<&str> = Some("ReplaceInput > CodeEditor");
/// The standalone color picker.
const STANDALONE_COLOR_PICKER: Option<&str> = Some("StandaloneColorPicker");
/// The editor while ghost text shows (Monaco's `inlineSuggestionVisible`).
const INLINE: Option<&str> = Some("CodeEditor && inline_suggestion_visible");
/// The editor while parameter hints show (Monaco's `parameterHintsVisible`).
const PARAMETER_HINTS: Option<&str> = Some("CodeEditor && parameter_hints_visible");
/// While parameter hints show several signatures (Monaco's
/// `parameterHintsMultipleSignatures`).
const PARAMETER_HINTS_MULTIPLE: Option<&str> =
    Some("CodeEditor && parameter_hints_multiple_signatures");
/// As [`PARAMETER_HINTS_MULTIPLE`], for keys the suggest widget takes first
/// (Monaco weighs its list keys above parameter hints').
const PARAMETER_HINTS_MULTIPLE_NO_SUGGEST: Option<&str> =
    Some("CodeEditor && parameter_hints_multiple_signatures && !suggest_widget_visible");

/// Runs the editor action `id` (Monaco `editor.trigger(source, id)`): a
/// built-in one such as `"editor.action.formatDocument"`, or one added with
/// [`crate::CodeEditor::add_action`].
#[derive(Clone, Debug, PartialEq, Eq, Action)]
#[action(namespace = code_editor, no_json)]
pub struct RunAction {
    /// The action's id.
    pub id: String,
}

/// Runs command `id` in the focused editor (what
/// [`crate::add_keybinding_rules`] binds, Monaco's keybinding `command` and
/// `commandArgs`): an editor action, a Monaco command such as
/// `"cursorDown"`, a command from [`crate::add_command`] (given `args`), or
/// a GPUI action name.
#[derive(Clone, Debug, PartialEq, Action)]
#[action(namespace = code_editor, no_json)]
pub struct RunCommand {
    /// The command's id.
    pub id: String,
    /// The argument for a registered command.
    pub args: Option<serde_json::Value>,
}

actions!(
    code_editor,
    [
        /// Moves the cursor one character left (Monaco `cursorLeft`).
        CursorLeft,
        /// Extends the selection one character left (Monaco `cursorLeftSelect`).
        CursorLeftSelect,
        /// Moves the cursor one character right (Monaco `cursorRight`).
        CursorRight,
        /// Extends the selection one character right (Monaco `cursorRightSelect`).
        CursorRightSelect,
        /// Moves the cursor one line up (Monaco `cursorUp`).
        CursorUp,
        /// Extends the selection one line up (Monaco `cursorUpSelect`).
        CursorUpSelect,
        /// Moves the cursor one line down (Monaco `cursorDown`).
        CursorDown,
        /// Extends the selection one line down (Monaco `cursorDownSelect`).
        CursorDownSelect,
        /// Moves the cursor to the previous word start (Monaco `cursorWordLeft`).
        CursorWordLeft,
        /// Extends the selection to the previous word start (Monaco
        /// `cursorWordLeftSelect`).
        CursorWordLeftSelect,
        /// Moves the cursor to the next word end (Monaco `cursorWordRight`).
        CursorWordRight,
        /// Extends the selection to the next word end (Monaco
        /// `cursorWordRightSelect`).
        CursorWordRightSelect,
        /// Moves the cursor to the previous camelCase or snake_case word part (Monaco
        /// `cursorWordPartLeft`).
        CursorWordPartLeft,
        /// Extends the selection to the previous word part (Monaco
        /// `cursorWordPartLeftSelect`).
        CursorWordPartLeftSelect,
        /// Moves the cursor to the next camelCase or snake_case word part (Monaco
        /// `cursorWordPartRight`).
        CursorWordPartRight,
        /// Extends the selection to the next word part (Monaco
        /// `cursorWordPartRightSelect`).
        CursorWordPartRightSelect,
        /// Moves the cursor to the line's first non-whitespace character, or to column 0
        /// if already there (Monaco `cursorHome`).
        CursorHome,
        /// Like [`CursorHome`], extending the selection (Monaco `cursorHomeSelect`).
        CursorHomeSelect,
        /// Moves the cursor to the end of the line (Monaco `cursorEnd`).
        CursorEnd,
        /// Extends the selection to the end of the line (Monaco `cursorEndSelect`).
        CursorEndSelect,
        /// Moves the cursor to the start of the text (Monaco `cursorTop`).
        CursorTop,
        /// Extends the selection to the start of the text (Monaco `cursorTopSelect`).
        CursorTopSelect,
        /// Moves the cursor to the end of the text (Monaco `cursorBottom`).
        CursorBottom,
        /// Extends the selection to the end of the text (Monaco
        /// `cursorBottomSelect`).
        CursorBottomSelect,
        /// Moves the cursor up one page (Monaco `cursorPageUp`).
        CursorPageUp,
        /// Extends the selection up one page (Monaco `cursorPageUpSelect`).
        CursorPageUpSelect,
        /// Moves the cursor down one page (Monaco `cursorPageDown`).
        CursorPageDown,
        /// Extends the selection down one page (Monaco `cursorPageDownSelect`).
        CursorPageDownSelect,
        /// Selects the whole text (Monaco `editor.action.selectAll`).
        SelectAll,
        /// Extends each selection to whole lines, then one more line per repeat (Monaco
        /// `expandLineSelection`).
        ExpandLineSelection,
        /// Moves to the previous word start (Monaco `cursorWordStartLeft`).
        CursorWordStartLeft,
        /// Selects to the previous word start (Monaco `cursorWordStartLeftSelect`).
        CursorWordStartLeftSelect,
        /// Moves to the next word start (Monaco `cursorWordStartRight`).
        CursorWordStartRight,
        /// Selects to the next word start (Monaco `cursorWordStartRightSelect`).
        CursorWordStartRightSelect,
        /// Moves to the previous word end (Monaco `cursorWordEndLeft`).
        CursorWordEndLeft,
        /// Selects to the previous word end (Monaco `cursorWordEndLeftSelect`).
        CursorWordEndLeftSelect,
        /// Moves to the next word end (Monaco `cursorWordEndRight`).
        CursorWordEndRight,
        /// Selects to the next word end (Monaco `cursorWordEndRightSelect`).
        CursorWordEndRightSelect,
        /// Deletes to the previous word start (Monaco `deleteWordStartLeft`).
        DeleteWordStartLeft,
        /// Deletes to the next word start (Monaco `deleteWordStartRight`).
        DeleteWordStartRight,
        /// Deletes to the previous word end (Monaco `deleteWordEndLeft`).
        DeleteWordEndLeft,
        /// Deletes to the next word end (Monaco `deleteWordEndRight`).
        DeleteWordEndRight,
        /// Deletes to the previous camelCase / `snake_case` part (Monaco
        /// `deleteWordPartLeft`, Ctrl+Alt+Backspace on macOS).
        DeleteWordPartLeft,
        /// Deletes to the next camelCase / `snake_case` part (Monaco
        /// `deleteWordPartRight`, Ctrl+Alt+Delete on macOS).
        DeleteWordPartRight,
        /// Inserts a line break after each cursor without moving it (Monaco
        /// `lineBreakInsert`, Ctrl+O on macOS).
        LineBreakInsert,
        /// Adds the previous occurrence of the selection as a new selection
        /// (Monaco `editor.action.addSelectionToPreviousFindMatch`).
        AddSelectionToPreviousFindMatch,
        /// Moves the last selection to the previous occurrence (Monaco
        /// `editor.action.moveSelectionToPreviousFindMatch`).
        MoveSelectionToPreviousFindMatch,
        /// Puts a cursor at the end of every selected line (Monaco
        /// `editor.action.insertCursorAtEndOfEachLineSelected`, Shift+Alt+I).
        InsertCursorAtEndOfEachLineSelected,
        /// Repeats the primary selection on every line below it (Monaco
        /// `editor.action.addCursorsToBottom`).
        AddCursorsToBottom,
        /// Repeats the primary selection on every line above it (Monaco
        /// `editor.action.addCursorsToTop`).
        AddCursorsToTop,
        /// Makes the next cursor primary (Monaco `editor.action.focusNextCursor`).
        FocusNextCursor,
        /// Makes the previous cursor primary (Monaco
        /// `editor.action.focusPreviousCursor`).
        FocusPreviousCursor,
        /// Moves to column 1 of the line (Monaco `cursorLineStart`, Ctrl+A on
        /// macOS).
        CursorLineStart,
        /// Selects to column 1 of the line (Monaco `cursorLineStartSelect`).
        CursorLineStartSelect,
        /// Moves to the end of the line (Monaco `cursorLineEnd`, Ctrl+E on
        /// macOS).
        CursorLineEnd,
        /// Selects to the end of the line (Monaco `cursorLineEndSelect`).
        CursorLineEndSelect,
        /// Extends the column selection one column left (Monaco
        /// `cursorColumnSelectLeft`, Cmd/Ctrl+Shift+Alt+Left).
        CursorColumnSelectLeft,
        /// Extends the column selection one column right (Monaco
        /// `cursorColumnSelectRight`).
        CursorColumnSelectRight,
        /// Extends the column selection one line up (Monaco
        /// `cursorColumnSelectUp`).
        CursorColumnSelectUp,
        /// Extends the column selection one line down (Monaco
        /// `cursorColumnSelectDown`).
        CursorColumnSelectDown,
        /// Extends the column selection one page up (Monaco
        /// `cursorColumnSelectPageUp`).
        CursorColumnSelectPageUp,
        /// Extends the column selection one page down (Monaco
        /// `cursorColumnSelectPageDown`).
        CursorColumnSelectPageDown,
        /// Switches between insert and overtype mode (Monaco
        /// `editor.action.toggleOvertypeInsertMode`, Insert).
        ToggleOvertypeInsertMode,
        /// Restores the previous cursor state (Monaco `cursorUndo`, Cmd/Ctrl+U).
        CursorUndo,
        /// Reapplies an undone cursor state (Monaco `cursorRedo`).
        CursorRedo,
        /// Marks the cursor position as the selection anchor (Monaco
        /// `editor.action.setSelectionAnchor`, Cmd/Ctrl+K Cmd/Ctrl+B).
        SetSelectionAnchor,
        /// Moves the cursor to the selection anchor (Monaco
        /// `editor.action.goToSelectionAnchor`).
        GoToSelectionAnchor,
        /// Selects from the anchor to the cursor (Monaco
        /// `editor.action.selectFromAnchorToCursor`, Cmd/Ctrl+K Cmd/Ctrl+K).
        SelectFromAnchorToCursor,
        /// Removes the selection anchor (Monaco
        /// `editor.action.cancelSelectionAnchor`, Escape).
        CancelSelectionAnchor,
        /// Scrolls up one line without moving the cursor (Monaco
        /// `scrollLineUp`, Ctrl+Up; Ctrl+PageUp on macOS).
        ScrollLineUp,
        /// Scrolls down one line (Monaco `scrollLineDown`).
        ScrollLineDown,
        /// Scrolls up one page without moving the cursor (Monaco
        /// `scrollPageUp`, Cmd+PageUp; Alt+PageUp elsewhere).
        ScrollPageUp,
        /// Scrolls down one page (Monaco `scrollPageDown`).
        ScrollPageDown,
        /// Scrolls to the top (Monaco `scrollEditorTop`).
        ScrollEditorTop,
        /// Scrolls to the bottom (Monaco `scrollEditorBottom`).
        ScrollEditorBottom,
        /// Collapses the primary selection (Monaco `cancelSelection`).
        CancelSelection,
        /// Sorts the selected lines, or the document (Monaco
        /// `editor.action.sortLinesAscending`).
        SortLinesAscending,
        /// Sorts the selected lines in descending order (Monaco
        /// `editor.action.sortLinesDescending`).
        SortLinesDescending,
        /// Removes duplicate lines (Monaco `editor.action.removeDuplicateLines`).
        DeleteDuplicateLines,
        /// Reverses the selected lines (Monaco `editor.action.reverseLines`).
        ReverseLines,
        /// Removes trailing whitespace (Monaco
        /// `editor.action.trimTrailingWhitespace`, Cmd/Ctrl+K Cmd/Ctrl+X).
        TrimTrailingWhitespace,
        /// Deletes to the end of the line (Monaco `deleteAllRight`, Ctrl+K /
        /// Cmd+Delete on macOS).
        DeleteAllRight,
        /// Joins lines (Monaco `editor.action.joinLines`, Ctrl+J on macOS).
        JoinLines,
        /// Swaps the characters around the cursor (Monaco
        /// `editor.action.transpose`).
        Transpose,
        /// Swaps the letters around the cursor (Monaco
        /// `editor.action.transposeLetters`, Ctrl+T on macOS).
        TransposeLetters,
        /// Uppercases the selection or word (Monaco
        /// `editor.action.transformToUppercase`).
        TransformToUppercase,
        /// Lowercases the selection or word (Monaco
        /// `editor.action.transformToLowercase`).
        TransformToLowercase,
        /// Title-cases the selection or word (Monaco
        /// `editor.action.transformToTitlecase`).
        TransformToTitlecase,
        /// `snake_case`s the selection or word (Monaco
        /// `editor.action.transformToSnakecase`).
        TransformToSnakecase,
        /// `camelCase`s the selection or word (Monaco
        /// `editor.action.transformToCamelcase`).
        TransformToCamelcase,
        /// `PascalCase`s the selection or word (Monaco
        /// `editor.action.transformToPascalcase`).
        TransformToPascalcase,
        /// `kebab-case`s the selection or word (Monaco
        /// `editor.action.transformToKebabcase`).
        TransformToKebabcase,
        /// Duplicates the selection, or the line (Monaco
        /// `editor.action.duplicateSelection`).
        DuplicateSelection,
        /// Moves the selected text one character left (Monaco
        /// `editor.action.moveCarretLeftAction`).
        MoveCaretLeft,
        /// Moves the selected text one character right (Monaco
        /// `editor.action.moveCarretRightAction`).
        MoveCaretRight,
        /// Reindents the document with the language's indentation rules
        /// (Monaco `editor.action.reindentlines`).
        ReindentLines,
        /// Reindents the selected lines (Monaco
        /// `editor.action.reindentselectedlines`).
        ReindentSelectedLines,
        /// Converts the indentation to spaces (Monaco
        /// `editor.action.indentationToSpaces`).
        IndentationToSpaces,
        /// Converts the indentation to tabs (Monaco
        /// `editor.action.indentationToTabs`).
        IndentationToTabs,
        /// Asks for a tab size and indents with spaces (Monaco
        /// `editor.action.indentUsingSpaces`).
        IndentUsingSpaces,
        /// Asks for a tab size and indents with tabs (Monaco
        /// `editor.action.indentUsingTabs`).
        IndentUsingTabs,
        /// Asks for how wide tabs show (Monaco
        /// `editor.action.changeTabDisplaySize`).
        ChangeTabDisplaySize,
        /// Guesses the indentation from the text (Monaco
        /// `editor.action.detectIndentation`).
        DetectIndentation,
        /// Replaces the number or value at the cursor with the previous one:
        /// `1` → `0`, `false` → `true` (Monaco
        /// `editor.action.inPlaceReplace.up`, "Replace with Previous Value",
        /// Cmd/Ctrl+Shift+,).
        InPlaceReplaceUp,
        /// Replaces the number or value at the cursor with the next one: `1`
        /// → `2`, `true` → `false` (Monaco
        /// `editor.action.inPlaceReplace.down`, "Replace with Next Value",
        /// Cmd/Ctrl+Shift+.).
        InPlaceReplaceDown,
        /// Removes the brackets around the cursor (Monaco
        /// `editor.action.removeBrackets`, Cmd/Ctrl+Alt+Backspace).
        RemoveBrackets,
        /// Selects the next occurrence of the symbol at the cursor (Monaco
        /// `editor.action.wordHighlight.next`, F7).
        WordHighlightNext,
        /// Selects the previous occurrence (Monaco
        /// `editor.action.wordHighlight.prev`, Shift+F7).
        WordHighlightPrev,
        /// Highlights the symbol's occurrences now (Monaco
        /// `editor.action.wordHighlight.trigger`).
        WordHighlightTrigger,
        /// Toggles line comments on the selected lines (Monaco
        /// `editor.action.commentLine`, Cmd+/).
        CommentLine,
        /// Adds line comments to the selected lines (Monaco
        /// `editor.action.addCommentLine`, Cmd+K Cmd+C).
        AddCommentLine,
        /// Removes line comments from the selected lines (Monaco
        /// `editor.action.removeCommentLine`, Cmd+K Cmd+U).
        RemoveCommentLine,
        /// Toggles a block comment around the selection (Monaco
        /// `editor.action.blockComment`, Shift+Alt+A).
        BlockComment,
        /// Escape: closes the suggest widget, marker zone, parameter hints, hover
        /// or find widget if one is open; otherwise keeps only the primary
        /// cursor, then clears its selection (Monaco `removeSecondaryCursors`).
        RemoveSecondaryCursors,
        /// Shift+Escape, Monaco's secondary Escape key: as
        /// [`RemoveSecondaryCursors`] for the commands Monaco gives it (the
        /// suggest widget, snippet mode, linked editing, the problem zone,
        /// parameter hints, the reference peek, the find widget, then the
        /// cursors and selection), leaving ghost text, the hover, inline
        /// messages, symbol navigation, pending operations and the
        /// selection anchor alone.
        SecondaryEscape,
        /// Deletes the selection or the character before each cursor (Monaco
        /// `deleteLeft`).
        DeleteLeft,
        /// Deletes the selection or the character after each cursor (Monaco
        /// `deleteRight`).
        DeleteRight,
        /// Deletes to the previous word start (Monaco `deleteWordLeft`).
        DeleteWordLeft,
        /// Deletes to the next word end (Monaco `deleteWordRight`).
        DeleteWordRight,
        /// Deletes from each cursor to the start of its line (Monaco `deleteAllLeft`).
        DeleteAllLeft,
        /// Accepts the selected suggestion, or indents (Monaco `tab`).
        Tab,
        /// Outdents the selected lines (Monaco `outdent`, Shift+Tab).
        Outdent,
        /// Accepts the selected suggestion, or inserts a line break with auto-indent.
        Newline,
        /// Indents the selected lines (Monaco `editor.action.indentLines`).
        IndentLines,
        /// Outdents the selected lines (Monaco `editor.action.outdentLines`).
        OutdentLines,
        /// Moves the selected lines up one line (Monaco `editor.action.moveLinesUpAction`).
        MoveLinesUp,
        /// Moves the selected lines down one line (Monaco
        /// `editor.action.moveLinesDownAction`).
        MoveLinesDown,
        /// Duplicates the selected lines above them (Monaco
        /// `editor.action.copyLinesUpAction`).
        CopyLinesUp,
        /// Duplicates the selected lines below them (Monaco
        /// `editor.action.copyLinesDownAction`).
        CopyLinesDown,
        /// Deletes the lines touched by each selection (Monaco
        /// `editor.action.deleteLines`).
        DeleteLines,
        /// Inserts an empty line below the cursor's line and moves there (Monaco
        /// `editor.action.insertLineAfter`).
        InsertLineAfter,
        /// Inserts an empty line above the cursor's line and moves there (Monaco
        /// `editor.action.insertLineBefore`).
        InsertLineBefore,
        /// Undoes the last edit (Monaco `undo`).
        Undo,
        /// Redoes the last undone edit (Monaco `redo`).
        Redo,
        /// Copies the selections; with an empty selection, the whole line when
        /// [`empty_selection_clipboard`](crate::EditorOptions::empty_selection_clipboard)
        /// is on.
        Copy,
        /// Cuts the selections; with an empty selection, the whole line when
        /// [`empty_selection_clipboard`](crate::EditorOptions::empty_selection_clipboard)
        /// is on.
        Cut,
        /// Pastes the clipboard; text copied from as many cursors as there are now is
        /// distributed one piece per cursor (Monaco `editor.action.clipboardPasteAction`).
        Paste,
        /// Turns tab focus mode on or off for every editor (Monaco
        /// `editor.action.toggleTabFocusMode`, Ctrl+M, Ctrl+Shift+M on macOS).
        ToggleTabFocusMode,
        /// Moves focus to the next element (Tab in tab focus mode, in
        /// read-only editors and in the editor's widgets: the browser's Tab
        /// in Monaco).
        FocusNextElement,
        /// Moves focus to the previous element (Shift+Tab in tab focus mode,
        /// in read-only editors and in the editor's widgets).
        FocusPreviousElement,
        /// Pastes the clipboard as plain text (Monaco
        /// `editor.action.pasteAsText`).
        PasteAsText,
        /// Picks another way to insert what was just pasted (Monaco
        /// `editor.changePasteType`, Cmd/Ctrl+. after a paste).
        ChangePasteType,
        /// Picks another way to insert what was just dropped (Monaco
        /// `editor.changeDropType`, Cmd/Ctrl+. after a drop).
        ChangeDropType,
        /// Hides the paste or drop options button (Monaco
        /// `editor.hidePasteWidget`, `editor.hideDropWidget`, Escape).
        HidePostEditWidget,
        /// Opens the platform's emoji and symbols picker.
        ShowCharacterPalette,
        /// Collapses every region (Monaco `editor.foldAll`).
        FoldAll,
        /// Expands every region (Monaco `editor.unfoldAll`).
        UnfoldAll,
        /// Expands the collapsed region at the cursor, or collapses the innermost one
        /// (Monaco `editor.toggleFold`).
        ToggleFold,
        /// Collapses the region at each cursor and all inside it (Monaco
        /// `editor.foldRecursively`).
        FoldRecursively,
        /// Expands the region at each cursor and all inside it (Monaco
        /// `editor.unfoldRecursively`).
        UnfoldRecursively,
        /// Toggles the region at each cursor and all inside it (Monaco
        /// `editor.toggleFoldRecursively`).
        ToggleFoldRecursively,
        /// Collapses every block comment (Monaco `editor.foldAllBlockComments`).
        FoldAllBlockComments,
        /// Collapses every marker region (Monaco `editor.foldAllMarkerRegions`).
        FoldAllMarkerRegions,
        /// Expands every marker region (Monaco `editor.unfoldAllMarkerRegions`).
        UnfoldAllMarkerRegions,
        /// Collapses every region not around a cursor (Monaco
        /// `editor.foldAllExcept`).
        FoldAllExcept,
        /// Expands every region not around a cursor (Monaco
        /// `editor.unfoldAllExcept`).
        UnfoldAllExcept,
        /// Collapses the regions at level 1 (Monaco `editor.foldLevel1`).
        FoldLevel1,
        /// Level 2 (Monaco `editor.foldLevel2`).
        FoldLevel2,
        /// Level 3 (Monaco `editor.foldLevel3`).
        FoldLevel3,
        /// Level 4 (Monaco `editor.foldLevel4`).
        FoldLevel4,
        /// Level 5 (Monaco `editor.foldLevel5`).
        FoldLevel5,
        /// Level 6 (Monaco `editor.foldLevel6`).
        FoldLevel6,
        /// Level 7 (Monaco `editor.foldLevel7`).
        FoldLevel7,
        /// Moves to the enclosing fold's start (Monaco `editor.gotoParentFold`).
        GoToParentFold,
        /// Moves to the previous fold (Monaco `editor.gotoPreviousFold`).
        GoToPreviousFold,
        /// Moves to the next fold (Monaco `editor.gotoNextFold`).
        GoToNextFold,
        /// Folds the selection (Monaco `editor.createFoldingRangeFromSelection`).
        CreateFoldFromSelection,
        /// Removes manual folding ranges (Monaco
        /// `editor.removeManualFoldingRanges`).
        RemoveManualFoldingRanges,
        /// Toggles the import ranges (Monaco `editor.toggleImportFold`).
        ToggleImportFold,
        /// Expands the selection (Monaco `editor.action.smartSelect.expand`).
        ExpandSelection,
        /// Shrinks the selection (Monaco `editor.action.smartSelect.shrink`).
        ShrinkSelection,
        /// Opens the find widget, seeded from the selection or word at the cursor (Monaco
        /// `actions.find`).
        StartFind,
        /// Opens the find widget with the replace input (Monaco
        /// `editor.action.startFindReplaceAction`).
        StartFindReplace,
        /// Selects the next find match (Monaco `editor.action.nextMatchFindAction`).
        NextMatch,
        /// Selects the previous find match (Monaco
        /// `editor.action.previousMatchFindAction`).
        PreviousMatch,
        /// Replaces the selected match and moves to the next (Monaco
        /// `editor.action.replaceOne`).
        ReplaceOne,
        /// Replaces every match (Monaco `editor.action.replaceAll`).
        ReplaceAll,
        /// Selects every find match and closes the find widget (Monaco
        /// `editor.action.selectAllMatches`).
        SelectAllMatches,
        /// Toggles the find widget's match case option (Monaco
        /// `toggleFindCaseSensitive`).
        ToggleCaseSensitive,
        /// Toggles the find widget's regex option (Monaco `toggleFindRegex`).
        ToggleRegex,
        /// Toggles the find widget's whole word option (Monaco `toggleFindWholeWord`).
        ToggleWholeWord,
        /// Toggles searching only in the selection (Monaco `toggleFindInSelection`).
        ToggleFindInSelection,
        /// Toggles keeping the case of replaced text (Monaco `togglePreserveCase`).
        TogglePreserveCase,
        /// Opens find with the selection, even over several lines, as the
        /// search string (Monaco `actions.findWithSelection`, Cmd+E on macOS).
        StartFindWithSelection,
        /// Searches for the selection or word at the cursor and selects the
        /// next match (Monaco `editor.action.nextSelectionMatchFindAction`,
        /// Cmd/Ctrl+F3).
        NextSelectionMatch,
        /// The same, backwards (Monaco
        /// `editor.action.previousSelectionMatchFindAction`).
        PreviousSelectionMatch,
        /// The previous search or replace string in the find widget (Monaco
        /// `history.showPrevious`, Up).
        FindHistoryPrevious,
        /// The next search or replace string in the find widget (Monaco
        /// `history.showNext`, Down).
        FindHistoryNext,
        /// Selects the word at the cursor, or adds the next occurrence of the selection
        /// (Monaco `editor.action.addSelectionToNextFindMatch`).
        AddSelectionToNextFindMatch,
        /// Moves the last selection to the next occurrence of its text (Monaco
        /// `editor.action.moveSelectionToNextFindMatch`).
        MoveSelectionToNextFindMatch,
        /// Selects every occurrence of the selection or the word at the cursor (Monaco
        /// `editor.action.selectHighlights`).
        SelectAllOccurrences,
        /// Moves each cursor to the matching bracket, the end of the
        /// enclosing pair or the next bracket (Monaco
        /// `editor.action.jumpToBracket`).
        JumpToBracket,
        /// Moves to the next marker and shows it inline (Monaco
        /// `editor.action.marker.next`, Alt+F8).
        GoToNextMarker,
        /// Moves to the previous marker and shows it inline (Monaco
        /// `editor.action.marker.prev`, Shift+Alt+F8).
        GoToPreviousMarker,
        /// Monaco `editor.action.marker.nextInFiles` (F8): the next marker
        /// of every uri, opening another uri's through the editor openers
        /// (see [`CodeEditor::go_to_marker_in_files`](crate::CodeEditor::go_to_marker_in_files)).
        GoToNextMarkerInFiles,
        /// Monaco `editor.action.marker.prevInFiles` (Shift+F8): the
        /// previous marker of every uri.
        GoToPreviousMarkerInFiles,
        /// Goes to the definition of the symbol at the cursor (Monaco
        /// `editor.action.revealDefinition`).
        GoToDefinition,
        /// Requests parameter hints at the cursor (Monaco
        /// `editor.action.triggerParameterHints`).
        TriggerParameterHints,
        /// Closes parameter hints (Monaco `closeParameterHints`).
        CloseParameterHints,
        /// Shows the next signature in parameter hints (Monaco
        /// `showNextParameterHint`).
        ShowNextParameterHint,
        /// Shows the previous signature in parameter hints (Monaco
        /// `showPrevParameterHint`).
        ShowPrevParameterHint,
        /// Requests completions at the cursor (Monaco `editor.action.triggerSuggest`).
        TriggerSuggest,
        /// Accepts the selected suggestion in the other insert mode (Monaco
        /// `acceptAlternativeSelectedSuggestion`).
        AcceptAlternativeSuggestion,
        /// Shows or hides the suggestion's details (Monaco
        /// `toggleSuggestionDetails`).
        ToggleSuggestionDetails,
        /// Selects the next suggestion (Monaco `selectNextSuggestion`).
        SelectNextSuggestion,
        /// Selects the previous suggestion (Monaco `selectPrevSuggestion`).
        SelectPreviousSuggestion,
        /// Selects the suggestion a page down (Monaco
        /// `selectNextPageSuggestion`).
        SelectNextPageSuggestion,
        /// Selects the suggestion a page up (Monaco
        /// `selectPrevPageSuggestion`).
        SelectPreviousPageSuggestion,
        /// Selects the first suggestion (Monaco `selectFirstSuggestion`).
        SelectFirstSuggestion,
        /// Selects the last suggestion (Monaco `selectLastSuggestion`).
        SelectLastSuggestion,
        /// Shows the definition preview hover for the symbol at the cursor,
        /// as Cmd/Ctrl+hover does (Monaco
        /// `editor.action.showDefinitionPreviewHover`).
        ShowDefinitionPreviewHover,
        /// Hides the hover (Monaco `editor.action.hideHover`; Escape in a
        /// focused hover).
        HideHover,
        /// Scrolls the focused hover up a line (Monaco
        /// `editor.action.scrollUpHover`, Up).
        ScrollUpHover,
        /// Scrolls the focused hover down a line (Monaco
        /// `editor.action.scrollDownHover`, Down).
        ScrollDownHover,
        /// Scrolls the focused hover left (Monaco
        /// `editor.action.scrollLeftHover`, Left).
        ScrollLeftHover,
        /// Scrolls the focused hover right (Monaco
        /// `editor.action.scrollRightHover`, Right).
        ScrollRightHover,
        /// Scrolls the focused hover up a page (Monaco
        /// `editor.action.pageUpHover`, PageUp or Alt+Up).
        PageUpHover,
        /// Scrolls the focused hover down a page (Monaco
        /// `editor.action.pageDownHover`, PageDown or Alt+Down).
        PageDownHover,
        /// Scrolls the focused hover to the top (Monaco
        /// `editor.action.goToTopHover`, Home or Cmd/Ctrl+Up).
        GoToTopHover,
        /// Scrolls the focused hover to the bottom (Monaco
        /// `editor.action.goToBottomHover`, End or Cmd/Ctrl+Down).
        GoToBottomHover,
        /// Asks the focused (else first) hover part's provider for more
        /// detail (Monaco `editor.action.increaseHoverVerbosityLevel`).
        IncreaseHoverVerbosityLevel,
        /// Asks the focused (else first) hover part's provider for less
        /// detail (Monaco `editor.action.decreaseHoverVerbosityLevel`).
        DecreaseHoverVerbosityLevel,
        /// Turns off long line warning hovers (Monaco
        /// `editor.action.hideLongLineWarningHover`, the warning's "Don't
        /// Show Again").
        HideLongLineWarningHover,
        /// Opens the link at the cursor (Monaco `editor.action.openLink`).
        OpenLink,
        /// Opens quick access to go to a line (Monaco `editor.action.gotoLine`).
        GotoLine,
        /// Opens quick access to go to a symbol (Monaco
        /// `editor.action.quickOutline`).
        QuickOutline,
        /// Opens quick access to go to a character offset (Monaco
        /// `editor.action.gotoOffset`, "Go to Offset...").
        GotoOffset,
        /// Throws away the model's tokens and tokenizes it again right away
        /// (Monaco `editor.action.forceRetokenize`, "Developer: Force
        /// Retokenize").
        ForceRetokenize,
        /// Shows the token at the cursor, with its language, type and colors
        /// (Monaco `editor.action.inspectTokens`, "Developer: Inspect
        /// Tokens"); Escape closes it.
        InspectTokens,
        /// Switches every editor between the high contrast theme and the
        /// theme before it (Monaco `editor.action.toggleHighContrast`).
        ToggleHighContrast,
        /// Cancels the newest pending request that Escape can cancel, such
        /// as a go to definition, format, rename or paste (Monaco
        /// `editor.cancelOperation`, Escape).
        CancelOperation,
        /// Turns off highlighting ambiguous characters (Monaco
        /// `editor.action.unicodeHighlight.disableHighlightingOfAmbiguousCharacters`).
        DisableHighlightingOfAmbiguousCharacters,
        /// Turns off highlighting invisible characters (Monaco
        /// `editor.action.unicodeHighlight.disableHighlightingOfInvisibleCharacters`).
        DisableHighlightingOfInvisibleCharacters,
        /// Turns off highlighting non-basic ASCII characters (Monaco
        /// `editor.action.unicodeHighlight.disableHighlightingOfNonBasicAsciiCharacters`).
        DisableHighlightingOfNonBasicAsciiCharacters,
        /// Opens the command palette (Monaco `editor.action.quickCommand`).
        QuickCommand,
        /// Turns sticky scroll on or off (Monaco
        /// `editor.action.toggleStickyScroll`).
        ToggleStickyScroll,
        /// Zooms every editor's font in a level (Monaco
        /// `editor.action.fontZoomIn`).
        FontZoomIn,
        /// Zooms every editor's font out a level (Monaco
        /// `editor.action.fontZoomOut`).
        FontZoomOut,
        /// Resets the font zoom (Monaco `editor.action.fontZoomReset`).
        FontZoomReset,
        /// Moves keyboard focus to the sticky lines (Monaco
        /// `editor.action.focusStickyScroll`).
        FocusStickyScroll,
        /// The next sticky line (Monaco
        /// `editor.action.selectNextStickyScrollLine`, Down).
        SelectNextStickyScrollLine,
        /// The previous sticky line (Monaco
        /// `editor.action.selectPreviousStickyScrollLine`, Up).
        SelectPreviousStickyScrollLine,
        /// Goes to the focused sticky line (Monaco
        /// `editor.action.goToFocusedStickyScrollLine`, Enter).
        GoToFocusedStickyScrollLine,
        /// Returns focus from the sticky lines to the text (Monaco
        /// `editor.action.selectEditor`, Escape).
        SelectEditor,
        /// Shows the context menu at the cursor (Monaco
        /// `editor.action.showContextMenu`, Shift+F10).
        ShowContextMenu,
        /// The next context menu item.
        ContextMenuNext,
        /// The previous context menu item.
        ContextMenuPrevious,
        /// The first context menu item.
        ContextMenuFirst,
        /// The last context menu item.
        ContextMenuLast,
        /// Runs the context menu item as it shows (its alternative while
        /// Alt is held), or opens its submenu.
        ContextMenuSelect,
        /// Runs the context menu item's alternative action, if it has one
        /// (Monaco `IAction.alt`; Enter or Space with Alt, or with Shift on
        /// Windows and Linux).
        ContextMenuSelectAlternative,
        /// Opens the context menu item's submenu.
        ContextMenuSubmenuOpen,
        /// Closes the context menu's submenu.
        ContextMenuSubmenuClose,
        /// Hides the context menu.
        ContextMenuHide,
        /// Shows the standalone color picker at the cursor, or focuses it
        /// (Monaco `editor.action.showOrFocusStandaloneColorPicker`).
        ShowOrFocusStandaloneColorPicker,
        /// Hides the standalone color picker (Monaco
        /// `editor.action.hideColorPicker`, Escape).
        HideColorPicker,
        /// Writes the standalone color picker's color and hides it (Monaco
        /// `editor.action.insertColorWithStandaloneColorPicker`, Enter).
        InsertColorWithStandaloneColorPicker,
        /// The next quick access item.
        QuickInputNext,
        /// The previous quick access item.
        QuickInputPrevious,
        /// A page down in the quick access list.
        QuickInputPageNext,
        /// A page up in the quick access list.
        QuickInputPagePrevious,
        /// The first quick access item.
        QuickInputFirst,
        /// The last quick access item.
        QuickInputLast,
        /// Starts linked editing at the cursor, even with the option off
        /// (Monaco `editor.action.linkedEditing`).
        StartLinkedEditing,
        /// Shows the next inline completion (Monaco
        /// `editor.action.inlineSuggest.showNext`).
        ShowNextInlineSuggestion,
        /// Shows the previous inline completion (Monaco
        /// `editor.action.inlineSuggest.showPrevious`).
        ShowPreviousInlineSuggestion,
        /// Accepts the ghost text or inline edit (Monaco
        /// `editor.action.inlineSuggest.commit`).
        AcceptInlineSuggestion,
        /// Jumps to the inline edit (Monaco `editor.action.inlineSuggest.jump`).
        JumpToNextInlineEdit,
        /// Runs the inline edit's alternative action instead of its edit
        /// (Monaco `editor.action.inlineSuggest.commitAlternativeAction`,
        /// Shift+Tab).
        AcceptInlineSuggestionAlternativeAction,
        /// Switches inline edits between showing collapsed and expanded
        /// (Monaco `editor.action.inlineSuggest.toggleShowCollapsed`).
        ToggleInlineSuggestionShowCollapsed,
        /// Accepts the ghost text's next word (Monaco
        /// `editor.action.inlineSuggest.acceptNextWord`).
        AcceptNextInlineWord,
        /// Accepts the ghost text's next line (Monaco
        /// `editor.action.inlineSuggest.acceptNextLine`).
        AcceptNextInlineLine,
        /// Hides the ghost text or inline edit (Monaco
        /// `editor.action.inlineSuggest.hide`).
        HideInlineSuggestion,
        /// Switches the inline suggestion toolbar between always shown and
        /// shown on hover (Monaco
        /// `editor.action.inlineSuggest.toggleAlwaysShowToolbar`).
        ToggleAlwaysShowInlineSuggestionToolbar,
        /// Formats the document (Monaco `editor.action.formatDocument`).
        FormatDocument,
        /// Goes to the declaration (Monaco `editor.action.revealDeclaration`).
        GoToDeclaration,
        /// Goes to the type definition (Monaco
        /// `editor.action.goToTypeDefinition`).
        GoToTypeDefinition,
        /// Goes to implementations (Monaco `editor.action.goToImplementation`).
        GoToImplementation,
        /// Goes to references (Monaco `editor.action.goToReferences`).
        GoToReferences,
        /// Peeks the definition (Monaco `editor.action.peekDefinition`).
        PeekDefinition,
        /// Peeks the declaration (Monaco `editor.action.peekDeclaration`).
        PeekDeclaration,
        /// Peeks the type definition (Monaco `editor.action.peekTypeDefinition`).
        PeekTypeDefinition,
        /// Peeks implementations (Monaco `editor.action.peekImplementation`).
        PeekImplementation,
        /// Peeks references (Monaco `editor.action.referenceSearch.trigger`).
        PeekReferences,
        /// The next result: in the peek view, or after going to one of
        /// several results (Monaco `goToNextReference`,
        /// `editor.gotoNextSymbolFromResult`).
        GoToNextReference,
        /// The previous result in the peek view (Monaco
        /// `goToPreviousReference`).
        GoToPreviousReference,
        /// Peek view list: the next row (Monaco `list.focusDown`).
        PeekSelectNext,
        /// Peek view list: the previous row (Monaco `list.focusUp`).
        PeekSelectPrevious,
        /// Peek view list: the first row (Monaco `list.focusFirst`).
        PeekSelectFirst,
        /// Peek view list: the last row (Monaco `list.focusLast`).
        PeekSelectLast,
        /// Peek view list: a page down (Monaco `list.focusPageDown`).
        PeekPageDown,
        /// Peek view list: a page up (Monaco `list.focusPageUp`).
        PeekPageUp,
        /// Peek view list: go to the focused result, or toggle the focused
        /// file (Monaco `revealReference`, `list.select`).
        PeekOpen,
        /// Peek view list: open the focused result to the side (Monaco
        /// `openReferenceToSide`).
        PeekOpenToSide,
        /// Peek view list: expand or collapse the focused file (Monaco
        /// `list.toggleExpand`).
        PeekToggle,
        /// Peek view list: collapse the focused file, or go to a result's
        /// file (Monaco `list.collapse`).
        PeekCollapse,
        /// Peek view list: expand the focused file, or go to its first
        /// result (Monaco `list.expand`).
        PeekExpand,
        /// Closes the peek view (Monaco `closeReferenceSearch`).
        ClosePeek,
        /// Renames the symbol at the cursor (Monaco `editor.action.rename`).
        Rename,
        /// Rename box: rename to the focused suggestion or the typed name
        /// (Monaco `acceptRenameInput`, Enter).
        AcceptRenameInput,
        /// Rename box: as [`AcceptRenameInput`], previewing the edits first
        /// when a [preview handler](crate::set_workspace_edit_preview_handler)
        /// is set (Monaco `acceptRenameInputWithPreview`, Cmd/Ctrl+Enter).
        AcceptRenameInputWithPreview,
        /// Rename box: close it (Monaco `cancelRenameInput`, Escape or
        /// Shift+Escape).
        CancelRenameInput,
        /// Rename box: focus the next suggestion (Monaco
        /// `focusNextRenameSuggestion`, Down).
        FocusNextRenameSuggestion,
        /// Rename box: focus the previous suggestion (Monaco
        /// `focusPreviousRenameSuggestion`, Up).
        FocusPreviousRenameSuggestion,
        /// Shows code actions (Monaco `editor.action.quickFix`).
        QuickFix,
        /// Organizes imports (Monaco `editor.action.organizeImports`).
        OrganizeImports,
        /// Fixes all auto-fixable problems (Monaco `editor.action.fixAll`).
        FixAll,
        /// Applies the preferred quick fix (Monaco `editor.action.autoFix`).
        AutoFix,
        /// Code action menu: the next action.
        CodeActionMenuNext,
        /// Code action menu: the previous action.
        CodeActionMenuPrevious,
        /// Code action menu: apply the focused action (Monaco
        /// `acceptSelectedCodeAction`).
        CodeActionMenuAccept,
        /// Code action menu: apply the focused action, previewing its edits
        /// first when a [preview handler](crate::set_workspace_edit_preview_handler)
        /// is set (Monaco `previewSelectedCodeAction`, Cmd/Ctrl+Enter).
        CodeActionMenuPreview,
        /// Code action menu: close it.
        CodeActionMenuHide,
        /// Formats the selections, or the cursor's line (Monaco
        /// `editor.action.formatSelection`).
        FormatSelection,
        /// Focuses the first suggestion when none is (Monaco
        /// `focusSuggestion`, see `suggest.selectionMode`).
        FocusSuggestion,
        /// Focuses the first suggestion and accepts it (Monaco
        /// `focusAndAcceptSuggestion`).
        FocusAndAcceptSuggestion,
        /// Shows how the focused suggestion was scored in the details
        /// panel (Monaco `toggleExplainMode`).
        ToggleExplainMode,
        /// Forgets the suggest widget's resized size (Monaco
        /// `editor.action.resetSuggestSize`).
        ResetSuggestSize,
        /// Moves the focus between the suggest list and its details panel,
        /// whose keys then scroll it (Monaco `toggleSuggestionFocus`).
        ToggleSuggestionFocus,
        /// Copies the focused suggest details panel's text (Monaco
        /// `suggestWidgetCopy`).
        SuggestWidgetCopy,
        /// Inserts a line break in a find widget input (Monaco's find and
        /// replace inputs handle Ctrl+Enter themselves).
        FindInputNewline,
        /// Moves the focus from a find widget input back to the editor,
        /// keeping the widget (Monaco's find and replace inputs handle
        /// Cmd/Ctrl+Down themselves).
        FindInputFocusEditor,
        /// Asks for a match number and selects that match (Monaco
        /// `editor.action.goToMatchFindAction`).
        GoToMatch,
        /// Opens the definition in an editor beside this one (Monaco
        /// `editor.action.revealDefinitionAside`): see
        /// [`CodeEditorEvent::OpenLocationToSide`](crate::CodeEditorEvent::OpenLocationToSide).
        RevealDefinitionAside,
        /// Moves focus between the peek view's preview and its results
        /// list (Monaco `togglePeekWidgetFocus`).
        TogglePeekWidgetFocus,
        /// Inserts a line break at the end of the document (Monaco
        /// `editor.action.insertFinalNewLine`).
        InsertFinalNewLine,
        /// Lists the code lens commands of the cursor's line and runs the
        /// chosen one (Monaco `codelens.showLensesInCurrentLine`).
        ShowLensesInCurrentLine,
    ]
);

/// Opens the find widget with these search settings (Monaco
/// `editor.actions.findWithArgs`, `IFindStartArguments`). Unset fields keep
/// their current values. As in Monaco, an empty search string is still
/// seeded from the selection (or the word at the cursor) first.
#[derive(
    Clone,
    Debug,
    Default,
    PartialEq,
    Eq,
    serde::Deserialize,
    Action,
    gpui::private::schemars::JsonSchema,
)]
#[schemars(crate = "gpui::private::schemars")]
#[serde(default, rename_all = "camelCase")]
#[action(namespace = code_editor)]
pub struct FindWithArgs {
    /// The search string.
    pub search_string: Option<String>,
    /// The replace string; giving one shows the replace input.
    pub replace_string: Option<String>,
    /// Treat the search string as a regular expression.
    pub is_regex: Option<bool>,
    /// Match whole words only.
    pub match_whole_word: Option<bool>,
    /// Match case.
    pub is_case_sensitive: Option<bool>,
    /// Preserve case when replacing.
    pub preserve_case: Option<bool>,
    /// Search in the current selections.
    pub find_in_selection: bool,
}

/// Asks for code actions at the selection (Monaco
/// `editor.action.codeAction`, `CodeActionCommandArgs`).
#[derive(
    Clone,
    Debug,
    Default,
    PartialEq,
    Eq,
    serde::Deserialize,
    Action,
    gpui::private::schemars::JsonSchema,
)]
#[schemars(crate = "gpui::private::schemars")]
#[serde(default)]
#[action(namespace = code_editor)]
pub struct TriggerCodeAction {
    /// Only actions of this kind (and its sub-kinds), e.g. `"refactor"`;
    /// unset for every action that has a kind.
    pub kind: Option<String>,
    /// When to apply an action without the menu: `"ifSingle"` (the
    /// default) when it's the only one, `"first"` the first one, or
    /// `"never"`. Case doesn't matter.
    pub apply: Option<String>,
    /// Only preferred actions.
    pub preferred: bool,
}

/// A position as Monaco's API commands take it (`IPosition`): a 1-based
/// line and UTF-16 column.
#[derive(
    Clone,
    Copy,
    Debug,
    Default,
    PartialEq,
    Eq,
    serde::Deserialize,
    serde::Serialize,
    gpui::private::schemars::JsonSchema,
)]
#[schemars(crate = "gpui::private::schemars")]
#[serde(rename_all = "camelCase")]
pub struct CommandPosition {
    /// The line, from 1.
    pub line_number: u32,
    /// The UTF-16 column, from 1.
    pub column: u32,
}

impl CommandPosition {
    /// The 0-based point.
    pub fn to_point(self) -> text::PointUtf16 {
        text::PointUtf16::new(
            self.line_number.saturating_sub(1),
            self.column.saturating_sub(1),
        )
    }
}

impl From<text::PointUtf16> for CommandPosition {
    fn from(point: text::PointUtf16) -> Self {
        Self {
            line_number: point.row + 1,
            column: point.column + 1,
        }
    }
}

/// A range as Monaco's API commands take it (`IRange`), 1-based.
#[derive(
    Clone,
    Copy,
    Debug,
    Default,
    PartialEq,
    Eq,
    serde::Deserialize,
    serde::Serialize,
    gpui::private::schemars::JsonSchema,
)]
#[schemars(crate = "gpui::private::schemars")]
#[serde(rename_all = "camelCase")]
pub struct CommandRange {
    /// The first line, from 1.
    pub start_line_number: u32,
    /// The first UTF-16 column, from 1.
    pub start_column: u32,
    /// The last line, from 1.
    pub end_line_number: u32,
    /// The UTF-16 column after the range, from 1.
    pub end_column: u32,
}

impl CommandRange {
    /// The 0-based range.
    pub fn to_range(self) -> std::ops::Range<text::PointUtf16> {
        CommandPosition {
            line_number: self.start_line_number,
            column: self.start_column,
        }
        .to_point()..CommandPosition {
            line_number: self.end_line_number,
            column: self.end_column,
        }
        .to_point()
    }
}

impl From<std::ops::Range<text::PointUtf16>> for CommandRange {
    fn from(range: std::ops::Range<text::PointUtf16>) -> Self {
        Self {
            start_line_number: range.start.row + 1,
            start_column: range.start.column + 1,
            end_line_number: range.end.row + 1,
            end_column: range.end.column + 1,
        }
    }
}

/// A location as Monaco's API commands take it (`Location`).
#[derive(
    Clone,
    Debug,
    Default,
    PartialEq,
    Eq,
    serde::Deserialize,
    serde::Serialize,
    gpui::private::schemars::JsonSchema,
)]
#[schemars(crate = "gpui::private::schemars")]
#[serde(rename_all = "camelCase")]
pub struct CommandLocation {
    /// The model's uri; without one, the editor's model.
    #[serde(default)]
    pub uri: Option<String>,
    /// The range.
    pub range: CommandRange,
}

impl CommandLocation {
    /// The [`Location`](crate::providers::Location) it stands for.
    pub fn to_location(&self) -> crate::providers::Location {
        crate::providers::Location {
            uri: self.uri.clone().map(Into::into),
            range: self.range.to_range(),
            ..Default::default()
        }
    }
}

/// Goes to `locations` from `position` (Monaco's `editor.action.goToLocations`
/// API command): see [`CodeEditor::go_to_locations`](crate::CodeEditor::go_to_locations).
/// Run in an editor, it does nothing when `uri` names another model, as in
/// standalone Monaco.
#[derive(
    Clone,
    Debug,
    Default,
    PartialEq,
    Eq,
    serde::Deserialize,
    Action,
    gpui::private::schemars::JsonSchema,
)]
#[schemars(crate = "gpui::private::schemars")]
#[serde(default, rename_all = "camelCase")]
#[action(namespace = code_editor)]
pub struct GoToLocations {
    /// The model to start in; unset for the editor's.
    pub uri: Option<String>,
    /// Where the cursor goes first; unset to stay.
    pub position: Option<CommandPosition>,
    /// The locations.
    pub locations: Vec<CommandLocation>,
    /// What to do with several: `"peek"`, `"gotoAndPeek"` or `"goto"`;
    /// unset (or anything else) for `goto_location.multiple_references`.
    pub multiple: Option<String>,
    /// Shown when there are no locations; unset to show nothing.
    pub no_results_message: Option<String>,
    /// Show the locations in a peek view, even a single one.
    pub open_in_peek: bool,
}

/// Peeks `locations` from `position` (Monaco's `editor.action.peekLocations`
/// API command, also `editor.action.showReferences`): [`GoToLocations`]
/// with `open_in_peek`.
#[derive(
    Clone,
    Debug,
    Default,
    PartialEq,
    Eq,
    serde::Deserialize,
    Action,
    gpui::private::schemars::JsonSchema,
)]
#[schemars(crate = "gpui::private::schemars")]
#[serde(default, rename_all = "camelCase")]
#[action(namespace = code_editor)]
pub struct PeekLocations {
    /// The model to start in; unset for the editor's.
    pub uri: Option<String>,
    /// Where the cursor goes first; unset to stay.
    pub position: Option<CommandPosition>,
    /// The locations.
    pub locations: Vec<CommandLocation>,
    /// What to do with several, as for [`GoToLocations`].
    pub multiple: Option<String>,
}

/// Declares an action taking a Monaco command's argument object: camelCase
/// JSON whose fields are all optional, so the action also builds without
/// one (as a keybinding or [`crate::CodeEditor::trigger`] runs it).
macro_rules! command_action {
    ($(#[$meta:meta])* pub struct $name:ident { $($(#[$field_meta:meta])* pub $field:ident: $ty:ty),* $(,)? }) => {
        $(#[$meta])*
        #[derive(
            Clone,
            Debug,
            Default,
            PartialEq,
            serde::Deserialize,
            Action,
            gpui::private::schemars::JsonSchema,
        )]
        #[schemars(crate = "gpui::private::schemars")]
        #[serde(default, rename_all = "camelCase")]
        #[action(namespace = code_editor)]
        pub struct $name {
            $($(#[$field_meta])* pub $field: $ty,)*
        }
    };
}

command_action! {
    /// Collapses regions (Monaco `editor.fold`, `FoldingArguments`). Without
    /// `levels` or `direction`, the innermost expanded region at each
    /// selection, or its first expanded parent.
    pub struct Fold {
        /// How many levels to collapse (1 when only `direction` is set).
        pub levels: Option<u32>,
        /// `"up"` collapses the regions around the lines, `"down"` (the
        /// default) the regions inside them.
        pub direction: Option<String>,
        /// The 0-based lines to fold at instead of the selections' first
        /// lines.
        pub selection_lines: Option<Vec<u32>>,
    }
}

command_action! {
    /// Expands regions (Monaco `editor.unfold`, `FoldingArguments`): by
    /// default the region at each selection, one level down.
    pub struct Unfold {
        /// How many levels to expand (1 by default).
        pub levels: Option<u32>,
        /// `"up"` expands the regions around the lines, `"down"` (the
        /// default) the regions inside them.
        pub direction: Option<String>,
        /// The 0-based lines to unfold at instead of the selections' first
        /// lines.
        pub selection_lines: Option<Vec<u32>>,
    }
}

command_action! {
    /// Shows refactorings (Monaco `editor.action.refactor`, with
    /// `CodeActionCommandArgs`).
    pub struct Refactor {
        /// Only this kind of refactoring (a sub-kind of `refactor`), e.g.
        /// `"refactor.extract"`; any other kind matches nothing.
        pub kind: Option<String>,
        /// When to apply one without the menu: `"first"`, `"ifSingle"` or
        /// `"never"` (the default).
        pub apply: Option<String>,
        /// Only preferred refactorings.
        pub preferred: bool,
    }
}

command_action! {
    /// Shows source actions (Monaco `editor.action.sourceAction`, with
    /// `CodeActionCommandArgs`).
    pub struct SourceAction {
        /// Only this kind of source action (a sub-kind of `source`), e.g.
        /// `"source.organizeImports"`; any other kind matches nothing.
        pub kind: Option<String>,
        /// When to apply one without the menu: `"first"`, `"ifSingle"` or
        /// `"never"` (the default).
        pub apply: Option<String>,
        /// Only preferred source actions.
        pub preferred: bool,
    }
}

command_action! {
    /// Picks how to paste the clipboard (Monaco `editor.action.pasteAs`).
    /// With a `kind` (or `preferences`), pastes with the first matching
    /// edit instead of asking.
    pub struct PasteAs {
        /// The paste edit kind to use, e.g. `"text.plain"` (or the kinds
        /// it contains). Takes precedence over `preferences`.
        pub kind: Option<String>,
        /// Kinds to try: the first edit, in the providers' order, whose
        /// kind one of them contains is applied (Monaco `preferences`).
        pub preferences: Option<Vec<String>>,
    }
}

command_action! {
    /// Shows the hover for the cursor position, or focuses a visible one
    /// (Monaco `editor.action.showHover`).
    pub struct ShowHover {
        /// Monaco's `focus`: `"noAutoFocus"` never focuses (it shows the
        /// hover again), `"focusIfVisible"` (the default) focuses a visible
        /// hover, `"autoFocusImmediately"` (or `true`) also focuses a new
        /// one. With `accessibility_support` on, new hovers take focus.
        pub focus: Option<serde_json::Value>,
    }
}

command_action! {
    /// Asks for inline completions (Monaco
    /// `editor.action.inlineSuggest.trigger`).
    pub struct TriggerInlineSuggest {
        /// Monaco's `providerId`: asks only the provider whose
        /// [`provider_id`](crate::providers::InlineCompletionsProvider::provider_id)
        /// it is (or `"extension:*"`), or every provider when none is.
        pub provider_id: Option<String>,
        /// An explicit request (the default); `false` asks as typing does.
        pub explicit: Option<bool>,
        /// Monaco's `changeHintData`, accepted for compatibility.
        pub change_hint_data: Option<serde_json::Value>,
        /// Monaco's `showNoResultNotification`: logs "No inline suggestion
        /// is available." (at info level) when the request shows nothing,
        /// as standalone Monaco's notification service logs it to the
        /// console.
        pub show_no_result_notification: bool,
    }
}

command_action! {
    /// Moves the cursors to a logical position (Monaco `cursorMove`,
    /// `CursorMove.RawArguments`).
    pub struct CursorMove {
        /// Where to: `"left"`, `"right"`, `"up"`, `"down"`,
        /// `"prevBlankLine"`, `"nextBlankLine"`, `"wrappedLineStart"`,
        /// `"wrappedLineFirstNonWhitespaceCharacter"`,
        /// `"wrappedLineColumnCenter"`, `"wrappedLineEnd"`,
        /// `"wrappedLineLastNonWhitespaceCharacter"`, `"viewPortTop"`,
        /// `"viewPortCenter"`, `"viewPortBottom"` or `"viewPortIfOutside"`.
        pub to: String,
        /// The unit: `"line"`, `"wrappedLine"`, `"character"`, `"halfLine"`
        /// or `"foldedLine"`.
        pub by: Option<String>,
        /// How many units (1 by default).
        pub value: Option<u32>,
        /// Extends the selections.
        pub select: bool,
        /// Keeps the move out of the cursor navigation history.
        pub no_history: bool,
    }
}

command_action! {
    /// Scrolls without moving the cursor (Monaco `editorScroll`,
    /// `EditorScroll_.RawArguments`).
    pub struct EditorScroll {
        /// `"up"`, `"down"`, `"left"` or `"right"`.
        pub to: String,
        /// The unit: `"line"`, `"wrappedLine"` (the default), `"page"`,
        /// `"halfPage"`, `"editor"`, or `"column"` for left and right.
        pub by: Option<String>,
        /// How many units (1 by default).
        pub value: Option<f64>,
        /// Moves the primary cursor into the new viewport if it would be
        /// outside it.
        pub reveal_cursor: bool,
        /// With `reveal_cursor`, extends the selection instead.
        pub select: bool,
    }
}

command_action! {
    /// Scrolls `line_number` into view (Monaco `revealLine`).
    pub struct RevealLine {
        /// The 0-based line, a number or a numeric string.
        pub line_number: Option<serde_json::Value>,
        /// `"top"`, `"center"` or `"bottom"`; by default the least
        /// scrolling.
        pub at: Option<String>,
    }
}

/// A selection as Monaco's API commands take it (`ISelection`), 1-based.
#[derive(
    Clone,
    Copy,
    Debug,
    Default,
    PartialEq,
    Eq,
    serde::Deserialize,
    serde::Serialize,
    gpui::private::schemars::JsonSchema,
)]
#[schemars(crate = "gpui::private::schemars")]
#[serde(rename_all = "camelCase")]
pub struct CommandSelection {
    /// The anchor's line, from 1.
    pub selection_start_line_number: u32,
    /// The anchor's UTF-16 column, from 1.
    pub selection_start_column: u32,
    /// The head's line, from 1.
    pub position_line_number: u32,
    /// The head's UTF-16 column, from 1.
    pub position_column: u32,
}

impl CommandSelection {
    /// The 0-based selection.
    pub fn to_selection(self) -> crate::cursor::Selection {
        crate::cursor::Selection::new(
            CommandPosition {
                line_number: self.selection_start_line_number,
                column: self.selection_start_column,
            }
            .to_point(),
            CommandPosition {
                line_number: self.position_line_number,
                column: self.position_column,
            }
            .to_point(),
        )
    }
}

command_action! {
    /// Replaces the selections with `selection` (Monaco `setSelection`).
    pub struct SetSelection {
        /// The new selection.
        pub selection: Option<CommandSelection>,
    }
}

command_action! {
    /// Selects each cursor's bracket pair: the one at the cursor, around it
    /// or after it, backwards from a closing bracket (Monaco
    /// `editor.action.selectToBracket`). Unbound by default.
    pub struct SelectToBracket {
        /// Whether the brackets are selected too, not only what's between
        /// them; unset is `true` (Monaco `selectBrackets`).
        pub select_brackets: Option<bool>,
    }
}

command_action! {
    /// Adds a cursor at `position`, or removes the cursor there (Monaco
    /// `createCursor`, Alt+click).
    pub struct CreateCursor {
        /// Where.
        pub position: Option<CommandPosition>,
        /// Where it shows, in view coordinates (view line and column, from
        /// 1, with wrapping and folding): at a wrap point, which segment
        /// the cursor is on. Ignored unless it shows `position`.
        pub view_position: Option<CommandPosition>,
        /// Selects the whole line instead.
        pub whole_line: bool,
    }
}

command_action! {
    /// Column-selects from the last column selection's start, or from
    /// `position`, to `position` at `mouse_column` (Monaco `columnSelect`,
    /// Shift+Alt+drag).
    pub struct ColumnSelect {
        /// The position under the mouse.
        pub position: Option<CommandPosition>,
        /// The same position in view coordinates (view line and column,
        /// from 1). With it the selection goes by view lines, as Monaco's
        /// does: a wrapped line's segments are lines of their own and a
        /// fold is one line. Without it, from where `position` shows.
        pub view_position: Option<CommandPosition>,
        /// The 1-based visible column under the mouse.
        pub mouse_column: Option<u32>,
        /// Extends the current column selection instead of starting one.
        pub do_column_select: bool,
    }
}

command_action! {
    /// Adds a cursor on the line above each cursor (Monaco
    /// `editor.action.insertCursorAbove`, `InsertCursorArgs`). Does nothing
    /// in a read-only editor.
    pub struct AddCursorAbove {
        /// `Some(false)` goes by view lines (a wrapped line's segments and a
        /// fold are lines); by model lines otherwise (the default).
        pub logical_line: Option<bool>,
    }
}

command_action! {
    /// Adds a cursor on the line below each cursor (Monaco
    /// `editor.action.insertCursorBelow`, `InsertCursorArgs`). Does nothing
    /// in a read-only editor.
    pub struct AddCursorBelow {
        /// `Some(false)` goes by view lines (a wrapped line's segments and a
        /// fold are lines); by model lines otherwise (the default).
        pub logical_line: Option<bool>,
    }
}

command_action! {
    /// Deletes the word under each cursor and the whitespace next to it
    /// (Monaco `deleteInsideWord`).
    pub struct DeleteInsideWord {
        /// Deletes only the word, leaving the whitespace around it.
        pub only_word: bool,
    }
}

/// Declares Monaco's mouse commands, which take `MoveCommandOptions`.
macro_rules! move_to_command {
    ($($(#[$meta:meta])* $name:ident,)*) => {$(
        command_action! {
            $(#[$meta])*
            pub struct $name {
                /// Where, 1-based.
                pub position: Option<CommandPosition>,
                /// Where it shows, in view coordinates (view line and
                /// column, from 1, with wrapping and folding): at a wrap
                /// point, which segment the cursor is on, and the view line
                /// a line selection extends from. Ignored unless it shows
                /// `position`; without it, where `position` shows.
                pub view_position: Option<CommandPosition>,
                /// Monaco's `revealType`: 2 doesn't scroll to the cursor.
                pub reveal_type: Option<u32>,
            }
        }
    )*};
}

move_to_command! {
    /// Moves the primary cursor to `position`, dropping the others (Monaco
    /// `_moveTo`, a click).
    MoveTo,
    /// Extends the primary selection to `position` (Monaco `_moveToSelect`,
    /// a Shift+click), by word or line after a double or triple click.
    MoveToSelect,
    /// Selects the word at `position` (Monaco `_wordSelect`, a double
    /// click).
    WordSelect,
    /// Extends a word selection to `position` (Monaco `_wordSelectDrag`).
    WordSelectDrag,
    /// Selects the line at `position` (Monaco `_lineSelect`, a triple
    /// click).
    LineSelect,
    /// Extends a line selection to `position` (Monaco `_lineSelectDrag`).
    LineSelectDrag,
    /// Extends the last added cursor's selection to `position` (Monaco
    /// `_lastCursorMoveToSelect`, an Alt+drag).
    LastCursorMoveToSelect,
    /// Selects the word at `position` with the last added cursor (Monaco
    /// `lastCursorWordSelect`).
    LastCursorWordSelect,
    /// Selects the line at `position` with the last added cursor (Monaco
    /// `lastCursorLineSelect`).
    LastCursorLineSelect,
    /// Extends the last added cursor's line selection to `position`
    /// (Monaco `lastCursorLineSelectDrag`).
    LastCursorLineSelectDrag,
}

command_action! {
    /// Types `text` at every cursor as the keyboard does (Monaco's `type`
    /// handler, `editor.trigger('keyboard', 'type', { text })`).
    pub struct Type {
        /// The text.
        pub text: String,
    }
}

command_action! {
    /// Replaces the `replace_char_cnt` UTF-16 units before each cursor with
    /// `text` (Monaco's `replacePreviousChar` handler).
    pub struct ReplacePreviousChar {
        /// The text.
        pub text: String,
        /// How many units to replace.
        pub replace_char_cnt: u32,
    }
}

command_action! {
    /// An IME composition update (Monaco's `compositionType` handler):
    /// replaces `replace_prev_char_cnt` units before and
    /// `replace_next_char_cnt` after each cursor with `text`, then moves it
    /// by `position_delta`.
    pub struct CompositionType {
        /// The composed text.
        pub text: String,
        /// Units to replace before each cursor.
        pub replace_prev_char_cnt: u32,
        /// Units to replace after each cursor.
        pub replace_next_char_cnt: u32,
        /// How far to move each cursor from the end of `text`.
        pub position_delta: i32,
    }
}

command_action! {
    /// Pastes `text` (Monaco's `paste` handler, `editor.trigger('keyboard',
    /// 'paste', { text })`): no paste providers, as in Monaco. Without
    /// `text`, pastes the clipboard.
    pub struct PasteText {
        /// The text.
        pub text: String,
        /// Pastes whole lines above the cursor (Monaco
        /// `pasteOnNewLine`, text copied from an empty selection).
        pub paste_on_new_line: bool,
        /// One piece per cursor, as copied from as many cursors.
        pub multicursor_text: Option<Vec<String>>,
        /// The language the text was copied from.
        pub mode: Option<String>,
    }
}

command_action! {
    /// Peeks the references at `position` (Monaco
    /// `editor.action.findReferences`). Run in an editor, it does nothing
    /// when `uri` names another model.
    pub struct FindReferences {
        /// The model; unset for the editor's.
        pub uri: Option<String>,
        /// Where; unset for the cursor.
        pub position: Option<CommandPosition>,
    }
}

actions!(
    code_editor,
    [
        /// Accepts the focused suggestion (Monaco `acceptSelectedSuggestion`
        /// and `acceptSelectedSuggestionOnEnter`, Tab and Enter).
        AcceptSelectedSuggestion,
        /// Closes the suggest widget (Monaco `hideSuggestWidget`, Escape).
        HideSuggestWidget,
        /// Inserts the best completion without the widget (Monaco
        /// `insertBestCompletion`, Tab with `tabCompletion` on).
        InsertBestCompletion,
        /// Replaces the last tab completion with the next suggestion (Monaco
        /// `insertNextSuggestion`, Tab after a tab completion).
        InsertNextSuggestion,
        /// Replaces the last tab completion with the previous suggestion
        /// (Monaco `insertPrevSuggestion`, Shift+Tab after one).
        InsertPrevSuggestion,
        /// Selects the snippet's next tab stop (Monaco
        /// `jumpToNextSnippetPlaceholder`, Tab).
        JumpToNextSnippetPlaceholder,
        /// Selects the snippet's previous tab stop (Monaco
        /// `jumpToPrevSnippetPlaceholder`, Shift+Tab).
        JumpToPrevSnippetPlaceholder,
        /// Leaves snippet mode (Monaco `leaveSnippet`, Escape).
        LeaveSnippet,
        /// Leaves snippet mode at the final tab stop (Monaco
        /// `acceptSnippet`).
        AcceptSnippet,
        /// Stops linked editing (Monaco `cancelLinkedEditingInput`,
        /// Escape).
        CancelLinkedEditingInput,
        /// Closes the inline message (Monaco `leaveEditorMessage`, Escape).
        LeaveEditorMessage,
        /// Asks which characters, comments, strings or kinds to stop
        /// highlighting (Monaco
        /// `editor.action.unicodeHighlight.showExcludeOptions`).
        ShowUnicodeHighlightExcludeOptions,
        /// Moves to the previous word start, by Monaco's accessibility word
        /// navigation (Monaco `cursorWordAccessibilityLeft`).
        CursorWordAccessibilityLeft,
        /// Selects to the previous word start, by accessibility word
        /// navigation (Monaco `cursorWordAccessibilityLeftSelect`).
        CursorWordAccessibilityLeftSelect,
        /// Moves to the next word end, by accessibility word navigation
        /// (Monaco `cursorWordAccessibilityRight`).
        CursorWordAccessibilityRight,
        /// Selects to the next word end, by accessibility word navigation
        /// (Monaco `cursorWordAccessibilityRightSelect`).
        CursorWordAccessibilityRightSelect,
        /// Closes the find widget (Monaco `closeFindWidget`, Escape).
        CloseFindWidget,
        /// Closes the problem zone (Monaco `closeMarkersNavigation`,
        /// Escape).
        CloseMarkersNavigation,
        /// After going to one of several results, goes to the next (Monaco
        /// `editor.gotoNextSymbolFromResult`).
        GoToNextSymbolFromResult,
        /// Stops stepping through results (Monaco
        /// `editor.gotoNextSymbolFromResult.cancel`, Escape).
        CancelSymbolNavigation,
        /// Formats the selections, or the document when they're empty
        /// (Monaco's legacy `editor.action.format`).
        FormatSelectionOrDocument,
        /// Scrolls two columns left (Monaco `scrollLeft`).
        ScrollLeft,
        /// Scrolls two columns right (Monaco `scrollRight`).
        ScrollRight,
        /// An IME composition starts (Monaco's `compositionStart` handler).
        CompositionStart,
        /// The IME composition ends (Monaco's `compositionEnd` handler).
        CompositionEnd,
        /// Deletes what Cut would cut, without writing the clipboard
        /// (Monaco's `cut` handler; [`Cut`] is
        /// `editor.action.clipboardCutAction`).
        CutText,
    ]
);

/// Monaco's default keybindings for the current platform.
pub fn default_key_bindings() -> Vec<KeyBinding> {
    let ctx = Some(KEY_CONTEXT);
    let mut bindings = vec![
        // The browser's Tab and Shift+Tab, where the editor's own (which
        // need `editorTextFocus`) don't apply: in its widgets, they move
        // among the widget's controls and on out.
        KeyBinding::new("tab", FocusNextElement, ctx),
        KeyBinding::new("shift-tab", FocusPreviousElement, ctx),
        KeyBinding::new("left", CursorLeft, ctx),
        KeyBinding::new("shift-left", CursorLeftSelect, ctx),
        KeyBinding::new("right", CursorRight, ctx),
        KeyBinding::new("shift-right", CursorRightSelect, ctx),
        KeyBinding::new("up", CursorUp, ctx),
        KeyBinding::new("shift-up", CursorUpSelect, ctx),
        KeyBinding::new("down", CursorDown, ctx),
        KeyBinding::new("shift-down", CursorDownSelect, ctx),
        KeyBinding::new("home", CursorHome, ctx),
        KeyBinding::new("shift-home", CursorHomeSelect, ctx),
        KeyBinding::new("end", CursorEnd, ctx),
        KeyBinding::new("shift-end", CursorEndSelect, ctx),
        KeyBinding::new("pageup", CursorPageUp, ctx),
        KeyBinding::new("shift-pageup", CursorPageUpSelect, ctx),
        KeyBinding::new("pagedown", CursorPageDown, ctx),
        KeyBinding::new("shift-pagedown", CursorPageDownSelect, ctx),
        KeyBinding::new("secondary-a", SelectAll, ctx),
        KeyBinding::new("secondary-l", ExpandLineSelection, ctx),
        KeyBinding::new("escape", RemoveSecondaryCursors, ctx),
        // Monaco's secondary key for `cancelSelection`,
        // `removeSecondaryCursors`, `hideSuggestWidget`, `leaveSnippet`,
        // `cancelLinkedEditingInput`, `closeFindWidget`,
        // `closeMarkersNavigation` and `closeReferenceSearch`.
        KeyBinding::new("shift-escape", SecondaryEscape, ctx),
        KeyBinding::new("backspace", DeleteLeft, ctx),
        KeyBinding::new("shift-backspace", DeleteLeft, ctx),
        KeyBinding::new("delete", DeleteRight, ctx),
        KeyBinding::new("tab", Tab, ctx),
        KeyBinding::new("shift-tab", Outdent, ctx),
        KeyBinding::new("enter", Newline, ctx),
        KeyBinding::new("shift-enter", Newline, ctx),
        KeyBinding::new("secondary-]", IndentLines, ctx),
        KeyBinding::new("secondary-[", OutdentLines, ctx),
        KeyBinding::new("alt-up", MoveLinesUp, ctx),
        KeyBinding::new("alt-down", MoveLinesDown, ctx),
        KeyBinding::new("secondary-shift-k", DeleteLines, ctx),
        KeyBinding::new("secondary-enter", InsertLineAfter, ctx),
        KeyBinding::new("secondary-shift-enter", InsertLineBefore, ctx),
        KeyBinding::new("secondary-z", Undo, ctx),
        KeyBinding::new("secondary-c", Copy, ctx),
        KeyBinding::new("secondary-x", Cut, ctx),
        KeyBinding::new("secondary-v", Paste, ctx),
        KeyBinding::new("secondary-f", StartFind, ctx),
        KeyBinding::new("secondary-d", AddSelectionToNextFindMatch, ctx),
        KeyBinding::new("secondary-/", CommentLine, ctx),
        KeyBinding::new("secondary-u", CursorUndo, ctx),
        KeyBinding::new("f7", WordHighlightNext, ctx),
        KeyBinding::new("shift-f7", WordHighlightPrev, ctx),
        KeyBinding::new("secondary-shift-,", InPlaceReplaceUp, ctx),
        KeyBinding::new("secondary-shift-.", InPlaceReplaceDown, ctx),
        KeyBinding::new("secondary-alt-backspace", RemoveBrackets, ctx),
        KeyBinding::new("secondary-k secondary-x", TrimTrailingWhitespace, ctx),
        KeyBinding::new("insert", ToggleOvertypeInsertMode, ctx),
        KeyBinding::new("secondary-k secondary-b", SetSelectionAnchor, ctx),
        KeyBinding::new("secondary-k secondary-k", SelectFromAnchorToCursor, ctx),
        KeyBinding::new("shift-alt-i", InsertCursorAtEndOfEachLineSelected, ctx),
        KeyBinding::new("secondary-k secondary-c", AddCommentLine, ctx),
        KeyBinding::new("secondary-k secondary-u", RemoveCommentLine, ctx),
        KeyBinding::new("secondary-shift-l", SelectAllOccurrences, ctx),
        // Monaco `editor.action.changeAll`, writable editors only.
        KeyBinding::new(
            "secondary-f2",
            SelectAllOccurrences,
            Some("CodeEditor && editorTextFocus && !editorReadonly"),
        ),
        KeyBinding::new("secondary-shift-\\", JumpToBracket, ctx),
        KeyBinding::new("f3", NextMatch, ctx),
        KeyBinding::new("secondary-f3", NextSelectionMatch, ctx),
        KeyBinding::new("secondary-shift-f3", PreviousSelectionMatch, ctx),
        KeyBinding::new("alt-f8", GoToNextMarker, ctx),
        KeyBinding::new("f8", GoToNextMarkerInFiles, ctx),
        // `editor.gotoNextSymbolFromResult` (`EditorContrib`): Go to
        // Definition comes later and wins while its precondition holds.
        KeyBinding::new("f12", GoToNextSymbolFromResult, ctx),
        KeyBinding::new("f12", GoToDefinition, ctx),
        KeyBinding::new("shift-f12", GoToReferences, ctx),
        KeyBinding::new("f2", Rename, ctx),
        KeyBinding::new("secondary-.", QuickFix, ctx),
        KeyBinding::new("ctrl-shift-r", Refactor::default(), ctx),
        KeyBinding::new("shift-alt-o", OrganizeImports, ctx),
        KeyBinding::new(
            if cfg!(target_os = "macos") {
                "cmd-alt-."
            } else {
                "shift-alt-."
            },
            AutoFix,
            ctx,
        ),
        // Monaco binds F4 only while a peek view is open (or in its preview).
        KeyBinding::new("f4", GoToNextReference, REFERENCE_SEARCH),
        KeyBinding::new("shift-f4", GoToPreviousReference, REFERENCE_SEARCH),
        KeyBinding::new("secondary-shift-space", TriggerParameterHints, ctx),
        KeyBinding::new("ctrl-space", TriggerSuggest, ctx),
        // Monaco's secondary keys (Alt+Esc only on macOS).
        KeyBinding::new("secondary-i", TriggerSuggest, ctx),
        // Also Monaco `focusSuggestion` while no suggestion is focused.
        KeyBinding::new("ctrl-space", ToggleSuggestionDetails, SUGGEST),
        KeyBinding::new("secondary-/", ToggleExplainMode, SUGGEST),
        // Monaco: Cmd/Ctrl+Alt+Space, Ctrl+Alt+Space on macOS.
        KeyBinding::new("ctrl-alt-space", ToggleSuggestionFocus, SUGGEST),
        KeyBinding::new("secondary-k f12", RevealDefinitionAside, ctx),
        KeyBinding::new("secondary-k f2", TogglePeekWidgetFocus, ctx),
        KeyBinding::new("secondary-shift-f2", StartLinkedEditing, ctx),
        // Monaco binds Ctrl+G on every platform.
        KeyBinding::new("ctrl-g", GotoLine, ctx),
        KeyBinding::new("secondary-shift-o", QuickOutline, ctx),
        KeyBinding::new("f1", QuickCommand, ctx),
        KeyBinding::new("shift-f10", ShowContextMenu, ctx),
        // Monaco's `KeyCode.ContextMenu` (`contextmenu.ts`): GPUI calls the
        // Menu (Apps) key `menu` on Windows and Linux.
        KeyBinding::new("menu", ShowContextMenu, ctx),
        KeyBinding::new(
            if cfg!(target_os = "macos") {
                "ctrl-shift-m"
            } else {
                "ctrl-m"
            },
            ToggleTabFocusMode,
            ctx,
        ),
        // After Tab's bindings, so they win in tab focus mode and in
        // read-only editors, and before the rules that outweigh them.
        KeyBinding::new("tab", FocusNextElement, TAB_MOVES_FOCUS),
        KeyBinding::new("shift-tab", FocusPreviousElement, TAB_MOVES_FOCUS),
        KeyBinding::new("tab", Tab, TAB_OUTWEIGHS_FOCUS),
        KeyBinding::new("shift-tab", Outdent, SHIFT_TAB_OUTWEIGHS_FOCUS),
        // After QuickFix's binding, so they win while the button shows.
        KeyBinding::new("secondary-.", ChangePasteType, POST_EDIT_WIDGET),
        KeyBinding::new("secondary-.", ChangeDropType, POST_EDIT_WIDGET),
        KeyBinding::new("down", SelectNextStickyScrollLine, STICKY_FOCUSED),
        KeyBinding::new("up", SelectPreviousStickyScrollLine, STICKY_FOCUSED),
        KeyBinding::new("enter", GoToFocusedStickyScrollLine, STICKY_FOCUSED),
        KeyBinding::new("escape", SelectEditor, STICKY_FOCUSED),
        KeyBinding::new("up", FindHistoryPrevious, FIND_INPUT),
        KeyBinding::new("down", FindHistoryNext, FIND_INPUT),
        KeyBinding::new("down", ContextMenuNext, CONTEXT_MENU),
        KeyBinding::new("up", ContextMenuPrevious, CONTEXT_MENU),
        KeyBinding::new("home", ContextMenuFirst, CONTEXT_MENU),
        KeyBinding::new("end", ContextMenuLast, CONTEXT_MENU),
        KeyBinding::new("pageup", ContextMenuFirst, CONTEXT_MENU),
        KeyBinding::new("pagedown", ContextMenuLast, CONTEXT_MENU),
        KeyBinding::new("enter", ContextMenuSelect, CONTEXT_MENU),
        // Monaco's menu takes its trigger keys with the modifiers that
        // show alternatives (`ActionBar.isTriggerKeyEvent`); Shift is
        // added on Windows and Linux below, and Space where it's a
        // trigger key.
        KeyBinding::new("alt-enter", ContextMenuSelectAlternative, CONTEXT_MENU),
        KeyBinding::new(
            "alt-shift-enter",
            ContextMenuSelectAlternative,
            CONTEXT_MENU,
        ),
        KeyBinding::new("right", ContextMenuSubmenuOpen, CONTEXT_MENU),
        KeyBinding::new("left", ContextMenuSubmenuClose, CONTEXT_MENU),
        KeyBinding::new("escape", ContextMenuHide, CONTEXT_MENU),
        KeyBinding::new("escape", HideColorPicker, STANDALONE_COLOR_PICKER),
        KeyBinding::new(
            "enter",
            InsertColorWithStandaloneColorPicker,
            STANDALONE_COLOR_PICKER,
        ),
        // In the quick access input (the list's keys win over the input's;
        // more below).
        KeyBinding::new("down", QuickInputNext, QUICK_INPUT),
        KeyBinding::new("up", QuickInputPrevious, QUICK_INPUT),
        KeyBinding::new("pagedown", QuickInputPageNext, QUICK_INPUT),
        KeyBinding::new("pageup", QuickInputPagePrevious, QUICK_INPUT),
        // VS Code `quickInput.first`/`last` (`quickInputActions.ts`):
        // Ctrl+Home/End on every platform (WinCtrl on macOS), also with Alt;
        // `next`/`previous` also with Ctrl.
        KeyBinding::new("ctrl-home", QuickInputFirst, QUICK_INPUT),
        KeyBinding::new("ctrl-end", QuickInputLast, QUICK_INPUT),
        KeyBinding::new("ctrl-alt-home", QuickInputFirst, QUICK_INPUT),
        KeyBinding::new("ctrl-alt-end", QuickInputLast, QUICK_INPUT),
        KeyBinding::new("ctrl-down", QuickInputNext, QUICK_INPUT),
        KeyBinding::new("ctrl-up", QuickInputPrevious, QUICK_INPUT),
        // Monaco's quick input closes on Escape only.
        KeyBinding::new("shift-escape", gpui::NoAction, QUICK_INPUT),
        KeyBinding::new("alt-]", ShowNextInlineSuggestion, INLINE),
        KeyBinding::new("alt-[", ShowPreviousInlineSuggestion, INLINE),
        KeyBinding::new(
            if cfg!(target_os = "macos") {
                "cmd-right"
            } else {
                "ctrl-right"
            },
            AcceptNextInlineWord,
            // Monaco leaves the key to word navigation for screen readers.
            Some(
                "CodeEditor && inline_suggestion_visible && cursor_before_ghost_text && !accessibilityModeEnabled",
            ),
        ),
        KeyBinding::new(
            if cfg!(target_os = "macos") {
                "cmd-i"
            } else {
                "ctrl-i"
            },
            ToggleSuggestionDetails,
            SUGGEST,
        ),
        KeyBinding::new("shift-enter", AcceptAlternativeSuggestion, SUGGEST),
        KeyBinding::new("shift-tab", AcceptAlternativeSuggestion, SUGGEST),
        // Monaco's second `jumpToPrevSnippetPlaceholder` rule
        // (`EditorContrib + 91`): in a snippet choice, whose suggest widget
        // shows, Shift+Tab goes back instead of accepting the alternative.
        KeyBinding::new(
            "shift-tab",
            JumpToPrevSnippetPlaceholder,
            Some("CodeEditor && editorTextFocus && hasPrevTabstop && inSnippetChoice"),
        ),
        KeyBinding::new("shift-alt-f8", GoToPreviousMarker, ctx),
        KeyBinding::new("shift-f8", GoToPreviousMarkerInFiles, ctx),
        KeyBinding::new("shift-f3", PreviousMatch, ctx),
        // Monaco `editor.action.selectAllMatches` (`editorFocus`, so also
        // from the find inputs; its precondition `findWidgetVisible` is
        // folded in below).
        KeyBinding::new("alt-enter", SelectAllMatches, ctx),
        // Inside the find widget's inputs: Shift+Enter is Find Previous in
        // the find input only; in the replace input no command takes it, so
        // Monaco's textarea inserts a line break. Cmd+Enter (macOS) is
        // bound in the replace input only, below.
        KeyBinding::new("shift-enter", PreviousMatch, FIND_INPUT_ONLY),
        KeyBinding::new("shift-enter", FindInputNewline, REPLACE_INPUT_ONLY),
        // After those, so they win: Monaco's find inputs take Ctrl+Enter
        // (on macOS too) for a line break.
        KeyBinding::new("ctrl-enter", FindInputNewline, FIND_INPUT),
        // Monaco `suggestWidgetCopy` (and Ctrl+Insert on Windows), after
        // Copy so it wins while the details panel has the focus.
        KeyBinding::new("secondary-c", SuggestWidgetCopy, SUGGEST_DETAILS_FOCUSED),
    ];

    bindings.push(KeyBinding::new(
        if cfg!(target_os = "linux") {
            "ctrl-shift-a"
        } else {
            "shift-alt-a"
        },
        BlockComment,
        ctx,
    ));

    if cfg!(any(target_os = "windows", target_os = "linux")) {
        // There Shift shows the context menu items' alternatives too
        // (`getActionForAltMode`).
        bindings.push(KeyBinding::new(
            "shift-enter",
            ContextMenuSelectAlternative,
            CONTEXT_MENU,
        ));
    }
    if cfg!(not(target_os = "windows")) {
        // Space is a trigger key of Monaco's menu on macOS and Linux only
        // (`Menu`'s `triggerKeys`), with the alternatives' modifiers.
        bindings.extend([
            KeyBinding::new("space", ContextMenuSelect, CONTEXT_MENU),
            KeyBinding::new("alt-space", ContextMenuSelectAlternative, CONTEXT_MENU),
            KeyBinding::new(
                "alt-shift-space",
                ContextMenuSelectAlternative,
                CONTEXT_MENU,
            ),
        ]);
        if cfg!(target_os = "linux") {
            bindings.push(KeyBinding::new(
                "shift-space",
                ContextMenuSelectAlternative,
                CONTEXT_MENU,
            ));
        }
    }

    if cfg!(target_os = "macos") {
        // The quick input's macOS keys: `quickInput.first`/`last` with Cmd
        // added (`withCmdMod`), and VS Code's `quickOpenSelectNext` and
        // `quickOpenSelectPrevious` (Ctrl+N/P, macOS only).
        bindings.extend([
            KeyBinding::new("cmd-ctrl-home", QuickInputFirst, QUICK_INPUT),
            KeyBinding::new("cmd-ctrl-end", QuickInputLast, QUICK_INPUT),
            KeyBinding::new("cmd-ctrl-alt-home", QuickInputFirst, QUICK_INPUT),
            KeyBinding::new("cmd-ctrl-alt-end", QuickInputLast, QUICK_INPUT),
            KeyBinding::new("ctrl-n", QuickInputNext, QUICK_INPUT),
            KeyBinding::new("ctrl-p", QuickInputPrevious, QUICK_INPUT),
        ]);
    }

    if cfg!(target_os = "macos") {
        bindings.extend([
            KeyBinding::new("alt-left", CursorWordLeft, ctx),
            KeyBinding::new("shift-alt-left", CursorWordLeftSelect, ctx),
            KeyBinding::new("alt-right", CursorWordRight, ctx),
            KeyBinding::new("shift-alt-right", CursorWordRightSelect, ctx),
            KeyBinding::new("ctrl-alt-left", CursorWordPartLeft, ctx),
            KeyBinding::new("ctrl-alt-backspace", DeleteWordPartLeft, ctx),
            KeyBinding::new("ctrl-alt-delete", DeleteWordPartRight, ctx),
            KeyBinding::new("ctrl-o", LineBreakInsert, ctx),
            KeyBinding::new("ctrl-k", DeleteAllRight, ctx),
            KeyBinding::new("cmd-delete", DeleteAllRight, ctx),
            KeyBinding::new("ctrl-j", JoinLines, ctx),
            KeyBinding::new("ctrl-t", TransposeLetters, ctx),
            KeyBinding::new("ctrl-a", CursorLineStart, ctx),
            KeyBinding::new("ctrl-shift-a", CursorLineStartSelect, ctx),
            KeyBinding::new("ctrl-shift-e", CursorLineEndSelect, ctx),
            // Monaco's Emacs-style secondary keys.
            KeyBinding::new("ctrl-b", CursorLeft, ctx),
            KeyBinding::new("ctrl-f", CursorRight, ctx),
            KeyBinding::new("ctrl-p", CursorUp, ctx),
            KeyBinding::new("ctrl-n", CursorDown, ctx),
            KeyBinding::new("ctrl-backspace", DeleteLeft, ctx),
            KeyBinding::new("ctrl-delete", DeleteRight, ctx),
            KeyBinding::new("alt-escape", TriggerSuggest, ctx),
            KeyBinding::new("cmd-shift-alt-left", CursorColumnSelectLeft, ctx),
            KeyBinding::new("cmd-shift-alt-right", CursorColumnSelectRight, ctx),
            KeyBinding::new("cmd-shift-alt-up", CursorColumnSelectUp, ctx),
            KeyBinding::new("cmd-shift-alt-down", CursorColumnSelectDown, ctx),
            KeyBinding::new("cmd-shift-alt-pageup", CursorColumnSelectPageUp, ctx),
            KeyBinding::new("cmd-shift-alt-pagedown", CursorColumnSelectPageDown, ctx),
            KeyBinding::new("ctrl-e", CursorLineEnd, ctx),
            KeyBinding::new("ctrl-pageup", ScrollLineUp, ctx),
            KeyBinding::new("ctrl-pagedown", ScrollLineDown, ctx),
            KeyBinding::new("cmd-pageup", ScrollPageUp, ctx),
            KeyBinding::new("cmd-pagedown", ScrollPageDown, ctx),
            KeyBinding::new("ctrl-shift-alt-left", CursorWordPartLeftSelect, ctx),
            KeyBinding::new("ctrl-alt-right", CursorWordPartRight, ctx),
            KeyBinding::new("ctrl-shift-alt-right", CursorWordPartRightSelect, ctx),
            KeyBinding::new("cmd-left", CursorHome, ctx),
            KeyBinding::new("cmd-shift-left", CursorHomeSelect, ctx),
            KeyBinding::new("cmd-right", CursorEnd, ctx),
            KeyBinding::new("cmd-shift-right", CursorEndSelect, ctx),
            KeyBinding::new("cmd-up", CursorTop, ctx),
            KeyBinding::new("cmd-shift-up", CursorTopSelect, ctx),
            KeyBinding::new("cmd-down", CursorBottom, ctx),
            KeyBinding::new("cmd-shift-down", CursorBottomSelect, ctx),
            KeyBinding::new("cmd-home", CursorTop, ctx),
            KeyBinding::new("cmd-end", CursorBottom, ctx),
            KeyBinding::new("shift-alt-up", CopyLinesUp, ctx),
            KeyBinding::new("shift-alt-down", CopyLinesDown, ctx),
            KeyBinding::new("cmd-alt-up", AddCursorAbove::default(), ctx),
            KeyBinding::new("cmd-alt-down", AddCursorBelow::default(), ctx),
            KeyBinding::new("alt-backspace", DeleteWordLeft, ctx),
            KeyBinding::new("alt-delete", DeleteWordRight, ctx),
            KeyBinding::new("cmd-backspace", DeleteAllLeft, ctx),
            KeyBinding::new("ctrl-h", DeleteLeft, ctx),
            KeyBinding::new("ctrl-d", DeleteRight, ctx),
            KeyBinding::new("cmd-shift-z", Redo, ctx),
            KeyBinding::new("ctrl-cmd-space", ShowCharacterPalette, ctx),
            KeyBinding::new("cmd-alt-[", Fold::default(), ctx),
            KeyBinding::new("cmd-alt-]", Unfold::default(), ctx),
            KeyBinding::new("cmd-k cmd-0", FoldAll, ctx),
            KeyBinding::new("cmd-k cmd-j", UnfoldAll, ctx),
            KeyBinding::new("cmd-k cmd-l", ToggleFold, ctx),
            KeyBinding::new("ctrl-cmd-shift-right", ExpandSelection, ctx),
            KeyBinding::new("ctrl-shift-right", ExpandSelection, ctx),
            KeyBinding::new("ctrl-cmd-shift-left", ShrinkSelection, ctx),
            KeyBinding::new("ctrl-shift-left", ShrinkSelection, ctx),
            KeyBinding::new("cmd-k cmd-[", FoldRecursively, ctx),
            KeyBinding::new("cmd-k cmd-]", UnfoldRecursively, ctx),
            KeyBinding::new("cmd-k cmd-shift-l", ToggleFoldRecursively, ctx),
            KeyBinding::new("cmd-k cmd-/", FoldAllBlockComments, ctx),
            KeyBinding::new("cmd-k cmd-8", FoldAllMarkerRegions, ctx),
            KeyBinding::new("cmd-k cmd-9", UnfoldAllMarkerRegions, ctx),
            KeyBinding::new("cmd-k cmd--", FoldAllExcept, ctx),
            KeyBinding::new("cmd-k cmd-=", UnfoldAllExcept, ctx),
            KeyBinding::new("cmd-k cmd-1", FoldLevel1, ctx),
            KeyBinding::new("cmd-k cmd-2", FoldLevel2, ctx),
            KeyBinding::new("cmd-k cmd-3", FoldLevel3, ctx),
            KeyBinding::new("cmd-k cmd-4", FoldLevel4, ctx),
            KeyBinding::new("cmd-k cmd-5", FoldLevel5, ctx),
            KeyBinding::new("cmd-k cmd-6", FoldLevel6, ctx),
            KeyBinding::new("cmd-k cmd-7", FoldLevel7, ctx),
            KeyBinding::new("cmd-k cmd-,", CreateFoldFromSelection, ctx),
            KeyBinding::new("cmd-k cmd-.", RemoveManualFoldingRanges, ctx),
            KeyBinding::new("cmd-alt-f", StartFindReplace, ctx),
            KeyBinding::new("cmd-k cmd-d", MoveSelectionToNextFindMatch, ctx),
            KeyBinding::new("cmd-k cmd-i", ShowHover::default(), ctx),
            KeyBinding::new("shift-alt-f", FormatDocument, ctx),
            KeyBinding::new("cmd-k cmd-f", FormatSelection, ctx),
            KeyBinding::new("cmd-f12", GoToImplementation, ctx),
            KeyBinding::new("cmd-shift-f12", PeekImplementation, ctx),
            KeyBinding::new("alt-f12", PeekDefinition, ctx),
            KeyBinding::new("cmd-g", NextMatch, ctx),
            KeyBinding::new("cmd-e", StartFindWithSelection, ctx),
            KeyBinding::new("cmd-shift-g", PreviousMatch, ctx),
            KeyBinding::new("cmd-alt-enter", ReplaceAll, ctx),
            KeyBinding::new("cmd-shift-1", ReplaceOne, ctx),
            KeyBinding::new("cmd-alt-c", ToggleCaseSensitive, ctx),
            KeyBinding::new("cmd-alt-r", ToggleRegex, ctx),
            KeyBinding::new("cmd-alt-w", ToggleWholeWord, ctx),
            KeyBinding::new("cmd-alt-l", ToggleFindInSelection, ctx),
            KeyBinding::new("cmd-alt-p", TogglePreserveCase, ctx),
        ]);
    } else {
        bindings.extend([
            KeyBinding::new("ctrl-left", CursorWordLeft, ctx),
            KeyBinding::new("ctrl-shift-left", CursorWordLeftSelect, ctx),
            KeyBinding::new("ctrl-right", CursorWordRight, ctx),
            KeyBinding::new("ctrl-shift-right", CursorWordRightSelect, ctx),
            KeyBinding::new("shift-alt-right", ExpandSelection, ctx),
            KeyBinding::new("shift-alt-left", ShrinkSelection, ctx),
            KeyBinding::new("ctrl-home", CursorTop, ctx),
            KeyBinding::new("ctrl-shift-home", CursorTopSelect, ctx),
            KeyBinding::new("ctrl-end", CursorBottom, ctx),
            KeyBinding::new("ctrl-shift-end", CursorBottomSelect, ctx),
            KeyBinding::new("ctrl-backspace", DeleteWordLeft, ctx),
            KeyBinding::new("ctrl-delete", DeleteWordRight, ctx),
            KeyBinding::new("ctrl-y", Redo, ctx),
            KeyBinding::new("ctrl-shift-z", Redo, ctx),
            KeyBinding::new("shift-insert", Paste, ctx),
            KeyBinding::new("ctrl-shift-[", Fold::default(), ctx),
            KeyBinding::new("ctrl-shift-]", Unfold::default(), ctx),
            KeyBinding::new("ctrl-k ctrl-0", FoldAll, ctx),
            KeyBinding::new("ctrl-k ctrl-j", UnfoldAll, ctx),
            KeyBinding::new("ctrl-k ctrl-l", ToggleFold, ctx),
            KeyBinding::new("ctrl-k ctrl-[", FoldRecursively, ctx),
            KeyBinding::new("ctrl-k ctrl-]", UnfoldRecursively, ctx),
            KeyBinding::new("ctrl-k ctrl-shift-l", ToggleFoldRecursively, ctx),
            KeyBinding::new("ctrl-k ctrl-/", FoldAllBlockComments, ctx),
            KeyBinding::new("ctrl-k ctrl-8", FoldAllMarkerRegions, ctx),
            KeyBinding::new("ctrl-k ctrl-9", UnfoldAllMarkerRegions, ctx),
            KeyBinding::new("ctrl-k ctrl--", FoldAllExcept, ctx),
            KeyBinding::new("ctrl-k ctrl-=", UnfoldAllExcept, ctx),
            KeyBinding::new("ctrl-k ctrl-1", FoldLevel1, ctx),
            KeyBinding::new("ctrl-k ctrl-2", FoldLevel2, ctx),
            KeyBinding::new("ctrl-k ctrl-3", FoldLevel3, ctx),
            KeyBinding::new("ctrl-k ctrl-4", FoldLevel4, ctx),
            KeyBinding::new("ctrl-k ctrl-5", FoldLevel5, ctx),
            KeyBinding::new("ctrl-k ctrl-6", FoldLevel6, ctx),
            KeyBinding::new("ctrl-k ctrl-7", FoldLevel7, ctx),
            KeyBinding::new("ctrl-k ctrl-,", CreateFoldFromSelection, ctx),
            KeyBinding::new("ctrl-k ctrl-.", RemoveManualFoldingRanges, ctx),
            KeyBinding::new("ctrl-h", StartFindReplace, ctx),
            KeyBinding::new("ctrl-k ctrl-d", MoveSelectionToNextFindMatch, ctx),
            KeyBinding::new("ctrl-k ctrl-i", ShowHover::default(), ctx),
            KeyBinding::new("ctrl-k ctrl-f", FormatSelection, ctx),
            KeyBinding::new("ctrl-f12", GoToImplementation, ctx),
            KeyBinding::new("ctrl-shift-f12", PeekImplementation, ctx),
            KeyBinding::new(
                if cfg!(target_os = "linux") {
                    "ctrl-shift-f10"
                } else {
                    "alt-f12"
                },
                PeekDefinition,
                ctx,
            ),
            KeyBinding::new(
                if cfg!(target_os = "linux") {
                    "ctrl-shift-i"
                } else {
                    "shift-alt-f"
                },
                FormatDocument,
                ctx,
            ),
            KeyBinding::new("ctrl-alt-enter", ReplaceAll, ctx),
            KeyBinding::new("ctrl-shift-1", ReplaceOne, ctx),
            KeyBinding::new("alt-c", ToggleCaseSensitive, ctx),
            KeyBinding::new("alt-r", ToggleRegex, ctx),
            KeyBinding::new("alt-w", ToggleWholeWord, ctx),
            KeyBinding::new("alt-l", ToggleFindInSelection, ctx),
            KeyBinding::new("alt-p", TogglePreserveCase, ctx),
        ]);
        bindings.extend([
            KeyBinding::new("ctrl-up", ScrollLineUp, ctx),
            KeyBinding::new("ctrl-down", ScrollLineDown, ctx),
            KeyBinding::new("alt-pageup", ScrollPageUp, ctx),
            KeyBinding::new("alt-pagedown", ScrollPageDown, ctx),
        ]);
        if cfg!(target_os = "windows") {
            bindings.extend([
                KeyBinding::new("ctrl-shift-alt-left", CursorColumnSelectLeft, ctx),
                KeyBinding::new("ctrl-shift-alt-right", CursorColumnSelectRight, ctx),
                KeyBinding::new("ctrl-shift-alt-up", CursorColumnSelectUp, ctx),
                KeyBinding::new("ctrl-shift-alt-down", CursorColumnSelectDown, ctx),
                KeyBinding::new("ctrl-shift-alt-pageup", CursorColumnSelectPageUp, ctx),
                KeyBinding::new("ctrl-shift-alt-pagedown", CursorColumnSelectPageDown, ctx),
            ]);
        }
        if cfg!(target_os = "windows") {
            // Monaco binds these secondaries on Windows only.
            bindings.extend([
                KeyBinding::new("shift-delete", Cut, ctx),
                KeyBinding::new("ctrl-insert", Copy, ctx),
                KeyBinding::new("ctrl-insert", SuggestWidgetCopy, SUGGEST_DETAILS_FOCUSED),
                KeyBinding::new("ctrl-shift-up", CursorUpSelect, ctx),
                KeyBinding::new("ctrl-shift-down", CursorDownSelect, ctx),
            ]);
        }
        if cfg!(target_os = "linux") {
            bindings.extend([
                KeyBinding::new("ctrl-shift-up", AddCursorAbove::default(), ctx),
                KeyBinding::new("ctrl-shift-down", AddCursorBelow::default(), ctx),
            ]);
        }
        if cfg!(target_os = "windows") {
            bindings.extend([
                KeyBinding::new("ctrl-alt-up", AddCursorAbove::default(), ctx),
                KeyBinding::new("ctrl-alt-down", AddCursorBelow::default(), ctx),
                KeyBinding::new("shift-alt-up", CopyLinesUp, ctx),
                KeyBinding::new("shift-alt-down", CopyLinesDown, ctx),
            ]);
        } else {
            bindings.extend([
                KeyBinding::new("shift-alt-up", AddCursorAbove::default(), ctx),
                KeyBinding::new("shift-alt-down", AddCursorBelow::default(), ctx),
                KeyBinding::new("ctrl-shift-alt-up", CopyLinesUp, ctx),
                KeyBinding::new("ctrl-shift-alt-down", CopyLinesDown, ctx),
            ]);
        }
    }
    // The code action menu. Monaco's secondary keys (Cmd/Ctrl+. accepts,
    // Cmd/Ctrl+Up/Down move) come first, so the row titles name the
    // primary ones.
    let menu = Some(crate::code_action::CODE_ACTION_MENU_KEY_CONTEXT);
    bindings.extend([
        KeyBinding::new("secondary-down", CodeActionMenuNext, menu),
        KeyBinding::new("secondary-up", CodeActionMenuPrevious, menu),
        KeyBinding::new("secondary-.", CodeActionMenuAccept, menu),
        KeyBinding::new("down", CodeActionMenuNext, menu),
        KeyBinding::new("up", CodeActionMenuPrevious, menu),
        KeyBinding::new("enter", CodeActionMenuAccept, menu),
        KeyBinding::new("secondary-enter", CodeActionMenuPreview, menu),
        KeyBinding::new("escape", CodeActionMenuHide, menu),
        KeyBinding::new("shift-escape", CodeActionMenuHide, menu),
    ]);
    if cfg!(target_os = "macos") {
        bindings.extend([
            KeyBinding::new("ctrl-n", CodeActionMenuNext, menu),
            KeyBinding::new("ctrl-p", CodeActionMenuPrevious, menu),
        ]);
    }
    // The rename box's input (Monaco `renameInputVisible`).
    let rename = Some("RenameInput > CodeEditor");
    bindings.extend([
        KeyBinding::new("enter", AcceptRenameInput, rename),
        KeyBinding::new("secondary-enter", AcceptRenameInputWithPreview, rename),
        KeyBinding::new("escape", CancelRenameInput, rename),
        KeyBinding::new("shift-escape", CancelRenameInput, rename),
        KeyBinding::new("down", FocusNextRenameSuggestion, rename),
        KeyBinding::new("up", FocusPreviousRenameSuggestion, rename),
    ]);
    // A focused hover (Monaco `hoverFocused`).
    let hover = Some(crate::hover::HOVER_KEY_CONTEXT);
    bindings.extend([
        KeyBinding::new("up", ScrollUpHover, hover),
        KeyBinding::new("down", ScrollDownHover, hover),
        KeyBinding::new("left", ScrollLeftHover, hover),
        KeyBinding::new("right", ScrollRightHover, hover),
        KeyBinding::new("pageup", PageUpHover, hover),
        KeyBinding::new("alt-up", PageUpHover, hover),
        KeyBinding::new("pagedown", PageDownHover, hover),
        KeyBinding::new("alt-down", PageDownHover, hover),
        KeyBinding::new("home", GoToTopHover, hover),
        KeyBinding::new("secondary-up", GoToTopHover, hover),
        KeyBinding::new("end", GoToBottomHover, hover),
        KeyBinding::new("secondary-down", GoToBottomHover, hover),
        KeyBinding::new("escape", HideHover, hover),
    ]);
    // The peek view's results list.
    let peek = Some(crate::peek::PEEK_KEY_CONTEXT);
    bindings.extend([
        KeyBinding::new("down", PeekSelectNext, peek),
        KeyBinding::new("up", PeekSelectPrevious, peek),
        KeyBinding::new("home", PeekSelectFirst, peek),
        KeyBinding::new("end", PeekSelectLast, peek),
        KeyBinding::new("pagedown", PeekPageDown, peek),
        KeyBinding::new("pageup", PeekPageUp, peek),
        KeyBinding::new("enter", PeekOpen, peek),
        // Monaco: Ctrl+Enter (WinCtrl on macOS, CtrlCmd elsewhere).
        KeyBinding::new("ctrl-enter", PeekOpenToSide, peek),
        KeyBinding::new("space", PeekToggle, peek),
        KeyBinding::new("left", PeekCollapse, peek),
        KeyBinding::new("right", PeekExpand, peek),
        KeyBinding::new("escape", ClosePeek, peek),
        KeyBinding::new("shift-escape", ClosePeek, peek),
        KeyBinding::new("f4", GoToNextReference, peek),
        // Monaco's secondary keys (the editor keeps F12 for go to
        // definition while it has text focus).
        KeyBinding::new("f12", GoToNextReference, peek),
        KeyBinding::new("shift-f4", GoToPreviousReference, peek),
        KeyBinding::new("shift-f12", GoToPreviousReference, peek),
    ]);
    // Monaco's macOS secondary key for `revealReference`.
    if cfg!(target_os = "macos") {
        bindings.push(KeyBinding::new("cmd-down", PeekOpen, peek));
    }
    // Monaco's secondary keys for moving in the suggest list (first and last
    // have none). Bound last: they share the editor's context depth, where
    // later bindings win, and must beat Cmd/Ctrl+Up/Down and Ctrl+N/P.
    bindings.extend([
        KeyBinding::new("secondary-down", SelectNextSuggestion, SUGGEST_NAVIGABLE),
        KeyBinding::new("secondary-up", SelectPreviousSuggestion, SUGGEST_NAVIGABLE),
        KeyBinding::new(
            "secondary-pagedown",
            SelectNextPageSuggestion,
            SUGGEST_NAVIGABLE,
        ),
        KeyBinding::new(
            "secondary-pageup",
            SelectPreviousPageSuggestion,
            SUGGEST_NAVIGABLE,
        ),
    ]);
    if cfg!(target_os = "macos") {
        bindings.extend([
            KeyBinding::new("ctrl-n", SelectNextSuggestion, SUGGEST_NAVIGABLE),
            KeyBinding::new("ctrl-p", SelectPreviousSuggestion, SUGGEST_NAVIGABLE),
        ]);
    }
    // Parameter hints (Monaco weighs them above the editor's own keys, such
    // as Escape and Alt+Up/Down for moving lines): bound after those.
    bindings.extend([
        KeyBinding::new("escape", CloseParameterHints, PARAMETER_HINTS),
        KeyBinding::new("shift-escape", CloseParameterHints, PARAMETER_HINTS),
        KeyBinding::new(
            "down",
            ShowNextParameterHint,
            PARAMETER_HINTS_MULTIPLE_NO_SUGGEST,
        ),
        KeyBinding::new(
            "up",
            ShowPrevParameterHint,
            PARAMETER_HINTS_MULTIPLE_NO_SUGGEST,
        ),
        KeyBinding::new("alt-down", ShowNextParameterHint, PARAMETER_HINTS_MULTIPLE),
        KeyBinding::new("alt-up", ShowPrevParameterHint, PARAMETER_HINTS_MULTIPLE),
    ]);
    if cfg!(target_os = "macos") {
        bindings.extend([
            KeyBinding::new(
                "ctrl-n",
                ShowNextParameterHint,
                PARAMETER_HINTS_MULTIPLE_NO_SUGGEST,
            ),
            KeyBinding::new(
                "ctrl-p",
                ShowPrevParameterHint,
                PARAMETER_HINTS_MULTIPLE_NO_SUGGEST,
            ),
        ]);
    }
    if cfg!(target_os = "windows") {
        // Monaco gives Ctrl+Left/Right to accessibility word navigation
        // while a screen reader is in use.
        let accessibility = Some("CodeEditor && accessibilityModeEnabled");
        bindings.extend([
            KeyBinding::new("ctrl-left", CursorWordAccessibilityLeft, accessibility),
            KeyBinding::new(
                "ctrl-shift-left",
                CursorWordAccessibilityLeftSelect,
                accessibility,
            ),
            KeyBinding::new("ctrl-right", CursorWordAccessibilityRight, accessibility),
            KeyBinding::new(
                "ctrl-shift-right",
                CursorWordAccessibilityRightSelect,
                accessibility,
            ),
        ]);
    }
    // The find widget's inputs' own keys (Monaco `_onFindInputKeyDown` and
    // `_onReplaceInputKeyDown`), last so they beat the input editor's.
    bindings.push(KeyBinding::new(
        "secondary-down",
        FindInputFocusEditor,
        FIND_INPUT,
    ));
    if cfg!(target_os = "macos") {
        // Monaco binds Cmd+Enter to Replace All in the replace input on
        // macOS only; elsewhere Ctrl+Enter inserts a line break.
        bindings.extend([
            KeyBinding::new("cmd-enter", ReplaceAll, REPLACE_INPUT_ONLY),
            KeyBinding::new("cmd-enter", gpui::NoAction, FIND_INPUT_ONLY),
        ]);
    }
    // Monaco folds an action's precondition into its keybindings' `when`.
    crate::editor_actions::with_preconditions(with_text_focus(by_precedence(bindings)))
}

/// Whether Monaco's keybinding for `action` also works while one of the
/// editor's widgets has the focus: its `kbExpr` is `editorFocus` (the find,
/// multi-cursor, marker navigation and quick access commands) or none
/// (`undo`, `redo`, `editor.action.selectAll`, the find widget's openers,
/// `editor.gotoNextSymbolFromResult`, `togglePeekWidgetFocus`, Toggle Tab
/// Key Moves Focus). Escape's and Shift+Escape's chains start with such
/// rules (`closeFindWidget`, `closeMarkersNavigation`, …), and the plain
/// Tab and Shift+Tab move the focus as the browser's do.
fn works_from_widgets(action: &dyn Action) -> bool {
    macro_rules! any_of {
        ($($action:ty),* $(,)?) => {
            $(action.as_any().is::<$action>())||*
        };
    }
    any_of!(
        StartFind,
        StartFindReplace,
        StartFindWithSelection,
        NextMatch,
        PreviousMatch,
        NextSelectionMatch,
        PreviousSelectionMatch,
        ToggleCaseSensitive,
        ToggleWholeWord,
        ToggleRegex,
        ToggleFindInSelection,
        TogglePreserveCase,
        ReplaceOne,
        ReplaceAll,
        SelectAllMatches,
        AddSelectionToNextFindMatch,
        MoveSelectionToNextFindMatch,
        SelectAllOccurrences,
        GoToNextMarker,
        GoToPreviousMarker,
        GoToNextMarkerInFiles,
        GoToPreviousMarkerInFiles,
        GotoLine,
        QuickOutline,
        QuickCommand,
        GoToNextSymbolFromResult,
        TogglePeekWidgetFocus,
        Undo,
        Redo,
        SelectAll,
        ToggleTabFocusMode,
        RemoveSecondaryCursors,
        SecondaryEscape,
        FocusNextElement,
        FocusPreviousElement,
    )
}

/// `bindings` with Monaco's `textInputFocus` required by the plain
/// [`KEY_CONTEXT`] ones whose Monaco keybindings need it or
/// `editorTextFocus`: all but [`works_from_widgets`]'s.
///
/// GPUI matches a binding at any depth of the focused element's context
/// stack, and the editor's widgets draw inside its key context, so without
/// it Enter in a focused hover ran `Newline` on the document. A widget's
/// input (the find, rename and quick input boxes) stands in for Monaco's
/// plain `<input>`, whose keys the browser handles: it has
/// `textInputFocus` but not `editorTextFocus`, so these keys still edit
/// it while the host's `editorTextFocus` rules don't apply there. A
/// focused sticky line has neither.
fn with_text_focus(bindings: Vec<KeyBinding>) -> Vec<KeyBinding> {
    use gpui::KeyBindingContextPredicate as P;
    let plain = P::Identifier(KEY_CONTEXT.into());
    let text_focus = P::And(
        Box::new(plain.clone()),
        Box::new(P::Identifier("textInputFocus".into())),
    );
    bindings
        .into_iter()
        .map(|binding| {
            if !binding.predicate().is_some_and(|p| *p == plain)
                || works_from_widgets(binding.action())
            {
                return binding;
            }
            let keystrokes: Vec<String> = binding
                .keystrokes()
                .iter()
                .map(|keystroke| keystroke.inner().unparse())
                .collect();
            KeyBinding::load(
                &keystrokes.join(" "),
                binding.action().boxed_clone(),
                Some(std::rc::Rc::new(text_focus.clone())),
                false,
                None,
                &gpui::DummyKeyboardMapper,
            )
            .unwrap_or(binding)
        })
        .collect()
}

/// `bindings` in the order that gives Monaco's precedence.
///
/// Monaco runs the matching rule with the highest weight, and its widget
/// and state rules (`EditorContrib` and up) outweigh the core keys
/// (`EditorCore`) they share keys with. GPUI runs, of the bindings that
/// match at the same depth, the one added last. So the plain
/// [`KEY_CONTEXT`] bindings, the platform blocks included, go first and
/// every binding with a more specific context after them, each group in
/// the order it was listed in. Before both come the few rules Monaco
/// weighs below the editor's own keys.
fn by_precedence(bindings: Vec<KeyBinding>) -> Vec<KeyBinding> {
    let plain = gpui::KeyBindingContextPredicate::Identifier(KEY_CONTEXT.into());
    let (plain, contextual): (Vec<_>, Vec<_>) = bindings
        .into_iter()
        .partition(|binding| binding.predicate().is_some_and(|p| *p == plain));
    // Monaco `goToNextReference`'s and `goToPreviousReference`'s secondary
    // keys (weight `EditorContrib - 10`): Go to Definition, Go to References
    // and `editor.gotoNextSymbolFromResult` (`EditorContrib`) win while
    // their preconditions hold.
    let weaker = [
        KeyBinding::new("f12", GoToNextReference, REFERENCE_SEARCH),
        KeyBinding::new("shift-f12", GoToPreviousReference, REFERENCE_SEARCH),
    ];
    weaker.into_iter().chain(plain).chain(contextual).collect()
}

/// Binds [`default_key_bindings`].
pub fn bind_default_keys(cx: &mut App) {
    cx.bind_keys(default_key_bindings());
}
