//! Find controller (Monaco `CommonFindController`): a port of Monaco's
//! `contrib/find/test/browser/findController.test.ts`.
//!
//! Positions in the assertions are Monaco's 1-based `[startLine,
//! startColumn, endLine, endColumn]` arrays, copied verbatim from the
//! TypeScript; the helpers convert to and from our 0-based `PointUtf16`.
//!
//! Mapping:
//! - `executeAction(StartFindAction)` / `editor.runAction(NextMatchFindAction)`
//!   dispatch our `StartFind` / `NextMatch` actions.
//! - `findState.change({...}, moveCursor)` is
//!   [`CodeEditor::change_find_options`]; `findState.isRevealed` is
//!   [`CodeEditor::is_find_visible`].
//! - `findController.moveToNextMatch()` / `replace()` / `closeFindWidget()`
//!   are [`CodeEditor::find_next`] / [`CodeEditor::replace_one`] /
//!   [`CodeEditor::close_find`].
//! - `findController.start({ seedSearchStringFromSelection: 'none', … })` is
//!   [`CodeEditor::open_find`] with `find.seedSearchStringFromSelection`
//!   set to `never` for the call (the per-call start options aren't public);
//!   `updateSearchScope: true` comes from `find.autoFindInSelection`, which
//!   Monaco's tests set too.
//! - `findState.searchScope` is [`CodeEditor::find_search_scope`].
//! - `TestFindController.hasFocus` (the find widget took focus) is "the
//!   editor no longer has text focus".
//!
//! Monaco's three global find clipboard tests
//! (`stores to the global clipboard buffer on start find action`,
//! `reads from the global clipboard buffer on next find action if buffer
//! exists`, `writes to the global clipboard buffer when text changes`) are
//! commented out upstream and are not ported.

use std::ops::Range;
use std::time::Duration;

use gpui::{Action, AppContext as _, Entity, Focusable, TestAppContext, VisualTestContext};
use gpui_monaco::actions::{
    NextMatch, NextSelectionMatch, StartFind, StartFindReplace, StartFindWithSelection,
};
use gpui_monaco::cursor::Selection;
use gpui_monaco::options::{AutoFindInSelection, FindEditorOptions, SeedSearchString};
use gpui_monaco::text::PointUtf16;
use gpui_monaco::{CodeEditor, FindOptions, TextModel};

type Rng = [u32; 4];

fn init(cx: &mut TestAppContext) {
    cx.update(gpui_monaco::init);
}

/// `withAsyncTestCodeEditor(lines, …)`, focused, without `init`.
fn open_editor<'a>(
    lines: &[&str],
    cx: &'a mut TestAppContext,
) -> (Entity<CodeEditor>, &'a mut VisualTestContext) {
    let text = lines.join("\n");
    let (editor, cx) = cx.add_window_view(|window, cx| {
        let model = cx.new(|_| TextModel::new(&text));
        CodeEditor::new(model, window, cx)
    });
    cx.update(|window, cx| window.focus(&editor.focus_handle(cx), cx));
    cx.run_until_parked();
    (editor, cx)
}

fn setup<'a>(
    lines: &[&str],
    cx: &'a mut TestAppContext,
) -> (Entity<CodeEditor>, &'a mut VisualTestContext) {
    init(cx);
    open_editor(lines, cx)
}

/// Monaco (line, column) → ours.
fn pt(line: u32, column: u32) -> PointUtf16 {
    PointUtf16::new(line - 1, column - 1)
}

/// Monaco `fromSelection`: `[startLine, startColumn, endLine, endColumn]`.
fn from_selection(s: &Selection) -> Rng {
    let (start, end) = (s.start(), s.end());
    [start.row + 1, start.column + 1, end.row + 1, end.column + 1]
}

/// `editor.getSelection()` (the primary selection).
fn selection(editor: &Entity<CodeEditor>, cx: &mut VisualTestContext) -> Rng {
    editor.read_with(cx, |e, _| from_selection(&e.selections()[0]))
}

/// `editor.getSelections().map(fromSelection)`.
fn selections(editor: &Entity<CodeEditor>, cx: &mut VisualTestContext) -> Vec<Rng> {
    editor.read_with(cx, |e, _| {
        e.selections().iter().map(from_selection).collect()
    })
}

