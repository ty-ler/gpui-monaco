//! Find model (Monaco `FindModelBoundToEditorModel`): a port of Monaco's
//! `contrib/find/test/browser/findModel.test.ts`.
//!
//! Positions in the assertions are Monaco's 1-based `[startLine,
//! startColumn, endLine, endColumn]` arrays, copied verbatim from the
//! TypeScript; the helpers convert to and from our 0-based `PointUtf16`.
//!
//! Monaco runs each `findTest` twice (on a model built from lines and on one
//! built from piece-tree chunks); the buffer construction is irrelevant here,
//! so each test runs once.

use std::ops::Range;

use gpui::{AppContext as _, Entity, TestAppContext, VisualTestContext};
use gpui_monaco::cursor::Selection;
use gpui_monaco::text::PointUtf16;
use gpui_monaco::{CodeEditor, FindOptions, MATCHES_LIMIT, TextModel};

const TEXT: &[&str] = &[
    "// my cool header",
    "#include \"cool.h\"",
    "#include <iostream>",
    "",
    "int main() {",
    "    cout << \"hello world, Hello!\" << endl;",
    "    cout << \"hello world again\" << endl;",
    "    cout << \"Hello world again\" << endl;",
    "    cout << \"helloworld again\" << endl;",
    "}",
    "// blablablaciao",
    "",
];

type Rng = [u32; 4];

fn setup_text(
    text: String,
    cx: &mut TestAppContext,
) -> (Entity<CodeEditor>, &mut VisualTestContext) {
    cx.update(gpui_monaco::init);
    let (editor, cx) = cx.add_window_view(|window, cx| {
        let model = cx.new(|_| TextModel::new(&text));
        CodeEditor::new(model, window, cx)
    });
    cx.run_until_parked();
    (editor, cx)
}

/// Monaco's `findTest` editor.
fn setup(cx: &mut TestAppContext) -> (Entity<CodeEditor>, &mut VisualTestContext) {
    setup_text(TEXT.join("\n"), cx)
}

/// Monaco (line, column) → ours.
fn pt(line: u32, column: u32) -> PointUtf16 {
    PointUtf16::new(line - 1, column - 1)
}

fn range(r: Rng) -> Range<PointUtf16> {
    pt(r[0], r[1])..pt(r[2], r[3])
}

/// Monaco `fromRange`.
fn from_range(r: &Range<PointUtf16>) -> Rng {
    [
        r.start.row + 1,
        r.start.column + 1,
        r.end.row + 1,
        r.end.column + 1,
    ]
}

/// Monaco `assertFindState`.
#[track_caller]
fn assert_find_state(
    editor: &Entity<CodeEditor>,
    cx: &mut VisualTestContext,
    cursor: Rng,
    highlighted: Option<Rng>,
    find_decorations: &[Rng],
) {
    let (selection, (current, all, _, _)) = editor.read_with(cx, |e, cx| {
        let s = &e.selections()[0];
        (s.start()..s.end(), e.find_state_for_tests(cx))
    });
    assert_eq!(from_range(&selection), cursor, "cursor");
    let actual = (
        current.as_ref().map(from_range),
        all.iter().map(from_range).collect::<Vec<_>>(),
    );
    assert_eq!(actual, (highlighted, find_decorations.to_vec()), "state");
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
}

/// `findState.change({ searchScope }, …)`.
fn set_scope(
    editor: &Entity<CodeEditor>,
    cx: &mut VisualTestContext,
    scope: Option<&[Rng]>,
    move_cursor: bool,
) {
    let scope = scope.map(|s| s.iter().copied().map(range).collect());
    editor.update(cx, |e, cx| e.set_find_search_scope(scope, move_cursor, cx));
}

/// `findState.change({ loop }, …)`.
fn set_loop(editor: &Entity<CodeEditor>, cx: &mut VisualTestContext, loop_: bool) {
    editor.update(cx, |e, cx| {
        let mut options = e.options().clone();
        options.find.loop_ = loop_;
        e.update_options(options, cx);
    });
}

/// `new FindModelBoundToEditorModel(editor, findState)`.
fn start(editor: &Entity<CodeEditor>, cx: &mut VisualTestContext) {
    editor.update(cx, |e, cx| e.start_find_model_for_tests(cx));
}

fn move_to_next_match(editor: &Entity<CodeEditor>, cx: &mut VisualTestContext) {
    editor.update(cx, |e, cx| e.find_next(cx));
}

fn move_to_prev_match(editor: &Entity<CodeEditor>, cx: &mut VisualTestContext) {
    editor.update(cx, |e, cx| e.find_previous(cx));
}

fn replace(editor: &Entity<CodeEditor>, cx: &mut VisualTestContext) {
    editor.update(cx, |e, cx| e.replace_one(cx));
}

fn replace_all(editor: &Entity<CodeEditor>, cx: &mut VisualTestContext) {
    editor.update(cx, |e, cx| e.replace_all(cx));
}

fn select_all_matches(editor: &Entity<CodeEditor>, cx: &mut VisualTestContext) {
    editor.update(cx, |e, cx| e.select_all_matches(cx));
}

/// `editor.trigger('mouse', CoreNavigationCommands.MoveTo.id, { position })`.
fn mouse_move_to(editor: &Entity<CodeEditor>, cx: &mut VisualTestContext, line: u32, column: u32) {
    let p = pt(line, column);
    editor.update(cx, |e, cx| {
        e.set_selections_explicit_for_tests(&[Selection::collapsed(p)], cx)
    });
}

/// `editor.setSelection(...)` / `editor.setPosition(...)` (an API call):
/// `[selectionStartLine, selectionStartColumn, positionLine, positionColumn]`.
fn set_selection(editor: &Entity<CodeEditor>, cx: &mut VisualTestContext, s: Rng) {
    let selection = Selection::new(pt(s[0], s[1]), pt(s[2], s[3]));
    editor.update(cx, |e, cx| e.set_selections(&[selection], cx));
}

/// `editor.getSelections()` as Monaco `Selection`s (anchor, then head).
fn selections(editor: &Entity<CodeEditor>, cx: &mut VisualTestContext) -> Vec<Rng> {
    editor.read_with(cx, |e, _| {
        e.selections()
            .iter()
            .map(|s| {
                [
                    s.anchor.row + 1,
                    s.anchor.column + 1,
                    s.head.row + 1,
                    s.head.column + 1,
                ]
            })
            .collect()
    })
}

/// `editor.getModel().getLineContent(line)`.
fn line_content(editor: &Entity<CodeEditor>, cx: &mut VisualTestContext, line: u32) -> String {
    editor.read_with(cx, |e, cx| e.model().read(cx).line_content(line - 1))
}

/// `findState.matchesCount`.
fn matches_count(editor: &Entity<CodeEditor>, cx: &mut VisualTestContext) -> usize {
    editor.read_with(cx, |e, cx| e.find_state_for_tests(cx).3)
}

