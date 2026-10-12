//! Cursor movement fixes from #333: word moves on model lines under word
//! wrap (`wordOperations.ts`), cursors following edits made elsewhere
//! (Monaco's `sideEditing.test.ts`, `cursor.test.ts` "Independent model
//! edit 1" and #111513) and `stickyTabStops` snapping mouse clicks
//! (`mouseTarget.ts`). The viewport targets of `cursorMove` are tested in
//! `reveal.rs`.

use std::time::Duration;

use gpui::{
    AppContext as _, Entity, Focusable as _, Modifiers, MouseButton, MouseDownEvent, MouseUpEvent,
    TestAppContext, VisualTestContext, point, px,
};
use gpui_monaco::cursor::Selection;
use gpui_monaco::options::WordWrap;
use gpui_monaco::text::PointUtf16;
use gpui_monaco::view::line_breaks::WrappingIndent;
use gpui_monaco::{CodeEditor, EditorOptions, MouseTargetKind, SingleEditOperation, TextModel};
use serde_json::json;

/// Monaco 1-based (line, column).
fn p(line: u32, column: u32) -> PointUtf16 {
    PointUtf16::from_monaco(line, column)
}

/// Monaco `new Selection(selectionStartLine, selectionStartColumn,
/// positionLine, positionColumn)`.
fn sel(start_line: u32, start_column: u32, line: u32, column: u32) -> Selection {
    Selection::new(p(start_line, start_column), p(line, column))
}

fn setup<'a>(
    text: &str,
    options: impl FnOnce(&mut EditorOptions),
    cx: &'a mut TestAppContext,
) -> (Entity<CodeEditor>, &'a mut VisualTestContext) {
    cx.update(gpui_monaco::init);
    let text = text.to_string();
    let (editor, cx) = cx.add_window_view(|window, cx| {
        let model = cx.new(|_| TextModel::new(&text).with_uri("file:///a.txt"));
        CodeEditor::new(model, window, cx)
    });
    editor.update(cx, |e, cx| {
        e.update_options_with(
            |o| {
                o.sticky_scroll.enabled = false;
                o.minimap.enabled = false;
                options(o);
            },
            cx,
        );
    });
    cx.update(|window, cx| window.focus(&editor.focus_handle(cx), cx));
    draw(cx);
    (editor, cx)
}

fn draw(cx: &mut VisualTestContext) {
    cx.update(|window, cx| {
        window.refresh();
        window.draw(cx).clear(cx);
    });
    cx.run_until_parked();
}

fn set(editor: &Entity<CodeEditor>, selections: &[Selection], cx: &mut VisualTestContext) {
    editor.update(cx, |e, cx| e.set_selections(selections, cx));
    draw(cx);
}

fn selections(editor: &Entity<CodeEditor>, cx: &mut VisualTestContext) -> Vec<Selection> {
    editor.read_with(cx, |e, _| e.selections())
}

fn keys(keys: &str, cx: &mut VisualTestContext) {
    cx.simulate_keystrokes(keys);
    draw(cx);
}

fn trigger(editor: &Entity<CodeEditor>, id: &str, cx: &mut VisualTestContext) {
    editor
        .update_in(cx, |e, window, cx| {
            e.trigger_with_payload(id, json!(null), window, cx)
        })
        .unwrap_or_else(|err| panic!("{id}: {err}"));
    draw(cx);
}

// ----- word moves under word wrap (§A.8) -------------------------------------

/// The audit's repro: at 20 columns, `"    foo(alpha, beta, gamma,
/// delta)"` wraps before `beta` and `delta` (Monaco columns 16 and 29),
/// the continuations indented like the line.
const WRAPPED: &str = "    foo(alpha, beta, gamma, delta)";

fn wrapped(cx: &mut TestAppContext) -> (Entity<CodeEditor>, &mut VisualTestContext) {
    setup(
        WRAPPED,
        |o| {
            o.word_wrap = WordWrap::WordWrapColumn;
            o.word_wrap_column = 20;
            o.wrapping_indent = WrappingIndent::Same;
        },
        cx,
    )
}

