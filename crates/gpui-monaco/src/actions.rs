//! Editor commands as GPUI actions, and Monaco's default keybindings.
//!
//! Every binding lives in the [`KEY_CONTEXT`] key context, so hosts can
//! override any of them by binding the same keystroke in that context after
//! [`crate::init`], or skip [`default_key_bindings`] entirely and bind their
//! own.

use gpui::{App, KeyBinding, actions};

/// The key context the editor element establishes.
pub const KEY_CONTEXT: &str = "CodeEditor";

/// Single-line editors (e.g. the find input) also set `single_line`.
const SINGLE_LINE: Option<&str> = Some("CodeEditor && single_line");

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
        /// Adds a cursor on the line above (Monaco `editor.action.insertCursorAbove`).
        AddCursorAbove,
        /// Adds a cursor on the line below (Monaco `editor.action.insertCursorBelow`).
        AddCursorBelow,
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
        /// Deletes the word under each cursor and the whitespace next to it
        /// (Monaco `deleteInsideWord`).
        DeleteInsideWord,
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
        /// Steps the number or value at the cursor up: `1` → `2`, `true` →
        /// `false` (Monaco `editor.action.inPlaceReplace.up`, Cmd/Ctrl+Shift+.).
        InPlaceReplaceUp,
        /// Steps the number or value at the cursor down (Monaco
        /// `editor.action.inPlaceReplace.down`, Cmd/Ctrl+Shift+,).
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
        /// Opens the platform's emoji and symbols picker.
        ShowCharacterPalette,
        /// Collapses the innermost expanded region at each cursor (Monaco `editor.fold`).
        Fold,
        /// Expands collapsed regions at each cursor (Monaco `editor.unfold`).
        Unfold,
        /// Collapses every region (Monaco `editor.foldAll`).
        FoldAll,
        /// Expands every region (Monaco `editor.unfoldAll`).
        UnfoldAll,
        /// Expands the collapsed region at the cursor, or collapses the innermost one
        /// (Monaco `editor.toggleFold`).
        ToggleFold,
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
        /// Selects the word at the cursor, or adds the next occurrence of the selection
        /// (Monaco `editor.action.addSelectionToNextFindMatch`).
        AddSelectionToNextFindMatch,
        /// Moves the last selection to the next occurrence of its text (Monaco
        /// `editor.action.moveSelectionToNextFindMatch`).
        MoveSelectionToNextFindMatch,
        /// Selects every occurrence of the selection or the word at the cursor (Monaco
        /// `editor.action.selectHighlights`).
        SelectAllOccurrences,
        /// Moves each cursor to the matching bracket (Monaco
        /// `editor.action.jumpToBracket`).
        JumpToBracket,
        /// Selects to the matching bracket, brackets included (Monaco
        /// `editor.action.selectToBracket`). Unbound by default.
        SelectToBracket,
        /// Moves to the next marker and shows it inline (Monaco
        /// `editor.action.marker.next`).
        GoToNextMarker,
        /// Moves to the previous marker and shows it inline (Monaco
        /// `editor.action.marker.prev`).
        GoToPreviousMarker,
        /// Goes to the definition of the symbol at the cursor (Monaco
        /// `editor.action.revealDefinition`).
        GoToDefinition,
        /// Requests parameter hints at the cursor (Monaco
        /// `editor.action.triggerParameterHints`).
        TriggerParameterHints,
        /// Requests completions at the cursor (Monaco `editor.action.triggerSuggest`).
        TriggerSuggest,
        /// Shows the hover for the cursor position (Monaco `editor.action.showHover`).
        ShowHover,
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
        /// Peek view list: the next row.
        PeekSelectNext,
        /// Peek view list: the previous row.
        PeekSelectPrevious,
        /// Peek view list: the first result.
        PeekSelectFirst,
        /// Peek view list: the last result.
        PeekSelectLast,
        /// Peek view list: go to the selected result.
        PeekOpen,
        /// Peek view list: collapse the selected file.
        PeekCollapse,
        /// Peek view list: expand the selected file.
        PeekExpand,
        /// Closes the peek view (Monaco `closeReferenceSearch`).
        ClosePeek,
        /// Formats the selections, or the cursor's line (Monaco
        /// `editor.action.formatSelection`).
        FormatSelection,
    ]
);

