//! Bundled languages for gpui-monaco, each behind a cargo feature
//! (`lang-rust`, `lang-js`, `lang-ts`, `lang-json`, `lang-yaml`, `lang-toml`,
//! `lang-markdown`, `lang-html`, `lang-css`; `all` is on by default).
//!
//! Grammars and highlight queries come from the official tree-sitter grammar
//! crates (MIT). Editing rules come from VS Code's `language-configuration.json`
//! files (MIT), copied into `configs/`.
#![warn(missing_docs)]

use std::sync::Arc;

use syntax::tree_sitter::Language as TsLanguage;
use syntax::{Grammar, Language, LanguageConfiguration, LanguageRegistry};

#[allow(dead_code)]
fn config(json: &str) -> LanguageConfiguration {
    LanguageConfiguration::from_json(json).unwrap_or_else(|error| {
        // These files are bundled; a parse failure is a packaging bug.
        panic!("invalid bundled language configuration: {error}")
    })
}

#[allow(dead_code)]
fn grammar(language: TsLanguage, highlights: &[&str], injections: Option<&str>) -> Grammar {
    let mut grammar = Grammar::new(language)
        .with_highlights_query(&highlights.join("\n"))
        .unwrap_or_else(|error| panic!("invalid bundled highlights query: {error}"));
    if let Some(injections) = injections {
        grammar = grammar
            .with_injections_query(injections)
            .unwrap_or_else(|error| panic!("invalid bundled injections query: {error}"));
    }
    grammar
}

/// Rust (`.rs`), with tree-sitter-rust highlights and injections and VS Code's
/// Rust configuration.
#[cfg(feature = "lang-rust")]
pub fn rust() -> Language {
    Language::new("rust", "Rust")
        .with_aliases(&["rs"])
        .with_extensions(&["rs"])
        .with_config(config(include_str!("../configs/rust.json")))
        .with_grammar(grammar(
            tree_sitter_rust::LANGUAGE.into(),
            &[tree_sitter_rust::HIGHLIGHTS_QUERY],
            Some(tree_sitter_rust::INJECTIONS_QUERY),
        ))
}

/// JavaScript and JSX (`.js`, `.mjs`, `.cjs`, `.jsx`), with tree-sitter-javascript
/// highlights (including JSX) and injections.
#[cfg(feature = "lang-js")]
pub fn javascript() -> Language {
    Language::new("javascript", "JavaScript")
        .with_aliases(&["js", "jsx", "javascriptreact"])
        .with_extensions(&["js", "mjs", "cjs", "jsx"])
        .with_config(config(include_str!("../configs/javascript.json")))
        .with_grammar(grammar(
            tree_sitter_javascript::LANGUAGE.into(),
            &[
                tree_sitter_javascript::HIGHLIGHT_QUERY,
                tree_sitter_javascript::JSX_HIGHLIGHT_QUERY,
            ],
            Some(tree_sitter_javascript::INJECTIONS_QUERY),
        ))
}

/// TypeScript highlighting layers the JavaScript query under the TypeScript
/// one, as tree-sitter-typescript's own `tree-sitter.json` does.
#[cfg(feature = "lang-ts")]
pub fn typescript() -> Language {
    Language::new("typescript", "TypeScript")
        .with_aliases(&["ts"])
        .with_extensions(&["ts", "mts", "cts"])
        .with_config(config(include_str!("../configs/typescript.json")))
        .with_grammar(grammar(
            tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
            &[
                tree_sitter_javascript::HIGHLIGHT_QUERY,
                tree_sitter_typescript::HIGHLIGHTS_QUERY,
            ],
            Some(tree_sitter_javascript::INJECTIONS_QUERY),
        ))
}

/// TypeScript React (`.tsx`), layering the JavaScript and JSX queries under the
/// TypeScript one like [`typescript`].
#[cfg(feature = "lang-ts")]
pub fn tsx() -> Language {
    Language::new("typescriptreact", "TypeScript React")
        .with_aliases(&["tsx"])
        .with_extensions(&["tsx"])
        .with_config(config(include_str!("../configs/typescript.json")))
        .with_grammar(grammar(
            tree_sitter_typescript::LANGUAGE_TSX.into(),
            &[
                tree_sitter_javascript::HIGHLIGHT_QUERY,
                tree_sitter_javascript::JSX_HIGHLIGHT_QUERY,
                tree_sitter_typescript::HIGHLIGHTS_QUERY,
            ],
            Some(tree_sitter_javascript::INJECTIONS_QUERY),
        ))
}

/// JSON, including JSONC and JSON5 files and common `.*rc` config files.
#[cfg(feature = "lang-json")]
pub fn json() -> Language {
    Language::new("json", "JSON")
        .with_aliases(&["jsonc"])
        .with_extensions(&["json", "jsonc", "json5"])
        .with_file_names(&[".prettierrc", ".babelrc", ".eslintrc"])
        .with_config(config(include_str!("../configs/json.json")))
        .with_grammar(grammar(
            tree_sitter_json::LANGUAGE.into(),
            // tree-sitter-json lists the key pattern before `(string) @string`,
            // but the last matching pattern wins, so repeat it at the end.
            &[
                tree_sitter_json::HIGHLIGHTS_QUERY,
                "(pair key: (_) @string.special.key)",
            ],
            None,
        ))
}