/// `editor.setSelection(new Selection(startLine, startColumn, endLine,
/// endColumn))` / `editor.setPosition(...)` (an API call).
fn set_selection(editor: &Entity<CodeEditor>, cx: &mut VisualTestContext, s: Rng) {
    let selection = Selection::new(pt(s[0], s[1]), pt(s[2], s[3]));
    editor.update(cx, |e, cx| e.set_selections(&[selection], cx));
}

/// Lets the find model's delayed re-search and the widget's history update
/// run.
fn flush(cx: &mut VisualTestContext) {
    cx.run_until_parked();
    cx.executor().advance_clock(Duration::from_millis(600));
    cx.run_until_parked();
}

/// `executeAction(…)` / `editor.runAction(…)`: dispatches `action` to the
/// focused element, as its keybinding would.
fn run(cx: &mut VisualTestContext, action: impl Action) {
    cx.dispatch_action(action);
    flush(cx);
}

/// `withAsyncTestCodeEditor(…, { find: { … } })`.
fn set_find_editor_options(
    editor: &Entity<CodeEditor>,
    cx: &mut VisualTestContext,
    f: impl FnOnce(&mut FindEditorOptions),
) {
    editor.update(cx, |e, cx| {
        let mut options = e.options().clone();
        f(&mut options.find);
        e.update_options(options, cx);
    });
}

/// `findState.searchScope`, as `[startLine, startColumn, endLine,
/// endColumn]`s.
fn search_scope(editor: &Entity<CodeEditor>, cx: &mut VisualTestContext) -> Option<Vec<Rng>> {
    editor.read_with(cx, |e, _| {
        e.find_search_scope().map(|scope| {
            scope
                .iter()
                .map(|r| {
                    [
                        r.start.row + 1,
                        r.start.column + 1,
                        r.end.row + 1,
                        r.end.column + 1,
                    ]
                })
                .collect()
        })
    })
}

/// `findState.change({...}, moveCursor)` for the query and toggles.
fn change(
    editor: &Entity<CodeEditor>,
    cx: &mut VisualTestContext,
    move_cursor: bool,
    f: impl FnOnce(&mut FindOptions),
) {
    editor.update(cx, |e, cx| {
        let mut options = e.find_options().clone();
        f(&mut options);
        e.change_find_options(options, move_cursor, cx);
    });
    flush(cx);
}

/// `findController.toggleRegex()`: `change({ isRegex: !isRegex }, false)`.
fn toggle_regex(editor: &Entity<CodeEditor>, cx: &mut VisualTestContext) {
    change(editor, cx, false, |o| o.is_regex = !o.is_regex);
}

/// Monaco `strings.escapeRegExpCharacters`.
fn escape_reg_exp_characters(value: &str) -> String {
    let mut escaped = String::with_capacity(value.len());
    for c in value.chars() {
        if "\\{}*+?|^$.[]()".contains(c) {
            escaped.push('\\');
        }
        escaped.push(c);
    }
    escaped
}

/// `findController.setSearchString(s)`: escaped in regex mode, without
/// moving the cursor.
fn set_search_string(editor: &Entity<CodeEditor>, cx: &mut VisualTestContext, s: &str) {
    change(editor, cx, false, |o| {
        o.query = if o.is_regex {
            escape_reg_exp_characters(s)
        } else {
            s.to_string()
        };
    });
}

/// `findController.start({ seedSearchStringFromSelection: 'none',
/// forceRevealReplace: false, … })`.
fn start_without_seed(editor: &Entity<CodeEditor>, cx: &mut VisualTestContext) {
    editor.update_in(cx, |e, window, cx| {
        let saved = e.options().clone();
        let mut options = saved.clone();
        options.find.seed_search_string_from_selection = SeedSearchString::Never;
        e.update_options(options, cx);
        e.open_find(false, window, cx);
        e.update_options(saved, cx);
    });
    flush(cx);
}

/// `findController.closeFindWidget()`.
fn close_find_widget(editor: &Entity<CodeEditor>, cx: &mut VisualTestContext) {
    editor.update_in(cx, |e, window, cx| e.close_find(window, cx));
    flush(cx);
}

