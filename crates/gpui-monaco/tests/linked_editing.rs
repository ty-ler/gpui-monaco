//! Linked editing (Monaco `contrib/linkedEditing`): HTML tags from the
//! syntax tree and mock providers, and a port of Monaco's
//! `linkedEditing.test.ts`.

use std::cell::Cell;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use gpui::{App, AppContext as _, Entity, Focusable, Task, TestAppContext, VisualTestContext};
use gpui_monaco::cursor::Selection;
use gpui_monaco::providers::*;
use gpui_monaco::syntax::{Language, LanguageConfiguration, Pattern};
use gpui_monaco::text::PointUtf16;
use gpui_monaco::{CodeEditor, CodeEditorEvent, TextModel};
use serde_json::json;

fn setup<'a>(
    text: &str,
    language: &str,
    cx: &'a mut TestAppContext,
) -> (Entity<CodeEditor>, &'a mut VisualTestContext) {
    cx.update(|cx| {
        gpui_monaco::init(cx);
        gpui_monaco::set_language_registry(cx, gpui_monaco_languages::registry());
    });
    let text = text.to_string();
    let language = language.to_string();
    let (editor, cx) = cx.add_window_view(|window, cx| {
        let model = cx.new(|_| TextModel::new(&text).with_language(language));
        let mut editor = CodeEditor::new(model, window, cx);
        let mut options = editor.options().clone();
        options.linked_editing = true;
        options.quick_suggestions = gpui_monaco::options::QuickSuggestions::OFF;
        options.word_based_suggestions = gpui_monaco::options::WordBasedSuggestions::Off;
        editor.update_options(options, cx);
        editor
    });
    cx.update(|window, cx| window.focus(&editor.focus_handle(cx), cx));
    cx.run_until_parked();
    (editor, cx)
}

/// A rename provider: Monaco's Start Linked Editing needs one.
struct NoRename;

impl RenameProvider for NoRename {
    fn provide_rename_edits(
        &self,
        _: &ProviderContext,
        _: PointUtf16,
        _: &str,
        _: &mut App,
    ) -> Task<anyhow::Result<Option<WorkspaceEdit>>> {
        Task::ready(Ok(None))
    }
}

fn settle(cx: &mut VisualTestContext) {
    cx.executor().advance_clock(Duration::from_millis(300));
    cx.run_until_parked();
}

fn move_to(editor: &Entity<CodeEditor>, cx: &mut VisualTestContext, row: u32, column: u32) {
    editor.update(cx, |e, cx| {
        e.set_selections(&[Selection::collapsed(PointUtf16::new(row, column))], cx)
    });
    settle(cx);
}

fn text(editor: &Entity<CodeEditor>, cx: &mut VisualTestContext) -> String {
    editor.read_with(cx, |e, cx| e.model().read(cx).text())
}

fn ranges(editor: &Entity<CodeEditor>, cx: &mut VisualTestContext) -> Vec<(u32, u32)> {
    editor.read_with(cx, |e, cx| {
        e.linked_editing_ranges_for_tests(cx)
            .into_iter()
            .map(|r| (r.start.column, r.end.column))
            .collect()
    })
}

#[gpui::test]
fn renaming_an_html_start_tag_renames_its_end_tag(cx: &mut TestAppContext) {
    let (editor, cx) = setup("<div>hi</div>", "html", cx);
    move_to(&editor, cx, 0, 2);
    // The tag at the cursor first.
    assert_eq!(ranges(&editor, cx), vec![(1, 4), (9, 12)]);
    cx.simulate_input("x");
    cx.run_until_parked();
    assert_eq!(text(&editor, cx), "<dxiv>hi</dxiv>");
    // At the end of the name too.
    cx.simulate_keystrokes("right right");
    cx.simulate_input("s");
    cx.run_until_parked();
    assert_eq!(text(&editor, cx), "<dxivs>hi</dxivs>");
    // Each edit and its mirror are one undo step.
    editor.update(cx, |e, cx| e.undo(cx));
    assert_eq!(text(&editor, cx), "<dxiv>hi</dxiv>");
    editor.update(cx, |e, cx| e.undo(cx));
    assert_eq!(text(&editor, cx), "<div>hi</div>");
}

