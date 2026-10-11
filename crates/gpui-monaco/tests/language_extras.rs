//! Monaco's `monaco.languages` extras: guessing a model's language,
//! `onLanguage`, `setLanguageConfiguration`, language filters, the
//! signature help context, provider change events and multi-range
//! formatting.

use std::cell::{Cell, RefCell};
use std::ops::Range;
use std::rc::Rc;

use gpui::{App, AppContext as _, Entity, Focusable, Task, TestAppContext, VisualTestContext};
use gpui_monaco::cursor::Selection;
use gpui_monaco::providers::*;
use gpui_monaco::text::PointUtf16;
use gpui_monaco::{CodeEditor, TextModel, TextModelEvent};

fn p(row: u32, column: u32) -> PointUtf16 {
    PointUtf16::new(row, column)
}

fn init(cx: &mut TestAppContext) {
    cx.update(|cx| {
        gpui_monaco::init(cx);
        gpui_monaco::set_language_registry(cx, gpui_monaco_languages::registry());
    });
}

fn language_of(model: &Entity<TextModel>, cx: &mut TestAppContext) -> Option<String> {
    model.update(cx, |m, cx| {
        m.ensure_syntax(cx);
        m.language_id().map(|id| id.to_string())
    })
}

#[gpui::test]
fn models_guess_their_language_from_the_uri_and_first_line(cx: &mut TestAppContext) {
    init(cx);
    let by_path = cx.new(|_| TextModel::new("fn main() {}").with_uri("file:///src/main.rs"));
    assert_eq!(language_of(&by_path, cx).as_deref(), Some("rust"));
    let by_shebang = cx
        .new(|_| TextModel::new("#!/usr/bin/env node\nconsole.log(1)").with_uri("untitled:script"));
    assert_eq!(language_of(&by_shebang, cx).as_deref(), Some("javascript"));
    let by_pattern = cx.new(|_| TextModel::new("{}").with_uri("file:///p/tsconfig.base.json"));
    assert_eq!(language_of(&by_pattern, cx).as_deref(), Some("json"));
    // A language given explicitly wins, and unknown files stay plain.
    let explicit = cx.new(|_| {
        TextModel::new("x")
            .with_language("css")
            .with_uri("file:///a.rs")
    });
    assert_eq!(language_of(&explicit, cx).as_deref(), Some("css"));
    let unknown = cx.new(|_| TextModel::new("x").with_uri("file:///a.unknown"));
    assert_eq!(language_of(&unknown, cx), None);
}

#[gpui::test]
fn on_language_runs_once_for_the_first_model(cx: &mut TestAppContext) {
    init(cx);
    let calls = Rc::new(Cell::new(0));
    let counter = calls.clone();
    cx.update(|cx| gpui_monaco::on_language(cx, "rust", move |_| counter.set(counter.get() + 1)));
    let first = cx.new(|_| TextModel::new("").with_language("rust"));
    language_of(&first, cx);
    cx.run_until_parked();
    assert_eq!(calls.get(), 1);
    let second = cx.new(|_| TextModel::new("").with_language("rust"));
    language_of(&second, cx);
    cx.run_until_parked();
    assert_eq!(calls.get(), 1);
    // As in Monaco, a language already used doesn't call back.
    let late = calls.clone();
    cx.update(|cx| gpui_monaco::on_language(cx, "rust", move |_| late.set(late.get() + 10)));
    cx.run_until_parked();
    assert_eq!(calls.get(), 1);
}

#[gpui::test]
fn set_language_configuration_at_runtime(cx: &mut TestAppContext) {
    init(cx);
    let model = cx.new(|_| TextModel::new("a <b> c").with_language("rust"));
    language_of(&model, cx);
    let events = Rc::new(RefCell::new(Vec::new()));
    let seen = events.clone();
    let _subscription = cx.update(|cx| {
        cx.subscribe(&model, move |_, event: &TextModelEvent, _| {
            if matches!(event, TextModelEvent::LanguageConfigurationChanged) {
                seen.borrow_mut().push(());
            }
        })
    });
    let mut config = model.read_with(cx, |m, _| m.language().unwrap().config.clone());
    config.brackets = vec![("<".into(), ">".into())];
    assert!(cx.update(|cx| gpui_monaco::set_language_configuration(cx, "rust", config)));
    cx.run_until_parked();
    assert_eq!(events.borrow().len(), 1);
    model.read_with(cx, |m, _| {
        assert_eq!(
            m.language().unwrap().config.brackets,
            vec![("<".to_string(), ">".to_string())]
        );
    });
    assert!(!cx.update(|cx| gpui_monaco::set_language_configuration(
        cx,
        "nope",
        Default::default()
    )));
}

