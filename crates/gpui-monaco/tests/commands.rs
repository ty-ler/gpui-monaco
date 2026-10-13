//! Commands, keybindings and context keys (#261): Monaco's command ids and
//! their arguments through `trigger`, preconditions, context keys, the
//! host's commands, actions and context keys, `executeCommand`, and the
//! screen reader alerts that come with commands; and their leftovers (#300):
//! the mouse commands' `viewPosition`, `pasteAs`' `preferences`,
//! `inlineSuggest.trigger`'s arguments and widgets' preconditions.

use std::cell::RefCell;
use std::ops::Range;
use std::rc::Rc;
use std::time::Duration;

use gpui::{
    App, AppContext as _, Context, Entity, Focusable, IntoElement, ParentElement, Render, Styled,
    Task, TestAppContext, VisualTestContext, Window, div,
};
use gpui_monaco::cursor::{CursorModel, Selection};
use gpui_monaco::options::WordWrap;
use gpui_monaco::providers::*;
use gpui_monaco::text::PointUtf16;
use gpui_monaco::{
    CodeEditor, CodeEditorEvent, CursorStateComputerData, EditOperationBuilder, EditorCommand,
    KeybindingRule, Marker, MarkerSeverity, TextModel,
};
use serde_json::json;

fn p(row: u32, column: u32) -> PointUtf16 {
    PointUtf16::new(row, column)
}

const TEXT: &str = "fn add(a: i32) -> i32 {\n    a + 1\n}\nfn main() {\n    add(1);\n}\n";

fn setup_model(
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
    setup_model(
        || {
            TextModel::new(TEXT)
                .with_language("rust")
                .with_uri("file:///src/main.rs")
        },
        cx,
    )
}

fn draw(cx: &mut VisualTestContext) {
    cx.run_until_parked();
    cx.update(|window, cx| {
        window.refresh();
        window.draw(cx).clear(cx);
    });
    cx.run_until_parked();
}

fn text(editor: &Entity<CodeEditor>, cx: &mut VisualTestContext) -> String {
    editor.read_with(cx, |e, cx| e.model().read(cx).text())
}

fn selections(editor: &Entity<CodeEditor>, cx: &mut VisualTestContext) -> Vec<Selection> {
    editor.read_with(cx, |e, _| e.selections())
}

fn head(editor: &Entity<CodeEditor>, cx: &mut VisualTestContext) -> PointUtf16 {
    selections(editor, cx)[0].head
}

fn set_cursor(editor: &Entity<CodeEditor>, cx: &mut VisualTestContext, at: PointUtf16) {
    editor.update(cx, |e, cx| {
        e.set_selections(&[Selection::collapsed(at)], cx)
    });
    draw(cx);
}

fn trigger(editor: &Entity<CodeEditor>, cx: &mut VisualTestContext, id: &str) {
    editor
        .update_in(cx, |e, window, cx| e.trigger(id, window, cx))
        .unwrap_or_else(|err| panic!("{id}: {err}"));
    draw(cx);
}

fn trigger_with(
    editor: &Entity<CodeEditor>,
    cx: &mut VisualTestContext,
    id: &str,
    payload: serde_json::Value,
) {
    editor
        .update_in(cx, |e, window, cx| {
            e.trigger_with_payload(id, payload, window, cx)
        })
        .unwrap_or_else(|err| panic!("{id}: {err}"));
    draw(cx);
}

fn has_key(editor: &Entity<CodeEditor>, cx: &mut VisualTestContext, key: &str) -> bool {
    editor.read_with(cx, |e, cx| e.context_keys(cx).contains(key))
}

fn key_value(editor: &Entity<CodeEditor>, cx: &mut VisualTestContext, key: &str) -> Option<String> {
    editor.read_with(cx, |e, cx| {
        e.context_keys(cx).get(key).map(|v| v.to_string())
    })
}

fn supported(editor: &Entity<CodeEditor>, cx: &mut VisualTestContext, id: &str) -> bool {
    editor.read_with(cx, |e, cx| {
        e.supported_actions(cx).iter().any(|a| a.id.as_ref() == id)
    })
}

fn announcement(editor: &Entity<CodeEditor>, cx: &mut VisualTestContext) -> Option<String> {
    editor.read_with(cx, |e, _| {
        e.last_announcement_for_tests().map(|a| a.to_string())
    })
}

fn read_only(editor: &Entity<CodeEditor>, cx: &mut VisualTestContext) {
    editor.update(cx, |e, cx| {
        let mut options = e.options().clone();
        options.read_only = true;
        e.update_options(options, cx);
    });
    draw(cx);
}

// ----- every Monaco command id -------------------------------------------------

/// Monaco command ids with nothing to run in an editor, and why.
const NOT_EDITOR_COMMANDS: &[(&str, &str)] = &[
    // Copy with syntax highlighting is #231.
    (
        "editor.action.clipboardCopyWithSyntaxHighlightingAction",
        "#231",
    ),
    // GPU renderer debugging and an inline completions dev command.
    ("editor.action.debugEditorGpuRenderer", "not planned"),
    ("editor.action.inlineSuggest.dev.extractRepro", "dev only"),
    // Run by the workbench's inline completion rename tracking.
    ("editor.action.inlineSuggest.renameSymbol", "workbench"),
    // Built by `goToLocations`' Go to Any Symbol, never registered alone.
    ("editor.action.goToLocation", "not a registered command"),
    // The workbench's reference tree list commands.
    ("openReference", "workbench"),
    ("openReferenceToSide", "workbench"),
    ("revealReference", "workbench"),
];

/// Monaco's internal API commands return what providers answer; they're
/// functions in `gpui_monaco::api_commands` (the in-scope ones) rather than
/// editor commands.
fn is_api_command(id: &str) -> bool {
    id.starts_with("_execute") || id.starts_with("_provide")
}

/// Every command id registered in Monaco's `coreCommands.ts`,
/// `editorExtensions.ts`, `contrib/` and `standalone/` (from the vendored
/// source, see `script/porting/monaco_command_ids.py`) resolves through
/// `trigger`.
#[gpui::test]
fn every_monaco_command_resolves(cx: &mut TestAppContext) {
    let (editor, cx) = setup(cx);
    let ids: Vec<String> = include_str!("fixtures/monaco_command_ids.txt")
        .lines()
        .filter(|line| !line.starts_with('#') && !line.is_empty())
        .map(|line| line.split('\t').next().unwrap().to_string())
        .collect();
    assert!(ids.len() > 350, "{}", ids.len());
    let mut unresolved = Vec::new();
    for id in &ids {
        if is_api_command(id) || NOT_EDITOR_COMMANDS.iter().any(|(skip, _)| skip == id) {
            continue;
        }
        let result = editor.update_in(cx, |e, window, cx| e.trigger(id, window, cx));
        if result.is_err() {
            unresolved.push(id.clone());
        }
        // Close whatever opened, so the next command finds the editor.
        cx.update(|window, cx| window.focus(&editor.focus_handle(cx), cx));
        cx.run_until_parked();
    }
    assert!(unresolved.is_empty(), "unresolved: {unresolved:?}");
    // The skipped ids still exist upstream.
    for (id, _) in NOT_EDITOR_COMMANDS {
        assert!(ids.contains(&id.to_string()), "{id} is gone from Monaco");
    }
}

// ----- provider preconditions use the model's uri ----------------------------

struct Definitions(Vec<Location>);

impl DefinitionProvider for Definitions {
    fn provide_definition(
        &self,
        _: &ProviderContext,
        _: PointUtf16,
        _: &mut App,
    ) -> Task<anyhow::Result<Vec<Location>>> {
        Task::ready(Ok(self.0.clone()))
    }
}

/// A provider registered for a uri scheme serves go to definition, and its
/// actions show (Monaco scores the whole selector, not just the language).
#[gpui::test]
fn scheme_selectors_enable_go_to_definition(cx: &mut TestAppContext) {
    let (editor, cx) = setup_model(
        || {
            TextModel::new(TEXT)
                .with_language("rust")
                .with_uri("inmemory://model/1")
        },
        cx,
    );
    assert!(!supported(&editor, cx, "editor.action.revealDefinition"));
    cx.update(|_, cx| {
        register_definition_provider(
            cx,
            LanguageSelector::Filters(vec![LanguageFilter {
                scheme: Some("inmemory".into()),
                ..Default::default()
            }]),
            Definitions(vec![Location::new(None, p(0, 3)..p(0, 6))]),
        );
    });
    draw(cx);
    assert!(supported(&editor, cx, "editor.action.revealDefinition"));
    assert!(supported(&editor, cx, "editor.action.peekDefinition"));
    assert!(has_key(&editor, cx, "editorHasDefinitionProvider"));
    set_cursor(&editor, cx, p(4, 5));
    cx.simulate_keystrokes("f12");
    draw(cx);
    assert_eq!(head(&editor, cx), p(0, 3));
}

struct RangeFormatter;

impl DocumentRangeFormattingProvider for RangeFormatter {
    fn provide_document_range_formatting_edits(
        &self,
        _: &ProviderContext,
        range: Range<PointUtf16>,
        _: FormattingOptions,
        _: &mut App,
    ) -> Task<anyhow::Result<FormattingEdits>> {
        Task::ready(Ok(vec![(
            range.start..range.start,
            "// formatted\n".into(),
        )]
        .into()))
    }
}

