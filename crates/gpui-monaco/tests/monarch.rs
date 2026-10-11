//! Monarch grammars (Monaco `languages.setMonarchTokensProvider`) in
//! models, `tokenize`, `colorize` and themes.

use std::collections::HashMap;
use std::sync::Arc;

use gpui::{AppContext as _, Entity, TestAppContext};
use gpui_monaco::colorize::{ColorizeOptions, colorize, tokenize};
use gpui_monaco::cursor::StandardTokenType;
use gpui_monaco::syntax::{HighlightId, Language, LanguageRegistry};
use gpui_monaco::theme::{TokenRule, parse_hex_color};
use gpui_monaco::{TextModel, Theme, TokenStyle};
use serde_json::json;

fn grammar() -> serde_json::Value {
    json!({
        "keywords": ["let", "fn"],
        "tokenizer": {
            "root": [
                ["[a-z]+", { "cases": { "@keywords": "keyword", "@default": "identifier" } }],
                ["\"", "string", "@string"],
                ["/\\*", "comment", "@comment"],
                ["<", { "token": "tag", "next": "@embedded", "nextEmbedded": "inner" }],
                [" +", "white"],
            ],
            "string": [["[^\"]+", "string"], ["\"", "string", "@pop"]],
            "comment": [["\\*/", "comment", "@pop"], [".", "comment"]],
            "embedded": [[">", { "token": "tag", "next": "@pop", "nextEmbedded": "@pop" }]],
        },
    })
}

fn registry() -> Arc<LanguageRegistry> {
    let registry = Arc::new(LanguageRegistry::new());
    registry.register(Language::new("mini", "Mini"));
    registry.register(
        Language::new("inner", "Inner")
            .with_monarch_grammar(&json!({ "tokenizer": { "root": [["\\d+", "number"]] } }))
            .unwrap(),
    );
    registry
}

fn init(cx: &mut TestAppContext) -> Arc<LanguageRegistry> {
    let registry = registry();
    cx.update(|cx| {
        gpui_monaco::init(cx);
        gpui_monaco::set_language_registry(cx, registry.clone());
    });
    registry
}

fn shown(text: &str, cx: &mut TestAppContext) -> Entity<TextModel> {
    let model = cx.new(|_| TextModel::new(text).with_language("mini"));
    model.update(cx, |model, cx| model.ensure_syntax(cx));
    cx.run_until_parked();
    model
}

fn name_at(model: &Entity<TextModel>, offset: usize, cx: &mut TestAppContext) -> Option<String> {
    model.read_with(cx, |model, _| {
        model.highlight_name_at(offset).map(|name| name.to_string())
    })
}

#[gpui::test]
fn set_monarch_tokens_provider_highlights_models(cx: &mut TestAppContext) {
    init(cx);
    let model = shown("let x = \"s\" /* c\nd */ y", cx);
    // No grammar yet.
    assert_eq!(name_at(&model, 0, cx), None);

    let disposable = cx
        .update(|cx| gpui_monaco::set_monarch_tokens_provider(cx, "mini", &grammar()))
        .unwrap();
    cx.run_until_parked();
    assert_eq!(name_at(&model, 0, cx).as_deref(), Some("keyword.mini"));
    assert_eq!(name_at(&model, 4, cx).as_deref(), Some("identifier.mini"));
    assert_eq!(name_at(&model, 6, cx).as_deref(), Some("source.mini"));
    assert_eq!(name_at(&model, 9, cx).as_deref(), Some("string.mini"));
    assert_eq!(name_at(&model, 13, cx).as_deref(), Some("comment.mini"));
    // The comment continues on the next line.
    assert_eq!(name_at(&model, 17, cx).as_deref(), Some("comment.mini"));
    model.read_with(cx, |model, _| {
        assert_eq!(model.token_type_at(9), StandardTokenType::String);
        assert_eq!(model.token_type_at(17), StandardTokenType::Comment);
        assert_eq!(model.token_type_at(0), StandardTokenType::Other);
    });

    // Edits re-tokenize: closing the comment early makes `d` an identifier.
    model.update(cx, |model, cx| model.edit([(16..16, "*/")], cx).unwrap());
    cx.run_until_parked();
    assert_eq!(name_at(&model, 19, cx).as_deref(), Some("identifier.mini"));

    // Line tokens are Monaco's, with UTF-16 offsets.
    let tokens = model.read_with(cx, |model, cx| model.line_tokens(0, cx).unwrap());
    let types: Vec<(u32, &str)> = tokens
        .iter()
        .map(|t| (t.offset, t.token_type.as_ref()))
        .collect();
    assert_eq!(
        types,
        vec![
            (0, "keyword.mini"),
            (3, "white.mini"),
            (4, "identifier.mini"),
            (5, "white.mini"),
            (6, "source.mini"),
            (7, "white.mini"),
            (8, "string.mini"),
            (11, "white.mini"),
            (12, "comment.mini"),
        ]
    );

    // Disposing goes back to no highlighting.
    cx.update(|cx| disposable.dispose(cx));
    cx.run_until_parked();
    assert_eq!(name_at(&model, 0, cx), None);
}

