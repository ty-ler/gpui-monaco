//! Every bundled language loads (queries compile, configs parse) and
//! highlights a representative sample.

use syntax::SyntaxSnapshot;
use text::Buffer;

/// (path, source, expected (text, highlight) pairs)
type Sample = (
    &'static str,
    &'static str,
    &'static [(&'static str, &'static str)],
);

const SAMPLES: &[Sample] = &[
    (
        "main.rs",
        "fn main() { let s = \"x\"; }",
        &[("fn", "keyword"), ("\"x\"", "string")],
    ),
    (
        "app.js",
        "const a = () => 1; // c",
        &[("const", "keyword"), ("// c", "comment")],
    ),
    (
        "app.ts",
        "let n: number = 1;",
        &[("let", "keyword"), ("number", "type.builtin")],
    ),
    (
        "view.tsx",
        "const v = <div className=\"a\" />;",
        &[("const", "keyword"), ("div", "tag")],
    ),
    (
        "data.json",
        "{\"key\": true}",
        &[
            ("\"key\"", "string.special.key"),
            ("true", "constant.builtin"),
        ],
    ),
    (
        "conf.yaml",
        "key: value # note",
        &[("key", "property"), ("# note", "comment")],
    ),
    (
        "Cargo.toml",
        "[package]\nname = \"x\"",
        &[("name", "property"), ("\"x\"", "string")],
    ),
    (
        "README.md",
        "# Title\n\n```js\nlet a = 1;\n```\n",
        &[("let", "keyword")],
    ),
    (
        "index.html",
        "<div><script>let a = 1;</script><style>p { color: red; }</style></div>",
        &[("div", "tag"), ("let", "keyword"), ("color", "property")],
    ),
    (
        "site.css",
        "a { color: red; }",
        &[("a", "tag"), ("color", "property")],
    ),
];

#[test]
fn bundled_languages_highlight_samples() {
    let registry = gpui_monaco_languages::registry();
    for (path, source, expected) in SAMPLES {
        let language = registry
            .language_for_path(path.as_ref())
            .unwrap_or_else(|| panic!("no language for {path}"));
        let buffer = Buffer::new(source);
        let syntax = SyntaxSnapshot::parse(language, &buffer.snapshot(), &registry);
        let spans = syntax.highlights(buffer.rope(), 0..buffer.len());
        let named: Vec<(String, String)> = spans
            .iter()
            .map(|s| {
                (
                    source[s.range.clone()].to_string(),
                    s.highlight.name().to_string(),
                )
            })
            .collect();
        for (text, name) in *expected {
            assert!(
                named
                    .iter()
                    .any(|(t, n)| t.trim() == *text
                        && (n == name || n.starts_with(&format!("{name}.")))),
                "{path}: expected {text:?} as {name}, got {named:?}"
            );
        }
    }
}

#[test]
fn configurations_load() {
    let registry = gpui_monaco_languages::registry();
    let rust = registry.language_for_id("rust").unwrap();
    assert_eq!(rust.config.comments().line_comment.as_deref(), Some("//"));
    assert!(!rust.config.auto_closing_pairs().is_empty());
    let ts = registry.language_for_name("ts").unwrap();
    assert!(
        ts.config
            .on_enter_rules()
            .iter()
            .all(|rule| rule.before_text.regex().is_some())
    );
}

#[test]
fn configuration_patterns_compile() {
    let registry = gpui_monaco_languages::registry();
    let mut failures = Vec::new();
    for language in registry.languages() {
        for (key, pattern) in language.config.patterns() {
            if let Some(error) = pattern.error() {
                failures.push(format!("{} {key}: {error}", language.id));
            }
        }
    }
    assert!(failures.is_empty(), "{failures:#?}");
}