struct Hovers;
impl HoverProvider for Hovers {
    fn provide_hover(
        &self,
        _: &ProviderContext,
        _: PointUtf16,
        _: &mut App,
    ) -> Task<anyhow::Result<Option<Hover>>> {
        Task::ready(Ok(None))
    }
}

#[gpui::test]
fn language_filters_match_scheme_and_pattern(cx: &mut TestAppContext) {
    init(cx);
    let filter = |language: Option<&str>, scheme: Option<&str>, pattern: Option<&str>| {
        LanguageSelector::Filters(vec![LanguageFilter {
            language: language.map(|s| s.to_string().into()),
            scheme: scheme.map(|s| s.to_string().into()),
            pattern: pattern.map(|s| s.to_string().into()),
        }])
    };
    let test_file = cx.new(|_| {
        TextModel::new("")
            .with_language("typescript")
            .with_uri("file:///src/a.test.ts")
    });
    let untitled = cx.new(|_| {
        TextModel::new("")
            .with_language("typescript")
            .with_uri("untitled:Untitled-1")
    });
    let matches =
        |selector: &LanguageSelector, model: &Entity<TextModel>, cx: &mut TestAppContext| {
            model.read_with(cx, |m, _| selector.matches_target(&m.into()))
        };
    let tests = filter(None, Some("file"), Some("**/*.test.ts"));
    assert!(matches(&tests, &test_file, cx));
    assert!(!matches(&tests, &untitled, cx));
    let any_ts = filter(Some("typescript"), Some("*"), None);
    assert!(matches(&any_ts, &untitled, cx));
    assert!(!matches(&filter(Some("rust"), None, None), &test_file, cx));
    // An empty filter selects nothing, as in Monaco.
    assert!(!matches(&filter(None, None, None), &test_file, cx));
    // Provider lookups see the model's uri.
    cx.update(|cx| register_hover_provider(cx, tests.clone(), Hovers));
    let found = |model: &Entity<TextModel>, cx: &mut TestAppContext| {
        cx.update(|cx| hover_providers(cx, model.read(cx)).len())
    };
    assert_eq!(found(&test_file, cx), 1);
    assert_eq!(found(&untitled, cx), 0);
}

fn editor_with<'a>(
    text: &str,
    cx: &'a mut TestAppContext,
) -> (Entity<CodeEditor>, &'a mut VisualTestContext) {
    let text = text.to_string();
    let (editor, cx) = cx.add_window_view(|window, cx| {
        let model = cx.new(|_| TextModel::new(&text));
        CodeEditor::new(model, window, cx)
    });
    cx.update(|window, cx| window.focus(&editor.focus_handle(cx), cx));
    cx.run_until_parked();
    (editor, cx)
}

fn draw(cx: &mut VisualTestContext) {
    cx.update(|window, _| window.refresh());
    cx.run_until_parked();
}

/// Records each request's context, answering with one signature.
struct Signatures(Rc<RefCell<Vec<SignatureHelpContext>>>);
impl SignatureHelpProvider for Signatures {
    fn provide_signature_help(
        &self,
        _: &ProviderContext,
        _: PointUtf16,
        _: SignatureHelpTrigger,
        _: &mut App,
    ) -> Task<anyhow::Result<Option<SignatureHelp>>> {
        unreachable!("called with the context")
    }

    fn provide_signature_help_with_context(
        &self,
        _: &ProviderContext,
        _: PointUtf16,
        help_context: &SignatureHelpContext,
        _: &mut App,
    ) -> Task<anyhow::Result<Option<SignatureHelp>>> {
        self.0.borrow_mut().push(help_context.clone());
        Task::ready(Ok(Some(SignatureHelp {
            signatures: vec![SignatureInformation {
                label: "f()".into(),
                documentation: None,
                parameters: Vec::new(),
                active_parameter: None,
            }],
            active_signature: 0,
            active_parameter: 0,
        })))
    }
}

/// Monaco `parameterHintsModel.test.ts` "Provider should be retriggered if
/// already active".
#[gpui::test]
fn signature_help_context_on_retrigger(cx: &mut TestAppContext) {
    init(cx);
    let contexts = Rc::new(RefCell::new(Vec::new()));
    let provider = Signatures(contexts.clone());
    cx.update(|cx| register_signature_help_provider(cx, LanguageSelector::All, provider));
    let (_editor, cx) = editor_with("", cx);
    cx.simulate_input("(");
    cx.run_until_parked();
    cx.simulate_input("(");
    cx.run_until_parked();
    // Typing also re-queries as a content change while hints show; the
    // trigger character requests are what Monaco's test checks.
    let contexts: Vec<_> = contexts
        .borrow()
        .iter()
        .filter(|c| c.trigger == SignatureHelpTrigger::TriggerCharacter('('))
        .cloned()
        .collect();
    assert_eq!(contexts.len(), 2);
    assert_eq!(
        contexts[0].trigger,
        SignatureHelpTrigger::TriggerCharacter('(')
    );
    assert!(!contexts[0].is_retrigger);
    assert_eq!(contexts[0].active_signature_help, None);
    assert_eq!(
        contexts[1].trigger,
        SignatureHelpTrigger::TriggerCharacter('(')
    );
    assert!(contexts[1].is_retrigger);
    assert_eq!(
        contexts[1]
            .active_signature_help
            .as_ref()
            .map(|help| help.signatures[0].label.to_string()),
        Some("f()".into())
    );
}