/// `findController.moveToNextMatch()`.
fn move_to_next_match(editor: &Entity<CodeEditor>, cx: &mut VisualTestContext) {
    editor.update(cx, |e, cx| e.find_next(cx));
    flush(cx);
}

/// `findController.replace()`.
fn replace(editor: &Entity<CodeEditor>, cx: &mut VisualTestContext) {
    editor.update(cx, |e, cx| e.replace_one(cx));
    flush(cx);
}

/// `findState.searchString`.
fn search_string(editor: &Entity<CodeEditor>, cx: &mut VisualTestContext) -> String {
    editor.read_with(cx, |e, _| e.find_options().query.clone())
}

/// `findState.isRevealed`.
fn is_revealed(editor: &Entity<CodeEditor>, cx: &mut VisualTestContext) -> bool {
    editor.read_with(cx, |e, _| e.is_find_visible())
}

/// `findState.matchesCount`.
fn matches_count(editor: &Entity<CodeEditor>, cx: &mut VisualTestContext) -> usize {
    editor.read_with(cx, |e, cx| e.find_state_for_tests(cx).3)
}

/// `TestFindController.hasFocus`: the find widget took the focus.
fn find_has_focus(editor: &Entity<CodeEditor>, cx: &mut VisualTestContext) -> bool {
    editor.update_in(cx, |e, window, _| !e.has_text_focus(window))
}

/// `editor.getValue()`.
fn value(editor: &Entity<CodeEditor>, cx: &mut VisualTestContext) -> String {
    editor.read_with(cx, |e, cx| e.model().read(cx).text())
}

/// `editor.getModel().getLineContent(line)`.
fn line_content(editor: &Entity<CodeEditor>, cx: &mut VisualTestContext, line: u32) -> String {
    editor.read_with(cx, |e, cx| e.model().read(cx).line_content(line - 1))
}

/// `editor.executeEdits('test', [{ range, text }])`.
fn execute_edit(
    editor: &Entity<CodeEditor>,
    cx: &mut VisualTestContext,
    range: Range<PointUtf16>,
    text: &str,
) {
    editor.update(cx, |e, cx| {
        e.execute_edits(vec![(range, text.to_string())], None, cx);
    });
    flush(cx);
}

/// `editor.pushUndoStop()`.
fn push_undo_stop(editor: &Entity<CodeEditor>, cx: &mut VisualTestContext) {
    editor.update(cx, |e, cx| e.push_undo_stop(cx));
}

// ----- suite('FindController') ------------------------------------------------

/// Monaco: "issue #1857: F3, Find Next, acts like "Find Under Cursor""
#[gpui::test]
fn issue_1857_f3_find_next_acts_like_find_under_cursor(cx: &mut TestAppContext) {
    let (editor, cx) = setup(&["ABC", "ABC", "XYZ", "ABC"], cx);
    // The cursor is at the very top, of the file, at the first ABC

    // I hit Ctrl+F to show the Find dialog
    run(cx, StartFind);

    // I type ABC.
    change(&editor, cx, true, |o| o.query = "A".into());
    change(&editor, cx, true, |o| o.query = "AB".into());
    change(&editor, cx, true, |o| o.query = "ABC".into());

    // The first ABC is highlighted.
    assert_eq!(selection(&editor, cx), [1, 1, 1, 4]);

    // I hit Esc to exit the Find dialog.
    close_find_widget(&editor, cx);
    assert!(!find_has_focus(&editor, cx));

    // The cursor is now at end of the first line, with ABC on that line highlighted.
    assert_eq!(selection(&editor, cx), [1, 1, 1, 4]);

    // I hit delete to remove it and change the text to XYZ.
    push_undo_stop(&editor, cx);
    execute_edit(&editor, cx, pt(1, 1)..pt(1, 4), "");
    execute_edit(&editor, cx, pt(1, 1)..pt(1, 1), "XYZ");
    push_undo_stop(&editor, cx);

    // At this point the text editor looks like this:
    //   XYZ
    //   ABC
    //   XYZ
    //   ABC
    assert_eq!(line_content(&editor, cx, 1), "XYZ");

    // The cursor is at end of the first line.
    assert_eq!(selection(&editor, cx), [1, 4, 1, 4]);

    // I hit F3 to "Find Next" to find the next occurrence of ABC, but instead it searches for XYZ.
    run(cx, NextMatch);

    assert_eq!(search_string(&editor, cx), "ABC");
    assert!(!find_has_focus(&editor, cx));
}

