//! Inline edits (next edit suggestions, Monaco's `inlineEdits`) with mock
//! providers: the view chosen for an edit, Tab jumping to and accepting it,
//! Escape, the gutter indicator and its menu, and end-of-lifetime reports.

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

use gpui::{App, AppContext as _, Entity, Focusable, Task, TestAppContext, VisualTestContext};
use gpui_monaco::cursor::Selection;
use gpui_monaco::options::{EditorOptions, InlineEditsCodeShifting};
use gpui_monaco::providers::*;
use gpui_monaco::text::PointUtf16;
use gpui_monaco::{CodeEditor, InlineEditViewKind, TextModel};

type Make = Rc<dyn Fn(&str, PointUtf16) -> Vec<InlineCompletion>>;

/// Offers what `make` returns for the document and cursor, and records what
/// the editor reports about the items.
#[derive(Clone)]
struct Edits {
    make: Make,
    commands: Vec<Command>,
    events: Rc<RefCell<Vec<String>>>,
    contexts: Rc<RefCell<Vec<InlineCompletionContext>>>,
}

impl Edits {
    fn new(make: impl Fn(&str, PointUtf16) -> Vec<InlineCompletion> + 'static) -> Self {
        Self {
            make: Rc::new(make),
            commands: Vec::new(),
            events: Rc::default(),
            contexts: Rc::default(),
        }
    }
}

impl InlineCompletionsProvider for Edits {
    fn provide_inline_completions(
        &self,
        context: &ProviderContext,
        position: PointUtf16,
        completion_context: InlineCompletionContext,
        _: &mut App,
    ) -> Task<anyhow::Result<InlineCompletionList>> {
        self.contexts.borrow_mut().push(completion_context);
        let text = context.snapshot.rope().to_string();
        Task::ready(Ok(InlineCompletionList {
            items: (self.make)(&text, position),
            commands: self.commands.clone(),
            ..Default::default()
        }))
    }

    fn handle_item_did_show(&self, item: &InlineCompletion) {
        self.events
            .borrow_mut()
            .push(format!("shown {}", item.insert_text));
    }

    fn handle_rejection(&self, item: &InlineCompletion) {
        self.events
            .borrow_mut()
            .push(format!("rejection {}", item.insert_text));
    }

    fn handle_end_of_lifetime(
        &self,
        item: &InlineCompletion,
        reason: &InlineCompletionEndOfLifeReason,
    ) {
        let reason = match reason {
            InlineCompletionEndOfLifeReason::Accepted => "accepted",
            InlineCompletionEndOfLifeReason::Rejected => "rejected",
            InlineCompletionEndOfLifeReason::Ignored { .. } => "ignored",
        };
        self.events
            .borrow_mut()
            .push(format!("{reason} {}", item.insert_text));
    }

    fn display_name(&self) -> Option<gpui::SharedString> {
        Some("Test Edits".into())
    }
}

/// An inline edit replacing the first `old` in the document with `new`.
fn replace(text: &str, old: &str, new: &str) -> Vec<InlineCompletion> {
    let Some(start) = text.find(old) else {
        return Vec::new();
    };
    let point = |offset: usize| {
        let before = &text[..offset];
        let row = before.matches('\n').count() as u32;
        let column = before[before.rfind('\n').map_or(0, |ix| ix + 1)..]
            .encode_utf16()
            .count() as u32;
        PointUtf16::new(row, column)
    };
    vec![InlineCompletion {
        insert_text: new.into(),
        range: Some(point(start)..point(start + old.len())),
        is_inline_edit: true,
        ..Default::default()
    }]
}

fn p(row: u32, column: u32) -> PointUtf16 {
    PointUtf16::new(row, column)
}

fn setup<'a>(
    text: &str,
    provider: Edits,
    configure: impl FnOnce(&mut EditorOptions),
    cx: &'a mut TestAppContext,
) -> (Entity<CodeEditor>, &'a mut VisualTestContext) {
    cx.update(|cx| {
        gpui_monaco::init(cx);
        register_inline_completions_provider(cx, LanguageSelector::All, provider);
    });
    let text = text.to_string();
    let (editor, cx) = cx.add_window_view(move |window, cx| {
        let model = cx.new(|_| TextModel::new(&text));
        let mut editor = CodeEditor::new(model, window, cx);
        let mut options = editor.options().clone();
        options.quick_suggestions = gpui_monaco::options::QuickSuggestions::OFF;
        options.word_based_suggestions = gpui_monaco::options::WordBasedSuggestions::Off;
        configure(&mut options);
        editor.update_options(options, cx);
        editor
    });
    cx.update(|window, cx| window.focus(&editor.focus_handle(cx), cx));
    cx.run_until_parked();
    (editor, cx)
}

