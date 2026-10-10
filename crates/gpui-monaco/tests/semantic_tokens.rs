//! Semantic tokens (Monaco `contrib/semanticTokens`) with mock providers.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use gpui::{
    App, AppContext as _, Entity, Focusable, SharedString, Task, TestAppContext, VisualTestContext,
    rgb,
};
use gpui_monaco::options::SemanticHighlighting;
use gpui_monaco::providers::*;
use gpui_monaco::text::PointUtf16;
use gpui_monaco::{CodeEditor, EditorOptions, TextModel, Theme};

const THEME: &str = r##"{
    "semanticHighlighting": true,
    "semanticTokenColors": {
        "variable": "#ff0000",
        "function": "#00ff00",
        "variable.readonly": "#0000ff"
    }
}"##;

fn setup<'a>(
    text: &str,
    theme: Option<&str>,
    options: impl FnOnce(&mut EditorOptions),
    cx: &'a mut TestAppContext,
) -> (Entity<CodeEditor>, &'a mut VisualTestContext) {
    cx.update(gpui_monaco::init);
    let text = text.to_string();
    let theme =
        theme.map(|json| Arc::new(Theme::from_vscode_json_with_includes(json, &|_| None).unwrap()));
    let (editor, cx) = cx.add_window_view(|window, cx| {
        let model = cx.new(|_| TextModel::new(&text).with_language("demo"));
        let mut editor = CodeEditor::new(model, window, cx);
        if let Some(theme) = theme {
            editor.set_theme(theme, cx);
        }
        editor
    });
    editor.update(cx, |editor, cx| {
        let mut opts = editor.options().clone();
        options(&mut opts);
        editor.update_options(opts, cx);
    });
    cx.update(|window, cx| window.focus(&editor.focus_handle(cx), cx));
    cx.run_until_parked();
    (editor, cx)
}

fn legend() -> SemanticTokensLegend {
    SemanticTokensLegend {
        token_types: vec!["variable".into(), "function".into(), "label".into()],
        token_modifiers: vec!["readonly".into()],
    }
}

/// Tokens per identifier: `let` lines' second word is a variable, words
/// followed by `(` are functions. Records the requests' result ids, and
/// answers with edits when it gets one.
#[derive(Clone, Default)]
struct Words {
    requests: Rc<RefCell<Vec<Option<SharedString>>>>,
    edits: bool,
    /// The length of the last result's data.
    last_len: Rc<RefCell<usize>>,
}

fn encode(text: &str) -> Vec<u32> {
    let mut data = Vec::new();
    let (mut last_line, mut last_start) = (0u32, 0u32);
    for (row, line) in text.lines().enumerate() {
        let row = row as u32;
        let mut push = |start: u32, len: u32, ty: u32, mods: u32| {
            let delta_line = row - last_line;
            let delta_start = if delta_line == 0 {
                start - last_start
            } else {
                start
            };
            data.extend([delta_line, delta_start, len, ty, mods]);
            last_line = row;
            last_start = start;
        };
        let indent = line.len() - line.trim_start().len();
        if let Some(rest) = line.trim_start().strip_prefix("let ") {
            let name = rest
                .split(|c: char| !c.is_alphanumeric())
                .next()
                .unwrap_or("");
            let readonly = if name.chars().all(|c| c.is_uppercase()) {
                1
            } else {
                0
            };
            push(indent as u32 + 4, name.len() as u32, 0, readonly);
        }
        if let Some(paren) = line.find('(') {
            let start = line[..paren]
                .rfind(|c: char| !c.is_alphanumeric())
                .map_or(0, |i| i + 1);
            push(start as u32, (paren - start) as u32, 1, 0);
        }
    }
    data
}

impl DocumentSemanticTokensProvider for Words {
    fn legend(&self) -> SemanticTokensLegend {
        legend()
    }

