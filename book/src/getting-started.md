# Getting started

## Installation

gpui-monaco builds against gpui from Zed's git repository at a pinned revision, because the gpui on crates.io is out of date. For the same reason, gpui-monaco is distributed through git for now.

Add gpui at the **same revision** gpui-monaco uses, plus the editor crates:

```toml
[dependencies]
gpui = { git = "https://github.com/zed-industries/zed", rev = "f16f9652ec57bf806e65b2a0d51bb92a63644914" }
gpui_platform = { git = "https://github.com/zed-industries/zed", rev = "f16f9652ec57bf806e65b2a0d51bb92a63644914", features = ["font-kit"] }
gpui-monaco = { git = "https://github.com/ty-ler/gpui-monaco", tag = "v0.1.0" }
# Optional: bundled tree-sitter grammars and language configurations.
gpui-monaco-languages = { git = "https://github.com/ty-ler/gpui-monaco", tag = "v0.1.0" }
```

Git dependencies don't inherit their workspace's `[patch]` section, so also copy gpui's patches into your workspace root `Cargo.toml`. These are the ones for v0.1.0:

```toml
[patch.crates-io]
async-task = { git = "https://github.com/zed-industries/async-task.git", rev = "015845a19fc31752c02367567d3b8900a406f9da" }
calloop = { git = "https://github.com/zed-industries/calloop", rev = "3759371fee14c40066d64777e2a36b6ffcc22590" }
khronos-egl = { git = "https://github.com/zed-industries/khronos-egl", rev = "42214b36347b108fd9630ca6254b96dde59e04fe" }
```

You need Rust 1.98 or newer, the toolchain pinned in the repository's `rust-toolchain.toml`.

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