/// Settles requests and draws, so the inline edit renders.
fn draw(cx: &mut VisualTestContext) {
    cx.executor().advance_clock(Duration::from_millis(100));
    cx.run_until_parked();
    cx.update(|window, _| window.refresh());
    cx.run_until_parked();
}

fn set_cursor(editor: &Entity<CodeEditor>, at: PointUtf16, cx: &mut VisualTestContext) {
    editor.update(cx, |e, cx| {
        e.set_selections(&[Selection::collapsed(at)], cx)
    });
}

/// Types `text` at `at`, which asks for inline suggestions as typing does.
fn type_at(editor: &Entity<CodeEditor>, at: PointUtf16, text: &str, cx: &mut VisualTestContext) {
    set_cursor(editor, at, cx);
    cx.simulate_input(text);
    draw(cx);
}

fn text(editor: &Entity<CodeEditor>, cx: &mut VisualTestContext) -> String {
    editor.read_with(cx, |e, cx| e.model().read(cx).text())
}

fn cursor(editor: &Entity<CodeEditor>, cx: &mut VisualTestContext) -> PointUtf16 {
    editor.read_with(cx, |e, _| e.selections()[0].head)
}

fn view(editor: &Entity<CodeEditor>, cx: &mut VisualTestContext) -> Option<InlineEditViewKind> {
    editor.read_with(cx, |e, _| e.inline_edit_view_for_tests())
}

fn visible(editor: &Entity<CodeEditor>, cx: &mut VisualTestContext) -> bool {
    editor.read_with(cx, |e, cx| e.inline_edit_visible(cx))
}

const DOC: &str = "let colour = 1;\nlet b = 2;\nlet c = 3;\nlet d = 4;\nlet e = 5;\n";

#[gpui::test]
fn tab_jumps_to_the_edit_then_accepts_it(cx: &mut TestAppContext) {
    let provider = Edits::new(|text, _| replace(text, "colour", "shade"));
    let events = provider.events.clone();
    let (editor, cx) = setup(DOC, provider, |_| {}, cx);
    // Typing far below asks; the edit shows where it is.
    type_at(&editor, p(4, 10), " ", cx);
    assert!(visible(&editor, cx));
    assert!(!editor.read_with(cx, |e, cx| e.inline_suggestion_visible(cx)));
    assert_eq!(
        view(&editor, cx),
        Some(InlineEditViewKind::WordReplacements)
    );
    assert_eq!(events.borrow().as_slice(), ["shown shade"]);
    // The cursor is away from it: Tab jumps.
    assert!(editor.read_with(cx, |e, cx| e.tab_should_jump_to_inline_edit(cx)));
    assert!(!editor.read_with(cx, |e, cx| e.cursor_at_inline_edit(cx)));
    cx.simulate_keystrokes("tab");
    assert_eq!(cursor(&editor, cx), p(0, 4));
    assert_eq!(text(&editor, cx), DOC.replace("e = 5;", "e = 5; "));
    draw(cx);
    // Now at it: Tab accepts.
    assert!(editor.read_with(cx, |e, cx| e.cursor_at_inline_edit(cx)));
    assert!(editor.read_with(cx, |e, cx| e.tab_should_accept_inline_edit(cx)));
    cx.simulate_keystrokes("tab");
    assert!(text(&editor, cx).starts_with("let shade = 1;\n"));
    // After the replacement.
    assert_eq!(cursor(&editor, cx), p(0, 9));
    assert!(events.borrow().contains(&"accepted shade".to_string()));
    draw(cx);
    assert_eq!(view(&editor, cx), None);
}

#[gpui::test]
fn escape_rejects_it(cx: &mut TestAppContext) {
    let provider = Edits::new(|text, _| replace(text, "colour", "shade"));
    let events = provider.events.clone();
    let (editor, cx) = setup(DOC, provider, |_| {}, cx);
    type_at(&editor, p(4, 10), " ", cx);
    assert!(visible(&editor, cx));
    // Moving the cursor keeps an inline edit, unlike ghost text.
    set_cursor(&editor, p(3, 0), cx);
    draw(cx);
    assert!(visible(&editor, cx));
    cx.simulate_keystrokes("escape");
    assert!(!visible(&editor, cx));
    assert_eq!(
        events.borrow().as_slice(),
        ["shown shade", "rejection shade", "rejected shade"]
    );
    draw(cx);
    assert_eq!(view(&editor, cx), None);
    assert_eq!(
        editor.read_with(cx, |e, _| e.inline_edit_gutter_indicator_for_tests()),
        None
    );
}

