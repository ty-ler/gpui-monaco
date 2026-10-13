//! IME composition: Monaco's `compositionType` at every cursor
//! (`TypeOperations.compositionType`, driven by the edit context) and the
//! IME's text formats (`nativeEditContext.ts` `_handleTextFormatUpdate`).

use gpui::{Entity, EntityInputHandler, Focusable, TestAppContext, VisualTestContext};
use gpui_monaco::CodeEditor;
use gpui_monaco::cursor::Selection;
use gpui_monaco::text::PointUtf16;

fn setup<'a>(
    text: &str,
    cx: &'a mut TestAppContext,
) -> (Entity<CodeEditor>, &'a mut VisualTestContext) {
    cx.update(gpui_monaco::init);
    let text = text.to_string();
    let (editor, cx) = cx.add_window_view(|window, cx| CodeEditor::with_text(&text, window, cx));
    cx.update(|window, cx| window.focus(&editor.focus_handle(cx), cx));
    cx.run_until_parked();
    (editor, cx)
}

/// Monaco 1-based (line, column).
fn p(line: u32, column: u32) -> PointUtf16 {
    PointUtf16::from_monaco(line, column)
}

fn text(editor: &Entity<CodeEditor>, cx: &mut VisualTestContext) -> String {
    editor.read_with(cx, |e, cx| e.model().read(cx).text())
}

fn selections(editor: &Entity<CodeEditor>, cx: &mut VisualTestContext) -> Vec<Selection> {
    editor.read_with(cx, |e, _| e.selections())
}

fn set(editor: &Entity<CodeEditor>, selections: &[Selection], cx: &mut VisualTestContext) {
    editor.update(cx, |e, cx| e.set_selections(selections, cx));
}

fn mark(
    editor: &Entity<CodeEditor>,
    text: &str,
    selected: Option<std::ops::Range<usize>>,
    cx: &mut VisualTestContext,
) {
    editor.update_in(cx, |e, window, cx| {
        e.replace_and_mark_text_in_range(None, text, selected, window, cx)
    });
}

fn commit(editor: &Entity<CodeEditor>, text: &str, cx: &mut VisualTestContext) {
    editor.update_in(cx, |e, window, cx| {
        e.replace_text_in_range(None, text, window, cx)
    });
}

fn marked(
    editor: &Entity<CodeEditor>,
    cx: &mut VisualTestContext,
) -> Option<std::ops::Range<usize>> {
    editor.update_in(cx, |e, window, cx| e.marked_text_range(window, cx))
}

/// Monaco composes at every cursor: the primary cursor's composition is
/// typed at each of them, and committing it commits them all.
#[gpui::test]
fn composition_with_multiple_cursors(cx: &mut TestAppContext) {
    let (editor, cx) = setup("ab\nab", cx);
    set(
        &editor,
        &[Selection::collapsed(p(1, 3)), Selection::collapsed(p(2, 3))],
        cx,
    );
    mark(&editor, "に", None, cx);
    assert_eq!(text(&editor, cx), "abに\nabに");
    assert_eq!(marked(&editor, cx), Some(2..3));
    mark(&editor, "にほ", None, cx);
    assert_eq!(text(&editor, cx), "abにほ\nabにほ");
    assert_eq!(marked(&editor, cx), Some(2..4));
    commit(&editor, "日本", cx);
    assert_eq!(text(&editor, cx), "ab日本\nab日本");
    assert_eq!(marked(&editor, cx), None);
    assert_eq!(
        selections(&editor, cx),
        vec![Selection::collapsed(p(1, 5)), Selection::collapsed(p(2, 5))]
    );
}

