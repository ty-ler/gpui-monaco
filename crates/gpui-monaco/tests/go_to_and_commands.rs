//! Go to locations commands, developer and theme actions, Escape
//! cancelling requests, copy-time paste data, the long lines helper, the
//! unicode highlighter banner and symbol icon colors (#227).

use std::cell::RefCell;
use std::rc::Rc;

use gpui::{
    App, AppContext as _, ClipboardItem, Entity, Focusable, SharedString, Task, TestAppContext,
    VisualTestContext,
};
use gpui_monaco::actions::{Copy, Paste};
use gpui_monaco::cursor::Selection;
use gpui_monaco::options::GoToLocation;
use gpui_monaco::providers::*;
use gpui_monaco::text::PointUtf16;
use gpui_monaco::{CodeEditor, CodeEditorEvent, TextModel, Theme};
use serde_json::json;

fn p(row: u32, column: u32) -> PointUtf16 {
    PointUtf16::new(row, column)
}

const URI: &str = "file:///src/main.rs";
const TEXT: &str = "fn add(a: i32) {}\nfn main() {\n    add(1);\n    add(2);\n}\n";

fn setup_with(
    model: impl FnOnce() -> TextModel + 'static,
    cx: &mut TestAppContext,
) -> (Entity<CodeEditor>, &mut VisualTestContext) {
    cx.update(|cx| {
        gpui_monaco::init(cx);
        gpui_monaco::set_language_registry(cx, gpui_monaco_languages::registry());
    });
    let (editor, cx) = cx.add_window_view(|window, cx| {
        let model = cx.new(|_| model());
        CodeEditor::new(model, window, cx)
    });
    cx.update(|window, cx| window.focus(&editor.focus_handle(cx), cx));
    draw(cx);
    (editor, cx)
}

fn setup(cx: &mut TestAppContext) -> (Entity<CodeEditor>, &mut VisualTestContext) {
    setup_with(|| TextModel::new(TEXT).with_uri(URI), cx)
}

fn draw(cx: &mut VisualTestContext) {
    cx.run_until_parked();
    cx.update(|window, cx| {
        window.refresh();
        window.draw(cx).clear(cx);
    });
    cx.run_until_parked();
}

fn head(editor: &Entity<CodeEditor>, cx: &mut VisualTestContext) -> PointUtf16 {
    editor.read_with(cx, |e, _| e.selections()[0].head)
}

fn set_cursor(editor: &Entity<CodeEditor>, cx: &mut VisualTestContext, at: PointUtf16) {
    editor.update(cx, |e, cx| {
        e.set_selections(&[Selection::collapsed(at)], cx)
    });
}

fn text(editor: &Entity<CodeEditor>, cx: &mut VisualTestContext) -> String {
    editor.read_with(cx, |e, cx| e.model().read(cx).text())
}

/// A location in Monaco's JSON shape (1-based).
fn location(uri: &str, line: u32, start: u32, end: u32) -> serde_json::Value {
    json!({
        "uri": uri,
        "range": {
            "startLineNumber": line,
            "startColumn": start,
            "endLineNumber": line,
            "endColumn": end,
        }
    })
}

fn trigger(
    editor: &Entity<CodeEditor>,
    cx: &mut VisualTestContext,
    id: &str,
    payload: serde_json::Value,
) {
    editor
        .update_in(cx, |e, window, cx| {
            e.trigger_with_payload(id, payload, window, cx)
        })
        .unwrap();
    draw(cx);
}

fn run(editor: &Entity<CodeEditor>, cx: &mut VisualTestContext, id: &str) {
    editor
        .update_in(cx, |e, window, cx| e.trigger(id, window, cx))
        .unwrap();
    draw(cx);
}

fn peek_rows(editor: &Entity<CodeEditor>, cx: &mut VisualTestContext) -> Option<Vec<u32>> {
    editor
        .read_with(cx, |e, cx| e.peek_results(cx))
        .map(|(locations, _)| locations.iter().map(|l| l.range.start.row).collect())
}