/// `findState.matchesPosition`.
fn matches_position(editor: &Entity<CodeEditor>, cx: &mut VisualTestContext) -> usize {
    editor.read_with(cx, |e, cx| e.find_state_for_tests(cx).2)
}

/// `findState.canNavigateForward()` and `canNavigateBack()`, which
/// `findState.ts` derives from the match position and count and `loop`.
fn can_navigate(editor: &Entity<CodeEditor>, cx: &mut VisualTestContext) -> (bool, bool) {
    let (position, count, loop_) = editor.read_with(cx, |e, cx| {
        let (_, _, position, count) = e.find_state_for_tests(cx);
        (position, count, e.options().find.loop_)
    });
    let in_loop = loop_ || count >= MATCHES_LIMIT;
    (in_loop || position < count, in_loop || position != 1)
}

#[track_caller]
fn assert_navigation(
    editor: &Entity<CodeEditor>,
    cx: &mut VisualTestContext,
    position: usize,
    forward: bool,
    back: bool,
) {
    assert_eq!(matches_position(editor, cx), position, "matchesPosition");
    assert_eq!(
        can_navigate(editor, cx),
        (forward, back),
        "(canNavigateForward, canNavigateBack)"
    );
}

/// Monaco: "incremental find from beginning of file"
#[gpui::test]
fn incremental_find_from_beginning_of_file(cx: &mut TestAppContext) {
    let (editor, cx) = setup(cx);
    set_selection(&editor, cx, [1, 1, 1, 1]);
    start(&editor, cx);

    // simulate typing the search string
    change(&editor, cx, true, |o| o.query = "H".into());
    assert_find_state(
        &editor,
        cx,
        [1, 12, 1, 13],
        Some([1, 12, 1, 13]),
        &[
            [1, 12, 1, 13],
            [2, 16, 2, 17],
            [6, 14, 6, 15],
            [6, 27, 6, 28],
            [7, 14, 7, 15],
            [8, 14, 8, 15],
            [9, 14, 9, 15],
        ],
    );

    // simulate typing the search string
    change(&editor, cx, true, |o| o.query = "He".into());
    assert_find_state(
        &editor,
        cx,
        [1, 12, 1, 14],
        Some([1, 12, 1, 14]),
        &[
            [1, 12, 1, 14],
            [6, 14, 6, 16],
            [6, 27, 6, 29],
            [7, 14, 7, 16],
            [8, 14, 8, 16],
            [9, 14, 9, 16],
        ],
    );

    // simulate typing the search string
    change(&editor, cx, true, |o| o.query = "Hello".into());
    assert_find_state(
        &editor,
        cx,
        [6, 14, 6, 19],
        Some([6, 14, 6, 19]),
        &[
            [6, 14, 6, 19],
            [6, 27, 6, 32],
            [7, 14, 7, 19],
            [8, 14, 8, 19],
            [9, 14, 9, 19],
        ],
    );

    // simulate toggling on `matchCase`
    change(&editor, cx, true, |o| o.match_case = true);
    assert_find_state(
        &editor,
        cx,
        [6, 27, 6, 32],
        Some([6, 27, 6, 32]),
        &[[6, 27, 6, 32], [8, 14, 8, 19]],
    );

    // simulate typing the search string
    change(&editor, cx, true, |o| o.query = "hello".into());
    assert_find_state(
        &editor,
        cx,
        [6, 14, 6, 19],
        Some([6, 14, 6, 19]),
        &[[6, 14, 6, 19], [7, 14, 7, 19], [9, 14, 9, 19]],
    );

    // simulate toggling on `wholeWord`
    change(&editor, cx, true, |o| o.whole_word = true);
    assert_find_state(
        &editor,
        cx,
        [6, 14, 6, 19],
        Some([6, 14, 6, 19]),
        &[[6, 14, 6, 19], [7, 14, 7, 19]],
    );

    // simulate toggling off `matchCase`
    change(&editor, cx, true, |o| o.match_case = false);
    assert_find_state(
        &editor,
        cx,
        [6, 14, 6, 19],
        Some([6, 14, 6, 19]),
        &[
            [6, 14, 6, 19],
            [6, 27, 6, 32],
            [7, 14, 7, 19],
            [8, 14, 8, 19],
        ],
    );

    // simulate toggling off `wholeWord`
    change(&editor, cx, true, |o| o.whole_word = false);
    assert_find_state(
        &editor,
        cx,
        [6, 14, 6, 19],
        Some([6, 14, 6, 19]),
        &[
            [6, 14, 6, 19],
            [6, 27, 6, 32],
            [7, 14, 7, 19],
            [8, 14, 8, 19],
            [9, 14, 9, 19],
        ],
    );

    // simulate adding a search scope
    set_scope(&editor, cx, Some(&[[8, 1, 10, 1]]), true);
    assert_find_state(
        &editor,
        cx,
        [8, 14, 8, 19],
        Some([8, 14, 8, 19]),
        &[[8, 14, 8, 19], [9, 14, 9, 19]],
    );

    // simulate removing the search scope
    set_scope(&editor, cx, None, true);
    assert_find_state(
        &editor,
        cx,
        [6, 14, 6, 19],
        Some([6, 14, 6, 19]),
        &[
            [6, 14, 6, 19],
            [6, 27, 6, 32],
            [7, 14, 7, 19],
            [8, 14, 8, 19],
            [9, 14, 9, 19],
        ],
    );
}

/// Monaco: "find model removes its decorations"
#[gpui::test]
fn find_model_removes_its_decorations(cx: &mut TestAppContext) {
    let (editor, cx) = setup(cx);
    change(&editor, cx, false, |o| o.query = "hello".into());
    start(&editor, cx);

    assert_eq!(matches_count(&editor, cx), 5);
    assert_find_state(
        &editor,
        cx,
        [1, 1, 1, 1],
        None,
        &[
            [6, 14, 6, 19],
            [6, 27, 6, 32],
            [7, 14, 7, 19],
            [8, 14, 8, 19],
            [9, 14, 9, 19],
        ],
    );

    // `findModel.dispose()`: there's no public way to unbind the find model
    // without the widget, but switching models unbinds it from the old one.
    // Monaco then reads the (old) model's find decorations, so do the same.
    let old_model = editor.read_with(cx, |e, _| e.model().clone());
    let new_model = cx.new(|_| TextModel::new(""));
    editor.update(cx, |e, cx| e.set_model(new_model, cx));
    let owner = editor.entity_id();
    let find_colors = editor.read_with(cx, |e, _| {
        let colors = &e.theme().colors;
        [
            colors.find_match_background,
            colors.find_match_highlight_background,
        ]
    });
    let remaining: Vec<Rng> = old_model.update(cx, |m, _| {
        let rope = m.buffer().rope().clone();
        m.all_decorations(Some(owner))
            .into_iter()
            .filter(|d| {
                d.options
                    .background
                    .is_some_and(|b| find_colors.contains(&b))
            })
            .map(|d| {
                from_range(
                    &(rope.offset_to_point_utf16(d.range.start)
                        ..rope.offset_to_point_utf16(d.range.end)),
                )
            })
            .collect()
    });
    assert_eq!(remaining, Vec::<Rng>::new(), "state");
}

