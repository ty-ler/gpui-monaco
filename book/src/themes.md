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