// ----- editor.action.goToLocations / peekLocations ----------------------------

#[gpui::test]
fn go_to_locations_peeks_several(cx: &mut TestAppContext) {
    let (editor, cx) = setup(cx);
    trigger(
        &editor,
        cx,
        "editor.action.goToLocations",
        json!({
            "uri": URI,
            "position": { "lineNumber": 3, "column": 5 },
            "locations": [location(URI, 4, 5, 8), location(URI, 1, 4, 7)],
        }),
    );
    // The cursor moves to the position, and several results (sorted) peek
    // by default (`multipleReferences`).
    assert_eq!(head(&editor, cx), p(2, 4));
    assert_eq!(peek_rows(&editor, cx), Some(vec![0, 3]));
}

#[gpui::test]
fn go_to_locations_follows_multiple(cx: &mut TestAppContext) {
    let (editor, cx) = setup(cx);
    trigger(
        &editor,
        cx,
        "editor.action.goToLocations",
        json!({
            "uri": URI,
            "position": { "lineNumber": 3, "column": 5 },
            "locations": [location(URI, 4, 5, 8), location(URI, 1, 4, 7)],
            "multiple": "goto",
        }),
    );
    // `goto`: the first result, and F12 steps through the rest (Monaco
    // `editor.gotoNextSymbolFromResult`; there's no definition provider to
    // take the key). F4 is only for peek views.
    assert_eq!(peek_rows(&editor, cx), None);
    assert_eq!(head(&editor, cx), p(0, 3));
    assert!(cx.update(|_, cx| gpui_monaco::has_symbol_navigation(cx)));
    cx.simulate_keystrokes("f4");
    assert_eq!(head(&editor, cx), p(0, 3));
    cx.simulate_keystrokes("f12");
    assert_eq!(head(&editor, cx), p(3, 4));
}

#[gpui::test]
fn go_to_locations_without_results(cx: &mut TestAppContext) {
    let (editor, cx) = setup(cx);
    // No message unless one is given (Monaco mutes it).
    trigger(
        &editor,
        cx,
        "editor.action.goToLocations",
        json!({ "uri": URI, "position": { "lineNumber": 2, "column": 1 }, "locations": [] }),
    );
    assert!(!editor.read_with(cx, |e, _| e.is_message_visible()));
    assert_eq!(head(&editor, cx), p(1, 0));
    trigger(
        &editor,
        cx,
        "editor.action.goToLocations",
        json!({
            "uri": URI,
            "position": { "lineNumber": 3, "column": 5 },
            "locations": [],
            "noResultsMessage": "Nothing here",
        }),
    );
    assert_eq!(
        editor.read_with(cx, |e, _| e.message_text()),
        Some("Nothing here".into())
    );
}

#[gpui::test]
fn go_to_locations_for_another_model_does_nothing(cx: &mut TestAppContext) {
    let (editor, cx) = setup(cx);
    trigger(
        &editor,
        cx,
        "editor.action.goToLocations",
        json!({
            "uri": "file:///src/other.rs",
            "position": { "lineNumber": 3, "column": 5 },
            "locations": [location(URI, 1, 4, 7)],
        }),
    );
    assert_eq!(head(&editor, cx), p(0, 0));
}

#[gpui::test]
fn peek_locations_peeks_even_one(cx: &mut TestAppContext) {
    let (editor, cx) = setup(cx);
    trigger(
        &editor,
        cx,
        "editor.action.peekLocations",
        json!({
            "uri": URI,
            "position": { "lineNumber": 3, "column": 5 },
            "locations": [location(URI, 1, 4, 7)],
        }),
    );
    assert_eq!(peek_rows(&editor, cx), Some(vec![0]));
    cx.simulate_keystrokes("escape");
    // `showReferences` is an alias.
    trigger(
        &editor,
        cx,
        "editor.action.showReferences",
        json!({
            "position": { "lineNumber": 3, "column": 5 },
            "locations": [location(URI, 4, 5, 8)],
        }),
    );
    assert_eq!(peek_rows(&editor, cx), Some(vec![3]));
}