#[gpui::test]
fn word_moves_cross_wrap_points_on_the_model_line(cx: &mut TestAppContext) {
    let (editor, cx) = wrapped(cx);
    // Down from the start of the line lands on the second segment's start.
    keys("down", cx);
    assert_eq!(selections(&editor, cx), [Selection::collapsed(p(1, 16))]);
    assert_eq!(
        editor.read_with(cx, |e, _| e.cursor_view_positions_for_tests()[0].0),
        1
    );

    // Monaco `cursorWordLeft` runs on the model line: the previous word's
    // start, not the wrap indent.
    keys("alt-left", cx);
    assert_eq!(selections(&editor, cx), [Selection::collapsed(p(1, 9))]);

    // Selecting does too.
    set(&editor, &[Selection::collapsed(p(1, 16))], cx);
    keys("alt-shift-left", cx);
    assert_eq!(selections(&editor, cx), [sel(1, 16, 1, 9)]);

    // Word moves go over wrap points without stopping.
    set(&editor, &[Selection::collapsed(p(1, 17))], cx);
    trigger(&editor, "cursorWordEndRight", cx);
    assert_eq!(selections(&editor, cx), [Selection::collapsed(p(1, 20))]);
    set(&editor, &[Selection::collapsed(p(1, 28))], cx);
    trigger(&editor, "cursorWordEndRight", cx);
    assert_eq!(selections(&editor, cx), [Selection::collapsed(p(1, 34))]);
    trigger(&editor, "cursorWordStartLeft", cx);
    assert_eq!(selections(&editor, cx), [Selection::collapsed(p(1, 29))]);
    trigger(&editor, "cursorWordStartLeft", cx);
    assert_eq!(selections(&editor, cx), [Selection::collapsed(p(1, 27))]);
}

/// The same moves give the same model positions with and without wrapping.
#[gpui::test]
fn word_moves_ignore_wrapping(cx: &mut TestAppContext) {
    let text = format!("{WRAPPED}\n{WRAPPED}");
    let (editor, cx) = setup(&text, |_| {}, cx);
    let commands = [
        "cursorWordLeft",
        "cursorWordRight",
        "cursorWordStartLeft",
        "cursorWordStartRight",
        "cursorWordEndLeft",
        "cursorWordEndRight",
        "cursorWordPartLeft",
        "cursorWordPartRight",
    ];
    let run = |editor: &Entity<CodeEditor>, cx: &mut VisualTestContext| {
        let mut results = Vec::new();
        for command in commands {
            for column in 0..=WRAPPED.len() as u32 {
                set(
                    editor,
                    &[Selection::collapsed(PointUtf16::new(0, column))],
                    cx,
                );
                trigger(editor, command, cx);
                results.push(selections(editor, cx));
            }
        }
        results
    };
    let unwrapped = run(&editor, cx);
    editor.update(cx, |e, cx| {
        e.update_options_with(
            |o| {
                o.word_wrap = WordWrap::WordWrapColumn;
                o.word_wrap_column = 12;
                o.wrapping_indent = WrappingIndent::Indent;
            },
            cx,
        )
    });
    draw(cx);
    assert_eq!(run(&editor, cx), unwrapped);
}

// ----- edits from elsewhere (§A.12) --------------------------------------------

/// Monaco `sideEditing.test.ts` `testCommand`: sets `selections`, applies
/// `edits` to the model (Monaco `model.applyEdits`) and checks the text and
/// where the cursors went.
fn test_command(
    lines: &[&str],
    selections_before: &[Selection],
    edits: Vec<SingleEditOperation>,
    expected_lines: &[&str],
    expected_selections: &[Selection],
    cx: &mut TestAppContext,
) {
    let (editor, cx) = setup(&lines.join("\n"), |_| {}, cx);
    set(&editor, selections_before, cx);
    let model = editor.read_with(cx, |e, _| e.model().clone());
    model
        .update(cx, |model, cx| model.apply_edit_operations(edits, cx))
        .unwrap();
    draw(cx);
    assert_eq!(
        model.read_with(cx, |m, _| m.text()),
        expected_lines.join("\n")
    );
    assert_eq!(selections(&editor, cx), expected_selections);
}

fn replace(range: (u32, u32, u32, u32), text: &str) -> SingleEditOperation {
    SingleEditOperation::new(p(range.0, range.1)..p(range.2, range.3), text)
}

#[gpui::test]
fn side_editing_replace_at_selection(cx: &mut TestAppContext) {
    test_command(
        &["first", "second line", "third line", "fourth"],
        &[sel(1, 1, 1, 1)],
        vec![replace((1, 1, 1, 1), "something ")],
        &["something first", "second line", "third line", "fourth"],
        &[sel(1, 11, 1, 11)],
        cx,
    );
}

