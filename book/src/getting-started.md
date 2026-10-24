# Getting started

## Installation

gpui-monaco builds against gpui from the `gpui-pre` crates, weekly crates.io snapshots of Zed's gpui. It pins the same snapshot as [gpui-component](https://github.com/longbridge/gpui-component), so an app can use both in one window. gpui-monaco itself is distributed through git for now.

Add gpui at the **same snapshot** gpui-monaco uses, plus the editor crates:

```toml
[dependencies]
gpui = { package = "gpui-pre", version = "=0.3.8" }
gpui_platform = { package = "gpui-pre-platform", version = "=0.3.8", features = ["font-kit"] }
gpui-monaco = { git = "https://github.com/ty-ler/gpui-monaco", tag = "v0.1.0" }
# Optional: bundled tree-sitter grammars and language configurations.
gpui-monaco-languages = { git = "https://github.com/ty-ler/gpui-monaco", tag = "v0.1.0" }
```

You need Rust 1.98 or newer, the toolchain pinned in the repository's `rust-toolchain.toml`.

While the repository is private, cargo's built-in git client can't authenticate. Tell cargo to use the `git` CLI, which uses your GitHub credentials, in `.cargo/config.toml`:

```toml
[net]
git-fetch-with-cli = true
```

`gpui-monaco-languages` enables every language by default. To keep fewer grammars, pick features:

```toml
gpui-monaco-languages = { git = "…", tag = "v0.1.0", default-features = false, features = ["lang-rust", "lang-json"] }
```

The language features are `lang-rust`, `lang-js`, `lang-ts` (TypeScript and TSX), `lang-json`, `lang-yaml`, `lang-toml`, `lang-markdown`, `lang-html` and `lang-css`.

`gpui-monaco` has one feature, `word-segmenter` (on by default). It adds ICU4X's word segmenter and its compiled data (about 2.5 MB) for the `word_segmenter_locales` [option](options.md), which finds words in Japanese, Chinese, Thai and other languages written without spaces. To leave it out:

```toml
gpui-monaco = { git = "…", tag = "v0.1.0", default-features = false }
```

### Icons and licensing

Monaco's widgets draw their icons with Microsoft's [codicon](https://github.com/microsoft/vscode-codicons) font. In gpui-monaco the font is optional, behind the `codicons` feature (off by default):

```toml
gpui-monaco = { git = "…", tag = "v0.1.0", features = ["codicons"] }
```

With it, `gpui_monaco::init` registers the font (about 150 KB, from the `gpui-monaco-codicons` crate) with GPUI's text system and the suggest list, find widget, lightbulb, folding chevrons, peek view and other widgets show Monaco's icons. Without it they draw similar Unicode characters. Either way, `gpui_monaco::codicons` has every Monaco icon id (`Codicon::SymbolMethod`, `Codicon::from_id("symbol-method")`), the `icon(Codicon)` element for host UI and `codicons_available(cx)`.

`gpui-monaco` is MIT, but the font is © Microsoft Corporation under [CC-BY-4.0](https://creativecommons.org/licenses/by/4.0/), which allows redistribution, including in commercial apps, with attribution. If you enable `codicons`, ship the notice from the `gpui-monaco-codicons` crate's `NOTICE` (or credit "Codicons © Microsoft Corporation, CC-BY-4.0") with your app's third-party notices. That crate's copy adds an `m` to the font's character map, since GPUI's macOS text system won't load a font without one.

Without it, `word_segmenter_locales` is accepted but has no effect.

## A minimal editor

```rust,ignore
use gpui::{App, AppContext as _, Bounds, WindowBounds, WindowOptions, px, size};
use gpui_monaco::{CodeEditor, TextModel};

fn main() {
    gpui_platform::application().run(|cx: &mut App| {
        // Monaco's default keybindings, in the `CodeEditor` key context.
        gpui_monaco::init(cx);
        // Languages, used for highlighting, brackets and indentation rules.
        gpui_monaco::set_language_registry(cx, gpui_monaco_languages::registry());

        let bounds = Bounds::centered(None, size(px(900.), px(600.)), cx);
        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                ..Default::default()
            },
            |window, cx| {
                let model = cx.new(|_| TextModel::new("fn main() {}\n").with_language("rust"));
                cx.new(|cx| CodeEditor::new(model, window, cx))
            },
        )
        .unwrap();
        cx.activate(true);
    });
}
```

`CodeEditor` is a GPUI view, so you can render it as a child of any element:

```rust,ignore
div().size_full().child(self.editor.clone())
```

To type into the editor, it must have focus:

```rust,ignore
window.focus(&editor.focus_handle(cx), cx);
```

## The playground

The repository's `examples/playground` app shows every feature:

- open files and switch languages
- switch themes, including the bundled VS Code themes
- toggle options
- demo providers, decorations and widgets

```sh
cargo run -p playground
```

Its source is a practical reference for wiring things together.

## With gpui-component

gpui-monaco and [gpui-component](https://github.com/longbridge/gpui-component) build against the same `gpui-pre` snapshot, so an app can use both in one window. Depend on the gpui-component release that pins the same snapshot (0.7.1 for `gpui-pre =0.3.8`), and initialize both:

```rust,ignore
gpui_component::init(cx);
gpui_monaco::init(cx);
// A gpui-component window: its Root wraps your view, which can hold editors.
cx.open_window(options, |window, cx| {
    let view = cx.new(|cx| MyView::new(window, cx));
    cx.new(|cx| gpui_component::Root::new(view, window, cx))
});
```

The editor keeps its own keybindings, focus and input inside gpui-component's `Root`. It also keeps its own Monaco theme: set one with `CodeEditor::set_theme` to match gpui-component's light or dark mode. [`examples/with-gpui-component`](https://github.com/ty-ler/gpui-monaco/tree/main/examples/with-gpui-component) is a complete window with gpui-component buttons above an editor.

