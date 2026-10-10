# gpui-monaco on the web

An editor in the browser through gpui's web platform (`gpui-pre-web`, which
`gpui_platform` uses on wasm).

gpui's web platform needs a **nightly** toolchain: it depends on `wasm_thread`,
which uses `#![feature(stdarch_wasm_atomic_wait)]`. gpui-component builds its web
examples the same way (`cargo +nightly`).

1. Install a nightly toolchain with the wasm target
   (`rustup toolchain install nightly --target wasm32-unknown-unknown`).
2. Install the `wasm-bindgen` CLI at the same version as the `wasm-bindgen`
   crate in `Cargo.lock` (`cargo install wasm-bindgen-cli --version <x.y.z>`).
3. Put a monospace TrueType font at `www/fonts/editor.ttf`. gpui's web platform
   ships no fonts. Pass its family name as `?family=…` if it isn't JetBrains Mono.
4. `./build.sh` (or `./build.sh --release`), then serve `www/` with any static
   server, e.g. `python3 -m http.server -d www`, and open it.

`cargo +nightly check -p web --target wasm32-unknown-unknown` checks it
compiles without the CLI.
