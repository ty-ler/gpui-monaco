//! Where folding ranges come from (Monaco `foldingStrategy`).

use gpui::{App, AppContext as _, Entity, Focusable, Task, TestAppContext, VisualTestContext};
use gpui_monaco::options::FoldingStrategy;
use gpui_monaco::providers::*;
use gpui_monaco::text::PointUtf16;
use gpui_monaco::{CodeEditor, TextModel};

const RUST: &str = "fn main() {\n    let v = vec![\n        1,\n    ];\n}\n// #region demo\nfn a() {}\n// #endregion\n";

fn setup<'a>(
    text: &str,
    cx: &'a mut TestAppContext,
) -> (Entity<CodeEditor>, &'a mut VisualTestContext) {
    cx.update(|cx| {
        gpui_monaco::init(cx);
        gpui_monaco::set_language_registry(cx, gpui_monaco_languages::registry());
    });
    let text = text.to_string();
    let (editor, cx) = cx.add_window_view(|window, cx| {
        let model = cx.new(|_| TextModel::new(&text).with_language("rust"));
        CodeEditor::new(model, window, cx)
    });
    cx.update(|window, cx| window.focus(&editor.focus_handle(cx), cx));
    settle(cx);
    (editor, cx)
}

/// Draws until the (debounced) ranges are computed.
fn settle(cx: &mut VisualTestContext) {
    for _ in 0..3 {
        cx.executor()
            .advance_clock(std::time::Duration::from_millis(300));
        cx.run_until_parked();
        cx.update(|window, cx| {
            window.refresh();
            window.draw(cx).clear(cx);
        });
    }
    cx.run_until_parked();
}

/// Ranges are computed per text version: an edit recomputes them.
fn append_line(editor: &Entity<CodeEditor>, cx: &mut VisualTestContext) {
    editor.update(cx, |e, cx| {
        let end = e
            .model()
            .read(cx)
            .validate_position(PointUtf16::new(u32::MAX, 0));
        e.execute_edits(vec![(end..end, "\n".into())], None, cx);
    });
    settle(cx);
}

fn ranges(editor: &Entity<CodeEditor>, cx: &mut VisualTestContext) -> Vec<(u32, u32)> {
    editor.read_with(cx, |e, _| {
        e.folding_ranges()
            .iter()
            .map(|r| (r.start_row, r.end_row))
            .collect()
    })
}

#[gpui::test]
fn syntax_folding_with_region_markers(cx: &mut TestAppContext) {
    let (editor, cx) = setup(RUST, cx);
    // The block and the macro's token tree (to the line before `];`), plus
    // the region from the markers.
    assert_eq!(ranges(&editor, cx), [(0, 3), (1, 2), (5, 7)]);
    let kinds: Vec<_> = editor.read_with(cx, |e, _| {
        e.folding_ranges().into_iter().map(|r| r.kind).collect()
    });
    assert_eq!(kinds[2], Some(FoldingRangeKind::Region));
}

/// Ranges from a provider, which win over the syntax tree.
struct Provided;
impl FoldingRangeProvider for Provided {
    fn provide_folding_ranges(
        &self,
        _: &ProviderContext,
        _: &FoldingContext,
        _: &mut App,
    ) -> Task<anyhow::Result<Option<Vec<FoldingRange>>>> {
        Task::ready(Ok(Some(vec![
            FoldingRange {
                start_row: 1,
                end_row: 3,
                kind: Some(FoldingRangeKind::Comment),
            },
            // Crossing the first range's end: kept by Monaco's sanitizing.
            FoldingRange {
                start_row: 0,
                end_row: 4,
                kind: None,
            },
            // Invalid: dropped.
            FoldingRange {
                start_row: 6,
                end_row: 6,
                kind: None,
            },
            FoldingRange {
                start_row: 6,
                end_row: 99,
                kind: None,
            },
        ])))
    }
}

/// Defers to the next source.
struct NoOpinion;
impl FoldingRangeProvider for NoOpinion {
    fn provide_folding_ranges(
        &self,
        _: &ProviderContext,
        _: &FoldingContext,
        _: &mut App,
    ) -> Task<anyhow::Result<Option<Vec<FoldingRange>>>> {
        Task::ready(Ok(None))
    }
}

#[gpui::test]
fn providers_win_and_none_falls_back(cx: &mut TestAppContext) {
    cx.update(|cx| {
        register_folding_range_provider(cx, LanguageSelector::All, NoOpinion);
    });
    let (editor, cx) = setup(RUST, cx);
    // No opinion: the syntax ranges.
    assert_eq!(ranges(&editor, cx), [(0, 3), (1, 2), (5, 7)]);

    cx.update(|_, cx| {
        register_folding_range_provider(cx, LanguageSelector::All, Provided);
    });
    append_line(&editor, cx);
    assert_eq!(ranges(&editor, cx), [(0, 4), (1, 3)]);
    // Folding all hides them.
    editor.update(cx, |e, cx| e.fold_all(cx));
    assert_eq!(
        editor.read_with(cx, |e, cx| e.collapsed_folds(cx)),
        [(0, 4), (1, 3)]
    );

    // `Indentation` ignores providers (and the syntax tree).
    editor.update(cx, |e, cx| {
        e.unfold_all(cx);
        let mut options = e.options().clone();
        options.folding_strategy = FoldingStrategy::Indentation;
        e.update_options(options, cx);
    });
    append_line(&editor, cx);
    assert_eq!(ranges(&editor, cx), [(0, 3), (1, 2), (5, 7)]);
}