/// Monaco: "find model updates state matchesCount"
#[gpui::test]
fn find_model_updates_state_matches_count(cx: &mut TestAppContext) {
    let (editor, cx) = setup(cx);
    change(&editor, cx, false, |o| o.query = "hello".into());
    start(&editor, cx);

    assert_eq!(matches_count(&editor, cx), 5);
    assert_find_state(
        &editor,
        cx,
        [1, 1, 1, 1],
        None,
        &[
            [6, 14, 6, 19],
            [6, 27, 6, 32],
            [7, 14, 7, 19],
            [8, 14, 8, 19],
            [9, 14, 9, 19],
        ],
    );

    change(&editor, cx, false, |o| o.query = "helloo".into());
    assert_eq!(matches_count(&editor, cx), 0);
    assert_find_state(&editor, cx, [1, 1, 1, 1], None, &[]);
}

/// Monaco: "find model reacts to position change"
#[gpui::test]
fn find_model_reacts_to_position_change(cx: &mut TestAppContext) {
    let (editor, cx) = setup(cx);
    change(&editor, cx, false, |o| o.query = "hello".into());
    start(&editor, cx);

    let all = [
        [6, 14, 6, 19],
        [6, 27, 6, 32],
        [7, 14, 7, 19],
        [8, 14, 8, 19],
        [9, 14, 9, 19],
    ];
    assert_find_state(&editor, cx, [1, 1, 1, 1], None, &all);

    mouse_move_to(&editor, cx, 6, 20);

    assert_find_state(&editor, cx, [6, 20, 6, 20], None, &all);

    change(&editor, cx, true, |o| o.query = "Hello".into());
    assert_find_state(&editor, cx, [6, 27, 6, 32], Some([6, 27, 6, 32]), &all);
}

/// Monaco: "find model next"
#[gpui::test]
fn find_model_next(cx: &mut TestAppContext) {
    let (editor, cx) = setup(cx);
    change(&editor, cx, false, |o| {
        o.query = "hello".into();
        o.whole_word = true;
    });
    start(&editor, cx);

    let all = [
        [6, 14, 6, 19],
        [6, 27, 6, 32],
        [7, 14, 7, 19],
        [8, 14, 8, 19],
    ];
    assert_find_state(&editor, cx, [1, 1, 1, 1], None, &all);

    move_to_next_match(&editor, cx);
    assert_find_state(&editor, cx, [6, 14, 6, 19], Some([6, 14, 6, 19]), &all);

    move_to_next_match(&editor, cx);
    assert_find_state(&editor, cx, [6, 27, 6, 32], Some([6, 27, 6, 32]), &all);

    move_to_next_match(&editor, cx);
    assert_find_state(&editor, cx, [7, 14, 7, 19], Some([7, 14, 7, 19]), &all);

    move_to_next_match(&editor, cx);
    assert_find_state(&editor, cx, [8, 14, 8, 19], Some([8, 14, 8, 19]), &all);

    move_to_next_match(&editor, cx);
    assert_find_state(&editor, cx, [6, 14, 6, 19], Some([6, 14, 6, 19]), &all);
}

/// Monaco: "find model next stays in scope"
#[gpui::test]
fn find_model_next_stays_in_scope(cx: &mut TestAppContext) {
    let (editor, cx) = setup(cx);
    set_scope(&editor, cx, Some(&[[7, 1, 9, 1]]), false);
    change(&editor, cx, false, |o| {
        o.query = "hello".into();
        o.whole_word = true;
    });
    start(&editor, cx);

    let all = [[7, 14, 7, 19], [8, 14, 8, 19]];
    assert_find_state(&editor, cx, [1, 1, 1, 1], None, &all);

    move_to_next_match(&editor, cx);
    assert_find_state(&editor, cx, [7, 14, 7, 19], Some([7, 14, 7, 19]), &all);

    move_to_next_match(&editor, cx);
    assert_find_state(&editor, cx, [8, 14, 8, 19], Some([8, 14, 8, 19]), &all);

    move_to_next_match(&editor, cx);
    assert_find_state(&editor, cx, [7, 14, 7, 19], Some([7, 14, 7, 19]), &all);
}

/// Monaco: "multi-selection find model next stays in scope (overlap)"
#[gpui::test]
fn multi_selection_find_model_next_stays_in_scope_overlap(cx: &mut TestAppContext) {
    let (editor, cx) = setup(cx);
    set_scope(&editor, cx, Some(&[[7, 1, 8, 2], [8, 1, 9, 1]]), false);
    change(&editor, cx, false, |o| {
        o.query = "hello".into();
        o.whole_word = true;
    });
    start(&editor, cx);

    let all = [[7, 14, 7, 19], [8, 14, 8, 19]];
    assert_find_state(&editor, cx, [1, 1, 1, 1], None, &all);

    move_to_next_match(&editor, cx);
    assert_find_state(&editor, cx, [7, 14, 7, 19], Some([7, 14, 7, 19]), &all);

    move_to_next_match(&editor, cx);
    assert_find_state(&editor, cx, [8, 14, 8, 19], Some([8, 14, 8, 19]), &all);

    move_to_next_match(&editor, cx);
    assert_find_state(&editor, cx, [7, 14, 7, 19], Some([7, 14, 7, 19]), &all);
}

/// Monaco: "multi-selection find model next stays in scope"
#[gpui::test]
fn multi_selection_find_model_next_stays_in_scope(cx: &mut TestAppContext) {
    let (editor, cx) = setup(cx);
    set_scope(&editor, cx, Some(&[[6, 1, 7, 38], [9, 3, 9, 38]]), false);
    change(&editor, cx, false, |o| {
        o.query = "hello".into();
        o.match_case = true;
        o.whole_word = false;
    });
    start(&editor, cx);

    let all = [
        [6, 14, 6, 19],
        // `matchCase: false` would
        // find this match as well:
        // [6, 27, 6, 32],
        [7, 14, 7, 19],
        // `wholeWord: true` would
        // exclude this match:
        [9, 14, 9, 19],
    ];
    assert_find_state(&editor, cx, [1, 1, 1, 1], None, &all);

    move_to_next_match(&editor, cx);
    assert_find_state(&editor, cx, [6, 14, 6, 19], Some([6, 14, 6, 19]), &all);

    move_to_next_match(&editor, cx);
    assert_find_state(&editor, cx, [7, 14, 7, 19], Some([7, 14, 7, 19]), &all);

    move_to_next_match(&editor, cx);
    assert_find_state(&editor, cx, [9, 14, 9, 19], Some([9, 14, 9, 19]), &all);

    move_to_next_match(&editor, cx);
    assert_find_state(&editor, cx, [6, 14, 6, 19], Some([6, 14, 6, 19]), &all);
}

