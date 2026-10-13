//! Monaco `languagesAssociations.test.ts`, as language registry lookups.
//! Each Monaco association is a language here (ids name the expected
//! mime). Not ported: user-configured associations (there is no separate
//! user tier) and data uris (the registry takes paths).

use std::path::Path;

use syntax::{Language, LanguageRegistry};

fn guess(registry: &LanguageRegistry, path: &str, first_line: Option<&str>) -> Option<String> {
    registry
        .language_for_path_or_first_line(Some(Path::new(path)), first_line)
        .map(|language| language.id.to_string())
}

#[test]
fn dynamically_register_text_mime() {
    let registry = LanguageRegistry::new();
    assert_eq!(guess(&registry, "foo.monaco", None), None);
    registry.register(Language::new("monaco", "Monaco").with_extensions(&["monaco"]));
    assert_eq!(
        guess(&registry, "foo.monaco", None).as_deref(),
        Some("monaco")
    );
    assert_eq!(guess(&registry, ".monaco", None).as_deref(), Some("monaco"));
    registry.register(Language::new("codefile", "Code").with_file_names(&["Codefile"]));
    assert_eq!(
        guess(&registry, "Codefile", None).as_deref(),
        Some("codefile")
    );
    assert_eq!(guess(&registry, "foo.Codefile", None), None);
    registry.register(Language::new("docker", "Docker").with_filename_patterns(&["Docker*"]));
    assert_eq!(
        guess(&registry, "Docker-debug", None).as_deref(),
        Some("docker")
    );
    assert_eq!(
        guess(&registry, "docker-PROD", None).as_deref(),
        Some("docker")
    );
    registry.register(Language::new("niceregex", "Nice").with_first_line("RegexesAreNice"));
    let random = "Randomfile.noregistration";
    assert_eq!(
        guess(&registry, random, Some("RegexesAreNice")).as_deref(),
        Some("niceregex")
    );
    assert_eq!(guess(&registry, random, Some("RegexesAreNotNice")), None);
    assert_eq!(
        guess(&registry, "Codefile", Some("RegexesAreNice")).as_deref(),
        Some("codefile")
    );
}

#[test]
fn mimes_priority() {
    let registry = LanguageRegistry::new();
    registry.register(Language::new("monaco", "Monaco").with_extensions(&["monaco"]));
    registry.register(Language::new("foobar", "Foobar").with_first_line("foobar"));
    assert_eq!(
        guess(&registry, "foo.monaco", None).as_deref(),
        Some("monaco")
    );
    assert_eq!(
        guess(&registry, "foo.monaco", Some("foobar")).as_deref(),
        Some("monaco")
    );
    registry.register(Language::new("winner", "Winner").with_file_names(&["dockerfile"]));
    registry.register(Language::new("looser", "Looser").with_filename_patterns(&["dockerfile*"]));
    assert_eq!(
        guess(&registry, "dockerfile", None).as_deref(),
        Some("winner")
    );
    registry.register(Language::new("azure-looser", "A").with_first_line("azure"));
    registry.register(Language::new("azure-winner", "B").with_first_line("azure"));
    assert_eq!(
        guess(&registry, "azure", Some("azure")).as_deref(),
        Some("azure-winner")
    );
}

#[test]
fn specificity_priority() {
    let registry = LanguageRegistry::new();
    registry.register(Language::new("monaco2", "M").with_extensions(&["monaco2"]));
    registry.register(Language::new("specific2", "S").with_file_names(&["specific.monaco2"]));
    assert_eq!(
        guess(&registry, "specific.monaco2", None).as_deref(),
        Some("specific2")
    );
    assert_eq!(
        guess(&registry, "foo.monaco2", None).as_deref(),
        Some("monaco2")
    );
    registry.register(Language::new("specific3", "S").with_file_names(&["specific.monaco3"]));
    registry.register(Language::new("monaco3", "M").with_extensions(&["monaco3"]));
    assert_eq!(
        guess(&registry, "specific.monaco3", None).as_deref(),
        Some("specific3")
    );
    assert_eq!(
        guess(&registry, "foo.monaco3", None).as_deref(),
        Some("monaco3")
    );
}

#[test]
fn longest_extension_wins() {
    let registry = LanguageRegistry::new();
    registry.register(Language::new("monaco", "M").with_extensions(&["monaco"]));
    registry.register(Language::new("monaco-xml", "X").with_extensions(&["monaco.xml"]));
    registry
        .register(Language::new("monaco-xml-build", "B").with_extensions(&["monaco.xml.build"]));
    assert_eq!(
        guess(&registry, "foo.monaco", None).as_deref(),
        Some("monaco")
    );
    assert_eq!(
        guess(&registry, "foo.monaco.xml", None).as_deref(),
        Some("monaco-xml")
    );
    assert_eq!(
        guess(&registry, "foo.monaco.xml.build", None).as_deref(),
        Some("monaco-xml-build")
    );
}

