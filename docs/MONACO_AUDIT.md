# Monaco parity audit (2026-10-10)

> **Status (2026-10-11):** this is a dated snapshot. Every finding has since been filed and, except for the items blocked on GPUI, fixed:
> - the suggested issues #193–#206, plus #216 (inline edits);
> - the untracked leftovers #225–#230;
> - the P3 items #231–#235.
>
> #231 (`copyWithSyntaxHighlighting`), #232 (`letterSpacing`) and #233 (`fontVariations`) stay open, blocked until GPUI supports rich clipboard contents, letter spacing and font variation axes. See [`MONACO_PARITY.md`](MONACO_PARITY.md) for the current state.

This audit compares the vendored Monaco source (`vendor/vscode/src/vs/editor`) with the port in four parts:
- the `contrib/` features
- editor, diff and model options
- commands and default keybindings
- the public API (`monaco.editor`, `ICodeEditor`, `ITextModel` and `monaco.languages`)

Each finding cites Monaco and our code. The keybinding bugs, the default-behaviour gaps and the stale-doc claims below were checked by hand. Options that only make sense in a DOM, such as `automaticLayout` and `overflowWidgetsDomNode`, are left out.

**Overall:** the core editor is in good shape. These are complete:
- folding
- line operations
- multi-cursor
- word operations
- links
- linked editing
- semantic tokens
- smart select
- sticky scroll
- the color picker
- drag and drop, and paste
- the diff and multi-diff editors
- every `register*Provider` except one

The gaps cluster in four places:
- defaults that behave differently from Monaco
- Monaco command ids that `trigger`/`run_action` can't resolve
- hover, inlay hint and inline-completion polish
- global registries, events and geometry queries in the API

## P1: Bugs and default behaviour that differs from Monaco

| # | Gap | Evidence | Fix size |
|---|---|---|---|
| 1 | **In-place replace keys are swapped.** Monaco binds `inPlaceReplace.up` to Cmd+Shift+, and `.down` to Cmd+Shift+. — we bind them the other way round. | Monaco `contrib/inPlaceReplace/browser/inPlaceReplace.ts:137,161`; ours `actions.rs:780-781` | XS |
| 2 | **Next/previous problem uses the wrong key.** Monaco binds `marker.next`/`prev` to Alt+F8 / Shift+Alt+F8. Plain F8 belongs to `marker.nextInFiles`, which isn't implemented. | `contrib/gotoError/browser/gotoError.ts:202-271`; ours `actions.rs:796,911` | XS |
| 3 | **`matchBrackets: 'always'` doesn't highlight the enclosing brackets.** That's Monaco's default; we only highlight brackets next to the cursor. The `'near'` value and the overview-ruler mark are also missing. | `contrib/bracketMatching/browser/bracketMatching.ts:300-306,392-395`; ours `bracket_matching.rs:89-104`, `options.rs:1230` (a `bool`) | S |
| 4 | **`trimAutoWhitespace` is missing.** Monaco's default is true, so Monaco removes indentation it inserted automatically when a line stays empty; we never do. | Model options; our port skips Monaco's `removeAutoWhitespace` tests (`cursor/tests/cursor_editing_test.rs:10`) | M |
| 5 | **`detectIndentation` is missing.** Monaco's default is true; we never infer tab size or spaces vs tabs from the file. The six indentation commands are missing too: `indentUsingSpaces/Tabs`, `changeTabDisplaySize`, `detectIndentation`, `indentationToSpaces/Tabs`. | `contrib/indentation/browser/indentation.ts:646-651` | M |
| 6 | **About 120 Monaco command ids can't be resolved by `trigger`/`run_action`.** This covers every coreCommand, word operation, `undo`/`redo`/`selectAll`/clipboard, and the find, suggest, snippet, peek, paste and drop, sticky scroll and diff commands. About 15 labelled Monaco actions are also missing from the command palette. | `editor_actions.rs:159-301` (the `BUILTINS` table); `api.rs:268-281` | M (mostly a table) |
| 7 | **Stale docs.** `book/src/differences.md:53,56` says code actions, rename, formatting, inlay hints, color decorators, links and the diff editor are missing; all of them exist. `MONACO_PARITY.md` overstates hover, indentation, bracket matching and inline completions (see P2). Two doc comments are also stale: `TabCompletion::OnlySnippets` says it acts as Off "until snippets land", and `WordBasedSuggestions::OffWithInlineSuggestions` says there are no inline suggestions. | as cited | S |
| 8 | **Built but not exposed:** `useTabStops` and `trimWhitespaceOnDelete` already exist in `CursorConfig` but aren't `EditorOptions`. `minimap.sectionHeaderLetterSpacing` is declared but never applied. | `cursor/mod.rs:186-189`, `editor.rs:818-838`, `options.rs:93` | XS |

## P2: Missing features