/// Monaco: "find model prev"
#[gpui::test]
fn find_model_prev(cx: &mut TestAppContext) {
    let (editor, cx) = setup(cx);
    change(&editor, cx, false, |o| {
        o.query = "hello".into();
        o.whole_word = true;
    });
    start(&editor, cx);

    let all = [
        [6, 14, 6, 19],
        [6, 27, 6, 32],
        [7, 14, 7, 19],
        [8, 14, 8, 19],
    ];
    assert_find_state(&editor, cx, [1, 1, 1, 1], None, &all);

    move_to_prev_match(&editor, cx);
    assert_find_state(&editor, cx, [8, 14, 8, 19], Some([8, 14, 8, 19]), &all);

    move_to_prev_match(&editor, cx);
    assert_find_state(&editor, cx, [7, 14, 7, 19], Some([7, 14, 7, 19]), &all);

    move_to_prev_match(&editor, cx);
    assert_find_state(&editor, cx, [6, 27, 6, 32], Some([6, 27, 6, 32]), &all);

    move_to_prev_match(&editor, cx);
    assert_find_state(&editor, cx, [6, 14, 6, 19], Some([6, 14, 6, 19]), &all);

    move_to_prev_match(&editor, cx);
    assert_find_state(&editor, cx, [8, 14, 8, 19], Some([8, 14, 8, 19]), &all);
}

/// Monaco: "find model prev stays in scope"
#[gpui::test]
fn find_model_prev_stays_in_scope(cx: &mut TestAppContext) {
    let (editor, cx) = setup(cx);
    set_scope(&editor, cx, Some(&[[7, 1, 9, 1]]), false);
    change(&editor, cx, false, |o| {
        o.query = "hello".into();
        o.whole_word = true;
    });
    start(&editor, cx);

    let all = [[7, 14, 7, 19], [8, 14, 8, 19]];
    assert_find_state(&editor, cx, [1, 1, 1, 1], None, &all);

    move_to_prev_match(&editor, cx);
    assert_find_state(&editor, cx, [8, 14, 8, 19], Some([8, 14, 8, 19]), &all);

    move_to_prev_match(&editor, cx);
    assert_find_state(&editor, cx, [7, 14, 7, 19], Some([7, 14, 7, 19]), &all);

    move_to_prev_match(&editor, cx);
    assert_find_state(&editor, cx, [8, 14, 8, 19], Some([8, 14, 8, 19]), &all);
}

/// Monaco: "find model next/prev with no matches"
#[gpui::test]
fn find_model_next_prev_with_no_matches(cx: &mut TestAppContext) {
    let (editor, cx) = setup(cx);
    change(&editor, cx, false, |o| {
        o.query = "helloo".into();
        o.whole_word = true;
    });
    start(&editor, cx);

    assert_find_state(&editor, cx, [1, 1, 1, 1], None, &[]);

    move_to_next_match(&editor, cx);
    assert_find_state(&editor, cx, [1, 1, 1, 1], None, &[]);

    move_to_prev_match(&editor, cx);
    assert_find_state(&editor, cx, [1, 1, 1, 1], None, &[]);
}

/// Monaco: "find model next/prev respects cursor position"
#[gpui::test]
fn find_model_next_prev_respects_cursor_position(cx: &mut TestAppContext) {
    let (editor, cx) = setup(cx);
    change(&editor, cx, false, |o| {
        o.query = "hello".into();
        o.whole_word = true;
    });
    start(&editor, cx);

    let all = [
        [6, 14, 6, 19],
        [6, 27, 6, 32],
        [7, 14, 7, 19],
        [8, 14, 8, 19],
    ];
    assert_find_state(&editor, cx, [1, 1, 1, 1], None, &all);

    mouse_move_to(&editor, cx, 6, 20);
    assert_find_state(&editor, cx, [6, 20, 6, 20], None, &all);

    move_to_next_match(&editor, cx);
    assert_find_state(&editor, cx, [6, 27, 6, 32], Some([6, 27, 6, 32]), &all);
}

const LINE_STARTS: [Rng; 12] = [
    [1, 1, 1, 1],
    [2, 1, 2, 1],
    [3, 1, 3, 1],
    [4, 1, 4, 1],
    [5, 1, 5, 1],
    [6, 1, 6, 1],
    [7, 1, 7, 1],
    [8, 1, 8, 1],
    [9, 1, 9, 1],
    [10, 1, 10, 1],
    [11, 1, 11, 1],
    [12, 1, 12, 1],
];

const LINE_ENDS: [Rng; 12] = [
    [1, 18, 1, 18],
    [2, 18, 2, 18],
    [3, 20, 3, 20],
    [4, 1, 4, 1],
    [5, 13, 5, 13],
    [6, 43, 6, 43],
    [7, 41, 7, 41],
    [8, 41, 8, 41],
    [9, 40, 9, 40],
    [10, 2, 10, 2],
    [11, 17, 11, 17],
    [12, 1, 12, 1],
];

const WHOLE_LINES: [Rng; 12] = [
    [1, 1, 1, 18],
    [2, 1, 2, 18],
    [3, 1, 3, 20],
    [4, 1, 4, 1],
    [5, 1, 5, 13],
    [6, 1, 6, 43],
    [7, 1, 7, 41],
    [8, 1, 8, 41],
    [9, 1, 9, 40],
    [10, 1, 10, 2],
    [11, 1, 11, 17],
    [12, 1, 12, 1],
];

/// Monaco: "find ^"
#[gpui::test]
fn find_caret(cx: &mut TestAppContext) {
    let (editor, cx) = setup(cx);
    change(&editor, cx, false, |o| {
        o.query = "^".into();
        o.is_regex = true;
    });
    start(&editor, cx);

    assert_find_state(&editor, cx, [1, 1, 1, 1], None, &LINE_STARTS);

    move_to_next_match(&editor, cx);
    assert_find_state(&editor, cx, [2, 1, 2, 1], Some([2, 1, 2, 1]), &LINE_STARTS);

    move_to_next_match(&editor, cx);
    assert_find_state(&editor, cx, [3, 1, 3, 1], Some([3, 1, 3, 1]), &LINE_STARTS);
}

/// Monaco: "find $"
#[gpui::test]
fn find_dollar(cx: &mut TestAppContext) {
    let (editor, cx) = setup(cx);
    change(&editor, cx, false, |o| {
        o.query = "$".into();
        o.is_regex = true;
    });
    start(&editor, cx);

    assert_find_state(&editor, cx, [1, 1, 1, 1], None, &LINE_ENDS);

    move_to_next_match(&editor, cx);
    assert_find_state(
        &editor,
        cx,
        [1, 18, 1, 18],
        Some([1, 18, 1, 18]),
        &LINE_ENDS,
    );

    move_to_next_match(&editor, cx);
    assert_find_state(
        &editor,
        cx,
        [2, 18, 2, 18],
        Some([2, 18, 2, 18]),
        &LINE_ENDS,
    );

    move_to_next_match(&editor, cx);
    assert_find_state(
        &editor,
        cx,
        [3, 20, 3, 20],
        Some([3, 20, 3, 20]),
        &LINE_ENDS,
    );
}

