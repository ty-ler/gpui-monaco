//! The side-by-side diff editor (Monaco `DiffEditorWidget`, MIT): the
//! original and modified models in two editors, with changes decorated,
//! padding view zones keeping the sides aligned, and scrolling in sync.
//!
//! Ports `diffEditorWidget.ts`, `diffEditorDecorations.ts`,
//! `diffEditorViewZones.ts` (`computeRangeAlignment`) and the options in
//! `diffEditorOptions.ts`.

use std::cell::Cell;
use std::future::Future;
use std::ops::Range;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use gpui::{
    App, AppContext as _, Bounds, Context, Entity, EventEmitter, FocusHandle, Focusable, FontStyle,
    FontWeight, HighlightStyle, Hsla, InteractiveElement as _, IntoElement, MouseButton,
    ParentElement as _, PathBuilder, Pixels, Render, SharedString, StatefulInteractiveElement as _,
    Styled as _, StyledText, Subscription, Task, WeakEntity, Window, canvas, div, point, px,
};
use text::{Point, PointUtf16};

use crate::codicons::{Codicon, icon as codicon};
use crate::context_keys::ContextKeyValue;
use crate::cursor::Selection;
use crate::decorations::{BlockDecoration, DecorationId, DecorationOptions, InjectedText};
use crate::diff::{
    DetailedLineRangeMapping, DiffOptions, LineRange, LineRangeMapping, LinesDiff, RangeMapping,
    compute_diff, compute_legacy_diff,
};
use crate::editor::{CodeEditor, CodeEditorEvent, ViewState};
use crate::events::{ContentChange, ContentSizeChange};
use crate::model::{TextModel, TextModelEvent};
use crate::options::WordWrapOverride;
use crate::theme::EditorColors;
use crate::widgets::{ViewZone, WidgetId, ZoneHeight};

mod accessible;
mod unchanged;

use accessible::AccessibleDiffViewer;
#[doc(hidden)]
pub use accessible::AccessibleDiffViewerAria;

use unchanged::{RevealPreference, UnchangedRegion};

gpui::actions!(
    diff_editor,
    [
        /// Goes to the next change (Monaco `goToDiff('next')`; unbound, as
        /// in standalone Monaco, where Alt+F5 is VS Code's workbench key).
        GoToNextDiff,
        /// Goes to the previous change (Monaco `goToDiff('previous')`;
        /// unbound, Shift+Alt+F5 in VS Code).
        GoToPreviousDiff,
        /// Hides every unchanged region again (Monaco
        /// `diffEditor.collapseAllUnchangedRegions`).
        CollapseAllUnchangedRegions,
        /// Shows every unchanged region (Monaco
        /// `diffEditor.showAllUnchangedRegions`).
        ShowAllUnchangedRegions,
        /// Reverts the changes at the cursor or in the selection (Monaco
        /// `diffEditor.revert`).
        Revert,
        /// Opens the accessible diff viewer, or goes to its next change
        /// (Monaco `editor.action.accessibleDiffViewer.next`, F7).
        AccessibleDiffViewerNext,
        /// Opens the accessible diff viewer at its previous change (Monaco
        /// `editor.action.accessibleDiffViewer.prev`, Shift+F7).
        AccessibleDiffViewerPrev,
        /// Stops comparing a moved block (Monaco
        /// `diffEditor.exitCompareMove`, Escape).
        ExitCompareMove,
        /// Focuses the other side, with the selection mapped through the
        /// diff (Monaco `diffEditor.switchSide`).
        SwitchSide,
        /// Turns hiding unchanged regions on or off (Monaco
        /// `diffEditor.toggleCollapseUnchangedRegions`).
        ToggleCollapseUnchangedRegions,
        /// Turns showing moved code blocks on or off (Monaco
        /// `diffEditor.toggleShowMovedCodeBlocks`).
        ToggleShowMovedCodeBlocks,
        /// Turns the inline view for narrow diff editors on or off (Monaco
        /// `diffEditor.toggleUseInlineViewWhenSpaceIsLimited`).
        ToggleUseInlineViewWhenSpaceIsLimited,
    ]
);

/// Set on the diff editor's key context while comparing a moved block
/// (Monaco `comparingMovedCode`).
const COMPARING_MOVED_CODE: &str = "comparingMovedCode";

/// The key context of the diff editor.
pub const DIFF_EDITOR_KEY_CONTEXT: &str = "DiffEditor";

/// Set on the diff editor's key context with `is_in_embedded_editor`
/// (Monaco `isEmbeddedDiffEditor`).
const IS_EMBEDDED_DIFF_EDITOR: &str = "isEmbeddedDiffEditor";

/// Binds the diff editor's keys (called by [`crate::init`]).
pub(crate) fn bind_keys(cx: &mut App) {
    // In a diff editor F7 opens the accessible diff viewer instead of the
    // editor's next highlight.
    let in_editor = format!(
        "{DIFF_EDITOR_KEY_CONTEXT} > {}",
        crate::actions::KEY_CONTEXT
    );
    let comparing = format!(
        "{DIFF_EDITOR_KEY_CONTEXT} && {COMPARING_MOVED_CODE} > {}",
        crate::actions::KEY_CONTEXT
    );
    cx.bind_keys([
        gpui::KeyBinding::new("escape", ExitCompareMove, Some(comparing.as_str())),
        gpui::KeyBinding::new("f7", AccessibleDiffViewerNext, Some(in_editor.as_str())),
        gpui::KeyBinding::new(
            "shift-f7",
            AccessibleDiffViewerPrev,
            Some(in_editor.as_str()),
        ),
        gpui::KeyBinding::new(
            "f7",
            AccessibleDiffViewerNext,
            Some(DIFF_EDITOR_KEY_CONTEXT),
        ),
        gpui::KeyBinding::new(
            "shift-f7",
            AccessibleDiffViewerPrev,
            Some(DIFF_EDITOR_KEY_CONTEXT),
        ),
    ]);
}

/// Monaco `IDiffEditorBaseOptions`.
#[derive(Clone, Debug, PartialEq)]
pub struct DiffEditorOptions {
    /// Let the user resize the split (default `true`).
    pub enable_split_view_resizing: bool,
    /// The original side's share of the width (default 0.5), clamped to
    /// 0.1–0.9 as Monaco validates it.
    pub split_view_default_ratio: f32,
    /// Ignore leading and trailing whitespace changes (default `true`).
    pub ignore_trim_whitespace: bool,
    /// Give up computing the diff after this long (default 5000ms; 0 never
    /// gives up).
    pub max_computation_time_ms: u64,
    /// `+` and `-` signs in the gutter (default `true`).
    pub render_indicators: bool,
    /// The original side can be edited (default `false`).
    pub original_editable: bool,
    /// Mark changes in the overview rulers (default `true`).
    pub render_overview_ruler: bool,
    /// Detect moved blocks (Monaco `experimental.showMoves`, default
    /// `false`).
    pub show_moves: bool,
    /// Show the sides next to each other; `false` shows deleted lines inline
    /// in the modified editor (default `true`).
    pub render_side_by_side: bool,
    /// Show inline when the diff editor is narrower than
    /// `render_side_by_side_inline_breakpoint` (default `true`).
    pub use_inline_view_when_space_is_limited: bool,
    /// The width below which a side-by-side diff shows inline (default
    /// 900px).
    pub render_side_by_side_inline_breakpoint: Pixels,
    /// Collapse long runs of unchanged lines (Monaco
    /// `hideUnchangedRegions`).
    pub hide_unchanged_regions: HideUnchangedRegions,
    /// A gutter beside the modified editor with a "Revert Block" button for
    /// each change (Monaco `renderGutterMenu`, default `true`). It only shows
    /// when the modified side is editable.
    pub render_gutter_menu: bool,
    /// Revert arrows in the modified editor's glyph margin when there's no
    /// gutter menu (Monaco `renderMarginRevertIcon`, default `true`).
    pub render_margin_revert_icon: bool,
    /// Show only the accessible diff viewer, which can't be closed (Monaco
    /// `onlyShowAccessibleDiffViewer`, default `false`).
    pub only_show_accessible_diff_viewer: bool,
    /// Don't compute a diff when either model is larger than this many MB
    /// (Monaco `maxFileSize`, default 50; 0 for no limit). Such models show
    /// without changes, and [`DiffEditor::diff`] is `None`. Monaco documents
    /// the option this way, and its earlier diff widget enforced it, but its
    /// current widget only validates it; set 0 to match.
    pub max_file_size: u64,
    /// Optimize for small views (Monaco `compactMode`, default `false`): a
    /// diff of only insertions and deletions shows inline, and hidden
    /// unchanged regions get thin "N hidden lines" separators, none at the
    /// start or end.
    pub compact_mode: bool,
    /// Override both editors' word wrap (Monaco `diffWordWrap`, default
    /// inherit): sets their [`word_wrap_override1`](crate::EditorOptions::word_wrap_override1).
    pub diff_word_wrap: WordWrapOverride,
    /// The diff algorithm (Monaco `diffAlgorithm`, default advanced).
    pub diff_algorithm: DiffAlgorithm,
    /// Show code lenses in both editors (Monaco `diffCodeLens`, default
    /// `false`): sets their [`code_lens`](crate::EditorOptions::code_lens).
    pub diff_code_lens: bool,
    /// Add a navigation tip to both editors' screen reader labels (Monaco
    /// `accessibilityVerbose`, default `false`).
    pub accessibility_verbose: bool,
    /// Mark where characters were inserted or deleted, when the other side
    /// has none, with a thin bar (Monaco `experimental.showEmptyDecorations`,
    /// default `true`).
    pub show_empty_decorations: bool,
    /// In the inline view, show changes within lines as struck-through
    /// deleted text before the inserted text instead of the deleted lines
    /// above (Monaco `experimental.useTrueInlineView`, default `false`).
    /// Only changes whose parts each stay on one line.
    pub use_true_inline_view: bool,
    /// In the inline view, don't show the original line numbers left of the
    /// modified editor (Monaco `hideOriginalLineNumbers`, default `false`;
    /// `compact_mode` implies it).
    pub hide_original_line_numbers: bool,
    /// The diff editor is embedded in another editor, e.g. a peek view
    /// (Monaco `isInEmbeddedEditor`, default `false`): sets the
    /// `isEmbeddedDiffEditor` key context.
    pub is_in_embedded_editor: bool,
    /// The original editor's screen reader label (Monaco
    /// `originalAriaLabel`); `None` keeps the editor's own.
    pub original_aria_label: Option<SharedString>,
    /// The modified editor's screen reader label (Monaco
    /// `modifiedAriaLabel`); `None` keeps the editor's own.
    pub modified_aria_label: Option<SharedString>,
}

/// Monaco `diffAlgorithm`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum DiffAlgorithm {
    /// Monaco's legacy line diff (`LegacyLinesDiffComputer`), which never
    /// detects moves.
    Legacy,
    /// Monaco's default line diff (`DefaultLinesDiffComputer`).
    #[default]
    Advanced,
}

/// A diff editor's view state (Monaco `IDiffEditorViewState`), from
/// [`DiffEditor::save_view_state`].
#[derive(Clone, Debug, PartialEq)]
pub struct DiffEditorViewState {
    /// The original editor's selections and scroll position.
    pub original: ViewState,
    /// The modified editor's.
    pub modified: ViewState,
    /// The hidden unchanged ranges of the modified side, as 1-based line
    /// ranges (Monaco's serialized `collapsedRegions`).
    pub collapsed_regions: Vec<LineRange>,
}

/// Monaco `hideUnchangedRegions`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HideUnchangedRegions {
    /// Hide unchanged regions (default `false`).
    pub enabled: bool,
    /// Unchanged lines kept around each change (default 3).
    pub context_line_count: usize,
    /// Only hide runs of at least this many lines (default 3).
    pub minimum_line_count: usize,
    /// Lines a click on a bar's edge reveals (default 20).
    pub reveal_line_count: usize,
}

impl Default for HideUnchangedRegions {
    fn default() -> Self {
        Self {
            enabled: false,
            context_line_count: 3,
            minimum_line_count: 3,
            reveal_line_count: 20,
        }
    }
}

impl Default for DiffEditorOptions {
    fn default() -> Self {
        Self {
            enable_split_view_resizing: true,
            split_view_default_ratio: 0.5,
            ignore_trim_whitespace: true,
            max_computation_time_ms: 5000,
            render_indicators: true,
            original_editable: false,
            render_overview_ruler: true,
            show_moves: false,
            render_side_by_side: true,
            use_inline_view_when_space_is_limited: true,
            render_side_by_side_inline_breakpoint: px(900.),
            hide_unchanged_regions: HideUnchangedRegions::default(),
            render_gutter_menu: true,
            render_margin_revert_icon: true,
            only_show_accessible_diff_viewer: false,
            max_file_size: 50,
            compact_mode: false,
            diff_word_wrap: WordWrapOverride::Inherit,
            diff_algorithm: DiffAlgorithm::Advanced,
            diff_code_lens: false,
            accessibility_verbose: false,
            show_empty_decorations: true,
            use_true_inline_view: false,
            hide_original_line_numbers: false,
            is_in_embedded_editor: false,
            original_aria_label: None,
            modified_aria_label: None,
        }
    }
}

/// Events a [`DiffEditor`] emits.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DiffEditorEvent {
    /// A new diff is shown (Monaco `onDidUpdateDiff`).
    DiffUpdated,
    /// [`DiffEditor::set_models`] showed other models (Monaco
    /// `onDidChangeModel`).
    ModelChanged,
    /// A sound or announcement cue for screen reader users (Monaco
    /// `IAccessibilitySignalService.playSignal`). Standalone Monaco ignores
    /// these; hosts may play a sound, as VS Code does.
    AccessibilitySignal(AccessibilitySignal),
    /// Either editor's content size changed (Monaco
    /// `IDiffEditor.onDidContentSizeChange`): `content_height` is
    /// [`DiffEditor::content_height`] and `content_width`
    /// [`DiffEditor::content_width`]; the `changed` flags are the editor's.
    /// Reported after the frame that changed it, so a host can size an
    /// auto-height diff editor to its content.
    ContentSizeChanged(ContentSizeChange),
}

/// Monaco `AccessibilitySignal`s a diff editor sends.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AccessibilitySignal {
    /// The accessible diff viewer, `go_to_diff` or a cursor move reached
    /// inserted lines (`diffLineInserted`).
    DiffLineInserted,
    /// The accessible diff viewer, `go_to_diff` or a cursor move reached
    /// deleted lines (`diffLineDeleted`).
    DiffLineDeleted,
    /// `go_to_diff` or a cursor move reached changed lines
    /// (`diffLineModified`).
    DiffLineModified,
}

/// Monaco's delay after edits before the diff is computed again.
const RECOMPUTE_DELAY: Duration = Duration::from_millis(200);

/// Monaco's `diff-aria-navigation-tip`, added to the editors' labels with
/// `accessibility_verbose`. Monaco names the accessibility help's key; this
/// port has no accessibility help, so it names the accessible diff viewer.
const ARIA_NAVIGATION_TIP: &str = " use F7 to open the accessible diff viewer.";

/// Monaco `DiffEditorEditors._updateAriaLabel`.
fn aria_label_with_tip(label: &str, verbose: bool) -> SharedString {
    let label = label.replace(ARIA_NAVIGATION_TIP, "");
    if verbose {
        format!("{label}{ARIA_NAVIGATION_TIP}").into()
    } else {
        label.into()
    }
}

/// Monaco `allowsTrueInlineDiffRendering`: every inner change stays on one
/// line on both sides (or inserts at the very start).
fn allows_true_inline_diff_rendering(mapping: &DetailedLineRangeMapping) -> bool {
    let single_line = |range: &crate::diff::Range| range.start_line_number == range.end_line_number;
    !mapping.inner_changes.is_empty()
        && mapping.inner_changes.iter().all(|c| {
            (single_line(&c.modified_range) && single_line(&c.original_range))
                || c.original_range == crate::diff::Range::new(1, 1, 1, 1)
        })
}

/// Monaco `normalizeDocumentDiff`, which the diff editor's view model runs
/// on every computed diff: an inner change covering whole lines on both
/// sides, from column 1 to the line end and not on the last line, takes
/// the line break too (ends at `(end line + 1, 1)`). The change then fills
/// to the right edge (`shouldFillLineOnLineBreak`), isn't shown by the true
/// inline view, and reverting it restores the line break. `original` and
/// `modified` are the texts' lines; moves are left as they are.
pub(crate) fn normalize_document_diff<S: AsRef<str>>(
    mut diff: LinesDiff,
    original: &[S],
    modified: &[S],
) -> LinesDiff {
    // Monaco `getLineMaxColumn` (UTF-16 columns) and `getLineCount`.
    let max_column = |lines: &[S], line: usize| {
        lines
            .get(line.wrapping_sub(1))
            .map_or(1, |l| l.as_ref().encode_utf16().count() + 1)
    };
    let line_count = |lines: &[S]| lines.len().max(1);
    for change in &mut diff.changes {
        for inner in &mut change.inner_changes {
            let (o, m) = (&mut inner.original_range, &mut inner.modified_range);
            if o.start_column == 1
                && m.start_column == 1
                && (o.end_column != 1 || m.end_column != 1)
                && o.end_column == max_column(original, o.end_line_number)
                && m.end_column == max_column(modified, m.end_line_number)
                && o.end_line_number < line_count(original)
                && m.end_line_number < line_count(modified)
            {
                o.end_line_number += 1;
                o.end_column = 1;
                m.end_line_number += 1;
                m.end_column = 1;
            }
        }
    }
    diff
}

/// Monaco `ILineRangeAlignment`: line ranges of the two sides shown with
/// the same height.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct LineRangeAlignment {
    pub original_range: LineRange,
    pub modified_range: LineRange,
    pub original_height: f32,
    pub modified_height: f32,
    /// The index of the change it's part of (Monaco's `diff`), if any.
    pub diff: Option<usize>,
}

/// Monaco `computeRangeAlignment`: the ranges to align, given each side's
/// extra heights after lines (wrapping), as (1-based line, pixels).
pub(crate) fn compute_range_alignment(
    diffs: &[DetailedLineRangeMapping],
    original_extra: &[(usize, f32)],
    modified_extra: &[(usize, f32)],
    original_line_height: f32,
    modified_line_height: f32,
    original_line_max_column: &dyn Fn(usize) -> Option<usize>,
    inner_hunk_alignment: bool,
) -> Vec<LineRangeAlignment> {
    let mut original_extra = std::collections::VecDeque::from(original_extra.to_vec());
    let mut modified_extra = std::collections::VecDeque::from(modified_extra.to_vec());
    let mut result = Vec::new();
    let mut last_original = 0usize;
    let mut last_modified = 0usize;

    let outside_diffs = |until_original: usize,
                         until_modified: usize,
                         original_extra: &mut std::collections::VecDeque<(usize, f32)>,
                         modified_extra: &mut std::collections::VecDeque<(usize, f32)>,
                         last_original: usize,
                         last_modified: usize,
                         result: &mut Vec<LineRangeAlignment>| loop {
        let mut original_next = original_extra
            .front()
            .copied()
            .filter(|(l, _)| *l < until_original);
        let mut modified_next = modified_extra
            .front()
            .copied()
            .filter(|(l, _)| *l < until_modified);
        if original_next.is_none() && modified_next.is_none() {
            break;
        }
        let dist_original = original_next.map_or(usize::MAX, |(l, _)| l - last_original);
        let dist_modified = modified_next.map_or(usize::MAX, |(l, _)| l - last_modified);
        if dist_original < dist_modified {
            original_extra.pop_front();
            let (l, _) = original_next.unwrap();
            modified_next = Some((l - last_original + last_modified, 0.));
        } else if dist_original > dist_modified {
            modified_extra.pop_front();
            let (l, _) = modified_next.unwrap();
            original_next = Some((l - last_modified + last_original, 0.));
        } else {
            original_extra.pop_front();
            modified_extra.pop_front();
        }
        let (original_line, original_px) = original_next.unwrap();
        let (modified_line, modified_px) = modified_next.unwrap();
        result.push(LineRangeAlignment {
            original_range: LineRange::new(original_line, original_line + 1),
            modified_range: LineRange::new(modified_line, modified_line + 1),
            original_height: original_line_height + original_px,
            modified_height: modified_line_height + modified_px,
            diff: None,
        });
    };

    for (diff_index, c) in diffs.iter().enumerate() {
        outside_diffs(
            c.original.start_line_number,
            c.modified.start_line_number,
            &mut original_extra,
            &mut modified_extra,
            last_original,
            last_modified,
            &mut result,
        );
        let mut first = true;
        let mut last_mod = c.modified.start_line_number;
        let mut last_orig = c.original.start_line_number;
        let mut emit = |orig_exclusive: usize,
                        mod_exclusive: usize,
                        force: bool,
                        original_extra: &mut std::collections::VecDeque<(usize, f32)>,
                        modified_extra: &mut std::collections::VecDeque<(usize, f32)>,
                        result: &mut Vec<LineRangeAlignment>| {
            if orig_exclusive < last_orig || mod_exclusive < last_mod {
                return;
            }
            if first {
                first = false;
            } else if !force && (orig_exclusive == last_orig || mod_exclusive == last_mod) {
                return;
            }
            let original_range = LineRange::new(last_orig, orig_exclusive);
            let modified_range = LineRange::new(last_mod, mod_exclusive);
            if original_range.is_empty() && modified_range.is_empty() {
                return;
            }
            let take = |extra: &mut std::collections::VecDeque<(usize, f32)>, until: usize| {
                let mut sum = 0.;
                while extra.front().is_some_and(|(l, _)| *l < until) {
                    sum += extra.pop_front().unwrap().1;
                }
                sum
            };
            let original_additional = take(original_extra, orig_exclusive);
            let modified_additional = take(modified_extra, mod_exclusive);
            result.push(LineRangeAlignment {
                original_height: original_range.len() as f32 * original_line_height
                    + original_additional,
                modified_height: modified_range.len() as f32 * modified_line_height
                    + modified_additional,
                original_range,
                modified_range,
                diff: Some(diff_index),
            });
            last_orig = orig_exclusive;
            last_mod = mod_exclusive;
        };
        if inner_hunk_alignment {
            for i in &c.inner_changes {
                if i.original_range.start_column > 1 && i.modified_range.start_column > 1 {
                    // Unchanged text on the line before the change.
                    emit(
                        i.original_range.start_line_number,
                        i.modified_range.start_line_number,
                        false,
                        &mut original_extra,
                        &mut modified_extra,
                        &mut result,
                    );
                }
                let max_column = original_line_max_column(i.original_range.end_line_number)
                    .unwrap_or(usize::MAX);
                if i.original_range.end_column < max_column {
                    // Unchanged text on the line after the change.
                    emit(
                        i.original_range.end_line_number,
                        i.modified_range.end_line_number,
                        false,
                        &mut original_extra,
                        &mut modified_extra,
                        &mut result,
                    );
                }
            }
        }
        emit(
            c.original.end_line_number_exclusive,
            c.modified.end_line_number_exclusive,
            true,
            &mut original_extra,
            &mut modified_extra,
            &mut result,
        );
        last_original = c.original.end_line_number_exclusive;
        last_modified = c.modified.end_line_number_exclusive;
    }
    outside_diffs(
        usize::MAX,
        usize::MAX,
        &mut original_extra,
        &mut modified_extra,
        last_original,
        last_modified,
        &mut result,
    );
    result
}

/// Monaco's `diagonal-fill` view zone: stripes where the other side has
/// lines.
struct DiagonalFill {
    color: Hsla,
}

impl Render for DiagonalFill {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let color = self.color;
        canvas(
            |_, _, _| {},
            move |bounds, _, window, _| {
                // 45° lines 8px apart, like Monaco's CSS gradient.
                let mut builder = PathBuilder::stroke(px(1.));
                let height = bounds.size.height;
                let mut x = bounds.left() - height;
                while x < bounds.right() {
                    builder.move_to(point(x, bounds.bottom()));
                    builder.line_to(point(x + height, bounds.top()));
                    x += px(8.);
                }
                if let Ok(path) = builder.build() {
                    window.with_content_mask(Some(gpui::ContentMask { bounds }), |window| {
                        window.paint_path(path, color)
                    });
                }
            },
        )
        .size_full()
    }
}

/// The height of a moved block's caption (Monaco's 18px placeholder zone).
const MOVE_CAPTION_HEIGHT: f32 = 18.;
/// The arrow at the end of a moved block's line.
const MOVE_ARROW_WIDTH: f32 = 15.;

/// Sorts `(line, pixels)` extra heights and sums those of the same line.
fn merge_extra_heights(extra: &mut Vec<(usize, f32)>) {
    extra.sort_by_key(|(line, _)| *line);
    extra.dedup_by(|b, a| {
        if a.0 == b.0 {
            a.1 += b.1;
            true
        } else {
            false
        }
    });
}

/// Monaco `LinesLayout.compute`: a track for each line spanning
/// `top..bottom`, the first whose lines it doesn't overlap, with at most 6
/// tracks (then the least overlapped).
fn move_line_tracks(lines: &[(Pixels, Pixels)]) -> Vec<usize> {
    const MAX_TRACKS: usize = 6;
    let mut tracks: Vec<Vec<(Pixels, Pixels)>> = Vec::new();
    let overlap =
        |a: &(Pixels, Pixels), b: &(Pixels, Pixels)| (a.1.min(b.1) - a.0.max(b.0)).max(px(0.));
    lines
        .iter()
        .map(|line| {
            let free = tracks
                .iter()
                .position(|set| set.iter().all(|r| overlap(r, line) == px(0.)));
            let ix = match free {
                Some(ix) => ix,
                None if tracks.len() < MAX_TRACKS => {
                    tracks.push(Vec::new());
                    tracks.len() - 1
                }
                None => (0..tracks.len())
                    .max_by(|&a, &b| {
                        let length = |t: usize| {
                            tracks[t]
                                .iter()
                                .map(|r| overlap(r, line))
                                .fold(px(0.), |a, b| a + b)
                        };
                        length(a)
                            .partial_cmp(&length(b))
                            .unwrap_or(std::cmp::Ordering::Equal)
                    })
                    .unwrap_or(0),
            };
            tracks[ix].push(*line);
            ix
        })
        .collect()
}

/// A moved block's caption (Monaco `MovedBlockOverlayWidget`'s
/// `.diff-moved-code-block`): right aligned, 4px up into the line above,
/// clear of the vertical scrollbar, with the Compare toggle.
struct MoveCaption {
    text: SharedString,
    colors: EditorColors,
    /// Its block is being compared (the toggle is checked).
    comparing: bool,
    diff: WeakEntity<DiffEditor>,
    /// The editor it's in, for its scrollbar width.
    editor: WeakEntity<CodeEditor>,
    index: usize,
}

impl Render for MoveCaption {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let (diff, index, comparing) = (self.diff.clone(), self.index, self.comparing);
        let scrollbar = self
            .editor
            .upgrade()
            .and_then(|e| {
                e.read(cx)
                    .layout
                    .as_ref()
                    .map(|l| l.layout_info.vertical_scrollbar_width)
            })
            .unwrap_or(px(0.));
        let colors = &self.colors;
        // Monaco's `actionBar.toggledBackground` (`inputOption.activeBackground`).
        let checked = colors.input_option_active_background;
        div()
            .size_full()
            .flex()
            .justify_end()
            .items_center()
            .mt(px(-4.))
            .pr(px(4.) + scrollbar)
            .gap(px(4.))
            .text_size(px(12.))
            .text_color(colors.foreground)
            .child(div().opacity(0.8).child(self.text.clone()))
            .child(
                div()
                    .id("compare-move")
                    .tooltip(crate::tooltip::tooltip("Compare", colors))
                    .px(px(2.))
                    .rounded(px(3.))
                    .cursor_pointer()
                    .when(comparing, |button| button.bg(checked))
                    .hover(|style| colors.toolbar_hover(style))
                    .role(gpui::Role::Button)
                    .aria_label(SharedString::from("Compare"))
                    .on_click(move |_, _, cx| {
                        if let Some(diff) = diff.upgrade() {
                            // Deferred: the caption is rendered by an editor
                            // the toggle changes.
                            cx.defer(move |cx| {
                                diff.update(cx, |diff, cx| {
                                    diff.set_move_to_compare((!comparing).then_some(index), cx)
                                })
                            });
                        }
                    })
                    // The moved block's `compare-changes` action, 12px.
                    .child(codicon(Codicon::CompareChanges).size(px(12.))),
            )
    }
}

/// What an alignment zone shows (Monaco's zones in `DiffEditorViewZones`).
#[derive(Clone, Debug, PartialEq)]
enum AlignmentZoneKind {
    /// Monaco's `diagonal-fill`.
    Fill,
    /// A deletion's padding with a revert arrow in its margin (change
    /// index).
    DeletionPadding(usize),
    /// The inline view's deleted lines (Monaco's `line-delete` zone), with
    /// each line's view line count.
    DeletedCode {
        change: usize,
        line_counts: Vec<usize>,
    },
    /// The inline view's padding in the original column, with a
    /// `gutter-delete` margin.
    GutterDelete,
}

/// An alignment zone (Monaco `IObservableViewZone`).
#[derive(Clone, Debug, PartialEq)]
struct AlignmentZone {
    /// In the original editor (else the modified).
    original: bool,
    /// Monaco `afterLineNumber`.
    after: usize,
    /// Monaco `heightInPx`.
    height: f32,
    kind: AlignmentZoneKind,
}

/// The inline view's padding in the original column (Monaco's fill zone
/// with a `gutter-delete` margin): the margin in the removed gutter color.
struct GutterDeleteFill {
    fill: Entity<DiagonalFill>,
    /// The original editor, for its gutter.
    editor: WeakEntity<CodeEditor>,
}

impl Render for GutterDeleteFill {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let Some(editor) = self.editor.upgrade() else {
            return div();
        };
        let editor = editor.read(cx);
        let (gutter, text_left) = editor.layout.as_ref().map_or((px(0.), px(0.)), |l| {
            (l.gutter_width, l.text_bounds.left() - l.bounds.left())
        });
        let background = editor.colors().diff_gutter_removed_line_background;
        div()
            .size_full()
            .flex()
            .flex_row()
            .child(
                div()
                    .w(text_left)
                    .h_full()
                    .flex_none()
                    .child(div().w(gutter).h_full().bg(background)),
            )
            .child(div().flex_1().h_full().child(self.fill.clone()))
    }
}

/// The changes' line mappings.
fn diff_mappings(diff: &LinesDiff) -> Vec<LineRangeMapping> {
    diff.changes
        .iter()
        .map(|c| c.line_range_mapping())
        .collect()
}

/// Monaco `RevertButton`: an arrow that reverts a change or the
/// selection.
#[derive(Clone)]
struct RevertArrow {
    diff: WeakEntity<DiffEditor>,
    revert: GutterRevert,
    colors: EditorColors,
}

impl RevertArrow {
    fn element(&self, id: &'static str) -> gpui::Stateful<gpui::Div> {
        let (diff, revert) = (self.diff.clone(), self.revert.clone());
        let label = match self.revert {
            GutterRevert::Block(_) => "Revert Change",
            GutterRevert::Selection(_) => "Revert Selected Changes",
        };
        div()
            .id(id)
            .size_full()
            .flex()
            .items_center()
            .justify_center()
            .cursor_pointer()
            .text_color(self.colors.foreground)
            .tooltip(crate::tooltip::tooltip(label, &self.colors))
            .role(gpui::Role::Button)
            .aria_label(SharedString::from(label))
            .on_click(move |_, _, cx| {
                let (diff, revert) = (diff.clone(), revert.clone());
                // Deferred: the arrow is rendered by the editor it changes.
                cx.defer(move |cx| {
                    diff.update(cx, |diff, cx| match &revert {
                        GutterRevert::Block(change) => diff.revert(change, cx),
                        GutterRevert::Selection(changes) => diff.revert_range_mappings(changes, cx),
                    })
                    .ok();
                });
            })
            .child(codicon(Codicon::ArrowRight))
    }
}

impl Render for RevertArrow {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        self.element("revert-arrow")
    }
}

/// A deletion's padding with a revert arrow in its margin (Monaco's
/// alignment zone with `createViewZoneMarginArrow`).
struct DeletionPadding {
    fill: Entity<DiagonalFill>,
    arrow: RevertArrow,
    /// The modified editor, for its gutter.
    editor: WeakEntity<CodeEditor>,
}

impl Render for DeletionPadding {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let (gutter, glyph) = self
            .editor
            .upgrade()
            .and_then(|e| {
                let layout = e.read(cx).layout.as_ref()?;
                Some((
                    layout.text_bounds.left() - layout.bounds.left(),
                    layout.glyph_margin_width,
                ))
            })
            .unwrap_or((px(0.), px(0.)));
        let line_height = self
            .editor
            .upgrade()
            .map_or(px(18.), |e| e.read(cx).line_height());
        div()
            .size_full()
            .flex()
            .flex_row()
            .child(
                div().w(gutter).h_full().flex_none().child(
                    div()
                        .w(glyph.max(px(16.)))
                        .h(line_height)
                        .child(self.arrow.element("deletion-revert-arrow")),
                ),
            )
            .child(div().flex_1().h_full().child(self.fill.clone()))
    }
}