#[gpui::test]
fn the_end_tag_renames_the_start_tag(cx: &mut TestAppContext) {
    let (editor, cx) = setup("<p>a</p>", "html", cx);
    move_to(&editor, cx, 0, 8 - 1);
    cx.simulate_keystrokes("backspace");
    cx.run_until_parked();
    assert_eq!(text(&editor, cx), "<>a</>");
    cx.simulate_input("em");
    cx.run_until_parked();
    assert_eq!(text(&editor, cx), "<em>a</em>");
    // The cursor stays in the end tag.
    assert_eq!(
        editor.read_with(cx, |e, _| e.selections()[0].head),
        PointUtf16::new(0, 9)
    );
}

#[gpui::test]
fn off_by_default_and_escape_ends_it(cx: &mut TestAppContext) {
    let (editor, cx) = setup("<b>x</b>", "html", cx);
    editor.update(cx, |e, cx| {
        let mut options = e.options().clone();
        options.linked_editing = false;
        e.update_options(options, cx);
    });
    move_to(&editor, cx, 0, 2);
    assert!(ranges(&editor, cx).is_empty());
    // Without a rename provider, Monaco's precondition leaves the key alone.
    cx.simulate_keystrokes("cmd-shift-f2");
    cx.run_until_parked();
    assert!(ranges(&editor, cx).is_empty());
    // With one, the command works anyway.
    cx.update(|_, cx| register_rename_provider(cx, LanguageSelector::All, NoRename));
    cx.simulate_keystrokes("cmd-shift-f2");
    cx.run_until_parked();
    assert_eq!(ranges(&editor, cx).len(), 2);
    cx.simulate_keystrokes("escape");
    assert!(ranges(&editor, cx).is_empty());
}

#[gpui::test]
fn non_word_characters_end_it(cx: &mut TestAppContext) {
    let (editor, cx) = setup("<div></div>", "html", cx);
    move_to(&editor, cx, 0, 4);
    cx.simulate_input(" ");
    cx.run_until_parked();
    // A space starts attributes: the end tag is left alone.
    assert_eq!(text(&editor, cx), "<div ></div>");
}

/// Links every `$x` to the others.
struct Dollars;
impl LinkedEditingRangeProvider for Dollars {
    fn provide_linked_editing_ranges(
        &self,
        context: &ProviderContext,
        _: PointUtf16,
        _: &mut App,
    ) -> Task<anyhow::Result<Option<LinkedEditingRanges>>> {
        let line = context.snapshot.line(0);
        let ranges = line
            .match_indices("$x")
            .map(|(ix, _)| PointUtf16::new(0, ix as u32 + 1)..PointUtf16::new(0, ix as u32 + 2))
            .collect();
        Task::ready(Ok(Some(LinkedEditingRanges {
            ranges,
            word_pattern: None,
        })))
    }
}

#[gpui::test]
fn providers_supply_ranges(cx: &mut TestAppContext) {
    cx.update(|cx| {
        gpui_monaco::init(cx);
        register_linked_editing_range_provider(cx, LanguageSelector::All, Dollars);
    });
    let (editor, cx) = setup("$x = $x + $x", "plaintext", cx);
    move_to(&editor, cx, 0, 6);
    assert_eq!(ranges(&editor, cx), vec![(6, 7), (1, 2), (11, 12)]);
    cx.simulate_input("y");
    cx.run_until_parked();
    assert_eq!(text(&editor, cx), "$yx = $yx + $yx");
}

/// Monaco's word pattern for a language without one is
/// `DEFAULT_WORD_REGEXP` (`getWordDefinition`), so a space ends linked
/// editing even when neither the language nor the provider has a pattern
/// (audit 4 A.2).
#[gpui::test]
fn the_default_word_pattern_ends_it(cx: &mut TestAppContext) {
    cx.update(|cx| {
        gpui_monaco::init(cx);
        gpui_monaco::register_language(cx, Language::new("nowords", "nowords"));
        register_linked_editing_range_provider(
            cx,
            LanguageSelector::All,
            WordMatches {
                word_pattern: None,
                calls: Arc::default(),
            },
        );
    });
    let (editor, cx) = setup("<foo></foo>", "nowords", cx);
    move_to(&editor, cx, 0, 4);
    assert_eq!(ranges(&editor, cx), vec![(1, 4), (7, 10)]);
    cx.simulate_input(" ");
    cx.run_until_parked();
    cx.simulate_input("a");
    settle(cx);
    assert_eq!(text(&editor, cx), "<foo a></foo>");
}