/// Monaco's default keybindings for the current platform.
pub fn default_key_bindings() -> Vec<KeyBinding> {
    let ctx = Some(KEY_CONTEXT);
    let mut bindings = vec![
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
        KeyBinding::new("secondary-shift-.", InPlaceReplaceUp, ctx),
        KeyBinding::new("secondary-shift-,", InPlaceReplaceDown, ctx),
        KeyBinding::new("secondary-alt-backspace", RemoveBrackets, ctx),
        KeyBinding::new("secondary-k secondary-x", TrimTrailingWhitespace, ctx),
        KeyBinding::new("insert", ToggleOvertypeInsertMode, ctx),
        KeyBinding::new("secondary-k secondary-b", SetSelectionAnchor, ctx),
        KeyBinding::new("secondary-k secondary-k", SelectFromAnchorToCursor, ctx),
        KeyBinding::new("shift-alt-i", InsertCursorAtEndOfEachLineSelected, ctx),
        KeyBinding::new("secondary-k secondary-c", AddCommentLine, ctx),
        KeyBinding::new("secondary-k secondary-u", RemoveCommentLine, ctx),
        KeyBinding::new("secondary-shift-l", SelectAllOccurrences, ctx),
        KeyBinding::new("secondary-f2", SelectAllOccurrences, ctx),
        KeyBinding::new("secondary-shift-\\", JumpToBracket, ctx),
        KeyBinding::new("f3", NextMatch, ctx),
        KeyBinding::new("f8", GoToNextMarker, ctx),
        KeyBinding::new("f12", GoToDefinition, ctx),
        KeyBinding::new("shift-f12", GoToReferences, ctx),
        KeyBinding::new("f4", GoToNextReference, ctx),
        KeyBinding::new("shift-f4", GoToPreviousReference, ctx),
        KeyBinding::new("secondary-shift-space", TriggerParameterHints, ctx),
        KeyBinding::new("ctrl-space", TriggerSuggest, ctx),
        KeyBinding::new("shift-f8", GoToPreviousMarker, ctx),
        KeyBinding::new("shift-f3", PreviousMatch, ctx),
        // Inside the find widget's inputs.
        KeyBinding::new("shift-enter", PreviousMatch, SINGLE_LINE),
        KeyBinding::new("alt-enter", SelectAllMatches, SINGLE_LINE),
        KeyBinding::new("secondary-enter", ReplaceAll, SINGLE_LINE),
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
            KeyBinding::new("cmd-alt-up", AddCursorAbove, ctx),
            KeyBinding::new("cmd-alt-down", AddCursorBelow, ctx),
            KeyBinding::new("alt-backspace", DeleteWordLeft, ctx),
            KeyBinding::new("alt-delete", DeleteWordRight, ctx),
            KeyBinding::new("cmd-backspace", DeleteAllLeft, ctx),
            KeyBinding::new("ctrl-h", DeleteLeft, ctx),
            KeyBinding::new("ctrl-d", DeleteRight, ctx),
            KeyBinding::new("cmd-shift-z", Redo, ctx),
            KeyBinding::new("ctrl-cmd-space", ShowCharacterPalette, ctx),
            KeyBinding::new("cmd-alt-[", Fold, ctx),
            KeyBinding::new("cmd-alt-]", Unfold, ctx),
            KeyBinding::new("cmd-k cmd-0", FoldAll, ctx),
            KeyBinding::new("cmd-k cmd-j", UnfoldAll, ctx),
            KeyBinding::new("cmd-k cmd-l", ToggleFold, ctx),
            KeyBinding::new("cmd-alt-f", StartFindReplace, ctx),
            KeyBinding::new("cmd-k cmd-d", MoveSelectionToNextFindMatch, ctx),
            KeyBinding::new("cmd-k cmd-i", ShowHover, ctx),
            KeyBinding::new("shift-alt-f", FormatDocument, ctx),
            KeyBinding::new("cmd-k cmd-f", FormatSelection, ctx),
            KeyBinding::new("cmd-f12", GoToImplementation, ctx),
            KeyBinding::new("cmd-shift-f12", PeekImplementation, ctx),
            KeyBinding::new("alt-f12", PeekDefinition, ctx),
            KeyBinding::new("cmd-g", NextMatch, ctx),
            KeyBinding::new("cmd-shift-g", PreviousMatch, ctx),
            KeyBinding::new("cmd-alt-enter", ReplaceAll, ctx),
            KeyBinding::new("cmd-shift-1", ReplaceOne, ctx),
            KeyBinding::new("cmd-alt-c", ToggleCaseSensitive, ctx),
            KeyBinding::new("cmd-alt-r", ToggleRegex, ctx),
            KeyBinding::new("cmd-alt-w", ToggleWholeWord, ctx),
            KeyBinding::new("cmd-alt-l", ToggleFindInSelection, ctx),
        ]);
    } else {
        bindings.extend([
            KeyBinding::new("ctrl-left", CursorWordLeft, ctx),
            KeyBinding::new("ctrl-shift-left", CursorWordLeftSelect, ctx),
            KeyBinding::new("ctrl-right", CursorWordRight, ctx),
            KeyBinding::new("ctrl-shift-right", CursorWordRightSelect, ctx),
            KeyBinding::new("alt-shift-left", CursorWordPartLeftSelect, ctx),
            KeyBinding::new("alt-shift-right", CursorWordPartRightSelect, ctx),
            KeyBinding::new("ctrl-home", CursorTop, ctx),
            KeyBinding::new("ctrl-shift-home", CursorTopSelect, ctx),
            KeyBinding::new("ctrl-end", CursorBottom, ctx),
            KeyBinding::new("ctrl-shift-end", CursorBottomSelect, ctx),
            KeyBinding::new("ctrl-backspace", DeleteWordLeft, ctx),
            KeyBinding::new("ctrl-delete", DeleteWordRight, ctx),
            KeyBinding::new("ctrl-y", Redo, ctx),
            KeyBinding::new("ctrl-shift-z", Redo, ctx),
            KeyBinding::new("shift-delete", Cut, ctx),
            KeyBinding::new("ctrl-insert", Copy, ctx),
            KeyBinding::new("shift-insert", Paste, ctx),
            KeyBinding::new("ctrl-shift-[", Fold, ctx),
            KeyBinding::new("ctrl-shift-]", Unfold, ctx),
            KeyBinding::new("ctrl-k ctrl-0", FoldAll, ctx),
            KeyBinding::new("ctrl-k ctrl-j", UnfoldAll, ctx),
            KeyBinding::new("ctrl-k ctrl-l", ToggleFold, ctx),
            KeyBinding::new("ctrl-h", StartFindReplace, ctx),
            KeyBinding::new("ctrl-k ctrl-d", MoveSelectionToNextFindMatch, ctx),
            KeyBinding::new("ctrl-k ctrl-i", ShowHover, ctx),
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
            bindings.extend([
                KeyBinding::new("ctrl-alt-up", AddCursorAbove, ctx),
                KeyBinding::new("ctrl-alt-down", AddCursorBelow, ctx),
                KeyBinding::new("shift-alt-up", CopyLinesUp, ctx),
                KeyBinding::new("shift-alt-down", CopyLinesDown, ctx),
            ]);
        } else {
            bindings.extend([
                KeyBinding::new("shift-alt-up", AddCursorAbove, ctx),
                KeyBinding::new("shift-alt-down", AddCursorBelow, ctx),
                KeyBinding::new("ctrl-shift-alt-up", CopyLinesUp, ctx),
                KeyBinding::new("ctrl-shift-alt-down", CopyLinesDown, ctx),
            ]);
        }
    }
    // The peek view's results list.
    let peek = Some(crate::peek::PEEK_KEY_CONTEXT);
    bindings.extend([
        KeyBinding::new("down", PeekSelectNext, peek),
        KeyBinding::new("up", PeekSelectPrevious, peek),
        KeyBinding::new("home", PeekSelectFirst, peek),
        KeyBinding::new("end", PeekSelectLast, peek),
        KeyBinding::new("enter", PeekOpen, peek),
        KeyBinding::new("left", PeekCollapse, peek),
        KeyBinding::new("right", PeekExpand, peek),
        KeyBinding::new("escape", ClosePeek, peek),
        KeyBinding::new("f4", GoToNextReference, peek),
        KeyBinding::new("shift-f4", GoToPreviousReference, peek),
    ]);
    bindings
}

/// Binds [`default_key_bindings`].
pub fn bind_default_keys(cx: &mut App) {
    cx.bind_keys(default_key_bindings());
}