/// The gutter menu's width (Monaco's 35px `DiffEditorGutter`).
const GUTTER_WIDTH: f32 = 35.;

/// The gutter's `.scroll-decoration` height and shadow size.
const GUTTER_SCROLL_SHADOW: f32 = 6.;
/// A gutter button's height.
const GUTTER_BUTTON_HEIGHT: f32 = 20.;

/// What a gutter button reverts.
#[derive(Clone)]
enum GutterRevert {
    Block(LineRangeMapping),
    Selection(Vec<RangeMapping>),
}

/// Monaco `DiffGutterItem`.
struct GutterItem {
    /// The modified lines it's beside.
    lines: LineRange,
    revert: GutterRevert,
    /// Shown without hovering the gutter (the cursor's change, or the
    /// selection).
    show_always: bool,
    /// What its toolbar's actions run on (Monaco's `mapping`).
    mapping: DetailedLineRangeMapping,
}

/// Monaco `DiffToolBar.layout`: the button's top for an item spanning
/// `top..bottom` in a viewport of `viewport` height. It centers on the item,
/// staying inside both while there's room.
fn gutter_button_top(top: Pixels, bottom: Pixels, viewport: Pixels, height: Pixels) -> Pixels {
    let margin = height;
    let mut button_top = top + (bottom - top) / 2. - height / 2.;
    let viewport_range = (margin, viewport - margin - height);
    let parent_range = (top + margin, bottom - height - margin);
    if viewport_range.0 < viewport_range.1 && parent_range.0 < parent_range.1 {
        button_top = button_top.clamp(viewport_range.0, viewport_range.1);
        button_top = button_top.clamp(parent_range.0, parent_range.1);
    }
    button_top
}

/// The original and modified ranges for reverting `change`: Monaco
/// `LineRange.toExclusiveRange` (the first line's start to the start of the
/// line after the last). At the end of the text, where there's no line
/// after, both start at the previous line's end instead, so the line break
/// moves with the lines.
fn revert_points(
    original: &text::Rope,
    modified: &text::Rope,
    change: &LineRangeMapping,
) -> (Range<PointUtf16>, Range<PointUtf16>) {
    let at_end = |rope: &text::Rope, lines: LineRange| {
        lines.end_line_number_exclusive > rope.line_count() as usize
    };
    let from_previous_line_end = change.original.start_line_number > 1
        && change.modified.start_line_number > 1
        && (at_end(original, change.original) || at_end(modified, change.modified));
    let points = |rope: &text::Rope, lines: LineRange| {
        let point = |line: usize| {
            if line > rope.line_count() as usize {
                rope.max_point_utf16()
            } else {
                PointUtf16::new(line as u32 - 1, 0)
            }
        };
        let start = if from_previous_line_end {
            let row = lines.start_line_number as u32 - 2;
            PointUtf16::new(row, rope.line_len_utf16(row))
        } else {
            point(lines.start_line_number)
        };
        start..point(lines.end_line_number_exclusive)
    };
    (
        points(original, change.original),
        points(modified, change.modified),
    )
}

/// A hidden-lines bar edge's pointer: Monaco's `n-resize`, `s-resize` or
/// `ns-resize`, whether dragging up and down does anything.
fn unchanged_edge_cursor(can_move_up: bool, can_move_down: bool) -> gpui::CursorStyle {
    match (can_move_up, can_move_down) {
        (true, false) => gpui::CursorStyle::ResizeUp,
        (false, true) => gpui::CursorStyle::ResizeDown,
        _ => gpui::CursorStyle::ResizeUpDown,
    }
}

/// A hidden-lines bar's edge being dragged (Monaco `isDragged`).
#[derive(Clone, Copy, Debug)]
struct UnchangedDrag {
    index: usize,
    /// The top edge (else the bottom).
    top: bool,
    /// In the original editor's bar (else the modified's), whose lines
    /// anchor the scroll position.
    original: bool,
    /// Where the drag started, in window coordinates.
    start_y: Pixels,
    /// The visible line count at the start.
    start: usize,
    /// It moved more than 2px, so it's not a click.
    moved: bool,
}

/// The height of a "hidden lines" bar (Monaco's 24px placeholder zone).
const UNCHANGED_BAR_HEIGHT: f32 = 24.;
/// Monaco's `PlaceholderViewZone` height in `compactMode`.
const COMPACT_UNCHANGED_BAR_HEIGHT: f32 = 12.;

/// Monaco `CollapsedCodeOverlayWidget`: the bar where unchanged lines are
/// hidden. Its top and bottom edges reveal more lines above and below; the
/// unfold button or a double click on the label shows them all; the
/// breadcrumbs (Monaco `diffEditorBreadcrumbs`) reveal their symbol.
struct UnchangedBar {
    diff: WeakEntity<DiffEditor>,
    /// The editor it's in, for the gutter width.
    editor: WeakEntity<CodeEditor>,
    /// In the original editor (else the modified).
    original: bool,
    /// Monaco `_hide`: the original editor's bar in the inline view, which
    /// shows only the background and edges (in `compact` mode, nothing),
    /// so its line-number column gets no unfold button or label.
    hide: bool,
    index: usize,
    hidden_lines: usize,
    /// Monaco `canMoveTop` for the top edge: lines above can be hidden
    /// again.
    visible_top: bool,
    /// Monaco `canMoveBottom` for the bottom edge.
    visible_bottom: bool,
    /// The symbols the hidden lines start, outermost first.
    breadcrumbs: Vec<BreadcrumbSymbol>,
    /// Monaco `CompactCollapsedCodeOverlayWidget` (`compactMode`): a thin
    /// labelled separator without controls.
    compact: bool,
}

/// The top and bottom edges' height: Monaco's 4px band with transparent
/// borders, 4px above and 2px below (`.diff-hidden-lines .top`).
const UNCHANGED_EDGE_HEIGHT: f32 = 10.;
/// The band's height and where it starts in the edge.
const UNCHANGED_EDGE_BAND: (f32, f32) = (4., 4.);
/// Where the top edge starts above the bar (Monaco's `-10px` translate of
/// `.diff-hidden-lines` and `4px` of `.top`): its band straddles the bar's
/// top.
const UNCHANGED_TOP_EDGE_OFFSET: f32 = 6.;
/// How far the bottom edge reaches below the bar (Monaco's `-6px` of
/// `.bottom`): its band straddles the bar's bottom.
const UNCHANGED_BOTTOM_EDGE_OVERHANG: f32 = 4.;

impl Render for UnchangedBar {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let Some(editor) = self.editor.upgrade() else {
            return div();
        };
        let editor = editor.read(cx);
        let colors = editor.colors().clone();
        if self.compact && self.hide {
            // Monaco's `replaceChildren()`: an empty widget.
            return div();
        }
        if self.compact {
            // `.diff-hidden-lines-compact`: a 20px rule, the label and a
            // rule to the end, in the code lens color.
            let rule = || {
                div()
                    .h(px(1.))
                    .bg(colors.code_lens_foreground.opacity(0.5))
                    .flex_none()
            };
            return div()
                .size_full()
                .flex()
                .flex_row()
                .items_center()
                .child(rule().w(px(20.)))
                .child(
                    div()
                        .mx(px(4.))
                        .flex_none()
                        .whitespace_nowrap()
                        .text_size(px(11.))
                        .line_height(px(11.))
                        .text_color(colors.code_lens_foreground)
                        .child(format!("{} hidden lines", self.hidden_lines)),
                )
                .child(rule().flex_1());
        }
        let gutter = editor
            .layout
            .as_ref()
            .map_or(px(0.), |l| l.text_bounds.left() - l.bounds.left());
        // Monaco's `n-resize`/`s-resize`/`ns-resize`: whether dragging up and
        // down does anything.
        let fully_revealed = self.hidden_lines == 0;
        let top_cursor = unchanged_edge_cursor(self.visible_top, !fully_revealed);
        let bottom_cursor = unchanged_edge_cursor(!fully_revealed, self.visible_bottom);
        // Monaco `isDragged`: the edge being dragged keeps its highlight
        // (`.top.dragging`), and no edge highlights on hover meanwhile
        // (`.diff-hidden-lines:not(.dragging)`).
        let dragged = self
            .diff
            .upgrade()
            .and_then(|diff| diff.read(cx).unchanged_drag)
            .filter(|drag| drag.index == self.index)
            .map(|drag| drag.top);
        let focus_border = colors.focus_border;
        let edge = |id: &'static str, cursor: gpui::CursorStyle, top: bool| {
            let (band_height, band_top) = UNCHANGED_EDGE_BAND;
            let band = div()
                .absolute()
                .left_0()
                .w_full()
                .top(px(band_top))
                .h(px(band_height));
            div()
                .id(id)
                .debug_selector(move || id.into())
                .group(id)
                .absolute()
                .left_0()
                .w_full()
                .h(px(UNCHANGED_EDGE_HEIGHT))
                .cursor(cursor)
                .child(match dragged {
                    Some(dragged) if dragged == top => band.bg(focus_border),
                    Some(_) => band,
                    None => band.group_hover(id, |style| style.bg(focus_border)),
                })
        };
        let (diff, index, original) = (self.diff.clone(), self.index, self.original);
        let with_bar = move |cx: &mut App, change: fn(&mut UnchangedRegion, usize)| {
            if let Some(diff) = diff.upgrade() {
                // Deferred: the bar is rendered by the editor being changed.
                cx.defer(move |cx| {
                    diff.update(cx, |diff, cx| {
                        let count = diff.options.hide_unchanged_regions.reveal_line_count;
                        diff.update_unchanged_region(index, |r| change(r, count), cx);
                    });
                });
            }
        };
        let (unfold, label) = (with_bar.clone(), with_bar);
        let diff = self.diff.clone();
        let start_drag = move |cx: &mut App, y: Pixels, top: bool| {
            if let Some(diff) = diff.upgrade() {
                cx.defer(move |cx| {
                    diff.update(cx, |diff, cx| {
                        diff.start_unchanged_drag(index, top, original, y, cx)
                    })
                });
            }
        };
        let (top, bottom) = (start_drag.clone(), start_drag);
        let shadow = colors.diff_unchanged_region_shadow;
        let link = colors.link_active_foreground;
        let breadcrumb_count = self.breadcrumbs.len();
        let breadcrumbs = self.breadcrumbs.iter().enumerate().map(|(ix, item)| {
            let diff = self.diff.clone();
            let line = item.start_line_number;
            let (glyph, glyph_color) = crate::quick_access::symbol_icon(item.kind, &colors);
            div()
                .id(("unchanged-breadcrumb", ix))
                .flex()
                .flex_row()
                .flex_none()
                .items_center()
                .cursor_pointer()
                .hover(|style| style.text_color(link))
                .on_click(move |_, _, cx| {
                    cx.stop_propagation();
                    if let Some(diff) = diff.upgrade() {
                        cx.defer(move |cx| {
                            diff.update(cx, |diff, cx| diff.reveal_modified_hidden_line(line, cx))
                        });
                    }
                })
                // The symbol kind's codicon, and `chevron-right` between
                // breadcrumbs.
                .child(codicon(glyph).size(px(13.)).color(glyph_color))
                .child("\u{a0}")
                .child(item.name.clone())
                .when(ix + 1 < breadcrumb_count, |item| {
                    item.child(
                        div()
                            .px(px(2.))
                            .child(codicon(Codicon::ChevronRight).size(px(13.))),
                    )
                })
        });
        // Monaco's `.center`, with the edges over it reaching past it.
        let hide = self.hide;
        let center = div()
            .size_full()
            .flex()
            .flex_row()
            .items_center()
            .overflow_hidden()
            .bg(colors.diff_unchanged_region_background)
            .text_color(colors.diff_unchanged_region_foreground)
            // Monaco's `.diff-hidden-lines`: 13px in the UI font.
            .text_size(px(13.))
            .when(!hide, |center| {
                center
                    .child(
                        div().w(gutter).flex_none().flex().justify_center().child(
                            div()
                                .id("unchanged-unfold")
                                .debug_selector(move || {
                                    if original {
                                        "unchanged-unfold-original".into()
                                    } else {
                                        "unchanged-unfold-modified".into()
                                    }
                                })
                                .tooltip(crate::tooltip::tooltip("Show Unchanged Region", &colors))
                                .cursor_pointer()
                                .px_1()
                                .hover(|style| style.text_color(link))
                                .on_click(move |_, _, cx| unfold(cx, |r, _| r.show_all()))
                                .child(codicon(Codicon::Unfold)),
                        ),
                    )
                    .child(
                        div()
                            .id("unchanged-label")
                            .tooltip(crate::tooltip::tooltip("Double click to unfold", &colors))
                            .flex_none()
                            .whitespace_nowrap()
                            .on_click(move |event, _, cx| {
                                if event.click_count() == 2 {
                                    label(cx, |r, _| r.show_all())
                                }
                            })
                            .child(format!("{} hidden lines", self.hidden_lines)),
                    )
                    .when(breadcrumb_count > 0, |bar| {
                        bar.child(
                            div()
                                .flex_none()
                                .whitespace_nowrap()
                                .child("\u{a0}\u{a0}|\u{a0}\u{a0}"),
                        )
                        .children(breadcrumbs)
                    })
            })
            // Monaco's inset shadows at the top and bottom.
            .border_t_1()
            .border_b_1()
            .border_color(shadow.opacity(0.35));
        div()
            .size_full()
            .relative()
            .child(center)
            .child(
                edge("unchanged-top", top_cursor, true)
                    .top(px(-UNCHANGED_TOP_EDGE_OFFSET))
                    .tooltip(crate::tooltip::tooltip(
                        "Click or drag to show more above",
                        &colors,
                    ))
                    .on_mouse_down(MouseButton::Left, move |e, _, cx| {
                        cx.stop_propagation();
                        top(cx, e.position.y, true)
                    }),
            )
            .child(
                edge("unchanged-bottom", bottom_cursor, false)
                    .bottom(px(-UNCHANGED_BOTTOM_EDGE_OVERHANG))
                    .tooltip(crate::tooltip::tooltip(
                        "Click or drag to show more below",
                        &colors,
                    ))
                    .on_mouse_down(MouseButton::Left, move |e, _, cx| {
                        cx.stop_propagation();
                        bottom(cx, e.position.y, false)
                    }),
            )
    }
}

/// Monaco's "Fold Unchanged Region" glyph (`fold-unchanged`): hides a
/// fully shown unchanged region again.
struct FoldUnchangedGlyph {
    diff: WeakEntity<DiffEditor>,
    index: usize,
    colors: EditorColors,
}

impl Render for FoldUnchangedGlyph {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let (diff, index) = (self.diff.clone(), self.index);
        div()
            .id("fold-unchanged")
            .size_full()
            .flex()
            .items_center()
            .justify_center()
            .cursor_pointer()
            .text_color(self.colors.foreground)
            .tooltip(crate::tooltip::tooltip(
                "Fold Unchanged Region",
                &self.colors,
            ))
            .role(gpui::Role::Button)
            .aria_label(SharedString::from("Fold Unchanged Region"))
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .on_click(move |_, _, cx| {
                if let Some(diff) = diff.upgrade() {
                    // Deferred: the glyph is rendered by the editor it
                    // changes.
                    cx.defer(move |cx| {
                        diff.update(cx, |diff, cx| {
                            diff.update_unchanged_region(index, UnchangedRegion::collapse_all, cx)
                        })
                    });
                }
            })
            // Monaco's "Fold Unchanged Region" glyph: the `fold` codicon.
            .child(codicon(Codicon::Fold))
    }
}

/// What a [`DeletedCode`] zone shows.
struct DeletedCodeSource {
    /// The modified editor, for its gutter, font, wrapping and horizontal
    /// scroll.
    editor: WeakEntity<CodeEditor>,
    diff: WeakEntity<DiffEditor>,
    original: Entity<TextModel>,
    /// The change whose deleted lines these are.
    change: LineRangeMapping,
    /// 0-based original rows.
    rows: Range<u32>,
    /// The changed characters, in original coordinates.
    inner_changes: Vec<Range<PointUtf16>>,
    render_indicators: bool,
}

/// One view line of the deleted lines (Monaco `renderLines`' output line).
#[derive(Clone, Debug, PartialEq)]
struct DeletedViewLine {
    /// The 0-based original row.
    row: u32,
    /// Its UTF-16 range in the row.
    range: Range<u32>,
    /// Visible columns of continuation indentation.
    indent: u32,
}

/// The visible column before each UTF-16 unit of `line`, and after the
/// last (tabs expand to `tab_size`).
fn utf16_visible_columns(line: &str, tab_size: u32) -> Vec<u32> {
    let mut columns = Vec::with_capacity(line.len() + 1);
    let mut column = 0;
    for ch in line.chars() {
        for _ in 0..ch.len_utf16() {
            columns.push(column);
        }
        column = if ch == '\t' {
            column + tab_size - column % tab_size
        } else {
            column + 1
        };
    }
    columns.push(column);
    columns
}

/// The byte offset of UTF-16 column `column` in `line`.
fn utf16_to_byte(line: &str, column: u32) -> usize {
    let mut units = 0;
    for (ix, ch) in line.char_indices() {
        if units >= column {
            return ix;
        }
        units += ch.len_utf16() as u32;
    }
    line.len()
}

/// A deleted line as Monaco's `renderViewLine` draws it with the modified
/// editor's `renderWhitespace` and `renderControlCharacters`.
#[derive(Clone, Debug, PartialEq)]
struct DeletedLineDisplay {
    /// The text to shape.
    text: String,
    /// Each byte offset of the line (and its end) in `text`.
    map: Vec<usize>,
    /// The rendered whitespace (`mtkw`), in `text`.
    whitespace: Vec<Range<usize>>,
    /// The rendered control characters (`mtkcontrol`), in `text`.
    controls: Vec<Range<usize>>,
}

/// Monaco `renderViewLine` for one deleted line: tabs to the next tab stop,
/// the tabs and spaces at UTF-16 `whitespace` columns (sorted) as `→` and
/// `·` (Monaco `mtkw`), and with `render_control_characters`, control
/// characters as their control pictures or `[U+XXXX]` (the BOM and line
/// separators always as U+FFFD).
fn deleted_line_display(
    line: &str,
    tab_size: usize,
    whitespace: &[u32],
    render_control_characters: bool,
) -> DeletedLineDisplay {
    use crate::element::Substitute;
    let mut text = String::with_capacity(line.len());
    let mut map = Vec::with_capacity(line.len() + 1);
    let mut rendered_whitespace = Vec::new();
    let mut controls = Vec::new();
    let mut visible_column = 0;
    let mut utf16_column = 0;
    for (byte, ch) in line.char_indices() {
        while map.len() <= byte {
            map.push(text.len());
        }
        let start = text.len();
        let marked = (ch == ' ' || ch == '\t') && whitespace.binary_search(&utf16_column).is_ok();
        utf16_column += ch.len_utf16() as u32;
        if ch == '\t' {
            let spaces = tab_size - visible_column % tab_size;
            if marked {
                // Monaco: `→`, then non-breaking spaces to the tab stop.
                text.push('\u{2192}');
                text.extend(std::iter::repeat_n('\u{a0}', spaces - 1));
                rendered_whitespace.push(start..text.len());
            } else {
                text.extend(std::iter::repeat_n(' ', spaces));
            }
            visible_column += spaces;
            continue;
        }
        if marked {
            // Monaco's `&middot;` (its zero-width non-joiner isn't needed).
            text.push('\u{b7}');
            rendered_whitespace.push(start..text.len());
            visible_column += 1;
            continue;
        }
        match crate::element::substitute(ch, render_control_characters) {
            Some(substitute) => {
                match substitute {
                    Substitute::Picture(picture) => text.push(picture),
                    Substitute::Hex(code) => {
                        use std::fmt::Write as _;
                        write!(text, "[U+{code:04X}]").ok();
                    }
                    Substitute::Replacement => text.push('\u{fffd}'),
                }
                if substitute != Substitute::Replacement {
                    controls.push(start..text.len());
                }
                visible_column += substitute.columns();
            }
            None => {
                text.push(ch);
                visible_column += 1;
            }
        }
    }
    while map.len() <= line.len() {
        map.push(text.len());
    }
    DeletedLineDisplay {
        text,
        map,
        whitespace: rendered_whitespace,
        controls,
    }
}

/// `highlights` less the parts inside `holes` (sorted, disjoint), so the
/// holes' own styles replace them.
fn without_ranges(
    highlights: Vec<(Range<usize>, HighlightStyle)>,
    holes: &[Range<usize>],
) -> Vec<(Range<usize>, HighlightStyle)> {
    if holes.is_empty() {
        return highlights;
    }
    let mut result = Vec::with_capacity(highlights.len());
    for (range, style) in highlights {
        let mut start = range.start;
        for hole in holes {
            if hole.end <= start || hole.start >= range.end {
                continue;
            }
            if hole.start > start {
                result.push((start..hole.start, style));
            }
            start = start.max(hole.end);
        }
        if start < range.end {
            result.push((start..range.end, style));
        }
    }
    result
}

/// Deleted lines in the inline view (Monaco `renderLines` into a
/// `line-delete line-delete-selectable` view zone, with
/// `inline-deleted-margin-view-zone` and `InlineDiffDeletedCodeMargin`):
/// highlighted original text on the removed background, wrapped like the
/// modified editor's lines, the changed characters marked, `−` in the
/// margin, and a light bulb with a copy and revert menu. The text can be
/// selected with the mouse and copied (Monaco `copySelection.ts`).
struct DeletedCode {
    source: DeletedCodeSource,
    /// The selection's anchor and head, in original rows and UTF-16
    /// columns.
    selection: Option<(PointUtf16, PointUtf16)>,
    selecting: bool,
    /// The view line under the mouse, where the light bulb shows.
    hovered: Option<usize>,
    /// The margin menu: where it opens (window coordinates) and the
    /// original row it's for.
    menu: Option<(gpui::Point<Pixels>, u32)>,
    /// The zone's bounds as of the last paint.
    bounds: Rc<Cell<Bounds<Pixels>>>,
    focus_handle: FocusHandle,
    _subscription: Subscription,
}

/// The light bulb's width in the margin.
const DELETED_LIGHT_BULB_WIDTH: f32 = 16.;

impl DeletedCode {
    fn new(source: DeletedCodeSource, cx: &mut Context<Self>) -> Self {
        // Monaco renders the zone again once tokenization completes.
        let subscription = cx.subscribe(&source.original, |_, _, event: &TextModelEvent, cx| {
            if matches!(event, TextModelEvent::SyntaxUpdated { .. }) {
                cx.notify();
            }
        });
        Self {
            source,
            selection: None,
            selecting: false,
            hovered: None,
            menu: None,
            bounds: Default::default(),
            focus_handle: cx.focus_handle(),
            _subscription: subscription,
        }
    }

    /// Monaco `renderLines`' `!hasCharChanges`: without character
    /// decorations, each view line is a full-width `char-delete`.
    fn full_width_char_delete(&self) -> bool {
        self.source.inner_changes.is_empty()
    }

    /// The deleted lines' view lines, with the modified editor's wrapping.
    fn view_lines(&self, cx: &App) -> Vec<DeletedViewLine> {
        let Some(editor) = self.source.editor.upgrade() else {
            return Vec::new();
        };
        let editor = editor.read(cx);
        let rope = self.source.original.read(cx).buffer().rope();
        let rows = self.source.rows.start..self.source.rows.end.min(rope.line_count());
        rows.flat_map(|row| {
            let (ranges, indent) = editor.line_breaks_for(&rope.line(row));
            ranges
                .into_iter()
                .enumerate()
                .map(move |(ix, range)| DeletedViewLine {
                    row,
                    range,
                    indent: if ix == 0 { 0 } else { indent },
                })
        })
        .collect()
    }

    /// Monaco `RenderLinesResult.minWidthInPx`: the widest view line's
    /// columns plus `scroll_beyond_last_column`, in the modified editor's
    /// character width.
    fn min_width(&self, cx: &App) -> Pixels {
        let Some(editor) = self.source.editor.upgrade() else {
            return px(0.);
        };
        let editor = editor.read(cx);
        let em_width = editor
            .layout
            .as_ref()
            .map_or(px(0.), |l| l.metrics.em_width);
        let model = self.source.original.read(cx);
        let tab_size = model.options().tab_size.max(1);
        let rope = model.buffer().rope();
        let max_columns = self
            .view_lines(cx)
            .iter()
            .map(|line| {
                let columns = utf16_visible_columns(&rope.line(line.row), tab_size);
                let at = |c: u32| columns[(c as usize).min(columns.len() - 1)];
                at(line.range.end) - at(line.range.start) + line.indent
            })
            .max()
            .unwrap_or(0);
        em_width * (max_columns + editor.options().scroll_beyond_last_column) as f32
    }

