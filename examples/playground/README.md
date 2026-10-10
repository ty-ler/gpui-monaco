# Playground

A Monaco-playground-style app for trying every editor feature.

```sh
cargo run -p playground
```

The toolbar has these controls:

- **Open…** opens a file. Its language is picked from the file name.
- **Language** cycles through the registered languages.
- **Theme** cycles through `vs-dark`, `vs` and the bundled VS Code themes (see `themes/`).
- **Wrap, Minimap, Sticky scroll, Line numbers, Folding** and **Read-only** toggle editor options. `// #region` and `// MARK:` comments show as section headers in the minimap.
- **Add demo widgets** adds sample decorations, a view zone, a content widget and an overlay widget.

Clicking the glyph margin, left of the line numbers, toggles a breakpoint dot. It uses the editor's mouse target events.

Colors such as `#ff0000` or `rgb(…)` get a swatch in any language, from the built-in color finder. Hovering a color or clicking its swatch opens the color picker.

The status bar shows the cursor position, selection count, language, line endings and file.

Demo providers are registered by default:

- hover on `Edit` / `Patch`
- completions (`Ctrl+Space`; press it again for details) with docs, label details, a deprecated item, one resolved lazily, and two snippets (`for` with tab stops, `derive` with a choice)
- signature help inside `(`
- go to definition (`F12` / Cmd+click)
- command palette (`F1`), go to line (`Ctrl+G`) and go to symbol (`Cmd+Shift+O`)
- references in a peek view (`Shift+F12`): every whole-word occurrence
- rename (`F2`) of every whole-word occurrence
- code actions: a lightbulb and `Cmd+.` with a quick fix per diagnostic and an UPPERCASE rewrite
- diagnostics (`F8`)
- a formatter that trims trailing spaces and collapses repeated ones. Use format document (`Shift+Alt+F`) or format selection (`Cmd+K Cmd+F`). It also runs on paste and when you type `;` or `}`.

## Environment variables

| Variable | Effect |
|---|---|
| `PLAYGROUND_FILE=path` | Open `path` instead of the sample |
| `PLAYGROUND_THEME=name\|path.json` | Start with a bundled theme (`Light+`, `Dark+`, …) or a VS Code theme file |
| `PLAYGROUND_WRAP=on\|<column>` | Start with word wrap on, or wrapped at a column |
| `PLAYGROUND_DEMO=1` | Add the demo decorations and widgets at startup |
| `PLAYGROUND_PLAIN=1` | Don't register the demo providers or markers |
| `PLAYGROUND_LSP="rust-analyzer"` | Start a language server for `PLAYGROUND_FILE`, rooted at the nearest project directory, instead of the demo providers |
| `PLAYGROUND_FONT="JetBrains Mono"` | Use another font family |
| `PLAYGROUND_LIGATURES=1` | Turn font ligatures on |
| `PLAYGROUND_ZOOM=2` | Start at a font zoom level (Cmd+wheel zooms too) |
| `PLAYGROUND_DEBUG=1` | Log selection changes |
| `PLAYGROUND_SCREENSHOT=out.png` | Save a screenshot and exit (needs `--features screenshot`) |
| `PLAYGROUND_KEYS="cmd-down text:hi wait"` | Keystrokes to play before the screenshot (`text:` types, `_` is a space, `wait` pauses 150ms) |