#[gpui::test]
fn go_to_locations_runs_as_a_command(cx: &mut TestAppContext) {
    let (editor, cx) = setup(cx);
    // Positional arguments, as Monaco's `executeCommand` passes them, e.g.
    // from a code lens.
    cx.update(|_, cx| {
        gpui_monaco::execute_command(
            cx,
            &Command {
                id: "editor.action.goToLocations".into(),
                title: SharedString::default(),
                arguments: vec![
                    json!(URI),
                    json!({ "lineNumber": 4, "column": 5 }),
                    json!([location(URI, 1, 4, 7)]),
                ],
                ..Default::default()
            },
        )
        .unwrap();
    });
    draw(cx);
    // One result: selected.
    assert_eq!(head(&editor, cx), p(0, 3));
    cx.update(|_, cx| {
        gpui_monaco::execute_command(
            cx,
            &Command {
                id: "editor.action.peekLocations".into(),
                title: SharedString::default(),
                arguments: vec![
                    json!(URI),
                    json!({ "lineNumber": 4, "column": 5 }),
                    json!([location(URI, 1, 4, 7), location(URI, 3, 5, 8)]),
                    json!("goto"),
                ],
                ..Default::default()
            },
        )
        .unwrap();
    });
    draw(cx);
    assert_eq!(head(&editor, cx), p(3, 4));
    assert_eq!(peek_rows(&editor, cx), Some(vec![0, 2]));
}

#[gpui::test]
fn go_to_locations_in_other_models_asks_the_host(cx: &mut TestAppContext) {
    let (editor, cx) = setup(cx);
    let opened = Rc::new(RefCell::new(Vec::new()));
    let log = opened.clone();
    cx.update(|_, cx| {
        cx.subscribe(&editor, move |_, event: &CodeEditorEvent, _| {
            if let CodeEditorEvent::OpenLocation(location) = event {
                log.borrow_mut().push(location.clone());
            }
        })
        .detach();
    });
    editor.update(cx, |e, cx| {
        e.go_to_locations(
            p(2, 4),
            vec![Location {
                uri: Some("file:///src/lib.rs".into()),
                range: p(4, 0)..p(4, 3),
                ..Default::default()
            }],
            Some(GoToLocation::Goto),
            None,
            cx,
        )
    });
    draw(cx);
    assert_eq!(opened.borrow().len(), 1);
    assert_eq!(
        opened.borrow()[0].uri.as_deref(),
        Some("file:///src/lib.rs")
    );
}

#[gpui::test]
fn multiple_tests_defaults_to_peek(cx: &mut TestAppContext) {
    let (editor, cx) = setup(cx);
    let options = editor.read_with(cx, |e, _| e.options().goto_location.clone());
    assert_eq!(options.multiple_tests, GoToLocation::Peek);
}

// ----- Developer and theme actions -------------------------------------------

#[gpui::test]
fn force_retokenize_parses_again(cx: &mut TestAppContext) {
    let (editor, cx) = setup_with(
        || TextModel::new(TEXT).with_uri(URI).with_language("rust"),
        cx,
    );
    let before = editor.read_with(cx, |e, cx| e.model().read(cx).syntax_snapshot().is_some());
    assert!(before);
    run(&editor, cx, "editor.action.forceRetokenize");
    editor.read_with(cx, |e, cx| {
        let model = e.model().read(cx);
        assert!(!model.is_parsing());
        let syntax = model.syntax_snapshot().unwrap();
        assert_eq!(syntax.text.version(), model.version());
        assert_eq!(
            model.token_type_at(0),
            gpui_monaco::cursor::StandardTokenType::Other
        );
    });
    // A model without a grammar has nothing to do.
    let plain = editor.update(cx, |e, cx| {
        e.model().update(cx, |m, cx| {
            m.set_language(None, cx);
            m.force_retokenize(cx)
        })
    });
    assert_eq!(plain, None);
}