    /// The view line and text position at window position `position`.
    fn position_at(
        &self,
        position: gpui::Point<Pixels>,
        cx: &App,
    ) -> Option<(usize, PointUtf16, bool)> {
        let editor = self.source.editor.upgrade()?;
        let editor = editor.read(cx);
        let layout = editor.layout.as_ref()?;
        let line_height = editor.line_height();
        let bounds = self.bounds.get();
        let lines = self.view_lines(cx);
        if lines.is_empty() {
            return None;
        }
        let ix = (((position.y - bounds.top()) / line_height).floor().max(0.) as usize)
            .min(lines.len() - 1);
        let line = &lines[ix];
        let text_left = bounds.left() + (layout.text_bounds.left() - layout.bounds.left());
        let in_text = position.x >= text_left;
        let model = self.source.original.read(cx);
        let text = model.buffer().rope().line(line.row);
        let columns = utf16_visible_columns(&text, model.options().tab_size.max(1));
        let em_width = layout.metrics.em_width.max(px(1.));
        let x = position.x - text_left + layout.scroll_left - em_width * line.indent as f32;
        let target = columns[line.range.start as usize] as f32 + (x / em_width).max(0.);
        // The nearest character boundary in the view line.
        let column = (line.range.start..=line.range.end)
            .min_by(|a, b| {
                let distance = |c: &u32| (columns[*c as usize] as f32 - target).abs();
                distance(a)
                    .partial_cmp(&distance(b))
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
            .unwrap_or(line.range.start);
        Some((ix, PointUtf16::new(line.row, column), in_text))
    }

    /// The selected text (Monaco `copySelection`'s `getValueInRange`).
    fn selected_text(&self, cx: &App) -> Option<String> {
        let (anchor, head) = self.selection?;
        let (start, end) = (anchor.min(head), anchor.max(head));
        if start == end {
            return None;
        }
        let rope = self.source.original.read(cx).buffer().rope();
        let range = rope.point_utf16_to_offset(start)..rope.point_utf16_to_offset(end);
        Some(rope.text_for_range(range))
    }

    /// Copies the selection to the clipboard.
    fn copy_selection(&self, cx: &mut App) -> bool {
        let Some(text) = self.selected_text(cx) else {
            return false;
        };
        cx.write_to_clipboard(gpui::ClipboardItem::new_string(text));
        true
    }

    /// Monaco's "Copy deleted lines": the change's original lines, through
    /// the start of the line after them (`toExclusiveRange`).
    fn deleted_text(&self, cx: &App) -> String {
        let rope = self.source.original.read(cx).buffer().rope();
        let rows = &self.source.rows;
        let start = rope.point_to_offset(Point::new(rows.start.min(rope.max_point().row), 0));
        let end = if rows.end < rope.line_count() {
            rope.point_to_offset(Point::new(rows.end, 0))
        } else {
            rope.len()
        };
        rope.text_for_range(start..end)
    }

    /// Monaco's "Copy deleted line (N)": one line, or a line break for an
    /// empty one.
    fn deleted_line_text(&self, row: u32, cx: &App) -> String {
        let line = self.source.original.read(cx).buffer().rope().line(row);
        if line.is_empty() {
            "\n".to_string()
        } else {
            line
        }
    }

    /// Runs margin menu item `item` (see [`DeletedCode::menu_items`]).
    fn run_menu_item(&mut self, item: DeletedMenuItem, row: u32, cx: &mut Context<Self>) {
        self.menu = None;
        match item {
            DeletedMenuItem::CopyAll => {
                cx.write_to_clipboard(gpui::ClipboardItem::new_string(self.deleted_text(cx)))
            }
            DeletedMenuItem::CopyLine => cx.write_to_clipboard(gpui::ClipboardItem::new_string(
                self.deleted_line_text(row, cx),
            )),
            DeletedMenuItem::Revert => {
                if let Some(diff) = self.source.diff.upgrade() {
                    let change = self.source.change;
                    // Deferred: the zone is rendered by the editor it changes.
                    cx.defer(move |cx| diff.update(cx, |diff, cx| diff.revert(&change, cx)));
                }
            }
        }
        cx.notify();
    }

    /// Monaco `InlineDiffDeletedCodeMargin`'s menu items, with their labels,
    /// for original row `row`.
    fn menu_items(&self, row: u32, cx: &App) -> Vec<(DeletedMenuItem, String)> {
        let change = &self.source.change;
        let deletion = change.modified.is_empty();
        let many = change.original.len() > 1;
        let what = if deletion { "deleted" } else { "changed" };
        let mut items = vec![(
            DeletedMenuItem::CopyAll,
            if many {
                format!("Copy {what} lines")
            } else {
                format!("Copy {what} line")
            },
        )];
        if many {
            items.push((
                DeletedMenuItem::CopyLine,
                format!("Copy {what} line ({})", row + 1),
            ));
        }
        let read_only = self
            .source
            .editor
            .upgrade()
            .is_none_or(|e| e.read(cx).options().read_only);
        if !read_only {
            items.push((DeletedMenuItem::Revert, "Revert this change".to_string()));
        }
        items
    }
}

/// An item of the deleted lines' margin menu.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum DeletedMenuItem {
    /// `diff.clipboard.copyDeletedContent`.
    CopyAll,
    /// `diff.clipboard.copyDeletedLineContent`.
    CopyLine,
    /// `diff.inline.revertChange`.
    Revert,
}

impl Render for DeletedCode {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let Some(editor) = self.source.editor.upgrade() else {
            return div().id("deleted-code");
        };
        let editor = editor.read(cx);
        let options = editor.options();
        let colors = editor.colors().clone();
        let theme = editor.theme().clone();
        let line_height = editor.line_height();
        let (gutter_width, text_left, decorations_width, scroll_left, em_width) =
            match &editor.layout {
                Some(layout) => (
                    layout.gutter_width,
                    layout.text_bounds.left() - layout.bounds.left(),
                    layout.layout_info.decorations_width,
                    layout.scroll_left,
                    layout.metrics.em_width,
                ),
                None => (px(0.), px(0.), px(0.), px(0.), px(0.)),
            };
        let font_family = options.font_family.clone();
        let font_size = options.font_size;
        let font_weight = options.font_weight;
        // Monaco `RenderOptions.fromEditor`: the modified editor's options.
        let font_features = options.font_ligatures.features();
        let render_whitespace = options.render_whitespace;
        let render_control_characters = options.render_control_characters;
        // Monaco's `.line-delete` and `.char-delete`: a 1px
        // `diffEditor.removedTextBorder`, dashed in high contrast themes.
        let removed_border = colors.diff_removed_text_border;
        let high_contrast = theme.high_contrast;
        let border_style = if high_contrast {
            gpui::BorderStyle::Dashed
        } else {
            gpui::BorderStyle::Solid
        };
        let has_char_changes = !self.full_width_char_delete();
        let view_lines = self.view_lines(cx);
        let model = self.source.original.read(cx);
        let rope = model.buffer().rope();
        let tab_size = model.options().tab_size.max(1) as usize;
        let rows = self.source.rows.start..self.source.rows.end.min(rope.line_count());
        let start = rope.point_to_offset(Point::new(rows.start, 0));
        let end = rope.point_to_offset(Point::new(rows.end.saturating_sub(1), u32::MAX));
        let tokens = crate::element::HighlightSource::new(model)
            .map(|source| source.highlights(&theme, start..end))
            .unwrap_or_default();
        let selection = self
            .selection
            .map(|(a, b)| (a.min(b), a.max(b)))
            .filter(|(a, b)| a != b);
        let focused = self.focus_handle.is_focused(window);
        let selection_color = if focused {
            colors.selection_background
        } else {
            colors.inactive_selection_background
        };
        let lines = view_lines.iter().enumerate().map(|(ix, view_line)| {
            let row = view_line.row;
            let line = rope.line(row);
            let line_start = rope.point_to_offset(Point::new(row, 0));
            let line_end = line_start + line.len();
            // Monaco renders each view line on its own, so whitespace
            // boundaries and trailing whitespace are the view line's.
            let segment = &line[utf16_to_byte(&line, view_line.range.start)
                ..utf16_to_byte(&line, view_line.range.end)];
            let whitespace = crate::view::guides::whitespace_columns(
                segment,
                view_line.range.start,
                0,
                false,
                render_whitespace,
                &[],
            );
            let DeletedLineDisplay {
                text,
                map,
                whitespace,
                controls,
            } = deleted_line_display(&line, tab_size, &whitespace, render_control_characters);
            let from = map[utf16_to_byte(&line, view_line.range.start)];
            let to = map[utf16_to_byte(&line, view_line.range.end)];
            // Highlights in expanded line offsets, clipped to this view line
            // and made relative to it.
            let clip = |range: Range<usize>| {
                let range = range.start.max(from)..range.end.min(to);
                (range.start < range.end).then(|| range.start - from..range.end - from)
            };
            let syntax: Vec<_> = tokens
                .iter()
                .filter(|(range, _)| range.start < line_end && range.end > line_start)
                .filter_map(|(range, style)| {
                    let range = map[range.start.max(line_start) - line_start]
                        ..map[range.end.min(line_end) - line_start];
                    Some((
                        clip(range)?,
                        HighlightStyle {
                            color: style.foreground,
                            font_weight: style.bold.then_some(FontWeight::BOLD),
                            font_style: style.italic.then_some(FontStyle::Italic),
                            ..Default::default()
                        },
                    ))
                })
                .collect();
            let columns = |r: &Range<PointUtf16>| {
                if r.start.row > row || r.end.row < row {
                    return None;
                }
                let start = if r.start.row == row {
                    r.start.column
                } else {
                    0
                };
                let end = if r.end.row == row {
                    r.end.column
                } else {
                    u32::MAX
                };
                Some(map[utf16_to_byte(&line, start)]..map[utf16_to_byte(&line, end)])
            };
            let changed: Vec<_> = self
                .source
                .inner_changes
                .iter()
                .filter_map(|r| {
                    Some((
                        clip(columns(r)?)?,
                        HighlightStyle {
                            background_color: Some(colors.diff_removed_text_background),
                            ..Default::default()
                        },
                    ))
                })
                .collect();
            let selected: Vec<_> = selection
                .and_then(|(a, b)| clip(columns(&(a..b))?))
                .map(|range| {
                    (
                        range,
                        HighlightStyle {
                            background_color: Some(selection_color),
                            ..Default::default()
                        },
                    )
                })
                .into_iter()
                .collect();
            // Monaco `mtkw` (`editorWhitespace.foreground`) and
            // `mtkcontrol` (white on dark red) replace the token's class.
            let rendered: Vec<_> = whitespace
                .iter()
                .filter_map(|range| {
                    Some((
                        clip(range.clone())?,
                        HighlightStyle {
                            color: Some(colors.whitespace),
                            ..Default::default()
                        },
                    ))
                })
                .chain(controls.iter().filter_map(|range| {
                    Some((
                        clip(range.clone())?,
                        HighlightStyle {
                            color: Some(crate::element::CONTROL_CHARACTER_FOREGROUND),
                            background_color: Some(crate::element::CONTROL_CHARACTER_BACKGROUND),
                            ..Default::default()
                        },
                    ))
                }))
                .collect();
            let mut holes: Vec<Range<usize>> = rendered.iter().map(|(r, _)| r.clone()).collect();
            holes.sort_by_key(|r| r.start);
            let mut syntax = without_ranges(syntax, &holes);
            syntax.extend(rendered);
            let changed_ranges: Vec<Range<usize>> =
                changed.iter().map(|(range, _)| range.clone()).collect();
            let highlights: Vec<_> = gpui::combine_highlights(
                gpui::combine_highlights(syntax, changed).collect::<Vec<_>>(),
                selected,
            )
            .collect();
            let text = text[from..to].to_string();
            let text = if text.is_empty() {
                " ".to_string()
            } else {
                text
            };
            let bulb = self.hovered == Some(ix);
            let first_of_row = view_line.range.start == 0;
            div()
                .h(line_height)
                .flex()
                .flex_row()
                .child(
                    div()
                        .relative()
                        .h_full()
                        .w(text_left)
                        .flex_none()
                        .child(
                            div()
                                .absolute()
                                .h_full()
                                .w(gutter_width)
                                .bg(colors.diff_gutter_removed_line_background),
                        )
                        .when(self.source.render_indicators && !bulb, |margin| {
                            // Monaco's `delete-sign` at the margin's right.
                            margin.child(
                                div()
                                    .absolute()
                                    .right(px(0.))
                                    .w(decorations_width)
                                    .h_full()
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .text_size(px(11.))
                                    .opacity(0.7)
                                    .text_color(colors.foreground)
                                    // `.delete-sign`: 11px `remove`.
                                    .child(codicon(Codicon::Remove).size(px(11.))),
                            )
                        })
                        .when(bulb, |margin| {
                            let color = colors.lightbulb_foreground;
                            margin.child(
                                canvas(
                                    |_, _, _| {},
                                    move |bounds, _, window, cx| {
                                        crate::element::paint_lightbulb(
                                            bounds,
                                            color,
                                            Codicon::LightBulb,
                                            window,
                                            cx,
                                        )
                                    },
                                )
                                .absolute()
                                .right(px(0.))
                                .w(px(DELETED_LIGHT_BULB_WIDTH))
                                .h_full(),
                            )
                        }),
                )
                .child({
                    let styled = StyledText::new(text).with_highlights(highlights);
                    // Monaco's `char-delete` spans' 1px
                    // `diffEditor.removedTextBorder`, around the changed
                    // characters' boxes.
                    let run_borders = (has_char_changes && !changed_ranges.is_empty())
                        .then_some(removed_border)
                        .flatten()
                        .map(|border| {
                            let layout = styled.layout().clone();
                            canvas(
                                |_, _, _| {},
                                move |bounds, _, window, _| {
                                    let x = |ix: usize| layout.position_for_index(ix).map(|p| p.x);
                                    for range in &changed_ranges {
                                        let (Some(left), Some(right)) =
                                            (x(range.start), x(range.end))
                                        else {
                                            continue;
                                        };
                                        window.paint_quad(gpui::quad(
                                            Bounds::new(
                                                point(left, bounds.top()),
                                                gpui::size(right - left, bounds.size.height),
                                            ),
                                            gpui::Corners::default(),
                                            gpui::transparent_black(),
                                            gpui::Edges::all(px(1.)),
                                            border,
                                            border_style,
                                        ));
                                    }
                                },
                            )
                            .absolute()
                            .top_0()
                            .left_0()
                            .size_full()
                        });
                    div()
                        .relative()
                        .h_full()
                        .flex_1()
                        .overflow_hidden()
                        .bg(colors.diff_removed_line_background)
                        // Monaco's `view-line char-delete` when the change
                        // has no inner changes: `diffEditor.removedTextBackground`
                        // and its border across the (1000000px wide) line.
                        .when(!has_char_changes, |line| {
                            line.child(
                                div()
                                    .absolute()
                                    .top_0()
                                    .h_full()
                                    .left(-scroll_left)
                                    .right(px(-2.))
                                    .bg(colors.diff_removed_text_background)
                                    .when_some(removed_border, |layer, border| {
                                        layer
                                            .border_1()
                                            .border_color(border)
                                            .when(high_contrast, |layer| layer.border_dashed())
                                    }),
                            )
                        })
                        .child(
                            div()
                                .relative()
                                .ml(-scroll_left)
                                .when(!first_of_row, |text| {
                                    text.pl(em_width * view_line.indent as f32)
                                })
                                .whitespace_nowrap()
                                .child(styled),
                        )
                        .children(run_borders)
                })
        });
        let lines: Vec<_> = lines.collect();
        // Monaco's `line-delete` zone node: the text area of all the view
        // lines, scrolled with the text, with a 1px
        // `diffEditor.removedTextBorder` (painted over the lines here).
        let zone_border = removed_border.map(|border| {
            div()
                .absolute()
                .top_0()
                .left(text_left)
                .right_0()
                .h(line_height * lines.len() as f32)
                .overflow_hidden()
                .child(
                    div()
                        .absolute()
                        .top_0()
                        .h_full()
                        .left(-scroll_left)
                        .right_0()
                        .border_1()
                        .border_color(border)
                        .when(high_contrast, |zone| zone.border_dashed()),
                )
        });
        let bounds = self.bounds.clone();
        let menu = self.menu.map(|(position, row)| {
            let items = self.menu_items(row, cx);
            deferred(
                anchored().position(position).child(
                    div()
                        .id("deleted-code-menu")
                        .occlude()
                        .min_w(px(160.))
                        .py(px(4.))
                        .rounded(px(5.))
                        .bg(colors.menu_background)
                        .text_color(colors.menu_foreground)
                        .text_size(px(13.))
                        .when_some(colors.menu_border, |menu, border| {
                            menu.border_1().border_color(border)
                        })
                        .shadow_md()
                        .role(gpui::Role::Menu)
                        .on_mouse_down_out(cx.listener(|this, _, _, cx| {
                            this.menu = None;
                            cx.notify();
                        }))
                        .children(items.into_iter().enumerate().map(|(ix, (item, label))| {
                            div()
                                .id(("deleted-code-menu-item", ix))
                                .px(px(12.))
                                .h(px(24.))
                                .flex()
                                .items_center()
                                .whitespace_nowrap()
                                .cursor_pointer()
                                .role(gpui::Role::MenuItem)
                                .hover(|style| {
                                    style
                                        .bg(colors.menu_selection_background)
                                        .text_color(colors.menu_selection_foreground)
                                })
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.run_menu_item(item, row, cx)
                                }))
                                .child(label)
                        })),
                ),
            )
            .with_priority(1)
        });
        div()
            .id("deleted-code")
            .track_focus(&self.focus_handle)
            .size_full()
            .overflow_hidden()
            .cursor(gpui::CursorStyle::IBeam)
            .font_family(font_family)
            .text_size(font_size)
            .font_weight(font_weight)
            .font_features(font_features)
            .line_height(line_height)
            .text_color(colors.foreground)
            .child(
                canvas(move |b, _, _| bounds.set(b), |_, _, _, _| {})
                    .absolute()
                    .size_full(),
            )
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, event: &gpui::MouseDownEvent, window, cx| {
                    let Some((ix, position, in_text)) = this.position_at(event.position, cx) else {
                        return;
                    };
                    cx.stop_propagation();
                    if !in_text {
                        // Monaco's light bulb opens the menu below itself.
                        let bounds = this.bounds.get();
                        let top = bounds.top() + line_height * (ix + 1) as f32;
                        let pad = (line_height / 3.).floor();
                        this.menu = Some((point(event.position.x, top + pad), position.row));
                        this.hovered = Some(ix);
                        cx.notify();
                        return;
                    }
                    window.focus(&this.focus_handle, cx);
                    this.selection = Some((position, position));
                    this.selecting = true;
                    cx.notify();
                }),
            )
            .on_mouse_move(cx.listener(|this, event: &gpui::MouseMoveEvent, _, cx| {
                let Some((ix, position, _)) = this.position_at(event.position, cx) else {
                    return;
                };
                let mut changed = this.hovered != Some(ix);
                this.hovered = Some(ix);
                if this.selecting && event.pressed_button == Some(MouseButton::Left) {
                    if let Some((_, head)) = this.selection.as_mut() {
                        changed |= *head != position;
                        *head = position;
                    }
                } else {
                    this.selecting = false;
                }
                if changed {
                    cx.notify();
                }
            }))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, _, _, _| this.selecting = false),
            )
            .on_hover(cx.listener(|this, hovered: &bool, _, cx| {
                if !hovered && this.menu.is_none() {
                    this.hovered = None;
                    cx.notify();
                }
            }))
            .on_key_down(cx.listener(|this, event: &gpui::KeyDownEvent, _, cx| {
                let keystroke = &event.keystroke;
                let secondary = if cfg!(target_os = "macos") {
                    keystroke.modifiers.platform
                } else {
                    keystroke.modifiers.control
                };
                if secondary && keystroke.key == "c" && this.copy_selection(cx) {
                    cx.stop_propagation();
                }
            }))
            .children(lines)
            .children(zone_border)
            .children(menu)
    }
}

/// The models a diff editor compares (Monaco `IDiffEditorModel`).
#[derive(Clone)]
pub struct DiffEditorModel {
    /// The original model (Monaco `original`).
    pub original: Entity<TextModel>,
    /// The modified model (Monaco `modified`).
    pub modified: Entity<TextModel>,
}

/// Monaco `ILineChange` (from the deprecated `getLineChanges`): a change's
/// 1-based inclusive line numbers. An insertion has
/// `original_end_line_number == 0` and `original_start_line_number` the
/// line before it; a deletion likewise on the modified side.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LineChange {
    /// Monaco `originalStartLineNumber`.
    pub original_start_line_number: usize,
    /// Monaco `originalEndLineNumber`.
    pub original_end_line_number: usize,
    /// Monaco `modifiedStartLineNumber`.
    pub modified_start_line_number: usize,
    /// Monaco `modifiedEndLineNumber`.
    pub modified_end_line_number: usize,
    /// Monaco `charChanges`: the changed characters, for changes with lines
    /// on both sides.
    pub char_changes: Option<Vec<CharChange>>,
}

/// Monaco `ICharChange`: 1-based lines and columns, end exclusive.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CharChange {
    /// Monaco `originalStartLineNumber`.
    pub original_start_line_number: usize,
    /// Monaco `originalStartColumn`.
    pub original_start_column: usize,
    /// Monaco `originalEndLineNumber`.
    pub original_end_line_number: usize,
    /// Monaco `originalEndColumn`.
    pub original_end_column: usize,
    /// Monaco `modifiedStartLineNumber`.
    pub modified_start_line_number: usize,
    /// Monaco `modifiedStartColumn`.
    pub modified_start_column: usize,
    /// Monaco `modifiedEndLineNumber`.
    pub modified_end_line_number: usize,
    /// Monaco `modifiedEndColumn`.
    pub modified_end_column: usize,
}

/// Monaco `toLineChanges`.
pub(crate) fn to_line_changes(diff: &LinesDiff) -> Vec<LineChange> {
    diff.changes
        .iter()
        .map(|m| {
            let mut inner = Some(&m.inner_changes);
            let (original_start_line_number, original_end_line_number) = if m.original.is_empty() {
                inner = None;
                (m.original.start_line_number - 1, 0)
            } else {
                (
                    m.original.start_line_number,
                    m.original.end_line_number_exclusive - 1,
                )
            };
            let (modified_start_line_number, modified_end_line_number) = if m.modified.is_empty() {
                inner = None;
                (m.modified.start_line_number - 1, 0)
            } else {
                (
                    m.modified.start_line_number,
                    m.modified.end_line_number_exclusive - 1,
                )
            };
            LineChange {
                original_start_line_number,
                original_end_line_number,
                modified_start_line_number,
                modified_end_line_number,
                char_changes: inner.map(|inner| {
                    inner
                        .iter()
                        .map(|c| CharChange {
                            original_start_line_number: c.original_range.start_line_number,
                            original_start_column: c.original_range.start_column,
                            original_end_line_number: c.original_range.end_line_number,
                            original_end_column: c.original_range.end_column,
                            modified_start_line_number: c.modified_range.start_line_number,
                            modified_start_column: c.modified_range.start_column,
                            modified_end_line_number: c.modified_range.end_line_number,
                            modified_end_column: c.modified_range.end_column,
                        })
                        .collect()
                }),
            }
        })
        .collect()
}

/// Which gutter toolbar a [`DiffHunkAction`] is in (Monaco's menus).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DiffHunkToolbar {
    /// Beside each change (Monaco `MenuId.DiffEditorHunkToolbar`).
    Hunk,
    /// Beside the selected changes (Monaco
    /// `MenuId.DiffEditorSelectionToolbar`).
    Selection,
}

/// What a [`DiffHunkAction`] runs on (Monaco
/// `DiffEditorSelectionHunkToolbarContext`).
#[derive(Clone, Debug)]
pub struct DiffHunkContext {
    /// The change, or the selected parts of changes (Monaco `mapping`).
    pub mapping: DetailedLineRangeMapping,
    /// The original text with the change applied (Monaco
    /// `originalWithModifiedChanges`), e.g. to stage it.
    pub original_with_modified_changes: String,
}

/// `text` with its `$(name)` codicons drawn as icons.
fn label_with_icons(text: &str) -> Vec<gpui::AnyElement> {
    crate::codicons::parse_label_with_icons(text)
        .into_iter()
        .map(|part| match part {
            crate::codicons::LabelPart::Text(text) => {
                SharedString::from(text.to_string()).into_any_element()
            }
            crate::codicons::LabelPart::Icon(icon) => codicon(icon).into_any_element(),
        })
        .collect()
}

/// What a [`DiffHunkAction`] runs.
pub type DiffHunkHandler = Rc<dyn Fn(&DiffHunkContext, &mut Window, &mut App)>;

/// A button in the diff editor's gutter menu (a Monaco
/// `DiffEditorHunkToolbar` or `DiffEditorSelectionToolbar` menu item), from
/// [`DiffEditor::add_hunk_action`].
#[derive(Clone)]
pub struct DiffHunkAction {
    /// Its id, for [`DiffEditor::remove_hunk_action`].
    pub id: SharedString,
    /// Its tooltip and screen reader label.
    pub label: SharedString,
    /// The button's label (Monaco shows a codicon): text, where `$(name)`
    /// is the codicon `name`, such as `"$(discard)"`.
    pub icon: SharedString,
    /// The toolbar it's in.
    pub toolbar: DiffHunkToolbar,
    /// Runs it.
    pub run: DiffHunkHandler,
}

impl std::fmt::Debug for DiffHunkAction {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DiffHunkAction")
            .field("id", &self.id)
            .field("label", &self.label)
            .field("toolbar", &self.toolbar)
            .finish_non_exhaustive()
    }
}

/// Monaco `DiffEditorGutter.computeStagedValue`: `original` with
/// `mapping`'s inner changes applied.
fn staged_value(
    original: &text::Rope,
    modified: &text::Rope,
    mapping: &DetailedLineRangeMapping,
) -> String {
    let mut result = String::new();
    let mut last = 0;
    let offset = |rope: &text::Rope, range: &crate::diff::Range| {
        let range = range.to_point_utf16_range();
        rope.point_utf16_to_offset(rope.clip_point_utf16(range.start, text::Bias::Left))
            ..rope.point_utf16_to_offset(rope.clip_point_utf16(range.end, text::Bias::Left))
    };
    for change in &mapping.inner_changes {
        let original_range = offset(original, &change.original_range);
        let modified_range = offset(modified, &change.modified_range);
        if original_range.start < last {
            continue;
        }
        result.push_str(&original.text_for_range(last..original_range.start));
        result.push_str(&modified.text_for_range(modified_range));
        last = original_range.end;
    }
    result.push_str(&original.text_for_range(last..original.len()));
    result
}

/// A symbol for the hidden-lines bars' breadcrumbs (Monaco
/// `IDiffEditorBreadcrumbsSource`'s items).
#[derive(Clone, Debug, PartialEq)]
struct BreadcrumbSymbol {
    name: SharedString,
    kind: crate::providers::SymbolKind,
    /// 1-based lines.
    start_line_number: usize,
    end_line_number: usize,
}

/// Monaco `DiffEditorBreadcrumbsSource.getBreadcrumbItems`: the symbols
/// starting in `range` and ending after it, outermost first.
fn breadcrumb_items(symbols: &[BreadcrumbSymbol], range: LineRange) -> Vec<BreadcrumbSymbol> {
    let mut items: Vec<_> = symbols
        .iter()
        .filter(|s| range.contains(s.start_line_number) && !range.contains(s.end_line_number))
        .cloned()
        .collect();
    items.sort_by_key(|s| std::cmp::Reverse(s.end_line_number - s.start_line_number));
    items
}

/// Monaco `DiffEditorSash`'s `MINIMUM_EDITOR_WIDTH`.
const MINIMUM_EDITOR_WIDTH: f32 = 100.;
/// The sash's pointer (Monaco's vertical `Sash`: `ew-resize`).
const SASH_CURSOR: gpui::CursorStyle = gpui::CursorStyle::ResizeColumn;
/// Monaco's sash size (`Sash` with the default `size` of 4px).
const SASH_SIZE: f32 = 4.;
/// Monaco `validateDiffEditorOptions`' `clampedFloat(splitViewDefaultRatio,
/// 0.5, 0.1, 0.9)`.
fn clamp_split_view_ratio(ratio: f32) -> f32 {
    if ratio.is_nan() {
        0.5
    } else {
        ratio.clamp(0.1, 0.9)
    }
}

/// Monaco `OverviewRulerFeature.ENTIRE_DIFF_OVERVIEW_WIDTH`.
const DIFF_OVERVIEW_WIDTH: f32 = 30.;
/// Monaco `ScrollbarState`'s minimum slider size.
const MINIMUM_SLIDER_SIZE: f32 = 20.;

/// Monaco `SashLayout._computeSashLeft`: the sash's x in a `width` wide
/// diff editor (less the overview ruler).
fn compute_sash_left(ratio: f32, default_ratio: f32, resizable: bool, width: Pixels) -> Pixels {
    let width = f32::from(width);
    let mid_point = (default_ratio * width).floor();
    let sash_left = if resizable {
        (ratio * width).floor()
    } else {
        mid_point
    };
    px(if width <= MINIMUM_EDITOR_WIDTH * 2. {
        mid_point
    } else {
        sash_left.clamp(MINIMUM_EDITOR_WIDTH, width - MINIMUM_EDITOR_WIDTH)
    })
}

/// Monaco `ScrollbarState`'s slider for a `visible` tall viewport over
/// `scroll_height` scrolled to `scroll_top` (no arrows): its top and size.
fn slider_geometry(visible: Pixels, scroll_height: Pixels, scroll_top: Pixels) -> (Pixels, Pixels) {
    let visible = f32::from(visible);
    let scroll_height = f32::from(scroll_height).max(visible);
    if visible <= 0. || scroll_height <= 0. {
        return (px(0.), px(0.));
    }
    let size = (visible * visible / scroll_height)
        .floor()
        .max(MINIMUM_SLIDER_SIZE)
        .min(visible)
        .round();
    let ratio = if scroll_height > visible {
        (visible - size) / (scroll_height - visible)
    } else {
        0.
    };
    (px(f32::from(scroll_top) * ratio), px(size))
}

/// What [`DiffEditor`] forces on its editors, and the host's values it
/// replaced, to restore them (Monaco `_adjustOptionsForSubEditor` reads
/// them from the diff editor's own options).
#[derive(Default)]
struct HostOptions {
    /// Each editor's sticky scroll before it was turned off.
    sticky_scroll: [Option<bool>; 2],
    /// The original editor's unicode highlighting before the inline view
    /// turned it off.
    unicode_highlight: Option<crate::options::UnicodeHighlightOptions>,
}

/// A command from [`DiffEditor::add_command`].
type DiffCommandHandler = Rc<dyn Fn(&mut DiffEditor, &mut Window, &mut Context<DiffEditor>)>;

/// The side-by-side diff editor (Monaco `DiffEditorWidget`).
pub struct DiffEditor {
    /// The counter in its [`DiffEditor::id`].
    id: u64,
    original: Entity<CodeEditor>,
    modified: Entity<CodeEditor>,
    options: DiffEditorOptions,
    diff: Option<Arc<LinesDiff>>,
    /// Monaco `IDocumentDiff.identical`: no changes and the same text.
    identical: bool,
    /// The model versions `diff` is for, and the computation running.
    diff_versions: Option<(text::Version, text::Version)>,
    diff_task: Option<Task<()>>,
    /// [`DiffEditor::wait_for_diff`]'s callers.
    diff_waiters: Vec<futures::channel::oneshot::Sender<()>>,
    /// Monaco `setModel(null)`: the editors show empty placeholder models.
    has_model: bool,
    decorations: (Vec<DecorationId>, Vec<DecorationId>),
    /// The alignment zones on each side.
    zones: (Vec<WidgetId>, Vec<WidgetId>),
    /// What the alignment zones show, to skip realigning when nothing
    /// changed.
    alignment: Vec<AlignmentZone>,
    /// The scheduled realignment (Monaco's `RunOnceScheduler(0)`).
    alignment_task: Option<Task<()>>,
    /// The moved blocks' caption zones on each side.
    move_caption_zones: (Vec<WidgetId>, Vec<WidgetId>),
    /// The inline view's deleted lines zones.
    deleted_code: Vec<WeakEntity<DeletedCode>>,
    /// The original side's share of the width.
    ratio: f32,
    /// The diff editor's bounds, for dragging the sash.
    bounds: Rc<Cell<Bounds<Pixels>>>,
    /// Monaco `DiffEditorSash`: a drag's start x and sash left.
    sash_drag: Option<(Pixels, Pixels)>,
    /// The sash showing `sash.hoverBorder` for the mouse resting on it.
    sash_hover: crate::sash::SashHover,
    /// Monaco's `diffViewport` being dragged: where in the slider it was
    /// grabbed.
    overview_drag: Option<Pixels>,
    /// The mouse's y over the overview ruler, from its top.
    overview_mouse_y: Option<Pixels>,
    syncing_scroll: bool,
    /// Whether the last render showed the diff inline.
    inline: bool,
    /// The modified editor runs optimized for screen readers (Monaco's
    /// `_screenReaderMode`), as of the last render.
    screen_reader_mode: bool,
    /// Editor options the diff editor replaced.
    host_options: HostOptions,
    /// Hideable unchanged regions, when `hide_unchanged_regions` is on
    /// (Monaco `_unchangedRegions`): `None` before the first diff, empty
    /// when everything shows. When nothing changed, one region covers the
    /// whole file, which the view skips.
    unchanged: Option<Vec<UnchangedRegion>>,
    /// The only region covers the whole modified model: no bars, glyphs or
    /// hidden lines (Monaco's `unchangedRegions` in
    /// `HideUnchangedRegionsFeature`).
    unchanged_whole_file: bool,
    /// Decorations on each region's lines, tracking them through edits.
    unchanged_decorations: (Vec<DecorationId>, Vec<DecorationId>),
    /// The "hidden lines" bars.
    unchanged_zones: (Vec<WidgetId>, Vec<WidgetId>),
    /// The "Fold Unchanged Region" glyphs on fully shown regions.
    fold_glyphs: (Vec<WidgetId>, Vec<WidgetId>),
    /// The modified model's symbols, for the bars' breadcrumbs (Monaco
    /// `diffEditorBreadcrumbs`).
    breadcrumb_symbols: Vec<BreadcrumbSymbol>,
    breadcrumb_task: Option<Task<()>>,
    /// The mouse is over the gutter menu, which shows every button.
    gutter_hovered: bool,
    /// Counts the mouse leaving the gutter, keying the buttons' fade out.
    gutter_fade: usize,
    /// Host buttons in the gutter menu.
    hunk_actions: Vec<DiffHunkAction>,
    /// Commands from [`DiffEditor::add_command`], by id.
    command_handlers: std::collections::HashMap<SharedString, DiffCommandHandler>,
    /// A hidden-lines bar's edge being dragged.
    unchanged_drag: Option<UnchangedDrag>,
    /// The margin revert arrows in the modified editor.
    revert_arrows: Vec<WidgetId>,
    /// The accessible diff viewer, while it shows.
    accessible_viewer: Option<Entity<AccessibleDiffViewer>>,
    /// Moved blocks' borders on each side.
    move_decorations: (Vec<DecorationId>, Vec<DecorationId>),
    /// The moved block at the last moved cursor (Monaco `activeMovedText`).
    active_move: Option<usize>,
    /// The moved block compared with its original (Monaco
    /// `movedTextToCompare`).
    move_to_compare: Option<usize>,
    /// Whether the original editor's cursor moved last (Monaco
    /// `lastChangedEditor`).
    last_cursor_original: bool,
    /// Monaco `shouldRenderInlineViewInSmartMode` for `compact_mode`:
    /// whether the first diff of these models was only insertions and
    /// deletions (decided once).
    simple_diff: Option<bool>,
    /// [`DiffEditor::reveal_first_diff`] waits for the diff.
    reveal_first_diff_pending: bool,
    /// Collapsed regions from [`DiffEditor::restore_view_state`], applied
    /// to the next diff when there was none yet.
    pending_collapsed_regions: Option<Vec<LineRange>>,
    /// Whether the original and the modified editor had focus in the last
    /// frame (Monaco `isInDiffLeftEditor`/`isInDiffRightEditor`).
    focused_sides: (bool, bool),
    /// The context keys last passed on to the editors.
    passed_keys: Vec<(SharedString, ContextKeyValue)>,
    /// The theme whose colors the diff decorations have.
    decorated_theme: Option<Arc<crate::theme::Theme>>,
    /// Shown by a [`MultiDiffEditor`](crate::MultiDiffEditor): `Some` with
    /// whether all its files are collapsed (Monaco `inMultiDiffEditor` and
    /// `multiDiffEditorAllCollapsed`).
    pub(crate) multi_diff_all_collapsed: Option<bool>,
    focus_handle: FocusHandle,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<DiffEditorEvent> for DiffEditor {}

impl Focusable for DiffEditor {
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        self.modified.focus_handle(cx)
    }
}

impl DiffEditor {
    /// A diff editor comparing `original` with `modified` (Monaco
    /// `createDiffEditor` + `setModel({ original, modified })`).
    pub fn new(
        original: Entity<TextModel>,
        modified: Entity<TextModel>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let options = DiffEditorOptions::default();
        let original_editor = cx.new(|cx| {
            let mut editor = CodeEditor::new(original, window, cx);
            let mut editor_options = editor.options().clone();
            editor_options.read_only = !options.original_editable;
            editor.update_options(editor_options, cx);
            editor
        });
        let modified_editor = cx.new(|cx| CodeEditor::new(modified, window, cx));
        // Monaco `DelegatingEditor.idCounter`.
        static NEXT_ID: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
        let mut this = Self {
            original: original_editor,
            modified: modified_editor,
            id: NEXT_ID.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
            ratio: options.split_view_default_ratio,
            options,
            diff: None,
            identical: false,
            diff_versions: None,
            diff_task: None,
            diff_waiters: Vec::new(),
            has_model: true,
            decorations: Default::default(),
            zones: Default::default(),
            alignment: Vec::new(),
            alignment_task: None,
            move_caption_zones: Default::default(),
            deleted_code: Vec::new(),
            bounds: Default::default(),
            sash_drag: None,
            sash_hover: Default::default(),
            overview_drag: None,
            overview_mouse_y: None,
            syncing_scroll: false,
            inline: false,
            screen_reader_mode: false,
            host_options: HostOptions::default(),
            unchanged: None,
            unchanged_whole_file: false,
            unchanged_decorations: Default::default(),
            unchanged_zones: Default::default(),
            fold_glyphs: Default::default(),
            breadcrumb_symbols: Vec::new(),
            breadcrumb_task: None,
            gutter_hovered: false,
            gutter_fade: 0,
            hunk_actions: Vec::new(),
            command_handlers: Default::default(),
            unchanged_drag: None,
            revert_arrows: Vec::new(),
            accessible_viewer: None,
            move_decorations: Default::default(),
            active_move: None,
            move_to_compare: None,
            last_cursor_original: false,
            simple_diff: None,
            reveal_first_diff_pending: false,
            pending_collapsed_regions: None,
            focused_sides: (false, false),
            passed_keys: Vec::new(),
            decorated_theme: None,
            multi_diff_all_collapsed: None,
            focus_handle: cx.focus_handle(),
            _subscriptions: Vec::new(),
        };
        this.apply_word_wrap(cx);
        this.apply_editor_options(cx);
        // After its editors, as Monaco's `addDiffEditor`.
        crate::registry::track_diff_editor(cx);
        this.subscribe(cx);
        this.compute_diff(true, cx);
        this
    }

    fn subscribe(&mut self, cx: &mut Context<Self>) {
        let original_model = self.original.read(cx).model().clone();
        let modified_model = self.modified.read(cx).model().clone();
        let editor_events = |original: bool| {
            move |this: &mut Self,
                  _: Entity<CodeEditor>,
                  event: &CodeEditorEvent,
                  cx: &mut Context<Self>| {
                match event {
                    CodeEditorEvent::ScrollChanged(_) => {
                        this.sync_scroll(original, cx);
                        // The gutter menu, the overview ruler and the moved
                        // block lines follow the editors.
                        cx.notify();
                    }
                    CodeEditorEvent::SelectionsChanged(change) => {
                        this.ensure_selections_visible(original, cx);
                        this.update_active_move(original, cx);
                        if change.reason == crate::events::CursorChangeReason::Explicit {
                            this.signal_change_at_cursor(original, cx);
                        }
                        if !original {
                            this.apply_revert_arrows(cx);
                            cx.notify();
                        }
                    }
                    // Monaco realigns on `onDidChangeViewZones` and on
                    // `wrappingInfo`/`lineHeight` changes; a layout change
                    // can change the wrapping.
                    CodeEditorEvent::ViewZonesChanged | CodeEditorEvent::LayoutChanged(_) => {
                        this.schedule_alignment(cx);
                        cx.notify();
                    }
                    CodeEditorEvent::ConfigurationChanged(_) => {
                        // The host may have changed what the diff editor
                        // forces (Monaco re-applies its overrides).
                        this.apply_editor_options(cx);
                        this.schedule_alignment(cx);
                    }
                    // Monaco `DiffEditorEditors._constructInnerEditor`.
                    CodeEditorEvent::ContentSizeChanged(change) => {
                        let event = ContentSizeChange {
                            content_height: this
                                .content_height(cx)
                                .unwrap_or(change.content_height),
                            content_width: this.content_width(cx),
                            ..*change
                        };
                        cx.emit(DiffEditorEvent::ContentSizeChanged(event));
                    }
                    _ => {}
                }
            }
        };
        self._subscriptions = vec![
            cx.subscribe(&original_model, |this, _, event: &TextModelEvent, cx| {
                if let TextModelEvent::Edited { changes, .. } = event {
                    this.model_edited(true, changes, cx);
                    this.compute_diff(false, cx);
                }
            }),
            cx.subscribe(&modified_model, |this, _, event: &TextModelEvent, cx| {
                if let TextModelEvent::Edited { changes, .. } = event {
                    this.model_edited(false, changes, cx);
                    this.compute_diff(false, cx);
                }
            }),
            cx.subscribe(&self.original, editor_events(true)),
            cx.subscribe(&self.modified, editor_events(false)),
        ];
    }

