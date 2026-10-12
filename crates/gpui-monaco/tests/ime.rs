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

/// A replacement away from the primary cursor isn't a `compositionType`:
/// it replaces that range only.
#[gpui::test]
fn replacing_text_away_from_the_cursors_replaces_it_once(cx: &mut TestAppContext) {
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
abc"
    );
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