#[gpui::test]
fn side_editing_replace_at_selection_2(cx: &mut TestAppContext) {
    test_command(
        &["first", "second line", "third line", "fourth"],
        &[sel(1, 1, 1, 6)],
        vec![replace((1, 1, 1, 6), "something")],
        &["something", "second line", "third line", "fourth"],
        &[sel(1, 1, 1, 10)],
        cx,
    );
}

#[gpui::test]
fn side_editing_insert_at_selection(cx: &mut TestAppContext) {
    test_command(
        &["first", "second line", "third line", "fourth"],
        &[sel(1, 1, 1, 1)],
        vec![replace((1, 1, 1, 1), "something ")],
        &["something first", "second line", "third line", "fourth"],
        &[sel(1, 11, 1, 11)],
        cx,
    );
}

#[gpui::test]
fn side_editing_insert_at_selection_sitting_on_max_column(cx: &mut TestAppContext) {
    test_command(
        &["first", "second line", "third line", "fourth"],
        &[sel(1, 6, 1, 6)],
        vec![replace((1, 6, 1, 6), " something\nnew ")],
        &[
            "first something",
            "new ",
            "second line",
            "third line",
            "fourth",
        ],
        &[sel(2, 5, 2, 5)],
        cx,
    );
}

#[gpui::test]
fn side_editing_issue_3994_replace_on_top_of_selection(cx: &mut TestAppContext) {
    test_command(
        &["$obj = New-Object \"system.col\""],
        &[sel(1, 30, 1, 30)],
        vec![replace((1, 19, 1, 31), "\"System.Collections\"").force_move_markers()],
        &["$obj = New-Object \"System.Collections\""],
        &[sel(1, 39, 1, 39)],
        cx,
    );
}

#[gpui::test]
fn side_editing_issue_15267_suggestion_that_adds_a_line(cx: &mut TestAppContext) {
    test_command(
        &[
            "package main",
            "",
            "import (",
            "\t\"fmt\"",
            ")",
            "",
            "func main(",
            "\tfmt.Println(strings.Con)",
            "}",
        ],
        &[sel(8, 25, 8, 25)],
        vec![replace((5, 1, 5, 1), "\t\"strings\"\n").force_move_markers()],
        &[
            "package main",
            "",
            "import (",
            "\t\"fmt\"",
            "\t\"strings\"",
            ")",
            "",
            "func main(",
            "\tfmt.Println(strings.Con)",
            "}",
        ],
        &[sel(9, 25, 9, 25)],
        cx,
    );
}

#[gpui::test]
fn side_editing_issue_15236_selections_broke_after_deleting_text(cx: &mut TestAppContext) {
    test_command(
        &["foofoofoo, foofoofoo, bar"],
        &[sel(1, 1, 1, 10), sel(1, 12, 1, 21)],
        vec![replace((1, 1, 1, 10), ""), replace((1, 12, 1, 21), "")],
        &[", , bar"],
        &[sel(1, 1, 1, 1), sel(1, 3, 1, 3)],
        cx,
    );
}

type Range4 = (u32, u32, u32, u32);

/// One `SideEditing` case: the selection's range, the edit, and the
/// expected selections `[[ltr, rtl] without forceMoveMarkers, [ltr, rtl]
/// with]`.
struct SideEdit {
    name: &'static str,
    selection: Range4,
    edit: Range4,
    text: &'static str,
    expected: [[Range4; 2]; 2],
}

/// Monaco `sideEditing.test.ts` suite `SideEditing` `runTest`: each case
/// with the selection left to right and right to left, without and with
/// `forceMoveMarkers`. Expected ranges are Monaco `Selection`s.
fn run_side_edits(cases: &[SideEdit], cx: &mut TestAppContext) {
    const LINES: &str = "My First Line\nMy Second Line\nThird Line";
    let (editor, cx) = setup(LINES, |_| {}, cx);
    let model = editor.read_with(cx, |e, _| e.model().clone());
    for case in cases {
        let (sl, sc, el, ec) = case.selection;
        for (force, expected) in [false, true].into_iter().zip(case.expected) {
            for (rtl, expected) in [false, true].into_iter().zip(expected) {
                model.update(cx, |m, cx| m.set_value(LINES, cx));
                let selection = if rtl {
                    sel(el, ec, sl, sc)
                } else {
                    sel(sl, sc, el, ec)
                };
                set(&editor, &[selection], cx);
                let mut edit = replace(case.edit, case.text);
                edit.force_move_markers = force;
                model
                    .update(cx, |m, cx| m.apply_edit_operations(vec![edit], cx))
                    .unwrap();
                draw(cx);
                let (a, b, c, d) = expected;
                assert_eq!(
                    selections(&editor, cx),
                    [sel(a, b, c, d)],
                    "{}: {}-{}",
                    case.name,
                    if force { "force" } else { "no-force" },
                    if rtl { "inverse" } else { "regular" },
                );
            }
        }
    }
}