    /// The diff editor's unique id, `vs.editor.IDiffEditor:v2:1`, … in
    /// creation order (Monaco `getId`).
    pub fn id(&self) -> String {
        format!("{}:v2:{}", self.editor_type(), self.id)
    }

    /// [`EditorType::DiffEditor`](crate::EditorType::DiffEditor) (Monaco
    /// `getEditorType`).
    pub fn editor_type(&self) -> crate::EditorType {
        crate::EditorType::DiffEditor
    }

    /// The editor showing the original model (Monaco `getOriginalEditor`).
    pub fn original_editor(&self) -> &Entity<CodeEditor> {
        &self.original
    }

    /// The editor showing the modified model (Monaco `getModifiedEditor`).
    pub fn modified_editor(&self) -> &Entity<CodeEditor> {
        &self.modified
    }

    /// The options.
    pub fn options(&self) -> &DiffEditorOptions {
        &self.options
    }

    /// Replaces the options (Monaco `updateOptions`).
    pub fn update_options(&mut self, mut options: DiffEditorOptions, cx: &mut Context<Self>) {
        options.split_view_default_ratio = clamp_split_view_ratio(options.split_view_default_ratio);
        let recompute = options.ignore_trim_whitespace != self.options.ignore_trim_whitespace
            || options.max_computation_time_ms != self.options.max_computation_time_ms
            || options.show_moves != self.options.show_moves
            || options.hide_unchanged_regions != self.options.hide_unchanged_regions
            || options.max_file_size != self.options.max_file_size
            || options.diff_algorithm != self.options.diff_algorithm
            || options.compact_mode != self.options.compact_mode
            // `compact_mode` decides on the first diff with it.
            || (options.compact_mode
                && options.use_true_inline_view != self.options.use_true_inline_view);
        if options.compact_mode && options.use_true_inline_view != self.options.use_true_inline_view
        {
            self.simple_diff = None;
        }
        if options.original_editable != self.options.original_editable {
            let read_only = !options.original_editable;
            self.original.update(cx, |editor, cx| {
                let mut editor_options = editor.options().clone();
                editor_options.read_only = read_only;
                editor.update_options(editor_options, cx);
            });
        }
        if options.split_view_default_ratio != self.options.split_view_default_ratio {
            self.ratio = options.split_view_default_ratio;
        }
        self.inline = !self.side_by_side_for(&options);
        let only_viewer = options.only_show_accessible_diff_viewer;
        let was_only_viewer = self.options.only_show_accessible_diff_viewer;
        self.options = options;
        self.apply_word_wrap(cx);
        self.apply_editor_options(cx);
        if only_viewer {
            self.set_accessible_viewer_visible(true, cx);
        } else if was_only_viewer {
            self.set_accessible_viewer_visible(false, cx);
        }
        if recompute {
            self.compute_diff(true, cx);
        } else {
            self.apply_diff(cx);
        }
        cx.notify();
    }

    /// Shows other models (Monaco `setModel`).
    pub fn set_models(
        &mut self,
        original: Entity<TextModel>,
        modified: Entity<TextModel>,
        cx: &mut Context<Self>,
    ) {
        self.set_model(Some(DiffEditorModel { original, modified }), cx);
    }

    /// Shows other models, or none (Monaco `setModel`, with `null`): without
    /// a model the editors show empty text and there's no diff.
    pub fn set_model(&mut self, model: Option<DiffEditorModel>, cx: &mut Context<Self>) {
        // Monaco closes the accessible diff viewer when the models go.
        if model.is_none() && self.has_model {
            self.set_accessible_viewer_visible(false, cx);
        }
        self.clear_diff(cx);
        self.simple_diff = None;
        self.has_model = model.is_some();
        let DiffEditorModel { original, modified } = model.unwrap_or_else(|| DiffEditorModel {
            original: cx.new(|_| TextModel::new("")),
            modified: cx.new(|_| TextModel::new("")),
        });
        self.original
            .update(cx, |editor, cx| editor.set_model(original, cx));
        self.modified
            .update(cx, |editor, cx| editor.set_model(modified, cx));
        self.subscribe(cx);
        if self.has_model {
            self.compute_diff(true, cx);
        } else {
            self.resolve_diff_waiters();
        }
        cx.emit(DiffEditorEvent::ModelChanged);
        cx.notify();
    }

    /// The models shown (Monaco `getModel`), `None` after
    /// `set_model(None)`.
    pub fn model(&self, cx: &App) -> Option<DiffEditorModel> {
        self.has_model.then(|| DiffEditorModel {
            original: self.original.read(cx).model().clone(),
            modified: self.modified.read(cx).model().clone(),
        })
    }

    /// The editors' word wrap overrides (Monaco `_adjustOptionsForLeftHandSide`
    /// and `...RightHandSide`): `diff_word_wrap`, except that the hidden
    /// original editor never wraps inline.
    fn apply_word_wrap(&mut self, cx: &mut Context<Self>) {
        let (original1, original2) = if self.inline {
            (WordWrapOverride::Off, WordWrapOverride::Off)
        } else {
            (self.options.diff_word_wrap, WordWrapOverride::Inherit)
        };
        let set = |editor: &Entity<CodeEditor>,
                   override1: WordWrapOverride,
                   override2: Option<WordWrapOverride>,
                   cx: &mut Context<Self>| {
            editor.update(cx, |editor, cx| {
                let current = editor.options();
                if current.word_wrap_override1 == override1
                    && override2.is_none_or(|o| current.word_wrap_override2 == o)
                {
                    return;
                }
                let mut options = current.clone();
                options.word_wrap_override1 = override1;
                if let Some(override2) = override2 {
                    options.word_wrap_override2 = override2;
                }
                editor.update_options(options, cx);
            });
        };
        set(&self.original, original1, Some(original2), cx);
        set(&self.modified, self.options.diff_word_wrap, None, cx);
    }

    /// What Monaco's `_adjustOptionsForSubEditor`, `...LeftHandSide` and
    /// `...RightHandSide` set on the editors besides word wrap: no minimap
    /// or folding; no sticky scroll with hidden unchanged regions (nor in
    /// the hidden original editor inline); no unicode highlighting in the
    /// hidden original editor; the original's glyph margin only side by
    /// side; `code_lens` from `diff_code_lens`; the aria labels, with the
    /// `accessibility_verbose` tip.
    fn apply_editor_options(&mut self, cx: &mut Context<Self>) {
        let code_lens = self.options.diff_code_lens;
        let verbose = self.options.accessibility_verbose;
        let inline = self.inline;
        let hide_sticky = self.options.hide_unchanged_regions.enabled;
        for (ix, editor) in [&self.original, &self.modified].into_iter().enumerate() {
            let original = ix == 0;
            let label = if original {
                self.options.original_aria_label.clone()
            } else {
                self.options.modified_aria_label.clone()
            };
            let sticky_off = hide_sticky || (original && inline);
            let saved_sticky = &mut self.host_options.sticky_scroll[ix];
            let saved_unicode = &mut self.host_options.unicode_highlight;
            editor.update(cx, |editor, cx| {
                let current = editor.options();
                let mut options = current.clone();
                let aria_label = label.as_deref().unwrap_or(&current.aria_label);
                options.aria_label = aria_label_with_tip(aria_label, verbose);
                options.code_lens = code_lens;
                options.minimap.enabled = false;
                options.folding = false;
                // Monaco `_adjustOptionsForSubEditor`: alignment assumes
                // every line is the line height.
                options.allow_variable_line_heights = false;
                if sticky_off {
                    if options.sticky_scroll.enabled {
                        *saved_sticky = Some(true);
                        options.sticky_scroll.enabled = false;
                    }
                } else if let Some(enabled) = saved_sticky.take() {
                    options.sticky_scroll.enabled = enabled;
                }
                if original {
                    options.glyph_margin = !inline;
                    let highlights = &mut options.unicode_highlight;
                    let shows_any = highlights.non_basic_ascii
                        || highlights.ambiguous_characters
                        || highlights.invisible_characters;
                    if inline {
                        if shows_any {
                            *saved_unicode = Some(highlights.clone());
                            highlights.non_basic_ascii = false;
                            highlights.ambiguous_characters = false;
                            highlights.invisible_characters = false;
                        }
                    } else if let Some(saved) = saved_unicode.take() {
                        *highlights = saved;
                    }
                }
                if options != *current {
                    editor.update_options(options, cx);
                }
            });
        }
    }

    /// Monaco `renderSideBySide` after `useInlineViewWhenSpaceIsLimited`:
    /// whether the sides show next to each other (else deleted lines show
    /// inline in the modified editor).
    pub fn renders_side_by_side(&self) -> bool {
        !self.inline
    }

    /// Monaco `renderSideBySideInAutomaticMode`: whether the automatic view
    /// mode shows the sides next to each other at this width (the
    /// `diffEditorAutomaticRenderSideBySide` key).
    pub fn renders_side_by_side_in_automatic_mode(&self) -> bool {
        if self.options.compact_mode && self.simple_diff == Some(true) {
            return false;
        }
        self.bounds.get().size.width > self.options.render_side_by_side_inline_breakpoint
            || self.screen_reader_mode
    }

    /// Whether `options` and the width call for side by side.
    fn side_by_side_for(&self, options: &DiffEditorOptions) -> bool {
        // Monaco `compactMode`: only insertions and deletions show inline.
        if options.compact_mode && self.simple_diff == Some(true) {
            return false;
        }
        let width = self.bounds.get().size.width;
        // Screen reader users keep the sides apart (Monaco
        // `_screenReaderMode`). Monaco `couldShowInlineViewBecauseOfSize`
        // is `width <= breakpoint`.
        options.render_side_by_side
            && !(options.use_inline_view_when_space_is_limited
                && width > px(0.)
                && width <= options.render_side_by_side_inline_breakpoint
                && !self.screen_reader_mode)
    }

    /// Monaco `inlineViewHideOriginalLineNumbers`: the inline view hides the
    /// original line numbers.
    fn hides_original_line_numbers(&self) -> bool {
        self.options.hide_original_line_numbers || self.options.compact_mode
    }

    /// Monaco's inline layout: the original editor shows as a column of its
    /// line numbers, `max(5, layoutInfoDecorationsLeft)` wide, or not at
    /// all with `hide_original_line_numbers`.
    fn inline_original_width(&self, cx: &App) -> Pixels {
        if self.hides_original_line_numbers() {
            return px(0.);
        }
        self.original
            .read(cx)
            .layout
            .as_ref()
            .map_or(px(5.), |l| l.layout_info.decorations_left.max(px(5.)))
    }

    /// Monaco `getLineChanges` (deprecated there for
    /// `getDiffComputationResult().changes2`, here [`DiffEditor::diff`]):
    /// the changes as `ILineChange`s, once computed.
    pub fn line_changes(&self) -> Option<Vec<LineChange>> {
        self.diff.as_deref().map(to_line_changes)
    }

    /// Monaco `getDiffComputationResult().identical`: no changes and the
    /// same text (whitespace changes ignored by `ignore_trim_whitespace`
    /// leave no changes, but the text differs), once computed.
    pub fn identical(&self) -> Option<bool> {
        self.diff.is_some().then_some(self.identical)
    }

    /// Monaco `waitForDiff`: resolves once the diff for the current text is
    /// computed (right away when there's nothing to compute).
    pub fn wait_for_diff(&mut self) -> impl Future<Output = ()> + use<> {
        let (sender, receiver) = futures::channel::oneshot::channel();
        if self.has_model && self.diff_task.is_some() {
            self.diff_waiters.push(sender);
        } else {
            sender.send(()).ok();
        }
        async move {
            receiver.await.ok();
        }
    }

    fn resolve_diff_waiters(&mut self) {
        for waiter in self.diff_waiters.drain(..) {
            waiter.send(()).ok();
        }
    }

    /// Adds an action to the modified editor (Monaco
    /// `IStandaloneDiffEditor.addAction`, which delegates the same way)
    /// until the returned [`Disposable`](crate::Disposable) is disposed.
    pub fn add_action(
        &mut self,
        descriptor: crate::editor_actions::EditorActionDescriptor,
        cx: &mut Context<Self>,
    ) -> crate::registry::Disposable {
        self.modified
            .update(cx, |editor, cx| editor.add_action(descriptor, cx))
    }

    /// Adds a command run by `keystrokes` (GPUI syntax, e.g. `"cmd-k
    /// cmd-m"`) in this diff editor (Monaco
    /// `IStandaloneDiffEditor.addCommand`, which delegates to the modified
    /// editor's per-editor `addCommand`), with either side focused, while
    /// `when` (a GPUI predicate over the focused side's
    /// [context keys](CodeEditor::context_keys)) holds. Returns the
    /// command's generated id, which [`DiffEditor::trigger`] runs too, and
    /// the [`Disposable`](crate::Disposable) that removes it. Fails if the
    /// keystrokes or `when` don't parse.
    pub fn add_command(
        &mut self,
        keystrokes: &str,
        handler: impl Fn(&mut DiffEditor, &mut Window, &mut Context<DiffEditor>) + 'static,
        when: Option<&str>,
        cx: &mut Context<Self>,
    ) -> anyhow::Result<(SharedString, crate::registry::Disposable)> {
        let id: SharedString = crate::editor_commands::next_dynamic_command_id().into();
        // Either side's editor (each names itself in its key context).
        let sides = format!(
            "{} && ({key} == {} || {key} == {})",
            crate::actions::KEY_CONTEXT,
            self.original.read(cx).commands.id,
            self.modified.read(cx).commands.id,
            key = crate::context_keys::EDITOR_ID_KEY,
        );
        let context = match when {
            Some(when) => format!("{sides} && ({when})"),
            None => sides,
        };
        let binding = gpui::KeyBinding::load(
            keystrokes,
            Box::new(crate::actions::RunCommand {
                id: id.to_string(),
                args: None,
            }),
            Some(Rc::new(gpui::KeyBindingContextPredicate::parse(&context)?)),
            false,
            None,
            &gpui::DummyKeyboardMapper,
        )?;
        let bindings = crate::editor_actions::bind_removable(cx, vec![binding]);
        self.command_handlers.insert(id.clone(), Rc::new(handler));
        let editor = cx.weak_entity();
        let handler_id = id.clone();
        let disposable = crate::registry::Disposable::new(move |cx| {
            bindings.dispose(cx);
            editor
                .update(cx, |editor, _| editor.command_handlers.remove(&handler_id))
                .ok();
        });
        Ok((id, disposable))
    }

    /// Runs a command from [`DiffEditor::add_command`]; whether `id` is one.
    fn run_added_command(&mut self, id: &str, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let Some(handler) = self.command_handlers.get(id).cloned() else {
            return false;
        };
        handler(self, window, cx);
        true
    }

    /// Adds a button to the gutter menu (a Monaco `DiffEditorHunkToolbar` or
    /// `DiffEditorSelectionToolbar` menu item), replacing one with the same
    /// id. The gutter shows while it has buttons, even for a read-only
    /// modified side.
    pub fn add_hunk_action(&mut self, action: DiffHunkAction, cx: &mut Context<Self>) {
        self.hunk_actions.retain(|a| a.id != action.id);
        self.hunk_actions.push(action);
        cx.notify();
    }

    /// Removes a button added with [`DiffEditor::add_hunk_action`].
    pub fn remove_hunk_action(&mut self, id: &str, cx: &mut Context<Self>) -> bool {
        let len = self.hunk_actions.len();
        self.hunk_actions.retain(|a| a.id.as_ref() != id);
        cx.notify();
        self.hunk_actions.len() != len
    }

    /// Monaco `applyOriginalEdits`/`applyModifiedEdits`: moves the shown
    /// diff's line ranges through an edit, so the sides stay aligned until
    /// the diff is computed again. Monaco has these disabled (a TODO) and
    /// keeps the stale diff; this ports their line range part
    /// (`applyEditToLineRange`). Changes the edit touches lose their inner
    /// changes until then.
    fn model_edited(&mut self, original: bool, changes: &[ContentChange], cx: &mut Context<Self>) {
        let Some(diff) = self.diff.as_deref() else {
            return;
        };
        // (start line, end line, line delta), in descending order.
        let edits: Vec<(usize, usize, isize)> = changes
            .iter()
            .map(|c| {
                let start = c.range.start.row as usize + 1;
                let end = c.range.end.row as usize + 1;
                let new_lines = c.text.matches('\n').count() as isize;
                (start, end, new_lines - (end - start) as isize)
            })
            .collect();
        if edits.iter().all(|(_, _, delta)| *delta == 0) {
            return;
        }
        let shift_inner = |inner: &mut Vec<RangeMapping>, delta: isize| {
            for c in inner {
                let range = if original {
                    &mut c.original_range
                } else {
                    &mut c.modified_range
                };
                range.start_line_number = (range.start_line_number as isize + delta) as usize;
                range.end_line_number = (range.end_line_number as isize + delta) as usize;
            }
        };
        let apply = |m: &mut DetailedLineRangeMapping| -> Option<()> {
            let range = if original { m.original } else { m.modified };
            let moved = apply_edits_to_line_range(range, &edits)?;
            if moved.len() == range.len() {
                shift_inner(
                    &mut m.inner_changes,
                    moved.start_line_number as isize - range.start_line_number as isize,
                );
            } else {
                m.inner_changes.clear();
            }
            if original {
                m.original = moved;
            } else {
                m.modified = moved;
            }
            Some(())
        };
        let mut new = diff.clone();
        for change in &mut new.changes {
            if apply(change).is_none() {
                return;
            }
        }
        let move_count = new.moves.len();
        new.moves.retain_mut(|m| {
            let lines = &mut m.line_range_mapping;
            let range = if original {
                &mut lines.original
            } else {
                &mut lines.modified
            };
            let Some(moved) = apply_edits_to_line_range(*range, &edits) else {
                return false;
            };
            *range = moved;
            m.changes.iter_mut().all(|c| apply(c).is_some())
        });
        if new.moves.len() != move_count {
            let compared = self.compared_move_range();
            self.move_to_compare = Self::find_compared_move(&new, compared);
            self.active_move = None;
        }
        self.diff = Some(Arc::new(new));
        self.schedule_alignment(cx);
        cx.notify();
    }

    /// Monaco `_handleCursorPositionChange`: a cursor moved onto a change
    /// plays its signal.
    fn signal_change_at_cursor(&mut self, original: bool, cx: &mut Context<Self>) {
        let Some(diff) = self.diff.clone() else {
            return;
        };
        let editor = if original {
            &self.original
        } else {
            &self.modified
        };
        let line = editor.read(cx).selections()[0].head.row as usize + 1;
        let change = diff.changes.iter().find(|m| {
            if original {
                m.original.contains(line)
            } else {
                m.modified.contains(line)
            }
        });
        if let Some(change) = change {
            self.play_signal(change_signal(change), cx);
        }
    }

    /// Monaco `DiffEditorBreadcrumbsSource`: the modified model's symbols
    /// (from its document symbol providers, or the language's tags), for the
    /// hidden-lines bars.
    fn update_breadcrumb_symbols(&mut self, cx: &mut Context<Self>) {
        if !self.options.hide_unchanged_regions.enabled || self.shown_unchanged().is_empty() {
            return;
        }
        let task = self
            .modified
            .update(cx, |editor, cx| editor.document_symbols(cx));
        self.breadcrumb_task = Some(cx.spawn(async move |this, cx| {
            let symbols = task.await;
            this.update(cx, |this, cx| {
                let symbols: Vec<BreadcrumbSymbol> = crate::outline::flatten_symbols(&symbols)
                    .into_iter()
                    .map(|(_, _, s)| BreadcrumbSymbol {
                        name: s.name.clone(),
                        kind: s.kind,
                        start_line_number: s.range.start.row as usize + 1,
                        end_line_number: s.range.end.row as usize + 1,
                    })
                    .collect();
                if symbols != this.breadcrumb_symbols {
                    this.breadcrumb_symbols = symbols;
                    this.apply_unchanged(cx);
                }
            })
            .ok();
        }));
    }

    /// Monaco `_revealModifiedHiddenLine`: shows a hidden modified line from
    /// the bottom of its region and reveals it.
    fn reveal_modified_hidden_line(&mut self, line: usize, cx: &mut Context<Self>) {
        let Some(region) = self
            .unchanged_regions_mut()
            .iter_mut()
            .find(|r| r.hidden_modified_range().contains(line))
        else {
            return;
        };
        region.show_modified_line(line, RevealPreference::FromBottom);
        self.apply_unchanged(cx);
        let point = PointUtf16::new(line as u32 - 1, 0);
        self.modified.update(cx, |editor, cx| {
            editor.reveal_range(point..point, crate::view::reveal::RevealType::Center, cx)
        });
    }

    /// Monaco's "Fold Unchanged Region" glyph (`fold-unchanged`) on the
    /// first line of each fully shown region, on both sides.
    fn apply_fold_glyphs(&mut self, cx: &mut Context<Self>) {
        let (old_original, old_modified) = std::mem::take(&mut self.fold_glyphs);
        for (editor, old) in [
            (&self.original, old_original),
            (&self.modified, old_modified),
        ] {
            editor.update(cx, |editor, cx| {
                for id in old {
                    editor.remove_widget(id, cx);
                }
            });
        }
        // Monaco skips it only in its `compact` variant (multi diff
        // cards), which the port doesn't have, not with `compactMode`.
        let colors = self.modified.read(cx).colors().clone();
        let this = cx.entity().downgrade();
        let mut glyphs = (Vec::new(), Vec::new());
        for (index, region) in self.shown_unchanged().iter().enumerate() {
            if !self.hides_unchanged_controls(index) {
                continue;
            }
            let sides = [
                (&self.original, region.original_line_number, &mut glyphs.0),
                (&self.modified, region.modified_line_number, &mut glyphs.1),
            ];
            for (editor, line, ids) in sides {
                let view = cx.new(|_| FoldUnchangedGlyph {
                    diff: this.clone(),
                    index,
                    colors: colors.clone(),
                });
                ids.push(editor.update(cx, |editor, cx| {
                    editor.add_glyph_margin_widget(
                        crate::widgets::GlyphMarginWidget::new(line as u32 - 1, view),
                        cx,
                    )
                }));
            }
        }
        self.fold_glyphs = glyphs;
    }

    /// The height to show everything without scrolling, as of the last
    /// layout: the taller side's lines and view zones (Monaco
    /// `onDidContentSizeChange`'s `contentHeight`).
    pub fn content_height(&self, cx: &App) -> Option<Pixels> {
        let modified = self.modified.read(cx).content_height();
        if self.inline {
            return modified;
        }
        match (self.original.read(cx).content_height(), modified) {
            (Some(a), Some(b)) => Some(a.max(b)),
            (a, b) => a.or(b),
        }
    }

    /// The diff editor's context keys, which GPUI sets on its element and
    /// which both editors inherit, as Monaco's editors inherit them from the
    /// diff editor's scoped context key service: the
    /// [`DIFF_EDITOR_KEY_CONTEXT`] identifier, `isInDiffEditor`,
    /// `inDiffEditor`, `isInDiffLeftEditor`/`isInDiffRightEditor` (that
    /// side has focus), `isEmbeddedDiffEditor`, `comparingMovedCode`,
    /// `diffEditorRenderSideBySideInlineBreakpointReached`,
    /// `diffEditorInlineMode`, `diffEditorAutomaticRenderSideBySide`,
    /// `diffEditorHasChanges`, `diffEditorOriginalWritable`,
    /// `diffEditorModifiedWritable`, the `diffEditorOriginalUri` and
    /// `diffEditorModifiedUri` strings, and, in a
    /// [`MultiDiffEditor`](crate::MultiDiffEditor), `inMultiDiffEditor` and
    /// `multiDiffEditorAllCollapsed`. So `add_command`'s `when` can be
    /// `isInDiffEditor && diffEditorHasChanges`. Focus keys are as of the
    /// last frame.
    pub fn context_keys(&self, cx: &App) -> gpui::KeyContext {
        key_context_with(&self.key_values(cx))
    }

