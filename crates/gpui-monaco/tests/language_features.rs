//! Hover, diagnostics markers, go to definition and signature help with mock
//! providers.

use gpui::{
    App, AppContext as _, Entity, Focusable, Modifiers, MouseButton, Task, TestAppContext,
    VisualTestContext, px,
};
use gpui_monaco::providers::*;
use gpui_monaco::text::PointUtf16;
use gpui_monaco::{CodeEditor, CodeEditorEvent, HOVER_DELAY, Marker, MarkerSeverity, TextModel};

fn setup<'a>(
    text: &str,
    cx: &'a mut TestAppContext,
) -> (Entity<CodeEditor>, &'a mut VisualTestContext) {
    cx.update(gpui_monaco::init);
    let text = text.to_string();
    let (editor, cx) = cx.add_window_view(|window, cx| {
        let model = cx.new(|_| {
            TextModel::new(&text)
                .with_language("demo")
                .with_uri("file:///main.demo")
        });
        CodeEditor::new(model, window, cx)
    });
    cx.update(|window, cx| window.focus(&editor.focus_handle(cx), cx));
    cx.run_until_parked();
    (editor, cx)
}

fn mouse_at(
    editor: &Entity<CodeEditor>,
    cx: &mut VisualTestContext,
    row: u32,
    column: u32,
) -> gpui::Point<gpui::Pixels> {
    let origin = editor
        .read_with(cx, |e, _| {
            e.scrolled_visible_position(PointUtf16::new(row, column))
        })
        .expect("laid out");
    gpui::point(origin.x + px(1.), origin.y + px(2.))
}

struct StaticHover;
impl HoverProvider for StaticHover {
    fn provide_hover(
        &self,
        context: &ProviderContext,
        position: PointUtf16,
        _: &mut App,
    ) -> Task<anyhow::Result<Option<Hover>>> {
        let word = context.snapshot.line(position.row);
        Task::ready(Ok(Some(Hover {
            contents: vec![format!("**{}**", word.trim()).into()],
            range: None,
        })))
    }
}

#[gpui::test]
fn hover_shows_provider_and_marker_contents(cx: &mut TestAppContext) {
    cx.update(|cx| register_hover_provider(cx, LanguageSelector::language("demo"), StaticHover));
    let (editor, cx) = setup("alpha beta\ngamma", cx);
    let model = editor.read_with(cx, |e, _| e.model().clone());
    model.update(cx, |m, cx| {
        m.set_markers(
            "lint",
            vec![Marker {
                severity: MarkerSeverity::Error,
                message: "bad beta".into(),
                range: PointUtf16::new(0, 6)..PointUtf16::new(0, 10),
                source: Some("demo".into()),
                code: None,
            }],
            cx,
        )
    });
    let position = mouse_at(&editor, cx, 0, 7);
    cx.simulate_mouse_move(position, None, Modifiers::default());
    assert!(!editor.read_with(cx, |e, _| e.is_hover_visible()));
    cx.executor().advance_clock(HOVER_DELAY);
    cx.run_until_parked();
    assert!(editor.read_with(cx, |e, _| e.is_hover_visible()));
    // Typing hides it.
    cx.simulate_input("x");
    assert!(!editor.read_with(cx, |e, _| e.is_hover_visible()));
}

#[gpui::test]
fn markers_follow_edits_and_f8_navigates(cx: &mut TestAppContext) {
    let (editor, cx) = setup("one\ntwo\nthree", cx);
    let model = editor.read_with(cx, |e, _| e.model().clone());
    let marker = |row, severity| Marker {
        severity,
        message: "problem".into(),
        range: PointUtf16::new(row, 0)..PointUtf16::new(row, 3),
        source: None,
        code: None,
    };
    model.update(cx, |m, cx| {
        m.set_markers(
            "lint",
            vec![
                marker(1, MarkerSeverity::Warning),
                marker(2, MarkerSeverity::Error),
            ],
            cx,
        )
    });
    cx.simulate_keystrokes("enter");
    let rows: Vec<u32> = model.read_with(cx, |m, _| {
        m.markers().iter().map(|m| m.range.start.row).collect()
    });
    assert_eq!(rows, vec![2, 3]);
    cx.simulate_keystrokes("f8");
    assert_eq!(
        editor.read_with(cx, |e, _| e.selections()[0].head),
        PointUtf16::new(2, 0)
    );
    cx.simulate_keystrokes("f8");
    assert_eq!(
        editor.read_with(cx, |e, _| e.selections()[0].head),
        PointUtf16::new(3, 0)
    );
    cx.simulate_keystrokes("shift-f8");
    assert_eq!(
        editor.read_with(cx, |e, _| e.selections()[0].head),
        PointUtf16::new(2, 0)
    );
    model.update(cx, |m, cx| m.set_markers("lint", vec![], cx));
    assert!(model.read_with(cx, |m, _| m.markers().is_empty()));
}

