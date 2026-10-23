//! Inline values while debugging (Monaco's `InlineValuesProvider`, shown as
//! VS Code's debugger shows them).

use std::sync::{Arc, Mutex};
use std::time::Duration;

use gpui::{App, AppContext as _, Entity, Focusable, Task, TestAppContext, VisualTestContext};
use gpui_monaco::providers::{
    InlineValue, InlineValueContext, InlineValuesProvider, LanguageSelector, ProviderContext,
    refresh_inline_values, register_inline_values_provider,
};
use gpui_monaco::text::PointUtf16;
use gpui_monaco::{CodeEditor, InlineValueLookup, TextModel};

const TEXT: &str = "let a = 1;\nlet b = a + 1;\nprint(b, zz);\n";

fn p(row: u32, column: u32) -> PointUtf16 {
    PointUtf16::new(row, column)
}

/// The view ports and contexts a provider was asked for.
type Requests = Arc<Mutex<Vec<(std::ops::Range<PointUtf16>, InlineValueContext)>>>;

/// The values of [`TEXT`], and the requests it got.
struct Values {
    requests: Requests,
    long: bool,
}

impl InlineValuesProvider for Values {
    fn provide_inline_values(
        &self,
        _context: &ProviderContext,
        view_port: std::ops::Range<PointUtf16>,
        inline_context: &InlineValueContext,
        _cx: &mut App,
    ) -> Task<anyhow::Result<Vec<InlineValue>>> {
        self.requests
            .lock()
            .unwrap()
            .push((view_port, inline_context.clone()));
        let mut values = vec![
            // Named by the text.
            InlineValue::VariableLookup {
                range: p(0, 4)..p(0, 5),
                variable_name: None,
                case_sensitive_lookup: true,
            },
            // Sorted by column.
            InlineValue::EvaluatableExpression {
                range: p(1, 8)..p(1, 13),
                expression: None,
            },
            InlineValue::VariableLookup {
                range: p(1, 4)..p(1, 5),
                variable_name: Some("b".into()),
                case_sensitive_lookup: false,
            },
            // Twice: shown once.
            InlineValue::VariableLookup {
                range: p(1, 8)..p(1, 9),
                variable_name: Some("b".into()),
                case_sensitive_lookup: false,
            },
            // The debugger doesn't know it: left out.
            InlineValue::VariableLookup {
                range: p(2, 9)..p(2, 11),
                variable_name: None,
                case_sensitive_lookup: true,
            },
            InlineValue::Text {
                range: p(2, 0)..p(2, 5),
                text: "printing".into(),
            },
        ];
        if self.long {
            values.push(InlineValue::Text {
                range: p(3, 0)..p(3, 0),
                text: "x".repeat(200).into(),
            });
        }
        Task::ready(Ok(values))
    }
}

fn resolve(lookup: InlineValueLookup, _cx: &mut App) -> Task<Option<gpui::SharedString>> {
    Task::ready(match lookup {
        InlineValueLookup::Variable {
            name,
            case_sensitive,
        } => match (name.as_ref(), case_sensitive) {
            ("a", true) => Some("1".into()),
            ("b", false) => Some("2".into()),
            _ => None,
        },
        InlineValueLookup::Expression(expression) => {
            (expression.as_ref() == "a + 1").then(|| "2".into())
        }
    })
}

fn setup(
    cx: &mut TestAppContext,
    long: bool,
) -> (Entity<CodeEditor>, Requests, &mut VisualTestContext) {
    let requests = Arc::new(Mutex::new(Vec::new()));
    let provider = Values {
        requests: requests.clone(),
        long,
    };
    cx.update(|cx| {
        gpui_monaco::init(cx);
        register_inline_values_provider(cx, LanguageSelector::All, provider);
    });
    let (editor, cx) = cx.add_window_view(|window, cx| {
        let model = cx.new(|_| TextModel::new(TEXT).with_uri("file:///main.js"));
        CodeEditor::new(model, window, cx)
    });
    cx.update(|window, cx| window.focus(&editor.focus_handle(cx), cx));
    draw(cx);
    (editor, requests, cx)
}

fn draw(cx: &mut VisualTestContext) {
    cx.run_until_parked();
    cx.update(|window, cx| {
        window.refresh();
        window.draw(cx).clear(cx);
    });
    cx.run_until_parked();
}

fn lines(editor: &Entity<CodeEditor>, cx: &mut VisualTestContext) -> Vec<(u32, String)> {
    draw(cx);
    editor.read_with(cx, |e, _| {
        e.inline_values()
            .iter()
            .map(|(row, text)| (*row, text.to_string()))
            .collect()
    })
}

fn context() -> InlineValueContext {
    InlineValueContext {
        frame_id: 7,
        stopped_location: p(1, 0)..p(1, 14),
    }
}