/// Monaco `reinitialize`: turning the option off removes the ranges at
/// once, and turning it on or registering a provider asks for them
/// without waiting for the cursor to move (audit 4 A.3).
#[gpui::test]
fn option_and_provider_changes_reinitialize(cx: &mut TestAppContext) {
    let (editor, cx) = setup("<div></div>", "html", cx);
    move_to(&editor, cx, 0, 2);
    assert_eq!(ranges(&editor, cx), vec![(1, 4), (7, 10)]);
    let set_enabled = |editor: &Entity<CodeEditor>, cx: &mut VisualTestContext, on: bool| {
        editor.update(cx, |e, cx| {
            let mut options = e.options().clone();
            options.linked_editing = on;
            e.update_options(options, cx);
        });
        cx.run_until_parked();
    };
    set_enabled(&editor, cx, false);
    assert!(ranges(&editor, cx).is_empty());
    set_enabled(&editor, cx, true);
    assert_eq!(ranges(&editor, cx), vec![(1, 4), (7, 10)]);

    // A provider registered while the caret waits in its range.
    let (editor, cx) = setup("$x = $x", "plaintext", cx);
    move_to(&editor, cx, 0, 1);
    assert!(ranges(&editor, cx).is_empty());
    cx.update(|_, cx| register_linked_editing_range_provider(cx, LanguageSelector::All, Dollars));
    cx.run_until_parked();
    assert_eq!(ranges(&editor, cx), vec![(1, 2), (6, 7)]);
}

// ----- Port of Monaco's `linkedEditing.test.ts` --------------------------------
//
// `contrib/linkedEditing/test/browser/linkedEditing.test.ts` (MIT): a
// language whose word pattern is `[a-zA-Z]+`, and a provider linking every
// whole-word occurrence of the word at the cursor, answering with the
// case's word pattern. Monaco leaves "Selection insert - across two
// boundary", "Delete - left all" and "Delete - left all then undo"
// commented out.

const LANGUAGE_ID: &str = "linkedEditingTestLangage";

/// Monaco `USUAL_WORD_SEPARATORS`.
const USUAL_WORD_SEPARATORS: &str = "`~!@#$%^&*()-=+[{]}\\|;:'\",.<>/?";

/// Monaco's mock provider: `getWordAtPosition`, then `findMatches(word,
/// wholeWord, USUAL_WORD_SEPARATORS)`.
struct WordMatches {
    word_pattern: Option<Pattern>,
    /// How many times it was asked.
    calls: Arc<AtomicUsize>,
}

impl LinkedEditingRangeProvider for WordMatches {
    fn provide_linked_editing_ranges(
        &self,
        context: &ProviderContext,
        position: PointUtf16,
        _: &mut App,
    ) -> Task<anyhow::Result<Option<LinkedEditingRanges>>> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        let text = context.snapshot.rope().to_string();
        let lines: Vec<Vec<char>> = text.split('\n').map(|l| l.chars().collect()).collect();
        let is_word = |c: char| c.is_ascii_alphabetic();
        let line = &lines[position.row as usize];
        let column = (position.column as usize).min(line.len());
        let start = column
            - line[..column]
                .iter()
                .rev()
                .take_while(|c| is_word(**c))
                .count();
        let end = column + line[column..].iter().take_while(|c| is_word(**c)).count();
        let mut ranges = Vec::new();
        if start < end {
            let word = &line[start..end];
            let is_separator = |c: Option<&char>| {
                c.is_none_or(|c| c.is_whitespace() || USUAL_WORD_SEPARATORS.contains(*c))
            };
            for (row, line) in lines.iter().enumerate() {
                for ix in 0..(line.len() + 1).saturating_sub(word.len()) {
                    if &line[ix..ix + word.len()] == word
                        && is_separator(ix.checked_sub(1).and_then(|i| line.get(i)))
                        && is_separator(line.get(ix + word.len()))
                    {
                        ranges.push(
                            PointUtf16::new(row as u32, ix as u32)
                                ..PointUtf16::new(row as u32, (ix + word.len()) as u32),
                        );
                    }
                }
            }
        }
        Task::ready(Ok(Some(LinkedEditingRanges {
            ranges,
            word_pattern: self.word_pattern.clone(),
        })))
    }
}

