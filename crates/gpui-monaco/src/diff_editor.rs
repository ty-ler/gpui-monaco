//! The side-by-side diff editor (Monaco `DiffEditorWidget`, MIT): the
//! original and modified models in two editors, with changes decorated,
//! padding view zones keeping the sides aligned, and scrolling in sync.
//!
//! Ports `diffEditorWidget.ts`, `diffEditorDecorations.ts`,
//! `diffEditorViewZones.ts` (`computeRangeAlignment`) and the options in
//! `diffEditorOptions.ts`.

use std::cell::Cell;
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

use crate::cursor::Selection;
use crate::decorations::{BlockDecoration, DecorationId, DecorationOptions, OverviewRulerLane};
use crate::diff::{
    DetailedLineRangeMapping, DiffOptions, LineRange, LineRangeMapping, LinesDiff, RangeMapping,
    compute_diff,
};
use crate::editor::{CodeEditor, CodeEditorEvent};
use crate::model::{TextModel, TextModelEvent};
use crate::theme::EditorColors;
use crate::widgets::{ViewZone, WidgetId, ZoneHeight};

mod accessible;
mod unchanged;

use accessible::AccessibleDiffViewer;

use unchanged::{RevealPreference, UnchangedRegion};

gpui::actions!(
    diff_editor,
    [
        /// Goes to the next change (Monaco `goToDiff('next')`, Alt+F5).
        GoToNextDiff,
        /// Goes to the previous change (Monaco `goToDiff('previous')`,
        /// Shift+Alt+F5).
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
    ]
);

/// Set on the diff editor's key context while comparing a moved block
/// (Monaco `comparingMovedCode`).
const COMPARING_MOVED_CODE: &str = "comparingMovedCode";

/// The key context of the diff editor.
pub const DIFF_EDITOR_KEY_CONTEXT: &str = "DiffEditor";

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
        gpui::KeyBinding::new("alt-f5", GoToNextDiff, Some(DIFF_EDITOR_KEY_CONTEXT)),
        gpui::KeyBinding::new(
            "shift-alt-f5",
            GoToPreviousDiff,
            Some(DIFF_EDITOR_KEY_CONTEXT),
        ),
    ]);
}

/// Monaco `IDiffEditorBaseOptions`.
#[derive(Clone, Debug, PartialEq)]
pub struct DiffEditorOptions {
    /// Let the user resize the split (default `true`).
    pub enable_split_view_resizing: bool,
    /// The original side's share of the width (default 0.5).
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
        }
    }
}

/// Events a [`DiffEditor`] emits.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DiffEditorEvent {
    /// A new diff is shown (Monaco `onDidUpdateDiff`).
    DiffUpdated,
}

/// Monaco's delay after edits before the diff is computed again.
const RECOMPUTE_DELAY: Duration = Duration::from_millis(200);

/// Monaco `ILineRangeAlignment`: line ranges of the two sides shown with
/// the same height.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct LineRangeAlignment {
    pub original_range: LineRange,
    pub modified_range: LineRange,
    pub original_height: f32,
    pub modified_height: f32,
    /// Whether it's part of a change.
    pub is_diff: bool,
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
            is_diff: false,
        });
    };

    for c in diffs {
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
                is_diff: true,
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

/// A moved block's caption (Monaco `MovedBlockOverlayWidget`), right
/// aligned.
struct MoveCaption {
    text: SharedString,
    color: Hsla,
    checked_background: Hsla,
    colors: EditorColors,
    /// Its block is being compared (the toggle is checked).
    comparing: bool,
    diff: WeakEntity<DiffEditor>,
    index: usize,
}

impl Render for MoveCaption {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let (diff, index, comparing) = (self.diff.clone(), self.index, self.comparing);
        div()
            .size_full()
            .flex()
            .justify_end()
            .items_center()
            .gap(px(8.))
            .pr(px(20.))
            .text_size(px(12.))
            .text_color(self.color.opacity(0.8))
            .child(self.text.clone())
            .child(
                div()
                    .id("compare-move")
                    .tooltip(crate::tooltip::tooltip("Compare", &self.colors))
                    .px_1()
                    .rounded(px(3.))
                    .cursor_pointer()
                    .when(comparing, |button| button.bg(self.checked_background))
                    .hover(|style| style.bg(self.checked_background))
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
                    .child("Compare"),
            )
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
            .child("→")
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
            .map_or(px(18.), |e| e.read(cx).options().resolved_line_height());
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

/// A hidden-lines bar's edge being dragged (Monaco `isDragged`).
#[derive(Clone, Copy, Debug)]
struct UnchangedDrag {
    index: usize,
    /// The top edge (else the bottom).
    top: bool,
    /// Where the drag started, in window coordinates.
    start_y: Pixels,
    /// The visible line count at the start.
    start: usize,
    /// It moved more than 2px, so it's not a click.
    moved: bool,
}

/// The height of a "hidden lines" bar (Monaco's 24px placeholder zone).
const UNCHANGED_BAR_HEIGHT: f32 = 24.;

/// Monaco `CollapsedCodeOverlayWidget`: the bar where unchanged lines are
/// hidden. Its top and bottom edges reveal more lines above and below; the
/// unfold button or a double click on the label shows them all.
struct UnchangedBar {
    diff: WeakEntity<DiffEditor>,
    /// The editor it's in, for the gutter width.
    editor: WeakEntity<CodeEditor>,
    index: usize,
    hidden_lines: usize,
}

impl Render for UnchangedBar {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let Some(editor) = self.editor.upgrade() else {
            return div();
        };
        let editor = editor.read(cx);
        let colors = editor.colors().clone();
        let gutter = editor
            .layout
            .as_ref()
            .map_or(px(0.), |l| l.text_bounds.left() - l.bounds.left());
        let font_family = editor.options().font_family.clone();
        let edge = |id: &'static str| {
            div()
                .id(id)
                .h(px(5.))
                .w_full()
                .flex_none()
                // Monaco's `n-resize`/`s-resize`/`ns-resize`: click or drag.
                .cursor(gpui::CursorStyle::ResizeUpDown)
                .hover(|style| style.bg(colors.focus_border))
        };
        let (diff, index) = (self.diff.clone(), self.index);
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
                    diff.update(cx, |diff, cx| diff.start_unchanged_drag(index, top, y, cx))
                });
            }
        };
        let (top, bottom) = (start_drag.clone(), start_drag);
        let shadow = colors.diff_unchanged_region_shadow;
        div()
            .size_full()
            .flex()
            .flex_col()
            .bg(colors.diff_unchanged_region_background)
            .text_color(colors.diff_unchanged_region_foreground)
            .text_size(px(13.))
            .child(
                edge("unchanged-top")
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
                div()
                    .flex_1()
                    .flex()
                    .flex_row()
                    .items_center()
                    .overflow_hidden()
                    .child(
                        div().w(gutter).flex_none().flex().justify_center().child(
                            div()
                                .id("unchanged-unfold")
                                .tooltip(crate::tooltip::tooltip("Show Unchanged Region", &colors))
                                .cursor_pointer()
                                .px_1()
                                .hover(|style| style.text_color(colors.focus_border))
                                .on_click(move |_, _, cx| unfold(cx, |r, _| r.show_all()))
                                .child("↕"),
                        ),
                    )
                    .child(
                        div()
                            .id("unchanged-label")
                            .tooltip(crate::tooltip::tooltip("Double click to unfold", &colors))
                            .font_family(font_family)
                            .whitespace_nowrap()
                            .on_click(move |event, _, cx| {
                                if event.click_count() == 2 {
                                    label(cx, |r, _| r.show_all())
                                }
                            })
                            .child(format!("{} hidden lines", self.hidden_lines)),
                    ),
            )
            .child(
                edge("unchanged-bottom")
                    .tooltip(crate::tooltip::tooltip(
                        "Click or drag to show more below",
                        &colors,
                    ))
                    .on_mouse_down(MouseButton::Left, move |e, _, cx| {
                        cx.stop_propagation();
                        bottom(cx, e.position.y, false)
                    }),
            )
            // Monaco's inset shadows at the top and bottom.
            .border_t_1()
            .border_b_1()
            .border_color(shadow.opacity(0.35))
    }
}