    /// Monaco's diff editor keys (`diffEditorWidget.ts`,
    /// `DiffEditorEditors`, `MultiDiffEditorWidgetImpl`).
    fn key_values(&self, cx: &App) -> Vec<(SharedString, ContextKeyValue)> {
        let options = &self.options;
        let width = self.bounds.get().size.width;
        let breakpoint = options.render_side_by_side_inline_breakpoint;
        let flag = |key: &'static str, on: bool| (SharedString::new_static(key), on.into());
        let uri = |key: &'static str, editor: &Entity<CodeEditor>| {
            let uri = if self.has_model {
                editor.read(cx).model().read(cx).resource().clone()
            } else {
                SharedString::default()
            };
            (SharedString::new_static(key), uri.into())
        };
        let mut keys = vec![
            flag("isInDiffEditor", true),
            // Monaco's editors set it from their `inDiffEditor` option.
            flag("inDiffEditor", true),
            flag("isInDiffLeftEditor", self.focused_sides.0),
            flag("isInDiffRightEditor", self.focused_sides.1),
            flag(IS_EMBEDDED_DIFF_EDITOR, options.is_in_embedded_editor),
            flag(COMPARING_MOVED_CODE, self.move_to_compare.is_some()),
            // Monaco `couldShowInlineViewBecauseOfSize`.
            flag(
                "diffEditorRenderSideBySideInlineBreakpointReached",
                options.render_side_by_side && width <= breakpoint,
            ),
            flag("diffEditorInlineMode", self.inline),
            // Monaco `renderSideBySideInAutomaticMode`: wide enough, or a
            // screen reader.
            flag(
                "diffEditorAutomaticRenderSideBySide",
                !(options.compact_mode && self.simple_diff == Some(true))
                    && (width > breakpoint || self.screen_reader_mode),
            ),
            flag(
                "diffEditorHasChanges",
                self.diff.as_ref().is_some_and(|d| !d.changes.is_empty()),
            ),
            flag("diffEditorOriginalWritable", options.original_editable),
            flag(
                "diffEditorModifiedWritable",
                !self.modified.read(cx).options().read_only,
            ),
            uri("diffEditorOriginalUri", &self.original),
            uri("diffEditorModifiedUri", &self.modified),
        ];
        if let Some(all_collapsed) = self.multi_diff_all_collapsed {
            keys.push(flag("inMultiDiffEditor", true));
            keys.push(flag("multiDiffEditorAllCollapsed", all_collapsed));
        }
        keys
    }

    /// Notes which side has focus and passes the context keys on to both
    /// editors when they changed (each frame).
    fn pass_context_keys(&mut self, window: &Window, cx: &mut Context<Self>) {
        self.focused_sides = (
            self.original.focus_handle(cx).contains_focused(window, cx),
            self.modified.focus_handle(cx).contains_focused(window, cx),
        );
        let keys = self.key_values(cx);
        if keys == self.passed_keys {
            return;
        }
        for editor in [&self.original, &self.modified] {
            editor.update(cx, |editor, cx| {
                editor.commands.parent_keys = keys.clone();
                cx.notify();
            });
        }
        self.passed_keys = keys;
    }

    /// The width to show everything without scrolling, as of the last
    /// layout (Monaco `onDidContentSizeChange`'s `contentWidth`): both
    /// editors' [`content_width`](CodeEditor::content_width) and the 30px
    /// overview ruler.
    pub fn content_width(&self, cx: &App) -> Pixels {
        let width =
            |editor: &Entity<CodeEditor>| editor.read(cx).content_width().unwrap_or_default();
        width(&self.original) + width(&self.modified) + px(DIFF_OVERVIEW_WIDTH)
    }

    /// Whether a diff is being computed, so what shows is about to change
    /// (Monaco's `isDiffUpToDate` is false meanwhile).
    pub(crate) fn is_computing_diff(&self) -> bool {
        self.diff_task.is_some()
    }

    /// The widest horizontal scroll range of the sides, as of the last
    /// layout.
    pub(crate) fn max_scroll_left(&self, cx: &App) -> Pixels {
        [&self.original, &self.modified]
            .iter()
            .filter_map(|e| e.read(cx).layout.as_ref().map(|l| l.max_scroll.x))
            .fold(px(0.), |a, b| a.max(b))
    }

    /// Scrolls both sides to `position` without clamping, for a host that
    /// sizes the diff editor as it scrolls (the multi-diff editor).
    pub(crate) fn set_scroll_position_unclamped(
        &mut self,
        position: gpui::Point<Pixels>,
        cx: &mut Context<Self>,
    ) {
        for editor in [&self.original, &self.modified] {
            editor.update(cx, |editor, cx| {
                editor.set_scroll_position_unclamped(position, cx)
            });
        }
    }

    /// The window bounds of the modified editor's glyph margin widgets (the
    /// margin revert arrows), for tests.
    #[doc(hidden)]
    pub fn revert_arrow_bounds_for_tests(&self, cx: &App) -> Vec<Bounds<Pixels>> {
        let Some(layout) = self.modified.read(cx).layout.as_ref() else {
            return Vec::new();
        };
        layout
            .widget_bounds
            .iter()
            .zip(&layout.widget_kinds)
            .filter(|(_, kind)| **kind == crate::mouse_target::EmbeddedKind::GlyphMarginWidget)
            .map(|(bounds, _)| *bounds)
            .collect()
    }

    /// The gutter menu items' vertical spans in the modified editor, for
    /// tests.
    #[doc(hidden)]
    pub fn gutter_spans_for_tests(&self, cx: &App) -> Vec<(Pixels, Pixels)> {
        let editor = self.modified.read(cx);
        self.gutter_items(cx)
            .iter()
            .filter_map(|item| editor.rows_y_span(item.lines.to_rows()))
            .collect()
    }

    /// The moved block under the last moved cursor, for tests.
    #[doc(hidden)]
    pub fn active_move_for_tests(&self) -> Option<usize> {
        self.active_move
    }

    /// The alignment view zones on each side, for tests.
    #[doc(hidden)]
    pub fn alignment_zones_for_tests(&self) -> (usize, usize) {
        (self.zones.0.len(), self.zones.1.len())
    }

    /// The alignment zones: (in the original editor, after line, height),
    /// for tests.
    #[doc(hidden)]
    pub fn alignment_heights_for_tests(&self) -> Vec<(bool, usize, f32)> {
        self.alignment
            .iter()
            .map(|z| (z.original, z.after, z.height))
            .collect()
    }

    /// The inline view's original column width, for tests.
    #[doc(hidden)]
    pub fn inline_original_width_for_tests(&self, cx: &App) -> Pixels {
        self.inline_original_width(cx)
    }

    /// The diff overview's marks, for tests.
    #[doc(hidden)]
    #[allow(clippy::type_complexity)]
    pub fn overview_zones_for_tests(&self, cx: &App) -> (Vec<(f32, f32)>, Vec<(f32, f32)>) {
        self.overview_zones(cx)
    }

    /// The "N hidden lines" bars on each side, for tests.
    #[doc(hidden)]
    pub fn unchanged_bars_for_tests(&self) -> (usize, usize) {
        (self.unchanged_zones.0.len(), self.unchanged_zones.1.len())
    }

    /// The "Fold Unchanged Region" glyphs on each side, for tests.
    #[doc(hidden)]
    pub fn fold_glyphs_for_tests(&self) -> (usize, usize) {
        (self.fold_glyphs.0.len(), self.fold_glyphs.1.len())
    }

    /// Region `index`'s breadcrumbs, for tests.
    #[doc(hidden)]
    pub fn breadcrumbs_for_tests(&self, index: usize) -> Vec<String> {
        self.shown_unchanged()
            .get(index)
            .map_or(Vec::new(), |region| {
                breadcrumb_items(&self.breadcrumb_symbols, region.hidden_modified_range())
                    .into_iter()
                    .map(|s| s.name.to_string())
                    .collect()
            })
    }

    /// Clicks breadcrumb `item` of region `index`, for tests.
    #[doc(hidden)]
    pub fn click_breadcrumb_for_tests(
        &mut self,
        index: usize,
        item: usize,
        cx: &mut Context<Self>,
    ) {
        let Some(region) = self.shown_unchanged().get(index) else {
            return;
        };
        let items = breadcrumb_items(&self.breadcrumb_symbols, region.hidden_modified_range());
        if let Some(symbol) = items.get(item) {
            self.reveal_modified_hidden_line(symbol.start_line_number, cx);
        }
    }

    /// Clicks region `index`'s "Fold Unchanged Region" glyph, for tests.
    #[doc(hidden)]
    pub fn fold_unchanged_for_tests(&mut self, index: usize, cx: &mut Context<Self>) {
        self.update_unchanged_region(index, UnchangedRegion::collapse_all, cx);
    }

    /// The deleted lines zone `zone`'s margin menu for original row `row`,
    /// for tests.
    #[doc(hidden)]
    pub fn deleted_menu_for_tests(&self, zone: usize, row: u32, cx: &App) -> Vec<String> {
        self.deleted_code
            .get(zone)
            .and_then(|z| z.upgrade())
            .map_or(Vec::new(), |z| {
                z.read(cx)
                    .menu_items(row, cx)
                    .into_iter()
                    .map(|(_, label)| label)
                    .collect()
            })
    }

    /// Runs item `item` of the deleted lines zone `zone`'s margin menu for
    /// original row `row`, for tests.
    #[doc(hidden)]
    pub fn run_deleted_menu_item_for_tests(
        &mut self,
        zone: usize,
        row: u32,
        item: usize,
        cx: &mut Context<Self>,
    ) {
        let Some(view) = self.deleted_code.get(zone).and_then(|z| z.upgrade()) else {
            return;
        };
        view.update(cx, |view, cx| {
            if let Some((item, _)) = view.menu_items(row, cx).get(item).cloned() {
                view.run_menu_item(item, row, cx);
            }
        });
    }

    /// Selects `range` (original rows and UTF-16 columns) in the deleted
    /// lines zone `zone` and copies it, for tests.
    #[doc(hidden)]
    pub fn copy_deleted_text_for_tests(
        &mut self,
        zone: usize,
        range: Range<PointUtf16>,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(view) = self.deleted_code.get(zone).and_then(|z| z.upgrade()) else {
            return false;
        };
        view.update(cx, |view, cx| {
            view.selection = Some((range.start, range.end));
            cx.notify();
            view.copy_selection(cx)
        })
    }

    /// The deleted lines zones' view line counts and min widths, for
    /// tests.
    #[doc(hidden)]
    pub fn deleted_code_layout_for_tests(&self, cx: &App) -> Vec<(usize, Pixels)> {
        self.deleted_code
            .iter()
            .filter_map(|z| z.upgrade())
            .map(|z| {
                let z = z.read(cx);
                (z.view_lines(cx).len(), z.min_width(cx))
            })
            .collect()
    }

    /// Whether each deleted lines zone tints its view lines full width
    /// (Monaco's `view-line char-delete`, without inner changes), for
    /// tests.
    #[doc(hidden)]
    pub fn deleted_code_full_width_for_tests(&self, cx: &App) -> Vec<bool> {
        self.deleted_code
            .iter()
            .filter_map(|z| z.upgrade())
            .map(|z| z.read(cx).full_width_char_delete())
            .collect()
    }

    /// Runs gutter action `id` for gutter item `item`, for tests.
    #[doc(hidden)]
    pub fn run_hunk_action_for_tests(
        &mut self,
        item: usize,
        id: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(item) = self.gutter_items(cx).into_iter().nth(item) else {
            return false;
        };
        let toolbar = match item.revert {
            GutterRevert::Block(_) => DiffHunkToolbar::Hunk,
            GutterRevert::Selection(_) => DiffHunkToolbar::Selection,
        };
        let Some(action) = self
            .toolbar_actions(toolbar, cx)
            .into_iter()
            .find(|a| a.id.as_ref() == id)
        else {
            return false;
        };
        self.run_hunk_action(&action, &item.revert, &item.mapping, window, cx);
        true
    }

    /// Whether the gutter menu shows, for tests.
    #[doc(hidden)]
    pub fn shows_gutter_for_tests(&self, cx: &App) -> bool {
        self.shows_gutter(cx)
    }

    /// The latest diff (Monaco `getDiffComputationResult`), once computed.
    pub fn diff(&self) -> Option<&LinesDiff> {
        self.diff.as_deref()
    }

    /// Computes the diff again, after a pause in edits unless `now`.
    fn compute_diff(&mut self, now: bool, cx: &mut Context<Self>) {
        let original = self.original.read(cx).model().read(cx);
        let modified = self.modified.read(cx).model().read(cx);
        // Monaco `maxFileSize`: no diff for larger models.
        let max_bytes = self.options.max_file_size.saturating_mul(1024 * 1024);
        if max_bytes > 0
            && (original.buffer().len() as u64 > max_bytes
                || modified.buffer().len() as u64 > max_bytes)
        {
            if self.diff.is_some() || self.diff_task.is_some() {
                self.clear_diff(cx);
                cx.notify();
            }
            return;
        }
        let versions = (original.buffer().version(), modified.buffer().version());
        let original_lines: Vec<String> = original.buffer().rope().lines().collect();
        let modified_lines: Vec<String> = modified.buffer().rope().lines().collect();
        let options = DiffOptions {
            ignore_trim_whitespace: self.options.ignore_trim_whitespace,
            max_computation_time_ms: self.options.max_computation_time_ms,
            compute_moves: self.options.show_moves,
            extend_to_subwords: false,
        };
        let algorithm = self.options.diff_algorithm;
        self.diff_task = Some(cx.spawn(async move |this, cx| {
            if !now {
                cx.background_executor().timer(RECOMPUTE_DELAY).await;
            }
            let (diff, identical) = cx
                .background_executor()
                .spawn(async move {
                    let diff = match algorithm {
                        DiffAlgorithm::Advanced => {
                            compute_diff(&original_lines, &modified_lines, &options)
                        }
                        DiffAlgorithm::Legacy => {
                            compute_legacy_diff(&original_lines, &modified_lines, &options)
                        }
                    };
                    // Monaco's worker: `identical` when there are no
                    // changes and the texts are equal.
                    let identical = diff.changes.is_empty() && original_lines == modified_lines;
                    let diff = normalize_document_diff(diff, &original_lines, &modified_lines);
                    (diff, identical)
                })
                .await;
            this.update(cx, |this, cx| {
                // Monaco `isSimpleDiff`, decided by the first diff: only
                // insertions and deletions, or changes the true inline view
                // shows.
                if this.simple_diff.is_none() {
                    let true_inline = this.options.use_true_inline_view;
                    this.simple_diff = Some(diff.changes.iter().all(|c| {
                        c.original.is_empty()
                            || c.modified.is_empty()
                            || (true_inline && allows_true_inline_diff_rendering(c))
                    }));
                }
                // Monaco: the compared move is looked up again by its
                // modified range, rather than kept by index.
                let compared = this.compared_move_range();
                this.diff = Some(Arc::new(diff));
                this.move_to_compare = this
                    .diff
                    .as_deref()
                    .and_then(|diff| Self::find_compared_move(diff, compared));
                // Monaco's `setActiveMovedTextFromCursor` autorun reads the
                // diff too.
                this.sync_active_move(this.last_cursor_original, cx);
                this.identical = identical;
                this.diff_versions = Some(versions);
                this.diff_task = None;
                this.inline = !this.side_by_side_for(&this.options);
                this.apply_word_wrap(cx);
                this.apply_diff(cx);
                if let Some(ranges) = this.pending_collapsed_regions.take() {
                    this.restore_collapsed_regions(&ranges, cx);
                }
                this.update_breadcrumb_symbols(cx);
                cx.emit(DiffEditorEvent::DiffUpdated);
                if std::mem::take(&mut this.reveal_first_diff_pending) {
                    this.reveal_first_change(cx);
                }
                this.resolve_diff_waiters();
                cx.notify();
            })
            .ok();
        }));
    }

    fn clear_diff(&mut self, cx: &mut Context<Self>) {
        self.diff = None;
        self.identical = false;
        self.diff_task = None;
        self.alignment.clear();
        let (original, modified) = std::mem::take(&mut self.decorations);
        let (original_zones, modified_zones) = std::mem::take(&mut self.zones);
        clear_side(&self.original, original, original_zones, cx);
        clear_side(&self.modified, modified, modified_zones, cx);
        let (original_zones, modified_zones) = std::mem::take(&mut self.move_caption_zones);
        clear_side(&self.original, Vec::new(), original_zones, cx);
        clear_side(&self.modified, Vec::new(), modified_zones, cx);
        self.unchanged = None;
        self.apply_unchanged(cx);
        self.active_move = None;
        self.move_to_compare = None;
        self.apply_move_decorations(cx);
        // Monaco's viewer has no groups while there's no diff.
        if let Some(viewer) = self.accessible_viewer.clone() {
            viewer.update(cx, |viewer, cx| viewer.set_diff(None, cx));
        }
    }

    /// Decorates both sides and aligns them with view zones.
    fn apply_diff(&mut self, cx: &mut Context<Self>) {
        let Some(diff) = self.diff.clone() else {
            return;
        };
        let theme = self.modified.read(cx).theme().clone();
        let colors = theme.colors.clone();
        let high_contrast = theme.high_contrast;
        self.decorated_theme = Some(theme);
        // Moves aren't shown any more (inline, or `show_moves` off).
        if self
            .move_to_compare
            .is_some_and(|ix| ix >= self.shown_moves(&diff).len())
        {
            self.move_to_compare = None;
        }
        // Comparing a moved block shows only its changes.
        let compared = self
            .move_to_compare
            .and_then(|ix| self.shown_moves(&diff).get(ix));
        let changes = compared.map_or(&diff.changes, |m| &m.changes);
        let original_rope = self
            .original
            .read(cx)
            .model()
            .read(cx)
            .buffer()
            .rope()
            .clone();
        let (original_decorations, modified_decorations) = diff_decorations(
            changes,
            &self.options,
            self.uses_true_inline_view(),
            &original_rope,
            &colors,
            high_contrast,
        );
        let original_id = self.original.entity_id();
        let modified_id = self.modified.entity_id();
        let (old_original, old_modified) = std::mem::take(&mut self.decorations);
        let to_offsets = |model: &TextModel, decorations: Vec<(Side, DecorationOptions)>| {
            decorations
                .into_iter()
                .map(|(range, options)| (range.to_offsets(model.buffer().rope()), options))
                .collect::<Vec<_>>()
        };
        let original_model = self.original.read(cx).model().clone();
        let modified_model = self.modified.read(cx).model().clone();
        let new_original = to_offsets(original_model.read(cx), original_decorations);
        let new_modified = to_offsets(modified_model.read(cx), modified_decorations);
        let original_ids = original_model.update(cx, |model, cx| {
            model.delta_decorations(Some(original_id), &old_original, new_original, cx)
        });
        let modified_ids = modified_model.update(cx, |model, cx| {
            model.delta_decorations(Some(modified_id), &old_modified, new_modified, cx)
        });
        self.decorations = (original_ids, modified_ids);
        self.apply_alignment(&diff, cx);
        self.apply_move_decorations(cx);
        self.apply_revert_arrows(cx);
        self.update_unchanged_regions(&diff, cx);
        if let Some(viewer) = self.accessible_viewer.clone() {
            viewer.update(cx, |viewer, cx| viewer.set_diff(Some(&diff), cx));
        }
    }

    /// The view zones the diff editor aligns with, which alignment ignores
    /// (Monaco `_origViewZonesToIgnore`/`_modViewZonesToIgnore`): its
    /// alignment zones and hidden-lines bars. Moved-block captions and
    /// every other zone (code lenses, peek views, the find and problem
    /// zones, host zones) count.
    fn ignored_zones(&self, original: bool) -> Vec<WidgetId> {
        let (alignment, bars) = if original {
            (&self.zones.0, &self.unchanged_zones.0)
        } else {
            (&self.zones.1, &self.unchanged_zones.1)
        };
        alignment.iter().chain(bars).copied().collect()
    }

    /// Monaco `DiffEditorViewZones.viewZones`: the zones that align the
    /// sides for `diff`, from `computeRangeAlignment` over each side's
    /// extra line heights (wrapping and other view zones).
    fn alignment_zones(&self, diff: &LinesDiff, cx: &App) -> Vec<AlignmentZone> {
        let original_editor = self.original.read(cx);
        let modified_editor = self.modified.read(cx);
        let original_line_height = f32::from(original_editor.line_height());
        let modified_line_height = f32::from(modified_editor.line_height());
        let extra = |editor: &CodeEditor, original: bool| {
            let mut extra = editor.wrap_extra_heights();
            extra.extend(editor.zone_heights(&self.ignored_zones(original), cx));
            merge_extra_heights(&mut extra);
            extra
        };
        let original_extra = extra(original_editor, true);
        let modified_extra = extra(modified_editor, false);
        let original_rope = original_editor.model().read(cx).buffer().rope().clone();
        let original_line_count = original_rope.line_count() as usize;
        let max_column = |line: usize| {
            (line >= 1 && line <= original_line_count)
                .then(|| original_rope.line_len_utf16(line as u32 - 1) as usize + 1)
        };
        let side_by_side = !self.inline;
        let alignments = compute_range_alignment(
            &diff.changes,
            &original_extra,
            &modified_extra,
            original_line_height,
            modified_line_height,
            &max_column,
            side_by_side,
        );
        let compared = self
            .move_to_compare
            .and_then(|ix| self.shown_moves(diff).get(ix));
        // Monaco `syncedMovedText...delta(-1).deltaLength(2).contains(line)`.
        let near = |range: LineRange, line: usize| {
            line + 1 >= range.start_line_number && line <= range.end_line_number_exclusive
        };
        let true_inline = self.uses_true_inline_view();
        let arrows = self.shows_margin_revert_arrows(cx);
        let mut zones = Vec::new();
        for a in &alignments {
            let change = a.diff.map(|ix| (ix, &diff.changes[ix]));
            match change {
                Some((ix, change))
                    if !side_by_side
                        && !(true_inline && allows_true_inline_diff_rendering(change)) =>
                {
                    if !a.original_range.is_empty() {
                        // `a.original_range` can be out of bounds until the
                        // diff is computed again (Monaco returns early).
                        if a.original_range.end_line_number_exclusive - 1 > original_line_count {
                            return zones;
                        }
                        let line_counts: Vec<usize> = (a.original_range.start_line_number
                            ..a.original_range.end_line_number_exclusive)
                            .map(|line| {
                                modified_editor
                                    .line_breaks_for(&original_rope.line(line as u32 - 1))
                                    .0
                                    .len()
                            })
                            .collect();
                        // The (collapsed) original editor doesn't wrap:
                        // fill for the deleted lines' wrapped rows.
                        for (i, count) in line_counts.iter().enumerate() {
                            if *count > 1 {
                                zones.push(AlignmentZone {
                                    original: true,
                                    after: a.original_range.start_line_number + i,
                                    height: (*count - 1) as f32 * modified_line_height,
                                    kind: AlignmentZoneKind::Fill,
                                });
                            }
                        }
                        let view_lines: usize = line_counts.iter().sum();
                        zones.push(AlignmentZone {
                            original: false,
                            after: a.modified_range.start_line_number - 1,
                            height: view_lines as f32 * modified_line_height,
                            kind: AlignmentZoneKind::DeletedCode {
                                change: ix,
                                line_counts,
                            },
                        });
                    }
                    zones.push(AlignmentZone {
                        original: true,
                        after: a.original_range.end_line_number_exclusive - 1,
                        height: a.modified_height,
                        kind: AlignmentZoneKind::GutterDelete,
                    });
                }
                _ => {
                    let delta = a.modified_height - a.original_height;
                    if delta > 0. {
                        let after = a.original_range.end_line_number_exclusive - 1;
                        if compared.is_some_and(|m| near(m.line_range_mapping.original, after)) {
                            continue;
                        }
                        zones.push(AlignmentZone {
                            original: true,
                            after,
                            height: delta,
                            kind: AlignmentZoneKind::Fill,
                        });
                    } else if delta < 0. {
                        let after = a.modified_range.end_line_number_exclusive - 1;
                        if compared.is_some_and(|m| near(m.line_range_mapping.modified, after)) {
                            continue;
                        }
                        // Monaco `createViewZoneMarginArrow`: a deletion's
                        // padding gets a revert arrow in its margin.
                        let kind = match change {
                            Some((ix, change)) if arrows && change.modified.is_empty() => {
                                AlignmentZoneKind::DeletionPadding(ix)
                            }
                            _ => AlignmentZoneKind::Fill,
                        };
                        zones.push(AlignmentZone {
                            original: false,
                            after,
                            height: -delta,
                            kind,
                        });
                    }
                }
            }
        }
        // Monaco `alignmentsSyncedMovedText`: the compared moved block's
        // own changes line up too.
        if let Some(m) = compared {
            let synced = compute_range_alignment(
                &m.changes,
                &original_extra,
                &modified_extra,
                original_line_height,
                modified_line_height,
                &max_column,
                true,
            );
            for a in synced {
                if m.line_range_mapping
                    .original
                    .intersect(&a.original_range)
                    .is_none()
                    || m.line_range_mapping
                        .modified
                        .intersect(&a.modified_range)
                        .is_none()
                {
                    continue;
                }
                let delta = a.modified_height - a.original_height;
                if delta > 0. {
                    zones.push(AlignmentZone {
                        original: true,
                        after: a.original_range.end_line_number_exclusive - 1,
                        height: delta,
                        kind: AlignmentZoneKind::Fill,
                    });
                } else if delta < 0. {
                    zones.push(AlignmentZone {
                        original: false,
                        after: a.modified_range.end_line_number_exclusive - 1,
                        height: -delta,
                        kind: AlignmentZoneKind::Fill,
                    });
                }
            }
        }
        zones
    }

    /// Aligns the sides again (Monaco's `RunOnceScheduler(0)` on view zone,
    /// wrapping and line height changes), unless nothing would change.
    fn schedule_alignment(&mut self, cx: &mut Context<Self>) {
        if self.alignment_task.is_some() || self.diff.is_none() {
            return;
        }
        self.alignment_task = Some(cx.spawn(async move |this, cx| {
            this.update(cx, |this, cx| {
                this.alignment_task = None;
                let Some(diff) = this.diff.clone() else {
                    return;
                };
                let zones = this.alignment_zones(&diff, cx);
                if zones != this.alignment {
                    this.keep_modified_scroll_stable(cx);
                    this.apply_alignment_zones(&diff, zones, cx);
                    cx.notify();
                }
            })
            .ok();
        }));
    }

    /// Monaco `DiffEditorViewZones`: zones where one side has fewer lines
    /// (or less height), so the sides line up.
    fn apply_alignment(&mut self, diff: &LinesDiff, cx: &mut Context<Self>) {
        self.keep_modified_scroll_stable(cx);
        self.apply_move_captions(diff, cx);
        let zones = self.alignment_zones(diff, cx);
        self.apply_alignment_zones(diff, zones, cx);
    }

    /// Monaco's `StableEditorScrollState` around applying the modified
    /// editor's view zones: its first visible line stays put on screen
    /// while the zones above it change (the original side follows with
    /// the scroll sync).
    fn keep_modified_scroll_stable(&self, cx: &mut Context<Self>) {
        self.modified
            .update(cx, |editor, _| editor.keep_scroll_stable());
    }

    /// Replaces the alignment zones with `zones`.
    fn apply_alignment_zones(
        &mut self,
        diff: &LinesDiff,
        zones: Vec<AlignmentZone>,
        cx: &mut Context<Self>,
    ) {
        let (old_original, old_modified) = std::mem::take(&mut self.zones);
        clear_side(&self.original, Vec::new(), old_original, cx);
        clear_side(&self.modified, Vec::new(), old_modified, cx);
        let colors = self.modified.read(cx).colors().clone();
        let fill = colors.diff_diagonal_fill;
        let this = cx.entity().downgrade();
        let original_model = self.original.read(cx).model().clone();
        let mut ids = (Vec::new(), Vec::new());
        self.deleted_code.clear();
        for zone in &zones {
            let editor = if zone.original {
                &self.original
            } else {
                &self.modified
            };
            let height = ZoneHeight::Pixels(px(zone.height));
            let after = zone.after as u32;
            let (view, full_width, min_width): (gpui::AnyView, bool, Option<Pixels>) =
                match &zone.kind {
                    AlignmentZoneKind::Fill => {
                        (cx.new(|_| DiagonalFill { color: fill }).into(), false, None)
                    }
                    AlignmentZoneKind::DeletionPadding(ix) => {
                        let change = diff.changes[*ix].line_range_mapping();
                        let view = cx.new(|cx| DeletionPadding {
                            fill: cx.new(|_| DiagonalFill { color: fill }),
                            arrow: RevertArrow {
                                diff: this.clone(),
                                revert: GutterRevert::Block(change),
                                colors: colors.clone(),
                            },
                            editor: self.modified.downgrade(),
                        });
                        (view.into(), true, None)
                    }
                    AlignmentZoneKind::GutterDelete => {
                        let view = cx.new(|cx| GutterDeleteFill {
                            fill: cx.new(|_| DiagonalFill { color: fill }),
                            editor: self.original.downgrade(),
                        });
                        (view.into(), true, None)
                    }
                    AlignmentZoneKind::DeletedCode { change, .. } => {
                        let change = &diff.changes[*change];
                        let range = change.original;
                        let rows = range.start_line_number as u32 - 1
                            ..range.end_line_number_exclusive as u32 - 1;
                        let inner_changes = change
                            .inner_changes
                            .iter()
                            .map(|i| i.original_range.to_point_utf16_range())
                            .collect();
                        let render_indicators = self.options.render_indicators;
                        let editor = self.modified.downgrade();
                        let model = original_model.clone();
                        let mapping = change.line_range_mapping();
                        let diff_editor = this.clone();
                        let view = cx.new(|cx| {
                            DeletedCode::new(
                                DeletedCodeSource {
                                    editor,
                                    diff: diff_editor,
                                    original: model,
                                    change: mapping,
                                    rows,
                                    inner_changes,
                                    render_indicators,
                                },
                                cx,
                            )
                        });
                        let min_width = view.read(cx).min_width(cx);
                        self.deleted_code.push(view.downgrade());
                        (view.into(), true, Some(min_width))
                    }
                };
            let id = editor.update(cx, |editor, cx| {
                let id = editor.add_zone(ViewZone::new(after, height, view), full_width, cx);
                if let Some(min_width) = min_width {
                    editor.set_zone_min_width(id, min_width, cx);
                }
                id
            });
            if zone.original {
                ids.0.push(id);
            } else {
                ids.1.push(id);
            }
        }
        self.zones = ids;
        self.alignment = zones;
    }

    /// Monaco `MovedBlockOverlayWidget` on an 18px placeholder zone: "Code
    /// moved to line …" above each moved block, on both sides. Alignment
    /// counts these zones like any host zone.
    fn apply_move_captions(&mut self, diff: &LinesDiff, cx: &mut Context<Self>) {
        let (old_original, old_modified) = std::mem::take(&mut self.move_caption_zones);
        clear_side(&self.original, Vec::new(), old_original, cx);
        clear_side(&self.modified, Vec::new(), old_modified, cx);
        let colors = self.modified.read(cx).colors().clone();
        let mut zones = (Vec::new(), Vec::new());
        for (move_index, m) in self.shown_moves(diff).iter().enumerate() {
            let mapping = m.line_range_mapping;
            let with_changes = if m.changes.is_empty() {
                ""
            } else {
                " with changes"
            };
            let captions = [
                (
                    &self.original,
                    mapping.original.start_line_number - 1,
                    format!(
                        "Code moved{with_changes} to line {}-{}",
                        mapping.modified.start_line_number,
                        mapping.modified.end_line_number_exclusive - 1
                    ),
                    &mut zones.0,
                ),
                (
                    &self.modified,
                    mapping.modified.start_line_number - 1,
                    format!(
                        "Code moved{with_changes} from line {}-{}",
                        mapping.original.start_line_number,
                        mapping.original.end_line_number_exclusive - 1
                    ),
                    &mut zones.1,
                ),
            ];
            let comparing = self.move_to_compare == Some(move_index);
            for (editor, after, text, ids) in captions {
                let diff_editor = cx.entity().downgrade();
                let view = cx.new(|_| MoveCaption {
                    text: text.into(),
                    colors: colors.clone(),
                    comparing,
                    diff: diff_editor,
                    editor: editor.downgrade(),
                    index: move_index,
                });
                let id = editor.update(cx, |editor, cx| {
                    editor.add_view_zone(
                        ViewZone::new(
                            after as u32,
                            ZoneHeight::Pixels(px(MOVE_CAPTION_HEIGHT)),
                            view.into(),
                        ),
                        cx,
                    )
                });
                ids.push(id);
            }
        }
        self.move_caption_zones = zones;
    }

    /// The moved blocks to show (Monaco shows them side by side).
    fn shown_moves<'a>(&self, diff: &'a LinesDiff) -> &'a [crate::diff::MovedText] {
        if self.options.show_moves && !self.inline {
            &diff.moves
        } else {
            &[]
        }
    }

    /// Monaco's `movedOriginal`/`movedModified` block decorations: a border
    /// around each moved block, highlighted for the active one.
    fn apply_move_decorations(&mut self, cx: &mut Context<Self>) {
        let colors = self.modified.read(cx).colors().clone();
        let moves = self
            .diff
            .as_deref()
            .map(|diff| self.shown_moves(diff).to_vec())
            .unwrap_or_default();
        let block = |active: bool| DecorationOptions {
            block: Some(BlockDecoration {
                border: if active {
                    colors.diff_move_active_border
                } else {
                    colors.diff_move_border
                },
                border_width: px(2.),
                padding: [px(4.), px(0.), px(4.), px(4.)],
                ..Default::default()
            }),
            ..Default::default()
        };
        let (old_original, old_modified) = std::mem::take(&mut self.move_decorations);
        let active = self.active_move;
        let decorate = |editor: &Entity<CodeEditor>,
                        old: Vec<DecorationId>,
                        original: bool,
                        cx: &mut Context<Self>| {
            let id = editor.entity_id();
            let model = editor.read(cx).model().clone();
            model.update(cx, |model, cx| {
                let new = moves
                    .iter()
                    .enumerate()
                    .map(|(ix, m)| {
                        let lines = if original {
                            m.line_range_mapping.original
                        } else {
                            m.line_range_mapping.modified
                        };
                        (
                            Side::Lines(lines).to_offsets(model.buffer().rope()),
                            block(active == Some(ix)),
                        )
                    })
                    .collect();
                model.delta_decorations(Some(id), &old, new, cx)
            })
        };
        self.move_decorations = (
            decorate(&self.original, old_original, true, cx),
            decorate(&self.modified, old_modified, false, cx),
        );
    }

    /// Monaco `setActiveMovedTextFromCursor`: the moved block under the
    /// cursor of the editor whose cursor moved last. Moving off the
    /// compared block stops comparing it.
    fn update_active_move(&mut self, original: bool, cx: &mut Context<Self>) {
        let comparing = self.move_to_compare;
        let active = self.active_move;
        self.sync_active_move(original, cx);
        if self.move_to_compare != comparing {
            self.apply_diff(cx);
            self.sync_scroll(false, cx);
            cx.notify();
        } else if self.active_move != active {
            self.apply_move_decorations(cx);
            cx.notify();
        }
    }

    /// [`Self::update_active_move`] without applying: sets the active move
    /// and stops comparing a move the cursor isn't in (Monaco
    /// `movedText !== movedTextToCompare`).
    fn sync_active_move(&mut self, original: bool, cx: &App) {
        self.last_cursor_original = original;
        let Some(diff) = self.diff.clone() else {
            return;
        };
        let editor = if original {
            &self.original
        } else {
            &self.modified
        };
        let line = editor.read(cx).selections()[0].head.row as usize + 1;
        let active = self.shown_moves(&diff).iter().position(|m| {
            if original {
                m.line_range_mapping.original.contains(line)
            } else {
                m.line_range_mapping.modified.contains(line)
            }
        });
        if active != self.move_to_compare {
            self.move_to_compare = None;
        }
        self.active_move = active;
    }

    /// The compared move's modified lines, to find it in a new diff.
    fn compared_move_range(&self) -> Option<LineRange> {
        let diff = self.diff.as_deref()?;
        let m = self.shown_moves(diff).get(self.move_to_compare?)?;
        Some(m.line_range_mapping.modified)
    }

    /// Monaco `moves.find(m => m.lineRangeMapping.modified.intersect(...))`:
    /// the move in `diff` where the compared one (`compared`) is now.
    fn find_compared_move(diff: &LinesDiff, compared: Option<LineRange>) -> Option<usize> {
        let compared = compared?;
        diff.moves
            .iter()
            .position(|m| m.line_range_mapping.modified.intersect(&compared).is_some())
    }

    /// The width the sides share (Monaco's `SashLayout.dimensions.width`):
    /// the diff editor's, less the overview ruler.
    fn sides_width(&self) -> Pixels {
        let overview = if self.options.render_overview_ruler {
            px(DIFF_OVERVIEW_WIDTH)
        } else {
            px(0.)
        };
        (self.bounds.get().size.width - overview).max(px(0.))
    }

    /// Monaco `SashLayout.sashLeft`: the sash's x from the diff editor's
    /// left, at least 100px from either side.
    fn sash_left(&self) -> Pixels {
        compute_sash_left(
            self.ratio,
            self.options.split_view_default_ratio,
            self.options.enable_split_view_resizing,
            self.sides_width(),
        )
    }

    /// The pointer over the whole window during a drag (Monaco's pointer
    /// capture): the sash's resize cursor, the overview slider's arrow, or
    /// a hidden-lines bar edge's `n-resize`/`s-resize`/`ns-resize` (whether
    /// dragging up and down still does anything).
    pub(crate) fn captured_cursor(&self) -> Option<gpui::CursorStyle> {
        if self.sash_drag.is_some() {
            return Some(SASH_CURSOR);
        }
        if self.overview_drag.is_some() {
            return Some(gpui::CursorStyle::Arrow);
        }
        let drag = self.unchanged_drag?;
        let region = self.unchanged_regions().get(drag.index)?;
        let fully_revealed = region.hidden_modified_range().is_empty();
        let (up, down) = if drag.top {
            (region.visible_top > 0, !fully_revealed)
        } else {
            (!fully_revealed, region.visible_bottom > 0)
        };
        Some(unchanged_edge_cursor(up, down))
    }

    /// [`Self::captured_cursor`], for tests: GPUI's test platform doesn't
    /// keep the cursor.
    #[doc(hidden)]
    pub fn captured_cursor_for_tests(&self) -> Option<gpui::CursorStyle> {
        self.captured_cursor()
    }

    /// Monaco `DiffEditorSash.onDidChange`: the sash moves by how far the
    /// mouse moved since the drag started.
    fn drag_sash(&mut self, event: &gpui::MouseMoveEvent, cx: &mut Context<Self>) {
        let Some((start_x, start_left)) = self.sash_drag else {
            return;
        };
        if event.pressed_button != Some(MouseButton::Left) {
            self.sash_drag = None;
            cx.notify();
            return;
        }
        let width = self.sides_width();
        if width > px(0.) {
            let left = start_left + (event.position.x - start_x);
            self.ratio = left / width;
            cx.notify();
        }
    }

    /// Monaco `DiffEditorSash.onDidReset` (a double click): back to
    /// `split_view_default_ratio`.
    pub fn reset_sash(&mut self, cx: &mut Context<Self>) {
        self.ratio = self.options.split_view_default_ratio;
        cx.notify();
    }

    /// The sash's x from the diff editor's left, for tests.
    #[doc(hidden)]
    pub fn sash_left_for_tests(&self) -> Pixels {
        self.sash_left()
    }

    /// Whether sash `id` (`diff-sash` or `diff-gutter-sash`) shows
    /// `sash.hoverBorder` for the mouse resting on it, for tests.
    #[doc(hidden)]
    pub fn sash_hover_for_tests(&self, id: &'static str) -> bool {
        self.sash_hover.shows(id)
    }

    /// Drags the sash from window x `from` to `to`, for tests.
    #[doc(hidden)]
    pub fn drag_sash_for_tests(&mut self, from: Pixels, to: Pixels, cx: &mut Context<Self>) {
        self.sash_drag = Some((from, self.sash_left()));
        let event = gpui::MouseMoveEvent {
            position: point(to, px(0.)),
            pressed_button: Some(MouseButton::Left),
            modifiers: Default::default(),
        };
        self.drag_sash(&event, cx);
        self.sash_drag = None;
    }

    /// Monaco `gutterFeature.ts`'s `delegateScrollFromMouseWheelEvent`: the
    /// wheel over the gutter (or the moved-lines column) scrolls the
    /// modified editor, which the original follows.
    fn delegate_wheel(&mut self, event: &gpui::ScrollWheelEvent, cx: &mut Context<Self>) {
        cx.stop_propagation();
        self.modified
            .update(cx, |editor, cx| editor.scroll_by_wheel(event, cx));
    }

    /// Monaco `MovedBlocksLinesFeature`: a column between the sides (its
    /// width, Monaco's `lineAreaWidth`) and, over it and the editors, a line
    /// from each moved block to where it went: from the original editor's
    /// vertical scrollbar into the modified editor's gutter, ending in an
    /// arrow on a background rectangle at the moved code.
    fn render_move_lines(&self, cx: &App) -> Option<(Pixels, gpui::AnyElement)> {
        let diff = self.diff.as_deref()?;
        let moves = self.shown_moves(diff);
        if moves.is_empty() {
            return None;
        }
        let original = self.original.read(cx);
        let modified = self.modified.read(cx);
        let middle = |editor: &CodeEditor, lines: LineRange| {
            let (top, bottom) = editor.rows_y_span(lines.to_rows())?;
            Some((top + bottom) / 2.)
        };
        let mut lines: Vec<(Pixels, Pixels, bool)> = moves
            .iter()
            .enumerate()
            .filter_map(|(ix, m)| {
                Some((
                    middle(original, m.line_range_mapping.original)?,
                    middle(modified, m.line_range_mapping.modified)?,
                    self.active_move == Some(ix),
                ))
            })
            .collect();
        // Monaco sorts downward moves first, then by where they start.
        lines.sort_by(|a, b| {
            let down = |l: &(Pixels, Pixels, bool)| l.0 > l.1;
            let key = |l: &(Pixels, Pixels, bool)| if down(l) { l.0 } else { -l.1 };
            down(a).cmp(&down(b)).then(
                key(a)
                    .partial_cmp(&key(b))
                    .unwrap_or(std::cmp::Ordering::Equal),
            )
        });
        let tracks = move_line_tracks(
            &lines
                .iter()
                .map(|&(from, to, _)| (from.min(to), from.max(to)))
                .collect::<Vec<_>>(),
        );
        let track_count = tracks.iter().copied().max().map_or(1, |t| t + 1);
        let padding = 10.;
        let line_area_width = px((track_count - 1) as f32 * 10. + padding * 2.);
        let (Some(original_layout), Some(modified_layout)) =
            (original.layout.as_ref(), modified.layout.as_ref())
        else {
            return Some((line_area_width, div().into_any_element()));
        };
        let root = self.bounds.get();
        let scrollbar = original_layout.layout_info.vertical_scrollbar_width;
        // Monaco: from `width - verticalScrollbarWidth` of the original
        // editor, `verticalScrollbarWidth + contentLeft - movedCodeBlockPadding
        // + lineAreaWidth` wide.
        let left = original_layout.bounds.right() - scrollbar - root.left();
        let width = scrollbar + line_area_width + modified_layout.layout_info.content_left - px(4.);
        let rect_width = modified_layout.layout_info.glyph_margin_width
            + modified_layout.layout_info.line_numbers_width;
        let colors = modified.colors().clone();
        let overlay = canvas(
            |_, _, _| {},
            move |bounds, _, window, _| {
                window.with_content_mask(Some(gpui::ContentMask { bounds }), |window| {
                    for ((from, to, active), track) in lines.iter().zip(&tracks) {
                        let color = if *active {
                            colors.diff_move_active_border
                        } else {
                            colors.diff_move_border
                        };
                        let x = |x: Pixels| bounds.left() + x;
                        let y = |y: Pixels| bounds.top() + y;
                        let vertical = scrollbar + px(padding + *track as f32 * 10.);
                        let right = bounds.size.width;
                        // Monaco's `arrow-rectangle` hides the line numbers
                        // behind the arrow.
                        window.paint_quad(gpui::fill(
                            Bounds::new(
                                point(x(right - rect_width), y(*to) - px(9.)),
                                gpui::size(rect_width, px(18.)),
                            ),
                            colors.background,
                        ));
                        let mut path = PathBuilder::stroke(px(2.));
                        path.move_to(point(x(px(0.)), y(*from)));
                        path.line_to(point(x(vertical), y(*from)));
                        path.line_to(point(x(vertical), y(*to)));
                        path.line_to(point(x(right - px(MOVE_ARROW_WIDTH)), y(*to)));
                        if let Ok(path) = path.build() {
                            window.paint_path(path, color);
                        }
                        let half = px(MOVE_ARROW_WIDTH / 2.);
                        let mut arrow = PathBuilder::fill();
                        arrow.move_to(point(x(right - px(MOVE_ARROW_WIDTH)), y(*to) - half));
                        arrow.line_to(point(x(right), y(*to)));
                        arrow.line_to(point(x(right - px(MOVE_ARROW_WIDTH)), y(*to) + half));
                        arrow.close();
                        if let Ok(arrow) = arrow.build() {
                            window.paint_path(arrow, color);
                        }
                    }
                });
            },
        )
        .absolute()
        .top_0()
        .left(left)
        .w(width)
        .h_full()
        .into_any_element();
        Some((line_area_width, overlay))
    }

    /// The diff overview's marks (Monaco `createZones`): each side's
    /// changes as fractions of its scroll height, top and bottom.
    #[allow(clippy::type_complexity)]
    fn overview_zones(&self, cx: &App) -> (Vec<(f32, f32)>, Vec<(f32, f32)>) {
        let zones = |editor: &CodeEditor, original: bool| -> Vec<(f32, f32)> {
            let Some(diff) = self.diff.as_deref() else {
                return Vec::new();
            };
            let Some(scroll_height) = editor.scroll_height() else {
                return Vec::new();
            };
            let scroll_top = editor.scroll_position().y;
            diff.changes
                .iter()
                .map(|c| if original { c.original } else { c.modified })
                .filter(|r| !r.is_empty())
                .filter_map(|r| {
                    let (top, bottom) = editor.rows_y_span(r.to_rows())?;
                    Some((
                        f32::from(top + scroll_top) / f32::from(scroll_height),
                        f32::from(bottom + scroll_top) / f32::from(scroll_height),
                    ))
                })
                .collect()
        };
        (
            zones(self.original.read(cx), true),
            zones(self.modified.read(cx), false),
        )
    }

    /// Monaco `OverviewRulerFeature`: a 30px column at the right with the
    /// original's changes in its left half and the modified's in its right
    /// half, and the modified editor's viewport (`diffViewport`) as a
    /// slider. Pointer down and the wheel go to the modified editor.
    fn render_overview_ruler(&self, cx: &mut Context<Self>) -> gpui::AnyElement {
        let modified = self.modified.read(cx);
        let colors = modified.colors().clone();
        let light = modified.theme().kind == crate::theme::ThemeKind::Light;
        let (insert_color, remove_color) = diff_overview_colors(modified.theme());
        let (original_zones, modified_zones) = self.overview_zones(cx);
        let slider = modified.layout.as_ref().and_then(|layout| {
            let scroll_height = modified.scroll_height()?;
            Some(slider_geometry(
                layout.bounds.size.height,
                scroll_height,
                modified.scroll_position().y,
            ))
        });
        let dragging = self.overview_drag.is_some();
        // Monaco's `.diffViewport:hover` and `:active`.
        let hovered = slider
            .zip(self.overview_mouse_y)
            .is_some_and(|((top, size), y)| y >= top && y < top + size);
        let slider_color = if dragging {
            colors.scrollbar_slider_active
        } else if hovered {
            colors.scrollbar_slider_hover
        } else {
            colors.scrollbar_slider
        };
        let background = if light {
            gpui::black().opacity(0.03)
        } else {
            gpui::white().opacity(0.01)
        };
        let half = DIFF_OVERVIEW_WIDTH / 2.;
        let paint = move |bounds: Bounds<Pixels>, window: &mut Window| {
            use crate::view::overview_ruler::{merge_color_zones, resolve_color_zones};
            // Each side is a Monaco `OverviewRuler`: `OverviewZoneManager`
            // zones (fractions of the scroll height here) at least 4px tall,
            // in device pixels, with overlapping ones merged
            // (`_renderOneLane`).
            let pixel_ratio = window.scale_factor();
            let device = |value: i32| px(value as f32 / pixel_ratio);
            for (zones, left, color) in [
                (&original_zones, 0., remove_color),
                (&modified_zones, half, insert_color),
            ] {
                let resolved = resolve_color_zones(
                    zones.iter().map(|(top, bottom)| (*top, *bottom, 1)),
                    f32::from(bounds.size.height),
                    1.,
                    pixel_ratio,
                );
                for (_, from, to) in merge_color_zones(&resolved) {
                    window.paint_quad(gpui::fill(
                        Bounds::new(
                            point(bounds.left() + px(left), bounds.top() + device(from)),
                            gpui::size(px(half), device(to - from)),
                        ),
                        color,
                    ));
                }
            }
            if let Some((top, size)) = slider {
                window.paint_quad(gpui::fill(
                    Bounds::new(
                        point(bounds.left(), bounds.top() + top),
                        gpui::size(bounds.size.width, size),
                    ),
                    slider_color,
                ));
            }
        };
        div()
            .id("diff-overview")
            .debug_selector(|| "diff-overview".into())
            .h_full()
            .w(px(DIFF_OVERVIEW_WIDTH))
            .flex_none()
            .bg(background)
            .child(
                canvas(
                    |_, _, _| {},
                    move |bounds, _, window, _| paint(bounds, window),
                )
                .size_full(),
            )
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, event: &gpui::MouseDownEvent, _, cx| {
                    cx.stop_propagation();
                    this.start_overview_drag(event.position.y, cx);
                }),
            )
            .on_mouse_move(cx.listener(|this, event: &gpui::MouseMoveEvent, _, cx| {
                let y = Some(event.position.y - this.bounds.get().top());
                if this.overview_mouse_y != y {
                    this.overview_mouse_y = y;
                    cx.notify();
                }
            }))
            .on_hover(cx.listener(|this, hovered: &bool, _, cx| {
                if !*hovered && this.overview_mouse_y.take().is_some() {
                    cx.notify();
                }
            }))
            .on_scroll_wheel(cx.listener(|this, event, _, cx| this.delegate_wheel(event, cx)))
            .into_any_element()
    }

    /// Monaco `delegateVerticalScrollbarPointerDown` from the diff
    /// overview: a press on the slider grabs it; elsewhere the slider jumps
    /// to center on the press first.
    fn start_overview_drag(&mut self, y: Pixels, cx: &mut Context<Self>) {
        let modified = self.modified.read(cx);
        let (Some(layout), Some(scroll_height)) =
            (modified.layout.as_ref(), modified.scroll_height())
        else {
            return;
        };
        let top = self.bounds.get().top();
        let (slider_top, size) = slider_geometry(
            layout.bounds.size.height,
            scroll_height,
            modified.scroll_position().y,
        );
        let offset = y - top - slider_top;
        let grab = if offset >= px(0.) && offset <= size {
            offset
        } else {
            size / 2.
        };
        self.overview_drag = Some(grab);
        self.drag_overview(y, cx);
        cx.notify();
    }

    /// Scrolls the modified editor so the slider's grabbed point is at
    /// window y `y`.
    fn drag_overview(&mut self, y: Pixels, cx: &mut Context<Self>) {
        let Some(grab) = self.overview_drag else {
            return;
        };
        let modified = self.modified.read(cx);
        let (Some(layout), Some(scroll_height)) =
            (modified.layout.as_ref(), modified.scroll_height())
        else {
            return;
        };
        let visible = layout.bounds.size.height;
        let (_, size) = slider_geometry(visible, scroll_height, px(0.));
        let track = visible - size;
        if track <= px(0.) {
            return;
        }
        let slider_top = (y - self.bounds.get().top() - grab).clamp(px(0.), track);
        let scroll_top = (scroll_height - visible) * (slider_top / track);
        self.modified.update(cx, |editor, cx| {
            let mut position = editor.scroll_position();
            position.y = scroll_top;
            editor.set_scroll_position(position, cx);
        });
    }

    /// Monaco `useTrueInlineDiffRendering`: the inline view with
    /// `use_true_inline_view`.
    fn uses_true_inline_view(&self) -> bool {
        self.inline && self.options.use_true_inline_view
    }

    /// Monaco `updateUnchangedRegions`: the hideable regions of a new diff,
    /// keeping what the user revealed.
    fn update_unchanged_regions(&mut self, diff: &LinesDiff, cx: &mut Context<Self>) {
        let settings = self.options.hide_unchanged_regions;
        let original_count = self.original.read(cx).model().read(cx).line_count() as usize;
        let modified_count = self.modified.read(cx).model().read(cx).line_count() as usize;
        self.unchanged = settings.enabled.then(|| {
            let regions = UnchangedRegion::from_diffs(
                &diff.changes,
                original_count,
                modified_count,
                settings.minimum_line_count,
                settings.context_line_count,
            );
            let previous = self.tracked_unchanged_regions(cx);
            unchanged::transfer_state(regions, previous.as_deref(), original_count, modified_count)
        });
        self.apply_unchanged(cx);
    }

    /// The regions with their ranges moved through edits since they were
    /// shown (Monaco tracks them with decorations), without those that
    /// edits made overlap the one before. `None` before the first diff.
    fn tracked_unchanged_regions(&self, cx: &App) -> Option<Vec<UnchangedRegion>> {
        let rows = |editor: &Entity<CodeEditor>, id: DecorationId| {
            let model = editor.read(cx).model().read(cx);
            let range = model.decoration(id)?.range.clone();
            let rope = model.buffer().rope();
            Some(
                rope.offset_to_point(range.start).row as usize + 1
                    ..rope.offset_to_point(range.end).row as usize + 2,
            )
        };
        let regions = self
            .unchanged
            .as_ref()?
            .iter()
            .zip(&self.unchanged_decorations.0)
            .zip(&self.unchanged_decorations.1)
            .filter_map(|((r, &original), &modified)| {
                let original = rows(&self.original, original)?;
                let modified = rows(&self.modified, modified)?;
                let length = original.len();
                let top = r.visible_top.min(length);
                Some(UnchangedRegion::new(
                    original.start,
                    modified.start,
                    length,
                    top,
                    r.visible_bottom.min(length - top),
                ))
            })
            .collect();
        Some(unchanged::drop_overlapping(regions))
    }

    /// The unchanged regions, none before the first diff.
    fn unchanged_regions(&self) -> &[UnchangedRegion] {
        self.unchanged.as_deref().unwrap_or_default()
    }

    fn unchanged_regions_mut(&mut self) -> &mut [UnchangedRegion] {
        self.unchanged.as_deref_mut().unwrap_or_default()
    }

    /// The unchanged regions the view shows: none when the only one covers
    /// the whole modified model (Monaco `HideUnchangedRegionsFeature`'s
    /// `unchangedRegions`).
    fn shown_unchanged(&self) -> &[UnchangedRegion] {
        if self.unchanged_whole_file {
            &[]
        } else {
            self.unchanged_regions()
        }
    }

    /// Hides the regions' hidden lines and shows a bar in their place.
    fn apply_unchanged(&mut self, cx: &mut Context<Self>) {
        // Monaco's "collapse touching unchanged ranges", except while a bar
        // is dragged.
        if self.unchanged_drag.is_none()
            && let Some(merged) = unchanged::merge_touching(self.unchanged_regions())
        {
            self.unchanged = Some(merged);
        }
        let modified_count = self.modified.read(cx).model().read(cx).line_count() as usize;
        self.unchanged_whole_file = matches!(
            self.unchanged_regions(),
            [r] if r.modified_line_number == 1 && r.line_count == modified_count
        );
        let colors = self.modified.read(cx).colors().clone();
        // The tracking decorations, which also tint the shown regions' lines
        // (Monaco `diff-unchanged-lines`).
        let line = DecorationOptions {
            background: (!self.unchanged_whole_file)
                .then_some(colors.diff_unchanged_code_background),
            is_whole_line: true,
            ..Default::default()
        };
        let (old_original, old_modified) = std::mem::take(&mut self.unchanged_decorations);
        let decorate = |editor: &Entity<CodeEditor>,
                        old: Vec<DecorationId>,
                        ranges: Vec<LineRange>,
                        cx: &mut Context<Self>| {
            let id = editor.entity_id();
            let model = editor.read(cx).model().clone();
            model.update(cx, |model, cx| {
                let new = ranges
                    .into_iter()
                    .map(|r| {
                        (
                            Side::Lines(r).to_offsets(model.buffer().rope()),
                            line.clone(),
                        )
                    })
                    .collect();
                model.delta_decorations(Some(id), &old, new, cx)
            })
        };
        let original_ranges = self
            .unchanged_regions()
            .iter()
            .map(|r| r.original_unchanged_range())
            .collect();
        let modified_ranges = self
            .unchanged_regions()
            .iter()
            .map(|r| r.modified_unchanged_range())
            .collect();
        self.unchanged_decorations = (
            decorate(&self.original, old_original, original_ranges, cx),
            decorate(&self.modified, old_modified, modified_ranges, cx),
        );
        let original_hidden = self
            .shown_unchanged()
            .iter()
            .map(|r| r.hidden_original_range().to_rows())
            .filter(|r| !r.is_empty())
            .collect();
        let modified_hidden = self
            .shown_unchanged()
            .iter()
            .map(|r| r.hidden_modified_range().to_rows())
            .filter(|r| !r.is_empty())
            .collect();
        self.original.update(cx, |editor, cx| {
            editor.set_hidden_areas_from("diffEditor", original_hidden, cx)
        });
        self.modified.update(cx, |editor, cx| {
            editor.set_hidden_areas_from("diffEditor", modified_hidden, cx)
        });

        let (old_original, old_modified) = std::mem::take(&mut self.unchanged_zones);
        clear_side(&self.original, Vec::new(), old_original, cx);
        clear_side(&self.modified, Vec::new(), old_modified, cx);
        let this = cx.entity().downgrade();
        let mut zones = (Vec::new(), Vec::new());
        let compact = self.options.compact_mode;
        for (index, region) in self.shown_unchanged().iter().enumerate() {
            let Some(height) = self.unchanged_bar_height(index) else {
                continue;
            };
            let sides = [
                (&self.original, region.hidden_original_range(), &mut zones.0),
                (&self.modified, region.hidden_modified_range(), &mut zones.1),
            ];
            let breadcrumbs =
                breadcrumb_items(&self.breadcrumb_symbols, region.hidden_modified_range());
            for (editor, hidden, ids) in sides {
                let original = editor == &self.original;
                let bar = cx.new(|_| UnchangedBar {
                    diff: this.clone(),
                    editor: editor.downgrade(),
                    original,
                    // Monaco's `_hide = !sideBySide` on the original side.
                    hide: original && self.inline,
                    index,
                    hidden_lines: hidden.len(),
                    visible_top: region.visible_top > 0,
                    visible_bottom: region.visible_bottom > 0,
                    breadcrumbs: breadcrumbs.clone(),
                    compact,
                });
                // Monaco `PlaceholderViewZone`'s `showInHiddenAreas`: the
                // line after the bar is the first hidden one.
                let mut zone = ViewZone::new(
                    hidden.start_line_number as u32 - 1,
                    ZoneHeight::Pixels(px(height)),
                    bar.into(),
                );
                zone.show_in_hidden_areas = true;
                let id = editor.update(cx, |editor, cx| editor.add_zone(zone, true, cx));
                ids.push(id);
            }
        }
        self.unchanged_zones = zones;
        self.apply_fold_glyphs(cx);
        cx.notify();
    }

    /// Monaco `shouldHideControls`: region `index` is fully shown and not
    /// being dragged, so it has no bar.
    fn hides_unchanged_controls(&self, index: usize) -> bool {
        self.unchanged_regions()
            .get(index)
            .is_none_or(|r| r.should_hide_controls())
            && self.unchanged_drag.is_none_or(|d| d.index != index)
    }

    /// The height of region `index`'s bar, if it has one.
    fn unchanged_bar_height(&self, index: usize) -> Option<f32> {
        if self.hides_unchanged_controls(index) {
            return None;
        }
        if !self.options.compact_mode {
            return Some(UNCHANGED_BAR_HEIGHT);
        }
        // Monaco `compactMode`: no separators at the start and end.
        (index != 0 && index + 1 != self.shown_unchanged().len())
            .then_some(COMPACT_UNCHANGED_BAR_HEIGHT)
    }

    /// Changes region `index` and shows the result.
    fn update_unchanged_region(
        &mut self,
        index: usize,
        change: impl FnOnce(&mut UnchangedRegion),
        cx: &mut Context<Self>,
    ) {
        if let Some(region) = self.unchanged_regions_mut().get_mut(index) {
            change(region);
            self.apply_unchanged(cx);
        }
    }

    /// Monaco's mouse down on a bar's top or bottom edge (in the
    /// `original` editor's bar or the modified's): a drag starts at window
    /// y `y`.
    fn start_unchanged_drag(
        &mut self,
        index: usize,
        top: bool,
        original: bool,
        y: Pixels,
        cx: &mut Context<Self>,
    ) {
        let Some(region) = self.unchanged_regions().get(index) else {
            return;
        };
        let start = if top {
            region.visible_top
        } else {
            region.visible_bottom
        };
        self.unchanged_drag = Some(UnchangedDrag {
            index,
            top,
            original,
            start_y: y,
            start,
            moved: false,
        });
        cx.notify();
    }

    /// Monaco's drag of a bar's edge: each line height moved shows or hides
    /// a line.
    fn drag_unchanged(&mut self, y: Pixels, cx: &mut Context<Self>) {
        let Some(drag) = self.unchanged_drag.as_mut() else {
            return;
        };
        let delta = y - drag.start_y;
        drag.moved |= delta.abs() > px(2.);
        let line_height = f32::from(self.modified.read(cx).line_height());
        let lines = (f32::from(delta) / line_height).round() as isize;
        let (index, top, original, start) =
            (drag.index, drag.top, drag.original, drag.start as isize);
        let Some(region) = self.unchanged_regions_mut().get_mut(index) else {
            return;
        };
        if top {
            let value = (start + lines).clamp(0, region.max_visible_top() as isize) as usize;
            if std::mem::replace(&mut region.visible_top, value) != value {
                self.apply_unchanged(cx);
            }
        } else {
            let value = (start - lines).clamp(0, region.max_visible_bottom() as isize) as usize;
            if region.visible_bottom != value {
                self.with_lower_scroll_anchor(index, original, |r| r.visible_bottom = value, cx);
            }
        }
    }

    /// Monaco `_runWithLowerScrollAnchor`: changes region `index` keeping
    /// the line after it in place on screen in the `original` editor (else
    /// the modified), so a dragged bottom edge follows the mouse.
    fn with_lower_scroll_anchor(
        &mut self,
        index: usize,
        original: bool,
        change: impl FnOnce(&mut UnchangedRegion),
        cx: &mut Context<Self>,
    ) {
        let hidden = |r: &UnchangedRegion| {
            if original {
                r.hidden_original_range().to_rows()
            } else {
                r.hidden_modified_range().to_rows()
            }
        };
        let bar_before = self.unchanged_bar_height(index);
        let Some(region) = self.unchanged_regions_mut().get_mut(index) else {
            return;
        };
        let before = hidden(region);
        change(region);
        let after = hidden(region);
        // A click that shows the rest removes the bar; a drag keeps it.
        let bar_after = self.unchanged_bar_height(index);
        // The lines shown or hidden and the bar are above the anchor, the
        // region's end. Monaco measures the anchor's top before and after;
        // hidden lines only lay out on the next frame, so this adds up the
        // view lines (wrapped parts included) they show. As in Monaco, the
        // alignment that pads the other side's wrapped lines comes after,
        // keeping the first visible line.
        let editor = if original {
            &self.original
        } else {
            &self.modified
        };
        let editor = editor.read(cx);
        let view_lines = &editor.view_lines;
        // The rows of `a` not in `b`.
        let minus = |a: &Range<u32>, b: &Range<u32>| {
            view_lines.unhidden_line_count(a.start..a.end.min(b.start))
                + view_lines.unhidden_line_count(a.start.max(b.end)..a.end)
        };
        let shown = minus(&before, &after) as f32 - minus(&after, &before) as f32;
        let line_height = editor.line_height();
        let delta =
            line_height * shown + px(bar_after.unwrap_or(0.)) - px(bar_before.unwrap_or(0.));
        for editor in [&self.original, &self.modified] {
            editor.update(cx, |editor, cx| {
                // Where a smooth scroll is going moves with the lines.
                let mut position = editor.future_scroll_position();
                position.y = (position.y + delta).max(px(0.));
                editor.set_scroll_position_unclamped(position, cx);
            });
        }
        self.apply_unchanged(cx);
    }

    /// Monaco's mouse up: a click without dragging shows
    /// `reveal_line_count` more lines. A region the drag fully showed loses
    /// its bar now, and touching regions merge.
    fn end_unchanged_drag(&mut self, cx: &mut Context<Self>) {
        let Some(drag) = self.unchanged_drag.take() else {
            return;
        };
        let count = self.options.hide_unchanged_regions.reveal_line_count;
        if drag.moved {
            self.apply_unchanged(cx);
        } else if drag.top {
            self.update_unchanged_region(drag.index, |r| r.show_more_above(count), cx);
        } else {
            self.with_lower_scroll_anchor(
                drag.index,
                drag.original,
                |r| r.show_more_below(count),
                cx,
            );
        }
        cx.notify();
    }

    /// Monaco `ensureModifiedLineIsVisible`/`ensureOriginalLineIsVisible`
    /// for each selection's ends: the cursor never sits in hidden lines.
    fn ensure_selections_visible(&mut self, original: bool, cx: &mut Context<Self>) {
        if self.unchanged_regions().is_empty() {
            return;
        }
        let editor = if original {
            &self.original
        } else {
            &self.modified
        };
        let lines: Vec<usize> = editor
            .read(cx)
            .selections()
            .iter()
            .flat_map(|s| [s.start().row as usize + 1, s.end().row as usize + 1])
            .collect();
        let mut changed = false;
        for line in lines {
            for region in self.unchanged_regions_mut() {
                let hidden = if original {
                    region.hidden_original_range()
                } else {
                    region.hidden_modified_range()
                };
                if hidden.contains(line) {
                    if original {
                        region.show_original_line(line, RevealPreference::FromCloserSide);
                    } else {
                        region.show_modified_line(line, RevealPreference::FromCloserSide);
                    }
                    changed = true;
                    break;
                }
            }
        }
        if changed {
            self.apply_unchanged(cx);
        }
    }

    /// Hides every unchanged region again (Monaco
    /// `collapseAllUnchangedRegions`).
    pub fn collapse_all_unchanged_regions(&mut self, cx: &mut Context<Self>) {
        self.unchanged_regions_mut()
            .iter_mut()
            .for_each(UnchangedRegion::collapse_all);
        self.apply_unchanged(cx);
    }

    /// Shows every unchanged region (Monaco `showAllUnchangedRegions`).
    pub fn show_all_unchanged_regions(&mut self, cx: &mut Context<Self>) {
        self.unchanged_regions_mut()
            .iter_mut()
            .for_each(UnchangedRegion::show_all);
        self.apply_unchanged(cx);
    }

    /// The modified editor's hidden unchanged lines (1-based, end
    /// exclusive), for tests.
    #[doc(hidden)]
    pub fn hidden_modified_ranges_for_tests(&self) -> Vec<Range<usize>> {
        self.shown_unchanged()
            .iter()
            .map(|r| r.hidden_modified_range())
            .filter(|r| !r.is_empty())
            .map(|r| r.start_line_number..r.end_line_number_exclusive)
            .collect()
    }

    /// Drags region `index`'s top (or bottom) bar edge from window y
    /// `from` to `to`, for tests.
    #[doc(hidden)]
    pub fn drag_unchanged_for_tests(
        &mut self,
        index: usize,
        top: bool,
        from: Pixels,
        to: Pixels,
        cx: &mut Context<Self>,
    ) {
        self.start_unchanged_drag(index, top, false, from, cx);
        self.drag_unchanged(to, cx);
        self.end_unchanged_drag(cx);
    }

    /// Reveals more of region `index` like a click on its bar's top
    /// (`above`) or bottom edge, for tests.
    #[doc(hidden)]
    pub fn reveal_unchanged_for_tests(
        &mut self,
        index: usize,
        above: bool,
        cx: &mut Context<Self>,
    ) {
        let count = self.options.hide_unchanged_regions.reveal_line_count;
        self.update_unchanged_region(
            index,
            |r| {
                if above {
                    r.show_more_above(count)
                } else {
                    r.show_more_below(count)
                }
            },
            cx,
        );
    }

    /// Monaco `shouldRenderOldRevertArrows`: margin arrows instead of the
    /// gutter menu, side by side, when the modified side is editable.
    fn shows_margin_revert_arrows(&self, cx: &App) -> bool {
        self.options.render_margin_revert_icon
            && !self.options.render_gutter_menu
            && !self.inline
            && !self.modified.read(cx).options().read_only
            // Monaco hides them while comparing a moved block.
            && self.move_to_compare.is_none()
    }

    /// Monaco `RevertButtonsFeature`: an arrow in the modified editor's
    /// glyph margin beside each change, or one reverting the selection.
    fn apply_revert_arrows(&mut self, cx: &mut Context<Self>) {
        let old = std::mem::take(&mut self.revert_arrows);
        self.modified.update(cx, |editor, cx| {
            for id in old {
                editor.remove_widget(id, cx);
            }
        });
        if !self.shows_margin_revert_arrows(cx) {
            return;
        }
        let colors = self.modified.read(cx).colors().clone();
        let this = cx.entity().downgrade();
        let selection_line = self
            .modified
            .read(cx)
            .selections()
            .last()
            .map(|s| s.head.row);
        let Some(diff) = self.diff.clone() else {
            return;
        };
        // Monaco `RevertButtonsFeature`: one for the selected changes, and
        // one for each other change with modified lines (deletions get
        // theirs in their padding's margin).
        let (selected, selected_changes) = self.selected_changes(cx);
        let mut arrows: Vec<(u32, GutterRevert)> = Vec::new();
        if !selected.is_empty()
            && let Some(line) = selection_line
        {
            arrows.push((line, GutterRevert::Selection(selected)));
        }
        for (ix, change) in diff.changes.iter().enumerate() {
            if selected_changes.contains(&ix) || change.modified.is_empty() {
                continue;
            }
            arrows.push((
                change.modified.to_rows().start,
                GutterRevert::Block(change.line_range_mapping()),
            ));
        }
        let mut ids = Vec::new();
        for (row, revert) in arrows {
            let view = cx.new(|_| RevertArrow {
                diff: this.clone(),
                revert,
                colors: colors.clone(),
            });
            ids.push(self.modified.update(cx, |editor, cx| {
                editor
                    .add_glyph_margin_widget(crate::widgets::GlyphMarginWidget::new(row, view), cx)
            }));
        }
        self.revert_arrows = ids;
    }

    /// Whether the diff is for the models' current text (Monaco
    /// `isDiffUpToDate`).
    fn is_diff_up_to_date(&self, cx: &App) -> bool {
        let original = self.original.read(cx).model().read(cx).buffer().version();
        let modified = self.modified.read(cx).model().read(cx).buffer().version();
        self.diff.is_some() && self.diff_versions == Some((original, modified))
    }

    /// Replaces modified ranges with original text (pairs of original and
    /// modified ranges), as one undo step.
    fn revert_edits(
        &mut self,
        edits: Vec<(Range<PointUtf16>, Range<PointUtf16>)>,
        cx: &mut Context<Self>,
    ) {
        if !self.is_diff_up_to_date(cx) || edits.is_empty() {
            return;
        }
        let original = self
            .original
            .read(cx)
            .model()
            .read(cx)
            .buffer()
            .rope()
            .clone();
        let edits = edits
            .into_iter()
            .map(|(original_range, modified_range)| {
                let range = original.point_utf16_to_offset(original_range.start)
                    ..original.point_utf16_to_offset(original_range.end);
                (modified_range, original.text_for_range(range))
            })
            .collect();
        self.modified.update(cx, |editor, cx| {
            editor.push_undo_stop(cx);
            editor.execute_edits_from("diffEditor", edits, None, cx);
            editor.push_undo_stop(cx);
        });
    }

    /// Monaco `revert`: replaces a change's modified lines with its
    /// original lines.
    pub fn revert(&mut self, change: &LineRangeMapping, cx: &mut Context<Self>) {
        let original = self
            .original
            .read(cx)
            .model()
            .read(cx)
            .buffer()
            .rope()
            .clone();
        let modified = self
            .modified
            .read(cx)
            .model()
            .read(cx)
            .buffer()
            .rope()
            .clone();
        self.revert_edits(vec![revert_points(&original, &modified, change)], cx);
    }

    /// Monaco `revertRangeMappings`: reverts character-level changes.
    pub fn revert_range_mappings(&mut self, changes: &[RangeMapping], cx: &mut Context<Self>) {
        let edits = changes
            .iter()
            .map(|c| {
                (
                    c.original_range.to_point_utf16_range(),
                    c.modified_range.to_point_utf16_range(),
                )
            })
            .collect();
        self.revert_edits(edits, cx);
    }

    /// Monaco `revertFocusedRangeMappings`: reverts the changes whose
    /// modified lines intersect or touch the modified editor's selection
    /// lines (`[start line, end line)`, empty for a cursor), when it has
    /// focus. So a cursor on the line below a deletion, or on the line
    /// after a change, reverts it.
    pub fn revert_focused_range_mappings(&mut self, window: &Window, cx: &mut Context<Self>) {
        let Some(diff) = self.diff.clone() else {
            return;
        };
        if !self
            .modified
            .read(cx)
            .focus_handle(cx)
            .contains_focused(window, cx)
        {
            return;
        }
        let selection = self.modified.read(cx).selections()[0];
        // Monaco `LineRange.fromRange(selection)`.
        let selected = LineRange::new(
            selection.start().row as usize + 1,
            selection.end().row as usize + 1,
        );
        let original = self
            .original
            .read(cx)
            .model()
            .read(cx)
            .buffer()
            .rope()
            .clone();
        let modified = self
            .modified
            .read(cx)
            .model()
            .read(cx)
            .buffer()
            .rope()
            .clone();
        let edits = diff
            .changes
            .iter()
            .filter(|c| c.modified.intersect(&selected).is_some())
            .map(|c| revert_points(&original, &modified, &c.line_range_mapping()))
            .collect();
        self.revert_edits(edits, cx);
    }

    /// Monaco `_selectedDiffs`: the inner changes the modified editor's
    /// selections intersect (`Range.areIntersecting`, which touching
    /// ranges don't), and the indices of the changes on the selected
    /// lines; nothing when no inner change is selected.
    fn selected_changes(&self, cx: &App) -> (Vec<RangeMapping>, Vec<usize>) {
        let Some(diff) = self.diff.as_ref() else {
            return Default::default();
        };
        let selections = self.modified.read(cx).selections();
        if selections.iter().all(|s| s.is_empty()) {
            return Default::default();
        }
        // Monaco's `LineRangeSet` of `LineRange.fromRangeInclusive`: the
        // selected lines, touching ranges joined.
        let mut selected_lines: Vec<LineRange> = selections
            .iter()
            .map(|s| LineRange::new(s.start().row as usize + 1, s.end().row as usize + 2))
            .collect();
        selected_lines.sort_by_key(|l| l.start_line_number);
        selected_lines.dedup_by(|next, joined| {
            let touches = next.start_line_number <= joined.end_line_number_exclusive;
            if touches {
                *joined = joined.join(next);
            }
            touches
        });
        // Monaco `LineRangeSet.intersects`.
        let lines_intersect = |change: &LineRange| {
            selected_lines.iter().any(|l| {
                l.start_line_number < change.end_line_number_exclusive
                    && l.end_line_number_exclusive > change.start_line_number
            })
        };
        let intersects =
            |a: &Range<PointUtf16>, b: &Range<PointUtf16>| a.end > b.start && b.end > a.start;
        let mut selected = Vec::new();
        let mut changes = Vec::new();
        for (ix, change) in diff.changes.iter().enumerate() {
            if change.inner_changes.is_empty() || !lines_intersect(&change.modified) {
                continue;
            }
            let inner: Vec<RangeMapping> = change
                .inner_changes
                .iter()
                .filter(|i| {
                    let range = i.modified_range.to_point_utf16_range();
                    selections
                        .iter()
                        .any(|s| intersects(&range, &(s.start()..s.end())))
                })
                .cloned()
                .collect();
            // Monaco keeps the change even when none of its characters
            // are selected, so it gets no button of its own.
            changes.push(ix);
            selected.extend(inner);
        }
        if selected.is_empty() {
            return Default::default();
        }
        (selected, changes)
    }

    /// Monaco `DiffEditorGutter`'s items: a "Revert Selection" item when
    /// the selection touches changed characters, else one per change.
    fn gutter_items(&self, cx: &App) -> Vec<GutterItem> {
        let Some(diff) = self.diff.as_ref() else {
            return Vec::new();
        };
        let (selected, _) = self.selected_changes(cx);
        if !selected.is_empty() {
            // Monaco `DetailedLineRangeMapping.fromRangeMappings`.
            let lines = |r: &crate::diff::Range| LineRange::from_range_inclusive(r);
            let join = |f: &dyn Fn(&RangeMapping) -> LineRange| {
                selected
                    .iter()
                    .map(f)
                    .reduce(|a, b| a.join(&b))
                    .unwrap_or(LineRange::new(1, 1))
            };
            let original = join(&|r| lines(&r.original_range));
            let modified = join(&|r| lines(&r.modified_range));
            return vec![GutterItem {
                lines: modified,
                mapping: DetailedLineRangeMapping::new(original, modified, selected.clone()),
                revert: GutterRevert::Selection(selected),
                show_always: true,
            }];
        }
        let original = self
            .original
            .read(cx)
            .model()
            .read(cx)
            .buffer()
            .rope()
            .clone();
        let modified = self
            .modified
            .read(cx)
            .model()
            .read(cx)
            .buffer()
            .rope()
            .clone();
        let cursor_line = self.modified.read(cx).selections()[0].head.row as usize + 1;
        diff.changes
            .iter()
            .map(|c| {
                let mapping = c.line_range_mapping();
                // Monaco `withInnerChangesFromLineRanges`.
                let (original_range, modified_range) =
                    revert_points(&original, &modified, &mapping);
                let whole = RangeMapping::new(
                    crate::diff::Range::from_point_utf16_range(original_range),
                    crate::diff::Range::from_point_utf16_range(modified_range),
                );
                GutterItem {
                    lines: c.modified,
                    revert: GutterRevert::Block(mapping),
                    show_always: c.modified.contains(cursor_line),
                    mapping: DetailedLineRangeMapping::new(c.original, c.modified, vec![whole]),
                }
            })
            .collect()
    }

    /// The gutter menu's actions for `toolbar`: the built-in revert (Monaco
    /// `diffEditor.revert`, while the modified side is editable), then the
    /// host's.
    fn toolbar_actions(&self, toolbar: DiffHunkToolbar, cx: &App) -> Vec<DiffHunkAction> {
        let mut actions = Vec::new();
        if !self.modified.read(cx).options().read_only {
            let (id, label) = match toolbar {
                DiffHunkToolbar::Hunk => ("diffEditor.revert", "Revert Block"),
                DiffHunkToolbar::Selection => ("diffEditor.revert", "Revert Selection"),
            };
            actions.push(DiffHunkAction {
                id: id.into(),
                label: label.into(),
                // Monaco's gutter menu: `discard` inline, `arrow-right` side
                // by side.
                icon: if self.inline {
                    "$(discard)"
                } else {
                    "$(arrow-right)"
                }
                .into(),
                toolbar,
                // Run by `run_hunk_action`, which knows the revert.
                run: Rc::new(|_, _, _| {}),
            });
        }
        actions.extend(
            self.hunk_actions
                .iter()
                .filter(|a| a.toolbar == toolbar)
                .cloned(),
        );
        actions
    }

    /// Monaco `DiffEditorGutter._hasActions`: the gutter shows while its
    /// menus have actions.
    fn shows_gutter(&self, cx: &App) -> bool {
        self.options.render_gutter_menu
            && (!self.modified.read(cx).options().read_only || !self.hunk_actions.is_empty())
    }

    /// Runs gutter action `action` for `item`.
    fn run_hunk_action(
        &mut self,
        action: &DiffHunkAction,
        revert: &GutterRevert,
        mapping: &DetailedLineRangeMapping,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let builtin = !self.hunk_actions.iter().any(|a| a.id == action.id);
        if builtin {
            match revert {
                GutterRevert::Block(change) => self.revert(change, cx),
                GutterRevert::Selection(changes) => self.revert_range_mappings(changes, cx),
            }
            return;
        }
        let original = self
            .original
            .read(cx)
            .model()
            .read(cx)
            .buffer()
            .rope()
            .clone();
        let modified = self
            .modified
            .read(cx)
            .model()
            .read(cx)
            .buffer()
            .rope()
            .clone();
        let context = DiffHunkContext {
            mapping: mapping.clone(),
            original_with_modified_changes: staged_value(&original, &modified, mapping),
        };
        (action.run)(&context, window, cx);
    }

    /// Whether the gutter shows its top scroll shadow (Monaco
    /// `EditorGutter`'s `isScrollTopZero`): the modified editor isn't
    /// scrolled to the top.
    fn gutter_scroll_decoration(&self, cx: &App) -> bool {
        self.modified.read(cx).scroll_position().y > px(0.)
    }

    /// Whether the gutter shows its top scroll shadow, for tests.
    #[doc(hidden)]
    pub fn gutter_scroll_decoration_for_tests(&self, cx: &App) -> bool {
        self.shows_gutter(cx) && self.gutter_scroll_decoration(cx)
    }

    /// Monaco `DiffEditorGutter`: a 35px column left of the modified editor
    /// with a vertical toolbar beside each change.
    fn render_gutter(&self, cx: &mut Context<Self>) -> Option<gpui::AnyElement> {
        if !self.shows_gutter(cx) {
            return None;
        }
        let editor = self.modified.read(cx);
        let colors = editor.colors().clone();
        let viewport = editor
            .layout
            .as_ref()
            .map_or(px(0.), |l| l.bounds.size.height);
        let hovered = self.gutter_hovered;
        let fade = self.gutter_fade;
        let mut items = Vec::new();
        for (ix, item) in self.gutter_items(cx).into_iter().enumerate() {
            let editor = self.modified.read(cx);
            let Some((top, bottom)) = editor.rows_y_span(item.lines.to_rows()) else {
                continue;
            };
            if bottom < px(0.) || top > viewport {
                continue;
            }
            let toolbar = match item.revert {
                GutterRevert::Block(_) => DiffHunkToolbar::Hunk,
                GutterRevert::Selection(_) => DiffHunkToolbar::Selection,
            };
            let mut actions = self.toolbar_actions(toolbar, cx);
            // Monaco's `overflowBehavior`: 3 buttons, or 1 when small.
            let small = item.mapping.original.start_line_number == 1 && bottom - top < px(30.);
            actions.truncate(if small { 1 } else { 3 });
            if actions.is_empty() {
                continue;
            }
            let toolbar_height = px(GUTTER_BUTTON_HEIGHT) * actions.len() as f32;
            let button_top = gutter_button_top(top, bottom, viewport, toolbar_height);
            // Monaco's `.gutterItem`: shown always, while the gutter is
            // hovered, or fading out (0.7s) after.
            if !item.show_always && !hovered && fade == 0 {
                continue;
            }
            let buttons = actions.into_iter().enumerate().map(|(button_ix, action)| {
                let (revert, mapping) = (item.revert.clone(), item.mapping.clone());
                let label = action.label.clone();
                let icon = action.icon.clone();
                div()
                    .id(("diff-hunk-action", ix * 8 + button_ix))
                    .tooltip(crate::tooltip::tooltip(label.clone(), &colors))
                    .h(px(GUTTER_BUTTON_HEIGHT))
                    .px(px(2.))
                    .py(px(1.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .cursor_pointer()
                    .rounded(px(4.))
                    .hover(|style| colors.toolbar_hover(style))
                    .role(gpui::Role::Button)
                    .aria_label(label)
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.run_hunk_action(&action, &revert, &mapping, window, cx)
                    }))
                    .children(label_with_icons(&icon))
            });
            let element = div()
                .absolute()
                .top(top)
                .h(bottom - top)
                .w_full()
                .child(
                    // Monaco's `.background`: a 1px line with a 2px left
                    // border at the middle.
                    div()
                        .absolute()
                        .left(px(GUTTER_WIDTH / 2.))
                        .h_full()
                        .w(px(1.))
                        .border_l_2()
                        .border_color(colors.menu_separator_background),
                )
                .child(
                    div()
                        .absolute()
                        .top(button_top - top)
                        .w_full()
                        .flex()
                        .justify_center()
                        .child(
                            div()
                                .flex()
                                .flex_col()
                                .rounded(px(4.))
                                .bg(colors.gutter_item_background)
                                .text_color(colors.gutter_item_glyph_foreground)
                                .children(buttons),
                        ),
                );
            if item.show_always || hovered {
                items.push(element.into_any_element());
            } else {
                items.push(
                    element
                        .with_animation(
                            ("diff-gutter-fade", fade * 4096 + ix),
                            gpui::Animation::new(Duration::from_millis(700)),
                            |element, delta| element.opacity(1. - delta),
                        )
                        .into_any_element(),
                );
            }
        }
        // Monaco `EditorGutter`'s `.scroll-decoration` while the modified
        // editor is scrolled down: `box-shadow: scrollbar.shadow 0 6px 6px
        // -6px inset`, 6px tall.
        let scroll_decoration = self.gutter_scroll_decoration(cx).then(|| {
            let shadow = colors.scrollbar_shadow;
            canvas(
                |_, _, _| {},
                move |bounds, _, window, _| {
                    window.paint_inset_shadows(
                        bounds,
                        gpui::Corners::default(),
                        &[gpui::BoxShadow {
                            color: shadow,
                            offset: point(px(0.), px(GUTTER_SCROLL_SHADOW)),
                            blur_radius: px(GUTTER_SCROLL_SHADOW),
                            spread_radius: px(-GUTTER_SCROLL_SHADOW),
                            inset: true,
                        }],
                    )
                },
            )
            .absolute()
            .top_0()
            .left_0()
            .w_full()
            .h(px(GUTTER_SCROLL_SHADOW))
        });
        Some(
            div()
                .id("diff-gutter")
                .relative()
                .h_full()
                .w(px(GUTTER_WIDTH))
                .flex_none()
                .overflow_hidden()
                .bg(colors.background)
                .children(scroll_decoration)
                .on_hover(cx.listener(|this, hovered: &bool, _, cx| {
                    if this.gutter_hovered && !hovered {
                        this.gutter_fade += 1;
                    }
                    this.gutter_hovered = *hovered;
                    cx.notify();
                }))
                // Monaco: a click on the gutter focuses the modified editor.
                .on_click(cx.listener(|this, _, window, cx| {
                    window.focus(&this.modified.focus_handle(cx), cx);
                }))
                .on_scroll_wheel(cx.listener(|this, event, _, cx| this.delegate_wheel(event, cx)))
                .children(items)
                .into_any_element(),
        )
    }

    /// Shows or hides the accessible diff viewer.
    pub(crate) fn set_accessible_viewer_visible(&mut self, visible: bool, cx: &mut Context<Self>) {
        let visible = visible || self.options.only_show_accessible_diff_viewer;
        if visible == self.accessible_viewer.is_some() {
            return;
        }
        if visible {
            let this = cx.entity().downgrade();
            let (original, modified) = (self.original.clone(), self.modified.clone());
            let can_close = !self.options.only_show_accessible_diff_viewer;
            let diff = self.diff.clone();
            self.accessible_viewer = Some(cx.new(|cx| {
                let mut viewer = AccessibleDiffViewer::new(this, original, modified, can_close, cx);
                viewer.set_diff(diff.as_ref(), cx);
                viewer
            }));
        } else {
            self.accessible_viewer = None;
        }
        cx.notify();
    }

    /// Monaco `accessibleDiffViewerNext`: opens the accessible diff viewer,
    /// or goes to its next change when it's open.
    pub fn accessible_diff_viewer_next(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let was_visible = self.accessible_viewer.is_some();
        self.set_accessible_viewer_visible(true, cx);
        if let Some(viewer) = self.accessible_viewer.clone() {
            if was_visible {
                viewer.update(cx, |viewer, cx| viewer.go_to_group_delta(1, cx));
            }
            window.focus(&viewer.read(cx).focus_handle(cx), cx);
        }
    }

    /// Monaco `accessibleDiffViewerPrev`: opens the accessible diff viewer
    /// at the previous change.
    pub fn accessible_diff_viewer_prev(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.set_accessible_viewer_visible(true, cx);
        if let Some(viewer) = self.accessible_viewer.clone() {
            viewer.update(cx, |viewer, cx| viewer.go_to_group_delta(-1, cx));
            window.focus(&viewer.read(cx).focus_handle(cx), cx);
        }
    }

    /// Sends an accessibility signal (Monaco `playSignal`).
    pub(crate) fn play_signal(&mut self, signal: AccessibilitySignal, cx: &mut Context<Self>) {
        cx.emit(DiffEditorEvent::AccessibilitySignal(signal));
    }

    /// Whether the accessible diff viewer shows.
    pub fn is_accessible_diff_viewer_visible(&self) -> bool {
        self.accessible_viewer.is_some()
    }

    /// The accessible diff viewer's group count, current group and current
    /// line, for tests.
    #[doc(hidden)]
    pub fn accessible_viewer_state_for_tests(&self, cx: &App) -> Option<(usize, usize, usize)> {
        Some(self.accessible_viewer.as_ref()?.read(cx).state_for_tests())
    }

    /// What the accessible diff viewer tells assistive technology, for
    /// tests (GPUI's test platform builds no accessibility tree).
    #[doc(hidden)]
    pub fn accessible_viewer_aria_for_tests(&self, cx: &App) -> Option<AccessibleDiffViewerAria> {
        Some(self.accessible_viewer.as_ref()?.read(cx).aria_for_tests(cx))
    }

    /// The accessible diff viewer's current line, for tests: its kind
    /// (`"header"`, `"unchanged"`, `"deleted"` or `"added"`).
    #[doc(hidden)]
    pub fn accessible_viewer_current_for_tests(&self, cx: &App) -> Option<&'static str> {
        use accessible::ViewElement::*;
        Some(
            match self
                .accessible_viewer
                .as_ref()?
                .read(cx)
                .current_for_tests()?
            {
                Header => "header",
                Unchanged { .. } => "unchanged",
                Deleted { .. } => "deleted",
                Added { .. } => "added",
            },
        )
    }

    /// Keeps both sides scrolled together (aligned, their content heights
    /// match).
    fn sync_scroll(&mut self, from_original: bool, cx: &mut Context<Self>) {
        if self.syncing_scroll {
            return;
        }
        self.syncing_scroll = true;
        let (source, target) = if from_original {
            (&self.original, &self.modified)
        } else {
            (&self.modified, &self.original)
        };
        let mut position = source.read(cx).scroll_position();
        // Comparing a moved block: its two places line up (Monaco's scroll
        // offsets).
        let delta = self.compare_scroll_delta(cx);
        position.y += if from_original { delta } else { -delta };
        // Inline, the original column doesn't scroll sideways with the
        // modified editor.
        if self.inline {
            position.x = target.read(cx).scroll_position().x;
        }
        if target.read(cx).scroll_position() != position {
            target.update(cx, |editor, cx| editor.set_scroll_position(position, cx));
        }
        self.syncing_scroll = false;
    }

    /// How much lower the compared moved block is in the modified editor
    /// than in the original (Monaco's `deltaOrigToMod`).
    fn compare_scroll_delta(&self, cx: &App) -> Pixels {
        let Some(m) = self
            .move_to_compare
            .and_then(|ix| self.diff.as_deref()?.moves.get(ix))
        else {
            return px(0.);
        };
        // From the top of the content, so it doesn't change as either side
        // scrolls.
        let top = |editor: &Entity<CodeEditor>, lines: LineRange| {
            let editor = editor.read(cx);
            let start = lines.to_rows().start;
            editor
                .rows_y_span(start..start + 1)
                .map(|(top, _)| top + editor.scroll_position().y)
        };
        match (
            top(&self.original, m.line_range_mapping.original),
            top(&self.modified, m.line_range_mapping.modified),
        ) {
            (Some(original), Some(modified)) => modified - original,
            _ => px(0.),
        }
    }

    /// Compares moved block `index` with its original, or stops (Monaco
    /// `movedTextToCompare`): only its changes show, and the sides scroll
    /// with its two places lined up.
    pub fn set_move_to_compare(&mut self, index: Option<usize>, cx: &mut Context<Self>) {
        if self.move_to_compare == index {
            return;
        }
        self.move_to_compare = index;
        self.apply_diff(cx);
        // Line up from the modified side.
        self.sync_scroll(false, cx);
        cx.notify();
    }

    /// The moved block being compared.
    pub fn move_to_compare(&self) -> Option<usize> {
        self.move_to_compare
    }

    /// Runs the diff editor command with Monaco id `name` (e.g.
    /// `"diffEditor.revert"`, `"diffEditor.toggleCollapseUnchangedRegions"`,
    /// `"editor.action.accessibleDiffViewer.next"`), or else the modified
    /// editor's command (see [`CodeEditor::trigger`]).
    pub fn trigger(
        &mut self,
        name: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> anyhow::Result<()> {
        let toggle = |this: &mut Self, f: fn(&mut DiffEditorOptions), cx: &mut Context<Self>| {
            let mut options = this.options.clone();
            f(&mut options);
            this.update_options(options, cx);
        };
        match name {
            "diffEditor.revert" => self.revert_focused_range_mappings(window, cx),
            "diffEditor.collapseAllUnchangedRegions" => self.collapse_all_unchanged_regions(cx),
            "diffEditor.showAllUnchangedRegions" => self.show_all_unchanged_regions(cx),
            "diffEditor.exitCompareMove" => self.set_move_to_compare(None, cx),
            "diffEditor.toggleCollapseUnchangedRegions" => toggle(
                self,
                |o| o.hide_unchanged_regions.enabled = !o.hide_unchanged_regions.enabled,
                cx,
            ),
            "diffEditor.toggleShowMovedCodeBlocks" => {
                toggle(self, |o| o.show_moves = !o.show_moves, cx)
            }
            "diffEditor.toggleUseInlineViewWhenSpaceIsLimited" => toggle(
                self,
                |o| {
                    o.use_inline_view_when_space_is_limited =
                        !o.use_inline_view_when_space_is_limited
                },
                cx,
            ),
            "diffEditor.switchSide" => self.switch_side(window, cx),
            "editor.action.accessibleDiffViewer.next" => {
                self.accessible_diff_viewer_next(window, cx)
            }
            "editor.action.accessibleDiffViewer.prev" => {
                self.accessible_diff_viewer_prev(window, cx)
            }
            _ if self.run_added_command(name, window, cx) => {}
            _ => {
                return self
                    .modified
                    .update(cx, |editor, cx| editor.trigger(name, window, cx));
            }
        }
        Ok(())
    }

    /// Monaco `revealFirstDiff`: once the diff is computed, moves the
    /// modified editor's cursor to the first change and reveals it.
    pub fn reveal_first_diff(&mut self, cx: &mut Context<Self>) {
        if self.diff_task.is_some() || self.diff.is_none() {
            // Monaco `waitForDiff`.
            self.reveal_first_diff_pending = true;
        } else {
            self.reveal_first_change(cx);
        }
    }

    /// Drops a [`DiffEditor::reveal_first_diff`] still waiting for its
    /// diff (the multi-diff editor's restored selections win over it).
    pub(crate) fn cancel_reveal_first_diff(&mut self) {
        self.reveal_first_diff_pending = false;
    }

    /// Monaco `_goTo(diffs[0])`.
    fn reveal_first_change(&mut self, cx: &mut Context<Self>) {
        let Some(change) = self.diff.as_ref().and_then(|d| d.changes.first()) else {
            return;
        };
        let modified = change.modified;
        self.modified.update(cx, |editor, cx| {
            let row = modified.start_line_number as u32 - 1;
            editor.set_selections(&[Selection::collapsed(PointUtf16::new(row, 0))], cx);
            // Monaco `toExclusiveRange`: through the start of the next line.
            let end = modified.end_line_number_exclusive as u32 - 1;
            editor.reveal_range(
                PointUtf16::new(row, 0)..PointUtf16::new(end, 0),
                crate::view::reveal::RevealType::Center,
                cx,
            );
        });
    }

    /// Monaco `mapToOtherSide`: the other editor than the focused one (the
    /// original when neither is), and the focused editor's selection
    /// mapped through the diff to it.
    pub fn map_to_other_side(
        &self,
        window: &Window,
        cx: &App,
    ) -> (Entity<CodeEditor>, Option<Range<PointUtf16>>) {
        let modified_focused = self.modified.focus_handle(cx).contains_focused(window, cx);
        let (source, destination) = if modified_focused {
            (&self.modified, &self.original)
        } else {
            (&self.original, &self.modified)
        };
        let selection = source.read(cx).selections()[0];
        let destination_selection = self.diff.as_ref().map(|diff| {
            let mappings: Vec<DetailedLineRangeMapping> = if modified_focused {
                diff.changes.iter().map(|c| c.flip()).collect()
            } else {
                diff.changes.clone()
            };
            let translate = |point: PointUtf16| {
                translate_position(crate::diff::Position::from_point_utf16(point), &mappings)
            };
            let range = translate(selection.start()).plus_range(&translate(selection.end()));
            range.to_point_utf16_range()
        });
        (destination.clone(), destination_selection)
    }

    /// Focuses the other side with the selection mapped to it (Monaco
    /// `diffEditor.switchSide`).
    pub fn switch_side(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let (destination, selection) = self.map_to_other_side(window, cx);
        window.focus(&destination.focus_handle(cx), cx);
        if let Some(range) = selection {
            destination.update(cx, |editor, cx| {
                editor.set_selections(
                    &[Selection {
                        anchor: range.start,
                        head: range.end,
                    }],
                    cx,
                );
            });
        }
    }

    /// Both editors' view states and the collapsed unchanged regions
    /// (Monaco `saveViewState`).
    pub fn save_view_state(&self, cx: &App) -> DiffEditorViewState {
        DiffEditorViewState {
            original: self.original.read(cx).save_view_state(cx),
            modified: self.modified.read(cx).save_view_state(cx),
            collapsed_regions: self
                .unchanged_regions()
                .iter()
                .map(|r| r.hidden_modified_range())
                .collect(),
        }
    }

    /// Restores a [`DiffEditor::save_view_state`] (Monaco
    /// `restoreViewState`). The collapsed regions apply to the regions
    /// they intersect, once the diff is computed.
    pub fn restore_view_state(&mut self, state: &DiffEditorViewState, cx: &mut Context<Self>) {
        self.original.update(cx, |editor, cx| {
            editor.restore_view_state(&state.original, cx)
        });
        self.modified.update(cx, |editor, cx| {
            editor.restore_view_state(&state.modified, cx)
        });
        if self.diff.is_some() && self.diff_task.is_none() {
            // After the editors' selection events, which reveal the lines
            // with cursors, like Monaco's order.
            let ranges = state.collapsed_regions.clone();
            let this = cx.entity().downgrade();
            cx.defer(move |cx| {
                this.update(cx, |this, cx| this.restore_collapsed_regions(&ranges, cx))
                    .ok();
            });
        } else {
            self.pending_collapsed_regions = Some(state.collapsed_regions.clone());
        }
    }

    /// Monaco `restoreSerializedState`: each region takes the first saved
    /// range it intersects as its hidden range.
    fn restore_collapsed_regions(&mut self, ranges: &[LineRange], cx: &mut Context<Self>) {
        if self.unchanged_regions().is_empty() {
            return;
        }
        for region in self.unchanged_regions_mut() {
            let unchanged = region.modified_unchanged_range();
            if let Some(range) = ranges.iter().find(|r| unchanged.intersect(r).is_some()) {
                region.set_hidden_modified_range(*range);
            }
        }
        self.apply_unchanged(cx);
    }

    /// Monaco `goToDiff`: moves the modified editor's cursor to the next or
    /// previous change and reveals it, leaving the focus where it is.
    pub fn go_to_diff(&mut self, next: bool, cx: &mut Context<Self>) {
        let Some(diff) = self.diff.clone() else {
            return;
        };
        if diff.changes.is_empty() {
            return;
        }
        let line = self.modified.read(cx).selections()[0].head.row as usize + 1;
        let line_count = self
            .modified
            .read(cx)
            .model()
            .read(cx)
            .buffer()
            .rope()
            .line_count() as usize;
        let change = if next {
            if line == line_count {
                &diff.changes[0]
            } else {
                diff.changes
                    .iter()
                    .find(|c| c.modified.start_line_number > line)
                    .unwrap_or(&diff.changes[0])
            }
        } else {
            diff.changes
                .iter()
                .rev()
                .find(|c| c.modified.start_line_number < line)
                .unwrap_or(&diff.changes[diff.changes.len() - 1])
        };
        let row = change.modified.start_line_number as u32 - 1;
        let end = (change
            .modified
            .end_line_number_exclusive
            .max(change.modified.start_line_number + 1) as u32)
            .saturating_sub(1);
        self.modified.update(cx, |editor, cx| {
            editor.set_selections(&[Selection::collapsed(PointUtf16::new(row, 0))], cx);
            editor.reveal_range(
                PointUtf16::new(row, 0)..PointUtf16::new(end, 0),
                crate::view::reveal::RevealType::Center,
                cx,
            );
        });
        self.play_signal(change_signal(change), cx);
    }
}