const NESTED: &str =
    "fn a() {\n    if x {\n        y();\n    }\n}\n/* one\n   two */\nfn b() {\n    z();\n}\n";

fn collapsed(editor: &Entity<CodeEditor>, cx: &mut VisualTestContext) -> Vec<(u32, u32)> {
    editor.read_with(cx, |e, cx| e.collapsed_folds(cx))
}

fn cursor(editor: &Entity<CodeEditor>, cx: &mut VisualTestContext, row: u32) {
    editor.update(cx, |e, cx| {
        e.set_selections(
            &[gpui_monaco::cursor::Selection::collapsed(PointUtf16::new(
                row, 0,
            ))],
            cx,
        )
    });
}

#[gpui::test]
fn recursive_level_and_except_commands(cx: &mut TestAppContext) {
    let (editor, cx) = setup(NESTED, cx);
    assert_eq!(ranges(&editor, cx), [(0, 3), (1, 2), (5, 6), (7, 8)]);
    // Recursive fold from the outer function folds its child too.
    cursor(&editor, cx, 0);
    cx.simulate_keystrokes(if cfg!(target_os = "macos") {
        "cmd-k cmd-["
    } else {
        "ctrl-k ctrl-["
    });
    assert_eq!(collapsed(&editor, cx), [(0, 3), (1, 2)]);
    // Unfold expands just the region at the cursor.
    editor.update(cx, |e, cx| e.unfold(cx));
    assert_eq!(collapsed(&editor, cx), [(1, 2)]);
    editor.update(cx, |e, cx| e.unfold_all(cx));
    // Level 2: the `if`, not the functions.
    editor.update(cx, |e, cx| e.fold_level(2, cx));
    assert_eq!(collapsed(&editor, cx), [(1, 2)]);
    editor.update(cx, |e, cx| e.unfold_all(cx));
    // Fold all except the region at the cursor (and its parents/children).
    cursor(&editor, cx, 8);
    editor.update(cx, |e, cx| e.fold_all_except(true, cx));
    assert_eq!(collapsed(&editor, cx), [(0, 3), (1, 2), (5, 6)]);
    editor.update(cx, |e, cx| e.unfold_all(cx));
    // Block comments, by the language's block comment start.
    editor.update(cx, |e, cx| e.fold_all_block_comments(cx));
    assert_eq!(collapsed(&editor, cx), [(5, 6)]);
    // Toggle twice is a no-op; toggling recursively folds the children.
    editor.update(cx, |e, cx| e.unfold_all(cx));
    cursor(&editor, cx, 0);
    editor.update(cx, |e, cx| e.toggle_fold_recursively(cx));
    assert_eq!(collapsed(&editor, cx), [(0, 3), (1, 2)]);
    editor.update(cx, |e, cx| e.toggle_fold(cx));
    assert_eq!(collapsed(&editor, cx), [(1, 2)]);
}

#[gpui::test]
fn fold_navigation(cx: &mut TestAppContext) {
    let (editor, cx) = setup(NESTED, cx);
    let head = |cx: &mut VisualTestContext| editor.read_with(cx, |e, _| e.selections()[0].head.row);
    cursor(&editor, cx, 2);
    editor.update(cx, |e, cx| e.go_to_parent_fold(cx));
    assert_eq!(head(cx), 1);
    editor.update(cx, |e, cx| e.go_to_parent_fold(cx));
    assert_eq!(head(cx), 0);
    editor.update(cx, |e, cx| e.go_to_next_fold(cx));
    assert_eq!(head(cx), 5);
    editor.update(cx, |e, cx| e.go_to_next_fold(cx));
    assert_eq!(head(cx), 7);
    editor.update(cx, |e, cx| e.go_to_previous_fold(cx));
    assert_eq!(head(cx), 5);
}

#[gpui::test]
fn manual_ranges_and_placeholder_clicks(cx: &mut TestAppContext) {
    let (editor, cx) = setup(NESTED, cx);
    // Rows 2–4 (ending at the start of row 5, which doesn't count).
    editor.update(cx, |e, cx| {
        e.set_selections(
            &[gpui_monaco::cursor::Selection::new(
                PointUtf16::new(2, 3),
                PointUtf16::new(5, 0),
            )],
            cx,
        )
    });
    cx.simulate_keystrokes(if cfg!(target_os = "macos") {
        "cmd-k cmd-,"
    } else {
        "ctrl-k ctrl-,"
    });
    assert_eq!(collapsed(&editor, cx), [(2, 4)]);
    let manual: Vec<_> = editor.read_with(cx, |e, _| {
        e.folding_ranges()
            .iter()
            .map(|r| (r.start_row, r.end_row))
            .collect()
    });
    assert!(manual.contains(&(2, 4)), "{manual:?}");
    // It survives recomputing.
    append_line(&editor, cx);
    assert!(ranges(&editor, cx).contains(&(2, 4)));
    // Clicking the `⋯` unfolds it.
    settle(cx);
    let at = editor.read_with(cx, |e, _| {
        let end = e.scrolled_visible_position(PointUtf16::new(2, 11)).unwrap();
        gpui::point(end.x + gpui::px(14.), end.y + gpui::px(8.))
    });
    cx.simulate_click(at, gpui::Modifiers::default());
    assert!(collapsed(&editor, cx).is_empty());
    // Removing manual ranges at the cursor.
    cursor(&editor, cx, 3);
    editor.update(cx, |e, cx| e.remove_manual_folding_ranges(cx));
    assert!(!ranges(&editor, cx).contains(&(2, 4)));
}

