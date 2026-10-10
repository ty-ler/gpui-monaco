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
- **Missing commands:** code actions, rename, formatting, inlay hints, color decorators and links are not built in.
- **Folding:** clicking the `⋯` placeholder doesn't unfold the region, and there is no folding by tree-sitter fold queries yet. Ranges come from indentation and markers.
- **Paste:** pasted text is not reindented (Monaco's `autoIndentOnPaste`).
- **Accessibility:** screen reader support is post-MVP. GPUI's accessibility support is still in progress upstream.
- **Diff editor:** not available.
- **Long lines:** very long lines, such as minified files, are not virtualized horizontally yet.

## Platform

- **macOS:** the primary platform.
- **Linux and Windows:** build in CI, but get less testing.
- **Keybindings:** follow Monaco's per-platform defaults (Cmd on macOS, Ctrl elsewhere).
