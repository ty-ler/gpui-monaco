# Getting started

## Installation

gpui-monaco builds against gpui from Zed's git repository at a pinned revision, because the gpui on crates.io is out of date. For the same reason, gpui-monaco is distributed through git for now.

```toml
[dependencies]
gpui-monaco = { git = "https://github.com/ty-ler/gpui-monaco", tag = "v0.1.0" }
# Optional: bundled tree-sitter grammars and language configurations.
gpui-monaco-languages = { git = "https://github.com/ty-ler/gpui-monaco", tag = "v0.1.0" }
```

Your app must use the **same gpui revision** as gpui-monaco; the revision is in gpui-monaco's workspace `Cargo.toml`.

Git dependencies don't inherit their workspace's `[patch]` section, so you also need to copy the `[patch.crates-io]` block from gpui-monaco's `Cargo.toml` into your own workspace.

`gpui-monaco-languages` enables every language by default. To keep fewer grammars, pick features:

```toml
gpui-monaco-languages = { git = "…", tag = "v0.1.0", default-features = false, features = ["lang-rust", "lang-json"] }
```

The language features are `lang-rust`, `lang-js`, `lang-ts` (TypeScript and TSX), `lang-json`, `lang-yaml`, `lang-toml`, `lang-markdown`, `lang-html` and `lang-css`.

## A minimal editor

```rust,ignore
use gpui::{App, AppContext as _, Application, Bounds, WindowBounds, WindowOptions, px, size};
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