/// Imports first, from a provider (so ranges have kinds).
struct Imports;
impl FoldingRangeProvider for Imports {
    fn provide_folding_ranges(
        &self,
        _: &ProviderContext,
        _: &FoldingContext,
        _: &mut App,
    ) -> Task<anyhow::Result<Option<Vec<FoldingRange>>>> {
        Task::ready(Ok(Some(vec![
            FoldingRange {
                start_row: 0,
                end_row: 2,
                kind: Some(FoldingRangeKind::Imports),
            },
            FoldingRange {
                start_row: 5,
                end_row: 6,
                kind: Some(FoldingRangeKind::Comment),
            },
        ])))
    }
}

#[gpui::test]
fn imports_by_default_and_typed_commands(cx: &mut TestAppContext) {
    cx.update(|cx| {
        gpui_monaco::init(cx);
        register_folding_range_provider(cx, LanguageSelector::All, Imports);
    });
    let text = NESTED.to_string();
    let (editor, cx) = cx.add_window_view(|window, cx| {
        let model = cx.new(|_| TextModel::new(&text).with_language("rust"));
        let mut editor = CodeEditor::new(model, window, cx);
        let mut options = editor.options().clone();
        options.folding_imports_by_default = true;
        editor.update_options(options, cx);
        editor
    });
    cx.update(|window, cx| window.focus(&editor.focus_handle(cx), cx));
    settle(cx);
    assert_eq!(collapsed(&editor, cx), [(0, 2)]);
    editor.update(cx, |e, cx| e.toggle_import_fold(cx));
    assert!(collapsed(&editor, cx).is_empty());
    // With kinds, block comments are found by kind.
    editor.update(cx, |e, cx| e.fold_all_block_comments(cx));
    assert_eq!(collapsed(&editor, cx), [(5, 6)]);
}

/// Records the folding contexts it's given.
struct ContextRecorder(std::rc::Rc<std::cell::RefCell<Vec<FoldingContext>>>);
impl FoldingRangeProvider for ContextRecorder {
    fn provide_folding_ranges(
        &self,
        _: &ProviderContext,
        folding_context: &FoldingContext,
        _: &mut App,
    ) -> Task<anyhow::Result<Option<Vec<FoldingRange>>>> {
        self.0.borrow_mut().push(folding_context.clone());
        Task::ready(Ok(None))
    }
}

/// Monaco's `syntaxRangeProvider.ts` passes providers a `FoldingContext`
/// (empty in Monaco too).
#[gpui::test]
fn providers_get_a_folding_context(cx: &mut TestAppContext) {
    let contexts = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
    let recorder = ContextRecorder(contexts.clone());
    cx.update(|cx| {
        register_folding_range_provider(cx, LanguageSelector::All, recorder);
    });
    let (editor, cx) = setup(RUST, cx);
    assert!(!contexts.borrow().is_empty());
    assert!(
        contexts
            .borrow()
            .iter()
            .all(|context| *context == FoldingContext::default())
    );
    // The provider had no opinion: the syntax ranges.
    assert_eq!(ranges(&editor, cx), [(0, 3), (1, 2), (5, 7)]);
}

/// Draws a frame without letting time pass: the folding ranges aren't
/// recomputed (Monaco waits 200ms after the last edit).
fn draw(cx: &mut VisualTestContext) {
    cx.run_until_parked();
    cx.update(|window, cx| {
        window.refresh();
        window.draw(cx).clear(cx);
    });
}

/// Monaco's 200ms between a cursor change and revealing the cursor.
fn wait_for_reveal(cx: &mut VisualTestContext) {
    cx.executor()
        .advance_clock(std::time::Duration::from_millis(200));
    cx.run_until_parked();
}

fn select(editor: &Entity<CodeEditor>, cx: &mut VisualTestContext, at: PointUtf16) {
    editor.update(cx, |e, cx| {
        e.set_selections(&[gpui_monaco::cursor::Selection::collapsed(at)], cx)
    });
}

fn fold_at(editor: &Entity<CodeEditor>, cx: &mut VisualTestContext, row: u32) {
    cursor(editor, cx, row);
    editor.update(cx, |e, cx| e.fold(cx));
}

fn head(editor: &Entity<CodeEditor>, cx: &mut VisualTestContext) -> PointUtf16 {
    editor.read_with(cx, |e, _| e.selections()[0].head)
}

/// `fn b`'s body (row 8) folds under row 7.
fn fold_b(editor: &Entity<CodeEditor>, cx: &mut VisualTestContext) {
    fold_at(editor, cx, 7);
    assert_eq!(collapsed(editor, cx), [(7, 8)]);
}

/// Monaco `FoldingController.revealCursor`: 200ms after a cursor lands in
/// a folded region, the region unfolds.
#[gpui::test]
fn set_selections_into_a_fold_unfolds_it(cx: &mut TestAppContext) {
    let (editor, cx) = setup(NESTED, cx);
    fold_b(&editor, cx);
    select(&editor, cx, PointUtf16::new(8, 4));
    // Not yet.
    cx.executor()
        .advance_clock(std::time::Duration::from_millis(150));
    cx.run_until_parked();
    assert_eq!(collapsed(&editor, cx), [(7, 8)]);
    wait_for_reveal(cx);
    assert!(collapsed(&editor, cx).is_empty());
    assert_eq!(head(&editor, cx), PointUtf16::new(8, 4));
    // A cursor that isn't hidden leaves the folds alone.
    fold_at(&editor, cx, 0);
    cursor(&editor, cx, 5);
    wait_for_reveal(cx);
    assert_eq!(collapsed(&editor, cx), [(0, 3)]);
}