#[gpui::test]
fn inspect_tokens_follows_the_cursor(cx: &mut TestAppContext) {
    let (editor, cx) = setup_with(
        || {
            TextModel::new("fn main() {\n\tlet s = \"hi\";\n}\n")
                .with_uri(URI)
                .with_language("rust")
        },
        cx,
    );
    run(&editor, cx, "editor.action.inspectTokens");
    let info = editor
        .read_with(cx, |e, _| e.token_inspection().cloned())
        .expect("shown");
    assert_eq!(info.text, "fn");
    assert_eq!(info.length, 2);
    assert_eq!(info.language.as_ref(), "rust");
    assert_eq!(info.token, "keyword.rust");
    assert_eq!(info.token_type, "Other");
    assert_eq!(info.font_style, "---");
    // vs-dark's keyword and background colors.
    assert_eq!(info.foreground, "#569cd6");
    assert_eq!(info.background, "#1e1e1e");
    assert!(info.capture.is_some());
    // Moving the cursor inspects the token there.
    set_cursor(&editor, cx, p(1, 10));
    let info = editor
        .read_with(cx, |e, _| e.token_inspection().cloned())
        .unwrap();
    assert_eq!(info.text, "\"hi\"");
    assert_eq!(info.token, "string.rust");
    assert_eq!(info.token_type, "String");
    // Between highlighted tokens (the grammar doesn't capture `s` or `=`),
    // with whitespace as `·`.
    let between = editor.read_with(cx, |e, cx| e.inspect_token_at(p(1, 4), cx));
    assert_eq!(between.text, "·s·=·");
    assert_eq!(between.token, "");
    let tab = editor.read_with(cx, |e, cx| e.inspect_token_at(p(1, 0), cx));
    assert_eq!(tab.text, "→");
    // Escape closes it.
    cx.simulate_keystrokes("escape");
    draw(cx);
    assert!(editor.read_with(cx, |e, _| e.token_inspection().is_none()));
}

#[gpui::test]
fn inspect_tokens_stops_when_the_model_changes(cx: &mut TestAppContext) {
    let (editor, cx) = setup_with(|| TextModel::new(TEXT), cx);
    run(&editor, cx, "editor.action.inspectTokens");
    let info = editor
        .read_with(cx, |e, _| e.token_inspection().cloned())
        .unwrap();
    // No grammar: one token for the line, in the editor's colors.
    assert_eq!(info.text, "fn·add(a:·i32)·{}");
    assert_eq!(info.token, "");
    assert_eq!(info.language.as_ref(), "plaintext");
    let other = cx.new(|_| TextModel::new("x"));
    editor.update(cx, |e, cx| e.set_model(other, cx));
    assert!(editor.read_with(cx, |e, _| e.token_inspection().is_none()));
}

fn theme_name(editor: &Entity<CodeEditor>, cx: &mut VisualTestContext) -> String {
    editor.read_with(cx, |e, _| e.theme().name.clone())
}

#[gpui::test]
fn toggle_high_contrast_and_back(cx: &mut TestAppContext) {
    let (editor, cx) = setup(cx);
    assert_eq!(theme_name(&editor, cx), "vs-dark");
    run(&editor, cx, "editor.action.toggleHighContrast");
    assert_eq!(theme_name(&editor, cx), "hc-black");
    assert!(editor.read_with(cx, |e, _| e.theme().high_contrast));
    run(&editor, cx, "editor.action.toggleHighContrast");
    assert_eq!(theme_name(&editor, cx), "vs-dark");
    // A light theme toggles to hc-light, and back to the same theme.
    cx.update(|_, cx| {
        let colors = std::collections::HashMap::new();
        let theme = Theme::define("my-light", &Theme::vs(), true, Vec::new(), &colors);
        gpui_monaco::define_theme(cx, theme).unwrap();
        gpui_monaco::set_theme(cx, "my-light");
    });
    run(&editor, cx, "editor.action.toggleHighContrast");
    assert_eq!(theme_name(&editor, cx), "hc-light");
    run(&editor, cx, "editor.action.toggleHighContrast");
    assert_eq!(theme_name(&editor, cx), "my-light");
}

