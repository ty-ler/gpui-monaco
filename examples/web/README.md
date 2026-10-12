# gpui-monaco on the web

The editor in the browser through gpui's web platform (`gpui-pre-web`, which
`gpui_platform` uses on wasm), showing the playground's sample
(`examples/playground/samples/sample.rs`). It draws with WebGPU (WebGL as a
fallback) into a full-page canvas, single-threaded.

## Requirements

gpui's web platform needs a **nightly** toolchain: it depends on `wasm_thread`,
which uses `#![feature(stdarch_wasm_atomic_wait)]`. gpui-component builds its web
examples the same way (`cargo +nightly`). The single-threaded build used here
needs no atomics, `build-std` or COOP/COEP headers.

1. A nightly toolchain with the wasm target and LLVM's tools:
   `rustup toolchain install nightly --target wasm32-unknown-unknown --component llvm-tools`.
   The tools provide the `llvm-ar` that `script/wasm/ar` uses: macOS's `ar` drops
   the WebAssembly objects of tree-sitter's C code.
2. The `wasm-bindgen` CLI at the same version as the `wasm-bindgen` crate in
   `Cargo.lock`: `cargo install wasm-bindgen-cli --locked --version <x.y.z>`.
   `build.sh` checks the version and prints the command.
3. A monospace TrueType or OpenType font: gpui's web platform ships no fonts
   (browser fonts are only a fallback for emoji). The page loads
   `www/fonts/editor.ttf` and uses the family `JetBrains Mono`; pass another
   family name as `?family=…`.

## Build and run

From the repository root:

```sh
examples/web/build.sh --release --font path/to/JetBrainsMono-Regular.ttf
python3 -m http.server -d examples/web/www
```

Then open <http://localhost:8000>. Without `--release` the wasm is about three
times larger and slower. `--font` only needs passing once; it copies the font to
`www/fonts/editor.ttf` (git-ignored, like the built `www/pkg/`).

`cargo +nightly check -p web --target wasm32-unknown-unknown` checks it compiles
without the CLI. CI's `build-web` job runs `build.sh --release` and uploads
`www/` (without a font) as the `web` artifact.

## Limits

- Nightly only, until gpui's web platform builds on stable.
- The page has to supply the font.
- No language servers: `gpui-monaco-lsp` starts processes, which browsers can't.