#[test]
fn patterns_match_on_path_and_last_registered_wins() {
    let registry = LanguageRegistry::new();
    registry.register(Language::new("monaco", "M").with_filename_patterns(&["**/dot.monaco.xml"]));
    registry.register(Language::new("other", "O").with_filename_patterns(&["*ot.other.xml"]));
    assert_eq!(
        guess(&registry, "/some/path/dot.monaco.xml", None).as_deref(),
        Some("monaco")
    );
    registry.register(Language::new("later", "L").with_filename_patterns(&["**/dot.monaco.xml"]));
    assert_eq!(
        guess(&registry, "/some/path/dot.monaco.xml", None).as_deref(),
        Some("later")
    );
}

#[test]
fn shebang_detection_for_typescript_runtimes() {
    let registry = LanguageRegistry::new();
    registry.register(
        Language::new("typescript", "TypeScript").with_first_line(r"^#!.*\b(deno|bun|ts-node)\b"),
    );
    for line in [
        "#!/usr/bin/env deno",
        "#!/usr/bin/env -S deno -A",
        "#!/usr/bin/deno",
        "#!/usr/bin/env bun",
        "#!/usr/bin/env -S bun run",
        "#!/usr/bin/env ts-node",
        "#!/usr/bin/env -S ts-node --esm",
    ] {
        assert_eq!(
            guess(&registry, "script", Some(line)).as_deref(),
            Some("typescript"),
            "{line}"
        );
    }
    for line in [
        "#!/usr/bin/env node",
        "#!/usr/bin/env python",
        "#!/bin/bash",
    ] {
        assert_eq!(guess(&registry, "script", Some(line)), None, "{line}");
    }
}

#[test]
fn mime_types_and_runtime_configuration() {
    let registry = LanguageRegistry::new();
    registry.register(Language::new("css", "CSS").with_mimetypes(&["text/css"]));
    assert_eq!(
        registry
            .language_for_mime_type("text/css")
            .map(|l| l.id.to_string())
            .as_deref(),
        Some("css")
    );
    assert!(registry.language_for_mime_type("text/html").is_none());
    let config = syntax::LanguageConfiguration {
        brackets: Some(vec![("<".into(), ">".into())]),
        ..Default::default()
    };
    assert!(registry.register_configuration("css", config).is_some());
    assert!(
        registry
            .register_configuration("html", Default::default())
            .is_none()
    );
    let css = registry.language_for_id("css").unwrap();
    assert_eq!(css.config.brackets(), [("<".to_string(), ">".to_string())]);
    assert_eq!(&*css.mimetypes, ["text/css"]);
}

/// Monaco `LanguagesRegistry._mergeLanguage`: registering an id again adds
/// to the language instead of replacing it.
#[test]
fn registering_again_merges() {
    let registry = LanguageRegistry::new();
    let config = syntax::LanguageConfiguration {
        comments: Some(syntax::Comments {
            line_comment: Some("//".into()),
            ..Default::default()
        }),
        brackets: Some(vec![("(".into(), ")".into())]),
        ..Default::default()
    };
    registry.register(
        Language::new("mylang", "My Language")
            .with_extensions(&["my"])
            .with_aliases(&["My Language"])
            .with_config(config)
            .with_monarch_grammar(&serde_json::json!({ "tokenizer": { "root": [] } }))
            .unwrap(),
    );
    // A bare registration of the same id, as `monaco.languages.register({ id })`.
    registry.register(
        Language::new("mylang", "mylang")
            .with_extensions(&["mine"])
            .with_file_names(&["MyFile"])
            .with_config(syntax::LanguageConfiguration {
                brackets: Some(vec![("[".into(), "]".into())]),
                ..Default::default()
            }),
    );
    let language = registry.language_for_id("mylang").unwrap();
    assert_eq!(&*language.name, "My Language");
    assert_eq!(language.extensions, ["my", "mine"]);
    assert_eq!(language.file_names, ["MyFile"]);
    assert_eq!(language.aliases, ["My Language"]);
    // The highlighter is kept, and only the fields set are replaced.
    assert!(language.highlighter.is_some());
    assert_eq!(
        language.config.comments().line_comment.as_deref(),
        Some("//")
    );
    assert_eq!(
        language.config.brackets(),
        [("[".to_string(), "]".to_string())]
    );
    assert_eq!(registry.languages().len(), 1);
    for path in ["a.my", "b.mine", "MyFile"] {
        assert_eq!(
            registry
                .language_for_path(Path::new(path))
                .map(|l| l.id.to_string())
                .as_deref(),
            Some("mylang"),
            "{path}"
        );
    }
}