#[gpui::test]
fn high_contrast_themes(cx: &mut TestAppContext) {
    cx.update(|_| {
        assert!(Theme::hc_black().high_contrast);
        assert!(Theme::hc_light().high_contrast);
        assert!(!Theme::vs().high_contrast);
        let colors = std::collections::HashMap::new();
        let hc = Theme::define("x", &Theme::hc_black(), true, Vec::new(), &colors);
        assert!(hc.high_contrast);
        let json = r#"{ "name": "HC", "type": "hc", "colors": {} }"#;
        assert!(Theme::from_vscode_json(json).unwrap().high_contrast);
    });
}

#[gpui::test]
fn go_to_offset_opens_quick_access(cx: &mut TestAppContext) {
    let (editor, cx) = setup(cx);
    run(&editor, cx, "editor.action.gotoOffset");
    let rows = editor.read_with(cx, |e, cx| e.quick_access_rows_for_tests(cx));
    assert_eq!(rows.len(), 1);
    assert!(
        rows[0].contains("Type a character position to go to"),
        "{rows:?}"
    );
    // It's in the command palette.
    let actions = editor.read_with(cx, |e, cx| e.supported_actions(cx));
    for (id, label) in [
        ("editor.action.gotoOffset", "Go to Offset..."),
        (
            "editor.action.forceRetokenize",
            "Developer: Force Retokenize",
        ),
        ("editor.action.inspectTokens", "Developer: Inspect Tokens"),
        (
            "editor.action.toggleHighContrast",
            "Toggle High Contrast Theme",
        ),
    ] {
        let action = actions
            .iter()
            .find(|a| a.id.as_ref() == id)
            .unwrap_or_else(|| panic!("{id}"));
        assert_eq!(action.label.as_ref(), label);
    }
}

// ----- editor.cancelOperation ------------------------------------------------

/// Answers go to definition only when told to.
struct Pending {
    rx: async_channel::Receiver<()>,
}

impl DefinitionProvider for Pending {
    fn provide_definition(
        &self,
        _: &ProviderContext,
        _: PointUtf16,
        cx: &mut App,
    ) -> Task<anyhow::Result<Vec<Location>>> {
        let rx = self.rx.clone();
        cx.background_spawn(async move {
            rx.recv().await.ok();
            Ok(vec![Location {
                uri: None,
                range: PointUtf16::new(0, 3)..PointUtf16::new(0, 6),
                ..Default::default()
            }])
        })
    }
}

#[gpui::test]
fn escape_cancels_a_pending_request(cx: &mut TestAppContext) {
    let (tx, rx) = async_channel::unbounded();
    cx.update(|cx| {
        register_definition_provider(cx, LanguageSelector::All, Pending { rx });
    });
    let (editor, cx) = setup(cx);
    set_cursor(&editor, cx, p(2, 5));
    editor.update(cx, |e, cx| {
        e.go_to(gpui_monaco::options::LocationKind::Definition, cx)
    });
    cx.run_until_parked();
    assert!(editor.read_with(cx, |e, _| e.has_cancellable_operation()));
    cx.simulate_keystrokes("escape");
    assert!(!editor.read_with(cx, |e, _| e.has_cancellable_operation()));
    tx.try_send(()).ok();
    draw(cx);
    // The answer came too late.
    assert_eq!(head(&editor, cx), p(2, 5));
    // Without a pending request, Escape does what it did before.
    editor.update(cx, |e, cx| {
        e.set_selections(
            &[Selection::collapsed(p(1, 0)), Selection::collapsed(p(2, 0))],
            cx,
        )
    });
    cx.simulate_keystrokes("escape");
    assert_eq!(editor.read_with(cx, |e, _| e.selections().len()), 1);
    // A finished request isn't pending.
    editor.update(cx, |e, cx| {
        e.go_to(gpui_monaco::options::LocationKind::Definition, cx)
    });
    tx.try_send(()).ok();
    draw(cx);
    assert_eq!(head(&editor, cx), p(0, 3));
    assert!(!editor.read_with(cx, |e, _| e.has_cancellable_operation()));
    // `editor.cancelOperation` by id.
    editor.update(cx, |e, cx| {
        e.go_to(gpui_monaco::options::LocationKind::Definition, cx)
    });
    cx.run_until_parked();
    run(&editor, cx, "editor.cancelOperation");
    assert!(!editor.read_with(cx, |e, _| e.has_cancellable_operation()));
}