/// Monaco: "find next ^$"
#[gpui::test]
fn find_next_caret_dollar(cx: &mut TestAppContext) {
    let (editor, cx) = setup(cx);
    change(&editor, cx, false, |o| {
        o.query = "^$".into();
        o.is_regex = true;
    });
    start(&editor, cx);

    let all = [[4, 1, 4, 1], [12, 1, 12, 1]];
    assert_find_state(&editor, cx, [1, 1, 1, 1], None, &all);

    move_to_next_match(&editor, cx);
    assert_find_state(&editor, cx, [4, 1, 4, 1], Some([4, 1, 4, 1]), &all);

    move_to_next_match(&editor, cx);
    assert_find_state(&editor, cx, [12, 1, 12, 1], Some([12, 1, 12, 1]), &all);

    move_to_next_match(&editor, cx);
    assert_find_state(&editor, cx, [4, 1, 4, 1], Some([4, 1, 4, 1]), &all);
}

/// Monaco: "find .*"
#[gpui::test]
fn find_dot_star(cx: &mut TestAppContext) {
    let (editor, cx) = setup(cx);
    change(&editor, cx, false, |o| {
        o.query = ".*".into();
        o.is_regex = true;
    });
    start(&editor, cx);

    assert_find_state(&editor, cx, [1, 1, 1, 1], None, &WHOLE_LINES);
}

/// Monaco: "find next ^.*$"
#[gpui::test]
fn find_next_caret_dot_star_dollar(cx: &mut TestAppContext) {
    let (editor, cx) = setup(cx);
    change(&editor, cx, false, |o| {
        o.query = "^.*$".into();
        o.is_regex = true;
    });
    start(&editor, cx);

    assert_find_state(&editor, cx, [1, 1, 1, 1], None, &WHOLE_LINES);

    move_to_next_match(&editor, cx);
    assert_find_state(
        &editor,
        cx,
        [1, 1, 1, 18],
        Some([1, 1, 1, 18]),
        &WHOLE_LINES,
    );

    move_to_next_match(&editor, cx);
    assert_find_state(
        &editor,
        cx,
        [2, 1, 2, 18],
        Some([2, 1, 2, 18]),
        &WHOLE_LINES,
    );
}

/// Monaco: "find prev ^.*$"
#[gpui::test]
fn find_prev_caret_dot_star_dollar(cx: &mut TestAppContext) {
    let (editor, cx) = setup(cx);
    change(&editor, cx, false, |o| {
        o.query = "^.*$".into();
        o.is_regex = true;
    });
    start(&editor, cx);

    assert_find_state(&editor, cx, [1, 1, 1, 1], None, &WHOLE_LINES);

    move_to_prev_match(&editor, cx);
    assert_find_state(
        &editor,
        cx,
        [12, 1, 12, 1],
        Some([12, 1, 12, 1]),
        &WHOLE_LINES,
    );

    move_to_prev_match(&editor, cx);
    assert_find_state(
        &editor,
        cx,
        [11, 1, 11, 17],
        Some([11, 1, 11, 17]),
        &WHOLE_LINES,
    );
}

/// Monaco: "find prev ^$"
#[gpui::test]
fn find_prev_caret_dollar(cx: &mut TestAppContext) {
    let (editor, cx) = setup(cx);
    change(&editor, cx, false, |o| {
        o.query = "^$".into();
        o.is_regex = true;
    });
    start(&editor, cx);

    let all = [[4, 1, 4, 1], [12, 1, 12, 1]];
    assert_find_state(&editor, cx, [1, 1, 1, 1], None, &all);

    move_to_prev_match(&editor, cx);
    assert_find_state(&editor, cx, [12, 1, 12, 1], Some([12, 1, 12, 1]), &all);

    move_to_prev_match(&editor, cx);
    assert_find_state(&editor, cx, [4, 1, 4, 1], Some([4, 1, 4, 1]), &all);

    move_to_prev_match(&editor, cx);
    assert_find_state(&editor, cx, [12, 1, 12, 1], Some([12, 1, 12, 1]), &all);
}

/// Monaco: "replace hello"
#[gpui::test]
fn replace_hello(cx: &mut TestAppContext) {
    let (editor, cx) = setup(cx);
    change(&editor, cx, false, |o| {
        o.query = "hello".into();
        o.replace = "hi".into();
        o.whole_word = true;
    });
    start(&editor, cx);

    let all = [
        [6, 14, 6, 19],
        [6, 27, 6, 32],
        [7, 14, 7, 19],
        [8, 14, 8, 19],
    ];
    assert_find_state(&editor, cx, [1, 1, 1, 1], None, &all);

    mouse_move_to(&editor, cx, 6, 20);
    assert_find_state(&editor, cx, [6, 20, 6, 20], None, &all);
    assert_eq!(
        line_content(&editor, cx, 6),
        "    cout << \"hello world, Hello!\" << endl;"
    );

    replace(&editor, cx);
    assert_find_state(&editor, cx, [6, 27, 6, 32], Some([6, 27, 6, 32]), &all);
    assert_eq!(
        line_content(&editor, cx, 6),
        "    cout << \"hello world, Hello!\" << endl;"
    );

    replace(&editor, cx);
    assert_find_state(
        &editor,
        cx,
        [7, 14, 7, 19],
        Some([7, 14, 7, 19]),
        &[[6, 14, 6, 19], [7, 14, 7, 19], [8, 14, 8, 19]],
    );
    assert_eq!(
        line_content(&editor, cx, 6),
        "    cout << \"hello world, hi!\" << endl;"
    );

    replace(&editor, cx);
    assert_find_state(
        &editor,
        cx,
        [8, 14, 8, 19],
        Some([8, 14, 8, 19]),
        &[[6, 14, 6, 19], [8, 14, 8, 19]],
    );
    assert_eq!(
        line_content(&editor, cx, 7),
        "    cout << \"hi world again\" << endl;"
    );

    replace(&editor, cx);
    assert_find_state(
        &editor,
        cx,
        [6, 14, 6, 19],
        Some([6, 14, 6, 19]),
        &[[6, 14, 6, 19]],
    );
    assert_eq!(
        line_content(&editor, cx, 8),
        "    cout << \"hi world again\" << endl;"
    );

    replace(&editor, cx);
    assert_find_state(&editor, cx, [6, 16, 6, 16], None, &[]);
    assert_eq!(
        line_content(&editor, cx, 6),
        "    cout << \"hi world, hi!\" << endl;"
    );
}

