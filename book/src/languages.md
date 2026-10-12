# Languages

A language is a `Language` in a `LanguageRegistry`. It has:

- an id, such as `rust`
- aliases, file extensions and file names
- editing rules from VS Code's `language-configuration.json`
- optionally, a tree-sitter `Grammar` with queries

The grammar only adds highlighting and tree-based features. A language registered with just a configuration still gets auto-closing, surround, bracket matching, comment toggling, its word pattern and folding markers, as in Monaco.

## Plain text

Models without a language, or with one that isn't registered, are plain text: `model.language()` is Monaco's `plaintext` language (`Language::plain_text()`), the registry's if it has one (`gpui_monaco_languages::registry()` does), else a built-in copy. Its configuration (`LanguageConfiguration::plain_text()`) is Monaco's:

- the brackets `()`, `[]` and `{}`, which auto-close and are matched, and which Enter splits onto an indented middle line;
- surrounding pairs for the brackets, `<>` and the three quotes, so typing `"` over a selection wraps it (but doesn't auto-close);
- no bracket pair colorization, and off-side indentation folding.

A model without a language has the id `plaintext` too, as Monaco's `getLanguageId()`: `language_id()` is `plaintext`, so providers registered for `plaintext` and `editorLangId == plaintext` apply to it. Provider selectors also see its `resource()`, `inmemory://model/N` without a uri, so a `{ scheme: "inmemory" }` filter matches models without one.

`set_language_configuration(cx, "plaintext", config)` changes it for every plain text model, when the registry has `plaintext`. A model with an unregistered id keeps that id in `language_id()`, where Monaco reports `plaintext`, so it takes the language once it's registered.

### When the language is resolved

As Monaco's `TextModel` constructor does, a model resolves its language when it's created, once `gpui_monaco::init` has run, and in `create_model` and `register_model` otherwise: `language_id()` and `language()` are set, a language guessed from the uri is picked, and `on_language` callbacks run. The model is parsed for highlighting only once an editor shows it (or `ensure_syntax` is called). Besides an id or a MIME type, a model takes a language's name, alias or extension (`"Rust"`, `"rs"`), and its `language_id()` is then the language's id, `rust`; Monaco gives `plaintext` for those.

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
- `autoClosingPairs`, `autoCloseBefore` and `surroundingPairs`: auto-closing and auto-surround. Without `autoClosingPairs` the brackets auto-close; an empty list closes nothing. The deprecated `__electricCharacterSupport.docComment` auto-closes too, as in Monaco.
- `indentationRules` and `onEnterRules`: auto-indent on Enter, typing and paste.
- `wordPattern`: word movement, double-click and completion words.
- `folding.markers`: region markers, added to syntax folding ranges too.
- `comments`: which tokens count as comments, for auto-close and quick suggestions.
- `folding.markers` and `folding.offSide`: indentation-based folding with `#region` markers.

Every key is optional (`None` in `LanguageConfiguration`), so a configuration can be layered over another: `combine` takes each field from the newer one when it sets it, as Monaco's `combineLanguageConfigurations`. Read the lists with the `brackets()`, `auto_closing_pairs()`, `surrounding_pairs()`, `on_enter_rules()` and `comments()` accessors.

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

### File associations

A model created with `with_uri` and no language gets one from the uri when it's created, as Monaco's `createModel` does (`LanguageRegistry::language_for_path_or_first_line`), and `create_model` without a uri or a language guesses from the first line alone:

1. an exact file name (`with_file_names`), ignoring case;
2. the longest matching `with_filename_patterns` glob, matched against the whole path when it contains a `/`;
3. the longest matching extension, so `.d.ts` beats `.ts`;
4. the first line's `with_first_line` regex, e.g. `^#!.*\bnode`.

Among equal matches, the language registered last wins. `with_mimetypes` sets MIME types for `language_for_mime_type`; a registration without any gets `text/x-<id>`, as in Monaco, and when two languages claim a MIME type the one registered last wins. Models (`with_language`, `set_language`) and `colorize` take a MIME type wherever they take a language id. The bundled languages use VS Code's associations.

### At runtime

- `gpui_monaco::set_language_configuration(cx, "rust", config)` layers a configuration over a language's (Monaco `setLanguageConfiguration`): the fields it sets replace the language's, the others stay, and later ones win. Models of that language pick it up and emit `TextModelEvent::LanguageConfigurationChanged`, and regions embedding it (JavaScript in an HTML `<script>`, a Markdown code fence) use it at once, since the configuration is looked up by language id at each use, as in Monaco. It fails for an unknown language, and disposing the returned `Disposable` removes it again.
- `gpui_monaco::on_language(cx, "rust", |cx| …)` runs once, the first time a model uses the language (Monaco `onLanguage`), for example to register providers lazily. `on_language_encountered` (Monaco `onLanguageEncountered`) also runs when the language is met inside another, like a Monarch grammar's `nextEmbedded`: the place to set an embedded language's tokenizer lazily. Both return a `Disposable` that cancels them.
- `gpui_monaco::register_language(cx, language)` registers a language after models exist (Monaco `languages.register`), and updates them as Monaco's language service does: a model created with the language's id or MIME type before it was registered, or whose uri now guesses it, takes the language (emitting `TextModelEvent::LanguageChanged` when its id changes, `LanguageConfigurationChanged` otherwise), and highlights with its grammar. Models don't notice languages added to the `LanguageRegistry` directly; `set_language_registry` also updates every model.
- Registering a language whose id is already registered merges into it, as Monaco's `languages.register`: its extensions, file names, patterns, aliases and MIME types are added, and its grammar, highlighter, name and configuration fields replace the existing ones only where it has them.

### Highlighting rules

Highlighting follows tree-sitter-highlight semantics: within a layer, the **last** matching pattern for a node wins. That's why queries list generic patterns such as `(identifier) @variable` first.

### Injections

An injections query (`with_injections_query`) embeds other languages. It supports `injection.language`, `injection.content`, `injection.combined` and `injection.include-children`. Examples include Markdown code fences, HTML `<script>`, and Rust macro token trees.

Parsing runs on a background thread and reparses incrementally after edits. Until a parse finishes, the previous highlights are shifted through the new edits.

### Monarch grammars

Monaco's Monarch grammars work unchanged, as JSON with regexes as strings (each regex backslash doubled). `gpui_monaco::set_monarch_tokens_provider` (Monaco `setMonarchTokensProvider`) highlights a language with one, replacing its tree-sitter highlighting, and models of the language re-tokenize. As in Monaco, the language needn't be registered yet: the grammar applies once it is. `Language::with_monarch_grammar` does the same when building a language:

```rust,ignore
use serde_json::json;

let grammar = json!({
    "defaultToken": "invalid",
    "keywords": ["fn", "let", "return"],
    "tokenizer": {
        "root": [
            ["[a-z_]\\w*", { "cases": { "@keywords": "keyword", "@default": "identifier" } }],
            ["\\d+", "number"],
            ["\"", "string", "@string"],
            ["//.*$", "comment"],
            ["[{}()\\[\\]]", "@brackets"],
            ["[ \\t]+", "white"],
        ],
        "string": [
            ["[^\\\\\"]+", "string"],
            ["\\\\.", "string.escape"],
            ["\"", "string", "@pop"],
        ],
    },
});
// Returns a `Disposable`; disposing it restores the language's own
// highlighting (not a tokenizer this one replaced).
let provider = gpui_monaco::set_monarch_tokens_provider(cx, "mylang", &grammar)?;
```

Tokens get the language as a suffix (`keyword.mylang`), and themes match them as Monaco does: the longest Monaco rule (`keyword.mylang`, then `keyword`), then VS Code `tokenColors` matching the token as a scope. `colorize` and `tokenize` give Monarch languages' own tokens. Everything Monarch has is supported, including `cases`, groups, `@rematch`, `$S2`-style state substitutions and `nextEmbedded` (other Monarch languages in the registry, by id, name, alias or MIME type).

Regexes are JavaScript sources run by `fancy-regex`, translated so `\d`, `\w`, `\s` and `.` keep their JavaScript meaning; lookaround and backreferences work. `\b` uses Unicode word characters, matching is by code point rather than UTF-16 code unit, and octal escapes aren't supported. A Monarch grammar tokenizes line by line, keeping each line's end state, and re-tokenizes after an edit until a line ends in the same state as before.

### Tokens providers

Monaco's "manual" tokenizers work too: implement `syntax::TokensProvider` (Monaco `TokensProvider`), which tokenizes one line at a time from the state the previous line ended in, and set it with `gpui_monaco::set_tokens_provider` (Monaco `setTokensProvider`). Each `Token` has a byte offset in the line (Monaco's `startIndex` counts UTF-16 units) and a Monaco token type that the theme matches like a Monarch token. Edits re-tokenize from the first changed line until a line ends in the same state as before, so make the state cheap to compare.