/// The IME's caret within the composition (`positionDelta`) moves every
/// cursor.
#[gpui::test]
fn composition_caret_with_multiple_cursors(cx: &mut TestAppContext) {
    let (editor, cx) = setup("x\ny", cx);
    set(
        &editor,
        &[Selection::collapsed(p(1, 2)), Selection::collapsed(p(2, 2))],
        cx,
    );
    mark(&editor, "abc", Some(1..1), cx);
    assert_eq!(text(&editor, cx), "xabc\nyabc");
    assert_eq!(
        selections(&editor, cx),
        vec![Selection::collapsed(p(1, 3)), Selection::collapsed(p(2, 3))]
    );
    assert_eq!(marked(&editor, cx), Some(1..4));
    commit(&editor, "ABC", cx);
    assert_eq!(text(&editor, cx), "xABC\nyABC");
}

/// A composition starting over selections replaces each of them.
#[gpui::test]
fn composition_over_multiple_selections(cx: &mut TestAppContext) {
    let (editor, cx) = setup("foo foo", cx);
    set(
        &editor,
        &[
            Selection {
                anchor: p(1, 1),
                head: p(1, 4),
            },
            Selection {
                anchor: p(1, 5),
                head: p(1, 8),
            },
        ],
        cx,
    );
    mark(&editor, "か", None, cx);
    assert_eq!(text(&editor, cx), "か か");
    commit(&editor, "下", cx);
    assert_eq!(text(&editor, cx), "下 下");
}

/// macOS press-and-hold: holding `e` types `e`, and picking `é` from the
/// accent menu replaces the `e` before the cursor without marked text.
/// Monaco sees `compositionType("é", 1, 0, 0)` (`nativeEditContext.ts`
/// `_emitTypeEvent`), which replaces it at every cursor.
#[gpui::test]
fn press_and_hold_accent_replaces_at_every_cursor(cx: &mut TestAppContext) {
    let (editor, cx) = setup(
        "ae
be", cx,
    );
    set(
        &editor,
        &[Selection::collapsed(p(1, 3)), Selection::collapsed(p(2, 3))],
        cx,
    );
    editor.update_in(cx, |e, window, cx| {
        e.replace_text_in_range(Some(1..2), "é", window, cx)
    });
    assert_eq!(
        text(&editor, cx),
        "aé
bé"
    );
    assert_eq!(marked(&editor, cx), None);
    assert_eq!(
        selections(&editor, cx),
        vec![Selection::collapsed(p(1, 3)), Selection::collapsed(p(2, 3))]
    );
    // One undo restores both.
    editor.update(cx, |e, cx| e.undo(cx));
    assert_eq!(
        text(&editor, cx),
        "ae
be"
    );
}

/// A replacement on the primary cursor's line that doesn't contain it is
/// still a `compositionType` at every cursor: Monaco `_emitTypeEvent` pads
/// the text between the range and the cursor into the inserted text, so
/// every line is edited and the cursors stay after it.
#[gpui::test]
fn replacing_text_away_from_the_cursors_replaces_it_at_every_cursor(cx: &mut TestAppContext) {
    let (editor, cx) = setup(
        "abc
abc", cx,
    );
    set(
        &editor,
        &[Selection::collapsed(p(1, 4)), Selection::collapsed(p(2, 4))],
        cx,
    );
    editor.update_in(cx, |e, window, cx| {
        e.replace_text_in_range(Some(0..1), "X", window, cx)
    });
    assert_eq!(
        text(&editor, cx),
        "Xbc
Xbc"
    );
    assert_eq!(
        selections(&editor, cx),
        vec![Selection::collapsed(p(1, 4)), Selection::collapsed(p(2, 4))]
    );
    // One undo restores both.
    editor.update(cx, |e, cx| e.undo(cx));
    assert_eq!(
        text(&editor, cx),
        "abc
abc"
    );
}

/// macOS autocorrect replacing `teh` in `teh |` (Monaco
/// `compositionType("the ", 4, 0, 0)`) keeps the caret after the space.
#[gpui::test]
fn autocorrect_before_the_caret_keeps_the_caret(cx: &mut TestAppContext) {
    let (editor, cx) = setup("teh ", cx);
    set(&editor, &[Selection::collapsed(p(1, 5))], cx);
    editor.update_in(cx, |e, window, cx| {
        e.replace_text_in_range(Some(0..3), "the", window, cx)
    });
    assert_eq!(text(&editor, cx), "the ");
    assert_eq!(selections(&editor, cx), vec![Selection::collapsed(p(1, 5))]);
}