struct CountedFolds(Rc<Cell<usize>>);
impl FoldingRangeProvider for CountedFolds {
    fn provide_folding_ranges(
        &self,
        _: &ProviderContext,
        _: &FoldingContext,
        _: &mut App,
    ) -> Task<anyhow::Result<Option<Vec<FoldingRange>>>> {
        self.0.set(self.0.get() + 1);
        Task::ready(Ok(Some(vec![FoldingRange {
            start_row: 0,
            end_row: 1,
            kind: None,
        }])))
    }
}

struct CountedLenses(Rc<Cell<usize>>);
impl CodeLensProvider for CountedLenses {
    fn provide_code_lenses(
        &self,
        _: &ProviderContext,
        _: &mut App,
    ) -> Task<anyhow::Result<Vec<CodeLens>>> {
        self.0.set(self.0.get() + 1);
        Task::ready(Ok(Vec::new()))
    }
}

#[gpui::test]
fn providers_can_ask_for_folding_ranges_and_lenses_again(cx: &mut TestAppContext) {
    init(cx);
    let (folds, lenses) = (Rc::new(Cell::new(0)), Rc::new(Cell::new(0)));
    let (f, l) = (CountedFolds(folds.clone()), CountedLenses(lenses.clone()));
    cx.update(|cx| {
        register_folding_range_provider(cx, LanguageSelector::All, f);
        register_code_lens_provider(cx, LanguageSelector::All, l);
    });
    let (_editor, cx) = editor_with("a {\n}\n", cx);
    draw(cx);
    let (folds_before, lenses_before) = (folds.get(), lenses.get());
    assert!(folds_before > 0 && lenses_before > 0);
    // Without a change, painting again doesn't ask.
    draw(cx);
    assert_eq!((folds.get(), lenses.get()), (folds_before, lenses_before));
    cx.update(|_, cx| {
        refresh_folding_ranges(cx);
        refresh_code_lenses(cx);
    });
    draw(cx);
    cx.executor()
        .advance_clock(std::time::Duration::from_secs(2));
    draw(cx);
    assert_eq!(folds.get(), folds_before + 1);
    assert_eq!(lenses.get(), lenses_before + 1);
}

/// Formats several ranges in one request, recording them.
struct AllAtOnce(Rc<RefCell<Vec<Vec<Range<PointUtf16>>>>>);
impl DocumentRangeFormattingProvider for AllAtOnce {
    fn provide_document_range_formatting_edits(
        &self,
        _: &ProviderContext,
        _: Range<PointUtf16>,
        _: FormattingOptions,
        _: &mut App,
    ) -> Task<anyhow::Result<Vec<TextEdit>>> {
        unreachable!("formats all ranges at once")
    }

    fn provide_document_ranges_formatting_edits(
        &self,
        _: &ProviderContext,
        ranges: &[Range<PointUtf16>],
        _: FormattingOptions,
        _: &mut App,
    ) -> Option<Task<anyhow::Result<Vec<TextEdit>>>> {
        self.0.borrow_mut().push(ranges.to_vec());
        let edits = ranges
            .iter()
            .map(|range| (range.start..range.start, "> ".to_string()))
            .collect();
        Some(Task::ready(Ok(edits)))
    }
}

#[gpui::test]
fn multi_range_formatting(cx: &mut TestAppContext) {
    init(cx);
    let requests = Rc::new(RefCell::new(Vec::new()));
    let provider = AllAtOnce(requests.clone());
    cx.update(|cx| {
        register_document_range_formatting_provider(cx, LanguageSelector::All, provider)
    });
    let (editor, cx) = editor_with("one\ntwo\nthree", cx);
    editor.update(cx, |e, cx| {
        e.set_selections(
            &[
                Selection::new(p(0, 0), p(0, 3)),
                Selection::new(p(2, 0), p(2, 5)),
            ],
            cx,
        );
        e.format_selection(cx);
    });
    cx.run_until_parked();
    assert_eq!(
        *requests.borrow(),
        [vec![p(0, 0)..p(0, 3), p(2, 0)..p(2, 5)]]
    );
    assert_eq!(
        editor.read_with(cx, |e, cx| e.model().read(cx).text()),
        "> one\ntwo\n> three"
    );
}
