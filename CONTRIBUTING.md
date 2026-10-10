# Contributing

## Licensing rules

gpui-monaco is MIT OR Apache-2.0. Keep it that way:

- **Do not copy code from Zed's GPL crates** (`rope`, `text`, `language`, `multi_buffer`,
  `editor`, and anything else under GPL-3.0 in the zed repo). Reading them for ideas is
  fine; copying or close paraphrase is not.
- Zed's `gpui` and `sum_tree` crates are Apache-2.0 and are used as dependencies.
- VS Code / Monaco is MIT. Porting its algorithms and tests is fine; keep a comment
  pointing at the source file you ported from.
- Tree-sitter grammars and queries must be permissively licensed (MIT/Apache/BSD-like).

## Monaco source

`script/fetch-monaco-source` sparse-clones VS Code's `src/vs/editor` and
`src/vs/base/common` into the gitignored `vendor/vscode` for reference and test porting.

## Bumping gpui

`gpui`, `gpui_platform` and `sum_tree` come from the `gpui-pre`, `gpui-pre-platform`
and `gpui-pre-sum-tree` crates: weekly crates.io snapshots of zed's crates, which
gpui-component also uses. They're pinned exactly (`=0.3.8`), because any snapshot may
change gpui's API. To bump:

1. Prefer the snapshot gpui-component pins (its workspace `Cargo.toml`), so apps can
   use both.
2. Update all three `version = "=…"` entries together.
3. Match `rust-toolchain.toml` to zed's `channel` at the snapshot's commit (in the
   crate description, e.g. "gpui-pre snapshot of zed@279fe07").
4. `cargo update -p gpui-pre && cargo test --workspace && cargo run -p playground`.

## Workflow

One branch and PR per issue (`<issue#>-<slug>`); PR body says `Closes #N`. Update the
issue (and `docs/MVP.md` if scope changes) with anything a later issue should know.

## Checks

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

## wasm

The editor crates compile for `wasm32-unknown-unknown` (CI's `check-wasm` job):

```sh
rustup target add wasm32-unknown-unknown
cargo check --target wasm32-unknown-unknown -p gpui-monaco -p gpui-monaco-languages
```

Grammar crates compile their C parsers through `script/wasm/cc` (set in
`.cargo/config.toml`). It uses tree-sitter's minimal libc headers (`script/wasm/include`, MIT,
from the `tree-sitter-language` crate) and a few inline ctype functions. It needs a `clang`
that targets wasm32; Apple's does, and you can choose another with `WASM_CLANG`. Use
`web_time::Instant`, not `std::time::Instant`, which panics in browsers.

## Releasing

The five library crates publish to crates.io; the examples don't (`publish = false`).

1. Bump `version` in the workspace `Cargo.toml`, and the `version` on each internal
   dependency in `[workspace.dependencies]`.
2. Move the CHANGELOG's `[Unreleased]` entries under the new version.
3. Dry run: `cargo package --workspace` packages every crate and builds each one from
   its package, against the others through a temporary registry.
4. Publish in dependency order: `gpui-monaco-text`, `gpui-monaco-syntax`,
   `gpui-monaco`, `gpui-monaco-languages`, `gpui-monaco-lsp`. Or run
   `cargo publish --workspace`, which orders them itself.
5. Tag `vX.Y.Z` and push the tag.