/// Monaco `createMockEditor` with linked editing on and the mock provider
/// answering `word_pattern` (`responseWordPattern`).
fn mock_editor<'a>(
    text: &str,
    word_pattern: Option<&str>,
    cx: &'a mut TestAppContext,
) -> (Entity<CodeEditor>, &'a mut VisualTestContext) {
    let (editor, cx, _) = mock_editor_counting(text, word_pattern, cx);
    (editor, cx)
}

/// [`mock_editor`], with the number of times the provider was asked.
fn mock_editor_counting<'a>(
    text: &str,
    word_pattern: Option<&str>,
    cx: &'a mut TestAppContext,
) -> (
    Entity<CodeEditor>,
    &'a mut VisualTestContext,
    Arc<AtomicUsize>,
) {
    let word_pattern = word_pattern.map(|source| Pattern::new(source, ""));
    let calls = Arc::<AtomicUsize>::default();
    cx.update(|cx| {
        gpui_monaco::init(cx);
        gpui_monaco::register_language(
            cx,
            Language::new(LANGUAGE_ID, LANGUAGE_ID).with_config(LanguageConfiguration {
                word_pattern: Some(Pattern::new("[a-zA-Z]+", "")),
                ..Default::default()
            }),
        );
        register_linked_editing_range_provider(
            cx,
            LanguageSelector::All,
            WordMatches {
                word_pattern,
                calls: calls.clone(),
            },
        );
    });
    let text = text.to_string();
    let (editor, cx) = cx.add_window_view(|window, cx| {
        let model = cx.new(|_| {
            TextModel::new(&text)
                .with_uri("test:somefile.ttt")
                .with_language(LANGUAGE_ID)
        });
        let mut editor = CodeEditor::new(model, window, cx);
        let mut options = editor.options().clone();
        options.linked_editing = true;
        options.quick_suggestions = gpui_monaco::options::QuickSuggestions::OFF;
        options.word_based_suggestions = gpui_monaco::options::WordBasedSuggestions::Off;
        editor.update_options(options, cx);
        editor
    });
    cx.update(|window, cx| window.focus(&editor.focus_handle(cx), cx));
    settle(cx);
    (editor, cx, calls)
}

/// Monaco 1-based (line, column).
fn mp(line: u32, column: u32) -> PointUtf16 {
    PointUtf16::from_monaco(line, column)
}

/// Monaco `editor.setPosition`, then the range update.
fn set_position(editor: &Entity<CodeEditor>, cx: &mut VisualTestContext, line: u32, column: u32) {
    editor.update(cx, |e, cx| {
        e.set_selections(&[Selection::collapsed(mp(line, column))], cx)
    });
    settle(cx);
}

/// Monaco `editor.setSelection(new Range(...))`, then the range update.
fn set_selection(
    editor: &Entity<CodeEditor>,
    cx: &mut VisualTestContext,
    (start_line, start_column, end_line, end_column): (u32, u32, u32, u32),
) {
    editor.update(cx, |e, cx| {
        e.set_selections(
            &[Selection::new(
                mp(start_line, start_column),
                mp(end_line, end_column),
            )],
            cx,
        )
    });
    settle(cx);
}

/// Monaco `testEditor.trigger('keyboard', handler, payload)`, then the sync.
fn trigger(
    editor: &Entity<CodeEditor>,
    cx: &mut VisualTestContext,
    handler: &str,
    payload: serde_json::Value,
) {
    editor
        .update_in(cx, |e, window, cx| {
            e.trigger_with_payload(handler, payload, window, cx)
        })
        .unwrap_or_else(|err| panic!("{handler}: {err}"));
    settle(cx);
}

