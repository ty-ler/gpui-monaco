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

## Documentation

- **Guide:** [`book/`](book/src/SUMMARY.md). Build it with `mdbook build book`; it covers embedding, models and editors, options, keybindings, themes, languages, decorations, widgets, providers and diagnostics.
- **API docs:** `cargo doc --open -p gpui-monaco`.
- **Monaco parity:** [`docs/MONACO_PARITY.md`](docs/MONACO_PARITY.md) lists what's implemented and links the issue for each gap.
- **Performance:** [`docs/PERFORMANCE.md`](docs/PERFORMANCE.md) has the large-file budget and how to measure it.
- **Playground:** [`examples/playground`](examples/playground/README.md) is a demo app with every feature.

## Using it

gpui-monaco depends on gpui from git at a pinned revision (`f16f9652ec57bf806e65b2a0d51bb92a63644914`), because crates.io's gpui is out of date. It is therefore distributed through git:

```toml
[dependencies]
gpui = { git = "https://github.com/zed-industries/zed", rev = "f16f9652ec57bf806e65b2a0d51bb92a63644914" }
gpui_platform = { git = "https://github.com/zed-industries/zed", rev = "f16f9652ec57bf806e65b2a0d51bb92a63644914", features = ["font-kit"] }
gpui-monaco = { git = "https://github.com/ty-ler/gpui-monaco", tag = "v0.1.0" }
gpui-monaco-languages = { git = "https://github.com/ty-ler/gpui-monaco", tag = "v0.1.0" }
```

Use the same gpui revision. Git dependencies don't inherit their workspace's `[patch]` section, so also copy the `[patch.crates-io]` block from this repo's `Cargo.toml` into your workspace. See [Getting started](book/src/getting-started.md) for a complete example. Rust 1.98 or newer is required.

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