#[gpui::test]
fn views_for_each_kind_of_edit(cx: &mut TestAppContext) {
    let next: Rc<RefCell<Vec<InlineCompletion>>> = Rc::default();
    let provider = {
        let next = next.clone();
        Edits::new(move |_, _| next.borrow().clone())
    };
    let (editor, cx) = setup(DOC, provider, |_| {}, cx);
    let edit = |range: std::ops::Range<PointUtf16>, text: &str| InlineCompletion {
        insert_text: text.into(),
        range: Some(range),
        is_inline_edit: true,
        ..Default::default()
    };
    let show = |completion: InlineCompletion, cx: &mut VisualTestContext| {
        *next.borrow_mut() = vec![completion];
        editor.update(cx, |e, cx| e.hide_inline_suggestion(cx));
        set_cursor(&editor, p(4, 10), cx);
        // Asking explicitly (Alt+\) leaves the text alone.
        editor.update(cx, |e, cx| e.trigger_inline_suggest(cx));
        draw(cx);
        view(&editor, cx)
    };
    use InlineEditViewKind::*;
    // Text inserted after the cursor's line shows in place.
    assert_eq!(
        show(edit(p(1, 0)..p(1, 10), "let b = 2 + 1;"), cx),
        Some(InsertionInline)
    );
    // Deleted text.
    assert_eq!(show(edit(p(1, 0)..p(1, 10), "let b;"), cx), Some(Deletion));
    // Inserted lines.
    assert_eq!(
        show(edit(p(1, 0)..p(1, 10), "let b = 2;\nlet bb = 22;"), cx),
        Some(InsertionMultiLine)
    );
    // Several changes on a line.
    assert_eq!(
        show(edit(p(1, 0)..p(1, 10), "var b = 3;"), cx),
        Some(LineReplacement)
    );
    // Several changed lines, beside the text.
    assert_eq!(
        show(
            edit(p(1, 0)..p(2, 10), "const b: u8 = 2;\nconst c: u8 = 3;"),
            cx
        ),
        Some(SideBySide)
    );
}

#[gpui::test]
fn never_side_by_side(cx: &mut TestAppContext) {
    let provider = Edits::new(|text, _| {
        replace(
            text,
            "let b = 2;\nlet c = 3;",
            "const b: u8 = 2;\nconst c: u8 = 3;",
        )
    });
    let (editor, cx) = setup(
        DOC,
        provider,
        |options| {
            options.inline_suggest.edits.render_side_by_side =
                gpui_monaco::options::InlineEditsRenderSideBySide::Never;
        },
        cx,
    );
    type_at(&editor, p(4, 10), " ", cx);
    // Below the text instead.
    assert_eq!(view(&editor, cx), Some(InlineEditViewKind::LineReplacement));
}

#[gpui::test]
fn without_code_shifting_insertions_show_in_a_box(cx: &mut TestAppContext) {
    let provider = Edits::new(|text, _| replace(text, "let b = 2;", "let b = 2 + 1;"));
    let (editor, cx) = setup(
        DOC,
        provider,
        |options| {
            options.inline_suggest.edits.allow_code_shifting = InlineEditsCodeShifting::Never;
        },
        cx,
    );
    type_at(&editor, p(4, 10), " ", cx);
    assert_eq!(
        view(&editor, cx),
        Some(InlineEditViewKind::WordReplacements)
    );
}