/// A replacement after the caret pads the text between them in front, and
/// the caret lands after the replacement.
#[gpui::test]
fn replacing_text_after_the_caret(cx: &mut TestAppContext) {
    let (editor, cx) = setup("a teh", cx);
    set(&editor, &[Selection::collapsed(p(1, 2))], cx);
    editor.update_in(cx, |e, window, cx| {
        e.replace_text_in_range(Some(2..5), "the", window, cx)
    });
    assert_eq!(text(&editor, cx), "a the");
    assert_eq!(selections(&editor, cx), vec![Selection::collapsed(p(1, 6))]);
}

/// With one cursor, the composition's updates and commit are
/// `compositionType` edits that join the typing before and after them in
/// one undo step (Monaco `CompositionOperation`, `TypingOther`).
#[gpui::test]
fn single_cursor_composition_is_one_undo_step_with_the_typing(cx: &mut TestAppContext) {
    let (editor, cx) = setup("", cx);
    editor.update(cx, |e, cx| e.type_text("a", cx));
    mark(&editor, "に", None, cx);
    mark(&editor, "にほ", None, cx);
    mark(&editor, "にほん", None, cx);
    commit(&editor, "日本", cx);
    assert_eq!(text(&editor, cx), "a日本");
    assert_eq!(selections(&editor, cx), vec![Selection::collapsed(p(1, 4))]);
    editor.update(cx, |e, cx| e.type_text("b", cx));
    assert_eq!(text(&editor, cx), "a日本b");
    editor.update(cx, |e, cx| e.undo(cx));
    assert_eq!(text(&editor, cx), "");
}

/// A single-cursor composition starting over a selection replaces it, in
/// the same undo step.
#[gpui::test]
fn single_cursor_composition_over_a_selection(cx: &mut TestAppContext) {
    let (editor, cx) = setup("foo bar", cx);
    set(
        &editor,
        &[Selection {
            anchor: p(1, 1),
            head: p(1, 4),
        }],
        cx,
    );
    mark(&editor, "か", None, cx);
    assert_eq!(text(&editor, cx), "か bar");
    assert_eq!(marked(&editor, cx), Some(0..1));
    commit(&editor, "下", cx);
    assert_eq!(text(&editor, cx), "下 bar");
    assert_eq!(selections(&editor, cx), vec![Selection::collapsed(p(1, 2))]);
    editor.update(cx, |e, cx| e.undo(cx));
    assert_eq!(text(&editor, cx), "foo bar");
}

/// The IME's caret within a single-cursor composition moves the cursor.
#[gpui::test]
fn single_cursor_composition_caret(cx: &mut TestAppContext) {
    let (editor, cx) = setup("x", cx);
    set(&editor, &[Selection::collapsed(p(1, 2))], cx);
    mark(&editor, "abc", Some(1..1), cx);
    assert_eq!(text(&editor, cx), "xabc");
    assert_eq!(selections(&editor, cx), vec![Selection::collapsed(p(1, 3))]);
    assert_eq!(marked(&editor, cx), Some(1..4));
    commit(&editor, "ABC", cx);
    assert_eq!(text(&editor, cx), "xABC");
    assert_eq!(selections(&editor, cx), vec![Selection::collapsed(p(1, 5))]);
}

