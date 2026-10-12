//! Tokenization limits and token types (#428): Monaco's
//! `maxTokenizationLineLength` in the Monarch tokenizer and not for
//! `setTokensProvider`, the bracket pair tokenizer's standard token types,
//! injected languages' configuration looked up at each use, and one
//! tokens provider factory per language.

use std::cell::Cell;
use std::rc::Rc;
use std::sync::Arc;

use gpui::{AppContext as _, Entity, Focusable, TestAppContext, VisualTestContext};
use gpui_monaco::cursor::{Selection, StandardTokenType};
use gpui_monaco::syntax::{
    Comments, Language, LanguageConfiguration, LanguageRegistry, LineTokens, Token, TokensProvider,
};
use gpui_monaco::text::PointUtf16;
use gpui_monaco::{CodeEditor, ModelOptions, TextModel};
use serde_json::json;

/// A C-like Monarch grammar: identifiers, `/* */` comments across lines,
/// `/.../` regexes, `'...'` invalid strings and parentheses.
fn grammar() -> serde_json::Value {
    json!({
        "keywords": ["int"],
        "tokenizer": {
            "root": [
                ["[a-z]+", { "cases": { "@keywords": "keyword", "@default": "identifier" } }],
                ["/\\*", "comment", "@comment"],
                ["/[^/]*/", "regexp"],
                ["'[^']*'", "invalid.string"],
                ["\\d+", "number"],
                ["[()]", "delimiter"],
                ["[ =;]+", "white"],
            ],
            "comment": [["\\*/", "comment", "@pop"], [".", "comment"]],
        },
    })
}

fn clike() -> Language {
    Language::new("clike", "C-like")
        .with_config(LanguageConfiguration {
            brackets: Some(vec![("(".into(), ")".into())]),
            ..Default::default()
        })
        .with_monarch_grammar(&grammar())
        .unwrap()
}

fn init(cx: &mut TestAppContext) -> Arc<LanguageRegistry> {
    let registry = Arc::new(LanguageRegistry::new());
    registry.register(clike());
    registry.register(Language::new("plain", "Plain"));
    cx.update(|cx| {
        gpui_monaco::init(cx);
        gpui_monaco::set_language_registry(cx, registry.clone());
    });
    registry
}

fn options(max_tokenization_line_length: u32) -> ModelOptions {
    ModelOptions {
        max_tokenization_line_length,
        ..Default::default()
    }
}

fn shown(text: &str, language: &str, limit: u32, cx: &mut TestAppContext) -> Entity<TextModel> {
    let language = language.to_string();
    let model = cx.new(|_| {
        TextModel::new(text)
            .with_language(language)
            .with_options(options(limit))
    });
    model.update(cx, |model, cx| model.ensure_syntax(cx));
    cx.run_until_parked();
    model
}

fn name_at(model: &Entity<TextModel>, offset: usize, cx: &mut TestAppContext) -> Option<String> {
    model.read_with(cx, |model, _| {
        model.highlight_name_at(offset).map(|name| name.to_string())
    })
}

/// Monaco `MonarchTokenizer.tokenize`: a line at least
/// `maxTokenizationLineLength` long gets `nullTokenize`, keeping its
/// state, so an unterminated comment on it doesn't reach the next line;
/// a larger limit tokenizes longer lines (§F.3).
#[gpui::test]
fn monarch_models_use_their_max_tokenization_line_length(cx: &mut TestAppContext) {
    init(cx);
    let long = format!("{}/*", "a".repeat(198));
    let model = shown(&format!("{long}\nint x;"), "clike", 100, cx);
    let second_line = long.len() + 1;
    assert_eq!(name_at(&model, 0, cx), None);
    assert_eq!(
        name_at(&model, second_line, cx).as_deref(),
        Some("keyword.clike")
    );
    // A longer limit re-tokenizes with it: the comment now runs on.
    model.update(cx, |model, cx| model.update_options(options(50_000), cx));
    cx.run_until_parked();
    assert_eq!(name_at(&model, 0, cx).as_deref(), Some("identifier.clike"));
    assert_eq!(
        name_at(&model, second_line, cx).as_deref(),
        Some("comment.clike")
    );

    // A 30,000-character line is past the default limit, not this one.
    let model = shown(&"a".repeat(30_000), "clike", 50_000, cx);
    assert_eq!(
        name_at(&model, 29_999, cx).as_deref(),
        Some("identifier.clike")
    );
    let model = shown(&"a".repeat(30_000), "clike", 20_000, cx);
    assert_eq!(name_at(&model, 29_999, cx), None);
}