macro_rules! side_edits {
    ($( $name:literal: $selection:expr, $edit:expr, $text:literal => [[$a:expr, $b:expr], [$c:expr, $d:expr]]; )*) => {
        &[$(SideEdit {
            name: $name,
            selection: $selection,
            edit: $edit,
            text: $text,
            expected: [[$a, $b], [$c, $d]],
        }),*]
    };
}

#[gpui::test]
fn side_editing_insert(cx: &mut TestAppContext) {
    run_side_edits(
        side_edits! {
            "collapsed before": (1, 4, 1, 4), (1, 3, 1, 3), "xx" => [[(1, 6, 1, 6), (1, 6, 1, 6)], [(1, 6, 1, 6), (1, 6, 1, 6)]];
            "collapsed equal": (1, 4, 1, 4), (1, 4, 1, 4), "xx" => [[(1, 6, 1, 6), (1, 6, 1, 6)], [(1, 6, 1, 6), (1, 6, 1, 6)]];
            "collapsed after": (1, 4, 1, 4), (1, 5, 1, 5), "xx" => [[(1, 4, 1, 4), (1, 4, 1, 4)], [(1, 4, 1, 4), (1, 4, 1, 4)]];
            "non-collapsed before": (1, 4, 1, 9), (1, 3, 1, 3), "xx" => [[(1, 6, 1, 11), (1, 11, 1, 6)], [(1, 6, 1, 11), (1, 11, 1, 6)]];
            "non-collapsed start": (1, 4, 1, 9), (1, 4, 1, 4), "xx" => [[(1, 4, 1, 11), (1, 11, 1, 4)], [(1, 6, 1, 11), (1, 11, 1, 6)]];
            "non-collapsed inside": (1, 4, 1, 9), (1, 5, 1, 5), "xx" => [[(1, 4, 1, 11), (1, 11, 1, 4)], [(1, 4, 1, 11), (1, 11, 1, 4)]];
            "non-collapsed end": (1, 4, 1, 9), (1, 9, 1, 9), "xx" => [[(1, 4, 1, 11), (1, 11, 1, 4)], [(1, 4, 1, 11), (1, 11, 1, 4)]];
            "non-collapsed after": (1, 4, 1, 9), (1, 10, 1, 10), "xx" => [[(1, 4, 1, 9), (1, 9, 1, 4)], [(1, 4, 1, 9), (1, 9, 1, 4)]];
        },
        cx,
    );
}

