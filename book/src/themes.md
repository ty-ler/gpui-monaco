# Themes

A `Theme` combines two things:

- **`EditorColors`:** background, gutter, selection, widgets, scrollbar and so on. These are VS Code `colors` keys such as `editor.background`; each field's docs name its key.
- **Token rules:** each rule maps a token or scope selector to a foreground color and font style.

## Colors and their defaults

A theme sets some color keys itself; every other key gets Monaco's default for the theme's kind (dark, light, high contrast dark or high contrast light), from the same color registry Monaco uses. Many defaults derive from other keys: `editorWidget.border` is `editorWidget.foreground` at 20%, `editor.inactiveSelectionBackground` is half of `editor.selectionBackground`, `peekView.border` is `editorInfo.foreground`, `menu.selectionBackground` is `list.activeSelectionBackground`. So a theme that sets `editor.selectionBackground` also moves the inactive selection, the fold background and the minimap's selection. Deprecated keys still work as defaults (`editorActiveLineNumber.foreground`, `editorIndentGuide.background`).

The editor's widgets read their own keys: the context menu `menu.*`, hovers and parameter hints `editorHoverWidget.*` (and the hovered range `editor.hoverHighlightBackground`), the code action list `editorActionList.*`, quick access `quickInput*` and `list.*` with `keybindingLabel.*` keybindings, toolbar buttons `toolbar.*`, find toggles `inputOption.*`, and diagnostics `editorError.*`, `editorWarning.*`, `editorInfo.*`, `editorHint.*`, `editorOverviewRuler.*Foreground` and `editorMarkerNavigation*`. High contrast themes outline find matches, word and selection highlights, range and symbol highlights and the focused list row with `contrastActiveBorder` (`editor.findMatchBorder`, `list.focusOutline`, …) instead of filling them, as Monaco does.

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

`EditorColors::symbol_icons` holds Monaco's `symbolIcon.*Foreground` colors for the suggest widget's and quick outline's icons; most default to the theme's `foreground`. `banner_background`, `banner_foreground` and `banner_icon_foreground` (`banner.*`) color banners such as the unicode highlighter's. `EditorColors::marker_color`, `indent_guide_colors` and `bracket_pair_guide_colors` resolve a marker's or a guide level's color as the editor draws it.

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

VS Code's bundled themes keep `"type"` (light or dark) in the extension manifest, not in the theme file. When a file doesn't set `type`, gpui-monaco decides from the lightness of `editor.background`. Colors the theme doesn't set get the registry defaults for its type, as in VS Code: `light`, `dark`, `hcDark` (or `hc`) or `hcLight`.

## How tokens get their color

tree-sitter highlight captures (`keyword`, `string.special`, `function.method`, …) are mapped to the names Monaco's themes use and to TextMate scopes. This lets one theme color both Monaco-style rules (`keyword`, `string.key.json`) and VS Code `tokenColors` (`entity.name.function`, `storage.type`).

As in Monaco's tokenizers, a capture's Monaco token carries its language: JSON's `true` is `keyword.json`, an HTML attribute value `attribute.value.html`, a YAML string `string.yaml`. So the built-in themes' language rules apply (`keyword.json` is `#0451A5` in `vs`), and so do your own. Where a language's Monaco tokenizer names things differently from the generic mapping, the language's names are used: JSON string values are `string.value`, YAML keys `type`, CSS properties `attribute.name`.

Rules resolve like Monaco's `TokenTheme`. The deepest matching rule wins (`keyword.flow` over `keyword`), and a rule inherits what it leaves unset from its parent's: `comment.doc` with only `fontStyle: "italic"` keeps `comment`'s color, and a color-only rule keeps the parent's bold. `fontStyle: ""` clears an inherited style. A `""` rule (a `tokenColors` entry without a `scope`) sets the default for every token. The same holds for `tokenColors`.

Semantic tokens from a [provider](providers.md#semantic-tokens) are colored by the theme's `semanticTokenColors` (VS Code's selector scoring: `type.modifier:language`, `*.modifier`). If none matches, VS Code's default TextMate scope for the token type is matched against `tokenColors`. Themes with Monaco rules, like the built-in ones, then match `type.modifier` against those, and as in Monaco's standalone themes a token nothing matches gets the default token style, replacing its syntax color. A theme's `semanticHighlighting` turns them on unless the editor's `semantic_highlighting` option says otherwise.

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
- whether to inherit the base's rules and colors
- extra rules
- VS Code color keys

```rust,ignore
let theme = Theme::define(
    "my-dark",
    &Theme::vs_dark(),
    true,
    // `{ token: 'comment', foreground: '6a9955', fontStyle: 'italic' }`
    vec![TokenRule::monaco("comment", Some("6a9955"), Some("italic"))?],
    &HashMap::from([("editor.background".into(), "#1b1b1f".into())]),
);
```

With `inherit`, the base's rules come first and its colors stay unless the new keys change them, directly or through a default derived from one. Without it, as in Monaco, only the new rules and colors apply, over the registry defaults for the base's kind (so `vs` without `inherit` loses `themes.ts`' `#E5EBF1` inactive selection). `editor.foreground` in the colors also becomes the default token color. `TokenRule::monaco` checks colors as Monaco does: `RRGGBB` or `RRGGBBAA` with an optional `#`, the alpha ignored; anything else is an error. Unlike Monaco, any theme can be the base, not only the built-in ones.