#[gpui::test]
fn find_into_a_fold_unfolds_it(cx: &mut TestAppContext) {
    let (editor, cx) = setup(NESTED, cx);
    fold_b(&editor, cx);
    cursor(&editor, cx, 0);
    editor.update_in(cx, |e, window, cx| e.open_find(false, window, cx));
    editor.update(cx, |e, cx| {
        let mut options = e.find_options().clone();
        options.query = "z()".into();
        e.change_find_options(options, false, cx);
    });
    cx.run_until_parked();
    editor.update(cx, |e, cx| e.find_next(cx));
    assert_eq!(head(&editor, cx).row, 8);
    wait_for_reveal(cx);
    assert!(collapsed(&editor, cx).is_empty());
}

#[gpui::test]
fn go_to_line_into_a_fold_unfolds_it(cx: &mut TestAppContext) {
    let (editor, cx) = setup(NESTED, cx);
    fold_b(&editor, cx);
    cx.simulate_keystrokes("ctrl-g");
    cx.simulate_input("9");
    cx.simulate_keystrokes("enter");
    assert_eq!(head(&editor, cx).row, 8);
    wait_for_reveal(cx);
    assert!(collapsed(&editor, cx).is_empty());
}

/// Answers every definition request with `z` in `fn b`.
struct DefinitionInB;
impl DefinitionProvider for DefinitionInB {
    fn provide_definition(
        &self,
        _: &ProviderContext,
        _: PointUtf16,
        _: &mut App,
    ) -> Task<anyhow::Result<Vec<Location>>> {
        Task::ready(Ok(vec![Location {
            uri: None,
            range: PointUtf16::new(8, 4)..PointUtf16::new(8, 5),
            ..Default::default()
        }]))
    }
}

#[gpui::test]
fn go_to_definition_into_a_fold_unfolds_it(cx: &mut TestAppContext) {
    cx.update(|cx| {
        register_definition_provider(cx, LanguageSelector::All, DefinitionInB);
    });
    let (editor, cx) = setup(NESTED, cx);
    fold_b(&editor, cx);
    select(&editor, cx, PointUtf16::new(2, 8));
    cx.simulate_keystrokes("f12");
    cx.run_until_parked();
    assert_eq!(head(&editor, cx), PointUtf16::new(8, 4));
    wait_for_reveal(cx);
    assert!(collapsed(&editor, cx).is_empty());
}

#[gpui::test]
fn jump_to_bracket_into_a_fold_unfolds_it(cx: &mut TestAppContext) {
    let (editor, cx) = setup(NESTED, cx);
    // A manual range over all of `fn a`, its closing brace included.
    editor.update(cx, |e, cx| {
        e.set_selections(
            &[gpui_monaco::cursor::Selection::new(
                PointUtf16::new(0, 0),
                PointUtf16::new(4, 1),
            )],
            cx,
        )
    });
    editor.update(cx, |e, cx| e.create_fold_from_selection(cx));
    assert_eq!(collapsed(&editor, cx), [(0, 4)]);
    // From the folded `{` to its `}`, with the keyboard.
    select(&editor, cx, PointUtf16::new(0, 7));
    cx.simulate_keystrokes(if cfg!(target_os = "macos") {
        "cmd-shift-\\"
    } else {
        "ctrl-shift-\\"
    });
    assert_eq!(head(&editor, cx).row, 4);
    wait_for_reveal(cx);
    assert!(collapsed(&editor, cx).is_empty());
}

#[gpui::test]
fn add_selection_to_next_find_match_into_a_fold_unfolds_it(cx: &mut TestAppContext) {
    let (editor, cx) = setup("fn y() {}\nfn b() {\n    y();\n}\n", cx);
    assert_eq!(ranges(&editor, cx), [(1, 2)]);
    fold_at(&editor, cx, 1);
    select(&editor, cx, PointUtf16::new(0, 3));
    // Select `y`, then add its next occurrence, which is folded.
    editor.update(cx, |e, cx| e.add_selection_to_next_find_match(cx));
    editor.update(cx, |e, cx| e.add_selection_to_next_find_match(cx));
    let rows: Vec<u32> = editor.read_with(cx, |e, _| {
        e.selections().iter().map(|s| s.head.row).collect()
    });
    assert_eq!(rows, [0, 2]);
    wait_for_reveal(cx);
    assert!(collapsed(&editor, cx).is_empty());
}

/// Monaco's folding decorations move with the text at once: the chevrons
/// and fold commands use the moved lines before the ranges are recomputed.
#[gpui::test]
fn chevrons_and_commands_follow_edits_at_once(cx: &mut TestAppContext) {
    let (editor, cx) = setup(NESTED, cx);
    let control_rows = |cx: &mut VisualTestContext| -> Vec<u32> {
        editor.read_with(cx, |e, _| {
            e.fold_controls_for_tests()
                .iter()
                .map(|(row, _, _)| *row)
                .collect()
        })
    };
    draw(cx);
    assert_eq!(control_rows(cx), [0, 1, 5, 7]);
    let start = PointUtf16::new(0, 0);
    editor.update(cx, |e, cx| {
        e.execute_edits(vec![(start..start, "\n\n\n".into())], None, cx);
    });
    draw(cx);
    assert_eq!(control_rows(cx), [3, 4, 8, 10]);
    // Fold All right away folds the moved ranges.
    editor.update(cx, |e, cx| e.fold_all(cx));
    assert_eq!(collapsed(&editor, cx), [(3, 6), (4, 5), (8, 9), (10, 11)]);
    // And they stay folded once the ranges are recomputed.
    settle(cx);
    assert_eq!(ranges(&editor, cx), [(3, 6), (4, 5), (8, 9), (10, 11)]);
    assert_eq!(collapsed(&editor, cx), [(3, 6), (4, 5), (8, 9), (10, 11)]);
}

