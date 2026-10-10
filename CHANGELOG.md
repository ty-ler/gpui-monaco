# Changelog

All notable changes to this project are documented here. The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and the project uses [Semantic Versioning](https://semver.org/). Until 1.0, minor versions may break the API.

## [Unreleased]

### Added
- Comment toggling: `editor.action.commentLine` (Cmd+/), `addCommentLine`, `removeCommentLine` and `blockComment` (Shift+Alt+A), using the language at the cursor (injections included). This is a port of Monaco's comment commands and their tests (#79).
- Word commands: `cursorWordStart/End{Left,Right}{,Select}`, `deleteWordStart/End{Left,Right}`, `deleteWordPart{Left,Right}` (Ctrl+Alt+Backspace/Delete on macOS) and `deleteInsideWord`. Also `lineBreakInsert` (Ctrl+O on macOS) (#81).
- Multi-cursor commands: `addSelectionToPreviousFindMatch`, `moveSelectionToPreviousFindMatch`, `insertCursorAtEndOfEachLineSelected` (Shift+Alt+I), `addCursorsToBottom`/`Top` and `focusNext`/`PreviousCursor` (#81).
- Keyboard column selection (`cursorColumnSelect*`, Cmd/Ctrl+Shift+Alt+arrows), `cursorLineStart`/`End` (Ctrl+A/E on macOS) and `cancelSelection` (#81).
- Cursor undo and redo (`cursorUndo`, Cmd/Ctrl+U) and the selection anchor (`setSelectionAnchor` Cmd+K Cmd+B, `selectFromAnchorToCursor` Cmd+K Cmd+K, `goToSelectionAnchor`, `cancelSelectionAnchor`) (#81).
- Scroll commands: `scrollLineUp`/`Down`, `scrollPageUp`/`Down` and `scrollEditorTop`/`Bottom`, plus `CodeEditor::scroll_by`, `scroll_lines` and `scroll_pages` (#81).
- Overtype mode (`toggleOvertypeInsertMode`, Insert), with `overtype_cursor_style` and `overtype_on_paste`, including IME compositions (#81).
- Options: `multi_cursor_modifier` (Alt or Cmd/Ctrl), `multi_cursor_paste`, `multi_cursor_limit`, `multi_cursor_merge_overlapping` and `column_selection` (#81).
- Line and text commands (#80):
  - Sort lines ascending/descending, delete duplicate lines and reverse lines.
  - Trim trailing whitespace (Cmd+K Cmd+X) and `deleteAllRight` (Ctrl+K / Cmd+Delete on macOS).
  - Join lines (Ctrl+J), `transpose`, `transposeLetters` (Ctrl+T) and move caret left/right.
  - Case transforms: upper, lower, title, snake, camel, Pascal and kebab.
  - Duplicate selection.
  - Reindent lines and reindent selected lines.
  - In-place replace (Cmd+Shift+. / ,).
  - Remove brackets (Cmd+Alt+Backspace).
- `EditOperation::from_edits`.
- Occurrence highlighting (#83):
  - The symbol at the cursor is highlighted after a short delay, from a new `DocumentHighlightProvider` (with read and write kinds) or the word's textual occurrences.
  - Other occurrences of the selected text are highlighted too.
  - F7 / Shift+F7 move between the highlights.
  - Options: `occurrences_highlight`, `occurrences_highlight_delay`, `selection_highlight`, `selection_highlight_max_length` and `selection_highlight_multiline`.
  - Theme colors: `editor.wordHighlight*` and `editor.selectionHighlightBackground`.
- Rendering options (#84):
  - `render_whitespace`: none, boundary, selection (default), trailing or all.
  - Indent guides with the active guide highlighted (`guides`), using a port of Monaco's guide computation.
  - `rulers`.
  - `render_control_characters`: shows ␇-style control pictures.
  - `render_final_newline`.
  - Theme colors: `editorWhitespace.foreground`, `editorIndentGuide.*` and `editorRuler.foreground`.
- Bracket pair colorization (`bracket_pair_colorization`, on by default):
  - Brackets are colored by nesting level, skipping strings and comments, and unmatched brackets use the unexpected-bracket color.
  - It honors `colorizedBracketPairs` and the `editorBracketHighlight.*` theme colors (#85).
- Faster whole-document highlighting: injection layers are merged per depth (5.5s → 0.24s on 100k lines of Rust).
- Embedder API (#105):
  - `TextModel`: `line_count`, `line_content`, `validate_position` and `validate_range`, `offset_at` and `position_at`, `value_in_range`, `word_at_position`, `word_until_position` and `find_matches`.
  - `CodeEditor`: `reveal_*` (a port of Monaco's reveal math, with `RevealType`), `visible_ranges`, `execute_edits`, `push_undo_stop`, `trigger` by action name and `has_text_focus`.
- Mouse events with hit-test targets (`MouseDown`, `MouseUp`, `MouseMove`, `MouseLeave`, `ContextMenu`, `CodeEditor::mouse_target`), a port of Monaco's `IMouseTarget` kinds (#105).
- `TextModelEvent::Edited` now carries `changes`: Monaco-style `ContentChange`s with the range, offset, length and new text (#105).
- `CodeEditorEvent::SelectionsChanged` now carries a `SelectionChange` with Monaco's `CursorChangeReason` (explicit, paste, undo, redo, content flush, recover from markers) and source (keyboard, mouse, API, model) (#105). This is a breaking change: match `SelectionsChanged(_)`.
- `CodeEditor::create_decorations_collection` returns a `DecorationsCollection`, a port of Monaco's `IEditorDecorationsCollection` (#105).
- `TextModel::language_at` (Monaco `getLanguageIdAtPosition`), and the `comments` editor option.
- Language configurations accept `lineComment` as `{ "comment", "noIndent" }`.
- **`gpui-monaco-lsp`**, a new crate that bridges language servers to the provider traits (#54):
  - document sync: `didOpen`, incremental `didChange` (also flushed before each request), `didClose`, `didSave`
  - diagnostics shown as markers
  - completion, hover, definition, document highlights and signature help
  - the requests a server sends: `workspace/configuration`, `applyEdit` and `workspaceFolders`
  - `$/cancelRequest` when a request is dropped
  - The playground runs it with `PLAYGROUND_LSP`.
- `gpui_monaco::events::content_changes`, which turns a patch into `ContentChange`s.
- **Formatting (#94):**
  - New `DocumentFormattingProvider`, `DocumentRangeFormattingProvider` and `OnTypeFormattingProvider` traits.
  - Format Document (`Shift+Alt+F`) and Format Selection (`Cmd/Ctrl+K Cmd/Ctrl+F`), plus the `format_on_type` and `format_on_paste` options.
  - Edits are reduced to minimal character changes (Monaco `computeMoreMinimalEdits`) and applied as one undo step. The cursor keeps its place on screen, and stale results are dropped.
  - The LSP bridge forwards `textDocument/formatting`, `rangeFormatting` and `onTypeFormatting`.
- **Navigation and the peek view (#95):**
  - `ReferenceProvider`, `DeclarationProvider`, `TypeDefinitionProvider` and `ImplementationProvider`.
  - Go to and peek commands: Shift+F12 references, Alt+F12 peek definition, Cmd/Ctrl+F12 implementations and Cmd/Ctrl+Shift+F12 peek implementations.
  - Monaco's `gotoLocation` options (peek, goto or goto-and-peek for several results, plus alternative commands), and F4 through results.
  - The peek view: a zone with an editable preview editor and a results list grouped by file, using the `peekView.*` theme colors.
  - Inline messages ("No definition found"), via `show_message`.
  - `register_model`, `model_for_uri` and `set_model_resolver`, so other documents can be previewed.
  - The LSP bridge forwards references, declaration, type definition and implementation requests.
- Clicks in view zones and widgets no longer focus the editor.
- **Smart select (#82):**
  - Expand and shrink selection through `SelectionRangeProvider`s, the syntax tree (`SyntaxSnapshot::selection_ranges`), or brackets, plus Monaco's word ranges and whole-line steps.
  - The `smart_select` options.
  - Ports `smartSelect.ts`, `wordSelections.ts` and `bracketSelections.ts` with their tests.
  - The LSP bridge forwards `textDocument/selectionRange`.
  - On Linux and Windows, Shift+Alt+Left/Right is now smart select, as in Monaco, instead of word-part selection (Monaco leaves word-part selection unbound there).
- **Folding commands (#102):**
  - Commands: fold and unfold recursively, toggle recursively, Fold Level 1–7, fold all block comments and regions, fold and unfold all except the selection, go to parent, previous and next fold, manual folding ranges from the selection (and removing them), and toggle import fold.
  - Monaco's gutter Shift- and Alt-clicks; clicking `⋯` unfolds.
  - Options: `folding_highlight` (`editor.foldBackground`), `unfold_on_click_after_end_of_line` and `folding_imports_by_default`. `folding_maximum_regions` also caps indentation ranges.
  - Ports `foldingModel.ts`'s commands and its tests.
- **Folding sources (#102):**
  - `FoldingRangeProvider` with `FoldingRangeKind`.
  - Syntax folding from tree-sitter `folds.scm` queries, now bundled for every language (`SyntaxSnapshot::fold_ranges`, `Highlighter::fold_ranges`).
  - The `folding_strategy` and `folding_maximum_regions` options, with Monaco's `sanitizeRanges` and nesting-level limit.
  - Debounced recomputation, and `folding_ranges` / `collapsed_folds`.
  - The LSP bridge forwards `textDocument/foldingRange`.
  - Breaking: `FoldRange::is_marker` becomes `kind`.
- **Inlay hints (#56):**
  - `InlayHintsProvider`, shown as injected text, with Monaco's request triggers (viewport, edits, `refresh_inlay_hints`).
  - The `inlay_hints` options, including Ctrl+Alt modes and maximum length.
  - The `editorInlayHint.*` colors.
  - The LSP bridge forwards `textDocument/inlayHint` and refresh.
- **Code actions (#56):**
  - `CodeActionProvider` with hierarchical `CodeActionKind`s and `resolve_code_action`.
  - The lightbulb, and Monaco's grouped menu.
  - Commands: Quick Fix (Cmd/Ctrl+.), Refactor, Source Action, Organize Imports, Fix All and Auto Fix, plus `trigger_code_actions`.
  - A command registry: `register_command` and `execute_command`.
  - `TextModelEvent::MarkersChanged`.
  - The LSP bridge forwards code actions, resolve and `workspace/executeCommand`.
- **Completion behavior (#103), ported from Monaco's `SuggestController` and `SuggestModel`:**
  - `CompletionProvider::resolve_completion_item`: the focused item is resolved, and additional edits that arrive after accepting are applied unless the text changed before them.
  - `CompletionItem` gains `replace_range`, `tags` (deprecated items are struck through), `command` (run after inserting) and `data`.
  - Insert and replace modes (`suggest.insert_mode`); Shift+Enter or Shift+Tab accepts in the other mode.
  - `accept_suggestion_on_enter` (on, smart or off) and `accept_suggestion_on_commit_character`.
  - `suggest_selection` (first, recently used, or recently used by prefix), with Monaco's selection memories.
  - Quick suggestions wait `quick_suggestions_delay`, need a word at the cursor, skip numbers, and can be enabled separately in comments, strings and other code.
  - `suggest_on_trigger_characters`.
  - Word-based suggestions (`word_based_suggestions`) from the current, matching or all registered documents when providers have nothing.
  - `suggest.hidden_kinds`, `suggest.show_deprecated`, `suggest.filter_graceful`, `suggest.match_on_word_start_only` and `snippet_suggestions`.
  - Select the first or last suggestion (Cmd+Up/Down, or Ctrl+Home/End elsewhere).
  - The LSP bridge maps tags, insert-and-replace edits and commands, and forwards `completionItem/resolve`.
  - Tab completion (`tab_completion`): at the end of a word, Tab inserts the best suggestion without showing the widget. Tab and Shift+Tab then cycle through the others, as do they after accepting with Tab or Enter.
  - `suggest.locality_bonus`: among equal matches, words used in the innermost bracket block sort first.
  - The session follows Monaco's rules as you type. It closes on another line or when the indentation changes. It asks again after deleting back past where it opened, or on starting a new word. An explicitly requested session keeps its last matches while nothing matches.
- **Rename (#56):** `RenameProvider` (with optional `resolve_rename_location`), F2 with Monaco's rename box, `WorkspaceEdit` and `CodeEditor::apply_workspace_edit`. The LSP bridge forwards `prepareRename` and `rename`.

### Changed
- `EditorOptions::quick_suggestions` is now `QuickSuggestions { other, comments, strings }` instead of a `bool` (breaking).

### Fixed

These are the small correctness follow-ups from #76:

- **Content widget placement:** when no preference fits, a content widget falls back to its first preference, as in Monaco, instead of always going below.
- **Line endings:** `TextModel::set_line_ending` takes `cx` and emits the new `TextModelEvent::LineEndingChanged` (breaking).
- **Scrolling:** `set_scroll_position` and wheel scrolling clamp at the maximum scroll right away.
- **Folding:** collapsed folds that edits shrink to one line are dropped.
- **Markdown:** inline code uses the code font, and web and mail links open on click.
- **Undo history:** `text::Buffer::clear_history` also drops the edits of an open transaction.
- **Parameter hints:** they re-query on cursor moves and close on clicks, as in Monaco's `ParameterHintsModel`.
- **Language configuration regexes:**
  - They translate more JavaScript syntax: `[^]`, `[]`, a literal `[` in a class, and Annex B hyphens such as `[\w-.]`.
  - They ignore `g`, `y`, `d`, `u` and `v` flags.
  - `Pattern::error` and `LanguageConfiguration::patterns` report regexes that don't compile.
  - The bundled HTML `onEnterRules` and CSS `wordPattern` never compiled before and now work.

## [0.1.0] - 2026-10-09

This is the first release: a Monaco-like code editor component for GPUI. It is distributed through git and pinned to gpui at zed rev `f16f9652ec57bf806e65b2a0d51bb92a63644914`. See the [guide](book/src/getting-started.md) for setup.

### Text (`gpui-monaco-text`)
- **Rope:** built on `sum_tree`, with byte, char, UTF-16 and line summaries, and `Point` / `PointUtf16` / `OffsetUtf16` coordinates.
- **Edits:** batch edits, with `Patch` composition, inversion and offset transforms.
- **Anchors:** resolved lazily with left or right bias.
- **Undo:** operation-based undo and redo with edit grouping and transactions.
- **Line endings:** LF and CRLF are detected and normalized, and a BOM is preserved.
- **Conformance:** Monaco's piece-tree and text-model buffer tests are ported.

### Syntax (`gpui-monaco-syntax`, `gpui-monaco-languages`)
- **`LanguageRegistry`:** finds languages by id, alias, extension and file name.
- **Editing rules:** VS Code `language-configuration.json` parsing, with `fancy-regex` patterns.
- **Highlighting:** tree-sitter, with incremental reparsing on a background thread and injections, including combined injections and `include-children`. Unchanged injection layers and their query results are reused across reparses.
- **Custom highlighters:** a pluggable `Highlighter` trait, set per language with `Language::with_highlighter`.
- **Bundled languages:** each is behind a cargo feature: Rust, JavaScript/JSX, TypeScript/TSX, JSON, YAML, TOML, Markdown (with fenced-code injections), HTML (with `<script>` and `<style>`) and CSS.

### Editor (`gpui-monaco`)
- **Model and view:**
  - A `TextModel` entity holds the text, language, undo history, decorations and markers.
  - Any number of `CodeEditor` views can share a model. Edits move the other views' cursors, and undo restores the cursors of the view that made the edit.
  - View state can be saved and restored.
- **Cursor core:** ported from Monaco's `cursor` with its unit tests:
  - character, word, word-part, line and page movement
  - column selection and multi-cursor
  - typing, deleting, tab and outdent
  - line operations
  - clipboard with multi-cursor distribution
  - IME composition
- **Typing helpers:** Monaco's rules for auto-closing pairs, overtype, auto-surround, auto-indent (`onEnterRules`, `indentationRules`, electric characters) and bracket matching.
- **Find and replace:** a find widget with case, whole-word, regex (Rust `regex`) and in-selection search, plus replace and replace all with `$n` substitution.
- **Multi-cursor commands:** add the next match, move to the next match, select all occurrences, and add cursors above or below.
- **View:**
  - gutter with line numbers, glyph margin and fold controls
  - current-line highlight
  - scrollbars with an overview ruler
  - word wrap following Monaco's line-break rules
  - indentation and marker folding
  - minimap
  - sticky scroll
- **Decorations:** `delta_decorations` supports backgrounds, borders, underlines, text color, whole-line decorations, glyphs, line-number color, gutter bars, overview ruler marks, injected before/after text, hover messages, stickiness and per-editor ownership.
- **Widgets:** view zones, content widgets and overlay widgets, any of which can be a GPUI view.
- **Providers:**
  - **Completion:** Monaco's `fuzzyScore` and completion model, and a suggest widget with docs.
  - **Hover:** markdown, with highlighted code blocks.
  - **Go to definition:** Cmd/Ctrl+hover link, Cmd/Ctrl+click and F12.
  - **Signature help.**
- **Diagnostics:** `set_markers` with squiggles, overview ruler marks, hover messages, and F8 navigation in an inline zone.
- **Themes:**
  - Monaco `vs` and `vs-dark`.
  - VS Code color theme JSON, with `include`; the type is inferred when the file omits it.
  - `Theme::define`.
- **Keymap:** Monaco's default keybindings per platform, as GPUI actions in the `CodeEditor` key context.

### Tooling
- **Playground:** an example app with:
  - file open and language and theme switching
  - option toggles and a status bar
  - demo providers and widgets
  - screenshot and perf modes
- **Performance:**
  - Criterion benchmarks for the text and editor components.
  - A large-file budget test: open in under 200ms, keystroke-to-paint p95 under 8ms and reparse in under 50ms on 100k lines.
  - [`docs/PERFORMANCE.md`](docs/PERFORMANCE.md) has the numbers.
- **Docs:** an mdBook guide in `book/`, and rustdoc on every public item.

### Known limitations
The [post-MVP issues](https://github.com/ty-ler/gpui-monaco/issues?q=label%3Apost-mvp) track:

- the diff editor
- screen reader support
- an LSP adapter
- snippets
- code actions, rename and inlay hints
- crates.io publishing
- wasm
- very long lines

[Differences from Monaco](book/src/differences.md) lists the smaller gaps, such as comment toggling, indent guides, bracket pair colorization and reindent on paste.

[0.1.0]: https://github.com/ty-ler/gpui-monaco/releases/tag/v0.1.0