    fn provide_document_semantic_tokens(
        &self,
        context: &ProviderContext,
        last_result_id: Option<SharedString>,
        _: &mut App,
    ) -> Task<anyhow::Result<Option<SemanticTokensResult>>> {
        self.requests.borrow_mut().push(last_result_id.clone());
        let id = self.requests.borrow().len().to_string();
        let data = encode(&context.snapshot.rope().to_string());
        let previous_len = self.last_len.replace(data.len());
        Task::ready(Ok(Some(match last_result_id {
            Some(_) if self.edits => SemanticTokensResult::Edits(SemanticTokensEdits {
                result_id: Some(id.into()),
                edits: vec![SemanticTokensEdit {
                    start: 0,
                    delete_count: previous_len,
                    data,
                }],
            }),
            _ => SemanticTokensResult::Tokens(SemanticTokens {
                result_id: Some(id.into()),
                data,
            }),
        })))
    }
}

fn colors(
    editor: &Entity<CodeEditor>,
    cx: &mut VisualTestContext,
    row: u32,
) -> Vec<(std::ops::Range<u32>, u32)> {
    cx.run_until_parked();
    editor.read_with(cx, |e, _| {
        e.semantic_tokens_for_tests(row)
            .into_iter()
            .map(|(range, style)| {
                let rgba = gpui::Rgba::from(style.foreground.unwrap());
                let hex = ((rgba.r * 255.).round() as u32) << 16
                    | ((rgba.g * 255.).round() as u32) << 8
                    | (rgba.b * 255.).round() as u32;
                (range, hex)
            })
            .collect()
    })
}

#[gpui::test]
fn colors_tokens_with_the_theme(cx: &mut TestAppContext) {
    let provider = Words::default();
    let requests = provider.requests.clone();
    cx.update(|cx| {
        gpui_monaco::init(cx);
        register_document_semantic_tokens_provider(cx, LanguageSelector::All, provider);
    });
    let (editor, cx) = setup(
        "let value = run(1);\nlet MAX = 2;\n",
        Some(THEME),
        |_| {},
        cx,
    );
    assert_eq!(
        colors(&editor, cx, 0),
        vec![(4..9, 0xff0000), (12..15, 0x00ff00)]
    );
    assert_eq!(colors(&editor, cx, 1), vec![(4..7, 0x0000ff)]);
    assert_eq!(*requests.borrow(), vec![None]);
}

#[gpui::test]
fn off_unless_the_theme_or_option_says_so(cx: &mut TestAppContext) {
    let provider = Words::default();
    let requests = provider.requests.clone();
    cx.update(|cx| {
        gpui_monaco::init(cx);
        register_document_semantic_tokens_provider(cx, LanguageSelector::All, provider);
    });
    // Monaco's built-in themes don't enable semantic highlighting.
    let (editor, cx) = setup("let value = 1;\n", None, |_| {}, cx);
    assert!(requests.borrow().is_empty());
    editor.update(cx, |e, cx| {
        let mut options = e.options().clone();
        options.semantic_highlighting = SemanticHighlighting::On;
        e.update_options(options, cx);
    });
    cx.run_until_parked();
    assert_eq!(requests.borrow().len(), 1);
    // The built-in theme styles `variable` (standalone themes match
    // `type.modifiers` against their rules).
    assert_eq!(
        editor.read_with(cx, |e, _| e.semantic_tokens_for_tests(0).len()),
        1
    );
}