/// Deleted lines in the inline view (Monaco `renderLines` into a
/// `line-delete` view zone, with `inline-deleted-margin-view-zone`):
/// highlighted original text on the removed background, the changed
/// characters marked, and `−` in the margin.
struct DeletedCode {
    /// The modified editor, for its gutter, font and horizontal scroll.
    editor: WeakEntity<CodeEditor>,
    original: Entity<TextModel>,
    /// 0-based original rows.
    rows: Range<u32>,
    /// The changed characters, in original coordinates.
    inner_changes: Vec<Range<PointUtf16>>,
    render_indicators: bool,
    _subscription: Subscription,
}

impl DeletedCode {
    fn new(
        editor: WeakEntity<CodeEditor>,
        original: Entity<TextModel>,
        rows: Range<u32>,
        inner_changes: Vec<Range<PointUtf16>>,
        render_indicators: bool,
        cx: &mut Context<Self>,
    ) -> Self {
        // Monaco renders the zone again once tokenization completes.
        let subscription = cx.subscribe(&original, |_, _, event: &TextModelEvent, cx| {
            if matches!(event, TextModelEvent::SyntaxUpdated) {
                cx.notify();
            }
        });
        Self {
            editor,
            original,
            rows,
            inner_changes,
            render_indicators,
            _subscription: subscription,
        }
    }
}

impl Render for DeletedCode {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let Some(editor) = self.editor.upgrade() else {
            return div();
        };
        let editor = editor.read(cx);
        let options = editor.options();
        let colors = editor.colors().clone();
        let theme = editor.theme().clone();
        let line_height = options.resolved_line_height();
        let (gutter_width, text_left, sign_left, scroll_left) = match &editor.layout {
            Some(layout) => (
                layout.gutter_width,
                layout.text_bounds.left() - layout.bounds.left(),
                layout.glyph_margin_width + layout.line_numbers_width,
                layout.scroll_left,
            ),
            None => (px(0.), px(0.), px(0.), px(0.)),
        };
        let decorations_width = options.line_decorations_width;
        let font_family = options.font_family.clone();
        let font_size = options.font_size;
        let font_weight = options.font_weight;
        let model = self.original.read(cx);
        let rope = model.buffer().rope();
        let tab_size = model.options().tab_size.max(1) as usize;
        let rows = self.rows.start..self.rows.end.min(rope.line_count());
        let start = rope.point_to_offset(Point::new(rows.start, 0));
        let end = rope.point_to_offset(Point::new(rows.end.saturating_sub(1), u32::MAX));
        let tokens = crate::element::HighlightSource::new(model)
            .map(|source| source.highlights(&theme, start..end))
            .unwrap_or_default();
        let lines = rows.clone().map(|row| {
            let line = rope.line(row);
            let line_start = rope.point_to_offset(Point::new(row, 0));
            let line_end = line_start + line.len();
            let (text, map) = crate::colorize::expand_tabs(&line, tab_size);
            let syntax: Vec<_> = tokens
                .iter()
                .filter(|(range, _)| range.start < line_end && range.end > line_start)
                .map(|(range, style)| {
                    let range = map[range.start.max(line_start) - line_start]
                        ..map[range.end.min(line_end) - line_start];
                    (
                        range,
                        HighlightStyle {
                            color: style.foreground,
                            font_weight: style.bold.then_some(FontWeight::BOLD),
                            font_style: style.italic.then_some(FontStyle::Italic),
                            ..Default::default()
                        },
                    )
                })
                .filter(|(range, _)| !range.is_empty())
                .collect();
            let byte = |column: u32| {
                let mut units = 0;
                for (ix, ch) in line.char_indices() {
                    if units >= column {
                        return ix;
                    }
                    units += ch.len_utf16() as u32;
                }
                line.len()
            };
            let changed: Vec<_> = self
                .inner_changes
                .iter()
                .filter(|r| r.start.row <= row && r.end.row >= row)
                .map(|r| {
                    let from = if r.start.row == row {
                        byte(r.start.column)
                    } else {
                        0
                    };
                    let to = if r.end.row == row {
                        byte(r.end.column)
                    } else {
                        line.len()
                    };
                    (
                        map[from]..map[to],
                        HighlightStyle {
                            background_color: Some(colors.diff_removed_text_background),
                            ..Default::default()
                        },
                    )
                })
                .filter(|(range, _)| !range.is_empty())
                .collect();
            let highlights: Vec<_> = gpui::combine_highlights(syntax, changed).collect();
            let text = if text.is_empty() {
                " ".to_string()
            } else {
                text
            };
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
                        .when(self.render_indicators, |margin| {
                            margin.child(
                                div()
                                    .absolute()
                                    .left(sign_left)
                                    .w(decorations_width)
                                    .h_full()
                                    .flex()
                                    .justify_center()
                                    .text_color(colors.foreground)
                                    .child("−"),
                            )
                        }),
                )
                .child(
                    div()
                        .h_full()
                        .flex_1()
                        .overflow_hidden()
                        .bg(colors.diff_removed_line_background)
                        .child(
                            div()
                                .ml(-scroll_left)
                                .whitespace_nowrap()
                                .child(StyledText::new(text).with_highlights(highlights)),
                        ),
                )
        });
        div()
            .size_full()
            .overflow_hidden()
            .font_family(font_family)
            .text_size(font_size)
            .font_weight(font_weight)
            .line_height(line_height)
            .text_color(colors.foreground)
            .children(lines.collect::<Vec<_>>())
    }
}