fn type_text(editor: &Entity<CodeEditor>, cx: &mut VisualTestContext, text: &str) {
    trigger(editor, cx, "type", json!({ "text": text }));
}

fn paste(editor: &Entity<CodeEditor>, cx: &mut VisualTestContext, text: &str) {
    trigger(editor, cx, "paste", json!({ "text": text }));
}

fn undo(editor: &Entity<CodeEditor>, cx: &mut VisualTestContext) {
    editor.update(cx, |e, cx| e.undo(cx));
    settle(cx);
}

fn redo(editor: &Entity<CodeEditor>, cx: &mut VisualTestContext) {
    editor.update(cx, |e, cx| e.redo(cx));
    settle(cx);
}

const STATE: &str = "<ooo></ooo>";

/// A Monaco `testCase` on `STATE` that sets the position, then types or
/// pastes `text`.
macro_rules! linked_case {
    ($name:ident, pattern: $pattern:expr, at: ($line:expr, $column:expr), $op:ident $text:expr, $expected:expr) => {
        #[gpui::test]
        fn $name(cx: &mut TestAppContext) {
            let (editor, cx) = mock_editor(STATE, $pattern, cx);
            set_position(&editor, cx, $line, $column);
            $op(&editor, cx, $text);
            assert_eq!(text(&editor, cx), $expected);
        }
    };
}

// Simple insertion.
linked_case!(simple_insert_initial, pattern: None, at: (1, 2), type_text "i", "<iooo></iooo>");
linked_case!(simple_insert_middle, pattern: None, at: (1, 3), type_text "i", "<oioo></oioo>");
linked_case!(simple_insert_end, pattern: None, at: (1, 5), type_text "i", "<oooi></oooi>");

// Simple insertion - end.
linked_case!(simple_insert_end_initial, pattern: None, at: (1, 8), type_text "i", "<iooo></iooo>");
linked_case!(simple_insert_end_middle, pattern: None, at: (1, 9), type_text "i", "<oioo></oioo>");
linked_case!(simple_insert_end_end, pattern: None, at: (1, 11), type_text "i", "<oooi></oooi>");

// Boundary insertion.
linked_case!(simple_insert_out_of_boundary,
    pattern: None, at: (1, 1), type_text "i", "i<ooo></ooo>");
linked_case!(simple_insert_out_of_boundary_2,
    pattern: None, at: (1, 6), type_text "i", "<ooo>i</ooo>");
linked_case!(simple_insert_out_of_boundary_3,
    pattern: None, at: (1, 7), type_text "i", "<ooo><i/ooo>");
linked_case!(simple_insert_out_of_boundary_4,
    pattern: None, at: (1, 12), type_text "i", "<ooo></ooo>i");

// Insert + move.
#[gpui::test]
fn continuous_insert(cx: &mut TestAppContext) {
    let (editor, cx) = mock_editor(STATE, None, cx);
    set_position(&editor, cx, 1, 2);
    type_text(&editor, cx, "i");
    type_text(&editor, cx, "i");
    assert_eq!(text(&editor, cx), "<iiooo></iiooo>");
}

#[gpui::test]
fn insert_move_insert(cx: &mut TestAppContext) {
    let (editor, cx) = mock_editor(STATE, None, cx);
    set_position(&editor, cx, 1, 2);
    type_text(&editor, cx, "i");
    set_position(&editor, cx, 1, 4);
    type_text(&editor, cx, "i");
    assert_eq!(text(&editor, cx), "<ioioo></ioioo>");
}

#[gpui::test]
fn insert_move_insert_outside_region(cx: &mut TestAppContext) {
    let (editor, cx) = mock_editor(STATE, None, cx);
    set_position(&editor, cx, 1, 2);
    type_text(&editor, cx, "i");
    set_position(&editor, cx, 1, 7);
    type_text(&editor, cx, "i");
    assert_eq!(text(&editor, cx), "<iooo>i</iooo>");
}