/// Monaco keeps lines inserted into a hidden area hidden, and a region
/// collapsed unless its lines changed with a cursor in it: Replace All or
/// Format Document don't unfold.
#[gpui::test]
fn edits_inside_folds_keep_them_folded(cx: &mut TestAppContext) {
    let (editor, cx) = setup(NESTED, cx);
    editor.update(cx, |e, cx| e.fold_all(cx));
    cursor(&editor, cx, 0);
    let all = [(0, 3), (1, 2), (5, 6), (7, 8)];
    assert_eq!(collapsed(&editor, cx), all);
    // Edits in hidden lines, as Replace All makes, one adding a line.
    editor.update(cx, |e, cx| {
        e.execute_edits(
            vec![
                (PointUtf16::new(2, 8)..PointUtf16::new(2, 9), "w".into()),
                (
                    PointUtf16::new(8, 4)..PointUtf16::new(8, 5),
                    "z1();\n    z2".into(),
                ),
            ],
            None,
            cx,
        );
    });
    let moved = [(0, 3), (1, 2), (5, 6), (7, 9)];
    assert_eq!(collapsed(&editor, cx), moved);
    settle(cx);
    assert_eq!(collapsed(&editor, cx), moved);
}

/// Monaco `sanitizeAndMerge`: a collapsed region takes its recomputed
/// range when the start line matches.
#[gpui::test]
fn a_collapsed_region_follows_its_recomputed_range(cx: &mut TestAppContext) {
    let (editor, cx) = setup("a\n  b\n  c\nd\ne\n", cx);
    editor.update(cx, |e, cx| {
        let mut options = e.options().clone();
        options.folding_strategy = FoldingStrategy::Indentation;
        e.update_options(options, cx);
    });
    append_line(&editor, cx);
    assert_eq!(ranges(&editor, cx), [(0, 2)]);
    fold_at(&editor, cx, 0);
    cursor(&editor, cx, 5);
    // Indent the line after the block into it: it hides with the rest.
    let at = PointUtf16::new(3, 0);
    editor.update(cx, |e, cx| {
        e.execute_edits(vec![(at..at, "  ".into())], None, cx);
    });
    settle(cx);
    assert_eq!(ranges(&editor, cx), [(0, 3)]);
    assert_eq!(collapsed(&editor, cx), [(0, 3)]);
}

/// Crossing ranges from a provider.
struct Crossing;
impl FoldingRangeProvider for Crossing {
    fn provide_folding_ranges(
        &self,
        _: &ProviderContext,
        _: &FoldingContext,
        _: &mut App,
    ) -> Task<anyhow::Result<Option<Vec<FoldingRange>>>> {
        let range = |start_row, end_row| FoldingRange {
            start_row,
            end_row,
            kind: None,
        };
        Task::ready(Ok(Some(vec![range(0, 3), range(1, 6), range(2, 3)])))
    }
}

/// Monaco `FoldingModel.update` drops a range not inside its parent.
#[gpui::test]
fn crossing_ranges_are_dropped(cx: &mut TestAppContext) {
    cx.update(|cx| {
        register_folding_range_provider(cx, LanguageSelector::All, Crossing);
    });
    let (editor, cx) = setup(NESTED, cx);
    assert_eq!(ranges(&editor, cx), [(0, 3), (2, 3)]);
}

/// Monaco's `setValue` drops the folding decorations: everything unfolds.
#[gpui::test]
fn set_value_unfolds(cx: &mut TestAppContext) {
    let (editor, cx) = setup(NESTED, cx);
    editor.update(cx, |e, cx| e.fold_all(cx));
    editor.update(cx, |e, cx| e.set_value(NESTED, cx));
    assert!(collapsed(&editor, cx).is_empty());
    settle(cx);
    assert_eq!(ranges(&editor, cx), [(0, 3), (1, 2), (5, 6), (7, 8)]);
    assert!(collapsed(&editor, cx).is_empty());
}

/// An editor with `folding_imports_by_default`, for `text`.
fn setup_imports<'a>(
    text: &str,
    cx: &'a mut TestAppContext,
) -> (Entity<CodeEditor>, &'a mut VisualTestContext) {
    cx.update(gpui_monaco::init);
    let text = text.to_string();
    let (editor, cx) = cx.add_window_view(|window, cx| {
        let model = cx.new(|_| TextModel::new(&text).with_language("rust"));
        let mut editor = CodeEditor::new(model, window, cx);
        editor.update_options_with(|o| o.folding_imports_by_default = true, cx);
        editor
    });
    cx.update(|window, cx| window.focus(&editor.focus_handle(cx), cx));
    settle(cx);
    (editor, cx)
}

/// Monaco `foldingImportsByDefault` keeps trying until imports were
/// folded: a provider registered after the editor opened folds them, and
/// registering one recomputes the ranges without an edit.
#[gpui::test]
fn imports_fold_when_a_provider_registers_late(cx: &mut TestAppContext) {
    let (editor, cx) = setup_imports(NESTED, cx);
    assert!(collapsed(&editor, cx).is_empty());
    let registration =
        cx.update(|_, cx| register_folding_range_provider(cx, LanguageSelector::All, Imports));
    settle(cx);
    assert_eq!(ranges(&editor, cx), [(0, 2), (5, 6)]);
    assert_eq!(collapsed(&editor, cx), [(0, 2)]);
    // Once folded, unfolding them sticks.
    editor.update(cx, |e, cx| e.toggle_import_fold(cx));
    append_line(&editor, cx);
    assert!(collapsed(&editor, cx).is_empty());
    // Unregistering recomputes too: back to the syntax ranges.
    cx.update(|_, cx| unregister_provider(cx, registration));
    settle(cx);
    assert_eq!(ranges(&editor, cx), [(0, 3), (1, 2), (5, 6), (7, 8)]);
}