/// Monaco: a range formatter formats documents too, so Format Document
/// shows with only one.
#[gpui::test]
fn format_document_with_only_a_range_formatter(cx: &mut TestAppContext) {
    let (editor, cx) = setup(cx);
    assert!(!supported(&editor, cx, "editor.action.formatDocument"));
    cx.update(|_, cx| {
        register_document_range_formatting_provider(cx, LanguageSelector::All, RangeFormatter);
    });
    draw(cx);
    assert!(supported(&editor, cx, "editor.action.formatDocument"));
    assert!(supported(&editor, cx, "editor.action.formatSelection"));
    trigger(&editor, cx, "editor.action.formatDocument");
    assert!(text(&editor, cx).starts_with("// formatted\nfn add"));
    // The legacy `editor.action.format` formats the document without a
    // selection.
    trigger(&editor, cx, "editor.action.format");
    assert!(text(&editor, cx).starts_with("// formatted\n// formatted\n"));
}

// ----- keybindings ----------------------------------------------------------

fn bound(keystrokes: &str, action: &str) -> bool {
    gpui_monaco::actions::default_key_bindings()
        .iter()
        .any(|b| {
            b.action().name() == action
                && b.keystrokes()
                    .iter()
                    .map(|k| k.inner().unparse())
                    .collect::<Vec<_>>()
                    .join(" ")
                    == keystrokes
        })
}

/// Monaco binds nothing to Alt+\ (it types « on macOS).
#[test]
fn alt_backslash_is_unbound() {
    assert!(
        !gpui_monaco::actions::default_key_bindings()
            .iter()
            .any(|b| b.action().name() == "code_editor::TriggerInlineSuggest")
    );
}

/// Shift+Escape is Monaco's secondary key everywhere Escape closes
/// something; on Windows Ctrl+Left/Right do accessibility word moves for
/// screen readers.
#[test]
fn secondary_escape_and_platform_bindings() {
    assert!(bound("shift-escape", "code_editor::SecondaryEscape"));
    assert!(bound("shift-escape", "code_editor::ClosePeek"));
    assert!(bound("shift-escape", "code_editor::CancelRenameInput"));
    assert!(bound("shift-escape", "code_editor::CloseParameterHints"));
    assert_eq!(
        bound("ctrl-left", "code_editor::CursorWordAccessibilityLeft"),
        cfg!(target_os = "windows")
    );
}

#[gpui::test]
fn shift_escape_closes_what_monaco_gives_it(cx: &mut TestAppContext) {
    let (editor, cx) = setup(cx);
    // Secondary cursors, then the selection.
    editor.update(cx, |e, cx| {
        e.set_selections(
            &[
                Selection::new(p(0, 0), p(0, 2)),
                Selection::collapsed(p(1, 0)),
            ],
            cx,
        )
    });
    cx.simulate_keystrokes("shift-escape");
    assert_eq!(selections(&editor, cx).len(), 1);
    assert_eq!(
        announcement(&editor, cx).as_deref(),
        Some("Removed secondary cursors")
    );
    cx.simulate_keystrokes("shift-escape");
    assert!(selections(&editor, cx)[0].is_empty());
    // The find widget.
    cx.simulate_keystrokes(if cfg!(target_os = "macos") {
        "cmd-f"
    } else {
        "ctrl-f"
    });
    draw(cx);
    assert!(editor.read_with(cx, |e, _| e.is_find_visible()));
    cx.simulate_keystrokes("shift-escape");
    draw(cx);
    assert!(!editor.read_with(cx, |e, _| e.is_find_visible()));
    // Snippet mode.
    editor.update(cx, |e, cx| {
        e.insert_snippet("x(${1:a}, ${2:b})", Default::default(), cx)
    });
    draw(cx);
    assert!(has_key(&editor, cx, "inSnippetMode"));
    cx.simulate_keystrokes("shift-escape");
    draw(cx);
    assert!(!editor.read_with(cx, |e, _| e.in_snippet_mode()));
    // Not the selection anchor, which Escape cancels.
    trigger(&editor, cx, "editor.action.setSelectionAnchor");
    cx.simulate_keystrokes("shift-escape");
    assert!(has_key(&editor, cx, "selectionAnchorSet"));
    cx.simulate_keystrokes("escape");
    assert!(!has_key(&editor, cx, "selectionAnchorSet"));
}

/// Monaco folds preconditions into keybindings: in a read-only editor
/// Cmd/Ctrl+/ does nothing, not even the read-only message.
#[gpui::test]
fn keybindings_respect_preconditions(cx: &mut TestAppContext) {
    let (editor, cx) = setup(cx);
    read_only(&editor, cx);
    cx.simulate_keystrokes(if cfg!(target_os = "macos") {
        "cmd-/"
    } else {
        "ctrl-/"
    });
    draw(cx);
    assert_eq!(text(&editor, cx), TEXT);
    assert!(!editor.read_with(cx, |e, _| e.is_message_visible()));
    // Typing still says why it can't edit.
    cx.simulate_input("x");
    draw(cx);
    assert!(editor.read_with(cx, |e, _| e.is_message_visible()));
    // `trigger` does nothing either.
    trigger(&editor, cx, "editor.action.commentLine");
    assert_eq!(text(&editor, cx), TEXT);
}

/// Monaco's palette preconditions: the selection anchor, folding and word
/// highlight actions need their state.
#[gpui::test]
fn palette_preconditions(cx: &mut TestAppContext) {
    let (editor, cx) = setup(cx);
    assert!(!supported(&editor, cx, "editor.action.goToSelectionAnchor"));
    trigger(&editor, cx, "editor.action.setSelectionAnchor");
    assert!(supported(&editor, cx, "editor.action.goToSelectionAnchor"));
    assert!(supported(&editor, cx, "editor.fold"));
    editor.update(cx, |e, cx| {
        let mut options = e.options().clone();
        options.folding = false;
        e.update_options(options, cx);
    });
    assert!(!supported(&editor, cx, "editor.fold"));
    assert!(!has_key(&editor, cx, "foldingEnabled"));
    assert!(!supported(&editor, cx, "editor.action.wordHighlight.next"));
    // Go to Symbol has the `tags.scm` outline here.
    assert!(supported(&editor, cx, "editor.action.quickOutline"));
}

// ----- core commands with arguments -----------------------------------------

#[gpui::test]
fn handler_commands(cx: &mut TestAppContext) {
    let (editor, cx) = setup_model(|| TextModel::new("abc\ndef"), cx);
    set_cursor(&editor, cx, p(0, 3));
    // `editor.trigger('keyboard', 'type', { text })`, and `default:type`.
    trigger_with(&editor, cx, "type", json!({ "text": "XY" }));
    trigger_with(&editor, cx, "default:type", json!({ "text": "Z" }));
    assert_eq!(text(&editor, cx), "abcXYZ\ndef");
    trigger_with(
        &editor,
        cx,
        "replacePreviousChar",
        json!({ "text": "!", "replaceCharCnt": 2 }),
    );
    assert_eq!(text(&editor, cx), "abcX!\ndef");
    trigger_with(
        &editor,
        cx,
        "compositionType",
        json!({ "text": "é", "replacePrevCharCnt": 1, "replaceNextCharCnt": 0, "positionDelta": 0 }),
    );
    assert_eq!(text(&editor, cx), "abcXé\ndef");
    // Composition events.
    let events = Rc::new(RefCell::new(Vec::new()));
    let seen = events.clone();
    let _subscription = cx.update(|_, cx| {
        cx.subscribe(&editor, move |_, event: &CodeEditorEvent, _| {
            if matches!(
                event,
                CodeEditorEvent::CompositionStarted | CodeEditorEvent::CompositionEnded
            ) {
                seen.borrow_mut().push(format!("{event:?}"));
            }
        })
    });
    trigger(&editor, cx, "compositionStart");
    assert!(has_key(&editor, cx, "isComposing"));
    trigger(&editor, cx, "compositionEnd");
    assert_eq!(*events.borrow(), ["CompositionStarted", "CompositionEnded"]);
    // `paste` with Monaco's arguments: no clipboard, no providers.
    set_cursor(&editor, cx, p(1, 0));
    trigger_with(
        &editor,
        cx,
        "paste",
        json!({ "text": "line\n", "pasteOnNewLine": true }),
    );
    assert_eq!(text(&editor, cx), "abcXé\nline\ndef");
    // The cursor stayed on its line, below the pasted one.
    assert_eq!(head(&editor, cx), p(2, 0));
    // `cut` deletes that line (empty selection) without the clipboard.
    cx.update(|_, cx| cx.write_to_clipboard(gpui::ClipboardItem::new_string("kept".into())));
    trigger(&editor, cx, "cut");
    assert_eq!(text(&editor, cx), "abcXé\nline");
    assert_eq!(
        cx.update(|_, cx| cx.read_from_clipboard().and_then(|c| c.text())),
        Some("kept".into())
    );
}

