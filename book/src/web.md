# Running in a browser

The editor runs in a browser through gpui's web platform (`gpui_platform` on
`wasm32-unknown-unknown`). [`examples/web`](https://github.com/ty-ler/gpui-monaco/tree/main/examples/web)
is a runner that opens the playground's sample in a full-page canvas.

## Try it

You need a nightly toolchain (gpui's web platform depends on `wasm_thread`, which
uses a nightly feature), the `wasm-bindgen` CLI at the version in `Cargo.lock`,
and a monospace font file:

```sh
rustup toolchain install nightly --target wasm32-unknown-unknown --component llvm-tools
cargo install wasm-bindgen-cli --locked --version <the wasm-bindgen version in Cargo.lock>
examples/web/build.sh --release --font path/to/JetBrainsMono-Regular.ttf
python3 -m http.server -d examples/web/www
```

Then open <http://localhost:8000>. The page uses the font's family
`JetBrains Mono`; for another font, add `?family=<its family name>` to the URL.

## Embedding

On the web, start gpui from a `#[wasm_bindgen]` function instead of `main`, and
register a font before opening the editor, since the web platform ships none:

```rust,ignore
#[wasm_bindgen]
pub fn run(font: Vec<u8>, family: String) {
    gpui_platform::web_init();
    let handle = gpui_platform::single_threaded_web().run_embedded(move |cx: &mut App| {
        cx.text_system().add_fonts(vec![Cow::Owned(font)]).unwrap();
        gpui_monaco::init(cx);
        gpui_monaco::set_language_registry(cx, gpui_monaco_languages::registry());
        cx.open_window(WindowOptions::default(), |window, cx| {
            let model = cx.new(|_| TextModel::new("fn main() {}\n").with_language("rust"));
            cx.new(|cx| {
                let mut editor = CodeEditor::new(model, window, cx);
                let mut options = editor.options().clone();
                options.font_family = family.into();
                editor.update_options(options, cx);
                editor
            })
        })
        .unwrap();
    });
    // Keep `handle` alive, e.g. in a thread_local, after `run` returns.
}
```

## Limits

- Nightly only, until gpui's web platform builds on stable.
- The page supplies the font; browser fonts are used only as a fallback for emoji.
- No language servers: `gpui-monaco-lsp` starts processes, which browsers can't.