/// Monaco: "replace bla"
#[gpui::test]
fn replace_bla(cx: &mut TestAppContext) {
    let (editor, cx) = setup(cx);
    change(&editor, cx, false, |o| {
        o.query = "bla".into();
        o.replace = "ciao".into();
    });
    start(&editor, cx);

    assert_find_state(
        &editor,
        cx,
        [1, 1, 1, 1],
        None,
        &[[11, 4, 11, 7], [11, 7, 11, 10], [11, 10, 11, 13]],
    );

    replace(&editor, cx);
    assert_find_state(
        &editor,
        cx,
        [11, 4, 11, 7],
        Some([11, 4, 11, 7]),
        &[[11, 4, 11, 7], [11, 7, 11, 10], [11, 10, 11, 13]],
    );
    assert_eq!(line_content(&editor, cx, 11), "// blablablaciao");

    replace(&editor, cx);
    assert_find_state(
        &editor,
        cx,
        [11, 8, 11, 11],
        Some([11, 8, 11, 11]),
        &[[11, 8, 11, 11], [11, 11, 11, 14]],
    );
    assert_eq!(line_content(&editor, cx, 11), "// ciaoblablaciao");

    replace(&editor, cx);
    assert_find_state(
        &editor,
        cx,
        [11, 12, 11, 15],
        Some([11, 12, 11, 15]),
        &[[11, 12, 11, 15]],
    );
    assert_eq!(line_content(&editor, cx, 11), "// ciaociaoblaciao");

    replace(&editor, cx);
    assert_find_state(&editor, cx, [11, 16, 11, 16], None, &[]);
    assert_eq!(line_content(&editor, cx, 11), "// ciaociaociaociao");
}

/// Monaco: "replaceAll hello"
#[gpui::test]
fn replace_all_hello(cx: &mut TestAppContext) {
    let (editor, cx) = setup(cx);
    change(&editor, cx, false, |o| {
        o.query = "hello".into();
        o.replace = "hi".into();
        o.whole_word = true;
    });
    start(&editor, cx);

    let all = [
        [6, 14, 6, 19],
        [6, 27, 6, 32],
        [7, 14, 7, 19],
        [8, 14, 8, 19],
    ];
    assert_find_state(&editor, cx, [1, 1, 1, 1], None, &all);

    mouse_move_to(&editor, cx, 6, 20);
    assert_find_state(&editor, cx, [6, 20, 6, 20], None, &all);
    assert_eq!(
        line_content(&editor, cx, 6),
        "    cout << \"hello world, Hello!\" << endl;"
    );

    replace_all(&editor, cx);
    assert_find_state(&editor, cx, [6, 17, 6, 17], None, &[]);
    assert_eq!(
        line_content(&editor, cx, 6),
        "    cout << \"hi world, hi!\" << endl;"
    );
    assert_eq!(
        line_content(&editor, cx, 7),
        "    cout << \"hi world again\" << endl;"
    );
    assert_eq!(
        line_content(&editor, cx, 8),
        "    cout << \"hi world again\" << endl;"
    );
}

/// Monaco: "replaceAll two spaces with one space"
#[gpui::test]
fn replace_all_two_spaces_with_one_space(cx: &mut TestAppContext) {
    let (editor, cx) = setup(cx);
    change(&editor, cx, false, |o| {
        o.query = "  ".into();
        o.replace = " ".into();
    });
    start(&editor, cx);

    assert_find_state(
        &editor,
        cx,
        [1, 1, 1, 1],
        None,
        &[
            [6, 1, 6, 3],
            [6, 3, 6, 5],
            [7, 1, 7, 3],
            [7, 3, 7, 5],
            [8, 1, 8, 3],
            [8, 3, 8, 5],
            [9, 1, 9, 3],
            [9, 3, 9, 5],
        ],
    );

    replace_all(&editor, cx);
    assert_find_state(
        &editor,
        cx,
        [1, 1, 1, 1],
        None,
        &[[6, 1, 6, 3], [7, 1, 7, 3], [8, 1, 8, 3], [9, 1, 9, 3]],
    );
    assert_eq!(
        line_content(&editor, cx, 6),
        "  cout << \"hello world, Hello!\" << endl;"
    );
    assert_eq!(
        line_content(&editor, cx, 7),
        "  cout << \"hello world again\" << endl;"
    );
    assert_eq!(
        line_content(&editor, cx, 8),
        "  cout << \"Hello world again\" << endl;"
    );
    assert_eq!(
        line_content(&editor, cx, 9),
        "  cout << \"helloworld again\" << endl;"
    );
}

/// Monaco: "replaceAll bla"
#[gpui::test]
fn replace_all_bla(cx: &mut TestAppContext) {
    let (editor, cx) = setup(cx);
    change(&editor, cx, false, |o| {
        o.query = "bla".into();
        o.replace = "ciao".into();
    });
    start(&editor, cx);

    assert_find_state(
        &editor,
        cx,
        [1, 1, 1, 1],
        None,
        &[[11, 4, 11, 7], [11, 7, 11, 10], [11, 10, 11, 13]],
    );

    replace_all(&editor, cx);
    assert_find_state(&editor, cx, [1, 1, 1, 1], None, &[]);
    assert_eq!(line_content(&editor, cx, 11), "// ciaociaociaociao");
}

/// Monaco: "replaceAll bla with \\t\\n"
#[gpui::test]
fn replace_all_bla_with_tab_newline(cx: &mut TestAppContext) {
    let (editor, cx) = setup(cx);
    change(&editor, cx, false, |o| {
        o.query = "bla".into();
        o.replace = r"<\n\t>".into();
        o.is_regex = true;
    });
    start(&editor, cx);

    assert_find_state(
        &editor,
        cx,
        [1, 1, 1, 1],
        None,
        &[[11, 4, 11, 7], [11, 7, 11, 10], [11, 10, 11, 13]],
    );

    replace_all(&editor, cx);
    assert_find_state(&editor, cx, [1, 1, 1, 1], None, &[]);
    assert_eq!(line_content(&editor, cx, 11), "// <");
    assert_eq!(line_content(&editor, cx, 12), "\t><");
    assert_eq!(line_content(&editor, cx, 13), "\t><");
    assert_eq!(line_content(&editor, cx, 14), "\t>ciao");
}

/// Monaco: "issue #3516: \"replace all\" moves page/cursor/focus/scroll to the place of the last replacement"
#[gpui::test]
fn issue_3516_replace_all_moves_page_cursor_focus_scroll_to_the_place_of_the_last_replacement(
    cx: &mut TestAppContext,
) {
    let (editor, cx) = setup(cx);
    change(&editor, cx, false, |o| {
        o.query = "include".into();
        o.replace = "bar".into();
    });
    start(&editor, cx);

    assert_find_state(
        &editor,
        cx,
        [1, 1, 1, 1],
        None,
        &[[2, 2, 2, 9], [3, 2, 3, 9]],
    );

    replace_all(&editor, cx);
    assert_find_state(&editor, cx, [1, 1, 1, 1], None, &[]);

    assert_eq!(line_content(&editor, cx, 2), "#bar \"cool.h\"");
    assert_eq!(line_content(&editor, cx, 3), "#bar <iostream>");
}