#[gpui::test]
fn cursor_move_and_selection_commands(cx: &mut TestAppContext) {
    let (editor, cx) = setup(cx);
    set_cursor(&editor, cx, p(0, 0));
    trigger_with(
        &editor,
        cx,
        "cursorMove",
        json!({ "to": "right", "value": 3 }),
    );
    assert_eq!(head(&editor, cx), p(0, 3));
    trigger_with(
        &editor,
        cx,
        "cursorMove",
        json!({ "to": "down", "by": "line", "value": 3, "select": true }),
    );
    assert_eq!(selections(&editor, cx)[0], Selection::new(p(0, 3), p(3, 3)));
    trigger_with(&editor, cx, "cursorMove", json!({ "to": "wrappedLineEnd" }));
    assert_eq!(head(&editor, cx), p(3, 11));
    trigger_with(
        &editor,
        cx,
        "cursorMove",
        json!({ "to": "wrappedLineFirstNonWhitespaceCharacter" }),
    );
    assert_eq!(head(&editor, cx), p(3, 0));
    trigger_with(&editor, cx, "cursorMove", json!({ "to": "nextBlankLine" }));
    assert_eq!(head(&editor, cx), p(6, 0));
    trigger_with(
        &editor,
        cx,
        "cursorMove",
        json!({ "to": "left", "by": "halfLine" }),
    );
    assert_eq!(head(&editor, cx), p(5, 1));
    // Monaco ignores other values.
    trigger_with(&editor, cx, "cursorMove", json!({ "to": "nowhere" }));
    assert_eq!(head(&editor, cx), p(5, 1));
    // `setSelection` with Monaco's `ISelection`.
    trigger_with(
        &editor,
        cx,
        "setSelection",
        json!({ "selection": {
            "selectionStartLineNumber": 2, "selectionStartColumn": 5,
            "positionLineNumber": 2, "positionColumn": 6
        }}),
    );
    assert_eq!(
        selections(&editor, cx),
        vec![Selection::new(p(1, 4), p(1, 5))]
    );
    // `createCursor` adds a cursor, then removes it.
    trigger_with(
        &editor,
        cx,
        "createCursor",
        json!({ "position": { "lineNumber": 1, "column": 1 } }),
    );
    assert_eq!(selections(&editor, cx).len(), 2);
    assert_eq!(announcement(&editor, cx), None);
    trigger_with(
        &editor,
        cx,
        "createCursor",
        json!({ "position": { "lineNumber": 1, "column": 1 } }),
    );
    assert_eq!(selections(&editor, cx).len(), 1);
    // Mouse commands.
    trigger_with(
        &editor,
        cx,
        "_moveTo",
        json!({ "position": { "lineNumber": 4, "column": 4 } }),
    );
    assert_eq!(selections(&editor, cx), vec![Selection::collapsed(p(3, 3))]);
    trigger_with(
        &editor,
        cx,
        "_wordSelect",
        json!({ "position": { "lineNumber": 4, "column": 5 } }),
    );
    assert_eq!(
        selections(&editor, cx),
        vec![Selection::new(p(3, 3), p(3, 7))]
    );
    trigger_with(
        &editor,
        cx,
        "_lineSelect",
        json!({ "position": { "lineNumber": 2, "column": 1 } }),
    );
    assert_eq!(
        selections(&editor, cx),
        vec![Selection::new(p(1, 0), p(2, 0))]
    );
    // The last cursor selects a word, extending its selection if it has
    // one.
    trigger_with(
        &editor,
        cx,
        "_moveTo",
        json!({ "position": { "lineNumber": 3, "column": 1 } }),
    );
    trigger_with(
        &editor,
        cx,
        "lastCursorWordSelect",
        json!({ "position": { "lineNumber": 1, "column": 5 } }),
    );
    assert_eq!(
        selections(&editor, cx),
        vec![Selection::new(p(0, 3), p(0, 6))]
    );
    trigger_with(
        &editor,
        cx,
        "columnSelect",
        json!({
            "position": { "lineNumber": 3, "column": 2 },
            "viewPosition": { "lineNumber": 3, "column": 2 },
            "mouseColumn": 2,
            "doColumnSelect": false
        }),
    );
    assert_eq!(selections(&editor, cx), vec![Selection::collapsed(p(2, 1))]);
    // Accessibility word navigation: word starts, past separators.
    set_cursor(&editor, cx, p(4, 4));
    trigger(&editor, cx, "cursorWordAccessibilityRight");
    assert_eq!(head(&editor, cx), p(4, 8));
}

#[gpui::test]
fn scroll_and_reveal_commands(cx: &mut TestAppContext) {
    let long: String = (0..200).map(|i| format!("line {i}\n")).collect();
    let (editor, cx) = setup_model(move || TextModel::new(&long), cx);
    let top = |cx: &mut VisualTestContext| {
        // Smooth scrolling starts on the next frame.
        cx.executor().advance_clock(Duration::from_secs(1));
        draw(cx);
        editor.read_with(cx, |e, _| e.scroll_top())
    };
    let line_height = editor.read_with(cx, |e, _| e.options().resolved_line_height());
    trigger_with(
        &editor,
        cx,
        "editorScroll",
        json!({ "to": "down", "by": "line", "value": 10 }),
    );
    cx.executor().advance_clock(Duration::from_secs(1));
    draw(cx);
    assert_eq!(top(cx), line_height * 10.);
    trigger_with(
        &editor,
        cx,
        "editorScroll",
        json!({ "to": "up", "value": 4 }),
    );
    draw(cx);
    assert_eq!(top(cx), line_height * 6.);
    // With `revealCursor`, the cursor comes along.
    trigger_with(
        &editor,
        cx,
        "editorScroll",
        json!({ "to": "down", "by": "page", "revealCursor": true }),
    );
    cx.executor().advance_clock(Duration::from_secs(1));
    draw(cx);
    assert!(head(&editor, cx).row > 0);
    trigger_with(
        &editor,
        cx,
        "revealLine",
        json!({ "lineNumber": 150, "at": "top" }),
    );
    cx.executor().advance_clock(Duration::from_secs(1));
    draw(cx);
    // Sticky scroll keeps five context lines above.
    assert_eq!(top(cx), line_height * 145.);
    trigger_with(
        &editor,
        cx,
        "revealLine",
        json!({ "lineNumber": "20", "at": "top" }),
    );
    cx.executor().advance_clock(Duration::from_secs(1));
    draw(cx);
    assert_eq!(top(cx), line_height * 15.);
    // Horizontal scrolling needs something to scroll.
    editor.update(cx, |e, cx| {
        e.model()
            .update(cx, |m, cx| m.set_value(&"x".repeat(2000), cx))
    });
    draw(cx);
    trigger(&editor, cx, "scrollRight");
    assert!(editor.read_with(cx, |e, _| e.scroll_left()) > gpui::px(0.));
    trigger(&editor, cx, "scrollLeft");
    assert_eq!(editor.read_with(cx, |e, _| e.scroll_left()), gpui::px(0.));
}

const FOLDS: &str = "a {\n  b {\n    c {\n      d\n    }\n  }\n}\n";

fn hidden_rows(editor: &Entity<CodeEditor>, cx: &mut VisualTestContext) -> usize {
    editor.read_with(cx, |e, _| {
        (0..8)
            .filter(|row| e.view_lines.is_row_hidden(*row))
            .count()
    })
}

/// `editor.fold` and `editor.unfold` take Monaco's `FoldingArguments`.
#[gpui::test]
fn fold_arguments(cx: &mut TestAppContext) {
    let (editor, cx) = setup_model(|| TextModel::new(FOLDS), cx);
    cx.executor().advance_clock(Duration::from_secs(1));
    draw(cx);
    // Two levels down from line 0: the outer two regions.
    trigger_with(
        &editor,
        cx,
        "editor.fold",
        json!({ "levels": 2, "direction": "down", "selectionLines": [0] }),
    );
    assert_eq!(hidden_rows(&editor, cx), 5);
    trigger_with(
        &editor,
        cx,
        "editor.unfold",
        json!({ "selectionLines": [0] }),
    );
    // The outer region is open, its child still folded.
    assert_eq!(hidden_rows(&editor, cx), 3);
    trigger_with(
        &editor,
        cx,
        "editor.unfold",
        json!({ "levels": 5, "selectionLines": [0] }),
    );
    assert_eq!(hidden_rows(&editor, cx), 0);
    // Up from line 3: the regions around it.
    trigger_with(
        &editor,
        cx,
        "editor.fold",
        json!({ "levels": 1, "direction": "up", "selectionLines": [3] }),
    );
    assert_eq!(hidden_rows(&editor, cx), 1);
    trigger_with(
        &editor,
        cx,
        "editor.fold",
        json!({ "levels": 2, "direction": "up", "selectionLines": [3] }),
    );
    assert_eq!(hidden_rows(&editor, cx), 3);
}

// ----- contribution commands and aliases ------------------------------------

struct Words;

impl CompletionProvider for Words {
    fn provide_completions(
        &self,
        _: &ProviderContext,
        _: PointUtf16,
        _: CompletionTrigger,
        _: &mut App,
    ) -> Task<anyhow::Result<CompletionList>> {
        Task::ready(Ok(CompletionList {
            items: vec![
                CompletionItem::new("alpha", CompletionItemKind::Text),
                CompletionItem::new("beta", CompletionItemKind::Text),
            ],
            incomplete: false,
        }))
    }
}

