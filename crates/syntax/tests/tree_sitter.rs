//! Tree-sitter backend: highlighting, incremental reparse equivalence and
//! injections, using real grammars.

use std::sync::Arc;

use proptest::prelude::*;
use syntax::{Grammar, HighlightSpan, Language, LanguageRegistry, SyntaxSnapshot};
use text::Buffer;

fn registry() -> Arc<LanguageRegistry> {
    let registry = Arc::new(LanguageRegistry::new());
    registry.register(
        Language::new("rust", "Rust")
            .with_aliases(&["rs"])
            .with_extensions(&["rs"])
            .with_grammar(
                Grammar::new(tree_sitter_rust::LANGUAGE.into())
                    .with_highlights_query(tree_sitter_rust::HIGHLIGHTS_QUERY)
                    .unwrap()
                    .with_injections_query(tree_sitter_rust::INJECTIONS_QUERY)
                    .unwrap(),
            ),
    );
    registry.register(
        Language::new("markdown", "Markdown")
            .with_extensions(&["md"])
            .with_grammar(
                Grammar::new(tree_sitter_md::LANGUAGE.into())
                    .with_highlights_query(tree_sitter_md::HIGHLIGHT_QUERY_BLOCK)
                    .unwrap()
                    .with_injections_query(tree_sitter_md::INJECTION_QUERY_BLOCK)
                    .unwrap(),
            ),
    );
    registry.register(
        Language::new("markdown_inline", "Markdown Inline")
            .hidden()
            .with_grammar(
                Grammar::new(tree_sitter_md::INLINE_LANGUAGE.into())
                    .with_highlights_query(tree_sitter_md::HIGHLIGHT_QUERY_INLINE)
                    .unwrap()
                    .with_injections_query(tree_sitter_md::INJECTION_QUERY_INLINE)
                    .unwrap(),
            ),
    );
    registry
}

fn named(text: &str, spans: &[HighlightSpan]) -> Vec<(String, String)> {
    spans
        .iter()
        .map(|span| {
            (
                text[span.range.clone()].to_string(),
                span.highlight.name().to_string(),
            )
        })
        .collect()
}

#[test]
fn highlights_rust() {
    let registry = registry();
    let rust = registry.language_for_id("rust").unwrap();
    let source = "fn main() {\n    let x = \"hi\"; // note\n}\n";
    let buffer = Buffer::new(source);
    let syntax = SyntaxSnapshot::parse(rust, &buffer.snapshot(), &registry);
    let spans = syntax.highlights(buffer.rope(), 0..buffer.len());
    let named = named(source, &spans);
    assert!(
        named.contains(&("fn".into(), "keyword".into())),
        "{named:?}"
    );
    assert!(
        named.contains(&("main".into(), "function".into())),
        "{named:?}"
    );
    assert!(
        named.contains(&("\"hi\"".into(), "string".into())),
        "{named:?}"
    );
    assert!(
        named.contains(&("// note".into(), "comment".into())),
        "{named:?}"
    );
    for pair in spans.windows(2) {
        assert!(pair[0].range.end <= pair[1].range.start);
    }
}

#[test]
fn markdown_injects_fenced_rust_and_inline() {
    let registry = registry();
    let markdown = registry.language_for_path("README.md".as_ref()).unwrap();
    let source = "# Title\n\nSome *emphasis* here.\n\n```rust\nfn main() {}\n```\n";
    let buffer = Buffer::new(source);
    let syntax = SyntaxSnapshot::parse(markdown, &buffer.snapshot(), &registry);
    let languages: Vec<&str> = syntax.layers().iter().map(|l| &*l.language.id).collect();
    assert!(languages.contains(&"rust"), "{languages:?}");
    assert!(languages.contains(&"markdown_inline"), "{languages:?}");
    let named = named(source, &syntax.highlights(buffer.rope(), 0..buffer.len()));
    assert!(
        named.contains(&("fn".into(), "keyword".into())),
        "{named:?}"
    );
    assert!(
        named.contains(&("main".into(), "function".into())),
        "{named:?}"
    );
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 128, max_global_rejects: 4096, ..ProptestConfig::default() })]
    /// Incremental reparse produces the same highlights as a fresh parse.
    #[test]
    fn incremental_reparse_matches_fresh_parse(
        edits in proptest::collection::vec((any::<usize>(), 0usize..6, prop_oneof![
            Just(""), Just("{"), Just("}"), Just("\n"), Just("let y = 1;"), Just("\"str"), Just("// c\n"), Just("```rust\nfn f() {}\n```\n"), Just("m!(z);"), Just(")"), Just("v![a, b]"),
        ]), 1..8),
        markdown in any::<bool>(),
    ) {
        let registry = registry();
        let (language, source) = if markdown {
            (registry.language_for_id("markdown").unwrap(), "# T\n\ntext *a*\n\n```rust\nfn main() { let x = 1; }\n```\n\nend\n")
        } else {
            (registry.language_for_id("rust").unwrap(), "fn main() {\n    let x = vec![1, 2];\n    println!(\"{}\", x[0]);\n}\n\nfn g() {\n    assert!(true);\n    m!{ a b };\n}\n")
        };
        let mut buffer = Buffer::new(source);
        let mut old = buffer.snapshot();
        let mut syntax = SyntaxSnapshot::parse(language.clone(), &old, &registry);
        for (at, len, insert) in edits {
            let start = buffer.clip_offset(at % (buffer.len() + 1), text::Bias::Left);
            let end = buffer.clip_offset((start + len).min(buffer.len()), text::Bias::Right);
            buffer.edit([(start..end, insert)]).unwrap();
            let new = buffer.snapshot();
            let patch = buffer.edits_since(old.version());
            syntax = syntax.reparse(old.rope(), &new, &patch, &registry);
            old = new;
        }
        let fresh = SyntaxSnapshot::parse(language, &old, &registry);
        // Tree-sitter's error recovery may legitimately differ between an
        // incremental and a fresh parse, so only compare error-free trees.
        let has_error = fresh.layers().iter().any(|layer| layer.tree.root_node().has_error());
        prop_assume!(!has_error);
        // Injection layers (cached and updated piecewise on reparse) match.
        let layers = |syntax: &SyntaxSnapshot| {
            let mut layers: Vec<_> = syntax
                .layers()
                .iter()
                .map(|l| (l.depth, l.language.id.to_string(), l.ranges.clone()))
                .collect();
            layers.sort_by(|a, b| (a.0, &a.2.first().map(|r| r.start)).cmp(&(b.0, &b.2.first().map(|r| r.start))));
            layers
        };
        prop_assert_eq!(layers(&syntax), layers(&fresh));
        let range = 0..buffer.len();
        prop_assert_eq!(
            syntax.highlights(buffer.rope(), range.clone()),
            fresh.highlights(buffer.rope(), range)
        );
        prop_assert_eq!(
            syntax.root().unwrap().tree.root_node().to_sexp(),
            fresh.root().unwrap().tree.root_node().to_sexp()
        );
    }
}