// ----- prepareDocumentPaste --------------------------------------------------

const IMPORTS: &str = "application/vnd.test.imports";

/// Records what was copied, and pastes it with a comment naming it.
struct WithImports {
    copied: Rc<RefCell<Vec<Vec<std::ops::Range<PointUtf16>>>>>,
}

impl DocumentPasteEditProvider for WithImports {
    fn paste_mime_types(&self) -> Vec<SharedString> {
        vec![IMPORTS.into()]
    }

    fn copy_mime_types(&self) -> Vec<SharedString> {
        vec![IMPORTS.into()]
    }

    fn prepare_document_paste(
        &self,
        _: &ProviderContext,
        ranges: &[std::ops::Range<PointUtf16>],
        _: &mut App,
    ) -> Option<Task<anyhow::Result<DataTransfer>>> {
        self.copied.borrow_mut().push(ranges.to_vec());
        let mut data = DataTransfer::default();
        data.insert(IMPORTS, DataTransferItem::Text("use add;".into()));
        Some(Task::ready(Ok(data)))
    }

    fn provide_document_paste_edits(
        &self,
        _: &ProviderContext,
        _: &[std::ops::Range<PointUtf16>],
        data: &DataTransfer,
        _: DocumentPasteTriggerKind,
        _: &mut App,
    ) -> Task<anyhow::Result<Vec<DocumentDropOrPasteEdit>>> {
        let imports = data.text(IMPORTS).unwrap_or_default();
        let text = data.text("text/plain").unwrap_or_default();
        Task::ready(Ok(vec![DocumentDropOrPasteEdit {
            title: "With imports".into(),
            kind: Some("text.imports".into()),
            insert_text: EditInsertText::Text(format!("{text} // {imports}")),
            ..Default::default()
        }]))
    }
}

#[gpui::test]
fn copy_prepares_data_for_the_paste(cx: &mut TestAppContext) {
    let copied = Rc::new(RefCell::new(Vec::new()));
    let provider = WithImports {
        copied: copied.clone(),
    };
    cx.update(|cx| {
        register_document_paste_edit_provider(cx, LanguageSelector::All, provider);
    });
    let (editor, cx) = setup_with(|| TextModel::new("add(1);\nx"), cx);
    editor.update(cx, |e, cx| {
        e.set_selections(&[Selection::new(p(0, 0), p(0, 3))], cx)
    });
    cx.dispatch_action(Copy);
    draw(cx);
    assert_eq!(*copied.borrow(), vec![vec![p(0, 0)..p(0, 3)]]);
    set_cursor(&editor, cx, p(1, 1));
    cx.dispatch_action(Paste);
    draw(cx);
    assert_eq!(text(&editor, cx), "add(1);\nxadd // use add;");
    // Text copied elsewhere has no data from the copy: the provider can't
    // handle it, so the paste is plain.
    cx.write_to_clipboard(ClipboardItem::new_string("y".into()));
    cx.dispatch_action(Paste);
    draw(cx);
    assert!(
        text(&editor, cx).ends_with("use add;y"),
        "{}",
        text(&editor, cx)
    );
    // An empty selection copies its line.
    set_cursor(&editor, cx, p(0, 2));
    cx.dispatch_action(Copy);
    draw(cx);
    assert_eq!(copied.borrow()[1], vec![p(0, 0)..p(0, 7)]);
}

// ----- The long lines helper -------------------------------------------------

