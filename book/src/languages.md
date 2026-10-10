# Languages

A language is a `Language` in a `LanguageRegistry`. It has:

- an id, such as `rust`
- aliases, file extensions and file names
- editing rules from VS Code's `language-configuration.json`
- optionally, a tree-sitter `Grammar` with queries

Install the registry once per app:

```rust,ignore
gpui_monaco::set_language_registry(cx, gpui_monaco_languages::registry());
```

Then give each model a language id, either directly or through the registry's lookup by path:

```rust,ignore
let registry = gpui_monaco::language_registry(cx);
let language = registry
    .language_for_path(Path::new("Cargo.toml"))
    .map(|l| SharedString::from(l.id.to_string()));
let model = cx.new(|_| TextModel::new(&text));
model.update(cx, |model, cx| model.set_language(language, cx));
```

## Bundled languages

The bundled languages and their cargo features:

| Feature | Languages |
|---|---|
| `lang-rust` | Rust |
| `lang-js` | JavaScript, JSX |
| `lang-ts` | TypeScript, TSX |
| `lang-json` | JSON, JSON with Comments |
| `lang-yaml` | YAML |
| `lang-toml` | TOML |
| `lang-markdown` | Markdown, with highlighted fenced code blocks |
| `lang-html` | HTML, with `<script>` and `<style>` |
| `lang-css` | CSS |

Grammars and highlight queries come from the official tree-sitter crates. Editing rules come from VS Code's language configurations. Both are MIT-licensed.

## What the configuration drives

The `language-configuration.json` keys behave as they do in VS Code:

- `brackets`: bracket matching and bracket-based indentation.
- `autoClosingPairs`, `autoCloseBefore` and `surroundingPairs`: auto-closing and auto-surround.
- `indentationRules` and `onEnterRules`: auto-indent on Enter, typing and paste.
- `wordPattern`: word movement, double-click and completion words.
- `folding.markers`: region markers, added to syntax folding ranges too.
- `comments`: which tokens count as comments, for auto-close and quick suggestions.
- `folding.markers` and `folding.offSide`: indentation-based folding with `#region` markers.

Patterns use the `fancy-regex` crate, so VS Code's lookaheads and backreferences work.

- **JavaScript syntax:** syntax that Rust regexes read differently is translated: `[^]`, `[]`, a `[` inside a class, and hyphens next to `\w`-style escapes.
- **Flags:** `i`, `m` and `s` are honored.
- **Errors:** a pattern that still doesn't compile never matches. `Pattern::error()` says why, and `config.patterns()` lists every regex so you can check yours in a test.

## Adding a language

```rust,ignore
use gpui_monaco::syntax::{Grammar, Language, LanguageConfiguration, LanguageRegistry};

let python = Language::new("python", "Python")
    .with_extensions(&["py"])
    .with_config(LanguageConfiguration::from_json(include_str!("python.json"))?)
    .with_grammar(
        Grammar::new(tree_sitter_python::LANGUAGE.into())
            .with_highlights_query(tree_sitter_python::HIGHLIGHTS_QUERY)?,
    );

let registry = gpui_monaco_languages::registry(); // or LanguageRegistry::new()
registry.register(python);
gpui_monaco::set_language_registry(cx, registry);
```

### Highlighting rules

Highlighting follows tree-sitter-highlight semantics: within a layer, the **last** matching pattern for a node wins. That's why queries list generic patterns such as `(identifier) @variable` first.

### Injections

An injections query (`with_injections_query`) embeds other languages. It supports `injection.language`, `injection.content`, `injection.combined` and `injection.include-children`. Examples include Markdown code fences, HTML `<script>`, and Rust macro token trees.

Parsing runs on a background thread and reparses incrementally after edits. Until a parse finishes, the previous highlights are shifted through the new edits.

### Custom highlighters

`syntax::Highlighter` is the pluggable interface behind highlighting, and tree-sitter is one implementation of it. To use another, such as TextMate grammars or LSP semantic tokens, build it with `Language::with_highlighter`:

```rust,ignore
let language = Language::new("mylang", "My Language")
    .with_extensions(&["my"])
    .with_highlighter(|language, registry| Arc::new(MyHighlighter::new(language.clone())));
```

A `Highlighter` has two methods:

- **`update(previous, text) -> HighlightState`:** runs on a background thread after edits. `previous` holds the last state and the edits since it, for incremental work. `HighlightState` wraps any `Send + Sync` value you choose.
- **`highlights(state, text, range)`:** returns sorted, non-overlapping spans for a range. It runs every frame for the visible lines, so it must be fast.

Spans carry `HighlightId`s, which are capture-style names such as `keyword` and `string`, and the theme maps them to colors. Bracket matching and auto-closing use names that start with `string` or `comment` to know what to skip.