/// Monaco: "issue #3090: F3 does not loop with two matches on a single line"
#[gpui::test]
fn issue_3090_f3_does_not_loop_with_two_matches_on_a_single_line(cx: &mut TestAppContext) {
    let (editor, cx) = setup(&["import nls = require('vs/nls');"], cx);

    set_selection(&editor, cx, [1, 9, 1, 9]);

    run(cx, NextMatch);
    assert_eq!(selection(&editor, cx), [1, 26, 1, 29]);

    run(cx, NextMatch);
    assert_eq!(selection(&editor, cx), [1, 8, 1, 11]);
}

/// Monaco: "issue #6149: Auto-escape highlighted text for search and replace regex mode"
#[gpui::test]
fn issue_6149_auto_escape_highlighted_text_for_search_and_replace_regex_mode(
    cx: &mut TestAppContext,
) {
    let (editor, cx) = setup(
        &["var x = (3 * 5)", "var y = (3 * 5)", "var z = (3  * 5)"],
        cx,
    );

    set_selection(&editor, cx, [1, 9, 1, 13]);

    toggle_regex(&editor, cx);
    run(cx, StartFind);

    run(cx, NextMatch);
    assert_eq!(selection(&editor, cx), [2, 9, 2, 13]);

    run(cx, NextMatch);
    assert_eq!(selection(&editor, cx), [1, 9, 1, 13]);
}

/// Monaco: "issue #41027: Don't replace find input value on replace action if find input is active"
#[gpui::test]
fn issue_41027_dont_replace_find_input_value_on_replace_action_if_find_input_is_active(
    cx: &mut TestAppContext,
) {
    // Monaco's suite shares one `InMemoryStorageService`, and the previous
    // test (#6149) toggled regex on, which the controller stores under
    // `editor.isRegex` (findController.ts `_onStateChanged`) and this test's
    // controller reads back on construction. So `toggleRegex()` below turns
    // regex *off*, and `setSearchString` doesn't escape: the assertion only
    // holds with that state. Each gpui test has its own App (our storage),
    // so we store it here.
    init(cx);
    store(cx, |o| o.is_regex = true);
    let (editor, cx) = open_editor(&["test"], cx);
    let test_regex_string = "tes.";

    toggle_regex(&editor, cx);
    set_search_string(&editor, cx, test_regex_string);
    // shouldFocus: FocusFindInput
    start_without_seed(&editor, cx);
    run(cx, NextMatch);
    run(cx, StartFindReplace);

    assert_eq!(search_string(&editor, cx), test_regex_string);
}

/// Monaco: "editor.find.closeOnResult: closes find widget when a match is found from explicit navigation"
#[gpui::test]
fn editor_find_close_on_result_closes_find_widget_when_a_match_is_found_from_explicit_navigation(
    cx: &mut TestAppContext,
) {
    let (editor, cx) = setup(&["ABC", "ABC", "XYZ"], cx);
    set_find_editor_options(&editor, cx, |o| o.close_on_result = true);

    run(cx, StartFind);
    assert!(is_revealed(&editor, cx));

    change(&editor, cx, true, |o| o.query = "ABC".into());
    run(cx, NextMatch);

    assert!(!is_revealed(&editor, cx));
}

/// Monaco: "editor.find.closeOnResult: keeps find widget open when no match is found"
#[gpui::test]
fn editor_find_close_on_result_keeps_find_widget_open_when_no_match_is_found(
    cx: &mut TestAppContext,
) {
    let (editor, cx) = setup(&["ABC", "DEF", "XYZ"], cx);
    set_find_editor_options(&editor, cx, |o| o.close_on_result = true);

    run(cx, StartFind);
    assert!(is_revealed(&editor, cx));

    change(&editor, cx, true, |o| o.query = "NO_MATCH".into());
    run(cx, NextMatch);

    assert_eq!(matches_count(&editor, cx), 0);
    assert!(is_revealed(&editor, cx));
}

