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
