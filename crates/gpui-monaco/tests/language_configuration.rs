//! Language configurations without a grammar, Monaco's `plaintext`
//! defaults, languages registered after models use them, MIME types,
//! standard token types and tokens provider factories (#326).

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::Arc;

use gpui::{AppContext as _, Entity, Focusable, TestAppContext, VisualTestContext};
use gpui_monaco::cursor::{Selection, StandardTokenType};
use gpui_monaco::syntax::{Comments, Language, LanguageConfiguration, LanguageRegistry, Pattern};
use gpui_monaco::text::PointUtf16;
use gpui_monaco::{CodeEditor, TextModel, TextModelEvent};
use serde_json::json;

fn p(row: u32, column: u32) -> PointUtf16 {
    PointUtf16::new(row, column)
}

fn init(registry: Arc<LanguageRegistry>, cx: &mut TestAppContext) {
    cx.update(|cx| {
        gpui_monaco::init(cx);
        gpui_monaco::set_language_registry(cx, registry);
    });
}

/// `mylang` from the audit's repro: brackets `()` only, `//` comments and
/// a word pattern, and no grammar or tokenizer.
fn mylang() -> Language {
    Language::new("mylang", "My").with_config(LanguageConfiguration {
        brackets: Some(vec![("(".into(), ")".into())]),
        comments: Some(Comments {
            line_comment: Some("//".into()),
            ..Default::default()
        }),
        word_pattern: Some(Pattern::new(r"[a-z]+(-[a-z]+)*", "")),
        ..Default::default()
    })
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

fn text(editor: &Entity<CodeEditor>, cx: &mut VisualTestContext) -> String {
    editor.read_with(cx, |editor, cx| editor.model().read(cx).text())
}

fn select(editor: &Entity<CodeEditor>, selection: Selection, cx: &mut VisualTestContext) {
    editor.update(cx, |e, cx| e.set_selections(&[selection], cx));
    cx.run_until_parked();
}

/// The ranges of the editor's bracket match decorations.
fn bracket_highlights(
    editor: &Entity<CodeEditor>,
    cx: &mut VisualTestContext,
) -> Vec<std::ops::Range<usize>> {
    let id = editor.entity_id();
    let model = editor.read_with(cx, |e, _| e.model().clone());
    model.update(cx, |m, _| {
        m.decorations_in_range(0..1000, Some(id))
            .into_iter()
            .map(|d| d.range)
            .collect()
    })
}

/// Typing `(`, surrounding a selection, Enter between brackets and
/// bracket matching.
fn assert_brackets_work(editor: &Entity<CodeEditor>, cx: &mut VisualTestContext) {
    // `(` auto-closes (the brackets, without `autoClosingPairs`).
    cx.simulate_input("(");
    assert_eq!(text(editor, cx), "()");
    // And the pair is matched.
    assert_eq!(bracket_highlights(editor, cx), vec![0..1, 1..2]);
    // Enter between them indents the middle line.
    cx.simulate_keystrokes("enter");
    assert_eq!(text(editor, cx), "(\n    \n)");
    // A selection is surrounded.
    cx.simulate_input("foo");
    select(editor, Selection::new(p(1, 4), p(1, 7)), cx);
    cx.simulate_input("(");
    assert_eq!(text(editor, cx), "(\n    (foo)\n)");
}

/// Monaco `plaintext`'s configuration for an editor without a language:
/// the brackets auto-close and drive Enter, and the surrounding pairs
/// include the quotes.
#[gpui::test]
fn an_editor_without_a_language_has_the_plaintext_configuration(cx: &mut TestAppContext) {
    // No registry at all: the built-in plain text.
    cx.update(gpui_monaco::init);
    let (editor, cx) = editor_for(|| TextModel::new(""), cx);
    let id = editor.read_with(cx, |e, cx| {
        e.model().read(cx).language().unwrap().id.clone()
    });
    assert_eq!(&*id, "plaintext");
    assert_brackets_work(&editor, cx);
    // `{` and `[` close too; `"` surrounds but doesn't auto-close.
    editor.update(cx, |e, cx| e.set_value("", cx));
    cx.simulate_input("{");
    assert_eq!(text(&editor, cx), "{}");
    cx.simulate_keystrokes("enter");
    assert_eq!(text(&editor, cx), "{\n    \n}");
    editor.update(cx, |e, cx| e.set_value("foo", cx));
    select(&editor, Selection::new(p(0, 0), p(0, 3)), cx);
    cx.simulate_input("\"");
    assert_eq!(text(&editor, cx), "\"foo\"");
    select(&editor, Selection::new(p(0, 1), p(0, 4)), cx);
    cx.simulate_input("<");
    assert_eq!(text(&editor, cx), "\"<foo>\"");
    editor.update(cx, |e, cx| e.set_value("", cx));
    cx.simulate_input("\"");
    assert_eq!(text(&editor, cx), "\"");
}

/// The bundled `plaintext` (`gpui_monaco_languages::plain_text`) is
/// Monaco's: `.txt`, the alias `text`, `text/plain` and its configuration.
#[gpui::test]
fn the_bundled_plain_text_is_monacos(cx: &mut TestAppContext) {
    let plain = gpui_monaco_languages::plain_text();
    assert_eq!(plain.extensions, vec!["txt"]);
    assert_eq!(plain.aliases, vec!["text"]);
    assert_eq!(plain.mimetypes, vec!["text/plain"]);
    init(gpui_monaco_languages::registry(), cx);
    let (editor, cx) = editor_for(|| TextModel::new("").with_language("plaintext"), cx);
    assert_brackets_work(&editor, cx);
    // Folding is off-side and brackets aren't colorized.
    let config = editor.read_with(cx, |e, cx| {
        e.model().read(cx).language().unwrap().config.clone()
    });
    assert!(config.folding.is_some_and(|folding| folding.off_side));
    assert_eq!(config.colorized_bracket_pairs, Some(Vec::new()));
}

/// A model whose language isn't registered is plain text, keeping its id
/// (Monaco reports `plaintext`).
#[gpui::test]
fn an_unregistered_language_is_plain_text(cx: &mut TestAppContext) {
    init(gpui_monaco_languages::registry(), cx);
    let (editor, cx) = editor_for(|| TextModel::new("").with_language("unknown"), cx);
    let (id, language_id) = editor.read_with(cx, |e, cx| {
        let model = e.model().read(cx);
        (
            model.language().unwrap().id.clone(),
            model.language_id().cloned(),
        )
    });
    assert_eq!(&*id, "plaintext");
    assert_eq!(language_id.as_deref(), Some("unknown"));
    assert_brackets_work(&editor, cx);
}

/// `plaintext`'s configuration can be changed like any language's, and
/// models without a language pick it up.
#[gpui::test]
fn set_language_configuration_applies_to_plain_text(cx: &mut TestAppContext) {
    init(gpui_monaco_languages::registry(), cx);
    let (editor, cx) = editor_for(|| TextModel::new(""), cx);
    let _config = cx
        .update(|_, cx| {
            gpui_monaco::set_language_configuration(
                cx,
                "plaintext",
                LanguageConfiguration {
                    auto_closing_pairs: Some(Vec::new()),
                    ..Default::default()
                },
            )
        })
        .unwrap();
    cx.simulate_input("(");
    assert_eq!(text(&editor, cx), "(");
}

/// Monaco's repro: a language registered with a configuration and no
/// grammar gets auto-closing, surround, Enter between brackets, bracket
/// matching, Toggle Line Comment and its word pattern.
#[gpui::test]
fn a_language_without_a_grammar_gets_its_configuration(cx: &mut TestAppContext) {
    let registry = Arc::new(LanguageRegistry::new());
    registry.register(mylang());
    init(registry, cx);
    let (editor, cx) = editor_for(|| TextModel::new("").with_language("mylang"), cx);
    assert_brackets_work(&editor, cx);
    // Only `()` are its brackets.
    editor.update(cx, |e, cx| e.set_value("", cx));
    cx.simulate_input("[");
    assert_eq!(text(&editor, cx), "[");
    // Toggle Line Comment.
    editor.update(cx, |e, cx| e.set_value("ab-cd x", cx));
    select(&editor, Selection::collapsed(p(0, 1)), cx);
    cx.simulate_keystrokes("secondary-/");
    assert_eq!(text(&editor, cx), "// ab-cd x");
    // The word pattern.
    let word = editor.read_with(cx, |e, cx| {
        e.model()
            .read(cx)
            .word_at_position(p(0, 4))
            .map(|word| word.word)
    });
    assert_eq!(word.as_deref(), Some("ab-cd"));
}

/// The configuration applies while a tokens provider factory's task is
/// still pending.
#[gpui::test]
fn the_configuration_applies_while_a_tokenizer_is_pending(cx: &mut TestAppContext) {
    let registry = Arc::new(LanguageRegistry::new());
    registry.register(mylang());
    init(registry, cx);
    let _factory = cx.update(|cx| {
        gpui_monaco::register_tokens_provider_factory(cx, "mylang", |cx| {
            cx.background_executor().spawn(std::future::pending())
        })
    });
    let (editor, cx) = editor_for(|| TextModel::new("").with_language("mylang"), cx);
    cx.simulate_input("(");
    assert_eq!(text(&editor, cx), "()");
}

/// Monaco's `LanguageSelection`: a model created with an id before the
/// language is registered gets it when it is, emitting
/// `LanguageConfigurationChanged`, and highlights with its grammar.
#[gpui::test]
fn registering_a_language_later_updates_models_using_its_id(cx: &mut TestAppContext) {
    init(Arc::new(LanguageRegistry::new()), cx);
    let (editor, cx) = editor_for(|| TextModel::new("").with_language("mylang"), cx);
    let model = editor.read_with(cx, |e, _| e.model().clone());
    let events = Rc::new(RefCell::new(Vec::new()));
    let seen = events.clone();
    let _subscription = cx.update(|_, cx| {
        cx.subscribe(&model, move |_, event: &TextModelEvent, _| match event {
            TextModelEvent::LanguageConfigurationChanged => seen.borrow_mut().push("config"),
            TextModelEvent::LanguageChanged { .. } => seen.borrow_mut().push("language"),
            _ => {}
        })
    });
    // Plain text until then: `[` closes.
    cx.simulate_input("[");
    assert_eq!(text(&editor, cx), "[]");
    editor.update(cx, |e, cx| e.set_value("", cx));
    let grammar = json!({ "tokenizer": { "root": [["[a-z]+", "keyword"]] } });
    cx.update(|_, cx| {
        gpui_monaco::register_language(cx, mylang().with_monarch_grammar(&grammar).unwrap())
    });
    cx.run_until_parked();
    assert_eq!(*events.borrow(), vec!["config"]);
    let id = model.read_with(cx, |m, _| m.language().unwrap().id.clone());
    assert_eq!(&*id, "mylang");
    cx.simulate_input("[(");
    assert_eq!(text(&editor, cx), "[()");
    editor.update(cx, |e, cx| e.set_value("abc", cx));
    cx.run_until_parked();
    let name = model.read_with(cx, |m, _| m.highlight_name_at(0));
    assert_eq!(name.as_deref(), Some("keyword.mylang"));
    // Registering another language leaves the model alone.
    cx.update(|_, cx| gpui_monaco::register_language(cx, Language::new("other", "Other")));
    cx.run_until_parked();
    assert_eq!(*events.borrow(), vec!["config"]);
}

/// A model with a uri and no language guesses again when languages are
/// registered (Monaco `createByFilepathOrFirstLine`), emitting
/// `LanguageChanged`; one given a language doesn't.
#[gpui::test]
fn registering_a_language_later_updates_models_guessing_it(cx: &mut TestAppContext) {
    init(Arc::new(LanguageRegistry::new()), cx);
    let guessed = cx.new(|_| TextModel::new("").with_uri("file:///a/b.my"));
    let explicit = cx.new(|_| TextModel::new("").with_uri("file:///a/c.my"));
    let changes = Rc::new(Cell::new(0));
    let count = changes.clone();
    let _subscription = cx.update(|cx| {
        cx.subscribe(&guessed, move |_, event: &TextModelEvent, _| {
            if let TextModelEvent::LanguageChanged {
                old_language,
                new_language,
            } = event
            {
                assert_eq!(old_language, &None);
                assert_eq!(new_language.as_deref(), Some("mylang"));
                count.set(count.get() + 1);
            }
        })
    });
    guessed.update(cx, |m, cx| m.ensure_syntax(cx));
    explicit.update(cx, |m, cx| {
        m.ensure_syntax(cx);
        m.set_language(None, cx);
    });
    assert_eq!(guessed.read_with(cx, |m, _| m.language_id().cloned()), None);
    cx.update(|cx| gpui_monaco::register_language(cx, mylang().with_extensions(&["my"])));
    cx.run_until_parked();
    assert_eq!(changes.get(), 1);
    let id = guessed.read_with(cx, |m, _| m.language().unwrap().id.clone());
    assert_eq!(&*id, "mylang");
    // `set_language(None)` chose plain text.
    let id = explicit.read_with(cx, |m, _| m.language().unwrap().id.clone());
    assert_eq!(&*id, "plaintext");
}

/// Monaco `_mergeLanguage`: a language without MIME types gets
/// `text/x-<id>`, and the language registered last wins a MIME type.
#[gpui::test]
fn mime_types(cx: &mut TestAppContext) {
    init(gpui_monaco_languages::registry(), cx);
    let model = cx.new(|_| TextModel::new(""));
    model.update(cx, |m, cx| m.set_language(Some("text/x-rust".into()), cx));
    assert_eq!(
        model
            .read_with(cx, |m, _| m.language_id().cloned())
            .as_deref(),
        Some("rust")
    );
    cx.update(|cx| {
        gpui_monaco::register_language(cx, mylang());
        gpui_monaco::register_language(
            cx,
            Language::new("first", "First").with_mimetypes(&["text/shared"]),
        );
        gpui_monaco::register_language(
            cx,
            Language::new("last", "Last").with_mimetypes(&["text/shared"]),
        );
    });
    let registry = cx.update(|cx| gpui_monaco::language_registry(cx));
    let mylang = registry.language_for_id("mylang").unwrap();
    assert_eq!(mylang.mimetypes, vec!["text/x-mylang"]);
    let shared = registry.language_for_mime_type("text/shared").unwrap();
    assert_eq!(&*shared.id, "last");
    model.update(cx, |m, cx| m.set_language(Some("text/x-mylang".into()), cx));
    assert_eq!(
        model
            .read_with(cx, |m, _| m.language_id().cloned())
            .as_deref(),
        Some("mylang")
    );
    // A model created with a MIME type before its language is registered
    // gets the language's id then.
    let late = cx.new(|_| TextModel::new("").with_language("text/x-later"));
    late.update(cx, |m, cx| m.ensure_syntax(cx));
    cx.update(|cx| gpui_monaco::register_language(cx, Language::new("later", "Later")));
    assert_eq!(
        late.read_with(cx, |m, _| m.language_id().cloned())
            .as_deref(),
        Some("later")
    );
}

/// Monaco `toStandardTokenType`: the first of `comment`, `string`, `regex`
/// and `regexp` as a whole word anywhere in the token.
#[gpui::test]
fn standard_token_types_match_whole_words(cx: &mut TestAppContext) {
    let grammar = json!({
        "tokenizer": {
            "root": [
                ["a", "invalid.string"],
                ["b", "meta.comment"],
                ["c", "string.regexp"],
                ["d", "regexp"],
                ["e", "strings"],
                ["f", "comment.string"],
            ],
        },
    });
    let registry = Arc::new(LanguageRegistry::new());
    registry.register(mylang().with_monarch_grammar(&grammar).unwrap());
    init(registry, cx);
    let model = cx.new(|_| TextModel::new("abcdef").with_language("mylang"));
    model.update(cx, |m, cx| m.ensure_syntax(cx));
    cx.run_until_parked();
    let types: Vec<_> = model.read_with(cx, |m, _| (0..6).map(|i| m.token_type_at(i)).collect());
    assert_eq!(
        types,
        vec![
            StandardTokenType::String,
            StandardTokenType::Comment,
            StandardTokenType::String,
            StandardTokenType::RegEx,
            StandardTokenType::Other,
            StandardTokenType::Comment,
        ]
    );
}

/// Monaco `TokenizationRegistry.getOrCreate`: a factory isn't called when
/// the language already has a tokenizer.
#[gpui::test]
fn a_tokens_provider_factory_keeps_an_existing_tokenizer(cx: &mut TestAppContext) {
    let registry = Arc::new(LanguageRegistry::new());
    registry.register(mylang());
    init(registry, cx);
    let monarch = json!({ "tokenizer": { "root": [["[a-z]+", "keyword"]] } });
    let _monarch = cx
        .update(|cx| gpui_monaco::set_monarch_tokens_provider(cx, "mylang", &monarch))
        .unwrap();
    let called = Rc::new(Cell::new(false));
    let flag = called.clone();
    let _factory = cx.update(|cx| {
        gpui_monaco::register_tokens_provider_factory(cx, "mylang", move |_| {
            flag.set(true);
            let other = json!({ "tokenizer": { "root": [["[a-z]+", "identifier"]] } });
            gpui::Task::ready(Ok(Some(gpui_monaco::LanguageTokenizer::Monarch(other))))
        })
    });
    let model = cx.new(|_| TextModel::new("abc").with_language("mylang"));
    model.update(cx, |m, cx| m.ensure_syntax(cx));
    cx.run_until_parked();
    assert!(!called.get());
    let name = model.read_with(cx, |m, _| m.highlight_name_at(0));
    assert_eq!(name.as_deref(), Some("keyword.mylang"));
}

/// The editor's own inputs stand in for Monaco's HTML inputs: they aren't
/// plain text, so the find input doesn't auto-close.
#[gpui::test]
fn widget_inputs_are_not_plain_text(cx: &mut TestAppContext) {
    cx.update(gpui_monaco::init);
    let (editor, cx) = editor_for(|| TextModel::new("f(x)"), cx);
    cx.simulate_keystrokes("secondary-f");
    cx.simulate_keystrokes("secondary-a");
    cx.simulate_input("(");
    cx.executor()
        .advance_clock(std::time::Duration::from_millis(600));
    cx.run_until_parked();
    let query = editor.read_with(cx, |e, _| e.find_options().query.clone());
    assert_eq!(query, "(");
}