**Hover**
- options: `enabled`, `delay`, `sticky`, `hidingDelay`, `above`
- resizing
- glyph-margin and line-number hovers
- keyboard focus with the scroll, page and top/bottom commands, and `hideHover`
- verbosity +/−
- the copy button
- the definition-preview hover

The delay is hard-coded at 300ms (`hover.rs:21`) and the size is fixed (`hover.rs:127`).

**Parameter hints**
- `parameterHints.enabled` and `cycle`
- the `closeParameterHints` and `showNext/PrevParameterHint` commands, and Alt+Up/Down
- clickable overload arrows

**Inlay hints**
- clickable label parts with a location or command
- tooltips (stored but never shown)
- double-click to apply text edits
- `resolveInlayHint`
- `inlayHints.fontSize` and `fontFamily`

**Inline completions**
- the hover toolbar (`showToolbar`)
- the commands `showNext/Previous`, `acceptNextWord/Line`, `commit` and `hide`, which have no Monaco ids yet
- `syntaxHighlightingEnabled` and `fontFamily`
- inline edits (next-edit suggestions) and their views; this is the largest single item
- provider lifecycle hooks (`handleRejection`, `handleEndOfLifetime`, `groupId`)

**Code actions and rename**
- the `editor.action.codeAction` command with its arguments
- refactor preview and "Show Disabled"
- `registerNewSymbolNameProvider` with rename suggestions, and `acceptRenameInputWithPreview`
- `CodeActionProviderMetadata.documentation`

**Suggest**
- `selectionMode`, `snippetsPreventQuickSuggestions`, `shareSuggestSelections` and `fitWidthToDetails`
- `quickSuggestions: 'inline'` and `'offWhenInlineCompletions'`
- the commands `focusSuggestion`, `toggleSuggestionFocus`, `toggleExplainMode`, `resetSuggestSize` and `suggestWidgetCopy`
- `CompletionItem.action`
- the missing kinds: Customcolor, User, Issue and Tool
- keys: `triggerSuggest` also on Alt+Esc and Cmd+I; Ctrl+N/P in the list. We bind Cmd+Up/Down to first/last suggestion, but Monaco binds them to next/previous.

**Find**
- multi-line search and replace input (Ctrl+Enter inserts a newline)
- `find.findOnType`
- `editor.actions.findWithArgs` and `editor.action.goToMatchFindAction`
- `globalFindClipboard` (the macOS find pasteboard)

**Go to and peek**
- `revealDefinitionAside`
- the `goToLocations`/`peekLocations` API commands
- `togglePeekWidgetFocus`
- a resizable peek view with a sash between preview and list
- `definitionLinkOpensInPeek` and `peekWidgetDefaultFocus`
- F12 runs `gotoNextSymbolFromResult` while symbol results are active
- `gotoLocation.multipleTests`

**Smaller items**
- `editor.action.insertFinalNewLine`
- the long-lines helper: clicking past `stopRenderingLineAfter` turns the limit off
- the unicode highlighter's "too many characters" banner, and its `disableHighlighting*` commands
- themeable symbol-icon colours (`symbolIcon.*Foreground`)
- `codelens.showLensesInCurrentLine`
- `editor.action.forceRetokenize`, `inspectTokens` and `toggleHighContrast`
- `gotoOffset`
- a `prepareDocumentPaste` hook at copy time
- `editor.cancelOperation`: Escape cancels pending requests

**Editor options that are missing or only partly supported**
- `selectOnLineNumbers`
- `wordWrapBreakBefore/AfterCharacters` (hard-coded), `wordWrapOverride1/2`, `wordWrapIndicator`, `wrapOnEscapedLineFeeds` and `wordBreak: keepAll`
- `cursorStyle`: the `block-outline` and `underline-thin` values
- `lineNumbers` as a function
- `occurrencesHighlight: 'multiFile'` and `occurrencesHighlightFromAllProviders`
- `selectedTextMatchMode`
- `mouseStyle` and `mouseMiddleClickAction`
- `minimap.autohide`
- `guides.highlightActiveIndentation: 'always'`
- `bracketPairColorization.independentColorPoolPerBracketType`
- `scrollbar.ignoreHorizontalScrollbarInContentHeight`
- `inertialScroll`
- `fullwidthCharacterWidth`
- `accessibilitySupport` and `screenReaderAnnounceInlineSuggestion`
- `largeFileOptimizations` and `defaultEOL` (model options)

**Diff editor options**
- `maxFileSize` (50MB): there's no cut-off before diffing
- `hideOriginalLineNumbers`, `compactMode` and `diffWordWrap`
- `diffAlgorithm: 'legacy'`
- `diffCodeLens`
- `experimental.useTrueInlineView` and `showEmptyDecorations`
- `accessibilityVerbose`
- commands for `toggleCollapseUnchangedRegions`, `toggleShowMovedCodeBlocks`, `toggleUseInlineViewWhenSpaceIsLimited` and `switchSide`
- dispatch by Monaco id: `diffEditor.revert` and the other `diffEditor.*` ids
- `saveViewState`/`restoreViewState`, an `onDidChangeModel` event and `revealFirstDiff`