#[gpui::test]
fn the_gutter_indicator_and_its_menu(cx: &mut TestAppContext) {
    let mut provider = Edits::new(|text, _| {
        let mut items = replace(text, "colour", "shade");
        if let Some(item) = items.first_mut() {
            item.gutter_menu_link_action = Some(Command {
                id: "test.learnMore".into(),
                title: "Learn More".into(),
                arguments: Vec::new(),
            });
        }
        items
    });
    provider.commands = vec![Command {
        id: "test.explain".into(),
        title: "Explain".into(),
        arguments: Vec::new(),
    }];
    let events = provider.events.clone();
    let (editor, cx) = setup(DOC, provider, |_| {}, cx);
    type_at(&editor, p(4, 10), " ", cx);
    // Beside the edit's line.
    assert_eq!(
        editor.read_with(cx, |e, _| e.inline_edit_gutter_indicator_for_tests()),
        Some(Ok(0))
    );
    assert_eq!(
        editor.read_with(cx, |e, cx| e.inline_edit_gutter_menu_for_tests(cx)),
        [
            "Test Edits",
            "Go To / Accept",
            "Reject",
            "Show Collapsed",
            "Explain",
            "Learn More..."
        ]
    );
    // "Show Collapsed": only a marker, and Tab jumps.
    editor.update(cx, |e, cx| {
        e.run_inline_edit_gutter_menu_item_for_tests("Show Collapsed", cx)
    });
    draw(cx);
    assert!(editor.read_with(cx, |e, _| e.options().inline_suggest.edits.show_collapsed));
    assert_eq!(view(&editor, cx), Some(InlineEditViewKind::Collapsed));
    assert!(editor.read_with(cx, |e, cx| e.tab_should_jump_to_inline_edit(cx)));
    // Jumping expands it.
    editor.update(cx, |e, cx| e.jump_to_next_inline_edit(cx));
    draw(cx);
    assert_eq!(
        view(&editor, cx),
        Some(InlineEditViewKind::WordReplacements)
    );
    // "Reject" rejects it.
    editor.update(cx, |e, cx| {
        e.run_inline_edit_gutter_menu_item_for_tests("Reject", cx)
    });
    assert!(!visible(&editor, cx));
    assert!(events.borrow().contains(&"rejected shade".to_string()));
    // Clicking the indicator beside the edit accepts it.
    type_at(&editor, p(4, 11), " ", cx);
    assert!(visible(&editor, cx));
    editor.update(cx, |e, cx| e.click_inline_edit_gutter_indicator(cx));
    assert!(text(&editor, cx).starts_with("let shade = 1;"));
    assert!(events.borrow().contains(&"accepted shade".to_string()));
}

#[gpui::test]
fn the_indicator_pins_while_the_edit_is_scrolled_away(cx: &mut TestAppContext) {
    let mut doc = String::new();
    for ix in 0..300 {
        doc.push_str(&format!("let v{ix} = {ix};\n"));
    }
    let provider = Edits::new(|text, _| replace(text, "let v250 = 250;", "let v250 = 2500;"));
    let (editor, cx) = setup(&doc, provider, |_| {}, cx);
    type_at(&editor, p(0, 0), " ", cx);
    assert!(visible(&editor, cx));
    assert_eq!(
        editor.read_with(cx, |e, _| e.inline_edit_gutter_indicator_for_tests()),
        Some(Err(false))
    );
    // Clicking it there jumps.
    editor.update(cx, |e, cx| e.click_inline_edit_gutter_indicator(cx));
    draw(cx);
    assert_eq!(cursor(&editor, cx).row, 250);
    assert_eq!(
        editor.read_with(cx, |e, _| e.inline_edit_gutter_indicator_for_tests()),
        Some(Ok(250))
    );
}

#[gpui::test]
fn a_visible_completion_wins(cx: &mut TestAppContext) {
    let provider = Edits::new(|text, position| {
        let mut items = replace(text, "colour", "shade");
        let line = text.lines().nth(position.row as usize).unwrap_or("");
        if line.ends_with("pri") {
            items.push(InlineCompletion {
                insert_text: format!("{line}nt"),
                range: Some(p(position.row, 0)..position),
                ..Default::default()
            });
        }
        items
    });
    let (editor, cx) = setup(DOC, provider, |_| {}, cx);
    type_at(&editor, p(5, 0), "pri", cx);
    // Ghost text at the cursor, no inline edit.
    assert!(editor.read_with(cx, |e, cx| e.inline_suggestion_visible(cx)));
    assert!(!visible(&editor, cx));
    assert_eq!(view(&editor, cx), None);
}

#[gpui::test]
fn show_range_limits_where_it_shows(cx: &mut TestAppContext) {
    let provider = Edits::new(|text, _| {
        let mut items = replace(text, "colour", "shade");
        for item in &mut items {
            item.show_range = Some(p(3, 0)..p(4, 11));
        }
        items
    });
    let (editor, cx) = setup(DOC, provider, |_| {}, cx);
    type_at(&editor, p(4, 10), " ", cx);
    assert!(visible(&editor, cx));
    set_cursor(&editor, p(1, 0), cx);
    assert!(!visible(&editor, cx));
    set_cursor(&editor, p(3, 2), cx);
    assert!(visible(&editor, cx));
}