#[gpui::test]
fn side_editing_delete(cx: &mut TestAppContext) {
    run_side_edits(
        side_edits! {
            "collapsed edit.end < range.start": (1, 4, 1, 4), (1, 1, 1, 3), "" => [[(1, 2, 1, 2), (1, 2, 1, 2)], [(1, 2, 1, 2), (1, 2, 1, 2)]];
            "collapsed edit.end <= range.start": (1, 4, 1, 4), (1, 2, 1, 4), "" => [[(1, 2, 1, 2), (1, 2, 1, 2)], [(1, 2, 1, 2), (1, 2, 1, 2)]];
            "collapsed edit.start < range.start && edit.end > range.end": (1, 4, 1, 4), (1, 3, 1, 5), "" => [[(1, 3, 1, 3), (1, 3, 1, 3)], [(1, 3, 1, 3), (1, 3, 1, 3)]];
            "collapsed edit.start >= range.end": (1, 4, 1, 4), (1, 4, 1, 6), "" => [[(1, 4, 1, 4), (1, 4, 1, 4)], [(1, 4, 1, 4), (1, 4, 1, 4)]];
            "collapsed edit.start > range.end": (1, 4, 1, 4), (1, 5, 1, 7), "" => [[(1, 4, 1, 4), (1, 4, 1, 4)], [(1, 4, 1, 4), (1, 4, 1, 4)]];
            "edit.end < range.start": (1, 4, 1, 9), (1, 1, 1, 3), "" => [[(1, 2, 1, 7), (1, 7, 1, 2)], [(1, 2, 1, 7), (1, 7, 1, 2)]];
            "edit.end <= range.start": (1, 4, 1, 9), (1, 2, 1, 4), "" => [[(1, 2, 1, 7), (1, 7, 1, 2)], [(1, 2, 1, 7), (1, 7, 1, 2)]];
            "edit.start < range.start && edit.end < range.end": (1, 4, 1, 9), (1, 3, 1, 5), "" => [[(1, 3, 1, 7), (1, 7, 1, 3)], [(1, 3, 1, 7), (1, 7, 1, 3)]];
            "edit.start < range.start && edit.end == range.end": (1, 4, 1, 9), (1, 3, 1, 9), "" => [[(1, 3, 1, 3), (1, 3, 1, 3)], [(1, 3, 1, 3), (1, 3, 1, 3)]];
            "edit.start < range.start && edit.end > range.end": (1, 4, 1, 9), (1, 3, 1, 10), "" => [[(1, 3, 1, 3), (1, 3, 1, 3)], [(1, 3, 1, 3), (1, 3, 1, 3)]];
            "edit.start == range.start && edit.end < range.end": (1, 4, 1, 9), (1, 4, 1, 6), "" => [[(1, 4, 1, 7), (1, 7, 1, 4)], [(1, 4, 1, 7), (1, 7, 1, 4)]];
            "edit.start == range.start && edit.end == range.end": (1, 4, 1, 9), (1, 4, 1, 9), "" => [[(1, 4, 1, 4), (1, 4, 1, 4)], [(1, 4, 1, 4), (1, 4, 1, 4)]];
            "edit.start == range.start && edit.end > range.end": (1, 4, 1, 9), (1, 4, 1, 10), "" => [[(1, 4, 1, 4), (1, 4, 1, 4)], [(1, 4, 1, 4), (1, 4, 1, 4)]];
            "edit.start > range.start && edit.start < range.end && edit.end < range.end": (1, 4, 1, 9), (1, 5, 1, 7), "" => [[(1, 4, 1, 7), (1, 7, 1, 4)], [(1, 4, 1, 7), (1, 7, 1, 4)]];
            "edit.start > range.start && edit.start < range.end && edit.end == range.end": (1, 4, 1, 9), (1, 5, 1, 9), "" => [[(1, 4, 1, 5), (1, 5, 1, 4)], [(1, 4, 1, 5), (1, 5, 1, 4)]];
            "edit.start > range.start && edit.start < range.end && edit.end > range.end": (1, 4, 1, 9), (1, 5, 1, 10), "" => [[(1, 4, 1, 5), (1, 5, 1, 4)], [(1, 4, 1, 5), (1, 5, 1, 4)]];
            "edit.start == range.end": (1, 4, 1, 9), (1, 9, 1, 11), "" => [[(1, 4, 1, 9), (1, 9, 1, 4)], [(1, 4, 1, 9), (1, 9, 1, 4)]];
            "edit.start > range.end": (1, 4, 1, 9), (1, 10, 1, 11), "" => [[(1, 4, 1, 9), (1, 9, 1, 4)], [(1, 4, 1, 9), (1, 9, 1, 4)]];
        },
        cx,
    );
}

