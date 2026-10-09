# gpui-monaco

A Monaco-like code editor component for [GPUI](https://github.com/zed-industries/zed/tree/main/crates/gpui).

> **Status:** pre-alpha, under active development toward the MVP. See the
> [MVP plan](docs/MVP.md) and the [tracking issue](https://github.com/ty-ler/gpui-monaco/issues/61).

## Goals

- **Behaves like Monaco**: keybindings, cursor and selection semantics, auto-closing,
  indentation, find, multi-cursor and IDE widgets match Monaco. Monaco's own unit tests
  are ported as a conformance suite.
- **Idiomatic Rust/GPUI API**: a `TextModel` entity shared by any number of
  `CodeEditor` views; host-supplied providers for completion, hover, diagnostics and more.
- **Large files**: 100k+ lines stay smooth (rope on `sum_tree`, viewport-only layout,
  incremental tree-sitter parsing off the main thread).
- **Permissive license**: MIT OR Apache-2.0.

## Non-goals (for the MVP)

Diff editor, screen reader support, a built-in LSP client, snippets, CRDT collaboration,
wasm, and huge single lines (minified files). See the `post-mvp` issues.

## Crates

| Crate | Path | Purpose |
|---|---|---|
| `gpui-monaco-text` | `crates/text` | Rope, coordinates, anchors, edits, undo. No gpui dependency. |
| `gpui-monaco-syntax` | `crates/syntax` | `Highlighter` trait, tree-sitter backend, `LanguageRegistry`. |
| `gpui-monaco` | `crates/gpui-monaco` | `TextModel`, `CodeEditor`, rendering, widgets, providers. |
| `gpui-monaco-languages` | `crates/gpui-monaco-languages` | Feature-flagged grammars and queries. |
| `playground` | `examples/playground` | Monaco-playground-like demo app. |

## Using it

gpui-monaco depends on gpui from git (crates.io's gpui is out of date), so it is
distributed via git for now:

```toml
[dependencies]
gpui-monaco = { git = "https://github.com/ty-ler/gpui-monaco" }
```

Because git dependencies do not inherit their workspace's `[patch]` section, copy the
`[patch.crates-io]` block from this repo's `Cargo.toml` into your workspace.

## Development

```bash
cargo test --workspace
cargo run -p playground
# Offscreen screenshot of the playground (no screen-recording permission needed):
PLAYGROUND_SCREENSHOT=/tmp/pg.png cargo run -p playground --features screenshot
```

## License

Licensed under either of [Apache License, Version 2.0](LICENSE-APACHE) or
[MIT license](LICENSE-MIT) at your option.