impl CodeEditor {
    /// The vertical span of model `rows` from the editor's top (Monaco
    /// `getTopForLineNumber`/`getBottomForLineNumber`). Lines are placed as
    /// of the last layout, scrolled by the current position, so hosts drawn
    /// before the editor's next frame line up with it. Empty `rows` span the
    /// gap before their row, with its view zones.
    pub(crate) fn rows_y_span(&self, rows: Range<u32>) -> Option<(Pixels, Pixels)> {
        let layout = self.layout.as_ref()?;
        let line_height = layout.metrics.line_height;
        let zones = &layout.zones;
        let scroll_top = self.scroll_position().y;
        let y = |line: u32| zones.line_y(line, line_height) - scroll_top;
        let first = self.view_lines.first_view_line_of_row(rows.start);
        if rows.is_empty() {
            let top = if first == 0 {
                -scroll_top
            } else {
                y(first - 1) + zones.height(first - 1, line_height)
            };
            return Some((top, y(first)));
        }
        let end = self.view_lines.first_view_line_of_row(rows.end);
        // Hidden rows have no view lines.
        (end > first).then(|| (y(first), y(end - 1) + zones.height(end - 1, line_height)))
    }

    /// Monaco `getAdditionalLineHeights` for wrapping: (1-based line,
    /// pixels) for each line that wraps, as of the last layout.
    pub(crate) fn wrap_extra_heights(&self) -> Vec<(usize, f32)> {
        let line_height = f32::from(self.line_height());
        let view_lines = &self.view_lines;
        if view_lines.wrap().is_none() {
            return Vec::new();
        }
        let rows = view_lines.rope().line_count();
        (0..rows)
            .filter(|&row| !view_lines.is_row_hidden(row))
            .filter_map(|row| {
                let first = view_lines.first_view_line_of_row(row);
                let next = if row + 1 < rows {
                    view_lines.first_view_line_of_row(row + 1)
                } else {
                    view_lines.line_count()
                };
                (next > first + 1)
                    .then(|| (row as usize + 1, (next - first - 1) as f32 * line_height))
            })
            .collect()
    }

