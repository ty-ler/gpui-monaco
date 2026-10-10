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
- **Completion:** snippet syntax in `insert_text` is inserted literally; snippets are post-MVP.
- **Word-based suggestions:** there are none. Completions come only from providers.
- **Missing commands:** code actions, rename, formatting, inlay hints, color decorators and links are not built in.
- **Folding:** clicking the `⋯` placeholder doesn't unfold the region, and there is no folding by tree-sitter fold queries yet. Ranges come from indentation and markers.
- **Paste:** pasted text is not reindented (Monaco's `autoIndentOnPaste`).
- **Bracket pair guides:** not drawn yet.
- **Accessibility:** screen reader support is post-MVP. GPUI's accessibility support is still in progress upstream.
- **Diff editor:** not available.
- **Long lines:** very long lines, such as minified files, are not virtualized horizontally yet.

## Platform

- **macOS:** the primary platform.
- **Linux and Windows:** build in CI, but get less testing.
- **Keybindings:** follow Monaco's per-platform defaults (Cmd on macOS, Ctrl elsewhere).