/// Monaco saves `foldedImports` in the view state: restored right after
/// `set_model`, folded imports stay folded and unfolded ones aren't folded
/// again.
#[gpui::test]
fn restored_view_states_remember_folded_imports(cx: &mut TestAppContext) {
    cx.update(|cx| {
        register_folding_range_provider(cx, LanguageSelector::All, Imports);
    });
    let (editor, cx) = setup_imports(NESTED, cx);
    assert_eq!(collapsed(&editor, cx), [(0, 2)]);
    let folded = editor.read_with(cx, |e, cx| e.save_view_state(cx));
    assert!(folded.folding.folded_imports);
    editor.update(cx, |e, cx| e.toggle_import_fold(cx));
    let unfolded = editor.read_with(cx, |e, cx| e.save_view_state(cx));
    assert!(unfolded.folding.folded_imports);
    assert!(unfolded.folding.regions.is_empty());

    for (state, expected) in [(&folded, vec![(0, 2)]), (&unfolded, vec![])] {
        editor.update(cx, |e, cx| {
            let model = cx.new(|_| TextModel::new(NESTED).with_language("rust"));
            e.set_model(model, cx);
            e.restore_view_state(state, cx);
        });
        settle(cx);
        assert_eq!(collapsed(&editor, cx), expected);
    }
}

/// Ranges from a provider, without kinds.
struct Untyped;
impl FoldingRangeProvider for Untyped {
    fn provide_folding_ranges(
        &self,
        _: &ProviderContext,
        _: &FoldingContext,
        _: &mut App,
    ) -> Task<anyhow::Result<Option<Vec<FoldingRange>>>> {
        let range = |start_row, end_row| FoldingRange {
            start_row,
            end_row,
            kind: None,
        };
        Task::ready(Ok(Some(vec![range(0, 3), range(5, 6), range(7, 8)])))
    }
}

/// Monaco `FoldingRegions.fromFoldRanges` drops the types when no range
/// has one: Fold All Block Comments then goes by the comment start.
#[gpui::test]
fn providers_without_kinds_have_no_types(cx: &mut TestAppContext) {
    cx.update(|cx| {
        register_folding_range_provider(cx, LanguageSelector::All, Untyped);
    });
    let (editor, cx) = setup(NESTED, cx);
    assert_eq!(ranges(&editor, cx), [(0, 3), (5, 6), (7, 8)]);
    editor.update(cx, |e, cx| e.fold_all_block_comments(cx));
    assert_eq!(collapsed(&editor, cx), [(5, 6)]);
}

/// Monaco `onFoldingStrategyChanged`: changing `folding_strategy`
/// recomputes the ranges without waiting for an edit.
#[gpui::test]
fn changing_the_strategy_recomputes(cx: &mut TestAppContext) {
    cx.update(|cx| {
        register_folding_range_provider(cx, LanguageSelector::All, Untyped);
    });
    let (editor, cx) = setup(NESTED, cx);
    assert_eq!(ranges(&editor, cx), [(0, 3), (5, 6), (7, 8)]);
    editor.update(cx, |e, cx| {
        e.update_options_with(|o| o.folding_strategy = FoldingStrategy::Indentation, cx)
    });
    settle(cx);
    assert_eq!(ranges(&editor, cx), [(0, 3), (1, 2), (5, 6), (7, 8)]);
}

/// Monaco `onDidChangeModelLanguageConfiguration`: a language change
/// recomputes the ranges.
#[gpui::test]
fn changing_the_language_recomputes(cx: &mut TestAppContext) {
    // Python folds off-side: the blank line after a block isn't part of it.
    let (editor, cx) = setup("a:\n    b\n\nc\n", cx);
    editor.update(cx, |e, cx| {
        e.update_options_with(|o| o.folding_strategy = FoldingStrategy::Indentation, cx)
    });
    settle(cx);
    assert_eq!(ranges(&editor, cx), [(0, 2)]);
    let model = editor.read_with(cx, |e, _| e.model().clone());
    model.update(cx, |m, cx| m.set_language(Some("python".into()), cx));
    settle(cx);
    assert_eq!(ranges(&editor, cx), [(0, 1)]);
}

/// Monaco discards the folds when `folding` is turned off, and starts
/// over when it's turned back on.
#[gpui::test]
fn turning_folding_off_discards_the_folds(cx: &mut TestAppContext) {
    let (editor, cx) = setup(NESTED, cx);
    editor.update(cx, |e, cx| e.fold_all(cx));
    assert_eq!(collapsed(&editor, cx).len(), 4);
    editor.update(cx, |e, cx| e.update_options_with(|o| o.folding = false, cx));
    settle(cx);
    assert!(collapsed(&editor, cx).is_empty());
    assert!(ranges(&editor, cx).is_empty());
    // Go to fold commands do nothing.
    cursor(&editor, cx, 2);
    editor.update(cx, |e, cx| e.go_to_parent_fold(cx));
    assert_eq!(head(&editor, cx).row, 2);
    editor.update(cx, |e, cx| e.update_options_with(|o| o.folding = true, cx));
    settle(cx);
    assert_eq!(ranges(&editor, cx), [(0, 3), (1, 2), (5, 6), (7, 8)]);
    assert!(collapsed(&editor, cx).is_empty());
}

