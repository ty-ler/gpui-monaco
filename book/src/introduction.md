# Introduction

gpui-monaco is a code editor component for [GPUI](https://github.com/zed-industries/zed/tree/main/crates/gpui), Zed's UI framework. It aims to behave like [Monaco](https://microsoft.github.io/monaco-editor/), the editor in VS Code, while exposing an idiomatic Rust and GPUI API.

## Features

- **Editing:**
  - Monaco's cursor and word movement, selection, clipboard and undo/redo.
  - Multi-cursor and column (box) selection.
  - Auto-closing brackets and quotes, auto-indent and bracket matching.
  - Code folding and a find/replace widget.
- **View:**
  - A gutter with line numbers and fold controls, and current-line highlight.
  - Scrollbars with an overview ruler.
  - Word wrap, minimap and sticky scroll.
- **Languages:**
  - Incremental tree-sitter highlighting off the main thread, with injections such as fenced code in Markdown and `<script>` in HTML.
  - VS Code `language-configuration.json` editing rules.
  - Bundled grammars for Rust, JavaScript/TypeScript/TSX, JSON, YAML, TOML, Markdown, HTML and CSS.
- **Themes:** Monaco's `vs` and `vs-dark`, and VS Code color theme JSON files.
- **IDE widgets:** completion, hover, diagnostics, go to definition and signature help. Your app supplies the data through provider traits.
- **Extensibility:**
  - Decorations: backgrounds, underlines, gutter glyphs, and inline text before or after a range.
  - View zones, content widgets and overlay widgets, which can be any GPUI view.
- **Large files:** 100k+ lines stay responsive. See [Performance](performance.md).

## Design

Like Monaco, the API separates the **model** from the **view**:

- A [`TextModel`](model-and-view.md) entity owns the text, language, undo history, decorations and diagnostics.
- Any number of [`CodeEditor`](model-and-view.md) entities can show and edit the same model. Each has its own cursors, scroll position and options.

Behavior follows Monaco closely. Large parts of Monaco's own unit tests are ported as a conformance suite. The API does not mirror Monaco's TypeScript one for one. Positions, for example, are 0-based `PointUtf16` values instead of 1-based `IPosition`. [Differences from Monaco](differences.md) lists where behavior or naming diverges.

The editor also [runs in a browser](web.md) through gpui's web platform, on a nightly toolchain.

## Not there yet

The diff editor's inline view and extras and collaborative editing (CRDT). Each one has a `post-mvp` issue in the [issue tracker](https://github.com/ty-ler/gpui-monaco/issues?q=label%3Apost-mvp).