#[gpui::test]
fn typing_the_edit_keeps_it_and_other_typing_collapses_it(cx: &mut TestAppContext) {
    let provider = Edits::new(|text, _| replace(text, "let b = 2;", "let b = 2 + 1;"));
    let (editor, cx) = setup(DOC, provider, |_| {}, cx);
    type_at(&editor, p(4, 10), " ", cx);
    assert_eq!(view(&editor, cx), Some(InlineEditViewKind::InsertionInline));
    // Typing without letting the request after it finish: the edit follows
    // the typing (Monaco `SingleUpdatedNextEdit`).
    let type_now = |at: PointUtf16, typed: &str, cx: &mut VisualTestContext| {
        set_cursor(&editor, at, cx);
        cx.simulate_input(typed);
        cx.update(|window, _| window.refresh());
        cx.run_until_parked();
    };
    // Typing part of it: it follows.
    type_now(p(1, 9), " +", cx);
    assert!(visible(&editor, cx));
    assert_eq!(view(&editor, cx), Some(InlineEditViewKind::InsertionInline));
    // Typing elsewhere moves it, and it shows collapsed (Monaco shows an
    // edit collapsed once other changes came since typing last matched it).
    type_now(p(3, 0), "x", cx);
    assert!(visible(&editor, cx));
    assert_eq!(view(&editor, cx), Some(InlineEditViewKind::Collapsed));
    // Typing the rest of it completes it, and it's gone.
    type_now(p(1, 11), " 1", cx);
    assert!(!visible(&editor, cx));
    assert!(text(&editor, cx).contains("let b = 2 + 1;"));
}

#[gpui::test]
fn inline_edits_can_be_turned_off(cx: &mut TestAppContext) {
    let provider = Edits::new(|text, _| replace(text, "colour", "shade"));
    let (contexts, events) = (provider.contexts.clone(), provider.events.clone());
    let (editor, cx) = setup(
        DOC,
        provider,
        |options| options.inline_suggest.edits.enabled = false,
        cx,
    );
    type_at(&editor, p(4, 10), " ", cx);
    assert!(!visible(&editor, cx));
    let context = contexts.borrow().last().cloned().unwrap();
    assert!(!context.include_inline_edits);
    assert!(context.include_inline_completions);
    assert_eq!(events.borrow().as_slice(), ["ignored shade"]);
}

#[gpui::test]
fn commands_by_monaco_id(cx: &mut TestAppContext) {
    let provider = Edits::new(|text, _| replace(text, "colour", "shade"));
    let (editor, cx) = setup(DOC, provider, |_| {}, cx);
    let infos = editor.read_with(cx, |e, cx| e.supported_actions(cx));
    // Jump needs a visible inline edit.
    assert!(
        !infos
            .iter()
            .any(|a| a.id == "editor.action.inlineSuggest.jump")
    );
    type_at(&editor, p(4, 10), " ", cx);
    let infos = editor.read_with(cx, |e, cx| e.supported_actions(cx));
    for id in [
        "editor.action.inlineSuggest.jump",
        "editor.action.inlineSuggest.commit",
        "editor.action.inlineSuggest.hide",
        "editor.action.inlineSuggest.toggleShowCollapsed",
    ] {
        assert!(infos.iter().any(|a| a.id == id), "{id}");
    }
    cx.update(|window, cx| {
        editor
            .update(cx, |e, cx| {
                e.trigger("editor.action.inlineSuggest.jump", window, cx)
            })
            .unwrap()
    });
    assert_eq!(cursor(&editor, cx), p(0, 4));
    cx.update(|window, cx| {
        editor
            .update(cx, |e, cx| {
                e.trigger("editor.action.inlineSuggest.commit", window, cx)
            })
            .unwrap()
    });
    assert!(text(&editor, cx).starts_with("let shade = 1;"));
}

// Ported from Monaco's `inlineEdits.test.ts`.

/// Monaco's test document; `point_marker` gives its `↓` positions.
const POINT: &str = "
class Point {
\tconstructor(public x: number, public y: number) {}

\tgetLength2D(): number {
\t\treturn Math.sqrt(this.x * this.x + this.y * this.y);
\t}

\tgetJson(): string {
\t\treturn Ü;
\t}
}
";