#[gpui::test]
fn side_editing_replace_short(cx: &mut TestAppContext) {
    run_side_edits(
        side_edits! {
            "collapsed edit.end < range.start": (1, 4, 1, 4), (1, 1, 1, 3), "c" => [[(1, 3, 1, 3), (1, 3, 1, 3)], [(1, 3, 1, 3), (1, 3, 1, 3)]];
            "collapsed edit.end <= range.start": (1, 4, 1, 4), (1, 2, 1, 4), "c" => [[(1, 3, 1, 3), (1, 3, 1, 3)], [(1, 3, 1, 3), (1, 3, 1, 3)]];
            "collapsed edit.start < range.start && edit.end > range.end": (1, 4, 1, 4), (1, 3, 1, 5), "c" => [[(1, 4, 1, 4), (1, 4, 1, 4)], [(1, 4, 1, 4), (1, 4, 1, 4)]];
            "collapsed edit.start >= range.end": (1, 4, 1, 4), (1, 4, 1, 6), "c" => [[(1, 4, 1, 4), (1, 4, 1, 4)], [(1, 5, 1, 5), (1, 5, 1, 5)]];
            "collapsed edit.start > range.end": (1, 4, 1, 4), (1, 5, 1, 7), "c" => [[(1, 4, 1, 4), (1, 4, 1, 4)], [(1, 4, 1, 4), (1, 4, 1, 4)]];
            "edit.end < range.start": (1, 4, 1, 9), (1, 1, 1, 3), "c" => [[(1, 3, 1, 8), (1, 8, 1, 3)], [(1, 3, 1, 8), (1, 8, 1, 3)]];
            "edit.end <= range.start": (1, 4, 1, 9), (1, 2, 1, 4), "c" => [[(1, 3, 1, 8), (1, 8, 1, 3)], [(1, 3, 1, 8), (1, 8, 1, 3)]];
            "edit.start < range.start && edit.end < range.end": (1, 4, 1, 9), (1, 3, 1, 5), "c" => [[(1, 4, 1, 8), (1, 8, 1, 4)], [(1, 4, 1, 8), (1, 8, 1, 4)]];
            "edit.start < range.start && edit.end == range.end": (1, 4, 1, 9), (1, 3, 1, 9), "c" => [[(1, 4, 1, 4), (1, 4, 1, 4)], [(1, 4, 1, 4), (1, 4, 1, 4)]];
            "edit.start < range.start && edit.end > range.end": (1, 4, 1, 9), (1, 3, 1, 10), "c" => [[(1, 4, 1, 4), (1, 4, 1, 4)], [(1, 4, 1, 4), (1, 4, 1, 4)]];
            "edit.start == range.start && edit.end < range.end": (1, 4, 1, 9), (1, 4, 1, 6), "c" => [[(1, 4, 1, 8), (1, 8, 1, 4)], [(1, 5, 1, 8), (1, 8, 1, 5)]];
            "edit.start == range.start && edit.end == range.end": (1, 4, 1, 9), (1, 4, 1, 9), "c" => [[(1, 4, 1, 5), (1, 5, 1, 4)], [(1, 5, 1, 5), (1, 5, 1, 5)]];
            "edit.start == range.start && edit.end > range.end": (1, 4, 1, 9), (1, 4, 1, 10), "c" => [[(1, 4, 1, 5), (1, 5, 1, 4)], [(1, 5, 1, 5), (1, 5, 1, 5)]];
            "edit.start > range.start && edit.start < range.end && edit.end < range.end": (1, 4, 1, 9), (1, 5, 1, 7), "c" => [[(1, 4, 1, 8), (1, 8, 1, 4)], [(1, 4, 1, 8), (1, 8, 1, 4)]];
            "edit.start > range.start && edit.start < range.end && edit.end == range.end": (1, 4, 1, 9), (1, 5, 1, 9), "c" => [[(1, 4, 1, 6), (1, 6, 1, 4)], [(1, 4, 1, 6), (1, 6, 1, 4)]];
            "edit.start > range.start && edit.start < range.end && edit.end > range.end": (1, 4, 1, 9), (1, 5, 1, 10), "c" => [[(1, 4, 1, 6), (1, 6, 1, 4)], [(1, 4, 1, 6), (1, 6, 1, 4)]];
            "edit.start == range.end": (1, 4, 1, 9), (1, 9, 1, 11), "c" => [[(1, 4, 1, 9), (1, 9, 1, 4)], [(1, 4, 1, 10), (1, 10, 1, 4)]];
            "edit.start > range.end": (1, 4, 1, 9), (1, 10, 1, 11), "c" => [[(1, 4, 1, 9), (1, 9, 1, 4)], [(1, 4, 1, 9), (1, 9, 1, 4)]];
        },
        cx,
    );
}

