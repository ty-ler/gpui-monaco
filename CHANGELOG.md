# Changelog

All notable changes to this project are documented here. The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and the project uses [Semantic Versioning](https://semver.org/). Until 1.0, minor versions may break the API.

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