#[gpui::test]
fn suggest_snippet_and_message_commands(cx: &mut TestAppContext) {
    let (editor, cx) = setup_model(|| TextModel::new(""), cx);
    cx.update(|_, cx| {
        register_completion_provider(cx, LanguageSelector::All, Words);
    });
    trigger(&editor, cx, "editor.action.triggerSuggest");
    assert!(has_key(&editor, cx, "suggestWidgetVisible"));
    assert!(has_key(&editor, cx, "suggestWidgetMultipleSuggestions"));
    assert!(has_key(&editor, cx, "suggestWidgetHasFocusedSuggestion"));
    // Its precondition: not while the widget shows.
    assert!(!supported(&editor, cx, "editor.action.triggerSuggest"));
    trigger(&editor, cx, "acceptSelectedSuggestionOnEnter");
    assert_eq!(text(&editor, cx), "alpha");
    trigger(&editor, cx, "editor.action.triggerSuggest");
    trigger(&editor, cx, "hideSuggestWidget");
    assert!(!editor.read_with(cx, |e, _| e.is_suggest_visible()));

    editor.update(cx, |e, cx| {
        e.insert_snippet("(${1:a}, ${2:b})$0", Default::default(), cx)
    });
    draw(cx);
    assert!(has_key(&editor, cx, "inSnippetMode"));
    assert!(has_key(&editor, cx, "hasNextTabstop"));
    assert!(!has_key(&editor, cx, "hasPrevTabstop"));
    trigger(&editor, cx, "jumpToNextSnippetPlaceholder");
    assert!(has_key(&editor, cx, "hasPrevTabstop"));
    trigger(&editor, cx, "jumpToPrevSnippetPlaceholder");
    assert!(!has_key(&editor, cx, "hasPrevTabstop"));
    trigger(&editor, cx, "leaveSnippet");
    assert!(!has_key(&editor, cx, "inSnippetMode"));

    editor.update(cx, |e, cx| e.show_message("Hello there", p(0, 0), cx));
    draw(cx);
    assert!(has_key(&editor, cx, "messageVisible"));
    // Monaco reads every message to screen readers.
    assert_eq!(announcement(&editor, cx).as_deref(), Some("Hello there"));
    trigger(&editor, cx, "leaveEditorMessage");
    assert!(!has_key(&editor, cx, "messageVisible"));
}

/// Monaco's aliases run their target, preconditions included.
#[gpui::test]
fn aliases(cx: &mut TestAppContext) {
    let (editor, cx) = setup(cx);
    cx.update(|_, cx| {
        register_definition_provider(
            cx,
            LanguageSelector::All,
            Definitions(vec![Location::new(None, p(0, 3)..p(0, 6))]),
        );
    });
    set_cursor(&editor, cx, p(4, 5));
    trigger(&editor, cx, "editor.action.goToDeclaration");
    assert_eq!(head(&editor, cx), p(0, 3));
    set_cursor(&editor, cx, p(4, 5));
    trigger(&editor, cx, "editor.action.previewDeclaration");
    assert!(editor.read_with(cx, |e, _| e.is_peek_visible()));
    assert!(has_key(&editor, cx, "referenceSearchVisible"));
    trigger(&editor, cx, "closeReferenceSearchEditor");
    assert!(!editor.read_with(cx, |e, _| e.is_peek_visible()));
    // `editor.action.smartSelect.grow` is expand.
    set_cursor(&editor, cx, p(1, 5));
    trigger(&editor, cx, "editor.action.smartSelect.grow");
    assert!(!selections(&editor, cx)[0].is_empty());
}

/// `editor.action.findReferences` peeks the references at a position.
#[gpui::test]
fn find_references_command(cx: &mut TestAppContext) {
    struct References;
    impl ReferenceProvider for References {
        fn provide_references(
            &self,
            _: &ProviderContext,
            _: PointUtf16,
            _: ReferenceContext,
            _: &mut App,
        ) -> Task<anyhow::Result<Vec<Location>>> {
            Task::ready(Ok(vec![
                Location::new(None, p(0, 3)..p(0, 6)),
                Location::new(None, p(4, 4)..p(4, 7)),
            ]))
        }
    }
    let (editor, cx) = setup(cx);
    cx.update(|_, cx| {
        register_reference_provider(cx, LanguageSelector::All, References);
    });
    trigger_with(
        &editor,
        cx,
        "editor.action.findReferences",
        json!({ "uri": "file:///elsewhere.rs", "position": { "lineNumber": 5, "column": 6 } }),
    );
    assert!(!editor.read_with(cx, |e, _| e.is_peek_visible()));
    trigger_with(
        &editor,
        cx,
        "editor.action.findReferences",
        json!({ "uri": "file:///src/main.rs", "position": { "lineNumber": 5, "column": 6 } }),
    );
    assert!(editor.read_with(cx, |e, _| e.is_peek_visible()));
}

// ----- context keys ---------------------------------------------------------

#[gpui::test]
fn monaco_context_keys(cx: &mut TestAppContext) {
    let (editor, cx) = setup(cx);
    assert!(has_key(&editor, cx, "editorTextFocus"));
    assert!(has_key(&editor, cx, "editorFocus"));
    assert_eq!(
        key_value(&editor, cx, "editorLangId").as_deref(),
        Some("rust")
    );
    assert!(!has_key(&editor, cx, "editorHasSelection"));
    assert!(has_key(&editor, cx, "foldingEnabled"));
    assert!(has_key(&editor, cx, "editorHasCompletionItemProvider"));
    assert!(has_key(&editor, cx, "editorHasDocumentSymbolProvider"));
    editor.update(cx, |e, cx| {
        e.set_selections(
            &[
                Selection::new(p(1, 4), p(1, 9)),
                Selection::collapsed(p(4, 0)),
            ],
            cx,
        )
    });
    assert!(has_key(&editor, cx, "editorHasSelection"));
    assert!(has_key(&editor, cx, "editorHasMultipleSelections"));
    assert!(!has_key(&editor, cx, "canUndo"));
    cx.simulate_input("x");
    assert!(has_key(&editor, cx, "canUndo"));
    // `editorSelectionHasDiagnostics`: a marker inside the selection.
    editor.update(cx, |e, cx| {
        e.set_selections(&[Selection::new(p(0, 0), p(0, 5))], cx);
        e.model().update(cx, |m, cx| {
            m.set_markers(
                "test",
                vec![Marker {
                    severity: MarkerSeverity::Warning,
                    message: "careful".into(),
                    range: p(0, 3)..p(0, 6),
                    ..Default::default()
                }],
                cx,
            )
        });
    });
    assert!(has_key(&editor, cx, "editorSelectionHasDiagnostics"));
    set_cursor(&editor, cx, p(2, 0));
    assert!(!has_key(&editor, cx, "editorSelectionHasDiagnostics"));
    // Read-only, find and blur.
    read_only(&editor, cx);
    assert!(has_key(&editor, cx, "editorReadonly"));
    trigger(&editor, cx, "actions.find");
    assert!(has_key(&editor, cx, "findWidgetVisible"));
    assert!(has_key(&editor, cx, "findInputFocussed"));
    assert!(has_key(&editor, cx, "editorFocus"));
    assert!(!has_key(&editor, cx, "editorTextFocus"));
    // The editor's own key context has them too, for keybinding rules.
    let rule_ran = Rc::new(RefCell::new(0));
    let ran = rule_ran.clone();
    cx.update(|_, cx| {
        let _command =
            gpui_monaco::add_command(cx, "test.countFind", move |_, _| *ran.borrow_mut() += 1);
        let _rule = gpui_monaco::add_keybinding_rule(
            cx,
            KeybindingRule::new("ctrl-alt-j", "test.countFind")
                .when("CodeEditor && findWidgetVisible && editorLangId == rust"),
        )
        .unwrap();
    });
    trigger(&editor, cx, "closeFindWidget");
    assert!(!has_key(&editor, cx, "findWidgetVisible"));
    cx.simulate_keystrokes("ctrl-alt-j");
    assert_eq!(*rule_ran.borrow(), 0);
    trigger(&editor, cx, "actions.find");
    cx.update(|window, cx| window.focus(&editor.focus_handle(cx), cx));
    draw(cx);
    cx.simulate_keystrokes("ctrl-alt-j");
    assert_eq!(*rule_ran.borrow(), 1);
}

struct Kinds(Option<Vec<CodeActionKind>>);

impl CodeActionProvider for Kinds {
    fn provide_code_actions(
        &self,
        _: &ProviderContext,
        _: Range<PointUtf16>,
        _: &CodeActionContext,
        _: &mut App,
    ) -> Task<anyhow::Result<Vec<CodeAction>>> {
        Task::ready(Ok(Vec::new()))
    }

    fn provided_kinds(&self) -> Option<Vec<CodeActionKind>> {
        self.0.clone()
    }
}

/// Monaco `supportedCodeAction`: Organize Imports, Fix All and Auto Fix
/// need a provider declaring their kind.
#[gpui::test]
fn supported_code_action_kinds(cx: &mut TestAppContext) {
    let (editor, cx) = setup(cx);
    cx.update(|_, cx| {
        register_code_action_provider(cx, LanguageSelector::All, Kinds(None));
    });
    draw(cx);
    assert!(supported(&editor, cx, "editor.action.quickFix"));
    assert!(supported(&editor, cx, "editor.action.refactor"));
    assert!(!supported(&editor, cx, "editor.action.organizeImports"));
    assert!(!supported(&editor, cx, "editor.action.autoFix"));
    assert_eq!(key_value(&editor, cx, "supportedCodeAction"), None);
    cx.update(|_, cx| {
        register_code_action_provider(
            cx,
            LanguageSelector::All,
            Kinds(Some(vec![
                "refactor.extract".into(),
                "source.organizeImports".into(),
            ])),
        );
    });
    draw(cx);
    assert!(supported(&editor, cx, "editor.action.organizeImports"));
    assert!(!supported(&editor, cx, "editor.action.fixAll"));
    assert_eq!(
        key_value(&editor, cx, "supportedCodeAction").as_deref(),
        Some("refactor.extract source.organizeImports")
    );
    assert!(has_key(&editor, cx, "supportedCodeAction_refactor"));
}