/// The side-by-side diff editor (Monaco `DiffEditorWidget`).
pub struct DiffEditor {
    original: Entity<CodeEditor>,
    modified: Entity<CodeEditor>,
    options: DiffEditorOptions,
    diff: Option<Arc<LinesDiff>>,
    /// The model versions `diff` is for, and the computation running.
    diff_versions: Option<(text::Version, text::Version)>,
    diff_task: Option<Task<()>>,
    decorations: (Vec<DecorationId>, Vec<DecorationId>),
    zones: (Vec<WidgetId>, Vec<WidgetId>),
    /// The original side's share of the width.
    ratio: f32,
    /// The diff editor's bounds, for dragging the sash.
    bounds: Rc<Cell<Bounds<Pixels>>>,
    dragging_sash: bool,
    syncing_scroll: bool,
    /// Whether the last render showed the diff inline.
    inline: bool,
    /// Hideable unchanged regions, when `hide_unchanged_regions` is on.
    unchanged: Vec<UnchangedRegion>,
    /// Decorations on each region's lines, tracking them through edits.
    unchanged_decorations: (Vec<DecorationId>, Vec<DecorationId>),
    /// The "hidden lines" bars.
    unchanged_zones: (Vec<WidgetId>, Vec<WidgetId>),
    /// The mouse is over the gutter menu, which shows every button.
    gutter_hovered: bool,
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
        let mut this = Self {
            original: original_editor,
            modified: modified_editor,
            ratio: options.split_view_default_ratio,
            options,
            diff: None,
            diff_versions: None,
            diff_task: None,
            decorations: Default::default(),
            zones: Default::default(),
            bounds: Default::default(),
            dragging_sash: false,
            syncing_scroll: false,
            inline: false,
            unchanged: Vec::new(),
            unchanged_decorations: Default::default(),
            unchanged_zones: Default::default(),
            gutter_hovered: false,
            unchanged_drag: None,
            revert_arrows: Vec::new(),
            accessible_viewer: None,
            move_decorations: Default::default(),
            active_move: None,
            move_to_compare: None,
            focus_handle: cx.focus_handle(),
            _subscriptions: Vec::new(),
        };
        this.subscribe(cx);
        this.compute_diff(true, cx);
        this
    }

    fn subscribe(&mut self, cx: &mut Context<Self>) {
        let original_model = self.original.read(cx).model().clone();
        let modified_model = self.modified.read(cx).model().clone();
        self._subscriptions = vec![
            cx.subscribe(&original_model, |this, _, event: &TextModelEvent, cx| {
                if matches!(event, TextModelEvent::Edited { .. }) {
                    this.compute_diff(false, cx);
                }
            }),
            cx.subscribe(&modified_model, |this, _, event: &TextModelEvent, cx| {
                if matches!(event, TextModelEvent::Edited { .. }) {
                    this.compute_diff(false, cx);
                }
            }),
            cx.subscribe(
                &self.original,
                |this, _, event: &CodeEditorEvent, cx| match event {
                    CodeEditorEvent::ScrollChanged => this.sync_scroll(true, cx),
                    CodeEditorEvent::SelectionsChanged(_) => {
                        this.ensure_selections_visible(true, cx);
                        this.update_active_move(true, cx);
                    }
                    _ => {}
                },
            ),
            cx.subscribe(
                &self.modified,
                |this, _, event: &CodeEditorEvent, cx| match event {
                    CodeEditorEvent::ScrollChanged => {
                        this.sync_scroll(false, cx);
                        // The gutter menu follows the modified editor.
                        cx.notify();
                    }
                    CodeEditorEvent::SelectionsChanged(_) => {
                        this.ensure_selections_visible(false, cx);
                        this.update_active_move(false, cx);
                        this.apply_revert_arrows(cx);
                        cx.notify();
                    }
                    _ => {}
                },
            ),
        ];
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
    pub fn update_options(&mut self, options: DiffEditorOptions, cx: &mut Context<Self>) {
        let recompute = options.ignore_trim_whitespace != self.options.ignore_trim_whitespace
            || options.max_computation_time_ms != self.options.max_computation_time_ms
            || options.show_moves != self.options.show_moves
            || options.hide_unchanged_regions != self.options.hide_unchanged_regions;
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
        self.clear_diff(cx);
        self.original
            .update(cx, |editor, cx| editor.set_model(original, cx));
        self.modified
            .update(cx, |editor, cx| editor.set_model(modified, cx));
        self.subscribe(cx);
        self.compute_diff(true, cx);
    }

    /// Monaco `renderSideBySide` after `useInlineViewWhenSpaceIsLimited`:
    /// whether the sides show next to each other (else deleted lines show
    /// inline in the modified editor).
    pub fn renders_side_by_side(&self) -> bool {
        !self.inline
    }

    /// Whether `options` and the width call for side by side.
    fn side_by_side_for(&self, options: &DiffEditorOptions) -> bool {
        let width = self.bounds.get().size.width;
        options.render_side_by_side
            && !(options.use_inline_view_when_space_is_limited
                && width > px(0.)
                && width < options.render_side_by_side_inline_breakpoint)
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

    /// The latest diff (Monaco `getDiffComputationResult`), once computed.
    pub fn diff(&self) -> Option<&LinesDiff> {
        self.diff.as_deref()
    }

    /// Computes the diff again, after a pause in edits unless `now`.
    fn compute_diff(&mut self, now: bool, cx: &mut Context<Self>) {
        let original = self.original.read(cx).model().read(cx);
        let modified = self.modified.read(cx).model().read(cx);
        let versions = (original.buffer().version(), modified.buffer().version());
        let original_lines: Vec<String> = original.buffer().rope().lines().collect();
        let modified_lines: Vec<String> = modified.buffer().rope().lines().collect();
        let options = DiffOptions {
            ignore_trim_whitespace: self.options.ignore_trim_whitespace,
            max_computation_time_ms: self.options.max_computation_time_ms,
            compute_moves: self.options.show_moves,
            extend_to_subwords: false,
        };
        self.diff_task = Some(cx.spawn(async move |this, cx| {
            if !now {
                cx.background_executor().timer(RECOMPUTE_DELAY).await;
            }
            let diff = cx
                .background_executor()
                .spawn(async move { compute_diff(&original_lines, &modified_lines, &options) })
                .await;
            this.update(cx, |this, cx| {
                this.diff = Some(Arc::new(diff));
                this.diff_versions = Some(versions);
                this.diff_task = None;
                this.apply_diff(cx);
                cx.emit(DiffEditorEvent::DiffUpdated);
                cx.notify();
            })
            .ok();
        }));
    }

    fn clear_diff(&mut self, cx: &mut Context<Self>) {
        self.diff = None;
        self.diff_task = None;
        let (original, modified) = std::mem::take(&mut self.decorations);
        let (original_zones, modified_zones) = std::mem::take(&mut self.zones);
        clear_side(&self.original, original, original_zones, cx);
        clear_side(&self.modified, modified, modified_zones, cx);
        self.unchanged.clear();
        self.apply_unchanged(cx);
        self.active_move = None;
        self.move_to_compare = None;
        self.apply_move_decorations(cx);
    }

    /// Decorates both sides and aligns them with view zones.
    fn apply_diff(&mut self, cx: &mut Context<Self>) {
        let Some(diff) = self.diff.clone() else {
            return;
        };
        let colors = self.modified.read(cx).colors().clone();
        // A new diff may not have the compared move any more.
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
        let (original_decorations, modified_decorations) =
            diff_decorations(changes, &self.options, &colors);
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
        self.apply_alignment(&diff, &colors, cx);
        self.apply_move_decorations(cx);
        self.apply_revert_arrows(cx);
        self.update_unchanged_regions(&diff, cx);
        if let Some(viewer) = self.accessible_viewer.clone() {
            let mappings = diff_mappings(&diff);
            viewer.update(cx, |viewer, cx| viewer.set_diffs(&mappings, cx));
        }
    }

    /// Monaco `DiffEditorViewZones`: padding where one side has fewer
    /// lines, so changes line up.
    fn apply_alignment(&mut self, diff: &LinesDiff, colors: &EditorColors, cx: &mut Context<Self>) {
        let (old_original, old_modified) = std::mem::take(&mut self.zones);
        for id in old_original {
            self.original
                .update(cx, |editor, cx| editor.remove_widget(id, cx));
        }
        for id in old_modified {
            self.modified
                .update(cx, |editor, cx| editor.remove_widget(id, cx));
        }
        if self.inline {
            self.apply_inline_zones(diff, cx);
            return;
        }
        let original_line_height =
            f32::from(self.original.read(cx).options().resolved_line_height());
        let modified_line_height =
            f32::from(self.modified.read(cx).options().resolved_line_height());
        let mut original_extra = self.original.read(cx).wrap_extra_heights();
        let mut modified_extra = self.modified.read(cx).wrap_extra_heights();
        // The moved blocks' captions count like any other view zone.
        let moves = self.shown_moves(diff);
        for m in moves {
            original_extra.push((
                m.line_range_mapping.original.start_line_number - 1,
                MOVE_CAPTION_HEIGHT,
            ));
            modified_extra.push((
                m.line_range_mapping.modified.start_line_number - 1,
                MOVE_CAPTION_HEIGHT,
            ));
        }
        merge_extra_heights(&mut original_extra);
        merge_extra_heights(&mut modified_extra);
        let original_rope = self
            .original
            .read(cx)
            .model()
            .read(cx)
            .buffer()
            .rope()
            .clone();
        let max_column = |line: usize| {
            (line >= 1 && line <= original_rope.line_count() as usize)
                .then(|| original_rope.line_len_utf16(line as u32 - 1) as usize + 1)
        };
        let alignments = compute_range_alignment(
            &diff.changes,
            &original_extra,
            &modified_extra,
            original_line_height,
            modified_line_height,
            &max_column,
            true,
        );
        let fill = colors.diff_diagonal_fill;
        let arrows = self.shows_margin_revert_arrows(cx);
        let this = cx.entity().downgrade();
        let mut zones = (Vec::new(), Vec::new());
        for a in alignments {
            let delta = a.modified_height - a.original_height;
            let (editor, after, height, ids) = if delta > 0. {
                (
                    &self.original,
                    a.original_range.end_line_number_exclusive - 1,
                    delta,
                    &mut zones.0,
                )
            } else if delta < 0. {
                (
                    &self.modified,
                    a.modified_range.end_line_number_exclusive - 1,
                    -delta,
                    &mut zones.1,
                )
            } else {
                continue;
            };
            // Monaco `createViewZoneMarginArrow`: a deletion's padding gets
            // a revert arrow in its margin.
            let deletion = (delta < 0. && arrows)
                .then(|| {
                    diff.changes.iter().find(|c| {
                        c.modified.is_empty() && c.original == a.original_range && a.is_diff
                    })
                })
                .flatten();
            let id = if let Some(change) = deletion {
                let view = cx.new(|cx| DeletionPadding {
                    fill: cx.new(|_| DiagonalFill { color: fill }),
                    arrow: RevertArrow {
                        diff: this.clone(),
                        revert: GutterRevert::Block(change.line_range_mapping()),
                        colors: colors.clone(),
                    },
                    editor: self.modified.downgrade(),
                });
                editor.update(cx, |editor, cx| {
                    editor.add_zone(
                        after as u32,
                        ZoneHeight::Pixels(px(height)),
                        view.into(),
                        true,
                        cx,
                    )
                })
            } else {
                let view = cx.new(|_| DiagonalFill { color: fill });
                editor.update(cx, |editor, cx| {
                    editor.add_view_zone(
                        ViewZone {
                            after_line: after as u32,
                            height: ZoneHeight::Pixels(px(height)),
                            view: view.into(),
                        },
                        cx,
                    )
                })
            };
            ids.push(id);
        }
        // Monaco `MovedBlockOverlayWidget`: "Code moved to line …" above
        // each moved block, on both sides.
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
                let color = colors.foreground;
                let checked_background = colors.widget_border;
                let diff_editor = cx.entity().downgrade();
                let view = cx.new(|_| MoveCaption {
                    text: text.into(),
                    color,
                    checked_background,
                    colors: colors.clone(),
                    comparing,
                    diff: diff_editor,
                    index: move_index,
                });
                let id = editor.update(cx, |editor, cx| {
                    editor.add_view_zone(
                        ViewZone {
                            after_line: after as u32,
                            height: ZoneHeight::Pixels(px(MOVE_CAPTION_HEIGHT)),
                            view: view.into(),
                        },
                        cx,
                    )
                });
                ids.push(id);
            }
        }
        self.zones = zones;
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
    /// cursor of the editor whose cursor moved last.
    fn update_active_move(&mut self, original: bool, cx: &mut Context<Self>) {
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
        if active != self.active_move {
            self.active_move = active;
            self.apply_move_decorations(cx);
            cx.notify();
        }
    }

    /// Monaco `MovedBlocksLinesFeature`: a column between the sides with a
    /// line from each moved block to where it went.
    fn render_move_lines(&self, cx: &App) -> Option<gpui::AnyElement> {
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
        let width = (track_count - 1) as f32 * 10. + padding * 2. + MOVE_ARROW_WIDTH;
        let colors = modified.colors().clone();
        let background = colors.background;
        Some(
            canvas(
                |_, _, _| {},
                move |bounds, _, window, _| {
                    window.with_content_mask(Some(gpui::ContentMask { bounds }), |window| {
                        for ((from, to, active), track) in lines.iter().zip(&tracks) {
                            let color = if *active {
                                colors.diff_move_active_border
                            } else {
                                colors.diff_move_border
                            };
                            let x = |x: f32| bounds.left() + px(x);
                            let y = |y: Pixels| bounds.top() + y;
                            let vertical = padding + *track as f32 * 10.;
                            let right = f32::from(bounds.size.width);
                            let mut path = PathBuilder::stroke(px(2.));
                            path.move_to(point(x(0.), y(*from)));
                            path.line_to(point(x(vertical), y(*from)));
                            path.line_to(point(x(vertical), y(*to)));
                            path.line_to(point(x(right - MOVE_ARROW_WIDTH), y(*to)));
                            if let Ok(path) = path.build() {
                                window.paint_path(path, color);
                            }
                            let half = px(MOVE_ARROW_WIDTH / 2.);
                            let mut arrow = PathBuilder::fill();
                            arrow.move_to(point(x(right - MOVE_ARROW_WIDTH), y(*to) - half));
                            arrow.line_to(point(x(right), y(*to)));
                            arrow.line_to(point(x(right - MOVE_ARROW_WIDTH), y(*to) + half));
                            arrow.close();
                            if let Ok(arrow) = arrow.build() {
                                window.paint_path(arrow, color);
                            }
                        }
                    });
                },
            )
            .h_full()
            .w(px(width))
            .flex_none()
            .bg(background)
            .into_any_element(),
        )
    }

    /// Monaco's inline view (`renderSideBySide: false`): each change's
    /// deleted lines show in a view zone above its modified lines.
    fn apply_inline_zones(&mut self, diff: &LinesDiff, cx: &mut Context<Self>) {
        let line_height = self.modified.read(cx).options().resolved_line_height();
        let original_model = self.original.read(cx).model().clone();
        let line_count = original_model.read(cx).buffer().rope().line_count() as usize;
        let mut ids = Vec::new();
        for change in &diff.changes {
            let range = change.original;
            // A diff for older text can be out of bounds until recomputed.
            if range.is_empty() || range.end_line_number_exclusive - 1 > line_count {
                continue;
            }
            let rows =
                range.start_line_number as u32 - 1..range.end_line_number_exclusive as u32 - 1;
            let inner_changes = change
                .inner_changes
                .iter()
                .map(|i| i.original_range.to_point_utf16_range())
                .collect();
            let editor = self.modified.downgrade();
            let render_indicators = self.options.render_indicators;
            let model = original_model.clone();
            let view = cx.new(|cx| {
                DeletedCode::new(
                    editor,
                    model,
                    rows.clone(),
                    inner_changes,
                    render_indicators,
                    cx,
                )
            });
            let id = self.modified.update(cx, |editor, cx| {
                editor.add_zone(
                    change.modified.start_line_number as u32 - 1,
                    ZoneHeight::Pixels(line_height * rows.len() as f32),
                    view.into(),
                    true,
                    cx,
                )
            });
            ids.push(id);
        }
        self.zones = (Vec::new(), ids);
    }

    /// Monaco `updateUnchangedRegions`: the hideable regions of a new diff,
    /// keeping what the user revealed.
    fn update_unchanged_regions(&mut self, diff: &LinesDiff, cx: &mut Context<Self>) {
        let settings = self.options.hide_unchanged_regions;
        let original_count = self.original.read(cx).model().read(cx).line_count() as usize;
        let modified_count = self.modified.read(cx).model().read(cx).line_count() as usize;
        let regions = if settings.enabled {
            let regions = UnchangedRegion::from_diffs(
                &diff.changes,
                original_count,
                modified_count,
                settings.minimum_line_count,
                settings.context_line_count,
            );
            let previous = self.tracked_unchanged_regions(cx);
            let regions =
                unchanged::transfer_state(regions, &previous, original_count, modified_count);
            // Nothing changed: Monaco shows everything.
            if regions.len() == 1
                && regions[0].modified_line_number == 1
                && regions[0].line_count == modified_count
            {
                Vec::new()
            } else {
                regions
            }
        } else {
            Vec::new()
        };
        self.unchanged = regions;
        self.apply_unchanged(cx);
    }

    /// The regions with their ranges moved through edits since they were
    /// shown (Monaco tracks them with decorations).
    fn tracked_unchanged_regions(&self, cx: &App) -> Vec<UnchangedRegion> {
        let rows = |editor: &Entity<CodeEditor>, id: DecorationId| {
            let model = editor.read(cx).model().read(cx);
            let range = model.decoration(id)?.range.clone();
            let rope = model.buffer().rope();
            Some(
                rope.offset_to_point(range.start).row as usize + 1
                    ..rope.offset_to_point(range.end).row as usize + 2,
            )
        };
        self.unchanged
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
            .collect()
    }

    /// Hides the regions' hidden lines and shows a bar in their place.
    fn apply_unchanged(&mut self, cx: &mut Context<Self>) {
        let colors = self.modified.read(cx).colors().clone();
        // The tracking decorations, which also tint the lines (Monaco
        // `diff-unchanged-lines`).
        let line = DecorationOptions {
            background: Some(colors.diff_unchanged_code_background),
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
            .unchanged
            .iter()
            .map(|r| r.original_unchanged_range())
            .collect();
        let modified_ranges = self
            .unchanged
            .iter()
            .map(|r| r.modified_unchanged_range())
            .collect();
        self.unchanged_decorations = (
            decorate(&self.original, old_original, original_ranges, cx),
            decorate(&self.modified, old_modified, modified_ranges, cx),
        );
        let original_hidden = self
            .unchanged
            .iter()
            .map(|r| r.hidden_original_range().to_rows())
            .filter(|r| !r.is_empty())
            .collect();
        let modified_hidden = self
            .unchanged
            .iter()
            .map(|r| r.hidden_modified_range().to_rows())
            .filter(|r| !r.is_empty())
            .collect();
        self.original.update(cx, |editor, cx| {
            editor.set_hidden_areas(original_hidden, cx)
        });
        self.modified.update(cx, |editor, cx| {
            editor.set_hidden_areas(modified_hidden, cx)
        });

        let (old_original, old_modified) = std::mem::take(&mut self.unchanged_zones);
        clear_side(&self.original, Vec::new(), old_original, cx);
        clear_side(&self.modified, Vec::new(), old_modified, cx);
        let this = cx.entity().downgrade();
        let mut zones = (Vec::new(), Vec::new());
        for (index, region) in self.unchanged.iter().enumerate() {
            if region.should_hide_controls() {
                continue;
            }
            let sides = [
                (&self.original, region.hidden_original_range(), &mut zones.0),
                (&self.modified, region.hidden_modified_range(), &mut zones.1),
            ];
            for (side, (editor, hidden, ids)) in sides.into_iter().enumerate() {
                // Inline, the original editor isn't shown.
                if side == 0 && self.inline {
                    continue;
                }
                let bar = cx.new(|_| UnchangedBar {
                    diff: this.clone(),
                    editor: editor.downgrade(),
                    index,
                    hidden_lines: hidden.len(),
                });
                let id = editor.update(cx, |editor, cx| {
                    editor.add_zone(
                        hidden.start_line_number as u32 - 1,
                        ZoneHeight::Pixels(px(UNCHANGED_BAR_HEIGHT)),
                        bar.into(),
                        true,
                        cx,
                    )
                });
                ids.push(id);
            }
        }
        self.unchanged_zones = zones;
        cx.notify();
    }

    /// Changes region `index` and shows the result.
    fn update_unchanged_region(
        &mut self,
        index: usize,
        change: impl FnOnce(&mut UnchangedRegion),
        cx: &mut Context<Self>,
    ) {
        if let Some(region) = self.unchanged.get_mut(index) {
            change(region);
            self.apply_unchanged(cx);
        }
    }

    /// Monaco's mouse down on a bar's top or bottom edge: a drag starts
    /// at window y `y`.
    fn start_unchanged_drag(&mut self, index: usize, top: bool, y: Pixels, cx: &mut Context<Self>) {
        let Some(region) = self.unchanged.get(index) else {
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
        let line_height = f32::from(self.modified.read(cx).options().resolved_line_height());
        let lines = (f32::from(delta) / line_height).round() as isize;
        let (index, top, start) = (drag.index, drag.top, drag.start as isize);
        let Some(region) = self.unchanged.get_mut(index) else {
            return;
        };
        let changed = if top {
            let value = (start + lines).clamp(0, region.max_visible_top() as isize) as usize;
            std::mem::replace(&mut region.visible_top, value) != value
        } else {
            let value = (start - lines).clamp(0, region.max_visible_bottom() as isize) as usize;
            std::mem::replace(&mut region.visible_bottom, value) != value
        };
        if changed {
            self.apply_unchanged(cx);
        }
    }

    /// Monaco's mouse up: a click without dragging shows
    /// `reveal_line_count` more lines.
    fn end_unchanged_drag(&mut self, cx: &mut Context<Self>) {
        let Some(drag) = self.unchanged_drag.take() else {
            return;
        };
        if !drag.moved {
            let count = self.options.hide_unchanged_regions.reveal_line_count;
            self.update_unchanged_region(
                drag.index,
                |r| {
                    if drag.top {
                        r.show_more_above(count)
                    } else {
                        r.show_more_below(count)
                    }
                },
                cx,
            );
        }
        cx.notify();
    }

    /// Monaco `ensureModifiedLineIsVisible`/`ensureOriginalLineIsVisible`
    /// for each selection's ends: the cursor never sits in hidden lines.
    fn ensure_selections_visible(&mut self, original: bool, cx: &mut Context<Self>) {
        if self.unchanged.is_empty() {
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
            for region in &mut self.unchanged {
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
        self.unchanged
            .iter_mut()
            .for_each(UnchangedRegion::collapse_all);
        self.apply_unchanged(cx);
    }

    /// Shows every unchanged region (Monaco `showAllUnchangedRegions`).
    pub fn show_all_unchanged_regions(&mut self, cx: &mut Context<Self>) {
        self.unchanged
            .iter_mut()
            .for_each(UnchangedRegion::show_all);
        self.apply_unchanged(cx);
    }

    /// The modified editor's hidden unchanged lines (1-based, end
    /// exclusive), for tests.
    #[doc(hidden)]
    pub fn hidden_modified_ranges_for_tests(&self) -> Vec<Range<usize>> {
        self.unchanged
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
        self.start_unchanged_drag(index, top, from, cx);
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
        let arrows: Vec<(u32, GutterRevert)> = self
            .gutter_items(cx)
            .into_iter()
            .filter_map(|item| match item.revert {
                GutterRevert::Selection(_) => Some((selection_line?, item.revert)),
                // Changes with modified lines (deletions get theirs in
                // their padding's margin).
                GutterRevert::Block(_) if !item.lines.is_empty() => {
                    Some((item.lines.to_rows().start, item.revert))
                }
                GutterRevert::Block(_) => None,
            })
            .collect();
        let mut ids = Vec::new();
        for (row, revert) in arrows {
            let view = cx.new(|_| RevertArrow {
                diff: this.clone(),
                revert,
                colors: colors.clone(),
            });
            ids.push(self.modified.update(cx, |editor, cx| {
                editor.add_glyph_margin_widget(
                    crate::widgets::GlyphMarginWidget {
                        row,
                        view: view.into(),
                    },
                    cx,
                )
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
            editor.execute_edits(edits, None, cx);
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

    /// Monaco `revertFocusedRangeMappings`: reverts the changes touching
    /// the modified editor's selection, when it has focus.
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
        let selected = LineRange::new(
            selection.start().row as usize + 1,
            selection.end().row as usize + 2,
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
            .filter(|c| {
                c.modified
                    .intersect(&selected)
                    .is_some_and(|r| !r.is_empty())
            })
            .map(|c| revert_points(&original, &modified, &c.line_range_mapping()))
            .collect();
        self.revert_edits(edits, cx);
    }

    /// Monaco `DiffEditorGutter`'s items: a "Revert Selection" item when
    /// the selection touches changed characters, else one per change.
    fn gutter_items(&self, cx: &App) -> Vec<GutterItem> {
        let Some(diff) = self.diff.as_ref() else {
            return Vec::new();
        };
        let editor = self.modified.read(cx);
        let selections = editor.selections();
        if selections.iter().any(|s| !s.is_empty()) {
            let selected_lines: Vec<LineRange> = selections
                .iter()
                .map(|s| LineRange::new(s.start().row as usize + 1, s.end().row as usize + 2))
                .collect();
            let intersects =
                |a: &Range<PointUtf16>, b: &Range<PointUtf16>| a.start <= b.end && b.start <= a.end;
            let selected: Vec<RangeMapping> = diff
                .changes
                .iter()
                .filter(|c| {
                    selected_lines
                        .iter()
                        .any(|l| c.modified.intersect(l).is_some_and(|r| !r.is_empty()))
                })
                .flat_map(|c| &c.inner_changes)
                .filter(|i| {
                    let range = i.modified_range.to_point_utf16_range();
                    selections
                        .iter()
                        .any(|s| intersects(&range, &(s.start()..s.end())))
                })
                .cloned()
                .collect();
            if !selected.is_empty() {
                let lines = |r: &RangeMapping| {
                    let range = r.modified_range.to_point_utf16_range();
                    range.start.row as usize + 1..range.end.row as usize + 2
                };
                let start = selected.iter().map(|c| lines(c).start).min().unwrap_or(1);
                let end = selected.iter().map(|c| lines(c).end).max().unwrap_or(1);
                return vec![GutterItem {
                    lines: LineRange::new(start, end),
                    revert: GutterRevert::Selection(selected),
                    show_always: true,
                }];
            }
        }
        let cursor_line = selections[0].head.row as usize + 1;
        diff.changes
            .iter()
            .map(|c| GutterItem {
                lines: c.modified,
                revert: GutterRevert::Block(c.line_range_mapping()),
                show_always: c.modified.contains(cursor_line),
            })
            .collect()
    }

    /// Monaco `DiffEditorGutter`: a 35px column left of the modified editor.
    fn render_gutter(&self, cx: &mut Context<Self>) -> Option<gpui::AnyElement> {
        let editor = self.modified.read(cx);
        if !self.options.render_gutter_menu || editor.options().read_only {
            return None;
        }
        let colors = editor.colors().clone();
        let viewport = editor
            .layout
            .as_ref()
            .map_or(px(0.), |l| l.bounds.size.height);
        let icon = if self.inline { "↺" } else { "→" };
        let hovered = self.gutter_hovered;
        let mut items = Vec::new();
        for (ix, item) in self.gutter_items(cx).into_iter().enumerate() {
            let editor = self.modified.read(cx);
            let Some((top, bottom)) = editor.rows_y_span(item.lines.to_rows()) else {
                continue;
            };
            if bottom < px(0.) || top > viewport {
                continue;
            }
            let button_top = gutter_button_top(top, bottom, viewport, px(GUTTER_BUTTON_HEIGHT));
            let visible = item.show_always || hovered;
            // Monaco's hunk toolbar titles.
            let title: SharedString = match item.revert {
                GutterRevert::Block(_) => "Revert Block".into(),
                GutterRevert::Selection(_) => "Revert Selection".into(),
            };
            let revert = item.revert;
            items.push(
                div()
                    .absolute()
                    .top(top)
                    .h(bottom - top)
                    .w_full()
                    .when(visible, |item| {
                        item.child(
                            // Monaco's `.background`: a line along the change.
                            div()
                                .absolute()
                                .left(px(GUTTER_WIDTH / 2. - 1.))
                                .h_full()
                                .w(px(2.))
                                .bg(colors.menu_separator_background),
                        )
                        .child(
                            div()
                                .id(("diff-revert", ix))
                                .tooltip(crate::tooltip::tooltip(title.clone(), &colors))
                                .absolute()
                                .top(button_top - top)
                                .left(px((GUTTER_WIDTH - 22.) / 2.))
                                .w(px(22.))
                                .h(px(GUTTER_BUTTON_HEIGHT))
                                .rounded(px(4.))
                                .bg(colors.gutter_item_background)
                                .text_color(colors.gutter_item_glyph_foreground)
                                .flex()
                                .items_center()
                                .justify_center()
                                .cursor_pointer()
                                .hover(|style| style.opacity(0.8))
                                .on_click(cx.listener(move |this, _, _, cx| match &revert {
                                    GutterRevert::Block(change) => this.revert(change, cx),
                                    GutterRevert::Selection(changes) => {
                                        this.revert_range_mappings(changes, cx)
                                    }
                                }))
                                .child(icon),
                        )
                    }),
            );
        }
        Some(
            div()
                .id("diff-gutter")
                .relative()
                .h_full()
                .w(px(GUTTER_WIDTH))
                .flex_none()
                .overflow_hidden()
                .bg(colors.background)
                .on_hover(cx.listener(|this, hovered: &bool, _, cx| {
                    this.gutter_hovered = *hovered;
                    cx.notify();
                }))
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
            let mappings = self.diff.as_deref().map(diff_mappings).unwrap_or_default();
            self.accessible_viewer = Some(cx.new(|cx| {
                let mut viewer = AccessibleDiffViewer::new(this, original, modified, can_close, cx);
                viewer.set_diffs(&mappings, cx);
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
        if self.syncing_scroll || self.inline {
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
            let layout_scroll = editor.layout.as_ref()?.scroll_top;
            editor
                .rows_y_span(start..start + 1)
                .map(|(top, _)| top + layout_scroll)
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

    /// Monaco `goToDiff`: moves the modified editor's cursor to the next or
    /// previous change and reveals it.
    pub fn go_to_diff(&mut self, next: bool, window: &mut Window, cx: &mut Context<Self>) {
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
        window.focus(&self.modified.focus_handle(cx), cx);
    }
}

impl CodeEditor {
    /// The vertical span of model `rows` from the editor's top, as of the
    /// last layout (Monaco `getTopForLineNumber`/`getBottomForLineNumber`).
    /// Empty `rows` span the gap before their row, with its view zones.
    pub(crate) fn rows_y_span(&self, rows: Range<u32>) -> Option<(Pixels, Pixels)> {
        let layout = self.layout.as_ref()?;
        let line_height = layout.metrics.line_height;
        let zones = &layout.zones;
        let y = |line: u32| zones.line_y(line, line_height) - layout.scroll_top;
        let first = self.view_lines.first_view_line_of_row(rows.start);
        if rows.is_empty() {
            let top = if first == 0 {
                -layout.scroll_top
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
        let line_height = f32::from(self.options.resolved_line_height());
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

/// A decoration on one side of the diff.
type SideDecoration = (Side, DecorationOptions);

/// Monaco `DiffEditorDecorations`: line backgrounds and gutter signs for
/// changed lines, and text backgrounds for the changed characters.
fn diff_decorations(
    changes: &[DetailedLineRangeMapping],
    options: &DiffEditorOptions,
    colors: &EditorColors,
) -> (Vec<SideDecoration>, Vec<SideDecoration>) {
    let line = |inserted: bool| {
        let (background, gutter, sign) = if inserted {
            (
                colors.diff_inserted_line_background,
                colors.diff_gutter_inserted_line_background,
                "+",
            )
        } else {
            (
                colors.diff_removed_line_background,
                colors.diff_gutter_removed_line_background,
                "−",
            )
        };
        DecorationOptions {
            background: Some(background),
            is_whole_line: true,
            margin_background: Some(gutter),
            lines_decoration_text: options
                .render_indicators
                .then(|| (SharedString::from(sign), colors.foreground)),
            overview_ruler: options.render_overview_ruler.then(|| {
                let color = if inserted {
                    colors.diff_inserted_text_background
                } else {
                    colors.diff_removed_text_background
                };
                (
                    color.opacity(1.0f32.min(color.a * 3.)),
                    OverviewRulerLane::Full,
                )
            }),
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
            is_whole_line: whole_line,
            ..Default::default()
        }
    };
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
            for i in &m.inner_changes {
                // Monaco skips empty markers outside the line range.
                if m.original.contains(i.original_range.start_line_number)
                    && !i.original_range.is_empty()
                {
                    original.push((
                        Side::Text(i.original_range.to_point_utf16_range()),
                        text(false, false),
                    ));
                }
                if m.modified.contains(i.modified_range.start_line_number)
                    && !i.modified_range.is_empty()
                {
                    modified.push((
                        Side::Text(i.modified_range.to_point_utf16_range()),
                        text(true, false),
                    ));
                }
            }
        }
    }
    (original, modified)
}

impl Render for DiffEditor {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // Monaco `useInlineViewWhenSpaceIsLimited`: the width decides.
        let inline = !self.side_by_side_for(&self.options);
        if inline != self.inline {
            self.inline = inline;
            self.apply_diff(cx);
        }
        let gutter = self.render_gutter(cx);
        // A bar edge drag follows the mouse anywhere in the window.
        let drag_listener = self.unchanged_drag.is_some().then(|| {
            let this = cx.entity().downgrade();
            canvas(
                |_, _, _| {},
                move |_, _, window, _| {
                    let moves = this.clone();
                    window.on_mouse_event(move |event: &gpui::MouseMoveEvent, phase, _, cx| {
                        if phase != gpui::DispatchPhase::Capture {
                            return;
                        }
                        let y = event.position.y;
                        let pressed = event.pressed_button == Some(MouseButton::Left);
                        moves
                            .update(cx, |this, cx| {
                                if pressed {
                                    this.drag_unchanged(y, cx)
                                } else {
                                    this.end_unchanged_drag(cx)
                                }
                            })
                            .ok();
                    });
                    let ups = this.clone();
                    window.on_mouse_event(move |_: &gpui::MouseUpEvent, phase, _, cx| {
                        if phase == gpui::DispatchPhase::Capture {
                            ups.update(cx, |this, cx| this.end_unchanged_drag(cx)).ok();
                        }
                    });
                },
            )
            .absolute()
            .size_0()
        });
        let move_lines = (!self.inline).then(|| self.render_move_lines(cx)).flatten();
        let bounds = self.bounds.clone();
        let ratio = self.ratio.clamp(0.1, 0.9);
        let border = self.modified.read(cx).colors().widget_border;
        let resizable = self.options.enable_split_view_resizing;
        div()
            .key_context({
                let mut context = gpui::KeyContext::new_with_defaults();
                context.add(DIFF_EDITOR_KEY_CONTEXT);
                if self.move_to_compare.is_some() {
                    context.add(COMPARING_MOVED_CODE);
                }
                context
            })
            .on_action(
                cx.listener(|this, _: &ExitCompareMove, _, cx| this.set_move_to_compare(None, cx)),
            )
            .track_focus(&self.focus_handle)
            .size_full()
            .relative()
            .flex()
            .flex_row()
            .on_action(
                cx.listener(|this, _: &GoToNextDiff, window, cx| this.go_to_diff(true, window, cx)),
            )
            .on_action(cx.listener(|this, _: &GoToPreviousDiff, window, cx| {
                this.go_to_diff(false, window, cx)
            }))
            .on_action(cx.listener(|this, _: &CollapseAllUnchangedRegions, _, cx| {
                this.collapse_all_unchanged_regions(cx)
            }))
            .on_action(cx.listener(|this, _: &ShowAllUnchangedRegions, _, cx| {
                this.show_all_unchanged_regions(cx)
            }))
            .on_action(cx.listener(|this, _: &Revert, window, cx| {
                this.revert_focused_range_mappings(window, cx)
            }))
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
            // Dragging the sash resizes the split.
            .on_mouse_move(
                cx.listener(move |this, event: &gpui::MouseMoveEvent, _, cx| {
                    if !this.dragging_sash {
                        return;
                    }
                    if event.pressed_button != Some(MouseButton::Left) {
                        this.dragging_sash = false;
                        return;
                    }
                    let bounds = this.bounds.get();
                    if bounds.size.width > px(0.) {
                        this.ratio = ((event.position.x - bounds.left()) / bounds.size.width)
                            .clamp(0.1, 0.9);
                        cx.notify();
                    }
                }),
            )
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, _: &gpui::MouseUpEvent, _, _| this.dragging_sash = false),
            )
            .child(
                canvas(move |b, _, _| bounds.set(b), |_, _, _, _| {})
                    .absolute()
                    .size_full(),
            )
            .when(!inline, |this| {
                this.child(
                    div()
                        .h_full()
                        .w(gpui::relative(ratio))
                        .child(self.original.clone()),
                )
                .child(
                    div()
                        .id("diff-sash")
                        .h_full()
                        .w(px(1.))
                        .flex_none()
                        .bg(border)
                        .when(resizable, |sash| {
                            sash.cursor_col_resize().on_mouse_down(
                                MouseButton::Left,
                                cx.listener(|this, _, _, cx| {
                                    this.dragging_sash = true;
                                    cx.stop_propagation();
                                }),
                            )
                        }),
                )
            })
            .children(move_lines)
            .children(gutter)
            .child(div().h_full().flex_1().child(self.modified.clone()))
            .children(drag_listener)
            // Monaco hides both editors behind the viewer.
            .children(self.accessible_viewer.clone())
    }
}

use gpui::prelude::FluentBuilder as _;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diff::compute_diff;

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
}