/// Monaco: "editor.find.closeOnResult: disabled keeps find widget open after navigation"
#[gpui::test]
fn editor_find_close_on_result_disabled_keeps_find_widget_open_after_navigation(
    cx: &mut TestAppContext,
) {
    let (editor, cx) = setup(&["ABC", "ABC", "XYZ"], cx);
    set_find_editor_options(&editor, cx, |o| o.close_on_result = false);

    run(cx, StartFind);
    assert!(is_revealed(&editor, cx));

    change(&editor, cx, true, |o| o.query = "ABC".into());
    run(cx, NextMatch);

    assert!(is_revealed(&editor, cx));
}

/// Monaco: "issue #9043: Clear search scope when find widget is hidden"
#[gpui::test]
fn issue_9043_clear_search_scope_when_find_widget_is_hidden(cx: &mut TestAppContext) {
    let (editor, cx) = setup(
        &["var x = (3 * 5)", "var y = (3 * 5)", "var z = (3 * 5)"],
        cx,
    );
    // shouldFocus: NoFocusChange, updateSearchScope: false
    start_without_seed(&editor, cx);

    assert_eq!(search_scope(&editor, cx), None);

    editor.update(cx, |e, cx| {
        e.set_find_search_scope(Some(vec![pt(1, 1)..pt(1, 5)]), false, cx)
    });
    flush(cx);

    assert_eq!(search_scope(&editor, cx), Some(vec![[1, 1, 1, 5]]));

    close_find_widget(&editor, cx);
    assert_eq!(search_scope(&editor, cx), None);
}

/// Monaco: "issue #18111: Regex replace with single space replaces with no space"
#[gpui::test]
fn issue_18111_regex_replace_with_single_space_replaces_with_no_space(cx: &mut TestAppContext) {
    let (editor, cx) = setup(&["HRESULT OnAmbientPropertyChange(DISPID   dispid);"], cx);

    run(cx, StartFind);

    change(&editor, cx, false, |o| {
        o.query = "\\b\\s{3}\\b".into();
        o.replace = " ".into();
        o.is_regex = true;
    });
    move_to_next_match(&editor, cx);

    assert_eq!(selections(&editor, cx), vec![[1, 39, 1, 42]]);

    replace(&editor, cx);

    assert_eq!(
        value(&editor, cx),
        "HRESULT OnAmbientPropertyChange(DISPID dispid);"
    );
}

/// Monaco: "issue #24714: Regular expression with ^ in search & replace"
#[gpui::test]
fn issue_24714_regular_expression_with_caret_in_search_and_replace(cx: &mut TestAppContext) {
    let (editor, cx) = setup(&["", "line2", "line3"], cx);

    run(cx, StartFind);

    change(&editor, cx, false, |o| {
        o.query = "^".into();
        o.replace = "x".into();
        o.is_regex = true;
    });
    move_to_next_match(&editor, cx);

    assert_eq!(selections(&editor, cx), vec![[2, 1, 2, 1]]);

    replace(&editor, cx);

    assert_eq!(value(&editor, cx), "\nxline2\nline3");
}

/// Monaco: "issue #38232: Find Next Selection, regex enabled"
#[gpui::test]
fn issue_38232_find_next_selection_regex_enabled(cx: &mut TestAppContext) {
    let (editor, cx) = setup(&["([funny]", "", "([funny]"], cx);

    // toggle regex
    change(&editor, cx, false, |o| o.is_regex = true);

    // change selection
    set_selection(&editor, cx, [1, 1, 1, 9]);

    // cmd+f3
    run(cx, NextSelectionMatch);

    assert_eq!(selections(&editor, cx), vec![[3, 1, 3, 9]]);
}