struct Definitions;
impl DefinitionProvider for Definitions {
    fn provide_definition(
        &self,
        _: &ProviderContext,
        position: PointUtf16,
        _: &mut App,
    ) -> Task<anyhow::Result<Vec<Location>>> {
        let location = if position.row == 1 {
            Location {
                uri: None,
                range: PointUtf16::new(0, 3)..PointUtf16::new(0, 7),
            }
        } else {
            Location {
                uri: Some("file:///other.demo".into()),
                range: PointUtf16::new(9, 0)..PointUtf16::new(9, 1),
            }
        };
        Task::ready(Ok(vec![location]))
    }
}

#[gpui::test]
fn go_to_definition_same_model_and_other_model(cx: &mut TestAppContext) {
    cx.update(|cx| register_definition_provider(cx, LanguageSelector::All, Definitions));
    let (editor, cx) = setup("fn main() {}\nmain();", cx);
    let events = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
    let sink = events.clone();
    cx.update(|_, cx| {
        cx.subscribe(&editor, move |_, event: &CodeEditorEvent, _| {
            sink.borrow_mut().push(event.clone())
        })
        .detach();
    });
    // Cmd/Ctrl+hover then Cmd/Ctrl+click on `main` in row 1.
    let position = mouse_at(&editor, cx, 1, 1);
    cx.simulate_mouse_move(position, None, Modifiers::secondary_key());
    cx.executor()
        .advance_clock(std::time::Duration::from_millis(60));
    cx.run_until_parked();
    cx.simulate_click(position, Modifiers::secondary_key());
    assert_eq!(
        editor.read_with(cx, |e, _| e.selections()[0].head),
        PointUtf16::new(0, 3)
    );
    // F12 on row 0 leads to another model: the host gets an event.
    cx.simulate_keystrokes("f12");
    cx.run_until_parked();
    assert!(events.borrow().iter().any(|e| matches!(e, CodeEditorEvent::OpenLocation(l) if l.uri.as_deref() == Some("file:///other.demo"))));
    let _ = MouseButton::Left;
}

struct Signatures;
impl SignatureHelpProvider for Signatures {
    fn provide_signature_help(
        &self,
        context: &ProviderContext,
        position: PointUtf16,
        _: SignatureHelpTrigger,
        _: &mut App,
    ) -> Task<anyhow::Result<Option<SignatureHelp>>> {
        let line = context.snapshot.line(position.row);
        let before: String = line.chars().take(position.column as usize).collect();
        // Inside a call when the last `(` comes after the last `)`.
        let Some(open) = before
            .rfind('(')
            .filter(|open| before.rfind(')').is_none_or(|close| close < *open))
        else {
            return Task::ready(Ok(None));
        };
        let active_parameter = before[open..].matches(',').count();
        let parameters = ["a: i32", "b: i32"]
            .into_iter()
            .map(|p| ParameterInformation {
                label: ParameterLabel::Text(p.into()),
                documentation: None,
            })
            .collect();
        Task::ready(Ok(Some(SignatureHelp {
            signatures: vec![SignatureInformation {
                label: "add(a: i32, b: i32)".into(),
                documentation: None,
                parameters,
                active_parameter: None,
            }],
            active_signature: 0,
            active_parameter,
        })))
    }
}

#[gpui::test]
fn signature_help_opens_on_trigger_and_closes(cx: &mut TestAppContext) {
    cx.update(|cx| register_signature_help_provider(cx, LanguageSelector::All, Signatures));
    let (editor, cx) = setup("", cx);
    cx.simulate_input("add(");
    cx.run_until_parked();
    assert!(editor.read_with(cx, |e, _| e.is_signature_help_visible()));
    cx.simulate_input("1, 2)");
    cx.run_until_parked();
    assert!(
        !editor.read_with(cx, |e, _| e.is_signature_help_visible()),
        "closes once the provider returns nothing"
    );
    cx.simulate_input("; add(");
    cx.run_until_parked();
    assert!(editor.read_with(cx, |e, _| e.is_signature_help_visible()));
    cx.simulate_keystrokes("escape");
    assert!(!editor.read_with(cx, |e, _| e.is_signature_help_visible()));
}