/// Monaco `createContextKey`: hosts' keys gate their keybindings.
#[gpui::test]
fn host_context_keys(cx: &mut TestAppContext) {
    let (editor, cx) = setup(cx);
    let key = editor.update(cx, |e, cx| e.create_context_key("myMode", false, cx));
    let ran = Rc::new(RefCell::new(0));
    let count = ran.clone();
    let (id, _command) = editor
        .update(cx, |e, cx| {
            e.add_command(
                "ctrl-alt-m",
                move |_, _, _| *count.borrow_mut() += 1,
                Some("myMode"),
                cx,
            )
        })
        .unwrap();
    assert!(id.starts_with("DYNAMIC_"));
    cx.simulate_keystrokes("ctrl-alt-m");
    assert_eq!(*ran.borrow(), 0);
    cx.update(|_, cx| key.set(true, cx));
    draw(cx);
    assert!(has_key(&editor, cx, "myMode"));
    cx.simulate_keystrokes("ctrl-alt-m");
    assert_eq!(*ran.borrow(), 1);
    cx.update(|_, cx| key.set("deep", cx));
    assert_eq!(key_value(&editor, cx, "myMode").as_deref(), Some("deep"));
    cx.update(|_, cx| key.reset(cx));
    assert!(!has_key(&editor, cx, "myMode"));
    // The command runs by id too.
    trigger(&editor, cx, &id);
    assert_eq!(*ran.borrow(), 2);
}

// ----- per-editor commands and actions --------------------------------------

struct TwoEditors {
    first: Entity<CodeEditor>,
    second: Entity<CodeEditor>,
}

impl Render for TwoEditors {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
            .size_full()
            .flex()
            .flex_col()
            .child(div().h_40().child(self.first.clone()))
            .child(div().h_40().child(self.second.clone()))
    }
}

fn two_editors(
    cx: &mut TestAppContext,
) -> (
    Entity<CodeEditor>,
    Entity<CodeEditor>,
    &mut VisualTestContext,
) {
    cx.update(gpui_monaco::init);
    let (view, cx) = cx.add_window_view(|window, cx| TwoEditors {
        first: cx.new(|cx| CodeEditor::with_text("one", window, cx)),
        second: cx.new(|cx| CodeEditor::with_text("two", window, cx)),
    });
    let (first, second) = view.read_with(cx, |v, _| (v.first.clone(), v.second.clone()));
    (first, second, cx)
}

fn focus(editor: &Entity<CodeEditor>, cx: &mut VisualTestContext) {
    cx.update(|window, cx| window.focus(&editor.focus_handle(cx), cx));
    draw(cx);
}

fn binding_count(cx: &mut VisualTestContext) -> usize {
    cx.update(|_, cx| cx.key_bindings().borrow().bindings().count())
}

/// Monaco `addCommand(keybinding, handler)`: the keybinding works in its
/// editor only, until disposed.
#[gpui::test]
fn add_command_is_scoped_to_its_editor(cx: &mut TestAppContext) {
    let (first, second, cx) = two_editors(cx);
    let ran = Rc::new(RefCell::new(Vec::new()));
    let log = ran.clone();
    let before = binding_count(cx);
    let (_, command) = first
        .update(cx, |e, cx| {
            e.add_command(
                "ctrl-alt-k",
                move |editor, _, cx| log.borrow_mut().push(editor.model().read(cx).text()),
                None,
                cx,
            )
        })
        .unwrap();
    focus(&second, cx);
    cx.simulate_keystrokes("ctrl-alt-k");
    assert!(ran.borrow().is_empty());
    focus(&first, cx);
    cx.simulate_keystrokes("ctrl-alt-k");
    assert_eq!(*ran.borrow(), ["one"]);
    cx.update(|_, cx| command.dispose(cx));
    assert_eq!(binding_count(cx), before);
    cx.simulate_keystrokes("ctrl-alt-k");
    assert_eq!(ran.borrow().len(), 1);
    // Bad keystrokes or conditions fail.
    assert!(
        first
            .update(cx, |e, cx| e.add_command(
                "ctrl-alt-k",
                |_, _, _| {},
                Some("&&"),
                cx
            ))
            .is_err()
    );
}

/// Monaco `addAction` returns a disposable, and its keybindings work only
/// in its editor instead of piling up app-wide.
#[gpui::test]
fn add_action_is_scoped_and_disposable(cx: &mut TestAppContext) {
    let (first, second, cx) = two_editors(cx);
    let before = binding_count(cx);
    let action = |label: &'static str| {
        gpui_monaco::EditorActionDescriptor::new("my.mark", label, |editor, _, cx| {
            editor.model().update(cx, |m, cx| m.set_value("marked", cx));
        })
        .keybinding("ctrl-alt-l")
    };
    let first_action = first.update(cx, |e, cx| e.add_action(action("Mark"), cx));
    let _second_action = second.update(cx, |e, cx| e.add_action(action("Mark"), cx));
    assert_eq!(binding_count(cx), before + 2);
    focus(&first, cx);
    cx.simulate_keystrokes("ctrl-alt-l");
    assert_eq!(text(&first, cx), "marked");
    assert_eq!(text(&second, cx), "two");
    cx.update(|_, cx| first_action.dispose(cx));
    assert_eq!(binding_count(cx), before + 1);
    assert!(!supported(&first, cx, "my.mark"));
    assert!(supported(&second, cx, "my.mark"));
    // `remove_action` unbinds on the next frame.
    second.update(cx, |e, _| assert!(e.remove_action("my.mark")));
    draw(cx);
    assert_eq!(binding_count(cx), before);
    // An editor's bindings go with it.
    let third = cx.update(|window, cx| cx.new(|cx| CodeEditor::with_text("3", window, cx)));
    let _third_action = third.update(cx, |e, cx| e.add_action(action("Mark"), cx));
    assert_eq!(binding_count(cx), before + 1);
    drop(third);
    draw(cx);
    assert_eq!(binding_count(cx), before);
}

// ----- executeCommand ------------------------------------------------------

/// Wraps the selection in brackets and selects what's inside.
struct Bracket {
    selection: Selection,
    inner: Option<gpui_monaco::TrackedSelection>,
}

impl EditorCommand for Bracket {
    fn edit_operations(&mut self, _: &dyn CursorModel, builder: &mut EditOperationBuilder) {
        let range = self.selection.range();
        builder.add_edit_operation(range.start..range.start, "[");
        builder.add_edit_operation(range.end..range.end, "]");
        self.inner = Some(builder.track_selection(self.selection, None));
    }

    fn compute_cursor_state(&self, data: &CursorStateComputerData) -> Selection {
        assert_eq!(data.inverse_edit_ranges().len(), 2);
        data.tracked_selection(self.inner.unwrap())
    }
}

#[gpui::test]
fn execute_commands_with_an_editor_command(cx: &mut TestAppContext) {
    let (editor, cx) = setup_model(|| TextModel::new("ab cd"), cx);
    editor.update(cx, |e, cx| {
        e.set_selections(&[Selection::new(p(0, 0), p(0, 2))], cx);
        let selection = e.selections()[0];
        e.execute_command(
            Some("test"),
            Bracket {
                selection,
                inner: None,
            },
            cx,
        );
    });
    assert_eq!(text(&editor, cx), "[ab] cd");
    assert_eq!(
        selections(&editor, cx),
        vec![Selection::new(p(0, 1), p(0, 3))]
    );
    editor.update(cx, |e, cx| {
        e.set_selections(
            &[
                Selection::new(p(0, 1), p(0, 3)),
                Selection::new(p(0, 5), p(0, 7)),
            ],
            cx,
        );
        let commands: Vec<Option<Box<dyn EditorCommand>>> = e
            .selections()
            .into_iter()
            .map(|selection| {
                Some(Box::new(Bracket {
                    selection,
                    inner: None,
                }) as Box<dyn EditorCommand>)
            })
            .collect();
        e.execute_commands(None, commands, cx);
    });
    assert_eq!(text(&editor, cx), "[[ab]] [cd]");
    editor.update(cx, |e, cx| e.undo(cx));
    assert_eq!(text(&editor, cx), "[ab] cd");
    // Read-only editors refuse it.
    read_only(&editor, cx);
    editor.update(cx, |e, cx| {
        let selection = e.selections()[0];
        e.execute_command(
            None,
            Bracket {
                selection,
                inner: None,
            },
            cx,
        );
    });
    assert_eq!(text(&editor, cx), "[ab] cd");
}

// ----- other arguments --------------------------------------------------------

#[gpui::test]
fn refactor_arguments(cx: &mut TestAppContext) {
    let contexts = Rc::new(RefCell::new(Vec::new()));
    struct Record(Rc<RefCell<Vec<Option<String>>>>);
    impl CodeActionProvider for Record {
        fn provide_code_actions(
            &self,
            _: &ProviderContext,
            _: Range<PointUtf16>,
            context: &CodeActionContext,
            _: &mut App,
        ) -> Task<anyhow::Result<Vec<CodeAction>>> {
            self.0
                .borrow_mut()
                .push(context.only.as_ref().map(|k| k.0.to_string()));
            Task::ready(Ok(Vec::new()))
        }
    }
    let (editor, cx) = setup(cx);
    let record = Record(contexts.clone());
    cx.update(|_, cx| {
        register_code_action_provider(cx, LanguageSelector::All, record);
    });
    // Registering asks for the lightbulb's actions (Monaco
    // `CodeActionModel._update`) first.
    cx.run_until_parked();
    trigger_with(
        &editor,
        cx,
        "editor.action.refactor",
        json!({ "kind": "refactor.extract" }),
    );
    trigger_with(&editor, cx, "editor.action.sourceAction", json!({}));
    let asked = contexts.borrow().clone();
    assert_eq!(
        asked[asked.len() - 2..],
        [
            Some("refactor.extract".to_string()),
            Some("source".to_string())
        ]
    );
    // A kind outside refactorings finds nothing.
    trigger_with(
        &editor,
        cx,
        "editor.action.refactor",
        json!({ "kind": "source" }),
    );
    assert!(editor.read_with(cx, |e, _| e.is_message_visible()));
}