/// Monaco: "issue #38232: Find Next Selection, regex enabled, find widget open"
#[gpui::test]
fn issue_38232_find_next_selection_regex_enabled_find_widget_open(cx: &mut TestAppContext) {
    let (editor, cx) = setup(&["([funny]", "", "([funny]"], cx);

    // cmd+f - open find widget
    run(cx, StartFind);

    // toggle regex
    change(&editor, cx, false, |o| o.is_regex = true);

    // change selection
    set_selection(&editor, cx, [1, 1, 1, 9]);

    // cmd+f3
    run(cx, NextSelectionMatch);

    assert_eq!(selections(&editor, cx), vec![[3, 1, 3, 9]]);
}

/// Monaco: "issue #47400, CMD+E supports feeding multiple line of text into the find widget"
#[gpui::test]
fn issue_47400_cmd_e_supports_feeding_multiple_line_of_text_into_the_find_widget(
    cx: &mut TestAppContext,
) {
    let (editor, cx) = setup(&["ABC", "ABC", "XYZ", "ABC", "ABC"], cx);

    // change selection
    set_selection(&editor, cx, [1, 1, 1, 1]);

    // cmd+f - open find widget
    run(cx, StartFind);

    set_selection(&editor, cx, [1, 1, 2, 4]);
    run(cx, StartFindWithSelection);

    // `searchString.split(/\r\n|\r|\n/g)`
    let query = search_string(&editor, cx).replace("\r\n", "\n");
    let lines: Vec<&str> = query.split(['\n', '\r']).collect();
    assert_eq!(lines, vec!["ABC", "ABC"]);

    set_selection(&editor, cx, [3, 1, 3, 1]);
    run(cx, StartFindWithSelection);
}

/// Monaco: "issue #109756, CMD+E with empty cursor should always work"
#[gpui::test]
fn issue_109756_cmd_e_with_empty_cursor_should_always_work(cx: &mut TestAppContext) {
    let (editor, cx) = setup(&["ABC", "ABC", "XYZ", "ABC", "ABC"], cx);
    set_selection(&editor, cx, [1, 2, 1, 2]);

    run(cx, StartFindWithSelection);

    assert_eq!(search_string(&editor, cx), "ABC");
}

// ----- suite('FindController query options persistence') ------------------------
//
// Monaco keeps the find toggles in workspace storage (`editor.isRegex`,
// `editor.matchCase`, `editor.wholeWord`, `editor.preserveCase`), shared by
// every editor: the controller reads them when created and when the widget
// shows, and writes them on every toggle. Ours live in an App global with no
// public setter, so `storageService.store(key, value)` is a toggle made in
// another editor of the same app, and `storageService.getBoolean(key)` is
// what a new editor starts with.

/// `storageService.store('editor.<toggle>', …, StorageScope.WORKSPACE)`.
fn store(cx: &mut TestAppContext, f: impl FnOnce(&mut FindOptions)) {
    let (other, cx) = open_editor(&[""], cx);
    change(&other, cx, false, f);
}

/// `storageService.getBoolean('editor.<toggle>', StorageScope.WORKSPACE)`.
fn stored(cx: &mut TestAppContext) -> FindOptions {
    let (other, cx) = open_editor(&[""], cx);
    other.read_with(cx, |e, _| e.find_options().clone())
}

/// Monaco: "matchCase"
#[gpui::test]
fn match_case(cx: &mut TestAppContext) {
    init(cx);
    store(cx, |o| o.match_case = true);
    let (editor, cx) = open_editor(&["abc", "ABC", "XYZ", "ABC"], cx);
    // The cursor is at the very top, of the file, at the first ABC

    // I hit Ctrl+F to show the Find dialog
    run(cx, StartFind);

    // I type ABC.
    change(&editor, cx, true, |o| o.query = "ABC".into());
    // The second ABC is highlighted as matchCase is true.
    assert_eq!(selection(&editor, cx), [2, 1, 2, 4]);
}

/// Monaco: "wholeWord"
#[gpui::test]
fn whole_word(cx: &mut TestAppContext) {
    init(cx);
    store(cx, |o| {
        o.match_case = false;
        o.whole_word = true;
    });
    let (editor, cx) = open_editor(&["ABC", "AB", "XYZ", "ABC"], cx);
    // The cursor is at the very top, of the file, at the first ABC

    // I hit Ctrl+F to show the Find dialog
    run(cx, StartFind);

    // I type AB.
    change(&editor, cx, true, |o| o.query = "AB".into());
    // The second AB is highlighted as wholeWord is true.
    assert_eq!(selection(&editor, cx), [2, 1, 2, 3]);
}