/// Monaco: "listens to model content changes"
#[gpui::test]
fn listens_to_model_content_changes(cx: &mut TestAppContext) {
    let (editor, cx) = setup(cx);
    change(&editor, cx, false, |o| {
        o.query = "hello".into();
        o.replace = "hi".into();
        o.whole_word = true;
    });
    start(&editor, cx);

    assert_find_state(
        &editor,
        cx,
        [1, 1, 1, 1],
        None,
        &[
            [6, 14, 6, 19],
            [6, 27, 6, 32],
            [7, 14, 7, 19],
            [8, 14, 8, 19],
        ],
    );

    // Monaco asserts right after `setValue`, before the 100ms re-search.
    let model = editor.read_with(cx, |e, _| e.model().clone());
    model.update(cx, |m, cx| m.set_value("hello\nhi", cx));
    assert_find_state(&editor, cx, [1, 1, 1, 1], None, &[]);
}

/// Monaco: "selectAllMatches"
#[gpui::test]
fn select_all_matches_test(cx: &mut TestAppContext) {
    let (editor, cx) = setup(cx);
    change(&editor, cx, false, |o| {
        o.query = "hello".into();
        o.replace = "hi".into();
        o.whole_word = true;
    });
    start(&editor, cx);

    let all = [
        [6, 14, 6, 19],
        [6, 27, 6, 32],
        [7, 14, 7, 19],
        [8, 14, 8, 19],
    ];
    assert_find_state(&editor, cx, [1, 1, 1, 1], None, &all);

    select_all_matches(&editor, cx);

    assert_eq!(
        selections(&editor, cx),
        vec![
            [6, 14, 6, 19],
            [6, 27, 6, 32],
            [7, 14, 7, 19],
            [8, 14, 8, 19]
        ]
    );

    assert_find_state(&editor, cx, [6, 14, 6, 19], None, &all);
}

/// Monaco: "issue #14143 selectAllMatches should maintain primary cursor if feasible"
#[gpui::test]
fn issue_14143_select_all_matches_should_maintain_primary_cursor_if_feasible(
    cx: &mut TestAppContext,
) {
    let (editor, cx) = setup(cx);
    change(&editor, cx, false, |o| {
        o.query = "hello".into();
        o.replace = "hi".into();
        o.whole_word = true;
    });
    start(&editor, cx);

    let all = [
        [6, 14, 6, 19],
        [6, 27, 6, 32],
        [7, 14, 7, 19],
        [8, 14, 8, 19],
    ];
    assert_find_state(&editor, cx, [1, 1, 1, 1], None, &all);

    set_selection(&editor, cx, [7, 14, 7, 19]);

    select_all_matches(&editor, cx);

    assert_eq!(
        selections(&editor, cx),
        vec![
            [7, 14, 7, 19],
            [6, 14, 6, 19],
            [6, 27, 6, 32],
            [8, 14, 8, 19]
        ]
    );

    assert_eq!(selections(&editor, cx)[0], [7, 14, 7, 19]);

    assert_find_state(&editor, cx, [7, 14, 7, 19], None, &all);
}

/// Monaco: "issue #1914: NPE when there is only one find match"
#[gpui::test]
fn issue_1914_npe_when_there_is_only_one_find_match(cx: &mut TestAppContext) {
    let (editor, cx) = setup(cx);
    change(&editor, cx, false, |o| o.query = "cool.h".into());
    start(&editor, cx);

    assert_find_state(&editor, cx, [1, 1, 1, 1], None, &[[2, 11, 2, 17]]);

    move_to_next_match(&editor, cx);
    assert_find_state(
        &editor,
        cx,
        [2, 11, 2, 17],
        Some([2, 11, 2, 17]),
        &[[2, 11, 2, 17]],
    );

    move_to_next_match(&editor, cx);
    assert_find_state(
        &editor,
        cx,
        [2, 11, 2, 17],
        Some([2, 11, 2, 17]),
        &[[2, 11, 2, 17]],
    );
}

// Skipped — these search with JavaScript lookahead (`(?=…)`), which the Rust
// `regex` crate doesn't support (the query fails to parse, so nothing is
// found):
// - Monaco: "replace when search string has look ahed regex"
// - Monaco: "replace when search string has look ahed regex and cursor is at
//   the last find match"
// - Monaco: "replaceAll when search string has look ahed regex"
// - Monaco: "replace when search string has look ahed regex and replace
//   string has capturing groups"
// - Monaco: "replaceAll when search string has look ahed regex and replace
//   string has capturing groups"
// - Monaco: "replaceAll when search string is multiline and has look ahed
//   regex and replace string has capturing groups"

/// Monaco: "replaceAll preserving case"
#[gpui::test]
fn replace_all_preserving_case(cx: &mut TestAppContext) {
    let (editor, cx) = setup(cx);
    change(&editor, cx, false, |o| {
        o.query = "hello".into();
        o.replace = "goodbye".into();
        o.is_regex = false;
        o.match_case = false;
        o.preserve_case = true;
    });
    start(&editor, cx);

    assert_find_state(
        &editor,
        cx,
        [1, 1, 1, 1],
        None,
        &[
            [6, 14, 6, 19],
            [6, 27, 6, 32],
            [7, 14, 7, 19],
            [8, 14, 8, 19],
            [9, 14, 9, 19],
        ],
    );

    replace_all(&editor, cx);

    assert_eq!(
        line_content(&editor, cx, 6),
        "    cout << \"goodbye world, Goodbye!\" << endl;"
    );
    assert_eq!(
        line_content(&editor, cx, 7),
        "    cout << \"goodbye world again\" << endl;"
    );
    assert_eq!(
        line_content(&editor, cx, 8),
        "    cout << \"Goodbye world again\" << endl;"
    );
    assert_eq!(
        line_content(&editor, cx, 9),
        "    cout << \"goodbyeworld again\" << endl;"
    );

    assert_find_state(&editor, cx, [1, 1, 1, 1], None, &[]);
}

/// Monaco: "issue #18711 replaceAll with empty string"
#[gpui::test]
fn issue_18711_replace_all_with_empty_string(cx: &mut TestAppContext) {
    let (editor, cx) = setup(cx);
    change(&editor, cx, false, |o| {
        o.query = "hello".into();
        o.replace = "".into();
        o.whole_word = true;
    });
    start(&editor, cx);

    assert_find_state(
        &editor,
        cx,
        [1, 1, 1, 1],
        None,
        &[
            [6, 14, 6, 19],
            [6, 27, 6, 32],
            [7, 14, 7, 19],
            [8, 14, 8, 19],
        ],
    );

    replace_all(&editor, cx);
    assert_find_state(&editor, cx, [1, 1, 1, 1], None, &[]);
    assert_eq!(
        line_content(&editor, cx, 6),
        "    cout << \" world, !\" << endl;"
    );
    assert_eq!(
        line_content(&editor, cx, 7),
        "    cout << \" world again\" << endl;"
    );
    assert_eq!(
        line_content(&editor, cx, 8),
        "    cout << \" world again\" << endl;"
    );
}