/// Monaco's quick edits with a range update before synchronization: two
/// edits in one update, then a range update (`updateRanges()`) before
/// their deferred syncs run, from a listener of the first edit's content
/// change, then a third edit. The range update finds the cursor in the
/// reference range at the edits' version and keeps the ranges without
/// asking the provider again (microsoft/vscode#343009).
fn quick_edits_with_range_update(
    at: (u32, u32),
    handler: &str,
    payloads: [serde_json::Value; 3],
    cx: &mut TestAppContext,
) -> String {
    let (editor, cx, calls) = mock_editor_counting(STATE, None, cx);
    set_position(&editor, cx, at.0, at.1);
    let pending = Rc::new(Cell::new(false));
    let _subscription = cx.update(|_, cx| {
        let pending = pending.clone();
        cx.subscribe(&editor, move |editor, event: &CodeEditorEvent, cx| {
            if matches!(event, CodeEditorEvent::ModelContentChanged(_)) && pending.take() {
                editor.update(cx, |e, cx| e.update_linked_editing(false, cx));
            }
        })
    });
    let [first, second, third] = payloads;
    let asked = calls.load(Ordering::SeqCst);
    pending.set(true);
    editor.update_in(cx, |e, window, cx| {
        for payload in [first, second] {
            e.trigger_with_payload(handler, payload, window, cx)
                .unwrap_or_else(|err| panic!("{handler}: {err}"));
        }
    });
    assert!(!pending.get(), "no content change");
    assert_eq!(calls.load(Ordering::SeqCst), asked, "asked again");
    settle(cx);
    trigger(&editor, cx, handler, third);
    text(&editor, cx)
}

#[gpui::test]
fn quick_inserts_with_a_range_update_before_synchronization(cx: &mut TestAppContext) {
    let text = quick_edits_with_range_update(
        (1, 2),
        "type",
        [
            json!({ "text": "i" }),
            json!({ "text": "j" }),
            json!({ "text": "k" }),
        ],
        cx,
    );
    assert_eq!(text, "<ijkooo></ijkooo>");
}

#[gpui::test]
fn quick_inserts_in_closing_tag_with_a_range_update_before_synchronization(
    cx: &mut TestAppContext,
) {
    let text = quick_edits_with_range_update(
        (1, 8),
        "type",
        [
            json!({ "text": "i" }),
            json!({ "text": "j" }),
            json!({ "text": "k" }),
        ],
        cx,
    );
    assert_eq!(text, "<ijkooo></ijkooo>");
}

#[gpui::test]
fn edits_outside_the_reference_range_refresh_linked_editing(cx: &mut TestAppContext) {
    let (editor, cx) = mock_editor(STATE, None, cx);
    set_position(&editor, cx, 1, 2);
    editor.update(cx, |e, cx| {
        e.execute_edits(vec![(mp(1, 8)..mp(1, 11), "other".into())], None, cx);
    });
    settle(cx);
    type_text(&editor, cx, "i");
    assert_eq!(text(&editor, cx), "<iooo></other>");
}

/// Sets the position, then the selection, then runs `op`: the text after.
fn selection_case(
    position: (u32, u32),
    selection: (u32, u32, u32, u32),
    op: impl FnOnce(&Entity<CodeEditor>, &mut VisualTestContext),
    cx: &mut TestAppContext,
) -> String {
    let (editor, cx) = mock_editor(STATE, None, cx);
    set_position(&editor, cx, position.0, position.1);
    set_selection(&editor, cx, selection);
    op(&editor, cx);
    text(&editor, cx)
}

// Selection insert.
#[gpui::test]
fn selection_insert_simple(cx: &mut TestAppContext) {
    let text = selection_case((1, 2), (1, 2, 1, 3), |e, cx| type_text(e, cx, "i"), cx);
    assert_eq!(text, "<ioo></ioo>");
}

#[gpui::test]
fn selection_insert_whole(cx: &mut TestAppContext) {
    let text = selection_case((1, 2), (1, 2, 1, 5), |e, cx| type_text(e, cx, "i"), cx);
    assert_eq!(text, "<i></i>");
}