/// Monaco `combineLanguageConfigurations`: runtime configurations stack,
/// each field from the newest that sets it, and removing one restores what
/// it covered.
#[test]
fn configurations_stack() {
    let registry = LanguageRegistry::new();
    registry.register(Language::new("js", "JavaScript").with_config(
        syntax::LanguageConfiguration {
            brackets: Some(vec![("{".into(), "}".into())]),
            auto_close_before: Some(";".into()),
            ..Default::default()
        },
    ));
    let comments = registry
        .register_configuration(
            "js",
            syntax::LanguageConfiguration {
                comments: Some(syntax::Comments {
                    line_comment: Some("//".into()),
                    ..Default::default()
                }),
                // Monaco treats an empty `autoCloseBefore` as unset.
                auto_close_before: Some(String::new()),
                ..Default::default()
            },
        )
        .unwrap();
    let brackets = registry
        .register_configuration(
            "js",
            syntax::LanguageConfiguration {
                brackets: Some(vec![("(".into(), ")".into())]),
                auto_closing_pairs: Some(Vec::new()),
                ..Default::default()
            },
        )
        .unwrap();
    let config = registry.language_for_id("js").unwrap().config.clone();
    assert_eq!(config.comments().line_comment.as_deref(), Some("//"));
    assert_eq!(config.brackets(), [("(".to_string(), ")".to_string())]);
    assert_eq!(config.auto_close_before.as_deref(), Some(";"));
    assert_eq!(config.auto_closing_pairs.as_deref(), Some(&[][..]));

    assert!(registry.unregister_configuration("js", brackets));
    assert!(!registry.unregister_configuration("js", brackets));
    let config = registry.language_for_id("js").unwrap().config.clone();
    assert_eq!(config.brackets(), [("{".to_string(), "}".to_string())]);
    assert_eq!(config.comments().line_comment.as_deref(), Some("//"));
    assert!(config.auto_closing_pairs.is_none());

    assert!(registry.unregister_configuration("js", comments));
    let config = registry.language_for_id("js").unwrap().config.clone();
    assert!(config.comments.is_none());
}

/// Monaco's `setMonarchTokensProvider` doesn't need the language
/// registered: the highlighter applies once it is.
#[test]
fn highlighters_wait_for_their_language() {
    let registry = LanguageRegistry::new();
    let tokenizer = std::sync::Arc::new(
        syntax::MonarchTokenizer::new("later", &serde_json::json!({ "tokenizer": { "root": [] } }))
            .unwrap(),
    );
    assert!(!registry.set_highlighter(
        "later",
        Some(syntax::MonarchHighlighter::factory(tokenizer))
    ));
    assert!(registry.pending_highlighter("later").is_some());
    let language = registry.register(Language::new("later", "Later"));
    assert!(language.highlighter.is_some());
    assert!(registry.pending_highlighter("later").is_none());
}

#[test]
fn globs() {
    use syntax::glob_matches;
    assert!(glob_matches("*.config.js", "webpack.config.js"));
    assert!(!glob_matches("*.config.js", "src/webpack.config.js"));
    assert!(glob_matches("**/*.test.ts", "/a/b/c.test.ts"));
    assert!(glob_matches("**/*.test.ts", "c.test.ts"));
    assert!(glob_matches("*.{js,ts}", "a.ts"));
    assert!(glob_matches("file?.txt", "file1.txt"));
    assert!(glob_matches("[ab].rs", "b.rs"));
    assert!(!glob_matches("[!ab].rs", "b.rs"));
}

/// Monaco's `LanguagesRegistry` anchors a `firstLine` without a `^` at the
/// line start, and compiles it as a JavaScript regex.
#[test]
fn first_line_is_anchored_javascript() {
    let registry = LanguageRegistry::new();
    registry.register(Language::new("xml", "XML").with_first_line(r"<\?xml"));
    let guess = |first_line| {
        registry
            .language_for_path_or_first_line(None, Some(first_line))
            .map(|language| language.id.to_string())
    };
    assert_eq!(guess("<?xml version=\"1.0\"?>").as_deref(), Some("xml"));
    assert_eq!(guess("foo <?xml"), None);
    registry.register(Language::new("word", "Word").with_first_line(r"^\w+$"));
    assert_eq!(guess("plain").as_deref(), Some("word"));
    // `\w` is ASCII, as in JavaScript.
    assert_eq!(guess("naïve"), None);
}