#[gpui::test]
fn set_monarch_tokens_provider_fails_like_monaco(cx: &mut TestAppContext) {
    init(cx);
    cx.update(|cx| {
        let unknown = gpui_monaco::set_monarch_tokens_provider(cx, "nope", &grammar());
        assert_eq!(
            unknown.unwrap_err().to_string(),
            "Cannot set tokens provider for unknown language nope"
        );
        let invalid = gpui_monaco::set_monarch_tokens_provider(
            cx,
            "mini",
            &json!({ "tokenizer": { "root": [["a", "x", "@nowhere"]] } }),
        );
        assert!(
            invalid
                .unwrap_err()
                .to_string()
                .starts_with("mini: the next state '@nowhere' is not defined")
        );
        // Monaco `remeasureFonts` is a no-op.
        gpui_monaco::remeasure_fonts(cx);
    });
}

#[gpui::test]
fn embedded_monarch_languages(cx: &mut TestAppContext) {
    init(cx);
    // Dropping the `Disposable` keeps the provider.
    let _provider = cx
        .update(|cx| gpui_monaco::set_monarch_tokens_provider(cx, "mini", &grammar()))
        .unwrap();
    let model = shown("a <12> b", cx);
    assert_eq!(name_at(&model, 3, cx).as_deref(), Some("number.inner"));
    model.read_with(cx, |model, cx| {
        assert_eq!(
            model.language_at(3).map(|l| l.id.to_string()).as_deref(),
            Some("inner")
        );
        assert_eq!(
            model.language_at(0).map(|l| l.id.to_string()).as_deref(),
            Some("mini")
        );
        let tokens = model.line_tokens(0, cx).unwrap();
        let languages: Vec<(u32, &str, &str)> = tokens
            .iter()
            .map(|t| (t.offset, t.token_type.as_ref(), t.language.as_ref()))
            .collect();
        assert_eq!(
            languages,
            vec![
                (0, "identifier.mini", "mini"),
                (1, "white.mini", "mini"),
                (2, "tag.mini", "mini"),
                (3, "number.inner", "inner"),
                (5, "tag.mini", "mini"),
                (6, "white.mini", "mini"),
                (7, "identifier.mini", "mini"),
            ]
        );
    });
}

#[test]
fn tokenize_and_colorize_with_monarch() {
    let registry = registry();
    registry.register(
        Language::new("mini", "Mini")
            .with_monarch_grammar(&grammar())
            .unwrap(),
    );
    let lines = tokenize("let é\n\n/* x", "mini", &registry);
    let lines: Vec<Vec<(u32, String, String)>> = lines
        .into_iter()
        .map(|line| {
            line.into_iter()
                .map(|t| (t.offset, t.token_type.to_string(), t.language.to_string()))
                .collect()
        })
        .collect();
    let t = |offset: u32, token_type: &str| (offset, token_type.to_string(), "mini".to_string());
    // Exactly the tokenizer's tokens: `é` falls back to `defaultToken`, and
    // the empty line has none, as in Monaco.
    assert_eq!(
        lines,
        vec![
            vec![
                t(0, "keyword.mini"),
                t(3, "white.mini"),
                t(4, "source.mini")
            ],
            vec![],
            vec![t(0, "comment.mini")],
        ]
    );

    let theme = Theme::vs_dark();
    let colorized = colorize(
        "let x",
        "mini",
        &theme,
        &registry,
        ColorizeOptions::default(),
    );
    let keyword = theme.style_for(HighlightId::intern("keyword"));
    assert_eq!(colorized[0].highlights[0].0, 0..3);
    assert_eq!(colorized[0].highlights[0].1.color, keyword.foreground);
}

fn color(hex: &str) -> TokenStyle {
    TokenStyle {
        foreground: parse_hex_color(hex),
        ..Default::default()
    }
}

#[test]
fn themes_match_monarch_tokens_like_monaco() {
    let keyword_mini = HighlightId::intern_monaco_token("keyword.mini");
    let keyword_other = HighlightId::intern_monaco_token("keyword.other");
    let type_mini = HighlightId::intern_monaco_token("type.identifier.mini");

    // Monaco rules: the longest matching prefix wins.
    let rule = |selector: &str, hex: &str| TokenRule::monaco(selector, Some(hex), None).unwrap();
    let theme = Theme::define(
        "monarch-test",
        &Theme::vs(),
        true,
        vec![rule("keyword", "#111111"), rule("keyword.mini", "#222222")],
        &HashMap::new(),
    );
    assert_eq!(theme.style_for(keyword_mini), color("#222222"));
    assert_eq!(theme.style_for(keyword_other), color("#111111"));
    // vs's `type` rule.
    assert_eq!(
        theme.style_for(type_mini).foreground,
        Theme::vs()
            .style_for(HighlightId::intern("type"))
            .foreground
    );

    // VS Code themes: TextMate rules, the token as a scope, then the
    // capture's scopes.
    let vscode = Theme::from_vscode_json(
        r##"{
            "tokenColors": [
                { "scope": "keyword", "settings": { "foreground": "#333333" } },
                { "scope": "entity.name.type", "settings": { "foreground": "#444444" } }
            ]
        }"##,
    )
    .unwrap();
    assert_eq!(vscode.style_for(keyword_mini), color("#333333"));
    assert_eq!(vscode.style_for(type_mini), color("#444444"));
}