struct Hello;

impl HoverProvider for Hello {
    fn provide_hover(
        &self,
        _: &ProviderContext,
        _: PointUtf16,
        _: &mut App,
    ) -> Task<anyhow::Result<Option<Hover>>> {
        Task::ready(Ok(Some(Hover {
            contents: vec!["hello".into()],
            range: None,
            ..Default::default()
        })))
    }
}

/// `editor.action.showHover`'s `focus` argument.
#[gpui::test]
fn show_hover_focus_argument(cx: &mut TestAppContext) {
    let (editor, cx) = setup(cx);
    cx.update(|_, cx| {
        register_hover_provider(cx, LanguageSelector::All, Hello);
    });
    set_cursor(&editor, cx, p(0, 4));
    let hover_focused = |cx: &mut VisualTestContext| has_key(&editor, cx, "editorHoverFocused");
    // By default a new hover doesn't take focus; showing it again does.
    trigger(&editor, cx, "editor.action.showHover");
    assert!(has_key(&editor, cx, "editorHoverVisible"));
    assert!(!hover_focused(cx));
    // `noAutoFocus` never focuses.
    trigger_with(
        &editor,
        cx,
        "editor.action.showHover",
        json!({ "focus": "noAutoFocus" }),
    );
    draw(cx);
    assert!(!hover_focused(cx));
    trigger(&editor, cx, "editor.action.showHover");
    draw(cx);
    assert!(hover_focused(cx));
    // `autoFocusImmediately` focuses a new one.
    trigger(&editor, cx, "editor.action.hideHover");
    cx.update(|window, cx| window.focus(&editor.focus_handle(cx), cx));
    draw(cx);
    trigger_with(
        &editor,
        cx,
        "editor.action.showHover",
        json!({ "focus": "autoFocusImmediately" }),
    );
    draw(cx);
    assert!(hover_focused(cx));
}

// ----- word highlights and screen reader alerts ------------------------------

fn word_highlights(editor: &Entity<CodeEditor>, cx: &mut VisualTestContext) -> bool {
    editor.read_with(cx, |e, cx| e.context_keys(cx).contains("hasWordHighlights"))
}

/// Monaco's word highlighter needs text focus, and stops when focus leaves
/// the editor.
#[gpui::test]
fn word_highlights_follow_focus(cx: &mut TestAppContext) {
    let (first, second, cx) = two_editors(cx);
    first.update(cx, |e, cx| {
        e.model().update(cx, |m, cx| m.set_value("foo bar foo", cx))
    });
    // Unfocused: nothing.
    set_cursor(&first, cx, p(0, 1));
    cx.executor().advance_clock(Duration::from_millis(500));
    draw(cx);
    assert!(!word_highlights(&first, cx));
    // Focusing the text highlights.
    focus(&first, cx);
    cx.executor().advance_clock(Duration::from_millis(500));
    draw(cx);
    assert!(word_highlights(&first, cx));
    // F7 alerts the position.
    cx.simulate_keystrokes("f7");
    assert_eq!(
        announcement(&first, cx).as_deref(),
        Some("foo bar foo, 2 of 2 for 'foo'")
    );
    // Another editor taking focus keeps them; focus going nowhere drops
    // them.
    focus(&second, cx);
    assert!(word_highlights(&first, cx));
    focus(&first, cx);
    cx.update(|window, cx| window.blur(cx));
    draw(cx);
    assert!(!word_highlights(&first, cx));
}

#[gpui::test]
fn multicursor_alerts(cx: &mut TestAppContext) {
    let (editor, cx) = setup(cx);
    set_cursor(&editor, cx, p(0, 0));
    trigger(&editor, cx, "editor.action.insertCursorBelow");
    assert_eq!(
        announcement(&editor, cx).as_deref(),
        Some("Cursor added: line 2 column 1")
    );
    // Monaco `status`: polite.
    assert!(editor.read_with(cx, |e, _| e.last_announcement_is_status_for_tests()));
    set_cursor(&editor, cx, p(0, 3));
    trigger(&editor, cx, "editor.action.addSelectionToNextFindMatch");
    trigger(&editor, cx, "editor.action.addSelectionToNextFindMatch");
    assert_eq!(
        announcement(&editor, cx).as_deref(),
        Some("Cursor added: line 5 column 8")
    );
}

// ----- go to offset ---------------------------------------------------------

#[gpui::test]
fn go_to_offset_zero_based(cx: &mut TestAppContext) {
    let (editor, cx) = setup_model(|| TextModel::new("abc\ndef"), cx);
    let go = |cx: &mut VisualTestContext, offset: &str| {
        trigger(&editor, cx, "editor.action.gotoOffset");
        cx.simulate_input(offset);
        cx.simulate_keystrokes("enter");
        draw(cx);
        head(&editor, cx)
    };
    assert_eq!(go(cx, "5"), p(1, 0));
    cx.update(|_, cx| gpui_monaco::quick_access::set_goto_offset_zero_based(true, cx));
    assert_eq!(go(cx, "5"), p(1, 1));
    assert_eq!(go(cx, "0"), p(0, 0));
    assert!(cx.update(|_, cx| gpui_monaco::quick_access::goto_offset_zero_based(cx)));
}

// ----- API commands -----------------------------------------------------------

/// What `task` resolves to.
fn resolve<T: 'static>(cx: &mut VisualTestContext, task: Task<T>) -> T {
    let result = Rc::new(RefCell::new(None));
    let out = result.clone();
    cx.update(|_, cx| {
        cx.spawn(async move |_| *out.borrow_mut() = Some(task.await))
            .detach()
    });
    cx.run_until_parked();
    result.borrow_mut().take().expect("resolved")
}

#[gpui::test]
fn api_commands_ask_providers(cx: &mut TestAppContext) {
    struct Links;
    impl LinkProvider for Links {
        fn provide_links(
            &self,
            _: &ProviderContext,
            _: &mut App,
        ) -> Task<anyhow::Result<Vec<DocumentLink>>> {
            Task::ready(Ok(vec![DocumentLink {
                range: p(0, 0)..p(0, 2),
                url: None,
                tooltip: None,
                data: None,
            }]))
        }

        fn resolve_link(
            &self,
            link: &DocumentLink,
            _: &mut App,
        ) -> Option<Task<anyhow::Result<DocumentLink>>> {
            let mut link = link.clone();
            link.url = Some("https://resolved".into());
            Some(Task::ready(Ok(link)))
        }
    }
    let (editor, cx) = setup(cx);
    cx.update(|_, cx| {
        register_link_provider(cx, LanguageSelector::All, Links);
        register_completion_provider(cx, LanguageSelector::All, Words);
    });
    let model = editor.read_with(cx, |e, _| e.model().clone());
    let links = cx.update(|_, cx| gpui_monaco::api_commands::execute_link_provider(&model, 1, cx));
    let links = resolve(cx, links);
    assert_eq!(links[0].url.as_deref(), Some("https://resolved"));
    let items = cx.update(|_, cx| {
        gpui_monaco::api_commands::execute_completion_item_provider(&model, p(0, 0), None, 0, cx)
    });
    let items = resolve(cx, items);
    assert_eq!(items.len(), 2);
    let ranges = cx.update(|_, cx| {
        gpui_monaco::api_commands::execute_selection_range_provider(&model, vec![p(0, 0)], cx)
    });
    assert!(resolve(cx, ranges).is_empty());
    assert!(cx.update(|_, cx| {
        gpui_monaco::api_commands::provide_document_semantic_tokens_legend(&model, cx).is_none()
    }));
}

// ----- #300: view positions, pasteAs, inlineSuggest.trigger, widgets ----------

/// "x", then "aaaa bbbb " | "cccc dddd" wrapped at 10 columns, then "y":
/// view lines 0, 1 and 2, and 3.
const WRAPPED: &str = "x\naaaa bbbb cccc dddd\ny";

fn setup_wrapped(cx: &mut TestAppContext) -> (Entity<CodeEditor>, &mut VisualTestContext) {
    let (editor, cx) = setup_model(|| TextModel::new(WRAPPED), cx);
    editor.update(cx, |e, cx| {
        e.update_options_with(
            |o| {
                o.word_wrap = WordWrap::WordWrapColumn;
                o.word_wrap_column = 10;
            },
            cx,
        )
    });
    draw(cx);
    (editor, cx)
}

fn view_positions(editor: &Entity<CodeEditor>, cx: &mut VisualTestContext) -> Vec<(u32, u32)> {
    editor.read_with(cx, |e, _| e.cursor_view_positions_for_tests())
}

/// Monaco's `{ lineNumber, column }`, from 1.
fn pos(line: u32, column: u32) -> serde_json::Value {
    json!({ "lineNumber": line, "column": column })
}