/// Monaco's `nodeAcceptEdit` keeps a decoration edge inside a replaced
/// span in place for the first `min(deleted, inserted)` characters, so the
/// reference range becomes `oo` rather than taking in the typed `i`.
#[gpui::test]
fn selection_insert_across_boundary(cx: &mut TestAppContext) {
    let text = selection_case((1, 2), (1, 1, 1, 3), |e, cx| type_text(e, cx, "i"), cx);
    assert_eq!(text, "ioo></oo>");
}

// Break out behavior.
linked_case!(breakout_type_space, pattern: None, at: (1, 5), type_text " ", "<ooo ></ooo>");

#[gpui::test]
fn breakout_type_space_then_undo(cx: &mut TestAppContext) {
    let (editor, cx) = mock_editor(STATE, None, cx);
    set_position(&editor, cx, 1, 5);
    type_text(&editor, cx, " ");
    undo(&editor, cx);
    assert_eq!(text(&editor, cx), "<ooo></ooo>");
}

linked_case!(breakout_type_space_in_middle,
    pattern: None, at: (1, 4), type_text " ", "<oo o></ooo>");
linked_case!(breakout_paste_content_starting_with_space,
    pattern: None, at: (1, 5), paste " i=\"i\"", "<ooo i=\"i\"></ooo>");

#[gpui::test]
fn breakout_paste_content_starting_with_space_then_undo(cx: &mut TestAppContext) {
    let (editor, cx) = mock_editor(STATE, None, cx);
    set_position(&editor, cx, 1, 5);
    paste(&editor, cx, " i=\"i\"");
    undo(&editor, cx);
    assert_eq!(text(&editor, cx), "<ooo></ooo>");
}

/// Monaco `_syncRanges` pops the undo stop the paste pushed after itself,
/// so the mirror edit joins the paste's undo step; typing after starts a
/// new one.
#[gpui::test]
fn paste_and_its_mirror_edit_are_one_undo_step(cx: &mut TestAppContext) {
    let (editor, cx) = mock_editor(STATE, None, cx);
    set_position(&editor, cx, 1, 5);
    paste(&editor, cx, "abc");
    assert_eq!(text(&editor, cx), "<oooabc></oooabc>");
    type_text(&editor, cx, "x");
    assert_eq!(text(&editor, cx), "<oooabcx></oooabcx>");
    undo(&editor, cx);
    assert_eq!(text(&editor, cx), "<oooabc></oooabc>");
    undo(&editor, cx);
    assert_eq!(text(&editor, cx), STATE);
    redo(&editor, cx);
    assert_eq!(text(&editor, cx), "<oooabc></oooabc>");
}

/// Likewise for a cut (`Other`, with undo stops before and after).
#[gpui::test]
fn cut_and_its_mirror_edit_are_one_undo_step(cx: &mut TestAppContext) {
    let (editor, cx) = mock_editor(STATE, None, cx);
    set_position(&editor, cx, 1, 2);
    set_selection(&editor, cx, (1, 2, 1, 4));
    trigger(&editor, cx, "cut", json!({}));
    assert_eq!(text(&editor, cx), "<o></o>");
    undo(&editor, cx);
    assert_eq!(text(&editor, cx), STATE);
}

linked_case!(breakout_paste_content_starting_with_space_in_middle,
    pattern: None, at: (1, 4), paste " i", "<oo io></ooo>");

// Break out with custom provider wordPattern.
const STOP_Z: Option<&str> = Some("[a-yA-Y]+");
const STOP_F: Option<&str> = Some("[a-eA-E]+");

linked_case!(breakout_with_stop_pattern_insert,
    pattern: STOP_Z, at: (1, 2), type_text "i", "<iooo></iooo>");
linked_case!(breakout_with_stop_pattern_insert_stop_char,
    pattern: STOP_Z, at: (1, 2), type_text "z", "<zooo></ooo>");
linked_case!(breakout_with_stop_pattern_paste_char,
    pattern: STOP_Z, at: (1, 2), paste "z", "<zooo></ooo>");
linked_case!(breakout_with_stop_pattern_paste_string,
    pattern: STOP_Z, at: (1, 2), paste "zo", "<zoooo></ooo>");
linked_case!(breakout_with_stop_pattern_insert_at_end,
    pattern: STOP_Z, at: (1, 5), type_text "z", "<oooz></ooo>");
