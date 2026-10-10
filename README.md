# gpui-monaco

A Monaco-like code editor component for [GPUI](https://github.com/zed-industries/zed/tree/main/crates/gpui).

> **Status:** v0.1, the first release. It is macOS-first; Linux and Windows build but get
> less testing. See the [CHANGELOG](CHANGELOG.md), the [MVP plan](docs/MVP.md) and the
> [post-MVP issues](https://github.com/ty-ler/gpui-monaco/issues?q=label%3Apost-mvp).

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

Diff editor, CRDT collaboration and a browser runner (the crates already compile for wasm32). See the `post-mvp` issues.

## Crates

| Crate | Path | Purpose |
|---|---|---|
| `gpui-monaco-text` | `crates/text` | Rope, coordinates, anchors, edits, undo. No gpui dependency. |
| `gpui-monaco-syntax` | `crates/syntax` | `Highlighter` trait, tree-sitter backend, `LanguageRegistry`. |
| `gpui-monaco` | `crates/gpui-monaco` | `TextModel`, `CodeEditor`, rendering, widgets, providers. |
| `gpui-monaco-languages` | `crates/gpui-monaco-languages` | Feature-flagged grammars and queries. |
| `gpui-monaco-lsp` | `crates/gpui-monaco-lsp` | Bridges language servers to the providers and markers. |
| `playground` | `examples/playground` | Monaco-playground-like demo app. |

## Documentation

- **Guide:** [`book/`](book/src/SUMMARY.md). Build it with `mdbook build book`; it covers embedding, models and editors, options, keybindings, themes, languages, decorations, widgets, providers and diagnostics.
- **API docs:** `cargo doc --open -p gpui-monaco`.
- **Monaco parity:** [`docs/MONACO_PARITY.md`](docs/MONACO_PARITY.md) lists what's implemented and links the issue for each gap.
- **Performance:** [`docs/PERFORMANCE.md`](docs/PERFORMANCE.md) has the large-file budget and how to measure it.
- **Playground:** [`examples/playground`](examples/playground/README.md) is a demo app with every feature.
- **With gpui-component:** [`examples/with-gpui-component`](examples/with-gpui-component/src/main.rs) uses the editor in a [gpui-component](https://github.com/longbridge/gpui-component) window.

## Using it

gpui-monaco depends on gpui through the `gpui-pre` crates.io snapshots of Zed's gpui, pinned to the same snapshot as [gpui-component](https://github.com/longbridge/gpui-component), so an app can use both. It's distributed through git for now:

```toml
[dependencies]
gpui = { package = "gpui-pre", version = "=0.3.8" }
gpui_platform = { package = "gpui-pre-platform", version = "=0.3.8", features = ["font-kit"] }
gpui-monaco = { git = "https://github.com/ty-ler/gpui-monaco", tag = "v0.1.0" }
gpui-monaco-languages = { git = "https://github.com/ty-ler/gpui-monaco", tag = "v0.1.0" }
```

Use the same gpui snapshot; no `[patch]` entries are needed. See [Getting started](book/src/getting-started.md) for a complete example. Rust 1.98 or newer is required.

While the repository is private, cargo's built-in git client can't authenticate. Tell cargo to use the `git` CLI, which uses your GitHub credentials, in `.cargo/config.toml`:

```toml
[net]
git-fetch-with-cli = true
```

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