/// `_moveTo`, `createCursor` and `_lastCursorMoveToSelect` put the cursor
/// at the `viewPosition` when it shows the position: at a wrap point, on
/// the end of the first segment rather than the start of the next.
#[gpui::test]
fn mouse_commands_honour_the_view_position(cx: &mut TestAppContext) {
    let (editor, cx) = setup_wrapped(cx);
    // The wrap point (2, 11): without a view position it shows at the start
    // of the second segment.
    trigger_with(&editor, cx, "_moveTo", json!({ "position": pos(2, 11) }));
    assert_eq!(head(&editor, cx), p(1, 10));
    assert_eq!(view_positions(&editor, cx), [(2, 0)]);
    trigger_with(
        &editor,
        cx,
        "_moveTo",
        json!({ "position": pos(2, 11), "viewPosition": pos(2, 11) }),
    );
    assert_eq!(head(&editor, cx), p(1, 10));
    assert_eq!(view_positions(&editor, cx), [(1, 10)]);
    // A view position elsewhere is ignored (Monaco `validateViewPosition`).
    trigger_with(
        &editor,
        cx,
        "_moveTo",
        json!({ "position": pos(2, 11), "viewPosition": pos(4, 1) }),
    );
    assert_eq!(view_positions(&editor, cx), [(2, 0)]);
    // `_moveToSelect` keeps the anchor and moves the head by view.
    set_cursor(&editor, cx, p(0, 0));
    trigger_with(
        &editor,
        cx,
        "_moveToSelect",
        json!({ "position": pos(2, 11), "viewPosition": pos(2, 11) }),
    );
    assert_eq!(selections(&editor, cx), [Selection::new(p(0, 0), p(1, 10))]);
    assert_eq!(view_positions(&editor, cx), [(1, 10)]);

    // `createCursor` at the view position, then toggled off there.
    set_cursor(&editor, cx, p(0, 0));
    let create = json!({ "position": pos(2, 11), "viewPosition": pos(2, 11) });
    trigger_with(&editor, cx, "createCursor", create.clone());
    assert_eq!(view_positions(&editor, cx), [(0, 0), (1, 10)]);
    trigger_with(&editor, cx, "createCursor", create);
    assert_eq!(view_positions(&editor, cx), [(0, 0)]);

    // `_lastCursorMoveToSelect` moves the last added cursor's head.
    trigger_with(
        &editor,
        cx,
        "createCursor",
        json!({ "position": pos(3, 1) }),
    );
    trigger_with(
        &editor,
        cx,
        "_lastCursorMoveToSelect",
        json!({ "position": pos(2, 11), "viewPosition": pos(2, 11) }),
    );
    assert_eq!(
        selections(&editor, cx),
        [
            Selection::collapsed(p(0, 0)),
            Selection::new(p(2, 0), p(1, 10))
        ]
    );
    assert_eq!(view_positions(&editor, cx), [(0, 0), (1, 10)]);
}

/// Monaco `CursorMoveCommands.line`: extending a line selection goes by view
/// lines, so a wrapped line's segments are selected one at a time.
#[gpui::test]
fn line_selection_extends_by_view_lines(cx: &mut TestAppContext) {
    let (editor, cx) = setup_wrapped(cx);
    trigger_with(&editor, cx, "_lineSelect", json!({ "position": pos(1, 1) }));
    assert_eq!(selections(&editor, cx), [Selection::new(p(0, 0), p(1, 0))]);
    // Onto the first segment: to the start of the second.
    trigger_with(
        &editor,
        cx,
        "_lineSelectDrag",
        json!({ "position": pos(2, 3), "viewPosition": pos(2, 3) }),
    );
    assert_eq!(selections(&editor, cx), [Selection::new(p(0, 0), p(1, 10))]);
    // Onto the second: to the next line.
    trigger_with(
        &editor,
        cx,
        "_lineSelectDrag",
        json!({ "position": pos(2, 13), "viewPosition": pos(3, 3) }),
    );
    assert_eq!(selections(&editor, cx), [Selection::new(p(0, 0), p(2, 0))]);
    // Upwards from the last line: to the start of the segment.
    trigger_with(&editor, cx, "_lineSelect", json!({ "position": pos(3, 1) }));
    trigger_with(
        &editor,
        cx,
        "_lineSelectDrag",
        json!({ "position": pos(2, 13), "viewPosition": pos(3, 3) }),
    );
    assert_eq!(head(&editor, cx), p(1, 10));
    assert_eq!(view_positions(&editor, cx), [(2, 0)]);
    // The last cursor's line selection does the same.
    trigger_with(&editor, cx, "_moveTo", json!({ "position": pos(1, 1) }));
    trigger_with(
        &editor,
        cx,
        "lastCursorLineSelect",
        json!({ "position": pos(1, 1) }),
    );
    trigger_with(
        &editor,
        cx,
        "lastCursorLineSelectDrag",
        json!({ "position": pos(2, 3), "viewPosition": pos(2, 3) }),
    );
    assert_eq!(selections(&editor, cx), [Selection::new(p(0, 0), p(1, 10))]);
}

/// Monaco's mouse commands go through view positions: a position inside a
/// fold lands where the fold shows.
#[gpui::test]
fn mouse_commands_respect_folds(cx: &mut TestAppContext) {
    let (editor, cx) = setup_model(|| TextModel::new(FOLDS), cx);
    cx.executor().advance_clock(Duration::from_secs(1));
    draw(cx);
    trigger_with(
        &editor,
        cx,
        "editor.fold",
        json!({ "levels": 1, "selectionLines": [1] }),
    );
    assert_eq!(hidden_rows(&editor, cx), 3);
    // Row 3 is hidden in the fold that starts on row 1 ("  b {").
    trigger_with(&editor, cx, "_moveTo", json!({ "position": pos(4, 3) }));
    assert_eq!(selections(&editor, cx), [Selection::collapsed(p(1, 5))]);
    // A line selection extended past the fold goes to the line after it.
    trigger_with(&editor, cx, "_lineSelect", json!({ "position": pos(1, 1) }));
    trigger_with(
        &editor,
        cx,
        "_lineSelectDrag",
        json!({ "position": pos(2, 1) }),
    );
    assert_eq!(selections(&editor, cx), [Selection::new(p(0, 0), p(5, 0))]);
}

/// Monaco `columnSelect` selects on view lines: each segment of a wrapped
/// line is a line of the box.
#[gpui::test]
fn column_select_goes_by_view_lines(cx: &mut TestAppContext) {
    let (editor, cx) = setup_wrapped(cx);
    trigger_with(
        &editor,
        cx,
        "columnSelect",
        json!({
            "position": pos(2, 2),
            "viewPosition": pos(2, 2),
            "mouseColumn": 2,
            "doColumnSelect": false
        }),
    );
    assert_eq!(selections(&editor, cx), [Selection::collapsed(p(1, 1))]);
    trigger_with(
        &editor,
        cx,
        "columnSelect",
        json!({
            "position": pos(2, 14),
            "viewPosition": pos(3, 4),
            "mouseColumn": 4,
            "doColumnSelect": true
        }),
    );
    assert_eq!(
        selections(&editor, cx),
        [
            Selection::new(p(1, 1), p(1, 3)),
            Selection::new(p(1, 11), p(1, 13)),
        ]
    );
    assert_eq!(view_positions(&editor, cx), [(1, 3), (2, 3)]);
    // On to the next line.
    trigger_with(
        &editor,
        cx,
        "columnSelect",
        json!({
            "position": pos(3, 2),
            "viewPosition": pos(4, 2),
            "mouseColumn": 4,
            "doColumnSelect": true
        }),
    );
    assert_eq!(selections(&editor, cx).len(), 3);
}

/// Pastes the clipboard as an edit of `kind` with `text`.
struct PasteKind {
    kind: &'static str,
    text: &'static str,
}

impl DocumentPasteEditProvider for PasteKind {
    fn paste_mime_types(&self) -> Vec<gpui::SharedString> {
        vec!["text/plain".into()]
    }

    fn provided_paste_edit_kinds(&self) -> Vec<gpui::SharedString> {
        vec![self.kind.into()]
    }

    fn provide_document_paste_edits(
        &self,
        _: &ProviderContext,
        _: &[Range<PointUtf16>],
        _: &DataTransfer,
        _: DocumentPasteTriggerKind,
        _: &mut App,
    ) -> Task<anyhow::Result<Vec<DocumentDropOrPasteEdit>>> {
        Task::ready(Ok(vec![DocumentDropOrPasteEdit {
            title: self.text.into(),
            kind: Some(self.kind.into()),
            insert_text: EditInsertText::Text(self.text.into()),
            ..Default::default()
        }]))
    }
}