```rust,ignore
use gpui_monaco::syntax::{LineTokens, Token, TokensProvider};

struct Comments;

impl TokensProvider for Comments {
    type State = bool; // inside a `/* */` comment

    fn initial_state(&self) -> bool {
        false
    }

    fn tokenize(&self, line: &str, in_comment: &bool) -> LineTokens<bool> {
        // ...
        LineTokens { tokens: vec![Token::new(0, "comment")], end_state: *in_comment }
    }
}

let provider = gpui_monaco::set_tokens_provider(cx, "mylang", Comments)?;
```

`register_tokens_provider_factory` (Monaco `registerTokensProviderFactory`) creates the tokenizer when the language is first needed, by a model or as an embedded language, from a task resolving to a `LanguageTokenizer` (a tokens provider, any highlighter, or a Monarch grammar); that also covers Monaco's providers given as a promise. A language has one factory, as in Monaco: registering another disposes the previous one, which then isn't called (or, if it was, its tokenizer is removed). As in Monaco, a factory isn't called when the language already has a tokenizer (from `set_monarch_tokens_provider`, `set_tokens_provider` or `with_highlighter`); a tree-sitter grammar doesn't count, so a factory can replace it. Like `set_monarch_tokens_provider`, both return a `Disposable` that restores the previous highlighting. Monaco's `EncodedTokensProvider`, which returns tokens with encoded color indices, has no equivalent.

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

Spans carry `HighlightId`s, which are capture-style names such as `keyword` and `string`, and the theme maps them to colors. Bracket matching and auto-closing find strings, comments and regexes by Monaco's rule: the first whole word (between dots or other non-word characters) that is `comment`, `string`, `regex` or `regexp`, so `string.regexp` and `invalid.string` are strings and `meta.comment` is a comment. A tree-sitter `character` capture counts as a string.

A grammar can also have a `tags.scm` query (`Grammar::with_tags_query`, tree-sitter's code navigation format: `@definition.<kind>` with `@name`). It gives the document symbols used by go to symbol when no provider is registered.
