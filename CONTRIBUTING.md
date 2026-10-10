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

`gpui`, `gpui_platform` and `sum_tree` are pinned to the same upstream zed rev in the
workspace `Cargo.toml`. To bump:

1. Pick a new rev from `zed-industries/zed` `main`.
2. Update all three `rev = ...` entries.
3. Copy the `[patch.crates-io]` entries that gpui needs from zed's `Cargo.toml` at that
   rev (tree-sitter-language, async-task, calloop, khronos-egl; check for new ones).
4. Match `rust-toolchain.toml` to zed's `channel` at that rev.
5. `cargo update -p gpui && cargo test --workspace && cargo run -p playground`.

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