/// `editor.action.pasteAs` with `preferences`: the first edit matching any
/// of them, without asking; `kind` wins over them.
#[gpui::test]
fn paste_as_preferences(cx: &mut TestAppContext) {
    let (editor, cx) = setup_model(|| TextModel::new(""), cx);
    cx.update(|_, cx| {
        register_document_paste_edit_provider(
            cx,
            LanguageSelector::All,
            PasteKind {
                kind: "text.shout",
                text: "HI",
            },
        );
        register_document_paste_edit_provider(
            cx,
            LanguageSelector::All,
            PasteKind {
                kind: "text.tag",
                text: "<hi>",
            },
        );
    });
    cx.write_to_clipboard(gpui::ClipboardItem::new_string("hi".into()));
    let paste_as = |args: serde_json::Value, cx: &mut VisualTestContext| {
        editor.update(cx, |e, cx| {
            e.model().update(cx, |m, cx| m.set_value("", cx))
        });
        draw(cx);
        trigger_with(&editor, cx, "editor.action.pasteAs", args);
        draw(cx);
        let rows = editor.read_with(cx, |e, cx| e.quick_access_rows_for_tests(cx));
        assert!(rows.is_empty(), "asked: {rows:?}");
        text(&editor, cx)
    };
    // A later preference applies when an earlier one matches nothing.
    assert_eq!(
        paste_as(json!({ "preferences": ["text.missing", "text.tag"] }), cx),
        "<hi>"
    );
    // A parent kind matches its children: several edits, the first applies.
    assert!(!paste_as(json!({ "preferences": ["text"] }), cx).is_empty());
    // `kind` first.
    assert_eq!(
        paste_as(
            json!({ "kind": "text.tag", "preferences": ["text.shout"] }),
            cx
        ),
        "<hi>"
    );
    // Nothing matches: Monaco's message, with every preference.
    assert_eq!(
        paste_as(json!({ "preferences": ["text.a", "text.b"] }), cx),
        ""
    );
    assert_eq!(
        editor.read_with(cx, |e, _| e.message_text()).as_deref(),
        Some("No paste edits for 'text.a, text.b' found")
    );
    paste_as(json!({ "preferences": [] }), cx);
    assert_eq!(
        editor.read_with(cx, |e, _| e.message_text()).as_deref(),
        Some("No paste edits for 'empty' found")
    );
}

/// An inline completions provider with an id, counting its requests.
struct Identified {
    id: Option<&'static str>,
    text: &'static str,
    requests: Rc<RefCell<usize>>,
}

impl InlineCompletionsProvider for Identified {
    fn provide_inline_completions(
        &self,
        _: &ProviderContext,
        position: PointUtf16,
        _: InlineCompletionContext,
        _: &mut App,
    ) -> Task<anyhow::Result<InlineCompletionList>> {
        *self.requests.borrow_mut() += 1;
        let items = if self.text.is_empty() {
            Vec::new()
        } else {
            vec![InlineCompletion {
                insert_text: self.text.into(),
                range: Some(position..position),
                ..Default::default()
            }]
        };
        Task::ready(Ok(InlineCompletionList {
            items,
            ..Default::default()
        }))
    }

    fn provider_id(&self) -> Option<gpui::SharedString> {
        self.id.map(Into::into)
    }
}

/// Logs the editor writes, for `showNoResultNotification`.
static LOGS: std::sync::Mutex<Vec<String>> = std::sync::Mutex::new(Vec::new());

struct CaptureLogs;

impl log::Log for CaptureLogs {
    fn enabled(&self, _: &log::Metadata) -> bool {
        true
    }

    fn log(&self, record: &log::Record) {
        LOGS.lock().unwrap().push(record.args().to_string());
    }

    fn flush(&self) {}
}

fn logged(message: &str) -> usize {
    LOGS.lock()
        .unwrap()
        .iter()
        .filter(|line| *line == message)
        .count()
}

/// `editor.action.inlineSuggest.trigger`'s `providerId` asks only that
/// provider (by its id or its extension's `ext:*`), every provider when
/// none matches; `showNoResultNotification` logs when nothing shows, as
/// standalone Monaco's notification service does.
#[gpui::test]
fn inline_suggest_trigger_arguments(cx: &mut TestAppContext) {
    static CAPTURE: CaptureLogs = CaptureLogs;
    if log::set_logger(&CAPTURE).is_ok() {
        log::set_max_level(log::LevelFilter::Info);
    }
    let (editor, cx) = setup_model(|| TextModel::new(""), cx);
    let (a, b) = (Rc::new(RefCell::new(0)), Rc::new(RefCell::new(0)));
    let (provider_a, provider_b) = (
        Identified {
            id: Some("ext.a:one"),
            text: "alpha",
            requests: a.clone(),
        },
        Identified {
            id: Some("ext.b"),
            text: "",
            requests: b.clone(),
        },
    );
    cx.update(|_, cx| {
        register_inline_completions_provider(cx, LanguageSelector::All, provider_a);
        register_inline_completions_provider(cx, LanguageSelector::All, provider_b);
    });
    // New text each time, so no request is the same as the last.
    let runs = std::cell::Cell::new(0);
    let run = |args: serde_json::Value, cx: &mut VisualTestContext| {
        runs.set(runs.get() + 1);
        let text = " ".repeat(runs.get());
        editor.update(cx, |e, cx| {
            e.model().update(cx, |m, cx| m.set_value(&text, cx))
        });
        draw(cx);
        let before = (*a.borrow(), *b.borrow());
        trigger_with(&editor, cx, "editor.action.inlineSuggest.trigger", args);
        cx.executor().advance_clock(Duration::from_millis(100));
        draw(cx);
        (*a.borrow() - before.0, *b.borrow() - before.1)
    };
    // Only the empty provider: nothing shows, and it says so.
    let message = "No inline suggestion is available.";
    let before = logged(message);
    assert_eq!(
        run(
            json!({ "providerId": "ext.b", "showNoResultNotification": true }),
            cx
        ),
        (0, 1)
    );
    assert!(!editor.read_with(cx, |e, cx| e.inline_suggestion_visible(cx)));
    assert_eq!(logged(message), before + 1);
    // Not without the argument, or when something shows.
    run(json!({ "providerId": "ext.b" }), cx);
    assert_eq!(
        run(
            json!({ "providerId": "ext.a:one", "showNoResultNotification": true }),
            cx
        ),
        (1, 0)
    );
    assert!(editor.read_with(cx, |e, cx| e.inline_suggestion_visible(cx)));
    assert_eq!(logged(message), before + 1);
    // An extension's providers, then no match: every provider.
    assert_eq!(run(json!({ "providerId": "ext.b:*" }), cx), (0, 1));
    assert_eq!(run(json!({ "providerId": "ext.c" }), cx), (1, 1));
    assert_eq!(run(json!({}), cx), (1, 1));
}

/// The default keybindings in a widget's key context get the editor's
/// preconditions there (Monaco folds an action's precondition into its
/// keybinding wherever it's bound).
#[test]
fn widget_keybindings_get_their_preconditions() {
    let bindings = gpui_monaco::actions::default_key_bindings();
    let predicate = |keystrokes: &str, action: &str| {
        bindings
            .iter()
            .find(|b| {
                b.action().name() == action
                    && b.keystrokes()
                        .iter()
                        .map(|k| k.inner().unparse())
                        .collect::<Vec<_>>()
                        .join(" ")
                        == keystrokes
            })
            .and_then(|b| b.predicate())
            .unwrap_or_else(|| panic!("{keystrokes} {action}"))
    };
    let contexts = |contexts: &[&str]| -> Vec<gpui::KeyContext> {
        contexts
            .iter()
            .map(|c| gpui::KeyContext::parse(c).unwrap())
            .collect()
    };
    // The hover's scroll keys need `editorHoverFocused`.
    let up = predicate("up", "code_editor::ScrollUpHover");
    assert!(!up.eval(&contexts(&["CodeEditor", "HoverWidget"])));
    assert!(up.eval(&contexts(&["CodeEditor", "HoverWidget editorHoverFocused"])));
    // The rename input's need `renameInputVisible`, in the box around the
    // input.
    let enter = predicate("enter", "code_editor::AcceptRenameInput");
    assert!(!enter.eval(&contexts(&["CodeEditor", "RenameInput", "CodeEditor"])));
    assert!(enter.eval(&contexts(&[
        "CodeEditor",
        "RenameInput renameInputVisible",
        "CodeEditor"
    ])));
    // The standalone color picker's.
    let escape = predicate("escape", "code_editor::HideColorPicker");
    assert!(!escape.eval(&contexts(&["CodeEditor", "StandaloneColorPicker"])));
    assert!(escape.eval(&contexts(&[
        "CodeEditor",
        "StandaloneColorPicker standaloneColorPickerVisible"
    ])));
}

/// A widget's key context carries its editor's context keys, so a rule in
/// the widget's context can test them, as Monaco's widgets see their
/// editor's keys.
#[gpui::test]
fn widget_key_contexts_see_the_editor_keys(cx: &mut TestAppContext) {
    let (editor, cx) = setup(cx);
    cx.update(|_, cx| {
        register_hover_provider(cx, LanguageSelector::All, Hello);
    });
    let ran = Rc::new(RefCell::new(0));
    let count = ran.clone();
    let _command = cx.update(|_, cx| {
        gpui_monaco::add_command(cx, "test.inHover", move |_, _| *count.borrow_mut() += 1)
    });
    let _rule = cx
        .update(|_, cx| {
            gpui_monaco::add_keybinding_rule(
                cx,
                KeybindingRule::new("ctrl-alt-h", "test.inHover")
                    .when("HoverWidget && editorLangId == rust && myMode"),
            )
        })
        .unwrap();
    let key = editor.update(cx, |e, cx| e.create_context_key("myMode", false, cx));
    set_cursor(&editor, cx, p(0, 4));
    // Show the hover, then focus it.
    trigger(&editor, cx, "editor.action.showHover");
    trigger(&editor, cx, "editor.action.showHover");
    assert!(has_key(&editor, cx, "editorHoverFocused"));
    cx.simulate_keystrokes("ctrl-alt-h");
    assert_eq!(*ran.borrow(), 0);
    cx.update(|_, cx| key.set(true, cx));
    draw(cx);
    cx.simulate_keystrokes("ctrl-alt-h");
    assert_eq!(*ran.borrow(), 1);
    // The hover's own keys still work there.
    cx.simulate_keystrokes("escape");
    draw(cx);
    assert!(!has_key(&editor, cx, "editorHoverVisible"));
}