/// A line-state tokenizer: `k` is a keyword, and `(` opens a comment
/// that runs until `)`, across lines.
struct Parens;

impl TokensProvider for Parens {
    type State = bool;

    fn initial_state(&self) -> bool {
        false
    }

    fn tokenize(&self, line: &str, in_comment: &bool) -> LineTokens<bool> {
        let mut tokens = Vec::new();
        let mut in_comment = *in_comment;
        for (ix, ch) in line.char_indices() {
            let scopes = match ch {
                '(' => {
                    in_comment = true;
                    "comment"
                }
                ')' => {
                    in_comment = false;
                    "comment"
                }
                _ if in_comment => "comment",
                'k' => "keyword",
                _ => "",
            };
            tokens.push(Token::new(ix, scopes));
        }
        LineTokens {
            tokens,
            end_state: in_comment,
        }
    }
}

/// Monaco's `TokenizationSupportAdapter` (`setTokensProvider`) has no
/// line length limit: long lines keep their tokens for display and for
/// token type queries such as auto-closing's `notIn` (§F.3).
#[gpui::test]
fn tokens_providers_tokenize_lines_of_any_length(cx: &mut TestAppContext) {
    init(cx);
    let _provider = cx
        .update(|cx| gpui_monaco::set_tokens_provider(cx, "plain", Parens))
        .unwrap();
    let text = format!("{}k (x", "a".repeat(20));
    let model = shown(&text, "plain", 10, cx);
    assert_eq!(name_at(&model, 20, cx).as_deref(), Some("keyword"));
    model.read_with(cx, |model, cx| {
        assert_eq!(model.token_type_at(23), StandardTokenType::Comment);
        let tokens = model.line_tokens(0, cx).unwrap();
        assert!(tokens.iter().any(|t| t.token_type.as_ref() == "keyword"));
    });
    // A Monarch model with the same limit leaves the line untokenized.
    let monarch = shown(&format!("{}/*", "a".repeat(20)), "clike", 10, cx);
    monarch.read_with(cx, |model, _| {
        assert_eq!(model.token_type_at(21), StandardTokenType::Other);
    });
}

fn editor_for(
    model: impl FnOnce() -> TextModel + 'static,
    cx: &mut TestAppContext,
) -> (Entity<CodeEditor>, &mut VisualTestContext) {
    let (editor, cx) = cx.add_window_view(|window, cx| {
        let model = cx.new(|_| model());
        CodeEditor::new(model, window, cx)
    });
    cx.update(|window, cx| window.focus(&editor.focus_handle(cx), cx));
    cx.run_until_parked();
    (editor, cx)
}

/// Monaco's bracket pair tokenizer counts a bracket only in a token whose
/// standard type is `Other` (`toStandardTokenType`), so brackets in
/// `regexp` and `invalid.string` tokens are skipped (§F.4).
#[gpui::test]
fn bracket_pair_colorization_skips_regex_and_invalid_string_tokens(cx: &mut TestAppContext) {
    init(cx);
    let (editor, cx) = editor_for(
        || TextModel::new("x = /(/; f(1) '(' g()").with_language("clike"),
        cx,
    );
    cx.executor()
        .advance_clock(std::time::Duration::from_millis(200));
    cx.run_until_parked();
    let (colors, ranges) = editor.read_with(cx, |e, cx| {
        (e.colors().clone(), e.bracket_colors_for_tests(cx))
    });
    let first = colors.bracket_color(0).unwrap();
    assert_eq!(
        ranges,
        vec![
            (10..11, first),
            (12..13, first),
            (19..20, first),
            (20..21, first),
        ]
    );
}

fn p(row: u32, column: u32) -> PointUtf16 {
    PointUtf16::new(row, column)
}