#[gpui::test]
fn clicking_past_the_rendering_limit_turns_it_off(cx: &mut TestAppContext) {
    let (editor, cx) = setup_with(|| TextModel::new(&"x".repeat(200)), cx);
    editor.update(cx, |e, cx| {
        let mut options = e.options().clone();
        options.stop_rendering_line_after = Some(50);
        e.update_options(options, cx);
    });
    draw(cx);
    let click = |editor: &Entity<CodeEditor>, cx: &mut VisualTestContext, column: u32| {
        let at = editor
            .read_with(cx, |e, _| e.scrolled_visible_position(p(0, column)))
            .unwrap();
        cx.simulate_click(
            gpui::point(at.x + gpui::px(1.), at.y + gpui::px(2.)),
            gpui::Modifiers::none(),
        );
        draw(cx);
    };
    // Before the limit: nothing changes.
    click(&editor, cx, 10);
    assert_eq!(
        editor.read_with(cx, |e, _| e.options().stop_rendering_line_after),
        Some(50)
    );
    // At Monaco's column 50 (the last one drawn).
    click(&editor, cx, 49);
    assert_eq!(
        editor.read_with(cx, |e, _| e.options().stop_rendering_line_after),
        None
    );
}

/// Monaco `.mtkoverflow`: `padding: 4px` inside a 1px border, so the chip's
/// box is the line's last 10px wider than its text and overflows the line
/// by 5px above and below.
#[gpui::test]
fn the_show_more_chip_is_padded(cx: &mut TestAppContext) {
    let (editor, cx) = setup_with(|| TextModel::new(&"x".repeat(200)), cx);
    editor.update(cx, |e, cx| {
        let mut options = e.options().clone();
        options.stop_rendering_line_after = Some(50);
        e.update_options(options, cx);
    });
    draw(cx);
    let (chips, start, at_limit, width, line_height) = editor.read_with(cx, |e, _| {
        (
            e.overflow_chips_for_tests(),
            e.scrolled_visible_position(p(0, 0)).unwrap(),
            e.scrolled_visible_position(p(0, 50)).unwrap(),
            e.width_of_line(0).unwrap(),
            e.options().resolved_line_height(),
        )
    });
    let [chip] = chips[..] else {
        panic!("{chips:?}");
    };
    // The line is shaped around the box: it starts where drawing stops and
    // ends the line.
    assert_eq!(chip.origin.x, at_limit.x);
    assert_eq!(chip.right(), start.x + width);
    // 5px on each side of its text.
    let (family, font_size) = editor.read_with(cx, |e, _| {
        (e.options().font_family.clone(), e.options().font_size)
    });
    let text = "Show more (150 chars)";
    let text_width = cx.update(|window, _| {
        let run = gpui::TextRun {
            len: text.len(),
            font: gpui::font(family),
            color: gpui::black(),
            background_color: None,
            underline: None,
            strikethrough: None,
        };
        window
            .text_system()
            .shape_line(text.into(), font_size, &[run], None)
            .width
    });
    assert!((chip.size.width - text_width - gpui::px(10.)).abs() < gpui::px(0.01));
    // Its text's height plus 5px each way, centered on the line.
    assert!(chip.size.height > gpui::px(10.));
    let middle = chip.origin.y + chip.size.height / 2.;
    assert_eq!(middle, start.y + line_height / 2.);
}

// ----- The unicode highlighter banner ----------------------------------------

#[gpui::test]
fn too_many_characters_show_a_banner(cx: &mut TestAppContext) {
    // More invisible characters than Monaco highlights.
    let text = "a\u{200b}".repeat(1_100);
    let (editor, cx) = setup_with(move || TextModel::new(&text), cx);
    draw(cx);
    let banner = editor.read_with(cx, |e, _| e.unicode_highlight_banner());
    assert_eq!(
        banner,
        Some((
            "This document contains many invisible unicode characters".into(),
            "Disable Invisible Highlight".into()
        ))
    );
    assert!(editor.read_with(cx, |e, _| e.has_banner()));
    let height =
        |cx: &mut VisualTestContext| editor.read_with(cx, |e, _| e.layout_info().unwrap().height);
    let with_banner = height(cx);
    // Its command turns the kind off, and the banner goes.
    run(
        &editor,
        cx,
        "editor.action.unicodeHighlight.disableHighlightingOfInvisibleCharacters",
    );
    draw(cx);
    assert!(!editor.read_with(cx, |e, _| {
        e.options().unicode_highlight.invisible_characters
    }));
    assert_eq!(
        editor.read_with(cx, |e, _| e.unicode_highlight_banner()),
        None
    );
    assert!(!editor.read_with(cx, |e, _| e.has_banner()));
    // The banner took its height off the text (Monaco `setBanner`).
    assert_eq!(height(cx) - with_banner, gpui::px(26.));
}