#[test]
fn html_enter_between_tags_matches() {
    // These patterns use `[\w-.]`, which only compiles with the JavaScript
    // translation.
    let registry = gpui_monaco_languages::registry();
    let html = registry.language_for_id("html").unwrap();
    let rule = &html.config.on_enter_rules()[0];
    assert!(rule.before_text.is_match("<my-tag class=\"a\">"));
    assert!(!rule.before_text.is_match("<br>"));
    assert!(rule.after_text.as_ref().unwrap().is_match("</my-tag>"));
}

/// (file name, text, expected folds as 0-based (first row, last row folded))
type FoldSample = (&'static str, &'static str, &'static [(u32, u32)]);

const FOLD_SAMPLES: &[FoldSample] = &[
    (
        "main.rs",
        "fn main() {\n    let v = vec![\n        1,\n    ];\n}\n/* a\n b */\n",
        &[(0, 3), (1, 2), (5, 6)],
    ),
    (
        "app.js",
        "function f(\n  a,\n) {\n  return {\n    b: 1,\n  };\n}\n",
        &[(0, 1), (2, 5), (3, 4)],
    ),
    ("app.ts", "interface A {\n  b: number;\n}\n", &[(0, 1)]),
    (
        "view.tsx",
        "const v = (\n  <div>\n    hi\n  </div>\n);\n",
        &[(1, 2)],
    ),
    (
        "data.json",
        "{\n  \"a\": [\n    1\n  ]\n}\n",
        &[(0, 3), (1, 2)],
    ),
    (
        "conf.yaml",
        "a:\n  b: 1\n  c:\n    - 2\n",
        &[(0, 3), (2, 3)],
    ),
    (
        "Cargo.toml",
        "[package]\nname = \"x\"\n\n[deps]\ny = 1\n",
        &[(0, 2), (3, 4)],
    ),
    (
        "doc.md",
        "# A\n\ntext\n\n## B\n\nmore\n\n```rs\nx\n```\n",
        &[(0, 10), (4, 10), (8, 9)],
    ),
    (
        "page.html",
        "<div>\n  <p>\n    x\n  </p>\n</div>\n",
        &[(0, 3), (1, 2)],
    ),
    ("style.css", "a {\n  color: red;\n}\n", &[(0, 1)]),
];

#[test]
fn bundled_fold_queries() {
    let registry = gpui_monaco_languages::registry();
    let mut wrong = Vec::new();
    for (path, text, expected) in FOLD_SAMPLES {
        let language = registry
            .language_for_path(std::path::Path::new(path))
            .unwrap();
        let buffer = Buffer::new(text);
        let syntax = SyntaxSnapshot::parse(language, &buffer.snapshot(), &registry);
        let folds = syntax.fold_ranges(buffer.rope()).unwrap();
        if folds != *expected {
            wrong.push(format!("{path}: {folds:?}"));
        }
    }
    assert!(wrong.is_empty(), "{wrong:#?}");
}

#[test]
fn syntax_selection_ranges_nest() {
    // Smart select's syntax ranges, inside out, for every bundled language.
    let registry = gpui_monaco_languages::registry();
    for (path, text, _) in FOLD_SAMPLES {
        let language = registry
            .language_for_path(std::path::Path::new(path))
            .unwrap();
        let buffer = Buffer::new(text);
        let syntax = SyntaxSnapshot::parse(language, &buffer.snapshot(), &registry);
        let offset = text.find('\n').unwrap() + 3;
        let ranges = syntax.selection_ranges(offset);
        assert!(ranges.len() >= 2, "{path}: {ranges:?}");
        // Every range is around the position, and the root layer's chain
        // (injected layers follow it) grows outward.
        assert!(
            ranges.iter().all(|r| r.start <= offset && offset <= r.end),
            "{path}: {ranges:?}"
        );
        let chain = ranges
            .windows(2)
            .take_while(|w| w[1].start <= w[0].start && w[0].end <= w[1].end)
            .count();
        assert!(chain >= 1, "{path}: {ranges:?}");
    }
}