/// The IME's selected range within the composition is its active clause
/// (Monaco's thick `edit-context-composition-primary` underline); the rest
/// of the composition is underlined thin.
#[gpui::test]
fn composition_active_clause(cx: &mut TestAppContext) {
    let (editor, cx) = setup("a", cx);
    set(&editor, &[Selection::collapsed(p(1, 2))], cx);
    mark(&editor, "にほんご", Some(2..4), cx);
    let composition = editor.read_with(cx, |e, _| e.composition_for_tests());
    assert_eq!(
        composition,
        Some((p(1, 2)..p(1, 6), Some(p(1, 4)..p(1, 6))))
    );
    // A caret (empty selection) is no clause.
    mark(&editor, "にほんご", Some(4..4), cx);
    let composition = editor.read_with(cx, |e, _| e.composition_for_tests());
    assert_eq!(composition, Some((p(1, 2)..p(1, 6), None)));
    commit(&editor, "日本語", cx);
    let composition = editor.read_with(cx, |e, _| e.composition_for_tests());
    assert_eq!(composition, None);
    // It renders.
    cx.update(|window, _| window.refresh());
    cx.run_until_parked();
}

fn js_setup<'a>(
    text: &str,
    cx: &'a mut TestAppContext,
) -> (Entity<CodeEditor>, &'a mut VisualTestContext) {
    use gpui::AppContext as _;
    cx.update(|cx| {
        gpui_monaco::init(cx);
        gpui_monaco::set_language_registry(cx, gpui_monaco_languages::registry());
    });
    let text = text.to_string();
    let (editor, cx) = cx.add_window_view(|window, cx| {
        let model = cx.new(|_| gpui_monaco::TextModel::new(&text).with_language("javascript"));
        CodeEditor::new(model, window, cx)
    });
    cx.update(|window, cx| window.focus(&editor.focus_handle(cx), cx));
    cx.run_until_parked();
    (editor, cx)
}

/// A dead-key quote committed by the IME (US International: `'` then
/// Space) auto-closes like a typed one (Monaco
/// `compositionEndWithInterceptors`), and the closing quote it adds is typed
/// over.
#[gpui::test]
fn dead_key_quote_auto_closes(cx: &mut TestAppContext) {
    let (editor, cx) = js_setup("", cx);
    mark(&editor, "'", None, cx);
    assert_eq!(text(&editor, cx), "'");
    commit(&editor, "'", cx);
    assert_eq!(text(&editor, cx), "''");
    assert_eq!(selections(&editor, cx), vec![Selection::collapsed(p(1, 2))]);
    // Typing the closing quote with the dead key types over it.
    mark(&editor, "'", None, cx);
    commit(&editor, "'", cx);
    assert_eq!(text(&editor, cx), "''");
    assert_eq!(selections(&editor, cx), vec![Selection::collapsed(p(1, 3))]);
}

/// A dead-key quote over a selection surrounds it.
#[gpui::test]
fn dead_key_quote_surrounds_the_selection(cx: &mut TestAppContext) {
    let (editor, cx) = js_setup("test", cx);
    set(
        &editor,
        &[Selection {
            anchor: p(1, 1),
            head: p(1, 5),
        }],
        cx,
    );
    mark(&editor, "'", None, cx);
    assert_eq!(text(&editor, cx), "'");
    commit(&editor, "'", cx);
    assert_eq!(text(&editor, cx), "'test'");
    assert_eq!(
        selections(&editor, cx),
        vec![Selection {
            anchor: p(1, 2),
            head: p(1, 6),
        }]
    );
}

/// A dead key that composes an accent (`'` then `e`) is left alone, and a
/// canceled composition (marking nothing) doesn't auto-close.
#[gpui::test]
fn composed_accents_and_canceled_compositions_are_left_alone(cx: &mut TestAppContext) {
    let (editor, cx) = js_setup("", cx);
    mark(&editor, "'", None, cx);
    commit(&editor, "é", cx);
    assert_eq!(text(&editor, cx), "é");
    mark(&editor, "(", None, cx);
    mark(&editor, "", None, cx);
    assert_eq!(text(&editor, cx), "é");
    assert_eq!(marked(&editor, cx), None);
}

