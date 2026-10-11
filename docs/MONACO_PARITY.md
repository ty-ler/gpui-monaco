# Monaco parity audit

This is how gpui-monaco 0.1 compares with Monaco, feature by feature.

The audit compares three things:

- the `src/vs/editor/contrib/*` directories
- the commands registered in `browser/coreCommands.ts`
- the `EditorOption` enum in `common/config/editorOptions.ts`

These were checked against our actions, options and modules. The Monaco source can be fetched with `script/fetch-monaco-source`.

[`MONACO_AUDIT.md`](MONACO_AUDIT.md) is a finer-grained audit (2026-10) of sub-features, every option and command id, and the public API, with gaps tracked as #193–#206.

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
| Find/replace: case, word, regex, in selection, replace all | `find` | ✅ | |
| Find model (Monaco's `FindModel`), preserve case, case escapes in replace, loop, seed and auto find in selection options | `find` | ✅ | #104 |
| Find controller commands, history, extra space on top, widget width and states, remembered toggles | `find` | ✅ | #104 |
| Find with arguments, go to match, `findOnType` | `find` | 🟡 no multi-line search inputs | #202 |
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
| Large file optimizations, default line ending | `largeFileOptimizations`, `defaultEOL` | ✅ | #203 |
| Inertial scrolling, two-cell full-width characters | `inertialScroll`, `fullwidthCharacterWidth` | ❌ | #203 |
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
| Occurrence and selection highlight, `selectedTextMatchMode` | `wordHighlighter` | ✅ `multiFile` highlights in the current editor only | #83, #203 |
| Unicode highlighting, unusual line terminators | `unicodeHighlighter`, `unusualLineTerminators` | ✅ | #86 |
| Font weight, ligatures, zoom, mouse wheel zoom | `fontZoom`, options | ✅ | #87 |
| Variable line heights (`lineHeight` decorations, `allowVariableLineHeights`) | decorations | ✅ | #87 |
| Letter spacing, font variations, per-decoration fonts | options | ❌ | #87 |
| Smooth scrolling, caret animation, blinking styles, cursor surrounding lines, padding, wheel and middle-click options | options | ✅ | #88 |
| Placeholder text, read-only message | `placeholderText`, `readOnlyMessage`, `message` | ✅ | #89 |
| Very long lines | `stopRenderingLineAfter`, `maxTokenizationLineLength` | ✅ | #59 |

## Extensibility

| Feature | Monaco | Status | Issue |
|---|---|---|---|
| Decorations: background, underline, glyph, gutter bar, before/after text, overview ruler, hover message, stickiness, owner | `deltaDecorations` | ✅ | |
| View zones, content widgets, overlay widgets | widget API | ✅ | |
| Custom commands and keybindings | `addAction`, `addCommand` | ✅ (GPUI actions) | |
| Global commands, actions and removable keybinding rules | `editor.addCommand`, `addEditorAction`, `addKeybindingRule(s)` | ✅ (`Disposable`; GPUI keystrokes and key contexts) | #204 |
| Editor, diff editor and model registries and events | `getEditors`, `getDiffEditors`, `onDidCreateEditor`, `onDidCreateDiffEditor`, `getModels`, `onDidCreateModel`, `onWillDisposeModel`, `onDidChangeModelLanguage` | ✅ | #204 |
| Markers across models | `getModelMarkers`, `removeAllMarkers`, `onDidChangeMarkers` | ✅ markers live in their model, so none for uris without one | #204 |
| Context menu, with host items | `contextmenu` | ✅ | #91 |
| Go to line, quick outline (quick input) | `quickAccess` | ✅ | #100 |
| Command palette, `addAction` | `quickAccess` | ✅ | #100 |
| Reveal APIs, `executeEdits`, mouse hit-test events, word and position helpers, content-change details, cursor change reasons, decorations collections | editor API | ✅ | #105 |
| Model helpers: lines, `modifyPosition`, find next/previous/scoped with captures, `applyEdits` undo edits, `pushEditOperations` with a cursor computer, `popStackElement`, alternative version id, bracket pairs and guides | `ITextModel` | ✅ | #206 |
| Custom highlighter | tokenization support | ✅ (`Language::with_highlighter`) | |
| Static colorize and tokenize | `colorizer` | ✅ | #108 |
| Colorize a model line | `colorizeModelLine` | ✅ | #204 |
| Themes: `vs`, `vs-dark`, VS Code JSON, `defineTheme` | standalone theme service | ✅ | |
| Themes by name for every editor | `editor.defineTheme`, `editor.setTheme` | ✅ | #204 |
| High-contrast themes (`hc-black`, `hc-light`) | standalone themes | ✅ | #53 |

## Language features (providers)

| Provider | Monaco | Status | Issue |
|---|---|---|---|
| Completion: fuzzy filter, docs, commit characters, additional edits | `suggest` | ✅ | |
| Completion: resolve, tags, insert/replace modes, word suggestions, accept and selection options | `suggest` | ✅ | #103 |
| Completion: tab completion, locality bonus, retrigger rules | `suggest` | ✅ | #103 |
| Completion widget: details toggle, side docs, resizing, status bar, loading and empty messages | `suggest` | ✅ | #103 |
| Completion: `selectionMode` and `focusSuggestion`, explain mode, reset size, `snippetsPreventQuickSuggestions`, every item kind | `suggest` | 🟡 no `quickSuggestions: 'inline'`/`'offWhenInlineCompletions'`, details focus or `shareSuggestSelections` | #202 |
| Hover | `hover` | ✅ content and gutter hovers, options, keyboard focus and scrolling, resizing, verbosity, copy button, long line warnings | #199 |
| Signature help: overload cycling and arrows, `enabled`/`cycle` options, close and next/previous commands | `parameterHints` | ✅ | #200 |
| Go to definition (link, F12) | `gotoSymbol` | ✅ | |
| References, declaration, type definition, implementation, peek view | `gotoSymbol`, `peekView` | ✅ | #95 |
| Open definition to the side (host event), peek focus toggle and default, resizable peek view, `definitionLinkOpensInPeek` | `gotoSymbol`, `peekView` | ✅ | #202 |
| Diagnostics: squiggles, ruler, hover, F8 | `gotoError` | ✅ | |
| Diagnostic tags (unused, deprecated), related information | `gotoError` | ✅ | #90 |
| Document highlights | `wordHighlighter` | ✅ | #83 |
| Formatting: document, range, on type, on paste | `format` | ✅ | #94 |
| Rename | `rename` | ✅ | #56 |
| Code actions and lightbulb (no AI actions, "Show Disabled" toggle, refactor preview or nearby quick fixes) | `codeAction` | ✅ | #56 |
| The `editor.action.codeAction` command with `kind`, `apply` and `preferred` | `codeAction` | ✅ | #202 |
| Rename suggestions (`NewSymbolNamesProvider`) | `rename` | ❌ | #202 |
| Inlay hints: label parts, tooltips, Cmd/Ctrl+click, double-click edits, resolve, font options (no part context menu or location hovers) | `inlayHints` | ✅ | #56, #200 |
| Code lens | `codelens` | ✅ (and "Show CodeLens Commands for Current Line", #202) | #96 |
| Color decorators (providers, default finder) | `colorDecorators` | ✅ | #97 |
| Color picker (hover and standalone) | `colorPicker` | ✅ | #97 |
| Links | `links` | ✅ | #93 |
| Semantic tokens | `semanticTokens` | ✅ | #98 |
| Inline completions (ghost text) | `inlineCompletions` | 🟡 ghost text, toolbar (hover or always), commands, options, lifetime hooks and provider groups; inline edits (next-edit suggestions) to come | #99, #201 |
| Document symbols | `documentSymbols` | ✅ | #100 |
| Selection ranges | `smartSelect` | ✅ | #82 |
| Folding ranges | `folding` | ✅ | #102 |
| Linked editing ranges | `linkedEditing` | ✅ | #101 |
| LSP bridge (`gpui-monaco-lsp`: sync, diagnostics, completion, hover, definition, highlights, signature help) | `monaco-languageclient` | ✅ | #54 |

## Other

| Feature | Status | Issue |
|---|---|---|
| Diff editor, multi-diff editor | ✅ `DiffEditor` (side by side or inline, hidden unchanged regions, revert buttons, moved blocks, the accessible diff viewer, legacy and advanced algorithms, `compactMode`, `diffWordWrap`, `maxFileSize`, switch side, view state, `revealFirstDiff`) and `MultiDiffEditor`; smaller gaps in #181 | #52, #181, #203 |
| Screen readers, tab focus mode, ARIA options | ✅ | #53 |
| wasm / web | 🟡 compiles for `wasm32-unknown-unknown`; browser runner pending | #58 |
| crates.io release | ❌ | #57 |
| Monaco migration notes | ✅ (book: Migrating from Monaco) | #60 |
| Small correctness follow-ups | — | #76 |

## Not planned

These Monaco internals have no equivalent here, by design:

- `gpu` renderer and the DOM `editContext`: GPUI renders on the GPU natively.
- `useShadowDOM`, `fixedOverflowWidgets`, `extraEditorClassName` and `automaticLayout`: DOM-specific.
- `inlineProgress` and `floatingMenu`: VS Code workbench UI.