    /// Monaco `getAdditionalLineHeights` for view zones: (1-based line the
    /// zone is after, pixels) for each zone not in `ignore`.
    pub(crate) fn zone_heights(&self, ignore: &[WidgetId], cx: &App) -> Vec<(usize, f32)> {
        let line_height = f32::from(self.line_height());
        let buffer = self.model().read(cx).buffer();
        self.widgets
            .zones
            .iter()
            .filter(|zone| !ignore.contains(&zone.id))
            .map(|zone| {
                let line = zone.anchor.as_ref().map_or(0, |anchor| {
                    buffer.offset_to_point(buffer.resolve(anchor)).row as usize + 1
                });
                let height = match zone.height {
                    ZoneHeight::Lines(lines) => lines * line_height,
                    ZoneHeight::Pixels(height) => f32::from(height),
                };
                (line, height)
            })
            .collect()
    }

    /// Sets a zone's Monaco `minWidthInPx`.
    pub(crate) fn set_zone_min_width(
        &mut self,
        id: WidgetId,
        width: Pixels,
        cx: &mut Context<Self>,
    ) {
        if let Some(zone) = self.widgets.zones.iter_mut().find(|z| z.id == id)
            && zone.min_width != width
        {
            zone.min_width = width;
            cx.notify();
        }
    }

    /// Monaco's line breaks computer (`createLineBreaksComputer`) for text
    /// that isn't in the model, such as the inline view's deleted lines:
    /// the UTF-16 ranges of `line`'s view lines with this editor's
    /// wrapping, and the continuation indent in visible columns.
    pub(crate) fn line_breaks_for(&self, line: &str) -> (Vec<Range<u32>>, u32) {
        let len = line.encode_utf16().count() as u32;
        let breaks = self.view_lines.wrap().and_then(|wrap| {
            crate::view::line_breaks::compute_measured_line_breaks(
                &wrap.classifier,
                line,
                wrap.tab_size,
                Some(wrap.column),
                wrap.indent,
                wrap.keep_all,
                wrap.wrap_on_escaped_line_feeds,
                wrap.measure.as_ref().map(|m| &*m.0),
            )
        });
        match breaks {
            Some(data) => (
                (0..data.output_line_count())
                    .map(|ix| data.input_range(ix))
                    .collect(),
                data.wrapped_text_indent_length,
            ),
            None => (std::iter::once(0..len).collect(), 0),
        }
    }
}