fn point_marker(ix: usize) -> PointUtf16 {
    [p(5, 8), p(5, 52), p(9, 9)][ix]
}

const LENGTH_2D: &str =
    "getLength2D(): number {\n\t\treturn Math.sqrt(this.x * this.x + this.y * this.y);\n\t}";
const LENGTH_3D: &str = "getLength3D(): number {\n\t\treturn Math.sqrt(this.x * this.x + this.y * this.y + this.z * this.z);\n\t}";

/// Monaco `MockSearchReplaceCompletionsProvider`.
fn search_replace(search: &'static str, new: &'static str) -> Edits {
    Edits::new(move |text, _| replace(text, search, new))
}

fn inline_edit(
    editor: &Entity<CodeEditor>,
    cx: &mut VisualTestContext,
) -> Option<Vec<(String, String)>> {
    editor.read_with(cx, |e, cx| e.inline_edit_for_tests(cx))
}

/// Types at `at` without letting the request after it finish.
fn type_now(editor: &Entity<CodeEditor>, at: PointUtf16, typed: &str, cx: &mut VisualTestContext) {
    set_cursor(editor, at, cx);
    cx.simulate_input(typed);
    cx.run_until_parked();
}

fn pair(old: &str, new: &str) -> (String, String) {
    (old.to_string(), new.to_string())
}

#[gpui::test]
fn can_accept_inline_edit(cx: &mut TestAppContext) {
    let (editor, cx) = setup(POINT, search_replace(LENGTH_2D, LENGTH_3D), |_| {}, cx);
    editor.update(cx, |e, cx| e.trigger_inline_suggest(cx));
    draw(cx);
    assert_eq!(
        inline_edit(&editor, cx),
        Some(vec![
            pair("Length2", "Length3"),
            pair("", " + this.z * this.z")
        ])
    );
    editor.update(cx, |e, cx| e.accept_inline_suggestion(cx));
    assert_eq!(text(&editor, cx), POINT.replace(LENGTH_2D, LENGTH_3D));
}

#[gpui::test]
fn can_type_inline_edit(cx: &mut TestAppContext) {
    let (editor, cx) = setup(POINT, search_replace(LENGTH_2D, LENGTH_3D), |_| {}, cx);
    editor.update(cx, |e, cx| e.trigger_inline_suggest(cx));
    draw(cx);
    type_now(&editor, point_marker(1), " + t", cx);
    assert_eq!(
        inline_edit(&editor, cx),
        Some(vec![pair("Length2", "Length3"), pair("", "his.z * this.z")])
    );
    let at = cursor(&editor, cx);
    type_now(&editor, at, "his.z * this.z", cx);
    assert_eq!(
        inline_edit(&editor, cx),
        Some(vec![pair("Length2", "Length3")])
    );
}

#[gpui::test]
fn inline_edit_is_correctly_shifted_when_typing(cx: &mut TestAppContext) {
    let (editor, cx) = setup(
        POINT,
        search_replace("Ü", "{x: this.x, y: this.y}"),
        |_| {},
        cx,
    );
    editor.update(cx, |e, cx| e.trigger_inline_suggest(cx));
    draw(cx);
    assert_eq!(
        inline_edit(&editor, cx),
        Some(vec![pair("Ü", "{x: this.x, y: this.y}")])
    );
    type_now(&editor, point_marker(2), "{", cx);
    assert_eq!(
        inline_edit(&editor, cx),
        Some(vec![pair("Ü", "x: this.x, y: this.y}")])
    );
}

#[gpui::test]
fn inline_edit_stays_on_unrelated_edit(cx: &mut TestAppContext) {
    let (editor, cx) = setup(POINT, search_replace(LENGTH_2D, LENGTH_3D), |_| {}, cx);
    editor.update(cx, |e, cx| e.trigger_inline_suggest(cx));
    draw(cx);
    let expected = Some(vec![
        pair("Length2", "Length3"),
        pair("", " + this.z * this.z"),
    ]);
    assert_eq!(inline_edit(&editor, cx), expected);
    type_now(&editor, point_marker(0), "/* */", cx);
    assert_eq!(inline_edit(&editor, cx), expected);
    // The provider no longer finds the text.
    draw(cx);
    assert_eq!(inline_edit(&editor, cx), None);
}