**Keybindings**
- macOS Emacs-style keys: Ctrl+B/F/P/N for cursor moves, Ctrl+Shift+A/E for line start/end with selection, Ctrl+Backspace/Delete
- `insertCursorAbove/Below` on Linux with Ctrl+Shift+Up/Down
- peek keys: F12 and Shift+F12 for next/previous reference, Cmd+Down to reveal
- the hover-focus keys
- Shift+Delete and Ctrl+Insert for cut and copy are Windows-only in Monaco, but we also bind them on Linux
- Alt+F5 for the diff editor is a VS Code workbench key, not a standalone Monaco default

## P2: Public API gaps

**Global registries and events**
- `getEditors`, `getDiffEditors`, `onDidCreateEditor`
- `getModels` (crate-private today), `onDidCreateModel`, `onWillDisposeModel`
- a global `onDidChangeModelLanguage` that carries the old language
- markers keyed by URI across models: `getModelMarkers` filters, `removeAllMarkers` and a global `onDidChangeMarkers`. `Marker` has no owner or resource field.
- themes registered by name with a global `setTheme`, including for the diff editors
- global `addCommand`/`addEditorAction`
- removable keybindings: `remove_action` leaves the binding in place
- `colorizeModelLine`

**Editor events**
- `onDidChangeModel`/`onWillChangeModel`
- `onDidChangeConfiguration`
- `onDidLayoutChange`/`onDidContentSizeChange`
- `onDidPaste`/`onWillPaste`/`onWillCopy`/`onWillCut`
- `onWillType`/`onDidType`
- key, wheel and drag-and-drop mouse events
- widget focus: `onDidFocusEditorWidget` and `hasWidgetFocus`
- composition
- hidden-area and view-zone changes

**Geometry**
- `getLayoutInfo`
- `getTopForLineNumber`/`getBottomForLineNumber`/`getTopForPosition`
- `getOffsetForColumn`, `getWidthOfLine`
- `getVisibleColumnFromPosition`
- `getContentWidth`, `getScrollWidth`/`Height`
- a `ScrollType` choice between smooth and immediate for `setScrollPosition` and the reveal methods
- `revealAllCursors`
- `saveViewState` that keeps folding and find state and anchors on the first visible line

**Model**
- missing: `getLinesContent`, `getValueLength`, `modifyPosition`, `getFullModelRange`, `findNextMatch`/`findPreviousMatch`, `getAlternativeVersionId`, `popStackElement`, attached-editor tracking, large-file flags, `onDidChangeLanguageConfiguration`
- partial:
  - `applyEdits` doesn't return the undo edits
  - `pushEditOperations` takes no cursor computer
  - `findMatches` has no scope or captures
  - `pushEOL` isn't undoable
- crate-private or free functions only: indentation helpers, bracket pairs (plus a missing `findEnclosingBrackets`), indent and bracket guides, and per-line tokens

**Languages**
- `register` lacks `mimetypes`, `firstLine` and `filenamePatterns`
- `onLanguage`
- a runtime `setLanguageConfiguration` that updates open models
- `LanguageSelector` with scheme, pattern and `score`
- hover verbosity in providers
- `SignatureHelpContext.isRetrigger`/`activeSignatureHelp`
- `FoldingContext` and change events for folding and code lens providers
- multi-range formatting and choosing between formatters

## P3: Not possible or very low value in GPUI

- `copyWithSyntaxHighlighting`: GPUI's clipboard has no HTML or RTF
- `letterSpacing`, `fontVariations` and `allowVariableFonts`: GPUI text has no such controls
- `wordSegmenterLocales`: needs Intl.Segmenter or an ICU segmenter
- `remeasureFonts`, `getEncodedLanguageId`, `setColorMap` and the Monarch tokenizer (tree-sitter replaces it by design)

## Suggested issues

1. Keybinding fixes and macOS Emacs keys (P1 #1, #2 and the keybinding list).
2. Resolve every Monaco command id in `trigger`/`run_action` and the palette (P1 #6).
3. Bracket matching `'always'`/`'near'` and the overview-ruler mark (P1 #3).
4. `trimAutoWhitespace`, plus porting Monaco's skipped tests (P1 #4).
5. Indentation detection and the indentation commands (P1 #5).
6. Fix stale docs and expose `useTabStops`/`trimWhitespaceOnDelete` (P1 #7, #8).
7. Hover options, resizing, keyboard focus and glyph hovers.
8. Parameter hint options and commands; interactive inlay hints.
9. Inline completions: toolbar, commands and options (inline edits as a separate, larger issue).
10. Suggest, find and peek extras.
11. Missing editor and diff options.
12. API: global registries and events.
13. API: editor events and geometry queries, and `ScrollType`.
14. API: model helpers and language extras.