fn show(editor: &Entity<CodeEditor>, cx: &mut VisualTestContext) {
    editor.update(cx, |e, cx| e.show_inline_values(context(), resolve, cx));
}

#[gpui::test]
fn shows_each_lines_values_after_it(cx: &mut TestAppContext) {
    let (editor, requests, cx) = setup(cx, false);
    // Nothing until the debugger pauses.
    assert!(lines(&editor, cx).is_empty());
    assert!(requests.lock().unwrap().is_empty());

    show(&editor, cx);
    assert_eq!(
        lines(&editor, cx),
        vec![
            (0, "a = 1".to_string()),
            (1, "b = 2, a + 1 = 2".to_string()),
            (2, "printing".to_string()),
        ]
    );
    // Asked once, for the rows on screen and around, with the frame.
    {
        let requests = requests.lock().unwrap();
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].0, p(0, 0)..p(3, 0));
        assert_eq!(requests[0].1, context());
    }

    // After each line's end: a no-break space, then the values with no-break
    // spaces, in the theme's colors, where the cursor doesn't stop.
    let owner = editor.entity_id();
    editor.read_with(cx, |e, cx| {
        let decorations = e.model().read(cx).injected_text_decorations(Some(owner));
        let after: Vec<_> = decorations
            .iter()
            .map(|d| {
                let after = d.options.after.as_ref().unwrap();
                (
                    d.range.clone(),
                    after.content.to_string(),
                    after.background.is_some(),
                )
            })
            .collect();
        assert_eq!(
            after,
            vec![
                (p(0, 10)..p(0, 10), "\u{a0}".to_string(), false),
                (p(0, 10)..p(0, 10), "a\u{a0}=\u{a0}1".to_string(), true),
                (p(1, 14)..p(1, 14), "\u{a0}".to_string(), false),
                (
                    p(1, 14)..p(1, 14),
                    "b\u{a0}=\u{a0}2,\u{a0}a\u{a0}+\u{a0}1\u{a0}=\u{a0}2".to_string(),
                    true
                ),
                (p(2, 13)..p(2, 13), "\u{a0}".to_string(), false),
                (p(2, 13)..p(2, 13), "printing".to_string(), true),
            ]
        );
    });

    // Drawing again doesn't ask again.
    draw(cx);
    assert_eq!(requests.lock().unwrap().len(), 1);

    // The debugger continues.
    editor.update(cx, |e, cx| e.clear_inline_values(cx));
    assert!(lines(&editor, cx).is_empty());
    editor.read_with(cx, |e, cx| {
        assert!(
            e.model()
                .read(cx)
                .injected_text_decorations(Some(owner))
                .is_empty()
        );
    });
    draw(cx);
    assert_eq!(requests.lock().unwrap().len(), 1);
}

#[gpui::test]
fn asks_again_after_edits_and_refreshes(cx: &mut TestAppContext) {
    let (editor, requests, cx) = setup(cx, false);
    show(&editor, cx);
    assert_eq!(lines(&editor, cx).len(), 3);
    assert_eq!(requests.lock().unwrap().len(), 1);

    // A refresh asks again at once.
    cx.update(|_, cx| refresh_inline_values(cx));
    draw(cx);
    assert_eq!(requests.lock().unwrap().len(), 2);

    // An edit, 200ms after the last one.
    editor.update(cx, |e, cx| {
        e.model()
            .update(cx, |m, cx| m.edit([(0..0, "\n")], cx).unwrap())
    });
    draw(cx);
    cx.executor().advance_clock(Duration::from_millis(100));
    editor.update(cx, |e, cx| {
        e.model()
            .update(cx, |m, cx| m.edit([(0..0, "\n")], cx).unwrap())
    });
    draw(cx);
    cx.executor().advance_clock(Duration::from_millis(150));
    draw(cx);
    assert_eq!(requests.lock().unwrap().len(), 2);
    cx.executor().advance_clock(Duration::from_millis(100));
    draw(cx);
    assert_eq!(requests.lock().unwrap().len(), 3);

    // Only while the editor shows the paused model.
    let shown = lines(&editor, cx);
    assert!(!shown.is_empty());
    let other = cx.new(|_| TextModel::new("other\n"));
    let model = editor.read_with(cx, |e, _| e.model().clone());
    editor.update(cx, |e, cx| e.set_model(other, cx));
    assert!(lines(&editor, cx).is_empty());
    editor.update(cx, |e, cx| e.set_model(model, cx));
    assert_eq!(lines(&editor, cx), shown);
}

/// VS Code's `MAX_INLINE_DECORATOR_LENGTH`.
#[gpui::test]
fn long_values_are_cut(cx: &mut TestAppContext) {
    let (editor, _, cx) = setup(cx, true);
    show(&editor, cx);
    let lines = lines(&editor, cx);
    assert_eq!(lines[3], (3, format!("{}...", "x".repeat(150))));
}
