# Monaco parity audit

This is how gpui-monaco 0.1 compares with Monaco, feature by feature.

The audit compares three things:

- the `src/vs/editor/contrib/*` directories
- the commands registered in `browser/coreCommands.ts`
- the `EditorOption` enum in `common/config/editorOptions.ts`

These were checked against our actions, options and modules. The Monaco source can be fetched with `script/fetch-monaco-source`.

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
| Copy with syntax highlighting, paste as, Linux primary selection, reindent on paste | `clipboard`, `dropOrPasteInto` | ❌ | #107 |
| Undo/redo with grouping, cursor restore | model | ✅ | |
| Auto-closing, overtype, auto-surround | `cursor` | ✅ | |
| Auto-indent (`onEnterRules`, `indentationRules`, electric characters) | `indentation` | ✅ | |
| Reindent lines | `indentation` | ✅ | #80 |
| Line operations: move, copy, delete, insert, indent | `linesOperations` | ✅ | |
| Sort, join, reverse, transpose, case transforms, delete duplicates, trim whitespace | `linesOperations`, `caretOperations` | ✅ | #80 |
| In-place replace | `inPlaceReplace` | ✅ | #80 |
| Toggle line and block comments | `comment` | ✅ | #79 |
| Smart select (expand and shrink) | `smartSelect` | ❌ | #82 |
| Linked editing (HTML tags) | `linkedEditing` | ❌ | #101 |
| Snippets | `snippet` | ❌ | #55 |
| Drag and drop text | `dnd` | ❌ | #92 |
| Find/replace: case, word, regex, in selection, replace all | `find` | ✅ | |
| Find history, preserve case, seed and loop options, case escapes in replace | `find` | ❌ | #104 |
| Bracket matching and jump | `bracketMatching` | ✅ | |

## View

| Feature | Monaco | Status | Issue |
|---|---|---|---|
| Gutter: line numbers (on, off, relative, interval), glyph margin, line decorations | view parts | ✅ | |
| Current-line highlight, scrollbars, overview ruler | view parts | ✅ | |
| Scrollbar and overview ruler options | `scrollbar`, `overviewRuler*` | ❌ | #88 |
| Word wrap: off, on, column, bounded; wrapping indent | `wordWrap` | ✅ | |
| Wrap for proportional fonts (`wrappingStrategy: advanced`), configurable break characters | `wordWrap*` | ❌ | #87 |
| Folding: indentation and markers, gutter chevrons | `folding` | ✅ | |
| Syntax and provider folding, fold level and recursive commands, unfold on `⋯` click | `folding` | ❌ | #102 |
| Minimap | `minimap` | ✅ | |
| Minimap options (side, render characters, size), section headers | `minimap`, `sectionHeaders` | ❌ | #88 |
| Sticky scroll (indentation folding model) | `stickyScroll` | ✅ | |
| Sticky scroll outline model, keyboard focus, toggle | `stickyScroll` | ❌ | #106 |
| Render whitespace, indent guides, rulers, control characters | view parts | ✅ | #84 |
| Bracket pair guides | view parts | ❌ | #118 |
| Bracket pair colorization | `bracketPairColorization` | ✅ | #85 |
| Occurrence and selection highlight | `wordHighlighter` | ✅ | #83 |
| Unicode highlighting, unusual line terminators | `unicodeHighlighter`, `unusualLineTerminators` | ❌ | #86 |
| Font weight, ligatures, letter spacing, zoom, variable line heights | `fontZoom`, options | ❌ | #87 |
| Smooth scrolling, caret animation, cursor surrounding lines, padding | options | ❌ | #88 |
| Placeholder text, read-only message | `placeholderText`, `readOnlyMessage`, `message` | ❌ | #89 |
| Very long lines | `longLinesHelper` | ❌ | #59 |

## Extensibility

| Feature | Monaco | Status | Issue |
|---|---|---|---|
| Decorations: background, underline, glyph, gutter bar, before/after text, overview ruler, hover message, stickiness, owner | `deltaDecorations` | ✅ | |
| View zones, content widgets, overlay widgets | widget API | ✅ | |
| Custom commands and keybindings | `addAction`, `addCommand` | ✅ (GPUI actions) | |
| Context menu, with host items | `contextmenu` | ❌ | #91 |
| Command palette, go to line, quick outline | `quickAccess` | ❌ | #100 |
| Reveal APIs, `executeEdits`, mouse hit-test events, word and position helpers | editor API | ❌ | #105 |
| Custom highlighter | tokenization support | ✅ (`Language::with_highlighter`) | |
| Static colorize and tokenize | `colorizer` | ❌ | #108 |
| Themes: `vs`, `vs-dark`, VS Code JSON, `defineTheme` | standalone theme service | ✅ | |
| High-contrast themes (`hc-black`, `hc-light`) | standalone themes | ❌ | #53 |

## Language features (providers)

| Provider | Monaco | Status | Issue |
|---|---|---|---|
| Completion: fuzzy filter, docs, commit characters, additional edits | `suggest` | ✅ | |
| Completion: resolve, tags, insert/replace modes, word suggestions, accept and selection options | `suggest` | ❌ | #103 |
| Hover | `hover` | ✅ | |
| Signature help (with cycling) | `parameterHints` | ✅ | |
| Go to definition (link, F12) | `gotoSymbol` | ✅ | |
| References, declaration, type definition, implementation, peek view | `gotoSymbol`, `peekView` | ❌ | #95 |
| Diagnostics: squiggles, ruler, hover, F8 | `gotoError` | ✅ | |
| Diagnostic tags (unused, deprecated), related information | `gotoError` | ❌ | #90 |
| Document highlights | `wordHighlighter` | ✅ | #83 |
| Formatting: document, range, on type, on paste | `format` | ❌ | #94 |
| Code actions and lightbulb, rename, inlay hints | `codeAction`, `rename`, `inlayHints` | ❌ | #56 |
| Code lens | `codelens` | ❌ | #96 |
| Colors and color picker | `colorPicker` | ❌ | #97 |
| Links | `links` | ❌ | #93 |
| Semantic tokens | `semanticTokens` | ❌ | #98 |
| Inline completions (ghost text) | `inlineCompletions` | ❌ | #99 |
| Document symbols | `documentSymbols` | ❌ | #100 |
| Selection ranges | `smartSelect` | ❌ | #82 |
| Folding ranges | `folding` | ❌ | #102 |
| Linked editing ranges | `linkedEditing` | ❌ | #101 |
| LSP bridge | — | ❌ | #54 |

## Other

| Feature | Status | Issue |
|---|---|---|
| Diff editor, multi-diff editor | ❌ | #52 |
| Screen readers, tab focus mode, ARIA options | ❌ | #53 |
| wasm / web | ❌ | #58 |
| crates.io release | ❌ | #57 |
| Monaco migration notes | ❌ | #60 |
| Small correctness follow-ups | — | #76 |

## Not planned

These Monaco internals have no equivalent here, by design:

- `gpu` renderer and the DOM `editContext`: GPUI renders on the GPU natively.
- `useShadowDOM`, `fixedOverflowWidgets`, `extraEditorClassName` and `automaticLayout`: DOM-specific.
- `inlineProgress` and `floatingMenu`: VS Code workbench UI.