/// YAML (`.yaml`, `.yml`).
#[cfg(feature = "lang-yaml")]
pub fn yaml() -> Language {
    Language::new("yaml", "YAML")
        .with_aliases(&["yml"])
        .with_extensions(&["yaml", "yml"])
        .with_config(config(include_str!("../configs/yaml.json")))
        .with_grammar(grammar(
            tree_sitter_yaml::LANGUAGE.into(),
            &[tree_sitter_yaml::HIGHLIGHTS_QUERY],
            None,
        ))
}

/// VS Code has no built-in TOML support; this configuration is our own.
#[cfg(feature = "lang-toml")]
pub fn toml() -> Language {
    Language::new("toml", "TOML")
        .with_extensions(&["toml"])
        .with_file_names(&["Cargo.lock"])
        .with_config(config(include_str!("../configs/toml.json")))
        .with_grammar(grammar(
            tree_sitter_toml_ng::LANGUAGE.into(),
            // Upstream captures the whole `pair` as @property; capture the key.
            &[
                tree_sitter_toml_ng::HIGHLIGHTS_QUERY,
                "(pair (bare_key) @property)",
            ],
            None,
        ))
}

/// Markdown's block grammar. Paragraph text is injected as [`markdown_inline`],
/// fenced code blocks as the language named by their info string, and HTML
/// blocks and front matter as HTML, YAML or TOML.
#[cfg(feature = "lang-markdown")]
pub fn markdown() -> Language {
    Language::new("markdown", "Markdown")
        .with_aliases(&["md"])
        .with_extensions(&["md", "markdown", "mdown", "mkd"])
        .with_config(config(include_str!("../configs/markdown.json")))
        .with_grammar(grammar(
            tree_sitter_md::LANGUAGE.into(),
            &[tree_sitter_md::HIGHLIGHT_QUERY_BLOCK],
            Some(tree_sitter_md::INJECTION_QUERY_BLOCK),
        ))
}

/// Markdown's inline grammar, injected by [`markdown`] into paragraphs.
#[cfg(feature = "lang-markdown")]
pub fn markdown_inline() -> Language {
    Language::new("markdown_inline", "Markdown Inline")
        .hidden()
        .with_grammar(grammar(
            tree_sitter_md::INLINE_LANGUAGE.into(),
            &[tree_sitter_md::HIGHLIGHT_QUERY_INLINE],
            Some(tree_sitter_md::INJECTION_QUERY_INLINE),
        ))
}

/// HTML, with `<script>` and `<style>` contents injected as JavaScript and CSS.
#[cfg(feature = "lang-html")]
pub fn html() -> Language {
    Language::new("html", "HTML")
        .with_aliases(&["htm", "xhtml"])
        .with_extensions(&["html", "htm", "xhtml"])
        .with_config(config(include_str!("../configs/html.json")))
        .with_grammar(grammar(
            tree_sitter_html::LANGUAGE.into(),
            &[tree_sitter_html::HIGHLIGHTS_QUERY],
            Some(tree_sitter_html::INJECTIONS_QUERY),
        ))
}

/// CSS (`.css`).
#[cfg(feature = "lang-css")]
pub fn css() -> Language {
    Language::new("css", "CSS")
        .with_extensions(&["css"])
        .with_config(config(include_str!("../configs/css.json")))
        .with_grammar(grammar(
            tree_sitter_css::LANGUAGE.into(),
            &[tree_sitter_css::HIGHLIGHTS_QUERY],
            None,
        ))
}

/// A plain-text language with no grammar.
pub fn plain_text() -> Language {
    Language::new("plaintext", "Plain Text")
        .with_aliases(&["text", "txt"])
        .with_extensions(&["txt"])
}

/// Registers every language enabled by cargo features, plus plain text.
pub fn register_all(registry: &LanguageRegistry) {
    registry.register(plain_text());
    #[cfg(feature = "lang-rust")]
    registry.register(rust());
    #[cfg(feature = "lang-js")]
    registry.register(javascript());
    #[cfg(feature = "lang-ts")]
    {
        registry.register(typescript());
        registry.register(tsx());
    }
    #[cfg(feature = "lang-json")]
    registry.register(json());
    #[cfg(feature = "lang-yaml")]
    registry.register(yaml());
    #[cfg(feature = "lang-toml")]
    registry.register(toml());
    #[cfg(feature = "lang-markdown")]
    {
        registry.register(markdown());
        registry.register(markdown_inline());
    }
    #[cfg(feature = "lang-html")]
    registry.register(html());
    #[cfg(feature = "lang-css")]
    registry.register(css());
}

/// A new registry with every enabled language.
pub fn registry() -> Arc<LanguageRegistry> {
    let registry = Arc::new(LanguageRegistry::new());
    register_all(&registry);
    registry
}