#[gpui::test]
fn signature_help_follows_cursor_moves(cx: &mut TestAppContext) {
    cx.update(|cx| register_signature_help_provider(cx, LanguageSelector::All, Signatures));
    let (editor, cx) = setup("", cx);
    cx.simulate_input("add(1, 2");
    cx.run_until_parked();
    assert!(editor.read_with(cx, |e, _| e.is_signature_help_visible()));
    // Moving within the call re-queries and keeps it open.
    cx.simulate_keystrokes("left left");
    cx.run_until_parked();
    assert!(editor.read_with(cx, |e, _| e.is_signature_help_visible()));
    // Moving out of the call re-queries, and the provider has nothing.
    cx.simulate_keystrokes("home");
    cx.run_until_parked();
    assert!(!editor.read_with(cx, |e, _| e.is_signature_help_visible()));
}

struct Completions;
impl CompletionProvider for Completions {
    fn trigger_characters(&self) -> Vec<char> {
        vec!['.']
    }

    fn provide_completions(
        &self,
        _: &ProviderContext,
        _: PointUtf16,
        trigger: CompletionTrigger,
        _: &mut App,
    ) -> Task<anyhow::Result<CompletionList>> {
        let items = match trigger {
            CompletionTrigger::TriggerCharacter('.') => vec![
                CompletionItem {
                    commit_characters: vec!['('],
                    ..CompletionItem::new("length", CompletionItemKind::Property)
                },
                CompletionItem::new("push", CompletionItemKind::Method),
            ],
            _ => ["print", "println", "process", "private"]
                .into_iter()
                .map(|label| CompletionItem::new(label, CompletionItemKind::Function))
                .collect(),
        };
        Task::ready(Ok(CompletionList {
            items,
            incomplete: false,
        }))
    }
}

/// Waits out the quick suggestions delay.
fn settle(cx: &mut VisualTestContext) {
    cx.executor()
        .advance_clock(std::time::Duration::from_millis(50));
    cx.run_until_parked();
}

fn suggest_labels(editor: &Entity<CodeEditor>, cx: &mut VisualTestContext) -> Option<Vec<String>> {
    editor.read_with(cx, |e, _| e.suggest_labels_for_tests())
}

#[gpui::test]
fn completion_filters_navigates_and_accepts(cx: &mut TestAppContext) {
    cx.update(|cx| register_completion_provider(cx, LanguageSelector::All, Completions));
    let (editor, cx) = setup("", cx);
    cx.simulate_input("pr");
    settle(cx);
    assert_eq!(
        suggest_labels(&editor, cx).unwrap(),
        vec!["print", "println", "private", "process"]
    );
    cx.simulate_input("in");
    cx.run_until_parked();
    assert_eq!(
        suggest_labels(&editor, cx).unwrap(),
        vec!["print", "println"]
    );
    cx.simulate_keystrokes("down enter");
    assert_eq!(
        editor.read_with(cx, |e, cx| e.model().read(cx).text()),
        "println"
    );
    assert!(!editor.read_with(cx, |e, _| e.is_suggest_visible()));
    // Trigger character, commit character.
    cx.simulate_input(".");
    cx.run_until_parked();
    assert_eq!(suggest_labels(&editor, cx).unwrap(), vec!["length", "push"]);
    cx.simulate_input("(");
    assert_eq!(
        editor.read_with(cx, |e, cx| e.model().read(cx).text()),
        "println.length("
    );
    // Escape closes; Ctrl+Space reopens.
    cx.simulate_input(" pri");
    settle(cx);
    assert!(editor.read_with(cx, |e, _| e.is_suggest_visible()));
    cx.simulate_keystrokes("escape");
    assert!(!editor.read_with(cx, |e, _| e.is_suggest_visible()));
    cx.simulate_keystrokes("ctrl-space");
    cx.run_until_parked();
    assert!(editor.read_with(cx, |e, _| e.is_suggest_visible()));
    // Deleting before the word start closes it.
    cx.simulate_keystrokes("backspace backspace backspace backspace");
    assert!(!editor.read_with(cx, |e, _| e.is_suggest_visible()));
}