/// Monaco: "toggling options is saved"
#[gpui::test]
fn toggling_options_is_saved(cx: &mut TestAppContext) {
    init(cx);
    {
        let (editor, cx) = open_editor(&["ABC", "AB", "XYZ", "ABC"], cx);
        // The cursor is at the very top, of the file, at the first ABC
        toggle_regex(&editor, cx);
    }
    assert!(stored(cx).is_regex);
}

const VAR_LINES: &[&str] = &["var x = (3 * 5)", "var y = (3 * 5)", "var z = (3 * 5)"];

/// Monaco: "issue #27083: Update search scope once find widget becomes visible"
#[gpui::test]
fn issue_27083_update_search_scope_once_find_widget_becomes_visible(cx: &mut TestAppContext) {
    let (editor, cx) = setup(VAR_LINES, cx);
    // find: { autoFindInSelection: 'always', globalFindClipboard: false }
    set_find_editor_options(&editor, cx, |o| {
        o.auto_find_in_selection = AutoFindInSelection::Always
    });

    set_selection(&editor, cx, [1, 1, 2, 1]);
    // updateSearchScope: true, shouldFocus: NoFocusChange
    start_without_seed(&editor, cx);
    assert_eq!(search_scope(&editor, cx), Some(vec![[1, 1, 2, 1]]));

    close_find_widget(&editor, cx);

    editor.update(cx, |e, cx| {
        e.set_selections(
            &[
                Selection::new(pt(1, 1), pt(2, 1)),
                Selection::new(pt(2, 1), pt(2, 5)),
            ],
            cx,
        )
    });
    start_without_seed(&editor, cx);
    assert_eq!(
        search_scope(&editor, cx),
        Some(vec![[1, 1, 2, 1], [2, 1, 2, 5]])
    );
}

/// Monaco: "issue #58604: Do not update searchScope if it is empty"
#[gpui::test]
fn issue_58604_do_not_update_search_scope_if_it_is_empty(cx: &mut TestAppContext) {
    let (editor, cx) = setup(VAR_LINES, cx);
    // find: { autoFindInSelection: 'always', globalFindClipboard: false }
    set_find_editor_options(&editor, cx, |o| {
        o.auto_find_in_selection = AutoFindInSelection::Always
    });
    set_selection(&editor, cx, [1, 2, 1, 2]);

    // updateSearchScope: true, shouldFocus: NoFocusChange
    start_without_seed(&editor, cx);

    assert_eq!(search_scope(&editor, cx), None);
}

/// Monaco: "issue #58604: Update searchScope if it is not empty"
#[gpui::test]
fn issue_58604_update_search_scope_if_it_is_not_empty(cx: &mut TestAppContext) {
    let (editor, cx) = setup(VAR_LINES, cx);
    // find: { autoFindInSelection: 'always', globalFindClipboard: false }
    set_find_editor_options(&editor, cx, |o| {
        o.auto_find_in_selection = AutoFindInSelection::Always
    });
    set_selection(&editor, cx, [1, 2, 1, 3]);

    // updateSearchScope: true, shouldFocus: NoFocusChange
    start_without_seed(&editor, cx);

    assert_eq!(search_scope(&editor, cx), Some(vec![[1, 2, 1, 3]]));
}

/// Monaco: "issue #27083: Find in selection when multiple lines are selected"
#[gpui::test]
fn issue_27083_find_in_selection_when_multiple_lines_are_selected(cx: &mut TestAppContext) {
    let (editor, cx) = setup(VAR_LINES, cx);
    // find: { autoFindInSelection: 'multiline', globalFindClipboard: false }
    set_find_editor_options(&editor, cx, |o| {
        o.auto_find_in_selection = AutoFindInSelection::Multiline
    });
    set_selection(&editor, cx, [1, 6, 2, 1]);

    // updateSearchScope: true, shouldFocus: NoFocusChange
    start_without_seed(&editor, cx);

    assert_eq!(search_scope(&editor, cx), Some(vec![[1, 6, 2, 1]]));
}