/// Monaco skips `adjustSelections` while restoring a view state: a cursor
/// saved inside a collapsed region stays there.
#[gpui::test]
fn restoring_a_view_state_keeps_cursors_in_folds(cx: &mut TestAppContext) {
    let (editor, cx) = setup(NESTED, cx);
    editor.update(cx, |e, cx| e.fold_all(cx));
    let mut state = editor.read_with(cx, |e, cx| e.save_view_state(cx));
    editor.update(cx, |e, cx| e.unfold_all(cx));
    state.selections = vec![gpui_monaco::cursor::Selection::collapsed(PointUtf16::new(
        8, 2,
    ))];
    editor.update(cx, |e, cx| e.restore_view_state(&state, cx));
    assert_eq!(collapsed(&editor, cx), [(0, 3), (1, 2), (5, 6), (7, 8)]);
    assert_eq!(head(&editor, cx), PointUtf16::new(8, 2));
}

/// Monaco `limitReporter`: how many ranges were found, and the limit they
/// were cut to, with an event when either changes.
#[gpui::test]
fn the_folding_limit_is_reported(cx: &mut TestAppContext) {
    use gpui_monaco::CodeEditorEvent;
    use gpui_monaco::view::folding::FoldingLimit;
    // Twelve functions, each with a nested block.
    let text = "fn a() {\n    if x {\n        y();\n    }\n}\n".repeat(12);
    let (editor, cx) = setup(&text, cx);
    let limit = |cx: &mut VisualTestContext| editor.read_with(cx, |e, _| e.folding_limit());
    assert_eq!(
        limit(cx),
        FoldingLimit {
            computed: 24,
            limited: None
        }
    );
    let changes = std::rc::Rc::new(std::cell::Cell::new(0));
    let sink = changes.clone();
    let _subscription = cx.update(|_, cx| {
        cx.subscribe(&editor, move |_, event: &CodeEditorEvent, _| {
            if *event == CodeEditorEvent::FoldingLimitChanged {
                sink.set(sink.get() + 1);
            }
        })
    });
    editor.update(cx, |e, cx| {
        e.update_options_with(|o| o.folding_maximum_regions = 15, cx)
    });
    settle(cx);
    assert_eq!(
        limit(cx),
        FoldingLimit {
            computed: 24,
            limited: Some(15)
        }
    );
    // Every function, then the first blocks.
    let kept = ranges(&editor, cx);
    assert_eq!(kept.len(), 15);
    assert_eq!(kept.iter().filter(|(start, _)| start % 5 == 0).count(), 12);
    assert_eq!(changes.get(), 1);
    // Unchanged: no event.
    append_line(&editor, cx);
    assert_eq!(changes.get(), 1);
}

/// Monaco `_executeFoldingRangeProvider`: a model's ranges as an editor
/// with the given options computes them.
#[gpui::test]
fn execute_folding_range_provider(cx: &mut TestAppContext) {
    cx.update(|cx| {
        gpui_monaco::init(cx);
        gpui_monaco::set_language_registry(cx, gpui_monaco_languages::registry());
        register_folding_range_provider(cx, LanguageSelector::All, Imports);
    });
    let model = cx.new(|_| TextModel::new(NESTED).with_language("rust"));
    let mut options = gpui_monaco::EditorOptions::default();
    let run = |options: &gpui_monaco::EditorOptions, cx: &mut TestAppContext| {
        let task = cx.update(|cx| {
            gpui_monaco::api_commands::execute_folding_range_provider(&model, options, cx)
        });
        cx.run_until_parked();
        cx.foreground_executor().block_test(task)
    };
    let ranges = run(&options, cx);
    assert_eq!(
        ranges
            .iter()
            .map(|r| (r.start_row, r.end_row, r.kind.clone()))
            .collect::<Vec<_>>(),
        [
            (0, 2, Some(FoldingRangeKind::Imports)),
            (5, 6, Some(FoldingRangeKind::Comment))
        ]
    );
    options.folding_strategy = FoldingStrategy::Indentation;
    options.folding_maximum_regions = 3;
    let ranges: Vec<_> = run(&options, cx)
        .iter()
        .map(|r| (r.start_row, r.end_row))
        .collect();
    assert_eq!(ranges, [(0, 3), (5, 6), (7, 8)]);
    options.folding = false;
    assert!(run(&options, cx).is_empty());
}

fn control(
    editor: &Entity<CodeEditor>,
    cx: &mut VisualTestContext,
    row: u32,
) -> gpui::Bounds<gpui::Pixels> {
    draw(cx);
    editor.read_with(cx, |e, _| {
        e.fold_controls_for_tests()
            .into_iter()
            .find(|(r, _, _)| *r == row)
            .map(|(_, bounds, _)| bounds)
            .unwrap()
    })
}