/// Monaco reads a language's configuration by id at each use
/// (`getLanguageConfiguration(model.getLanguageIdAtPosition(...))`), so
/// `setLanguageConfiguration` applies to embedded regions at once (§F.5).
#[gpui::test]
fn injected_languages_use_their_current_configuration(cx: &mut TestAppContext) {
    cx.update(|cx| {
        gpui_monaco::init(cx);
        gpui_monaco::set_language_registry(cx, gpui_monaco_languages::registry());
    });
    let (editor, cx) = editor_for(
        || TextModel::new("<script>\nlet x;\n</script>").with_language("html"),
        cx,
    );
    let offset = "<script>\n".len();
    let comments = |editor: &Entity<CodeEditor>, cx: &mut VisualTestContext| {
        editor.read_with(cx, |e, cx| {
            let language = e.model().read(cx).language_at(offset).unwrap();
            assert_eq!(&*language.id, "javascript");
            language.config.comments().line_comment.clone()
        })
    };
    assert_eq!(comments(&editor, cx).as_deref(), Some("//"));
    let registration = cx
        .update(|_, cx| {
            gpui_monaco::set_language_configuration(
                cx,
                "javascript",
                LanguageConfiguration {
                    comments: Some(Comments {
                        line_comment: Some("#".into()),
                        ..Default::default()
                    }),
                    ..Default::default()
                },
            )
        })
        .unwrap();
    cx.run_until_parked();
    assert_eq!(comments(&editor, cx).as_deref(), Some("#"));
    editor.update(cx, |e, cx| {
        e.set_selections(&[Selection::collapsed(p(1, 1))], cx)
    });
    cx.simulate_keystrokes("secondary-/");
    let text = editor.read_with(cx, |e, cx| e.model().read(cx).text());
    assert_eq!(text, "<script>\n# let x;\n</script>");
    // Disposing restores the language's own comments.
    cx.update(|_, cx| registration.dispose(cx));
    cx.run_until_parked();
    assert_eq!(comments(&editor, cx).as_deref(), Some("//"));
}

fn monarch(token: &str) -> gpui_monaco::LanguageTokenizer {
    gpui_monaco::LanguageTokenizer::Monarch(json!({ "tokenizer": { "root": [["[a-z]+", token]] } }))
}

/// Monaco `TokenizationRegistry.registerFactory`: a language has one
/// factory, and a new one disposes the previous, so only the latest is
/// called (§F.6).
#[gpui::test]
fn a_new_tokens_provider_factory_replaces_the_previous(cx: &mut TestAppContext) {
    init(cx);
    let calls = Rc::new(Cell::new((0, 0)));
    let (tx, rx) = async_channel::bounded::<()>(1);
    let first = {
        let calls = calls.clone();
        cx.update(|cx| {
            gpui_monaco::register_tokens_provider_factory(cx, "plain", move |cx| {
                calls.set((calls.get().0 + 1, calls.get().1));
                // Resolves after the second.
                cx.background_spawn(async move {
                    rx.recv().await.ok();
                    Ok(Some(monarch("first")))
                })
            })
        })
    };
    let second = {
        let calls = calls.clone();
        cx.update(|cx| {
            gpui_monaco::register_tokens_provider_factory(cx, "plain", move |_| {
                calls.set((calls.get().0, calls.get().1 + 1));
                gpui::Task::ready(Ok(Some(monarch("second"))))
            })
        })
    };
    let model = shown("abc", "plain", 20_000, cx);
    tx.try_send(()).ok();
    cx.run_until_parked();
    assert_eq!(calls.get(), (0, 1));
    assert_eq!(name_at(&model, 0, cx).as_deref(), Some("second.plain"));
    // Disposing the replaced factory does nothing.
    cx.update(|cx| first.dispose(cx));
    cx.run_until_parked();
    assert_eq!(name_at(&model, 0, cx).as_deref(), Some("second.plain"));

    // Once the language is in use, a new factory removes the tokenizer
    // the previous one installed, and its own is created.
    let third = cx.update(|cx| {
        gpui_monaco::register_tokens_provider_factory(cx, "plain", |_| {
            gpui::Task::ready(Ok(Some(monarch("third"))))
        })
    });
    cx.run_until_parked();
    assert_eq!(name_at(&model, 0, cx).as_deref(), Some("third.plain"));
    cx.update(|cx| second.dispose(cx));
    cx.run_until_parked();
    assert_eq!(name_at(&model, 0, cx).as_deref(), Some("third.plain"));
    cx.update(|cx| third.dispose(cx));
    cx.run_until_parked();
    assert_eq!(name_at(&model, 0, cx), None);
}