#[gpui::test]
fn tokens_follow_edits_and_refresh_after_a_pause(cx: &mut TestAppContext) {
    let provider = Words {
        edits: true,
        ..Default::default()
    };
    let requests = provider.requests.clone();
    cx.update(|cx| {
        gpui_monaco::init(cx);
        register_document_semantic_tokens_provider(cx, LanguageSelector::All, provider);
    });
    let (editor, cx) = setup("let value = run(1);\n", Some(THEME), |_| {}, cx);
    assert_eq!(
        colors(&editor, cx, 0),
        vec![(4..9, 0xff0000), (12..15, 0x00ff00)]
    );
    // Typing at the start of the line moves the tokens right away.
    editor.update(cx, |e, cx| {
        e.model()
            .update(cx, |m, cx| m.edit([(0..0, "  ")], cx).unwrap())
    });
    assert_eq!(
        colors(&editor, cx, 0),
        vec![(6..11, 0xff0000), (14..17, 0x00ff00)]
    );
    // Typing a word character at a token's end extends it (Monaco).
    editor.update(cx, |e, cx| {
        e.model()
            .update(cx, |m, cx| m.edit([(11..11, "s")], cx).unwrap())
    });
    assert_eq!(colors(&editor, cx, 0)[0], (6..12, 0xff0000));
    assert_eq!(requests.borrow().len(), 1);
    // After the debounce, a delta request against the last result.
    cx.executor().advance_clock(Duration::from_millis(2100));
    assert_eq!(colors(&editor, cx, 0)[0], (6..12, 0xff0000));
    assert_eq!(*requests.borrow(), vec![None, Some("1".into())]);
}

/// Colors every line's first word as a variable.
struct FirstWords {
    ranges: Rc<RefCell<Vec<std::ops::Range<PointUtf16>>>>,
}

impl DocumentRangeSemanticTokensProvider for FirstWords {
    fn legend(&self) -> SemanticTokensLegend {
        legend()
    }

    fn provide_document_range_semantic_tokens(
        &self,
        context: &ProviderContext,
        range: std::ops::Range<PointUtf16>,
        _: &mut App,
    ) -> Task<anyhow::Result<Option<SemanticTokens>>> {
        self.ranges.borrow_mut().push(range.clone());
        let mut data = Vec::new();
        let mut last = range.start.row;
        for row in range.start.row..=range.end.row {
            let line = context.snapshot.line(row);
            let len = line.split(' ').next().unwrap_or("").len() as u32;
            if len > 0 {
                data.extend([row - last, 0, len, 0, 0]);
                last = row;
            }
        }
        Task::ready(Ok(Some(SemanticTokens {
            result_id: None,
            data,
        })))
    }
}

#[gpui::test]
fn range_provider_colors_the_viewport(cx: &mut TestAppContext) {
    let ranges = Rc::new(RefCell::new(Vec::new()));
    let provider = FirstWords {
        ranges: ranges.clone(),
    };
    cx.update(|cx| {
        gpui_monaco::init(cx);
        register_document_range_semantic_tokens_provider(cx, LanguageSelector::All, provider);
    });
    let text = (0..200).map(|i| format!("w{i} x\n")).collect::<String>();
    let (editor, cx) = setup(&text, Some(THEME), |_| {}, cx);
    cx.executor().advance_clock(Duration::from_millis(200));
    assert_eq!(colors(&editor, cx, 0), vec![(0..2, 0xff0000)]);
    // Only the visible lines and some around them.
    let requested = ranges.borrow()[0].clone();
    assert_eq!(requested.start.row, 0);
    assert!(requested.end.row < 199, "{requested:?}");
    assert!(colors(&editor, cx, 199).is_empty());
    let _ = rgb(0);
}

#[gpui::test]
fn providers_registered_later_are_asked(cx: &mut TestAppContext) {
    // E.g. a language server that starts after the editor opened.
    let (editor, cx) = setup("let value = run(1);\n", Some(THEME), |_| {}, cx);
    assert!(colors(&editor, cx, 0).is_empty());
    let provider = Words::default();
    let requests = provider.requests.clone();
    cx.update(|_, cx| {
        register_document_semantic_tokens_provider(cx, LanguageSelector::All, provider);
    });
    assert_eq!(
        colors(&editor, cx, 0),
        vec![(4..9, 0xff0000), (12..15, 0x00ff00)]
    );
    // A refresh asks again.
    cx.update(|_, cx| refresh_semantic_tokens(cx));
    cx.run_until_parked();
    assert_eq!(requests.borrow().len(), 2);
}
