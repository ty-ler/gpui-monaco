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
