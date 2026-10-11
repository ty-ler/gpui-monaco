# Differences from Monaco

gpui-monaco aims to *behave* like Monaco. Where it differs, it's usually because of Rust, GPUI or the MVP scope.

## API shape

- **Positions:** 0-based `PointUtf16 { row, column }` instead of 1-based `{ lineNumber, column }`. Use `PointUtf16::from_monaco` and `to_monaco` to convert. View zones' `after_line` keeps Monaco's 1-based meaning.
- **Edits and decorations:** take byte offset ranges, not `IRange`.
- **Options:** a typed `EditorOptions` struct, not a dynamic options bag. Change them with `update_options`.
- **Commands:** keybindings and commands are GPUI actions. Monaco's `addAction` and `addCommand` correspond to defining your own actions; see [Keybindings](keymap.md).
- **Providers:** registered app-wide per language selector. A provider returns a GPUI `Task`, not a `Promise`, and has no cancellation token; dropping the task cancels it.
- **Highlighting:** done by tree-sitter grammars and queries, not Monarch tokenizers. Monaco theme rules still apply, because captures are mapped to Monaco token names and TextMate scopes.

## Behavior

The repository's [`docs/MONACO_PARITY.md`](https://github.com/ty-ler/gpui-monaco/blob/main/docs/MONACO_PARITY.md) has a full feature-by-feature audit with tracking issues. The highlights:


- **Find regexes:** use the Rust [`regex`](https://docs.rs/regex) crate (linear time). It has no lookahead, lookbehind or backreferences.
  - `\n` in a pattern still enables multiline search, as in Monaco.
  - Replacement patterns support `$1`, `$&`, `$$`, `\n` and `\t`. Monaco's case-changing escapes (`\u`, `\U`, `\l`, `\L`) are not supported.
  - Language configuration patterns (`wordPattern`, `indentationRules`, …) use `fancy-regex`, so VS Code's configurations work unchanged.
- **Cursor and scrolling:** the smooth caret only animates the primary cursor. Horizontal smooth scrolling isn't animated. `inertialScroll` isn't there: the platforms GPUI supports give trackpad momentum themselves.
- **Context menu:** it's drawn inside the editor, flipping up or left at its edges, rather than as an OS-level menu. The scrollbar menu and the "Copy As" and "Share" submenus aren't there.
- **Bracket pair guides:** they use the bracket pair colors. The `editorBracketPairGuide.*` theme colors aren't read; Monaco's defaults for them fall back to the same colors. Pairs come from the colorized bracket pairs, and `highlight_active_indentation` has no `"always"`.
- **Advanced wrapping:** Monaco lets the browser break lines in a hidden DOM node. Here, characters are measured in the editor font and breaks follow the same break-before/after characters as `Simple` wrapping. `disableMonospaceOptimizations` isn't needed: cursor and selection positions always come from shaped text.
- **Unicode highlighting:** there's no workspace trust, so Monaco's `inUntrustedWorkspace` defaults resolve as trusted, as in the standalone editor. When there are too many highlights, Monaco shows a banner offering to turn the kind off; here nothing is highlighted, without the banner. "Adjust settings" changes the editor's options, not user settings.
- **Unusual line terminators:** Monaco asks in a modal dialog; here a quick pick asks, and dismissing it ignores them, like the dialog's Ignore.
- **Colorize:** `colorize` returns lines and highlights (and `colorize_element` an element) rather than HTML. Token types come from tree-sitter captures mapped to Monaco's names, so they can differ from a Monarch tokenizer's.
- **Copy with syntax highlighting:** not available, because GPUI's clipboard holds plain text, images and files but no HTML or RTF.
- **Drop and paste:** GPUI delivers dropped files, not dragged text from other apps, and the clipboard's text only (no `text/html` or images), so the HTML provider only applies to data given to `drop_data`/providers directly. There's no relative path provider, because the editor has no workspace, and no copy-time metadata (`prepareDocumentPaste`). The options button is a small chip; its picker is the quick pick.
- **Fonts:** `letterSpacing` and `fontVariations` aren't supported, because GPUI text has neither. Decorations can't change the font (`fontFamily`, `fontSize` and so on), because GPUI shapes a line in one font size. Taller lines from `line_height` stay uniform in the minimap and sticky scroll. A variable font's weight comes from `font_weight`.
- **Minimap section headers:** `section_header_letter_spacing` is accepted but not applied, because GPUI text has no letter spacing.
- **Color picker:** the standalone picker isn't bound to a key (as in Monaco). It runs from the command palette or `ShowOrFocusStandaloneColorPicker`.
- **Quick access:**
  - Symbols are matched with the suggest widget's fuzzy scorer, not VS Code's quick open scorer.
  - The command palette has no similar-command or TF-IDF suggestions (those are VS Code workbench features), and accents aren't folded when matching.
  - Icons are glyphs, not codicons.
- **Inline completions:**
  - There is no toolbar (`showToolbar`).
  - Ghost text isn't syntax highlighted.
  - Secondary cursors don't get it.
  - Provider groups (`yieldsToGroupIds`) and `completeBracketPairs` aren't supported.
  - The suggest widget's items aren't extended by inline completions.
  - A multi-line completion inserted mid-line keeps the rest of the line after its first line, rather than moving it to the end of the ghost text.
- **Snippets:**
  - Transform regexes run on Rust's regex engine (via JavaScript translation), so exotic JavaScript syntax may not compile.
  - `$CURRENT_*` day and month names are English.
  - There's no overtyping capture for `$SELECTION`.
- **Word-based suggestions:** words are not ranked by distance from the cursor, and the documents are the models registered with `register_model`.
- **Suggestion memories:** they are kept in memory for the app's lifetime, not persisted. The same goes for the suggest widget's size and whether details are expanded.
- **Suggest widget:** the details panel goes beside the list or below it, never to the left or above. Its contents can't be focused or scrolled with the keyboard, and explain mode (`toggleExplainMode`) isn't ported. Icons are glyphs, not codicons.
- **Code actions, rename, inlay hints:** there's no refactor preview, no rename suggestions, and inlay hint label parts can't be clicked. [`docs/MONACO_AUDIT.md`](https://github.com/ty-ler/gpui-monaco/blob/main/docs/MONACO_AUDIT.md) lists every known gap.
- **Folding:** there is no folding by tree-sitter fold queries yet. Ranges come from indentation, markers and folding range providers.
- **Accessibility:** screen readers get the lines around the selection as AccessKit text runs, as Monaco's hidden textarea does. Tab focus mode is announced only in Monaco: GPUI has no live announcements. `accessibilitySupport` isn't an option: the text runs are built whenever assistive technology is connected.
- **Diff editor:** only Monaco's advanced diff algorithm exists, and there's no `maxFileSize` cut-off, `hideOriginalLineNumbers`, `compactMode` or `diffWordWrap` (#203).
- **Long lines:** as in Monaco, very long lines stop rendering after `stop_rendering_line_after` with a "Show more (…)" chip. Clicking the chip doesn't expand the line; set the option to `None` to draw everything.

## Platform

- **macOS:** the primary platform.
- **Linux and Windows:** build in CI, but get less testing.
- **Keybindings:** follow Monaco's per-platform defaults (Cmd on macOS, Ctrl elsewhere).