/// With several cursors, the committed dead-key quote auto-closes at each.
#[gpui::test]
fn dead_key_quote_auto_closes_at_every_cursor(cx: &mut TestAppContext) {
    let (editor, cx) = js_setup("a = \nb = ", cx);
    set(
        &editor,
        &[Selection::collapsed(p(1, 5)), Selection::collapsed(p(2, 5))],
        cx,
    );
    mark(&editor, "'", None, cx);
    commit(&editor, "'", cx);
    assert_eq!(text(&editor, cx), "a = ''\nb = ''");
    assert_eq!(
        selections(&editor, cx),
        vec![Selection::collapsed(p(1, 6)), Selection::collapsed(p(2, 6))]
    );
}

/// `unmark_text` ends the composition with the marked text, which then
/// auto-closes.
#[gpui::test]
fn unmark_text_ends_the_composition(cx: &mut TestAppContext) {
    let (editor, cx) = js_setup("", cx);
    mark(&editor, "(", None, cx);
    editor.update_in(cx, |e, window, cx| e.unmark_text(window, cx));
    assert_eq!(text(&editor, cx), "()");
    assert_eq!(marked(&editor, cx), None);
}

fn select(anchor: PointUtf16, head: PointUtf16) -> Selection {
    Selection { anchor, head }
}

/// Monaco `_emitTypeEvent`: inserting text over exactly the primary
/// selection (the Character Viewer, say) has no deltas, so it's a `type`
/// at every selection.
#[gpui::test]
fn replacing_the_primary_selection_types_at_every_selection(cx: &mut TestAppContext) {
    let (editor, cx) = setup("foo foo foo", cx);
    set(
        &editor,
        &[
            select(p(1, 1), p(1, 4)),
            select(p(1, 5), p(1, 8)),
            select(p(1, 9), p(1, 12)),
        ],
        cx,
    );
    editor.update_in(cx, |e, window, cx| {
        e.replace_text_in_range(Some(0..3), "😀", window, cx)
    });
    assert_eq!(text(&editor, cx), "😀 😀 😀");
    assert_eq!(
        selections(&editor, cx),
        vec![
            Selection::collapsed(p(1, 3)),
            Selection::collapsed(p(1, 6)),
            Selection::collapsed(p(1, 9)),
        ]
    );
}

/// The `type` runs the typing interceptors: an opening bracket inserted
/// over a selection surrounds it (`autoSurround`).
#[gpui::test]
fn replacing_the_selection_with_a_bracket_surrounds_it(cx: &mut TestAppContext) {
    cx.update(|cx| {
        gpui_monaco::init(cx);
        gpui_monaco::set_language_registry(cx, gpui_monaco_languages::registry());
    });
    let (editor, cx) = cx.add_window_view(|window, cx| {
        let model = gpui::AppContext::new(cx, |_| {
            gpui_monaco::TextModel::new("foo").with_language("rust")
        });
        CodeEditor::new(model, window, cx)
    });
    cx.update(|window, cx| window.focus(&editor.focus_handle(cx), cx));
    cx.run_until_parked();
    set(&editor, &[select(p(1, 1), p(1, 4))], cx);
    editor.update_in(cx, |e, window, cx| {
        e.replace_text_in_range(Some(0..3), "(", window, cx)
    });
    assert_eq!(text(&editor, cx), "(foo)");
}

/// Press-and-hold with one cursor is a `compositionType` too: a
/// `TypingOther` edit that joins the typing before it, so one undo
/// removes `café`.
#[gpui::test]
fn press_and_hold_accent_joins_the_typing_undo_step(cx: &mut TestAppContext) {
    let (editor, cx) = setup("", cx);
    editor.update(cx, |e, cx| e.type_text("cafe", cx));
    editor.update_in(cx, |e, window, cx| {
        e.replace_text_in_range(Some(3..4), "é", window, cx)
    });
    assert_eq!(text(&editor, cx), "café");
    assert_eq!(selections(&editor, cx), vec![Selection::collapsed(p(1, 5))]);
    editor.update(cx, |e, cx| e.undo(cx));
    assert_eq!(text(&editor, cx), "");
}
