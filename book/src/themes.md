# Themes

A `Theme` combines two things:

- **`EditorColors`:** background, gutter, selection, widgets, scrollbar and so on. These are VS Code `colors` keys such as `editor.background`.
- **Token rules:** each rule maps a token or scope selector to a foreground color and font style.

The editor applies themes per instance:

```rust,ignore
editor.update(cx, |editor, cx| editor.set_theme(Arc::new(Theme::vs()), cx));
```

## Built-in themes

- `Theme::vs_dark()`: Monaco's `vs-dark` (the default)
- `Theme::vs()`: Monaco's `vs`
- `Theme::hc_black()` and `Theme::hc_light()`: Monaco's high-contrast `hc-black` and `hc-light`

## Themes by name

As in Monaco, themes can also be set for every editor by name. `set_theme` switches every editor (including those in diff editors) and the ones created later; an unknown name gets `vs`. `define_theme` adds a theme for it, or replaces one; replacing the current theme updates the editors.

```rust,ignore
// Names are ASCII letters, digits and `-`, as in Monaco.
let mut theme = Theme::from_vscode_file(Path::new("themes/dark_modern.json"))?;
theme.name = "dark-modern".into();
gpui_monaco::define_theme(cx, theme)?;
gpui_monaco::set_theme(cx, "dark-modern");
```

`theme_named` looks one up (`vs`, `vs-dark`, `hc-black`, `hc-light` are built in), and `global_theme` is the current one.

"Toggle High Contrast Theme" (`editor.action.toggleHighContrast`) switches every editor to `hc-black` or `hc-light`, whichever matches the current theme's kind, and back to the theme it replaced. `Theme::high_contrast` says whether a theme is one (VS Code themes of `type` `hc` or `hcLight` are).

`EditorColors::symbol_icons` holds Monaco's `symbolIcon.*Foreground` colors for the suggest widget's and quick outline's icons; most default to the theme's `foreground`. `banner_background`, `banner_foreground` and `banner_icon_foreground` (`banner.*`) color banners such as the unicode highlighter's.

## VS Code themes

Any VS Code color theme JSON works, with or without comments. Its `colors` set the editor colors, and its `tokenColors` are matched against TextMate scopes. Some themes `include` others; for example, Dark Modern includes Dark+, which includes Dark (Visual Studio).

```rust,ignore
// Follows `include` relative to the file.
let theme = Theme::from_vscode_file(Path::new("themes/dark_modern.json"))?;

// From strings: resolve includes yourself (e.g. from `include_str!`s).
let theme = Theme::from_vscode_json_with_includes(json, &|include| {
    bundled.get(include.trim_start_matches("./")).map(|s| s.to_string())
})?;
```

VS Code's bundled themes keep `"type"` (light or dark) in the extension manifest, not in the theme file. When a file doesn't set `type`, gpui-monaco decides from the lightness of `editor.background`. Colors the theme doesn't set fall back to `vs` or `vs-dark`.

## How tokens get their color

tree-sitter highlight captures (`keyword`, `string.special`, `function.method`, …) are mapped to the names Monaco's themes use and to TextMate scopes. The most specific matching rule wins. This lets one theme color both Monaco-style rules (`keyword`, `string.key.json`) and VS Code `tokenColors` (`entity.name.function`, `storage.type`).

Semantic tokens from a [provider](providers.md#semantic-tokens) are colored by the theme's `semanticTokenColors` (VS Code's selector scoring: `type.modifier:language`, `*.modifier`). If none matches, VS Code's default TextMate scope for the token type is matched against `tokenColors`. A theme's `semanticHighlighting` turns them on unless the editor's `semantic_highlighting` option says otherwise.

## Highlighting code outside an editor

`gpui_monaco::colorize` highlights text without an editor, as Monaco's `monaco.editor.colorize` and `tokenize` do. Use it for docs, tooltips or any other host UI:

```rust
use gpui_monaco::colorize::{ColorizeOptions, colorize_element, tokenize};

let element = colorize_element(code, "rust", &theme, &registry, ColorizeOptions::default());
let tokens = tokenize(code, "rust", &registry); // per line: offset, `keyword.rust`, ...
```

`colorize` returns the lines (tabs expanded) with their highlights, and `colorize_highlights` gives the highlights for a `StyledText`. `colorize_model_line(model, row, tab_size, &theme, cx)` (Monaco `colorizeModelLine`) colors a line of a model with the model's own syntax, without waiting for a background parse.

## Custom themes

`Theme::define` is Monaco's `defineTheme`. It takes:

- a base theme
- whether to inherit the base's rules
- extra rules
- VS Code color keys

```rust,ignore
let theme = Theme::define(
    "my-dark",
    &Theme::vs_dark(),
    true,
    vec![TokenRule {
        kind: RuleKind::Monaco,
        selector: "comment".into(),
        style: TokenStyle { foreground: Some(rgb(0x6a9955).into()), italic: true, ..Default::default() },
    }],
    &HashMap::from([("editor.background".into(), "#1b1b1f".into())]),
);
```
