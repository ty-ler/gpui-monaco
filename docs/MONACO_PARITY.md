# Monaco parity audit

This is how gpui-monaco 0.1 compares with Monaco, feature by feature.

The audit compares three things:

- the `src/vs/editor/contrib/*` directories
- the commands registered in `browser/coreCommands.ts`
- the `EditorOption` enum in `common/config/editorOptions.ts`

These were checked against our actions, options and modules. The Monaco source can be fetched with `script/fetch-monaco-source`.

[`MONACO_AUDIT.md`](MONACO_AUDIT.md) is a finer-grained audit (2026-10) of sub-features, every option and command id, and the public API, with gaps tracked as #193–#206.

> **Second audit (2026-10-11):** [`MONACO_AUDIT_2.md`](MONACO_AUDIT_2.md) checked widget fidelity, core view and input, the diff editors, commands, options, theme and API in more depth. Its issues (#245–#265, #272) are all fixed. What still differs is listed in each row and in the book's differences page; #231–#233 wait on GPUI.

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
| Multi-cursor: add above/below, Cmd+D and previous-match variants, select all occurrences, cursors at line ends/top/bottom, Alt+click, box select by mouse and keyboard, options | `multicursor` | ✅ | #81, #260 |
| Mouse gestures: click counting, quadruple-click select all, Shift+double/triple-click and line-number Shift+click extension, `createCursor` toggle, column select following `multiCursorModifier`, middle-drag, drag autoscroll | `mouseHandler`, `viewController.dispatchMouse`, `dragScrolling` | ✅ | #245, #260 |
| IME composition at every cursor, active-clause underline | `compositionType`, `nativeEditContext` | 🟡 GPUI passes the IME's selected range but not its clause boundaries or underline styles, so only the selected clause is drawn thick | #260 |
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
| Line operations: move (reindenting with `autoIndent: full`), copy, delete, insert, indent | `linesOperations`, `moveLinesCommand` | ✅ | #260 |
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
| Find widget layout: toggles in the inputs, replace toggle column, reduced/narrow/collapsed widths, measured view zone, placeholders, titles with keybindings, disabled states, regex error message, theme colors, slide-in, sash, Tab order, screen-reader status | `findWidget` | ✅ no reduced-motion setting; titles use Monaco's keybinding labels | #250, #251 |
| Find commands from the editor and the find inputs: Alt+Enter, Cmd/Ctrl+D and Cmd/Ctrl+Shift+L acting on the editor, Cmd/Ctrl+D seeding the search string (with Monaco's whole-word, match-case and regex overrides), session-aware selection highlights, per-input Shift+Enter, Cmd+Enter and Cmd/Ctrl+Down | `findController`, `multicursor` | ✅ | #250 |
| Bracket matching and jump | `bracketMatching` | ✅ `always`/`near`/`never`, overview ruler mark | #195 |

## View

| Feature | Monaco | Status | Issue |
|---|---|---|---|
| Gutter: line numbers (on, off, relative, interval, custom function), glyph margin, line decorations, `selectOnLineNumbers` | view parts | ✅ | #203 |
| Gutter fidelity: the primary cursor's active number, the left-aligned relative current line, the dimmed final newline, glyph margin lanes (`GlyphMarginLanesModel`, decoration `glyphMargin.position` and widget lanes and z-index), glyphs on every line of a range, folding controls' size, color, fade and tooltip, the "⋯" placeholder, gutter clicks keeping focus | `lineNumbers`, `glyphMargin`, `foldingDecorations`, `mouseHandler` | ✅ glyph margin widgets take a row, not a range; no manual-folding icons | #254 |
| Cursor styles (line, block, underline, thin variants, block outline), `mouseStyle`, `mouseMiddleClickAction` | options | ✅ | #203 |
| Mouse pointer: the I-beam (`mouseStyle`) over the text only, the arrow over the gutter, scrollbars, minimap and widgets, a hand over folding controls, links and overflow chips, each area's pointer kept while drag-selecting text, and a drag's pointer kept over the whole window until the release (sashes, sliders, color strips, line-number selections) | `.view-lines` `monaco-mouse-cursor-text`, view part CSS, `GlobalPointerMoveMonitor`'s `setPointerCapture` | ✅ | |
| Current-line highlight, scrollbars, overview ruler, scroll shadow | view parts (`DecorationsOverviewRuler`, `OverviewZoneManager`, `ScrollDecorationViewPart`) | ✅ marks placed by Monaco's scroll-height formula (view zones, padding, scrolling beyond the last line), spanning their lines with a 6px minimum, merged per color and lane; 2px cursor marks in the multi-cursor colors; Monaco's lanes, border and background; the 6px top shadow; paged track clicks that keep dragging, rounded sliders, the 100ms fade-in | #256 |
| Current-line highlight modes (`gutter` only in the gutter and kept with a selection), every cursor's line and wrapped lines, border and inactive color | `currentLineHighlight` | ✅ | #254 |
| Scrollbar and overview ruler options | `scrollbar`, `overviewRuler*` | ✅ | #88 |
| Word wrap: off, on, column, bounded; wrapping indent | `wordWrap` | ✅ | |
| Wrap for proportional fonts (`wrappingStrategy: advanced`), configurable break characters | `wordWrap*` | ✅ | #87 |
| Wrap overrides, `wordBreak: keepAll`, `wrapOnEscapedLineFeeds` | `wordWrapOverride1/2`, `wordBreak`, `wrapOnEscapedLineFeeds` | ✅ | #203 |
| Word wrap indicator | `wordWrapIndicator` | ✅ | #228 |
| Horizontal scrollbar left out of the content height | `scrollbar.ignoreHorizontalScrollbarInContentHeight` | ✅ | #228 |
| Large file optimizations, default line ending | `largeFileOptimizations`, `defaultEOL` | ✅ | #203 |
| Option validation: clamps and sentinel values (`fontSize` 0, `lineHeight` 0 and multipliers, sensitivities, minimap, sticky scroll, folding and color limits, delays, …), `tabSize`/`indentSize` at least 1, `indentSize: 'tabSize'` | each option's `validate`, `TextModelResolvedOptions` | ✅ the editor renders with `EditorOptions::validated`, `options()` returns the options as set | #262 |
| Layout widths: a line-height glyph margin, 16px folding column (none with `showFoldingControls: never`), the minimap's proportional share and `fill`/`fit` sizing, `lineDecorationsWidth` in characters | `EditorLayoutInfoComputer.computeLayout` | ✅ | #262, #254 |
| Platform defaults, `cursorWidth` 0 and its cap | `renderFinalNewline` (dimmed on Linux), `defaultEOL` (CRLF on Windows), `cursorWidth` | ✅ | #262 |
| Inertial scrolling, two-cell full-width characters | `inertialScroll`, `fullwidthCharacterWidth` | ✅ ghost text and the inline diff's deleted lines keep the font's width | #203, #228 |
| Folding: indentation and markers, gutter chevrons | `folding` | ✅ | |
| Syntax and provider folding (`foldingStrategy`, `foldingMaximumRegions`) | `folding` | ✅ | #102 |
| Fold level and recursive commands, fold navigation, manual ranges, unfold on `⋯` click, `foldingHighlight`, `foldingImportsByDefault` | `folding` | ✅ | #102 |
| Minimap | `minimap` | ✅ Monaco's `MinimapLayout` (scroll, slider, start line), relative slider drags, click to reveal centered or drag (`fill`/`fit`), prebaked glyphs (`MinimapCharRenderer`) with `minimap.foregroundOpacity` and the light-theme variant, selection, find, occurrence, marker and custom decorations (`Inline`, `Gutter`), slider hover/active colors and the shadow. Scales above 2 device pixels enlarge the scale-2 glyphs; no touch gestures | #255 |
| Minimap options (side, render characters, size, autohide), section headers | `minimap`, `sectionHeaders` | ✅ | #88, #203 |
| Sticky scroll (outline, folding and indentation models) | `stickyScroll` | ✅ | |
| Sticky scroll keyboard focus, toggle, folding controls, hover, scroll with editor | `stickyScroll` | ✅ | #106 |
| Sticky scroll widget state (`findScrollWidgetState`, the pushed last line, the 25% cap), models without `folding`, `offSide`, column and chevron clicks, Cmd/Ctrl+click definition, decorations and inlay hints, shadow, border and gutter colors, its context menu | `stickyScroll` | ✅ the `mouseover` chevrons don't fade | #254, #251 |
| Render whitespace, indent guides, rulers, control characters | view parts | ✅ whitespace always as Monaco's default SVG marks (no `experimentalWhitespaceRendering`) | #84, #259 |
| Cursor and selection rendering: block cursor text, view positions at wrap points, multi-cursor colors, selection corners and insets, view part paint order | `viewCursors`, `selections`, `CursorState.viewState` | ✅ the "Show more" chip has no 4px padding | #259 |
| Bracket pair guides, `guides.highlightActiveIndentation: 'always'` | view parts | ✅ | #118, #203 |
| Bracket pair colorization, independent color pool per bracket type | `bracketPairColorization` | ✅ | #85, #203 |
| Occurrence and selection highlight, `selectedTextMatchMode`, `occurrencesHighlightFromAllProviders` | `wordHighlighter` | ✅ `multiFile` highlights in the current editor only | #83, #203, #228 |
| Unicode highlighting, unusual line terminators | `unicodeHighlighter`, `unusualLineTerminators` | ✅ | #86 |
| Unicode highlighter "too many characters" banner, `disableHighlightingOf*` commands | `unicodeHighlighter`, `setBanner` | ✅ the commands change the editor's options, not user settings | #227 |
| Font weight, ligatures, zoom, mouse wheel zoom | `fontZoom`, options | ✅ | #87 |
| Variable line heights (`lineHeight` decorations, `allowVariableLineHeights`) | decorations | ✅ | #87 |
| Per-decoration fonts (`fontFamily`, `fontWeight`, `fontStyle`) | decorations | ✅ wrapping measures the editor's font; no per-decoration `fontSize` | #272 |
| Letter spacing, font variations | options | ❌ | #87 |
| Smooth scrolling, caret animation, blinking styles, cursor surrounding lines, padding, wheel and middle-click options | options | ✅ | #88 |
| Placeholder text, read-only message | `placeholderText`, `readOnlyMessage`, `message` | ✅ the message's arrow, 8px gap, fade in and out, reveal, `editorHoverWidget.*` colors with the `inputValidation.infoBorder` border, and staying open on blur under the mouse | #89, #251 |
| Very long lines | `stopRenderingLineAfter`, `maxTokenizationLineLength` | ✅ | #59 |
| Clicking past the rendering limit turns it off | `longLinesHelper` | ✅ | #227 |

## Extensibility

| Feature | Monaco | Status | Issue |
|---|---|---|---|
| Decorations: background, underline, glyph, gutter bar, before/after text, overview ruler, hover message, stickiness, owner | `deltaDecorations` | ✅ | |
| View zones, content widgets, overlay widgets, `allowEditorOverflow`; widgets keep their wheel, clicks and focus, clipped to the text area | widget API | ✅ overflowing widgets are GPUI deferred draws above the whole window | #245 |
| `IViewZone` fields (`afterColumn`/affinity, `ordinal`, `showInHiddenAreas`, `suppressMouseDown`, `minWidthInPx`, `marginDomNode`, `layoutZone`), zones scrolling horizontally at the scroll width; `IContentWidget` `secondaryPosition`, `positionAffinity`, `suppressMouseDown`, `maxWidth`, `layoutContentWidget` with a null position; overlay corners, `stackOrdinal`, null positions, `getMinContentWidthInPx`, `layoutOverlayWidget`; zones dropped and widgets re-anchored on `setModel` | `viewZones`, `contentWidgets`, `overlayWidgets` | ✅ no `onDomNodeTop`/`onComputedHeight`, `beforeRender`/`afterRender`, `useDisplayNone` or batched `changeViewZones`; stacked right-corner overlay widgets stack downward (Monaco's code offsets them by their width) | #252 |
| Custom commands and keybindings | `addAction`, `addCommand` | ✅ `add_action` and `add_command(keystrokes, handler, when)` bind in their editor only and return a `Disposable`; GPUI keystrokes | #261 |
| Commands by id with their arguments: core (`type`, `paste`, `cut`, `cursorMove`, `editorScroll`, `revealLine`, `setSelection`, `createCursor`, `columnSelect`, the mouse commands, `compositionStart`/`End`, `default:*`), the contributions' commands and aliases, `editor.fold`/`unfold` `FoldingArguments`, `refactor`/`sourceAction`/`pasteAs`/`showHover`/`inlineSuggest.trigger` arguments | `trigger`, `executeCommand` | ✅ a test checks every id registered in `coreCommands.ts`, `editorExtensions.ts`, `contrib/` and `standalone/` resolves; mouse commands take model positions (`viewPosition` is ignored) | #261 |
| Preconditions on the palette, `trigger` and keybindings (a failing one lets the key fall through) | `precondition`, `kbOpts` | ✅ Monaco's expressions over the context keys; bindings in a widget's own key context (hover, rename input, peek list) keep theirs | #261 |
| Context keys for keybinding rules (`EditorContextKeys` and the contributions' keys), host keys | `createContextKey`, `when` clauses | ✅ GPUI key context entries; values compare as identifiers, so `supportedCodeAction`'s kinds also have one key each (`supportedCodeAction_refactor`, …) | #261 |
| `executeCommand(source, ICommand)`, `executeCommands` | `ICodeEditor` | ✅ `EditorCommand` with Monaco's builder and cursor state data | #261 |
| Internal API commands | `_executeLinkProvider`, `_executeSignatureHelpProvider`, `_executeSelectionRangeProvider`, `_executeCompletionItemProvider`, `_provideDocument(Range)SemanticTokens(Legend)` | ✅ functions in `api_commands` returning the providers' answers (commands return nothing); the other `_execute*` ids are left to the extension host | #261 |
| Global commands, actions and removable keybinding rules | `editor.addCommand`, `addEditorAction`, `addKeybindingRule(s)` | ✅ (`Disposable`; GPUI keystrokes and key contexts) | #204 |
| Editor, diff editor and model registries and events | `getEditors`, `getDiffEditors`, `onDidCreateEditor`, `onDidCreateDiffEditor`, `getModels`, `onDidCreateModel`, `onWillDisposeModel`, `onDidChangeModelLanguage` | ✅ widget inputs and placeholder models are left out (Monaco `isSimpleWidget`) | #204, #245 |
| Markers across models | `getModelMarkers`, `removeAllMarkers`, `onDidChangeMarkers` | ✅ markers live in their model, so none for uris without one | #204 |
| Context menu, with host items | `contextmenu` | ✅ measured width, scrolling, submenus flipping at the window edge after a 250ms delay, the sticky scroll menu, none on injected text, Shift+F10 revealing the cursor | #91, #251 |
| Go to line, quick outline (quick input) | `quickAccess` | ✅ a virtualized list with wheel scrolling and Monaco's scrollbar, separators on their group's first item, row hover, theme highlight colors and keycap keybindings | #100, #251 |
| Keybinding labels | `keybindingLabels`, `USLayoutResolvedKeybinding`, `KeybindingLabel` | ✅ UI, ARIA, Electron and settings labels (`keybinding_label`) | #251 |
| Go to offset (`::`, `editor.action.gotoOffset`), with the zero-based toggle | `quickAccess` | ✅ the toggle is app-wide state, not storage | #100, #227, #261 |
| Command palette, `addAction` | `quickAccess` | ✅ | #100 |
| Reveal APIs, `executeEdits`, mouse hit-test events, word and position helpers, content-change details, cursor change reasons, decorations collections | editor API | ✅ | #105 |
| Editor events: model, language, configuration, layout and content size changes, typing, clipboard, key, wheel and drag events, widget focus, IME composition, hidden areas, view zones, begin/end update | `ICodeEditor.on*` | ✅ hidden-area changes are reported when the next frame applies them; listeners can't change clipboard data or cancel keys | #205, #229 |
| Events only when something changed (Monaco's `Scrollable` and cursor state comparisons, so two editors can sync scroll and selections both ways), with Monaco's payloads; editor-level model events | `onDidScrollChange` (`IScrollEvent`), `onDidChangeCursorPosition`, `onDidChangeCursorSelection`, `onDidChangeModelContent`, `onDidChangeModelDecorations`, `onDidChangeModelOptions`, `onDidChangeModelTokens`, `onDidChangeModelLanguageConfiguration` | ✅ token change ranges (`onDidChangeTokens`) are the edited rows widened by what the highlighter changed (tree-sitter's changed ranges, the lines a Monarch grammar retokenized); other highlighters report from the first edit on | #264, #272 |
| Monaco offsets and text getters: `getOffsetAt`/`getPositionAt` and `rangeOffset`/`rangeLength` in UTF-16 units with the model's line ending, `getValue(eol, preserveBOM)`, `getValueInRange(range, eol)`, `getValueLength(InRange)`, `getCharacterCountInRange` | `ITextModel` | ✅ the byte offsets (`offset_at`, `ContentChange::range_offset`) stay beside them; change texts have `\n` line breaks | #264 |
| `setEOL`/`pushEOL` as content changes and new versions; `setValue` destroying decorations; an editor's decorations leaving the model it stops showing | `ITextModel`, `_detachModel` | ✅ | #264 |
| `executeEdits` with `forceMoveMarkers` and a cursor state computer, joining the open undo element; `popUndoStop`; `setHiddenAreas` with a source; `IMouseTarget` `detail`, `range` and `mouseColumn`; model and editor decoration getters; decorations collections following `setModel` | `ICodeEditor`, `ITextModel` | ✅ forced moves apply to decorations and the editor's cursors | #264 |
| Global openers | `registerLinkOpener`, `registerEditorOpener` | ✅ openers answer synchronously (Monaco's may return a promise) | #264 |
| Editor conveniences: `getPosition`/`setPosition`, `setSelection`, `getValue`/`setValue`, `getConfiguredWordAtPosition`, `fontInfo`, partial `updateOptions`, `getVisibleRangesPlusViewportAboveBelow`, `getScrolledVisiblePosition`, MIME types for `createModel`/`setModelLanguage` | `ICodeEditor`, `monaco.editor` | ✅ `update_options_with` takes a closure; `content_height` leaves out the `scrollBeyondLastLine` room Monaco's `getContentHeight` adds | #264 |
| Decoration options: `showIfCollapsed`, `firstLineDecorationClassName`, theme colors (`ThemeColor`) and `darkColor` for the overview ruler and minimap; injected text `cursorStops` (in Left/Right/Up/Down/End moves and clicks, ported from `ModelLineProjectionData`), `attachedData` (mouse targets) and `tokens` | `IModelDecorationOptions`, `InjectedTextOptions`, view model | ✅ styles instead of class names (`inlineClassNameAffectsLetterSpacing` isn't needed); the cursor's place around injected text is kept beside its model position, not in a view column of its own, so wrapped lines and selections still use model columns | #272 |
| Remaining API: per-model `onWillDispose`, `setModel(null)`, `hasModel`, `getContribution`, `getFontSizeAtPosition`, `writeScreenReaderContent`, `setAriaOptions`, a `Disposable` from `registerCommand`, marker code links (`code: { value, target }`), `createModel` failing for a taken uri | `ICodeEditor`, `ITextModel`, `monaco.editor` | ✅ contributions are found by type (`get_contribution::<T>()`), and Monaco's built-in ones are editor methods; `getFontSizeAtPosition` is the editor's size, as decorations don't change it; `TextModel::on_will_dispose` fires as the model is released (GPUI has no explicit `dispose`) | #264, #247, #272 |
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
| Symbol icon colors | `symbolIcon.*Foreground` | ✅ suggest widget and quick outline codicons | #227, #246 |
| Codicons | `base/common/codicons.ts`, `@vscode/codicons` | ✅ the codicon font (CC-BY-4.0) behind the optional `codicons` feature, else Unicode stand-ins; every Monaco icon id (`codicons::Codicon`) and the icons Monaco's widgets use, with Unicode fallbacks when the font can't load | #246 |

## Language features (providers)

| Provider | Monaco | Status | Issue |
|---|---|---|---|
| Completion: fuzzy filter, docs, commit characters, additional edits | `suggest` | ✅ | |
| Completion: resolve, tags, insert/replace modes, word suggestions, accept and selection options | `suggest` | ✅ | #103 |
| Completion: tab completion, locality bonus, retrigger rules | `suggest` | ✅ | #103 |
| Completion widget: details toggle, side docs, resizing, status bar, loading and empty messages | `suggest` | ✅ | #103 |
| Completion widget layout: sized to the room above and below without flipping while typing, north sash, Monaco's list scrollbar, details placed east, west, below or above and resizable, clickable status bar, ellipsis, the details type line in the editor font, `fitWidthToDetails`, screen reader announcements | `SuggestWidget._layout`, `SuggestDetailsOverlay._placeAtAnchor`, `SuggestWidgetStatus` | ✅ the list scrolls by whole rows; the scrollbars appear and disappear without Monaco's fade | #248 |
| Completion: Left/Right refilter in the word, Up/Down with a single focused suggestion move the cursor | `SuggestModel._onCursorChange`, `selectNextSuggestion` precondition | ✅ | #248 |
| Completion: `selectionMode` and `focusSuggestion`, explain mode, reset size, `snippetsPreventQuickSuggestions`, `shareSuggestSelections`, every item kind | `suggest` | ✅ memories are kept per window unless shared (Monaco's workspace and profile storage); nothing is persisted | #202, #248 |
| Completion: `quickSuggestions: 'inline'` (completion items as ghost text) and `'offWhenInlineCompletions'`, details focus (`toggleSuggestionFocus`), `suggestWidgetCopy` | `suggest`, `suggestInlineCompletions` | ✅ copying takes the whole details panel (there's no text selection in it); inline items skip word-based suggestions and eager resolving | #226 |
| Hover | `hover` | ✅ content and gutter hovers, options, keyboard focus and scrolling, resizing from full edges, verbosity, copy button, long line warnings, the `hoverHighlight` range, rounded card with `shadow-lg`, full-width row separators, read on focus; the problem hover's status bar (View Problem, Quick Fix from the marker's quick fixes, "No quick fixes available", AI fixes; no menu-contributed actions), editor-font body with code and related-information links | #199, #247 |
| Markdown in hovers, suggest details, parameter hints and messages | `MarkdownRenderer`, `IMarkdownString` | 🟡 GFM tables, quotes, nested lists, strikethrough, images, highlighted code blocks, `$(icon)`s (`supportThemeIcons`), a sanitized HTML subset (`supportHtml`), trusted `command:` links, plain-text documentation, the editor's line height; text can't be selected (GPUI has no selectable text) and inline code isn't rounded | #246 |
| Signature help: overload cycling and arrows, `enabled`/`cycle` options, close and next/previous commands | `parameterHints` | ✅ | #200 |
| Signature help fidelity: maximum height and scrolling, whole-word parameter labels, active parameter style and docs separator, the 120ms delay with merged triggers, no default trigger characters, screen reader announcements | `ParameterHintsModel`, `ParameterHintsWidget` | ✅ | #248 |
| Go to definition (link, F12) | `gotoSymbol` | ✅ | |
| Definition preview hover (Cmd/Ctrl+hover, `showDefinitionPreviewHover`) | `gotoSymbol` | ✅ | #225 |
| References, declaration, type definition, implementation, peek view | `gotoSymbol`, `peekView` | ✅ | #95 |
| Peek view fidelity: virtualized 23px results list with a scrollbar, no file row for one file, count badges, focus and selection colors, tree keys (Left/Right/Space/Enter on files, PageUp/PageDown, type-to-navigate, Ctrl+Enter to the side), 1px frame, header separator, title spacing and click, codicon close action, shadow, arrow at column 2, whole-line height sash, the preview's options and smooth reveal; F4 and F12 as Monaco binds them | `referencesWidget`, `referencesTree`, `peekView`, `zoneWidget`, `symbolNavigation` | ✅ the sash hover shows at once (Monaco waits 300ms); no file icons (Monaco's standalone tree has none either) | #252 |
| Open definition to the side (host event), peek focus toggle and default, resizable peek view, `definitionLinkOpensInPeek` | `gotoSymbol`, `peekView` | ✅ | #202 |
| `goToLocations`/`peekLocations` API commands (and `showReferences`), `gotoLocation.multipleTests` | `gotoSymbol` | ✅ | #227 |
| Diagnostics: squiggles, ruler, hover, F8 | `gotoError` | ✅ | |
| Diagnostic tags (unused, deprecated), related information, code links (`code: { value, target }` as `Marker::code_target`) | `gotoError` | ✅ | #90, #247 |
| Document highlights | `wordHighlighter` | ✅ 50ms after the cursor rests, then `occurrencesHighlightDelay` (0) from the move | #83, #262 |
| Formatting: document, range, on type, on paste | `format` | ✅ | #94 |
| Choosing between formatters (`FormattingConflicts.setFormatterSelector`, `displayName`) | `format` | ✅ the first by default, as standalone Monaco; no built-in picker (`formatDocument.multiple` is VS Code workbench) | #229 |
| Rename | `rename` | ✅ measured input width, line-height input and candidates, scrolling suggestions, `widget.shadow`, cancelled when scrolled out of view | #56, #251 |
| Rename preview (`acceptRenameInputWithPreview`, through a host preview handler) | `rename` | ✅ | #225 |
| Code actions and lightbulb (no AI actions or nearby quick fixes) | `codeAction` | ✅ the lightbulb's title with its keybinding and AI colors; the menu measured, scrolling to the focused row with a sticky action bar, keycaps and `widget.shadow` | #56, #251 |
| Code action menu: "Show Disabled", documentation links, preview (Cmd/Ctrl+Enter, through a host preview handler; no Refactor Preview panel) | `codeAction` | ✅ | #225 |
| The `editor.action.codeAction` command with `kind`, `apply` and `preferred` | `codeAction` | ✅ | #202 |
| Rename suggestions (`NewSymbolNamesProvider`) | `rename` | ✅ | #225 |
| Inlay hints: label parts, tooltips, Cmd/Ctrl+click, double-click edits, resolve, font options, spacers, padding and rounded boxes, the part location menu and location hovers, fixed lengths while typing, the per-model cache | `inlayHints` | ✅ | #56, #200, #253 |
| Code lens: placeholder while resolving, theme/font/zoom-following style and height, first non-whitespace column, horizontal scrolling, `$(icon)` titles, tooltips, fade-in | `codelens` | ✅ (and "Show CodeLens Commands for Current Line", #202) | #96, #253 |
| Color decorators (providers, default finder) | `colorDecorators` | ✅ | #97 |
| Color picker (hover and standalone) | `colorPicker` | ✅ `editorHoverWidget.*` chrome, `color-mode` header icon and title, hover states, grab cursors | #97, #251 |
| Links | `links` | ✅ | #93 |
| Semantic tokens | `semanticTokens` | ✅ | #98 |
| Inline completions (ghost text, inline edits) | `inlineCompletions` | 🟡 ghost text, toolbar (hover or always; its menu and the gutter menu work from the keyboard), commands, options (`triggerCommandOnProviderChange`, `experimental.suppressInlineSuggestions` and `showOnSuggestConflict`, `inlineCompletionsAccessibilityVerbose`), lifetime hooks, partial accept kinds, provider groups with `excludesGroupIds`, provider refresh with a change hint (`onDidChangeInlineCompletions`), `completeBracketPairs`, `warning`, `shownCommand` and the full request context; the suggest widget's selection is passed to providers and its preview extended (`_computeAugmentation`, `handleSuggestAccepted`), and inline edits hide while it has one; inline edits with Monaco's word, line, insertion, deletion, side-by-side, collapsed, custom (`hint`) and jump-to views, the gutter indicator (line number, pulse; across the whole gutter, so without a glyph margin too) and its menu (kept in the window, closed on scroll), the long-distance hint, cross-file edits (through `OpenLocation`), `jumpToPosition` items, alternative actions (Shift+Tab; supplied by the provider), `showInlineEditMenu`, undo/redo special cases and the fade-out; no snooze, model picker, onboarding, suggest preselection, `disposeInlineCompletions` or `enableForwardStability` | #99, #201, #216, #230, #249 |
| Document symbols | `documentSymbols` | ✅ | #100 |
| Selection ranges | `smartSelect` | ✅ | #82 |
| Folding ranges | `folding` | ✅ | #102 |
| Linked editing ranges | `linkedEditing` | ✅ | #101 |
| LSP bridge (`gpui-monaco-lsp`: sync, diagnostics, completion, hover, definition, highlights, signature help) | `monaco-languageclient` | ✅ | #54 |

## Other

| Feature | Status | Issue |
|---|---|---|
| Diff editor, multi-diff editor | ✅ `DiffEditor` (side by side or inline, hidden unchanged regions, revert buttons, moved blocks, the accessible diff viewer, legacy and advanced algorithms, `compactMode`, `diffWordWrap`, `maxFileSize`, `diffCodeLens`, `accessibilityVerbose`, `experimental.showEmptyDecorations` and `useTrueInlineView`, `hideOriginalLineNumbers`, `isInEmbeddedEditor`, `originalAriaLabel`/`modifiedAriaLabel`, switch side, view state, `revealFirstDiff`, `getLineChanges`, `identical`, `setModel(null)`, `waitForDiff`, `addAction`/`addCommand`; Monaco's sub-editor overrides, alignment around all view zones, the diff overview ruler, sash, extensible gutter toolbar, inline original line numbers and copyable deleted lines, `diffEditorBreadcrumbs`) and `MultiDiffEditor`. Not ported: `createViewModel` and `createContextKey`, the temporary inline mode during a smooth resize, and the compare offset animation; codicons are text glyphs until #246. `MultiDiffEditor`: scrollbars, virtualized editors from Monaco's template pool with view state kept, Monaco's standard headers (file label, status, toolbar from `add_file_action`, sticky shadow, borders, active outline), next/previous change across files, `go_to_file`, `reveal` with a range; not ported: the compact and card variants, binary file placeholders, smooth scrolling and the compressed scroll layout for very tall lists, and view state save/restore of the whole list | #52, #181, #203, #228, #257, #258 |
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
