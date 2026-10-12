//! A model's language as Monaco's language service gives it: `plaintext`
//! for none, resolved when the model is created, and canonical ids.

use std::cell::RefCell;
use std::rc::Rc;

use gpui::{App, AppContext as _, Entity, Task, TestAppContext};
use gpui_monaco::providers::*;
use gpui_monaco::syntax::Language;
use gpui_monaco::text::PointUtf16;
use gpui_monaco::{CodeEditor, TextModel, api_commands};

/// One link covering the first character, to tell providers apart by url.
struct Link(&'static str);
impl LinkProvider for Link {
    fn provide_links(
        &self,
        _: &ProviderContext,
        _: &mut App,
    ) -> Task<anyhow::Result<Vec<DocumentLink>>> {
        Task::ready(Ok(vec![DocumentLink {
            range: PointUtf16::new(0, 0)..PointUtf16::new(0, 1),
            url: Some(self.0.into()),
            tooltip: None,
        }]))
    }
}

/// The urls of the model's links, from `_executeLinkProvider`.
fn link_urls(model: &Entity<TextModel>, cx: &mut TestAppContext) -> Vec<String> {
    let task = cx.update(|cx| api_commands::execute_link_provider(model, 0, cx));
    cx.run_until_parked();
    let mut urls: Vec<String> = cx
        .foreground_executor()
        .block_on(task)
        .into_iter()
        .filter_map(|link| link.url.map(|url| url.to_string()))
        .collect();
    urls.sort();
    urls
}

fn language_id(model: &Entity<TextModel>, cx: &mut TestAppContext) -> Option<String> {
    model.read_with(cx, |m, _| m.language_id().map(|id| id.to_string()))
}

fn register_rust(cx: &mut App) {
    gpui_monaco::register_language(
        cx,
        Language::new("rust", "Rust")
            .with_aliases(&["Rust", "rs"])
            .with_extensions(&["rs"]),
    );
}

// Monaco `_createAndGetLanguageIdentifier`: no language is `plaintext`.
#[gpui::test]
fn a_model_without_a_language_is_plaintext(cx: &mut TestAppContext) {
    cx.update(|cx| {
        gpui_monaco::init(cx);
        register_link_provider(cx, LanguageSelector::language("plaintext"), Link("plain"));
        register_link_provider(
            cx,
            LanguageSelector::Filters(vec![LanguageFilter {
                scheme: Some("inmemory".into()),
                ..Default::default()
            }]),
            Link("inmemory"),
        );
        register_link_provider(cx, LanguageSelector::language("rust"), Link("rust"));
    });
    let model = cx.update(|cx| gpui_monaco::create_model(cx, "hi", None, None).unwrap());
    assert_eq!(language_id(&model, cx).as_deref(), Some("plaintext"));
    assert_eq!(link_urls(&model, cx), ["inmemory", "plain"]);
    model.read_with(cx, |m, _| {
        let target = ProviderTarget::from(m);
        assert_eq!(target.language_id.as_deref(), Some("plaintext"));
        assert_eq!(target.uri.as_ref(), Some(m.resource()));
        assert_eq!(
            m.provider_context().language_id.as_deref(),
            Some("plaintext")
        );
    });

    // A model built with `TextModel::new` and shown in an editor.
    let (editor, cx) = cx.add_window_view(|window, cx| {
        let model = cx.new(|_| TextModel::new("hi"));
        CodeEditor::new(model, window, cx)
    });
    cx.run_until_parked();
    let lang = editor.read_with(cx, |e, cx| {
        e.context_keys(cx)
            .get("editorLangId")
            .map(|v| v.to_string())
    });
    assert_eq!(lang.as_deref(), Some("plaintext"));
}

// Monaco `createModel`: the language is resolved, from the uri when none is
// given, before anything shows the model.
#[gpui::test]
fn create_model_resolves_the_language_right_away(cx: &mut TestAppContext) {
    let requested = Rc::new(RefCell::new(Vec::<String>::new()));
    let created = Rc::new(RefCell::new(Vec::<Option<String>>::new()));
    cx.update(|cx| {
        gpui_monaco::init(cx);
        register_rust(cx);
        let requested_ = requested.clone();
        let _ = gpui_monaco::on_language(cx, "rust", move |cx| {
            requested_.borrow_mut().push("rust".into());
            register_link_provider(cx, LanguageSelector::language("rust"), Link("rust"));
        });
        let created = created.clone();
        gpui_monaco::on_did_create_model(cx, move |model, cx| {
            let id = model.read(cx).language_id().map(|id| id.to_string());
            created.borrow_mut().push(id);
        })
        .detach();
    });
    let model = cx.update(|cx| {
        gpui_monaco::create_model(cx, "fn main() {}", None, Some("file:///a.rs")).unwrap()
    });
    assert_eq!(language_id(&model, cx).as_deref(), Some("rust"));
    model.read_with(cx, |m, _| assert_eq!(&*m.language().unwrap().id, "rust"));
    cx.run_until_parked();
    // `onLanguage` ran, so its link provider answers for the headless model.
    assert_eq!(*requested.borrow(), ["rust"]);
    assert_eq!(*created.borrow(), [Some("rust".to_string())]);
    assert_eq!(link_urls(&model, cx), ["rust"]);

    // With a language id, `onLanguage` fires at creation too.
    let fired = Rc::new(RefCell::new(0));
    cx.update(|cx| {
        let fired = fired.clone();
        let _ = gpui_monaco::on_language(cx, "toml", move |_| *fired.borrow_mut() += 1);
        gpui_monaco::create_model(cx, "a = 1", Some("toml"), None).unwrap();
    });
    cx.run_until_parked();
    assert_eq!(*fired.borrow(), 1);
}

// Monaco `createTextModel` guesses from the first line without a uri.
#[gpui::test]
fn create_model_guesses_from_the_first_line_without_a_uri(cx: &mut TestAppContext) {
    cx.update(|cx| {
        gpui_monaco::init(cx);
        gpui_monaco::register_language(
            cx,
            Language::new("shell", "Shell").with_first_line(r"^#!.*\bbash"),
        );
    });
    let model = cx.update(|cx| {
        gpui_monaco::create_model(cx, "#!/usr/bin/env bash\necho", None, None).unwrap()
    });
    assert_eq!(language_id(&model, cx).as_deref(), Some("shell"));
}

// Models resolve their language when created, but parse only when shown.
#[gpui::test]
fn models_parse_only_once_shown(cx: &mut TestAppContext) {
    cx.update(|cx| {
        gpui_monaco::init(cx);
        gpui_monaco::set_language_registry(cx, gpui_monaco_languages::registry());
    });
    let model = cx.new(|_| TextModel::new("fn main() {}").with_language("rust"));
    cx.run_until_parked();
    model.read_with(cx, |m, _| {
        assert_eq!(m.language_id().map(|id| id.as_ref()), Some("rust"));
        assert!(m.language().is_some_and(|l| l.has_highlighter()));
        assert!(m.syntax_snapshot().is_none());
    });
    model.update(cx, |m, cx| m.edit([(0..0, "// x\n")], cx).unwrap());
    cx.run_until_parked();
    assert!(model.read_with(cx, |m, _| m.syntax_snapshot().is_none()));
    model.update(cx, |m, cx| m.ensure_syntax(cx));
    cx.run_until_parked();
    assert!(model.read_with(cx, |m, _| m.syntax_snapshot().is_some()));
}

// Monaco only takes ids and MIME types; the port also takes a language's
// name, alias or extension, and gives the model the language's id.
#[gpui::test]
fn names_aliases_and_extensions_give_the_language_id(cx: &mut TestAppContext) {
    cx.update(|cx| {
        gpui_monaco::init(cx);
        register_rust(cx);
    });
    for given in ["Rust", "RUST", "rs", "text/x-rust"] {
        let model = cx.new(|_| TextModel::new("fn main() {}").with_language(given));
        assert_eq!(language_id(&model, cx).as_deref(), Some("rust"), "{given}");
        model.update(cx, |m, cx| m.set_language(None, cx));
        assert_eq!(language_id(&model, cx).as_deref(), Some("plaintext"));
        model.update(cx, |m, cx| m.set_language(Some(given.into()), cx));
        assert_eq!(language_id(&model, cx).as_deref(), Some("rust"), "{given}");
    }
    // An unregistered id is kept, so the model takes it once registered.
    let model = cx.new(|_| TextModel::new("x").with_language("python"));
    assert_eq!(language_id(&model, cx).as_deref(), Some("python"));
    model.read_with(cx, |m, _| {
        assert_eq!(&*m.language().unwrap().id, Language::PLAIN_TEXT_ID)
    });
}
