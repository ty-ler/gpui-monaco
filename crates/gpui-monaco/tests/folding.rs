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