linked_case!(breakout_with_stop_pattern_insert_stop_char_respos,
    pattern: STOP_F, at: (1, 2), type_text "i", "<iooo></ooo>");

// Delete.
fn delete(handler: &'static str) -> impl Fn(&Entity<CodeEditor>, &mut VisualTestContext) {
    move |editor, cx| trigger(editor, cx, handler, json!({}))
}

#[gpui::test]
fn delete_left_char(cx: &mut TestAppContext) {
    let (editor, cx) = mock_editor(STATE, None, cx);
    set_position(&editor, cx, 1, 5);
    delete("deleteLeft")(&editor, cx);
    assert_eq!(text(&editor, cx), "<oo></oo>");
}

#[gpui::test]
fn delete_left_char_then_undo(cx: &mut TestAppContext) {
    let (editor, cx) = mock_editor(STATE, None, cx);
    set_position(&editor, cx, 1, 5);
    delete("deleteLeft")(&editor, cx);
    undo(&editor, cx);
    assert_eq!(text(&editor, cx), "<ooo></ooo>");
}

#[gpui::test]
fn quick_deletes_with_a_range_update_before_synchronization(cx: &mut TestAppContext) {
    let text =
        quick_edits_with_range_update((1, 5), "deleteLeft", [json!({}), json!({}), json!({})], cx);
    assert_eq!(text, "<></>");
}

#[gpui::test]
fn delete_left_word(cx: &mut TestAppContext) {
    let (editor, cx) = mock_editor(STATE, None, cx);
    set_position(&editor, cx, 1, 5);
    delete("deleteWordLeft")(&editor, cx);
    assert_eq!(text(&editor, cx), "<></>");
}

#[gpui::test]
fn delete_left_word_then_undo(cx: &mut TestAppContext) {
    let (editor, cx) = mock_editor(STATE, None, cx);
    set_position(&editor, cx, 1, 5);
    delete("deleteWordLeft")(&editor, cx);
    undo(&editor, cx);
    undo(&editor, cx);
    assert_eq!(text(&editor, cx), "<ooo></ooo>");
}

#[gpui::test]
fn delete_left_all_then_undo_twice(cx: &mut TestAppContext) {
    let (editor, cx) = mock_editor(STATE, None, cx);
    set_position(&editor, cx, 1, 5);
    delete("deleteAllLeft")(&editor, cx);
    undo(&editor, cx);
    undo(&editor, cx);
    assert_eq!(text(&editor, cx), "<ooo></ooo>");
}

#[gpui::test]
fn delete_selection(cx: &mut TestAppContext) {
    let text = selection_case((1, 5), (1, 2, 1, 3), delete("deleteLeft"), cx);
    assert_eq!(text, "<oo></oo>");
}

#[gpui::test]
fn delete_selection_across_boundary(cx: &mut TestAppContext) {
    let text = selection_case((1, 3), (1, 1, 1, 3), delete("deleteLeft"), cx);
    assert_eq!(text, "oo></oo>");
}

// Undo / redo.
#[gpui::test]
fn undo_redo_simple_undo(cx: &mut TestAppContext) {
    let (editor, cx) = mock_editor(STATE, None, cx);
    set_position(&editor, cx, 1, 2);
    type_text(&editor, cx, "i");
    undo(&editor, cx);
    undo(&editor, cx);
    assert_eq!(text(&editor, cx), "<ooo></ooo>");
}

#[gpui::test]
fn undo_redo_simple_undo_redo(cx: &mut TestAppContext) {
    let (editor, cx) = mock_editor(STATE, None, cx);
    set_position(&editor, cx, 1, 2);
    type_text(&editor, cx, "i");
    undo(&editor, cx);
    redo(&editor, cx);
    assert_eq!(text(&editor, cx), "<iooo></iooo>");
}

// Multi line.
#[gpui::test]
fn multiline_insert(cx: &mut TestAppContext) {
    let (editor, cx) = mock_editor("<ooo>\n</ooo>", None, cx);
    set_position(&editor, cx, 1, 2);
    type_text(&editor, cx, "i");
    assert_eq!(text(&editor, cx), "<iooo>\n</iooo>");
}