/// Monaco `onEditorMouseDown`/`onEditorMouseUp`: a chevron toggles on
/// release on the same line, not on press, and not from its first 4px.
#[gpui::test]
fn chevrons_toggle_on_mouse_up_on_the_same_line(cx: &mut TestAppContext) {
    use gpui::{Modifiers, MouseButton, point, px};
    let (editor, cx) = setup(NESTED, cx);
    let bounds = control(&editor, cx, 0);
    let at = point(bounds.right() - px(4.), bounds.center().y);
    cx.simulate_mouse_down(at, MouseButton::Left, Modifiers::default());
    assert!(collapsed(&editor, cx).is_empty());
    cx.simulate_mouse_up(at, MouseButton::Left, Modifiers::default());
    assert_eq!(collapsed(&editor, cx), [(0, 3)]);
    // Released on another line: nothing.
    editor.update(cx, |e, cx| e.unfold_all(cx));
    let other = control(&editor, cx, 5);
    cx.simulate_mouse_down(at, MouseButton::Left, Modifiers::default());
    cx.simulate_mouse_move(
        other.center(),
        Some(MouseButton::Left),
        Modifiers::default(),
    );
    cx.simulate_mouse_up(other.center(), MouseButton::Left, Modifiers::default());
    assert!(collapsed(&editor, cx).is_empty());
    // The dead zone at the control's left.
    let dead = point(bounds.left() + px(2.), bounds.center().y);
    cx.simulate_click(dead, Modifiers::default());
    assert!(collapsed(&editor, cx).is_empty());
    // Middle click folds recursively.
    cx.simulate_mouse_down(at, MouseButton::Middle, Modifiers::default());
    cx.simulate_mouse_up(at, MouseButton::Middle, Modifiers::default());
    assert_eq!(collapsed(&editor, cx), [(1, 2)]);
}

/// Monaco `onEditorMouseUp`: a click at the end of a collapsed line's text
/// unfolds it, and moves the cursor there.
#[gpui::test]
fn a_click_at_the_end_of_a_collapsed_line_unfolds_it(cx: &mut TestAppContext) {
    let (editor, cx) = setup(NESTED, cx);
    cursor(&editor, cx, 9);
    editor.update(cx, |e, cx| e.fold_all(cx));
    draw(cx);
    let end = editor.read_with(cx, |e, _| {
        e.scrolled_visible_position(PointUtf16::new(7, 8)).unwrap()
    });
    // Just left of the end of `fn b() {`, so on its last character's
    // right half.
    let at = gpui::point(end.x - gpui::px(2.), end.y + gpui::px(8.));
    cx.simulate_click(at, gpui::Modifiers::default());
    assert_eq!(head(&editor, cx), PointUtf16::new(7, 8));
    assert_eq!(collapsed(&editor, cx), [(0, 3), (1, 2), (5, 6)]);
}

/// Monaco's collapsed region decorations shade the minimap
/// (`foldedBackgroundMinimap`): the header's line, at half the color.
#[gpui::test]
fn the_fold_highlight_shades_the_minimap(cx: &mut TestAppContext) {
    let (editor, cx) = setup(NESTED, cx);
    editor.update(cx, |e, cx| {
        e.update_options_with(|o| o.minimap.enabled = true, cx)
    });
    cursor(&editor, cx, 9);
    let fold = editor.read_with(cx, |e, _| e.colors().fold_background);
    let shaded = |cx: &mut VisualTestContext| {
        draw(cx);
        editor.read_with(cx, |e, _| {
            e.minimap_marks_for_tests()
                .into_iter()
                .filter(|(_, color)| (color.h, color.s, color.l) == (fold.h, fold.s, fold.l))
                .map(|(bounds, _)| bounds.top())
                .collect::<Vec<_>>()
        })
    };
    assert!(shaded(cx).is_empty());
    cursor(&editor, cx, 9);
    editor.update(cx, |e, cx| e.fold_all(cx));
    // The two functions' and the comment's headers; the `if` is hidden.
    let mut tops = shaded(cx);
    tops.sort();
    tops.dedup();
    assert_eq!(tops.len(), 3, "{tops:?}");
    // Without `folding_highlight`, none.
    editor.update(cx, |e, cx| {
        e.update_options_with(|o| o.folding_highlight = false, cx)
    });
    assert!(shaded(cx).is_empty());
}

/// Ranges a test changes.
struct Changing(std::rc::Rc<std::cell::RefCell<Vec<FoldingRange>>>);
impl FoldingRangeProvider for Changing {
    fn provide_folding_ranges(
        &self,
        _: &ProviderContext,
        _: &FoldingContext,
        _: &mut App,
    ) -> Task<anyhow::Result<Option<Vec<FoldingRange>>>> {
        Task::ready(Ok(Some(self.0.borrow().clone())))
    }
}

/// Monaco `foldingDecorations.ts`: a collapsed region the provider no
/// longer gives is kept, recovered, with the manual icon.
#[gpui::test]
fn recovered_folds_get_the_manual_icon(cx: &mut TestAppContext) {
    use gpui_monaco::view::folding::FoldingIcon;
    let provided = std::rc::Rc::new(std::cell::RefCell::new(vec![FoldingRange {
        start_row: 0,
        end_row: 3,
        kind: None,
    }]));
    let provider = Changing(provided.clone());
    cx.update(|cx| {
        register_folding_range_provider(cx, LanguageSelector::All, provider);
    });
    let (editor, cx) = setup(NESTED, cx);
    let icons = |cx: &mut VisualTestContext| {
        draw(cx);
        editor.read_with(cx, |e, cx| {
            e.fold_control_icons_for_tests(cx)
                .into_iter()
                .map(|(row, icon, _)| (row, icon))
                .collect::<Vec<_>>()
        })
    };
    cursor(&editor, cx, 0);
    editor.update(cx, |e, cx| e.fold(cx));
    assert_eq!(icons(cx), [(0, FoldingIcon::Collapsed)]);
    provided.borrow_mut().clear();
    cx.update(|_, cx| refresh_folding_ranges(cx));
    settle(cx);
    assert_eq!(collapsed(&editor, cx), [(0, 3)]);
    assert_eq!(icons(cx), [(0, FoldingIcon::ManualCollapsed)]);
}