#[gpui::test]
fn side_editing_replace_long(cx: &mut TestAppContext) {
    run_side_edits(
        side_edits! {
            "collapsed edit.end < range.start": (1, 4, 1, 4), (1, 1, 1, 3), "cccc" => [[(1, 6, 1, 6), (1, 6, 1, 6)], [(1, 6, 1, 6), (1, 6, 1, 6)]];
            "collapsed edit.end <= range.start": (1, 4, 1, 4), (1, 2, 1, 4), "cccc" => [[(1, 6, 1, 6), (1, 6, 1, 6)], [(1, 6, 1, 6), (1, 6, 1, 6)]];
            "collapsed edit.start < range.start && edit.end > range.end": (1, 4, 1, 4), (1, 3, 1, 5), "cccc" => [[(1, 4, 1, 4), (1, 4, 1, 4)], [(1, 7, 1, 7), (1, 7, 1, 7)]];
            "collapsed edit.start >= range.end": (1, 4, 1, 4), (1, 4, 1, 6), "cccc" => [[(1, 4, 1, 4), (1, 4, 1, 4)], [(1, 8, 1, 8), (1, 8, 1, 8)]];
            "collapsed edit.start > range.end": (1, 4, 1, 4), (1, 5, 1, 7), "cccc" => [[(1, 4, 1, 4), (1, 4, 1, 4)], [(1, 4, 1, 4), (1, 4, 1, 4)]];
            "edit.end < range.start": (1, 4, 1, 9), (1, 1, 1, 3), "cccc" => [[(1, 6, 1, 11), (1, 11, 1, 6)], [(1, 6, 1, 11), (1, 11, 1, 6)]];
            "edit.end <= range.start": (1, 4, 1, 9), (1, 2, 1, 4), "cccc" => [[(1, 4, 1, 11), (1, 11, 1, 4)], [(1, 6, 1, 11), (1, 11, 1, 6)]];
            "edit.start < range.start && edit.end < range.end": (1, 4, 1, 9), (1, 3, 1, 5), "cccc" => [[(1, 4, 1, 11), (1, 11, 1, 4)], [(1, 7, 1, 11), (1, 11, 1, 7)]];
            "edit.start < range.start && edit.end == range.end": (1, 4, 1, 9), (1, 3, 1, 9), "cccc" => [[(1, 4, 1, 7), (1, 7, 1, 4)], [(1, 7, 1, 7), (1, 7, 1, 7)]];
            "edit.start < range.start && edit.end > range.end": (1, 4, 1, 9), (1, 3, 1, 10), "cccc" => [[(1, 4, 1, 7), (1, 7, 1, 4)], [(1, 7, 1, 7), (1, 7, 1, 7)]];
            "edit.start == range.start && edit.end < range.end": (1, 4, 1, 9), (1, 4, 1, 6), "cccc" => [[(1, 4, 1, 11), (1, 11, 1, 4)], [(1, 8, 1, 11), (1, 11, 1, 8)]];
            "edit.start == range.start && edit.end == range.end": (1, 4, 1, 9), (1, 4, 1, 9), "cccc" => [[(1, 4, 1, 8), (1, 8, 1, 4)], [(1, 8, 1, 8), (1, 8, 1, 8)]];
            "edit.start == range.start && edit.end > range.end": (1, 4, 1, 9), (1, 4, 1, 10), "cccc" => [[(1, 4, 1, 8), (1, 8, 1, 4)], [(1, 8, 1, 8), (1, 8, 1, 8)]];
            "edit.start > range.start && edit.start < range.end && edit.end < range.end": (1, 4, 1, 9), (1, 5, 1, 7), "cccc" => [[(1, 4, 1, 11), (1, 11, 1, 4)], [(1, 4, 1, 11), (1, 11, 1, 4)]];
            "edit.start > range.start && edit.start < range.end && edit.end == range.end": (1, 4, 1, 9), (1, 5, 1, 9), "cccc" => [[(1, 4, 1, 9), (1, 9, 1, 4)], [(1, 4, 1, 9), (1, 9, 1, 4)]];
            "edit.start > range.start && edit.start < range.end && edit.end > range.end": (1, 4, 1, 9), (1, 5, 1, 10), "cccc" => [[(1, 4, 1, 9), (1, 9, 1, 4)], [(1, 4, 1, 9), (1, 9, 1, 4)]];
            "edit.start == range.end": (1, 4, 1, 9), (1, 9, 1, 11), "cccc" => [[(1, 4, 1, 9), (1, 9, 1, 4)], [(1, 4, 1, 13), (1, 13, 1, 4)]];
            "edit.start > range.end": (1, 4, 1, 9), (1, 10, 1, 11), "cccc" => [[(1, 4, 1, 9), (1, 9, 1, 4)], [(1, 4, 1, 9), (1, 9, 1, 4)]];
        },
        cx,
    );
}

/// Monaco `cursor.test.ts` "Independent model edit 1".
#[gpui::test]
fn independent_model_edit_1(cx: &mut TestAppContext) {
    let text = "    \tMy First Line\t \n\tMy Second Line\n    Third Line🐶\n\n1";
    let (editor, cx) = setup(text, |_| {}, cx);
    set(&editor, &[sel(1, 1, 2, 16)], cx);
    let model = editor.read_with(cx, |e, _| e.model().clone());
    model
        .update(cx, |m, cx| {
            m.apply_edits(vec![(p(2, 1)..p(2, 2), String::new())], cx)
        })
        .unwrap();
    draw(cx);
    assert_eq!(selections(&editor, cx), [sel(1, 1, 2, 15)]);
}