/// Monaco `applyEditToLineRange`: `range` after `edits` (start line, end
/// line, line delta), given in descending order; `None` when an edit
/// swallows it.
fn apply_edits_to_line_range(
    range: LineRange,
    edits: &[(usize, usize, isize)],
) -> Option<LineRange> {
    let mut start = range.start_line_number as isize;
    let mut end = range.end_line_number_exclusive as isize;
    for &(edit_start, edit_end, delta) in edits {
        let (edit_start, edit_end) = (edit_start as isize, edit_end as isize);
        if edit_end < start {
            // The edit is before the range.
            start += delta;
            end += delta;
        } else if edit_start > end {
            // The edit is after the range.
        } else if edit_start < start && end < edit_end {
            // The range is inside the edit.
            return None;
        } else if edit_start < start && edit_end <= end {
            // The edit ends inside the range.
            start = edit_end + 1 + delta;
            end += delta;
        } else {
            end += delta;
        }
    }
    (start >= 1 && end >= start).then(|| LineRange::new(start as usize, end as usize))
}

/// Monaco's signal for a change: `diffLineDeleted`, `diffLineInserted` or
/// `diffLineModified`.
fn change_signal(change: &DetailedLineRangeMapping) -> AccessibilitySignal {
    if change.modified.is_empty() {
        AccessibilitySignal::DiffLineDeleted
    } else if change.original.is_empty() {
        AccessibilitySignal::DiffLineInserted
    } else {
        AccessibilitySignal::DiffLineModified
    }
}

/// Monaco `translatePosition`: where `position` in the original text is in
/// the modified text, given the changes (a range when it's inside a
/// changed span).
fn translate_position(
    position: crate::diff::Position,
    mappings: &[DetailedLineRangeMapping],
) -> crate::diff::Range {
    use crate::diff::{Position, Range};
    let at = |p: Position| Range::from_positions(p, p);
    let Some(mapping) = mappings
        .iter()
        .rev()
        .find(|m| m.original.start_line_number <= position.line_number)
    else {
        // No changes before the position.
        return at(position);
    };
    if mapping.original.end_line_number_exclusive <= position.line_number {
        let line = position.line_number - mapping.original.end_line_number_exclusive
            + mapping.modified.end_line_number_exclusive;
        return at(Position::new(line, position.column));
    }
    // Only for the legacy algorithm, which may have no inner changes.
    if mapping.inner_changes.is_empty() {
        return at(Position::new(mapping.modified.start_line_number, 1));
    }
    let Some(inner) = mapping
        .inner_changes
        .iter()
        .rev()
        .find(|m| m.original_range.start() <= position)
    else {
        let line = position.line_number - mapping.original.start_line_number
            + mapping.modified.start_line_number;
        return at(Position::new(line, position.column));
    };
    let original = inner.original_range;
    if original.start() <= position && position <= original.end() {
        return inner.modified_range;
    }
    // Monaco `lengthBetweenPositions(end, position).addToPosition(...)`.
    let end = original.end();
    let modified_end = inner.modified_range.end();
    let translated = if end.line_number == position.line_number {
        Position::new(
            modified_end.line_number,
            modified_end.column + position.column - end.column,
        )
    } else {
        Position::new(
            modified_end.line_number + position.line_number - end.line_number,
            position.column,
        )
    };
    at(translated)
}

fn clear_side(
    editor: &Entity<CodeEditor>,
    decorations: Vec<DecorationId>,
    zones: Vec<WidgetId>,
    cx: &mut Context<DiffEditor>,
) {
    let id = editor.entity_id();
    let model = editor.read(cx).model().clone();
    model.update(cx, |model, cx| {
        model.delta_decorations(Some(id), &decorations, Vec::new(), cx);
    });
    editor.update(cx, |editor, cx| {
        for zone in zones {
            editor.remove_widget(zone, cx);
        }
    });
}

/// A decoration's range on one side: whole lines or a text range.
#[derive(Clone, Debug, PartialEq)]
enum Side {
    /// 1-based lines, inclusive of the last line's end.
    Lines(LineRange),
    /// A text range in rows and UTF-16 columns.
    Text(Range<PointUtf16>),
}

impl Side {
    fn to_offsets(&self, rope: &text::Rope) -> Range<usize> {
        match self {
            Side::Lines(lines) => {
                let rows = lines.to_rows();
                let last = rope.max_point().row;
                let start = rope.point_to_offset(Point::new(rows.start.min(last), 0));
                let end_row = rows.end.saturating_sub(1).min(last);
                start..rope.line_range(end_row).end
            }
            Side::Text(range) => {
                rope.point_utf16_to_offset(range.start)..rope.point_utf16_to_offset(range.end)
            }
        }
    }
}

/// The diff overview's insert and remove colors (Monaco
/// `OverviewRulerFeature`'s `currentColors`): `diffEditorOverview.*Foreground`,
/// else `diffEditor.*TextBackground`, else Monaco's `defaultInsertColor`
/// (`rgba(155, 185, 85, .2)`) or `defaultRemoveColor` (`rgba(255, 0, 0,
/// .2)`), at twice the alpha. High contrast themes set none of them.
fn diff_overview_colors(theme: &crate::theme::Theme) -> (Hsla, Hsla) {
    let color = |overview: &str, text: &str, default: gpui::Rgba| {
        theme.color(overview).unwrap_or_else(|| {
            let c = theme.color(text).unwrap_or_else(|| default.into());
            Hsla {
                a: (c.a * 2.).min(1.),
                ..c
            }
        })
    };
    let rgba = |r: f32, g: f32, b: f32| gpui::Rgba {
        r: r / 255.,
        g: g / 255.,
        b: b / 255.,
        a: 0.2,
    };
    (
        color(
            "diffEditorOverview.insertedForeground",
            "diffEditor.insertedTextBackground",
            rgba(155., 185., 85.),
        ),
        color(
            "diffEditorOverview.removedForeground",
            "diffEditor.removedTextBackground",
            rgba(255., 0., 0.),
        ),
    )
}

/// A decoration on one side of the diff.
type SideDecoration = (Side, DecorationOptions);

/// Monaco `DiffEditorDecorations`: line backgrounds and gutter signs for
/// changed lines, and text backgrounds for the changed characters. Both
/// get the `diffEditor.insertedTextBorder`/`removedTextBorder`, dashed in
/// high contrast themes (Monaco's `.line-insert`, `.char-insert`, … in
/// `style.css`).
fn diff_decorations(
    changes: &[DetailedLineRangeMapping],
    options: &DiffEditorOptions,
    true_inline: bool,
    original_text: &text::Rope,
    colors: &EditorColors,
    high_contrast: bool,
) -> (Vec<SideDecoration>, Vec<SideDecoration>) {
    let border_style = if high_contrast {
        gpui::BorderStyle::Dashed
    } else {
        gpui::BorderStyle::Solid
    };
    let line = |inserted: bool| {
        let (background, gutter, sign, border) = if inserted {
            (
                colors.diff_inserted_line_background,
                colors.diff_gutter_inserted_line_background,
                Codicon::Add,
                colors.diff_inserted_text_border,
            )
        } else {
            (
                colors.diff_removed_line_background,
                colors.diff_gutter_removed_line_background,
                Codicon::Remove,
                colors.diff_removed_text_border,
            )
        };
        DecorationOptions {
            background: Some(background),
            border,
            border_style,
            is_whole_line: true,
            margin_background: Some(gutter),
            // Monaco's `insert-sign`/`delete-sign`: 0.7 opacity.
            lines_decoration_text: options
                .render_indicators
                .then(|| (sign.text(), colors.foreground.opacity(0.7))),
            ..Default::default()
        }
    };
    let text = |inserted: bool, whole_line: bool| {
        let (background, border) = if inserted {
            (
                colors.diff_inserted_text_background,
                colors.diff_inserted_text_border,
            )
        } else {
            (
                colors.diff_removed_text_background,
                colors.diff_removed_text_border,
            )
        };
        DecorationOptions {
            background: (background.a > 0.).then_some(background),
            border,
            border_style,
            is_whole_line: whole_line,
            // Monaco `diffAddDecoration`/`diffDeleteDecoration`.
            should_fill_line_on_line_break: !whole_line,
            ..Default::default()
        }
    };
    // Monaco `diffDeleteDecorationEmpty`/`diffAddDecorationEmpty`: a 3px
    // bar in the text background color (`diff-range-empty`), a pixel left
    // of the position on the original side.
    let empty = |inserted: bool| {
        let (color, offset) = if inserted {
            (colors.diff_inserted_text_background, px(0.))
        } else {
            (colors.diff_removed_text_background, px(-1.))
        };
        DecorationOptions {
            left_border: Some(crate::decorations::LeftBorder {
                color,
                width: px(3.),
                offset,
            }),
            ..Default::default()
        }
    };
    let show_empty = options.show_empty_decorations;
    let mut original = Vec::new();
    let mut modified = Vec::new();
    for m in changes {
        if !m.original.is_empty() {
            original.push((Side::Lines(m.original), line(false)));
        }
        if !m.modified.is_empty() {
            modified.push((Side::Lines(m.modified), line(true)));
        }
        if m.modified.is_empty() || m.original.is_empty() {
            if !m.original.is_empty() {
                original.push((Side::Lines(m.original), text(false, true)));
            }
            if !m.modified.is_empty() {
                modified.push((Side::Lines(m.modified), text(true, true)));
            }
        } else {
            let inline_diff = true_inline && allows_true_inline_diff_rendering(m);
            for i in &m.inner_changes {
                // Monaco skips empty markers outside the line range.
                if m.original.contains(i.original_range.start_line_number) {
                    let range = Side::Text(i.original_range.to_point_utf16_range());
                    if !i.original_range.is_empty() {
                        original.push((range, text(false, false)));
                    } else if show_empty {
                        original.push((range, empty(false)));
                    }
                }
                if m.modified.contains(i.modified_range.start_line_number) {
                    let range = Side::Text(i.modified_range.to_point_utf16_range());
                    if !i.modified_range.is_empty() {
                        modified.push((range, text(true, false)));
                    } else if show_empty && !inline_diff {
                        modified.push((range, empty(true)));
                    }
                }
                // Monaco's true inline view: the deleted text, struck
                // through, before the inserted text (`inline-deleted-text`).
                if inline_diff {
                    let deleted = Side::Text(i.original_range.to_point_utf16_range())
                        .to_offsets(original_text);
                    let deleted = original_text.text_for_range(deleted);
                    if !deleted.is_empty() {
                        let start = i.modified_range.to_point_utf16_range().start;
                        modified.push((
                            Side::Text(start..start),
                            DecorationOptions {
                                before: Some(InjectedText {
                                    content: deleted.into(),
                                    style: crate::theme::TokenStyle {
                                        strikethrough: true,
                                        ..Default::default()
                                    },
                                    background: Some(colors.diff_removed_text_background),
                                    ..Default::default()
                                }),
                                show_if_collapsed: true,
                                z_index: 100_000,
                                ..Default::default()
                            },
                        ));
                    }
                }
            }
        }
    }
    (original, modified)
}

/// The diff editor's key context with `keys`.
fn key_context_with(keys: &[(SharedString, ContextKeyValue)]) -> gpui::KeyContext {
    let mut context = gpui::KeyContext::new_with_defaults();
    context.add(DIFF_EDITOR_KEY_CONTEXT);
    for (key, value) in keys {
        value.apply(key, &mut context);
    }
    context
}

impl Render for DiffEditor {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.screen_reader_mode = self.modified.read(cx).screen_reader_optimized(window);
        // Monaco `useInlineViewWhenSpaceIsLimited`: the width decides.
        let inline = !self.side_by_side_for(&self.options);
        if inline != self.inline {
            self.inline = inline;
            self.apply_word_wrap(cx);
            self.apply_editor_options(cx);
            self.apply_diff(cx);
        } else if self
            .decorated_theme
            .as_ref()
            .is_some_and(|theme| !Arc::ptr_eq(theme, self.modified.read(cx).theme()))
        {
            // The decorations carry the theme's colors (and its high
            // contrast borders).
            self.apply_diff(cx);
        }
        self.pass_context_keys(window, cx);
        let gutter = self.render_gutter(cx);
        let gutter_width = if gutter.is_some() {
            px(GUTTER_WIDTH)
        } else {
            px(0.)
        };
        let this = cx.entity().downgrade();
        // Drags follow the mouse anywhere in the window, with the pointer
        // they started with (Monaco's pointer capture).
        let captured_cursor = self.captured_cursor();
        let drag_listener = self.unchanged_drag.is_some().then(|| {
            crate::pointer_capture::capture(
                this.clone(),
                captured_cursor,
                |this: &mut Self, event, cx| this.drag_unchanged(event.position.y, cx),
                |this, cx| this.end_unchanged_drag(cx),
            )
        });
        // The editors under a bar edge drag take no mouse.
        let drag_occluder = self.unchanged_drag.is_some().then(|| {
            div()
                .id("diff-drag-occluder")
                .absolute()
                .size_full()
                .occlude()
        });
        let sash_listener = self.sash_drag.is_some().then(|| {
            crate::pointer_capture::capture(
                this.clone(),
                captured_cursor,
                |this: &mut Self, event, cx| this.drag_sash(event, cx),
                |this, cx| {
                    this.sash_drag = None;
                    cx.notify();
                },
            )
        });
        let overview_listener = self.overview_drag.is_some().then(|| {
            crate::pointer_capture::capture(
                this.clone(),
                captured_cursor,
                |this: &mut Self, event, cx| this.drag_overview(event.position.y, cx),
                |this, cx| {
                    this.overview_drag = None;
                    cx.notify();
                },
            )
        });
        let move_lines = (!inline).then(|| self.render_move_lines(cx)).flatten();
        let (move_lines_width, move_lines_overlay) = match move_lines {
            Some((width, overlay)) => (width, Some(overlay)),
            None => (px(0.), None),
        };
        let move_column = (move_lines_width > px(0.)).then(|| {
            div()
                .id("diff-move-lines")
                .h_full()
                .w(move_lines_width)
                .flex_none()
                .on_scroll_wheel(cx.listener(|this, event, _, cx| this.delegate_wheel(event, cx)))
        });
        let overview = self
            .options
            .render_overview_ruler
            .then(|| self.render_overview_ruler(cx));
        let bounds = self.bounds.clone();
        let colors = self.modified.read(cx).colors().clone();
        let resizable = self.options.enable_split_view_resizing;
        let measured = self.sides_width() > px(0.);
        let sash_left = self.sash_left();
        // Monaco's layout: side by side, the original ends at the sash less
        // the gutter and moved-lines column; inline, the original is a
        // column of its line numbers after the gutter.
        let original_width = if inline {
            self.inline_original_width(cx)
        } else {
            (sash_left - gutter_width - move_lines_width).max(px(0.))
        };
        let original = if inline {
            // The editor lays out wider than its column, which shows only
            // its line numbers.
            div()
                .relative()
                .h_full()
                .w(original_width)
                .flex_none()
                .overflow_hidden()
                .child(
                    div()
                        .absolute()
                        .top_0()
                        .left_0()
                        .h_full()
                        .w(original_width + px(200.))
                        .child(self.original.clone()),
                )
        } else {
            // Monaco's `.editor.original`: `diffEditor.border` and a
            // `scrollbar.shadow` box shadow on its right.
            let original = div().h_full();
            let original = if measured {
                original.w(original_width).flex_none()
            } else {
                original.w(gpui::relative(self.ratio.clamp(0.1, 0.9)))
            };
            original
                .when_some(colors.diff_border, |original, border| {
                    original.border_r_1().border_color(border)
                })
                .shadow(vec![gpui::BoxShadow {
                    color: colors.scrollbar_shadow,
                    offset: point(px(6.), px(0.)),
                    blur_radius: px(5.),
                    spread_radius: px(-5.),
                    inset: false,
                }])
                .child(self.original.clone())
        };
        // Monaco `Sash`: a 4px hit area on the boundary, `sash.hoverBorder`
        // once the mouse has rested on it for the hover delay, or at once
        // while dragging; a double click resets the ratio. A second one left
        // of the gutter moves the same boundary.
        let sash = |id: &'static str, x: Pixels, cx: &mut Context<Self>| {
            let active = self.sash_drag.is_some() || self.sash_hover.shows(id);
            div()
                .id(id)
                .absolute()
                .top_0()
                .h_full()
                .left(x - px(SASH_SIZE / 2.))
                .w(px(SASH_SIZE))
                .cursor(SASH_CURSOR)
                .when(active, |sash| sash.bg(colors.sash_hover_border))
                .on_hover(cx.listener(move |this, hovered: &bool, _, cx| {
                    this.sash_hover.set_hovered(
                        id,
                        *hovered,
                        |this: &mut Self| &mut this.sash_hover,
                        cx,
                    );
                }))
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(|this, event: &gpui::MouseDownEvent, _, cx| {
                        cx.stop_propagation();
                        if event.click_count == 2 {
                            this.reset_sash(cx);
                            return;
                        }
                        this.sash_drag = Some((event.position.x, this.sash_left()));
                        cx.notify();
                    }),
                )
        };
        let sashes = (!inline && resizable && measured).then(|| {
            let mut sashes = vec![sash("diff-sash", sash_left, cx)];
            if gutter.is_some() {
                sashes.push(sash("diff-gutter-sash", sash_left - gutter_width, cx));
            }
            sashes
        });
        // Monaco's boundary: `diffEditor.border` and a `scrollbar.shadow`
        // box shadow on the modified editor's left.
        let shadow = colors.scrollbar_shadow;
        let modified = div()
            .h_full()
            .flex_1()
            .min_w_0()
            .when(!inline, |modified| {
                modified
                    .when_some(colors.diff_border, |modified, border| {
                        modified.border_l_1().border_color(border)
                    })
                    .shadow(vec![gpui::BoxShadow {
                        color: shadow,
                        offset: point(px(-6.), px(0.)),
                        blur_radius: px(5.),
                        spread_radius: px(-5.),
                        inset: false,
                    }])
            })
            .child(self.modified.clone());
        let viewer_shown = self.accessible_viewer.is_some();
        let root = div()
            .key_context(key_context_with(&self.passed_keys))
            .on_action(
                cx.listener(|this, _: &ExitCompareMove, _, cx| this.set_move_to_compare(None, cx)),
            )
            // Keybindings from `add_command`, which the focused side passes
            // on.
            .on_action(
                cx.listener(|this, action: &crate::actions::RunCommand, window, cx| {
                    if !this.run_added_command(&action.id, window, cx) {
                        cx.propagate();
                    }
                }),
            )
            .track_focus(&self.focus_handle)
            .size_full()
            .relative()
            .flex()
            .flex_row()
            .bg(colors.background)
            .on_action(cx.listener(|this, _: &GoToNextDiff, _, cx| this.go_to_diff(true, cx)))
            .on_action(cx.listener(|this, _: &GoToPreviousDiff, _, cx| this.go_to_diff(false, cx)))
            .on_action(cx.listener(|this, _: &CollapseAllUnchangedRegions, _, cx| {
                this.collapse_all_unchanged_regions(cx)
            }))
            .on_action(cx.listener(|this, _: &ShowAllUnchangedRegions, _, cx| {
                this.show_all_unchanged_regions(cx)
            }))
            .on_action(cx.listener(|this, _: &Revert, window, cx| {
                this.revert_focused_range_mappings(window, cx)
            }))
            .on_action(cx.listener(|this, _: &SwitchSide, window, cx| this.switch_side(window, cx)))
            .on_action(
                cx.listener(|this, _: &ToggleCollapseUnchangedRegions, window, cx| {
                    this.trigger("diffEditor.toggleCollapseUnchangedRegions", window, cx)
                        .ok();
                }),
            )
            .on_action(
                cx.listener(|this, _: &ToggleShowMovedCodeBlocks, window, cx| {
                    this.trigger("diffEditor.toggleShowMovedCodeBlocks", window, cx)
                        .ok();
                }),
            )
            .on_action(cx.listener(
                |this, _: &ToggleUseInlineViewWhenSpaceIsLimited, window, cx| {
                    this.trigger(
                        "diffEditor.toggleUseInlineViewWhenSpaceIsLimited",
                        window,
                        cx,
                    )
                    .ok();
                },
            ))
            .on_action(
                cx.listener(|this, _: &AccessibleDiffViewerNext, window, cx| {
                    this.accessible_diff_viewer_next(window, cx)
                }),
            )
            .on_action(
                cx.listener(|this, _: &AccessibleDiffViewerPrev, window, cx| {
                    this.accessible_diff_viewer_prev(window, cx)
                }),
            )
            .children(sash_listener)
            .children(overview_listener)
            .child(
                canvas(move |b, _, _| bounds.set(b), |_, _, _, _| {})
                    .absolute()
                    .size_full(),
            );
        let root = if inline {
            root.children(gutter)
                .when(original_width > px(0.), |this| this.child(original))
        } else {
            root.child(original).children(move_column).children(gutter)
        };
        root.child(modified)
            .children(overview)
            .children(move_lines_overlay)
            .children(sashes.into_iter().flatten())
            .children(drag_listener)
            .children(drag_occluder)
            // Monaco hides both editors behind the viewer.
            .when(viewer_shown, |this| {
                this.children(self.accessible_viewer.clone())
            })
    }
}

use gpui::prelude::FluentBuilder as _;
use gpui::{AnimationExt as _, anchored, deferred};

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diff::compute_diff;

    fn hex(c: Hsla) -> String {
        let c = gpui::Rgba::from(c);
        format!(
            "{:02x}{:02x}{:02x}{:02x}",
            (c.r * 255.).round() as u8,
            (c.g * 255.).round() as u8,
            (c.b * 255.).round() as u8,
            (c.a * 255.).round() as u8,
        )
    }

    /// Monaco `OverviewRulerFeature`: high contrast themes set neither the
    /// overview nor the text colors, so `defaultInsertColor` and
    /// `defaultRemoveColor` show, at twice their alpha.
    #[test]
    fn overview_colors_fall_back_to_monaco_defaults() {
        for theme in [
            crate::theme::Theme::hc_black(),
            crate::theme::Theme::hc_light(),
        ] {
            let (insert, remove) = diff_overview_colors(&theme);
            assert_eq!(hex(insert), "9bb95566");
            assert_eq!(hex(remove), "ff000066");
        }
        // vs-dark: `diffEditor.insertedTextBackground` (#9ccc2c33) doubled.
        let (insert, remove) = diff_overview_colors(&crate::theme::Theme::vs_dark());
        assert_eq!(hex(insert), "9ccc2c66");
        assert_eq!(hex(remove), "ff000066");
    }

    /// Monaco `renderViewLine` with `renderWhitespace` and
    /// `renderControlCharacters`, as the inline deleted lines render.
    #[test]
    fn deleted_line_display_renders_whitespace_and_controls() {
        let plain = deleted_line_display("\ta b", 4, &[], false);
        assert_eq!(plain.text, "    a b");
        assert_eq!(plain.map, vec![0, 4, 5, 6, 7]);
        assert!(plain.whitespace.is_empty() && plain.controls.is_empty());

        let all = crate::view::guides::whitespace_columns(
            "\ta b",
            0,
            0,
            false,
            crate::options::RenderWhitespace::All,
            &[],
        );
        let shown = deleted_line_display("\ta b", 4, &all, false);
        assert_eq!(shown.text, "\u{2192}\u{a0}\u{a0}\u{a0}a\u{b7}b");
        assert_eq!(shown.map, vec![0, 9, 10, 12, 13]);
        assert_eq!(shown.whitespace, vec![0..9, 10..12]);

        let controls = deleted_line_display("a\u{7}b\u{202e}", 4, &[], true);
        assert_eq!(controls.text, "a\u{2407}b[U+202E]");
        assert_eq!(controls.controls, vec![1..4, 5..13]);
        // Without `renderControlCharacters` they're themselves.
        assert_eq!(
            deleted_line_display("a\u{7}b", 4, &[], false).text,
            "a\u{7}b"
        );
    }

    /// Monaco `renderLines` with no selection: `selection` mode draws no
    /// whitespace, and `trailing` draws a view line's trailing whitespace.
    #[test]
    fn deleted_lines_whitespace_modes() {
        use crate::options::RenderWhitespace;
        let columns =
            |mode| crate::view::guides::whitespace_columns("a  b  ", 0, 0, false, mode, &[]);
        assert!(columns(RenderWhitespace::Selection).is_empty());
        assert_eq!(columns(RenderWhitespace::Trailing), vec![4, 5]);
        assert_eq!(columns(RenderWhitespace::Boundary), vec![1, 2, 4, 5]);
    }

    #[test]
    fn without_ranges_cuts_holes() {
        let style = HighlightStyle::default();
        let cut = without_ranges(vec![(0..10, style), (12..14, style)], &[2..3, 5..7, 13..20]);
        let ranges: Vec<_> = cut.into_iter().map(|(r, _)| r).collect();
        assert_eq!(ranges, vec![0..2, 3..5, 7..10, 12..13]);
    }

    /// The inner changes of the normalized diff, as Monaco prints ranges.
    fn normalized(original: &str, modified: &str) -> (Vec<String>, LinesDiff) {
        let original: Vec<&str> = original.split('\n').collect();
        let modified: Vec<&str> = modified.split('\n').collect();
        let options = DiffOptions {
            ignore_trim_whitespace: false,
            ..Default::default()
        };
        let diff = normalize_document_diff(
            compute_diff(&original, &modified, &options),
            &original,
            &modified,
        );
        let inner = diff
            .changes
            .iter()
            .flat_map(|c| &c.inner_changes)
            .map(|i| format!("{} {}", i.original_range, i.modified_range))
            .collect();
        (inner, diff)
    }

    #[test]
    fn normalizing_fills_whole_line_changes_to_the_next_line() {
        // Monaco `normalizeRangeMapping`: column 1 to the line end on both
        // sides takes the line break.
        let (inner, diff) = normalized("a\nfoo\nb", "a\nbar\nb");
        assert_eq!(inner, ["[2,1 -> 3,1] [2,1 -> 3,1]"]);
        // So the true inline view doesn't show it.
        assert!(!allows_true_inline_diff_rendering(&diff.changes[0]));
        // Two whole lines.
        let (inner, _) = normalized("a\nfoo\nfoo\nb", "a\nbar\nbar\nb");
        assert_eq!(inner, ["[2,1 -> 4,1] [2,1 -> 4,1]"]);
    }

    #[test]
    fn the_compared_move_is_found_by_its_modified_lines() {
        let moved = |original: usize, modified: usize| crate::diff::MovedText {
            line_range_mapping: LineRangeMapping::new(
                LineRange::new(original, original + 3),
                LineRange::new(modified, modified + 3),
            ),
            changes: Vec::new(),
        };
        // A move above the compared one (modified lines 20-22) appeared:
        // it's now the second.
        let diff = LinesDiff {
            moves: vec![moved(1, 40), moved(5, 21)],
            ..Default::default()
        };
        let compared = Some(LineRange::new(20, 23));
        assert_eq!(DiffEditor::find_compared_move(&diff, compared), Some(1));
        // Gone.
        let diff = LinesDiff {
            moves: vec![moved(1, 40)],
            ..Default::default()
        };
        assert_eq!(DiffEditor::find_compared_move(&diff, compared), None);
        assert_eq!(DiffEditor::find_compared_move(&diff, None), None);
    }

    #[test]
    fn normalizing_keeps_partial_and_last_line_changes() {
        // Not from column 1.
        let (inner, diff) = normalized("a\nlet x\nb", "a\nlet y\nb");
        assert_eq!(inner, ["[2,5 -> 2,6] [2,5 -> 2,6]"]);
        assert!(allows_true_inline_diff_rendering(&diff.changes[0]));
        // Not to the line end on both sides.
        let (inner, _) = normalized("a\nfoo x\nb", "a\nbar x\nb");
        assert_eq!(inner, ["[2,1 -> 2,4] [2,1 -> 2,4]"]);
        // On the last line there is no next line to end on.
        let (inner, _) = normalized("a\nfoo", "a\nbar");
        assert_eq!(inner, ["[2,1 -> 2,4] [2,1 -> 2,4]"]);
        // Both ranges empty at column 1 stay empty.
        let mapping = RangeMapping::new(
            crate::diff::Range::new(2, 1, 2, 1),
            crate::diff::Range::new(2, 1, 2, 1),
        );
        let diff = LinesDiff {
            changes: vec![DetailedLineRangeMapping::new(
                LineRange::new(2, 3),
                LineRange::new(2, 3),
                vec![mapping],
            )],
            ..Default::default()
        };
        let diff = normalize_document_diff(diff, &["a", "", "b"], &["a", "", "b"]);
        assert_eq!(diff.changes[0].inner_changes, [mapping]);
    }

    fn alignments(original: &str, modified: &str) -> Vec<(String, String, f32, f32)> {
        let original: Vec<&str> = original.split('\n').collect();
        let modified: Vec<&str> = modified.split('\n').collect();
        let diff = compute_diff(&original, &modified, &DiffOptions::default());
        let max = |line: usize| original.get(line.wrapping_sub(1)).map(|l| l.len() + 1);
        compute_range_alignment(&diff.changes, &[], &[], 10., 10., &max, true)
            .into_iter()
            .map(|a| {
                (
                    a.original_range.to_string(),
                    a.modified_range.to_string(),
                    a.original_height,
                    a.modified_height,
                )
            })
            .collect()
    }

    #[test]
    fn insertions_align_with_padding() {
        // Two lines inserted after line 1: 0px against 20px.
        assert_eq!(
            alignments("a\nb", "a\nx\ny\nb"),
            vec![("[2,2)".to_string(), "[2,4)".to_string(), 0., 20.)]
        );
    }

    #[test]
    fn wrapped_lines_outside_changes_align() {
        let original = ["a", "b", "c"];
        let modified = ["a", "b", "c"];
        let diff = compute_diff(&original, &modified, &DiffOptions::default());
        let max = |_: usize| Some(2);
        // Line 2 wraps once on the original side.
        let a = compute_range_alignment(&diff.changes, &[(2, 10.)], &[], 10., 10., &max, true);
        assert_eq!(a.len(), 1);
        assert_eq!((a[0].original_height, a[0].modified_height), (20., 10.));
    }

    #[test]
    fn sash_left_keeps_100px_per_side() {
        // Monaco `SashLayout._computeSashLeft`.
        assert_eq!(compute_sash_left(0.5, 0.5, true, px(1000.)), px(500.));
        assert_eq!(compute_sash_left(0.01, 0.5, true, px(1000.)), px(100.));
        assert_eq!(compute_sash_left(0.99, 0.5, true, px(1000.)), px(900.));
        // Not resizable: the default ratio.
        assert_eq!(compute_sash_left(0.2, 0.5, false, px(1000.)), px(500.));
        // Too narrow for the minimum: the midpoint.
        assert_eq!(compute_sash_left(0.1, 0.5, true, px(150.)), px(75.));
    }

    #[test]
    fn slider_like_monaco_scrollbar_state() {
        // A tenth visible: 60px slider, moving 540px over 5400px of scroll.
        assert_eq!(
            slider_geometry(px(600.), px(6000.), px(2700.)),
            (px(270.), px(60.))
        );
        // At least 20px.
        assert_eq!(slider_geometry(px(100.), px(100_000.), px(0.)).1, px(20.));
        // Nothing to scroll: the whole height.
        assert_eq!(
            slider_geometry(px(600.), px(300.), px(0.)),
            (px(0.), px(600.))
        );
    }

    #[test]
    fn edits_move_line_ranges() {
        let range = LineRange::new(5, 8);
        // A line inserted above.
        assert_eq!(
            apply_edits_to_line_range(range, &[(2, 2, 1)]),
            Some(LineRange::new(6, 9))
        );
        // A line inserted inside.
        assert_eq!(
            apply_edits_to_line_range(range, &[(6, 6, 1)]),
            Some(LineRange::new(5, 9))
        );
        // Below: no change.
        assert_eq!(
            apply_edits_to_line_range(range, &[(10, 10, 3)]),
            Some(range)
        );
        // Swallowed.
        assert_eq!(apply_edits_to_line_range(range, &[(3, 10, -7)]), None);
    }

    #[test]
    fn breadcrumbs_start_in_the_hidden_lines_and_end_after_them() {
        let symbol = |name: &str, start, end| BreadcrumbSymbol {
            name: SharedString::from(name.to_string()),
            kind: crate::providers::SymbolKind::Function,
            start_line_number: start,
            end_line_number: end,
        };
        let symbols = [
            symbol("inside", 3, 5),
            symbol("small", 8, 30),
            symbol("big", 2, 50),
            symbol("before", 1, 40),
        ];
        let names: Vec<_> = breadcrumb_items(&symbols, LineRange::new(2, 20))
            .into_iter()
            .map(|s| s.name.to_string())
            .collect();
        assert_eq!(names, ["big", "small"]);
    }

    #[test]
    fn staged_value_applies_the_inner_changes() {
        let original = text::Rope::from("a\nfoo\nb");
        let modified = text::Rope::from("a\nfoo bar\nb");
        let diff = compute_diff(
            &["a", "foo", "b"],
            &["a", "foo bar", "b"],
            &DiffOptions::default(),
        );
        assert_eq!(
            staged_value(&original, &modified, &diff.changes[0]),
            "a\nfoo bar\nb"
        );
    }
}