#[gpui::test]
fn a_closed_banner_stays_closed_for_the_model(cx: &mut TestAppContext) {
    let text = "a\u{200b}".repeat(1_100);
    let again = text.clone();
    let (editor, cx) = setup_with(move || TextModel::new(&text), cx);
    draw(cx);
    assert!(editor.update(cx, |e, cx| e.close_unicode_highlight_banner(cx)));
    assert!(!editor.read_with(cx, |e, _| e.has_banner()));
    // Edits don't bring it back.
    cx.simulate_input("b");
    cx.executor()
        .advance_clock(std::time::Duration::from_millis(300));
    draw(cx);
    assert!(!editor.read_with(cx, |e, _| e.has_banner()));
    // Another model does.
    let other = cx.new(|_| TextModel::new(&again));
    editor.update(cx, |e, cx| e.set_model(other, cx));
    cx.executor()
        .advance_clock(std::time::Duration::from_millis(300));
    draw(cx);
    assert!(editor.read_with(cx, |e, _| e.has_banner()));
    // The other disable commands exist too.
    for id in [
        "editor.action.unicodeHighlight.disableHighlightingOfAmbiguousCharacters",
        "editor.action.unicodeHighlight.disableHighlightingOfNonBasicAsciiCharacters",
    ] {
        run(&editor, cx, id);
    }
    let options = editor.read_with(cx, |e, _| e.options().unicode_highlight.clone());
    assert!(!options.ambiguous_characters && !options.non_basic_ascii);
}

// ----- Symbol icon colors ----------------------------------------------------

#[gpui::test]
fn symbol_icon_colors_are_themeable(cx: &mut TestAppContext) {
    cx.update(|_| {
        let hex = |value: &str| gpui_monaco::theme::parse_hex_color(value).unwrap();
        let dark = Theme::vs_dark().colors.symbol_icons;
        assert_eq!(
            dark.for_completion_kind(CompletionItemKind::Method),
            Some(hex("#B180D7"))
        );
        assert_eq!(
            dark.for_completion_kind(CompletionItemKind::Class),
            Some(hex("#EE9D28"))
        );
        assert_eq!(
            dark.for_completion_kind(CompletionItemKind::Value),
            Some(hex("#EE9D28"))
        );
        assert_eq!(
            dark.for_completion_kind(CompletionItemKind::Text),
            Some(hex("#CCCCCC"))
        );
        assert_eq!(dark.for_completion_kind(CompletionItemKind::User), None);
        assert_eq!(dark.for_symbol_kind(SymbolKind::Variable), hex("#75BEFF"));
        let light = Theme::vs().colors.symbol_icons;
        assert_eq!(light.for_symbol_kind(SymbolKind::Function), hex("#652D90"));
        assert_eq!(light.for_symbol_kind(SymbolKind::String), hex("#616161"));
        let json = r##"{
            "type": "dark",
            "colors": {
                "foreground": "#abcdef",
                "symbolIcon.functionForeground": "#123456",
                "banner.background": "#222222"
            }
        }"##;
        let theme = Theme::from_vscode_json(json).unwrap();
        let icons = &theme.colors.symbol_icons;
        assert_eq!(icons.function, hex("#123456"));
        // Colors defaulting to `foreground` follow it; others keep theirs.
        assert_eq!(icons.string, hex("#abcdef"));
        assert_eq!(icons.class, hex("#EE9D28"));
        assert_eq!(theme.colors.banner_background, hex("#222222"));
    });
}