/// Monaco `cursor.test.ts` "issue #111513: Text gets automatically
/// selected when typing at the same location in another editor": typing
/// at another editor's cursor moves it past the text, unselected.
#[gpui::test]
fn issue_111513_typing_at_another_editors_cursor(cx: &mut TestAppContext) {
    let (editor1, cx) = setup("just\n\nsome text", |_| {}, cx);
    let model = editor1.read_with(cx, |e, _| e.model().clone());
    let editor2 = cx.update(|window, cx| cx.new(|cx| CodeEditor::new(model.clone(), window, cx)));
    set(&editor1, &[Selection::collapsed(p(2, 1))], cx);
    set(&editor2, &[Selection::collapsed(p(2, 1))], cx);
    editor2.update(cx, |e, cx| e.type_text("e", cx));
    draw(cx);
    assert_eq!(model.read_with(cx, |m, _| m.text()), "just\ne\nsome text");
    assert_eq!(selections(&editor2, cx), [Selection::collapsed(p(2, 2))]);
    assert_eq!(selections(&editor1, cx), [Selection::collapsed(p(2, 2))]);
}

// ----- stickyTabStops and the mouse (§A.13) ----------------------------------

/// A single click on `at`'s character, just right of its left edge.
fn click(editor: &Entity<CodeEditor>, at: PointUtf16, cx: &mut VisualTestContext) {
    // Not a double click on the same snapped position.
    cx.executor().advance_clock(Duration::from_secs(1));
    let origin = editor
        .read_with(cx, |e, _| e.scrolled_visible_position(at))
        .unwrap();
    let target = point(origin.x + px(1.), origin.y + px(2.));
    let modifiers = Modifiers::default();
    cx.simulate_event(MouseDownEvent {
        position: target,
        button: MouseButton::Left,
        modifiers,
        click_count: 1,
        first_mouse: false,
    });
    cx.simulate_event(MouseUpEvent {
        position: target,
        button: MouseButton::Left,
        modifiers,
        click_count: 1,
    });
    draw(cx);
}

/// Monaco `MouseTargetFactory._snapToSoftTabBoundary`: with
/// `stickyTabStops`, a click in soft-tab indentation lands on the nearest
/// tab stop.
#[gpui::test]
fn sticky_tab_stops_snap_clicks_in_indentation(cx: &mut TestAppContext) {
    let (editor, cx) = setup("        foo\n      bar", |o| o.sticky_tab_stops = true, cx);
    let model = editor.read_with(cx, |e, _| e.model().clone());
    model.update(cx, |model, cx| {
        let options = gpui_monaco::ModelOptions {
            tab_size: 4,
            ..model.options().clone()
        };
        model.update_options(options, cx);
    });
    draw(cx);
    let column = |editor: &Entity<CodeEditor>, cx: &mut VisualTestContext| {
        selections(editor, cx)[0].head.column
    };
    // Monaco `Direction.Nearest` (tab size 4): columns 1 and 2 go to 0, 3
    // to 4, 5 and 6 to 4, 7 to 8.
    for (clicked, snapped) in [(1, 0), (2, 0), (3, 4), (4, 4), (5, 4), (6, 4), (7, 8)] {
        click(&editor, PointUtf16::new(0, clicked), cx);
        assert_eq!(column(&editor, cx), snapped, "click at {clicked}");
    }
    // In the text, or in indentation that isn't whole tab stops, nothing
    // snaps.
    click(&editor, PointUtf16::new(0, 9), cx);
    assert_eq!(column(&editor, cx), 9);
    click(&editor, PointUtf16::new(1, 5), cx);
    assert_eq!(column(&editor, cx), 5);

    // The mouse target says so too, its range spanning both positions.
    let at = editor
        .read_with(cx, |e, _| {
            e.scrolled_visible_position(PointUtf16::new(0, 1))
        })
        .unwrap();
    let target = editor.update_in(cx, |e, window, cx| {
        e.mouse_target(point(at.x + px(1.), at.y + px(2.)), window, cx)
    });
    assert_eq!(target.kind, MouseTargetKind::ContentText);
    assert_eq!(target.position, Some(PointUtf16::new(0, 0)));
    assert_eq!(
        target.range,
        Some(PointUtf16::new(0, 0)..PointUtf16::new(0, 1))
    );

    // Off, clicks land where they are.
    editor.update(cx, |e, cx| {
        e.update_options_with(|o| o.sticky_tab_stops = false, cx)
    });
    draw(cx);
    click(&editor, PointUtf16::new(0, 1), cx);
    assert_eq!(column(&editor, cx), 1);
}