/// Monaco: "issue #32522 replaceAll with ^ on more than 1000 matches"
#[gpui::test]
fn issue_32522_replace_all_with_caret_on_more_than_1000_matches(cx: &mut TestAppContext) {
    let (editor, cx) = setup(cx);
    let mut initial_text = String::new();
    for i in 0..1100 {
        initial_text += &format!("line{i}\n");
    }
    let model = editor.read_with(cx, |e, _| e.model().clone());
    model.update(cx, |m, cx| m.set_value(&initial_text, cx));
    change(&editor, cx, false, |o| {
        o.query = "^".into();
        o.replace = "a ".into();
        o.is_regex = true;
    });
    start(&editor, cx);

    replace_all(&editor, cx);

    let mut expected_text = String::new();
    for i in 0..1100 {
        expected_text += &format!("a line{i}\n");
    }
    expected_text += "a ";
    assert_eq!(model.read_with(cx, |m, _| m.text()), expected_text);
}

/// Monaco: "issue #19740 Find and replace capture group/backreference inserts `undefined` instead of empty string"
#[gpui::test]
fn issue_19740_find_and_replace_capture_group_backreference_inserts_undefined_instead_of_empty_string(
    cx: &mut TestAppContext,
) {
    let (editor, cx) = setup(cx);
    change(&editor, cx, false, |o| {
        o.query = "hello(z)?".into();
        o.replace = "hi$1".into();
        o.is_regex = true;
        o.match_case = true;
    });
    start(&editor, cx);

    assert_find_state(
        &editor,
        cx,
        [1, 1, 1, 1],
        None,
        &[[6, 14, 6, 19], [7, 14, 7, 19], [9, 14, 9, 19]],
    );

    replace_all(&editor, cx);
    assert_find_state(&editor, cx, [1, 1, 1, 1], None, &[]);
    assert_eq!(
        line_content(&editor, cx, 6),
        "    cout << \"hi world, Hello!\" << endl;"
    );
    assert_eq!(
        line_content(&editor, cx, 7),
        "    cout << \"hi world again\" << endl;"
    );
    assert_eq!(
        line_content(&editor, cx, 9),
        "    cout << \"hiworld again\" << endl;"
    );
}

/// Monaco: "issue #27083. search scope works even if it is a single line"
#[gpui::test]
fn issue_27083_search_scope_works_even_if_it_is_a_single_line(cx: &mut TestAppContext) {
    let (editor, cx) = setup(cx);
    set_scope(&editor, cx, Some(&[[7, 1, 8, 1]]), false);
    change(&editor, cx, false, |o| {
        o.query = "hello".into();
        o.whole_word = true;
    });
    start(&editor, cx);

    assert_find_state(&editor, cx, [1, 1, 1, 1], None, &[[7, 14, 7, 19]]);
}

/// Monaco: "issue #3516: Control behavior of \"Next\" operations (not looping back to beginning)"
#[gpui::test]
fn issue_3516_control_behavior_of_next_operations_not_looping_back_to_beginning(
    cx: &mut TestAppContext,
) {
    let (editor, cx) = setup(cx);
    set_loop(&editor, cx, false);
    change(&editor, cx, false, |o| o.query = "hello".into());
    start(&editor, cx);

    assert_eq!(matches_count(&editor, cx), 5);

    // Test next operations
    assert_navigation(&editor, cx, 0, true, true);

    move_to_next_match(&editor, cx);
    assert_navigation(&editor, cx, 1, true, false);

    move_to_next_match(&editor, cx);
    assert_navigation(&editor, cx, 2, true, true);

    move_to_next_match(&editor, cx);
    assert_navigation(&editor, cx, 3, true, true);

    move_to_next_match(&editor, cx);
    assert_navigation(&editor, cx, 4, true, true);

    move_to_next_match(&editor, cx);
    assert_navigation(&editor, cx, 5, false, true);

    move_to_next_match(&editor, cx);
    assert_navigation(&editor, cx, 5, false, true);

    move_to_next_match(&editor, cx);
    assert_navigation(&editor, cx, 5, false, true);

    // Test previous operations
    move_to_prev_match(&editor, cx);
    assert_navigation(&editor, cx, 4, true, true);

    move_to_prev_match(&editor, cx);
    assert_navigation(&editor, cx, 3, true, true);

    move_to_prev_match(&editor, cx);
    assert_navigation(&editor, cx, 2, true, true);

    move_to_prev_match(&editor, cx);
    assert_navigation(&editor, cx, 1, true, false);

    move_to_prev_match(&editor, cx);
    assert_navigation(&editor, cx, 1, true, false);

    move_to_prev_match(&editor, cx);
    assert_navigation(&editor, cx, 1, true, false);
}

/// Monaco: "issue #3516: Control behavior of \"Next\" operations (looping back to beginning)"
#[gpui::test]
fn issue_3516_control_behavior_of_next_operations_looping_back_to_beginning(
    cx: &mut TestAppContext,
) {
    let (editor, cx) = setup(cx);
    change(&editor, cx, false, |o| o.query = "hello".into());
    start(&editor, cx);

    assert_eq!(matches_count(&editor, cx), 5);

    // Test next operations
    assert_navigation(&editor, cx, 0, true, true);

    move_to_next_match(&editor, cx);
    assert_navigation(&editor, cx, 1, true, true);

    move_to_next_match(&editor, cx);
    assert_navigation(&editor, cx, 2, true, true);

    move_to_next_match(&editor, cx);
    assert_navigation(&editor, cx, 3, true, true);

    move_to_next_match(&editor, cx);
    assert_navigation(&editor, cx, 4, true, true);

    move_to_next_match(&editor, cx);
    assert_navigation(&editor, cx, 5, true, true);

    move_to_next_match(&editor, cx);
    assert_navigation(&editor, cx, 1, true, true);

    move_to_next_match(&editor, cx);
    assert_navigation(&editor, cx, 2, true, true);

    // Test previous operations
    move_to_prev_match(&editor, cx);
    assert_navigation(&editor, cx, 1, true, true);

    move_to_prev_match(&editor, cx);
    assert_navigation(&editor, cx, 5, true, true);

    move_to_prev_match(&editor, cx);
    assert_navigation(&editor, cx, 4, true, true);

    move_to_prev_match(&editor, cx);
    assert_navigation(&editor, cx, 3, true, true);

    move_to_prev_match(&editor, cx);
    assert_navigation(&editor, cx, 2, true, true);

    move_to_prev_match(&editor, cx);
    assert_navigation(&editor, cx, 1, true, true);
}

/// Monaco: "issue #288515: Wrong current index in find widget if matches > 1000"
#[gpui::test]
fn issue_288515_wrong_current_index_in_find_widget_if_matches_over_1000(cx: &mut TestAppContext) {
    // Create 1001 lines of 'hello'
    let (editor, cx) = setup_text(vec!["hello"; 1001].join("\n"), cx);

    // Place cursor at line 900, selecting 'hello'
    set_selection(&editor, cx, [900, 1, 900, 6]);

    change(&editor, cx, false, |o| o.query = "hello".into());
    start(&editor, cx);

    assert_eq!(matches_count(&editor, cx), 1001);
    // With cursor selecting 'hello' at line 900, matchesPosition should be 900
    assert_eq!(matches_position(&editor, cx), 900);
}
