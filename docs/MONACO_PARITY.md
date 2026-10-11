# Monaco parity audit

This is how gpui-monaco 0.1 compares with Monaco, feature by feature.

The audit compares three things:

- the `src/vs/editor/contrib/*` directories
- the commands registered in `browser/coreCommands.ts`
- the `EditorOption` enum in `common/config/editorOptions.ts`

These were checked against our actions, options and modules. The Monaco source can be fetched with `script/fetch-monaco-source`.

[`MONACO_AUDIT.md`](MONACO_AUDIT.md) is a finer-grained audit (2026-10) of sub-features, every option and command id, and the public API, with gaps tracked as #193–#206.

> **Second audit (2026-10-11):** [`MONACO_AUDIT_2.md`](MONACO_AUDIT_2.md) checks widget fidelity, core view and input, the diff editors, commands, options, theme and API in more depth. Several rows below marked ✅ have open fidelity issues there (tracked as #245–#265). Until those are fixed, read ✅ as "the feature exists", not "pixel- and behavior-identical".

**Legend:**

- ✅ done
- 🟡 partial; the issue covers the rest
- ❌ missing

Issues are linked by number.

## Editing

| Feature | Monaco | Status | Issue |
|---|---|---|---|
| Typing, delete, tab, outdent, newline | `coreCommands` | ✅ | |
| Cursor movement: char, word, word part, line, page, document | `coreCommands`, `wordOperations`, `wordPartOperations` | ✅ | #81 |
| Word segmentation for languages without spaces (Japanese, Chinese, Thai) in word movement, deletion and selection | `wordSegmenterLocales` | ✅ ICU4X's segmenter instead of `Intl.Segmenter`, with the first valid locale only; needs the `word-segmenter` cargo feature (on by default) | #234 |
| Multi-cursor: add above/below, Cmd+D and previous-match variants, select all occurrences, cursors at line ends/top/bottom, Alt+click, box select by mouse and keyboard, options | `multicursor` | ✅ | #81 |
| Cursor undo, selection anchor, overtype mode | `cursorUndo`, `anchorSelect` | ✅ | #81 |
| Scroll commands (Ctrl+Up/Down, …) | `coreCommands` | ✅ | #81 |
| Clipboard with multi-cursor distribution | `clipboard` | ✅ | |
| Paste as, Linux primary selection, reindent on paste | `clipboard`, `dropOrPasteInto`, `autoIndentOnPaste` | ✅ | #107 |
| Copy with syntax highlighting | `copyWithSyntaxHighlighting` | ❌ (GPUI's clipboard has no HTML) | #107 |
| Undo/redo with grouping, cursor restore | model | ✅ | |
| Auto-closing, overtype, auto-surround | `cursor` | ✅ | |
| Auto-indent (`onEnterRules`, `indentationRules`, electric characters) | `indentation` | ✅ | |
| Reindent lines | `indentation` | ✅ | #80 |
| Indentation detection and conversion commands | `indentation` | ✅ | #197 |
| Line operations: move, copy, delete, insert, indent | `linesOperations` | ✅ | |
| Sort, join, reverse, transpose, case transforms, delete duplicates, trim whitespace | `linesOperations`, `caretOperations` | ✅ | #80 |
| Insert final new line | `insertFinalNewLine` | ✅ | #202 |
| In-place replace | `inPlaceReplace` | ✅ | #80 |
| Toggle line and block comments | `comment` | ✅ | #79 |
| Smart select (expand and shrink) | `smartSelect` | ✅ | #82 |
| Linked editing (HTML tags) | `linkedEditing` | ✅ | #101 |
| Snippet syntax: parser, variables, transforms | `snippet` | ✅ | #55 |
| Snippet sessions: tab stops, choices, nesting, completion integration | `snippet` | ✅ | #55 |
| Drag and drop text, double-click block selection | `dnd`, `doubleClickSelectsBlock` | ✅ | #92 |
| External drops, paste as | `dropOrPasteInto` | ✅ | #92 |
| Copy-time paste data (`prepareDocumentPaste`, `copyMimeTypes`) | `dropOrPasteInto` | ✅ | #227 |
| Escape cancels pending go to, format, rename, paste and drop requests | `editor.cancelOperation` | ✅ | #227 |
| Find/replace: case, word, regex, in selection, replace all | `find` | ✅ | |
| Find model (Monaco's `FindModel`), preserve case, case escapes in replace, loop, seed and auto find in selection options | `find` | ✅ | #104 |
| Find controller commands, history, extra space on top, widget width and states, remembered toggles | `find` | ✅ | #104 |
| Find with arguments, go to match, `findOnType` | `find` | ✅ | #202 |
| Multi-line search and replace inputs (Ctrl+Enter, growing inputs), global find clipboard | `findWidget`, `find.globalFindClipboard` | ✅ the find pasteboard on macOS, an app-wide find string elsewhere | #226 |
| Bracket matching and jump | `bracketMatching` | ✅ `always`/`near`/`never`, overview ruler mark | #195 |

## View

| Feature | Monaco | Status | Issue |
|---|---|---|---|
| Gutter: line numbers (on, off, relative, interval, custom function), glyph margin, line decorations, `selectOnLineNumbers` | view parts | ✅ | #203 |
| Cursor styles (line, block, underline, thin variants, block outline), `mouseStyle`, `mouseMiddleClickAction` | options | ✅ | #203 |
| Current-line highlight, scrollbars, overview ruler | view parts | ✅ | |
| Scrollbar and overview ruler options | `scrollbar`, `overviewRuler*` | ✅ | #88 |
| Word wrap: off, on, column, bounded; wrapping indent | `wordWrap` | ✅ | |
| Wrap for proportional fonts (`wrappingStrategy: advanced`), configurable break characters | `wordWrap*` | ✅ | #87 |
| Wrap overrides, `wordBreak: keepAll`, `wrapOnEscapedLineFeeds` | `wordWrapOverride1/2`, `wordBreak`, `wrapOnEscapedLineFeeds` | ✅ | #203 |
| Word wrap indicator | `wordWrapIndicator` | ✅ | #228 |
| Horizontal scrollbar left out of the content height | `scrollbar.ignoreHorizontalScrollbarInContentHeight` | ✅ | #228 |
| Large file optimizations, default line ending | `largeFileOptimizations`, `defaultEOL` | ✅ | #203 |
| Option validation: clamps and sentinel values (`fontSize` 0, `lineHeight` 0 and multipliers, sensitivities, minimap, sticky scroll, folding and color limits, delays, …), `tabSize`/`indentSize` at least 1, `indentSize: 'tabSize'` | each option's `validate`, `TextModelResolvedOptions` | ✅ the editor renders with `EditorOptions::validated`, `options()` returns the options as set | #262 |
| Layout widths: a line-height glyph margin, 16px folding column (none with `showFoldingControls: never`), the minimap's proportional share and `fill`/`fit` sizing, `lineDecorationsWidth` in characters | `EditorLayoutInfoComputer.computeLayout` | ✅ one glyph margin lane (no lane decorations); the widest digit is the typical character width | #262 |
| Platform defaults, `cursorWidth` 0 and its cap | `renderFinalNewline` (dimmed on Linux), `defaultEOL` (CRLF on Windows), `cursorWidth` | ✅ | #262 |
| Inertial scrolling, two-cell full-width characters | `inertialScroll`, `fullwidthCharacterWidth` | ✅ ghost text and the inline diff's deleted lines keep the font's width | #203, #228 |
| Folding: indentation and markers, gutter chevrons | `folding` | ✅ | |
| Syntax and provider folding (`foldingStrategy`, `foldingMaximumRegions`) | `folding` | ✅ | #102 |
| Fold level and recursive commands, fold navigation, manual ranges, unfold on `⋯` click, `foldingHighlight`, `foldingImportsByDefault` | `folding` | ✅ | #102 |
| Minimap | `minimap` | ✅ | |
| Minimap options (side, render characters, size, autohide), section headers | `minimap`, `sectionHeaders` | ✅ | #88, #203 |
| Sticky scroll (outline, folding and indentation models) | `stickyScroll` | ✅ | |
| Sticky scroll keyboard focus, toggle, folding controls, hover, scroll with editor | `stickyScroll` | ✅ | #106 |
| Render whitespace, indent guides, rulers, control characters | view parts | ✅ | #84 |
| Bracket pair guides, `guides.highlightActiveIndentation: 'always'` | view parts | ✅ | #118, #203 |
| Bracket pair colorization, independent color pool per bracket type | `bracketPairColorization` | ✅ | #85, #203 |
| Occurrence and selection highlight, `selectedTextMatchMode`, `occurrencesHighlightFromAllProviders` | `wordHighlighter` | ✅ `multiFile` highlights in the current editor only | #83, #203, #228 |
| Unicode highlighting, unusual line terminators | `unicodeHighlighter`, `unusualLineTerminators` | ✅ | #86 |
| Unicode highlighter "too many characters" banner, `disableHighlightingOf*` commands | `unicodeHighlighter`, `setBanner` | ✅ the commands change the editor's options, not user settings | #227 |
| Font weight, ligatures, zoom, mouse wheel zoom | `fontZoom`, options | ✅ | #87 |
| Variable line heights (`lineHeight` decorations, `allowVariableLineHeights`) | decorations | ✅ | #87 |
| Letter spacing, font variations, per-decoration fonts | options | ❌ | #87 |
| Smooth scrolling, caret animation, blinking styles, cursor surrounding lines, padding, wheel and middle-click options | options | ✅ | #88 |
| Placeholder text, read-only message | `placeholderText`, `readOnlyMessage`, `message` | ✅ | #89 |
| Very long lines | `stopRenderingLineAfter`, `maxTokenizationLineLength` | ✅ | #59 |
| Clicking past the rendering limit turns it off | `longLinesHelper` | ✅ | #227 |

## Extensibility

| Feature | Monaco | Status | Issue |
|---|---|---|---|
| Decorations: background, underline, glyph, gutter bar, before/after text, overview ruler, hover message, stickiness, owner | `deltaDecorations` | ✅ | |
| View zones, content widgets, overlay widgets, `allowEditorOverflow`; widgets keep their wheel, clicks and focus, clipped to the text area | widget API | ✅ overflowing widgets are GPUI deferred draws above the whole window | #245 |
| Custom commands and keybindings | `addAction`, `addCommand` | ✅ (GPUI actions) | |
| Global commands, actions and removable keybinding rules | `editor.addCommand`, `addEditorAction`, `addKeybindingRule(s)` | ✅ (`Disposable`; GPUI keystrokes and key contexts) | #204 |
| Editor, diff editor and model registries and events | `getEditors`, `getDiffEditors`, `onDidCreateEditor`, `onDidCreateDiffEditor`, `getModels`, `onDidCreateModel`, `onWillDisposeModel`, `onDidChangeModelLanguage` | ✅ widget inputs and placeholder models are left out (Monaco `isSimpleWidget`) | #204, #245 |
| Markers across models | `getModelMarkers`, `removeAllMarkers`, `onDidChangeMarkers` | ✅ markers live in their model, so none for uris without one | #204 |
| Context menu, with host items | `contextmenu` | ✅ | #91 |
| Go to line, quick outline (quick input) | `quickAccess` | ✅ | #100 |
| Go to offset (`::`, `editor.action.gotoOffset`) | `quickAccess` | ✅ | #100, #227 |
| Command palette, `addAction` | `quickAccess` | ✅ | #100 |
| Reveal APIs, `executeEdits`, mouse hit-test events, word and position helpers, content-change details, cursor change reasons, decorations collections | editor API | ✅ | #105 |
| Editor events: model, language, configuration, layout and content size changes, typing, clipboard, key, wheel and drag events, widget focus, IME composition, hidden areas, view zones, begin/end update | `ICodeEditor.on*` | ✅ hidden-area changes are reported when the next frame applies them; listeners can't change clipboard data or cancel keys | #205, #229 |
| Geometry and scrolling: `getLayoutInfo`, line and column offsets, `getWidthOfLine`, content and scroll sizes, `ScrollType`, `revealAllCursors`, view state with folding and the first visible line, `getStatusbarColumn` | `ICodeEditor` | ✅ answered from the last frame (`None` before it) | #205, #229 |
| Model helpers: lines, `modifyPosition`, find next/previous/scoped with captures, `applyEdits` undo edits, `pushEditOperations` with a cursor computer, `popStackElement`, alternative version id, bracket pairs and guides | `ITextModel` | ✅ | #206 |
| Model extras: `isAttachedToEditor`/`onDidChangeAttached`, undoable `pushEOL`, per-line tokens (`tokenization.getLineTokens`), `normalizeIndentation`, first/last non-whitespace and indent columns, indent levels | `ITextModel` | ✅ tokens are tree-sitter captures mapped to Monaco token types (Monarch tokens as they are), not encoded metadata | #229, #235 |
| Language extras: `register` file associations (patterns, first line, mimetypes), merged on repeat registration, `onLanguage` and `onLanguageEncountered` (cancellable), runtime `setLanguageConfiguration` (layered, disposable), `LanguageFilter` scheme, pattern and `exclusive`, `score`, `SignatureHelpContext`, folding and code lens `onDidChange`, multi-range formatting, `FoldingContext` | `monaco.languages` | ✅ `hasAccessToAllModels` and `notebookType` don't apply | #206, #229, #265 |
| Provider ranking: score, then newest first; completion groups stopping at the first with items; trigger characters and incomplete lists asking only their providers | `LanguageFeatureRegistry.ordered`, `orderedGroups`, `provideSuggestionItems`, `SuggestModel` | ✅ | #265 |
| `LocationLink` origin and target selection ranges; `WorkspaceEdit` file operations, snippet edits, versions and `needsConfirmation` | `languages.ts`, `IBulkEditService` | ✅ file operations go to a host handler (standalone Monaco rejects them), before the text edits | #265 |
| Custom highlighter | tokenization support | ✅ (`Language::with_highlighter`) | |
| Monarch tokenizer | `setMonarchTokensProvider`, `monarchCompile`, `monarchLexer` | ✅ JSON grammars, also before the language is registered; JavaScript regexes run by `fancy-regex` (Unicode `\b`, code points rather than UTF-16 units); only Monarch languages can be embedded | #235, #265 |
| Line-state tokens providers | `setTokensProvider`, `registerTokensProviderFactory` | ✅ (`syntax::TokensProvider`, byte offsets rather than UTF-16 `startIndex`); no `EncodedTokensProvider` | #265 |
| Token-level APIs | `remeasureFonts`, `setColorMap`, `getEncodedLanguageId` | ✅ `remeasure_fonts` is a no-op (GPUI measures every frame); no `setColorMap` or `getEncodedLanguageId`, which serve encoded token metadata | #235 |
| Static colorize and tokenize | `colorizer` | ✅ | #108 |
| Colorize a model line | `colorizeModelLine` | ✅ | #204 |
| Themes: `vs`, `vs-dark`, VS Code JSON, `defineTheme` | standalone theme service | ✅ | #263 |
| Theme colors and their defaults | color registry (`registerColor`), `StandaloneTheme.getColor` | ✅ the editor's and platform registry colors with Monaco's per-kind defaults, derived lazily from the theme's own colors; widgets use their own keys (`menu.*`, `editorHoverWidget.*`, `list.*`, `quickInput*`, `keybindingLabel.*`, `toolbar.*`, markers); high contrast outlines; workbench-only colors left out | #263 |
| Token rules | `TokenTheme`, `ThemeTrieElement` | ✅ rule inheritance, `''` default rules and the built-in themes' language rules; tree-sitter captures get their language suffix; the default color is the editor foreground rather than `#000000`, and token backgrounds are ignored | #263 |
| Themes by name for every editor | `editor.defineTheme`, `editor.setTheme` | ✅ | #204 |
| High-contrast themes (`hc-black`, `hc-light`) | standalone themes | ✅ | #53 |
| Toggle high contrast, force retokenize, inspect tokens | `toggleHighContrast`, `tokenization`, `inspectTokens` | ✅ inspect tokens shows tree-sitter captures and their TextMate scopes | #227 |
| Symbol icon colors | `symbolIcon.*Foreground` | ✅ suggest widget and quick outline (glyphs, not codicons) | #227 |

## Language features (providers)

| Provider | Monaco | Status | Issue |
|---|---|---|---|
| Completion: fuzzy filter, docs, commit characters, additional edits | `suggest` | ✅ | |
| Completion: resolve, tags, insert/replace modes, word suggestions, accept and selection options | `suggest` | ✅ | #103 |
| Completion: tab completion, locality bonus, retrigger rules | `suggest` | ✅ | #103 |
| Completion widget: details toggle, side docs, resizing, status bar, loading and empty messages | `suggest` | ✅ | #103 |
| Completion: `selectionMode` and `focusSuggestion`, explain mode, reset size, `snippetsPreventQuickSuggestions`, every item kind | `suggest` | 🟡 no `shareSuggestSelections` | #202 |
| Completion: `quickSuggestions: 'inline'` (completion items as ghost text) and `'offWhenInlineCompletions'`, details focus (`toggleSuggestionFocus`), `suggestWidgetCopy` | `suggest`, `suggestInlineCompletions` | ✅ copying takes the whole details panel (there's no text selection in it); inline items skip word-based suggestions and eager resolving | #226 |
| Hover | `hover` | ✅ content and gutter hovers, options, keyboard focus and scrolling, resizing, verbosity, copy button, long line warnings | #199 |
| Signature help: overload cycling and arrows, `enabled`/`cycle` options, close and next/previous commands | `parameterHints` | ✅ | #200 |
| Go to definition (link, F12) | `gotoSymbol` | ✅ | |
| Definition preview hover (Cmd/Ctrl+hover, `showDefinitionPreviewHover`) | `gotoSymbol` | ✅ | #225 |
| References, declaration, type definition, implementation, peek view | `gotoSymbol`, `peekView` | ✅ | #95 |
| Open definition to the side (host event), peek focus toggle and default, resizable peek view, `definitionLinkOpensInPeek` | `gotoSymbol`, `peekView` | ✅ | #202 |
| `goToLocations`/`peekLocations` API commands (and `showReferences`), `gotoLocation.multipleTests` | `gotoSymbol` | ✅ | #227 |
| Diagnostics: squiggles, ruler, hover, F8 | `gotoError` | ✅ | |
| Diagnostic tags (unused, deprecated), related information | `gotoError` | ✅ | #90 |
| Document highlights | `wordHighlighter` | ✅ 50ms after the cursor rests, then `occurrencesHighlightDelay` (0) from the move | #83, #262 |
| Formatting: document, range, on type, on paste | `format` | ✅ | #94 |
| Choosing between formatters (`FormattingConflicts.setFormatterSelector`, `displayName`) | `format` | ✅ the first by default, as standalone Monaco; no built-in picker (`formatDocument.multiple` is VS Code workbench) | #229 |
| Rename | `rename` | ✅ | #56 |
| Rename preview (`acceptRenameInputWithPreview`, through a host preview handler) | `rename` | ✅ | #225 |
| Code actions and lightbulb (no AI actions or nearby quick fixes) | `codeAction` | ✅ | #56 |
| Code action menu: "Show Disabled", documentation links, preview (Cmd/Ctrl+Enter, through a host preview handler; no Refactor Preview panel) | `codeAction` | ✅ | #225 |
| The `editor.action.codeAction` command with `kind`, `apply` and `preferred` | `codeAction` | ✅ | #202 |
| Rename suggestions (`NewSymbolNamesProvider`) | `rename` | ✅ | #225 |
| Inlay hints: label parts, tooltips, Cmd/Ctrl+click, double-click edits, resolve, font options (no part context menu or location hovers) | `inlayHints` | ✅ | #56, #200 |
| Code lens | `codelens` | ✅ (and "Show CodeLens Commands for Current Line", #202) | #96 |
| Color decorators (providers, default finder) | `colorDecorators` | ✅ | #97 |
| Color picker (hover and standalone) | `colorPicker` | ✅ | #97 |
| Links | `links` | ✅ | #93 |
| Semantic tokens | `semanticTokens` | ✅ | #98 |
| Inline completions (ghost text, inline edits) | `inlineCompletions` | 🟡 ghost text, toolbar (hover or always), commands, options, lifetime hooks and provider groups; inline edits with Monaco's word, line, insertion, deletion, side-by-side, collapsed, custom (`hint`) and jump-to views, the gutter indicator (line number, pulse) and menu, the long-distance hint, cross-file edits (through `OpenLocation`), `jumpToPosition` items, alternative actions (Shift+Tab; supplied by the provider), `showInlineEditMenu`, undo/redo special cases and the fade-out; no snooze, model picker or onboarding | #99, #201, #216, #230 |
| Document symbols | `documentSymbols` | ✅ | #100 |
| Selection ranges | `smartSelect` | ✅ | #82 |
| Folding ranges | `folding` | ✅ | #102 |
| Linked editing ranges | `linkedEditing` | ✅ | #101 |
| LSP bridge (`gpui-monaco-lsp`: sync, diagnostics, completion, hover, definition, highlights, signature help) | `monaco-languageclient` | ✅ | #54 |

## Other

| Feature | Status | Issue |
|---|---|---|
| Diff editor, multi-diff editor | ✅ `DiffEditor` (side by side or inline, hidden unchanged regions, revert buttons, moved blocks, the accessible diff viewer, legacy and advanced algorithms, `compactMode`, `diffWordWrap`, `maxFileSize`, `diffCodeLens`, `accessibilityVerbose`, `experimental.showEmptyDecorations` and `useTrueInlineView`, switch side, view state, `revealFirstDiff`) and `MultiDiffEditor`; smaller gaps in #181 | #52, #181, #203, #228 |
| Screen readers, tab focus mode, ARIA options, `accessibilitySupport`, `screenReaderAnnounceInlineSuggestion` | ✅ announcements are AccessKit live regions; no accessibility signals (sounds) for inline suggestions | #53, #228 |
| Standalone options: `stablePeek`, `autoDetectHighContrast`, `accessibilityHelpUrl`, `tabIndex` | ✅ GPUI doesn't report the OS's high contrast mode, so hosts report it (`set_os_high_contrast`); `accessibilityHelpUrl` is stored only, as there's no accessibility help yet | #262 |
| wasm / web | 🟡 compiles for `wasm32-unknown-unknown`; browser runner pending | #58 |
| crates.io release | ❌ | #57 |
| Monaco migration notes | ✅ (book: Migrating from Monaco) | #60 |
| Small correctness follow-ups | — | #76 |

## Not planned

These Monaco internals have no equivalent here, by design:

- `gpu` renderer and the DOM `editContext`: GPUI renders on the GPU natively.
- `useShadowDOM`, `fixedOverflowWidgets`, `extraEditorClassName` and `automaticLayout`: DOM-specific.
- `inlineProgress` and `floatingMenu`: VS Code workbench UI.
