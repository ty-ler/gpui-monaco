//! The long-distance hint: while the inline edit is scrolled away (or in
//! another file), a box near the cursor previews its line and offers to
//! jump there.
//!
//! Ported from Monaco / VS Code (MIT License, Copyright (c) Microsoft
//! Corporation), `vs/editor/contrib/inlineCompletions/browser/view/inlineEdits/`:
//! - `inlineEditsViews/longDistanceHint/inlineEditsLongDistanceHint.ts`
//!   (`InlineEditsLongDistanceHint`, `scrollToReveal`), as an overlay
//!   widget with the preview drawn as lines rather than an embedded editor,
//! - `.../longDistanceHint/longDistancePreviewEditor.ts` (which lines the
//!   preview shows, `expandLineRangeWithContext`),
//! - `.../longDistanceHint/longDistnaceWidgetPlacement.ts`
//!   ([`WidgetPlacementContext`]),
//! - `utils/towersLayout.ts` ([`max_tower_height_in_available_area`]) and
//!   `utils/flexBoxLayout.ts` ([`distribute_flex_box_layout`]),
//! - `inlineEditsView.ts` (`_getLongDistanceHintState`).

use std::ops::Range;
use std::sync::Arc;

use gpui::prelude::FluentBuilder as _;
use gpui::{
    AppContext as _, Context, Hsla, InteractiveElement, IntoElement, MouseButton, ParentElement,
    Pixels, Render, SharedString, Styled, WeakEntity, Window, div, point, px,
};
use text::PointUtf16;

use super::inline_edits::InlineEditDiff;
use super::inline_edits_view::{Segment, TRANSITION_DURATION, TabAction, Transition};
use crate::codicons::{Codicon, icon as codicon};
use crate::editor::CodeEditor;
use crate::theme::Theme;
use crate::widgets::{OverlayPosition, OverlayWidget, WidgetId};

/// Monaco `BORDER_RADIUS`.
const BORDER_RADIUS: f32 = 6.;
/// Monaco `MAX_WIDGET_WIDTH.EMPTY_SPACE`.
const MAX_WIDGET_WIDTH_EMPTY_SPACE: f32 = 425.;
/// Monaco `MAX_WIDGET_WIDTH.OVERLAY`.
const MAX_WIDGET_WIDTH_OVERLAY: f32 = 375.;
/// Monaco `MIN_WIDGET_WIDTH`.
const MIN_WIDGET_WIDTH: f32 = 250.;

/// Monaco `DEFAULT_WIDGET_LAYOUT_CONSTANTS`.
pub(crate) const DEFAULT_LAYOUT_CONSTANTS: WidgetLayoutConstants = WidgetLayoutConstants {
    preview_editor_margin: 2.,
    widget_padding: 2.,
    widget_border: 1.,
    lower_bar_height: 20.,
    min_widget_width: MIN_WIDGET_WIDTH,
};

/// Monaco `WidgetLayoutConstants`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct WidgetLayoutConstants {
    pub preview_editor_margin: f32,
    pub widget_padding: f32,
    pub widget_border: f32,
    pub lower_bar_height: f32,
    pub min_widget_width: f32,
}

/// Monaco `WidgetOutline`: where the widget may go (pixels in the editor's
/// content coordinates, unscrolled).
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct WidgetOutline {
    pub horizontal: Range<f32>,
    pub vertical: Range<f32>,
}

/// Monaco `ContinuousLineSizes`: lines laid out one after another, from
/// `top`, with each line's (width, height).
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct ContinuousLineSizes {
    /// The lines (any numbering, end exclusive).
    pub lines: Range<u32>,
    pub top: f32,
    pub sizes: Vec<(f32, f32)>,
}

/// Monaco `WidgetPlacementContext`: the free space right of a run of
/// lines.
#[derive(Clone, Debug)]
pub(crate) struct WidgetPlacementContext {
    info: ContinuousLineSizes,
    /// Each line's free (width, height) right of its text.
    #[allow(dead_code)] // Read by the tests ported from Monaco.
    pub available_space_sizes: Vec<(f32, f32)>,
    /// Heights summed from the first line: `[0, h0, h0 + h1, ...]`.
    pub available_space_height_prefix_sums: Vec<f32>,
    /// `available_space_sizes` as (height, width).
    pub available_space_sizes_transposed: Vec<(f32, f32)>,
}

impl WidgetPlacementContext {
    /// The free space of `info`'s lines in a text area `content_width`
    /// wide, leaving `end_of_line_padding(line)` after each line's text.
    pub fn new(
        info: ContinuousLineSizes,
        content_width: f32,
        end_of_line_padding: impl Fn(u32) -> f32,
    ) -> Self {
        let available_space_sizes: Vec<(f32, f32)> = info
            .sizes
            .iter()
            .enumerate()
            .map(|(ix, &(width, height))| {
                let line = info.lines.start + ix as u32;
                (
                    (content_width - width - end_of_line_padding(line)).max(0.),
                    height,
                )
            })
            .collect();
        let mut available_space_height_prefix_sums = vec![0.];
        let mut sum = 0.;
        for (_, height) in &available_space_sizes {
            sum += height;
            available_space_height_prefix_sums.push(sum);
        }
        let available_space_sizes_transposed =
            available_space_sizes.iter().map(|&(w, h)| (h, w)).collect();
        Self {
            info,
            available_space_sizes,
            available_space_height_prefix_sums,
            available_space_sizes_transposed,
        }
    }

    /// Monaco `getWidgetVerticalOutline`: the widget's vertical extent with
    /// its preview starting at `line`.
    pub fn widget_vertical_outline(
        &self,
        line: u32,
        preview_height: f32,
        constants: &WidgetLayoutConstants,
    ) -> Range<f32> {
        let ix = (line.saturating_sub(self.info.lines.start) as usize)
            .min(self.available_space_height_prefix_sums.len() - 1);
        let top = self.info.top + self.available_space_height_prefix_sums[ix];
        let margin =
            constants.preview_editor_margin + constants.widget_padding + constants.widget_border;
        top - margin..top + preview_height + margin + constants.lower_bar_height
    }

    /// Monaco `tryFindWidgetOutline`: the place nearest `target_line`
    /// (not the run's first or last line) where at least
    /// `min_widget_width` is free beside every line the widget covers.
    pub fn try_find_widget_outline(
        &self,
        target_line: u32,
        preview_height: f32,
        content_right: f32,
        constants: &WidgetLayoutConstants,
    ) -> Option<WidgetOutline> {
        if self.info.lines.len() < 3 {
            return None;
        }
        let range = self.info.lines.start + 1..self.info.lines.end - 1;
        find_first_minimize_distance(range, target_line, |line| {
            let vertical = self.widget_vertical_outline(line, preview_height, constants);
            let max_width = max_tower_height_in_available_area(
                vertical.start - self.info.top..vertical.end - self.info.top,
                &self.available_space_sizes_transposed,
            );
            (max_width >= constants.min_widget_width).then_some(WidgetOutline {
                horizontal: content_right - max_width..content_right,
                vertical,
            })
        })
    }
}

/// Monaco `findFirstMinimzeDistance`: `predicate` on the lines of `range`
/// in order of distance from `target` (below first), the first answer.
fn find_first_minimize_distance<T>(
    range: Range<u32>,
    target: u32,
    predicate: impl Fn(u32) -> Option<T>,
) -> Option<T> {
    let (start, end) = (range.start as i64, range.end as i64);
    let target = target as i64;
    for offset in 0i64.. {
        let down = target + offset;
        if down <= end
            && down >= 0
            && let Some(result) = predicate(down as u32)
        {
            return Some(result);
        }
        let up = target - offset;
        if up >= start
            && let Some(result) = predicate(up as u32)
        {
            return Some(result);
        }
        if up < start && down > end {
            return None;
        }
    }
    None
}

/// Monaco `getMaxTowerHeightInAvailableArea`: towers of the given
/// (width, height) stand side by side from 0, aligned at the bottom; how
/// high can a tower spanning `range` be and stay inside them?
pub(crate) fn max_tower_height_in_available_area(range: Range<f32>, areas: &[(f32, f32)]) -> f32 {
    let mut min_height = f32::MAX;
    let mut left = 0.;
    for &(width, height) in areas {
        let right = left + width;
        if range.start.max(left) < range.end.min(right) {
            min_height = min_height.min(height);
        }
        left = right;
    }
    if range.end > left || min_height == f32::MAX {
        return 0.;
    }
    min_height
}

/// Monaco `IFlexBoxPartExtensionRule`: grow up to `max` at `priority`,
/// sharing with others of the same priority by `share`.
#[derive(Clone, Copy, Debug)]
pub(crate) struct FlexRule {
    pub max: f32,
    pub priority: i32,
    pub share: f32,
}

/// Monaco `distributeFlexBoxLayout`: `total` split between parts given as
/// (minimum, growth rules): each gets its minimum, then the highest
/// priority rules that can still grow share what's left, and so on.
/// `None` if the minimums don't fit.
pub(crate) fn distribute_flex_box_layout(
    total: f32,
    parts: &[(f32, Vec<FlexRule>)],
) -> Option<Vec<f32>> {
    let mut result: Vec<f32> = parts.iter().map(|(min, _)| *min).collect();
    let used: f32 = result.iter().sum();
    if used > total {
        return None;
    }
    let mut remaining = total - used;
    while remaining > 0. {
        let candidates: Vec<(usize, FlexRule)> = parts
            .iter()
            .enumerate()
            .flat_map(|(ix, (_, rules))| rules.iter().map(move |rule| (ix, *rule)))
            .filter(|(ix, rule)| result[*ix] < rule.max)
            .collect();
        let Some(priority) = candidates.iter().map(|(_, rule)| rule.priority).max() else {
            break;
        };
        let top: Vec<_> = candidates
            .into_iter()
            .filter(|(_, rule)| rule.priority == priority)
            .collect();
        let total_share: f32 = top.iter().map(|(_, rule)| rule.share).sum();
        let mut distributed = 0.;
        let mut amounts = Vec::with_capacity(top.len());
        for (ix, rule) in &top {
            let amount = (remaining * rule.share / total_share).min(rule.max - result[*ix]);
            amounts.push((*ix, amount));
            distributed += amount;
        }
        if distributed == 0. {
            break;
        }
        for (ix, amount) in amounts {
            result[ix] += amount;
        }
        remaining -= distributed;
        if remaining < 0.0001 {
            break;
        }
    }
    Some(result)
}

/// Monaco `scrollToReveal`: the scroll position closest to `current` that
/// shows `range` in a window `window` wide (its start if it's wider).
pub(crate) fn scroll_to_reveal(current: f32, window: f32, range: Range<f32>) -> f32 {
    let visible = current..current + window;
    if visible.start <= range.start && range.end <= visible.end {
        return current;
    }
    if range.end - range.start > window {
        return range.start;
    }
    if range.end > visible.end {
        return range.end - window;
    }
    if range.start < visible.start {
        return range.start;
    }
    current
}

/// Monaco `expandLineRangeWithContext`: `target` and `context` rows around
/// it, within `row_count` rows.
pub(crate) fn expand_rows_with_context(target: u32, context: u32, row_count: u32) -> Range<u32> {
    let target = target.min(row_count.saturating_sub(1));
    target.saturating_sub(context)..(target + context + 1).min(row_count)
}

/// What the hint previews.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Preview {
    /// The rows shown (of the original text, or of the text with the edit
    /// applied).
    pub rows: Range<u32>,
    /// The row the edit starts on.
    pub target_row: u32,
    /// Each row's text pieces.
    pub lines: Vec<Vec<Segment>>,
    /// The new text's horizontal extent on the target row, in UTF-16
    /// columns (for Monaco's `preferredRangeToReveal`).
    pub reveal: Range<u32>,
    /// The target row's indentation, in UTF-16 columns.
    pub indentation: u32,
}

/// The long-distance hint (Monaco `InlineEditsLongDistanceHint`): a
/// preview of the edit's lines with their line numbers, and a bar with
/// where it is and how to get there. Clicking it jumps to the edit.
pub(crate) struct LongDistanceHint {
    editor: WeakEntity<CodeEditor>,
    theme: Arc<Theme>,
    tab_action: TabAction,
    preview: Preview,
    /// The enclosing symbol, or the file name of a cross-file edit.
    location: Option<SharedString>,
    /// "Tab to jump", "Tab to open".
    label: SharedString,
    /// `arrow-right` (cross-file), `arrow-down` or `arrow-up`.
    arrow: Codicon,
    width: Pixels,
    /// How far the preview is scrolled right (Monaco
    /// `desiredPreviewEditorScrollLeft`).
    scroll_left: Pixels,
    gutter_width: Pixels,
    font: SharedString,
    font_size: Pixels,
    line_height: Pixels,
    /// Monaco's `transition: opacity 200ms ease-in-out`: it fades in when
    /// it shows, and out when the edit comes into view.
    opacity: Transition<f32>,
}

impl Render for LongDistanceHint {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = &self.theme.colors;
        let [_, border, _] = match self.tab_action {
            TabAction::Jump => colors.inline_edit_gutter_indicator_primary(),
            TabAction::Accept => colors.inline_edit_gutter_indicator_successful(),
            TabAction::Inactive => colors.inline_edit_gutter_indicator_secondary(),
        };
        let [pill_fg, pill_border, pill_bg] = match self.tab_action {
            TabAction::Jump => colors.inline_edit_gutter_indicator_primary(),
            TabAction::Accept => colors.inline_edit_gutter_indicator_successful(),
            TabAction::Inactive => colors.inline_edit_gutter_indicator_secondary(),
        };
        let editor = self.editor.clone();
        let line_height = self.line_height;
        let target_ix = self
            .preview
            .target_row
            .saturating_sub(self.preview.rows.start) as usize;
        let rows = self.preview.rows.clone();
        let constants = DEFAULT_LAYOUT_CONSTANTS;
        let lines = self.preview.lines.iter().enumerate().map(|(ix, segments)| {
            let row = rows.start + ix as u32;
            let gutter = div()
                .flex_none()
                .w(self.gutter_width)
                .h(line_height)
                .flex()
                .justify_end()
                .items_center()
                .pr(px(4.))
                .text_color(colors.line_number)
                .when(ix == target_ix, |this| {
                    // Monaco puts the gutter indicator on the target line.
                    this.child(
                        div()
                            .mr(px(2.))
                            .px(px(2.))
                            .h(line_height - px(2.))
                            .flex()
                            .items_center()
                            .rounded(px(3.))
                            .border_1()
                            .border_color(pill_border)
                            .bg(pill_bg)
                            .text_color(pill_fg)
                            .text_size(px(10.))
                            .child(codicon(Codicon::KeyboardTab).size(px(10.))),
                    )
                })
                .child((row + 1).to_string());
            let text = div().flex_1().overflow_hidden().h(line_height).child(
                div()
                    .ml(-self.scroll_left)
                    .flex()
                    .whitespace_nowrap()
                    .children(segments.iter().map(|segment| {
                        div()
                            .text_color(segment.color)
                            .when_some(segment.background, |this, background| this.bg(background))
                            .child(segment.text.clone())
                    })),
            );
            div().flex().h(line_height).child(gutter).child(text)
        });
        let hint = div()
            .id("inline-edit-long-distance-hint")
            .w(self.width)
            .p(px(constants.widget_padding))
            .flex()
            .flex_col()
            .overflow_hidden()
            .cursor_pointer()
            .occlude()
            .rounded(px(BORDER_RADIUS))
            .border_1()
            .border_color(border)
            .bg(colors.widget_background)
            .font_family(self.font.clone())
            .text_size(self.font_size)
            .line_height(line_height)
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |_, _, _, cx| {
                    cx.stop_propagation();
                    if let Some(editor) = editor.upgrade() {
                        editor.update(cx, |editor, cx| editor.jump_to_next_inline_edit(cx));
                    }
                }),
            )
            .child(
                div()
                    .p(px(constants.preview_editor_margin))
                    .bg(colors.background)
                    .overflow_hidden()
                    .children(lines),
            )
            .child(
                div()
                    .h(px(constants.lower_bar_height))
                    .mx(px(4.))
                    .flex()
                    .justify_between()
                    .items_center()
                    .text_size(px(12.))
                    // Monaco's `.bar`: `descriptionForeground`.
                    .text_color(colors.description_foreground)
                    .child(
                        div()
                            .overflow_hidden()
                            .whitespace_nowrap()
                            .text_ellipsis()
                            .children(self.location.clone()),
                    )
                    .child(
                        div()
                            .flex_none()
                            .pl(px(6.))
                            .flex()
                            .items_center()
                            .gap(px(3.))
                            .child(self.label.clone())
                            .child(codicon(self.arrow).size(px(12.))),
                    ),
            );
        self.opacity.apply(
            "inline-edit-long-distance-hint-fade",
            hint,
            |hint, opacity| hint.opacity(opacity),
        )
    }
}

/// What the hint shows and where (Monaco's view state for
/// `InlineEditsLongDistanceHint`).
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct LongDistanceState {
    /// Monaco `hint.lineNumber`: the row it shows beside (the first
    /// cursor row while the edit showed).
    pub hint_row: u32,
    pub preview: Preview,
    pub location: Option<SharedString>,
    pub cross_file: bool,
}

/// The widget's state in the editor.
#[derive(Default)]
pub(crate) struct LongDistanceHintState {
    /// Monaco `_currentInlineEditCache`: the item and the first cursor row
    /// while it showed.
    pub first_cursor_row: Option<(u64, u32)>,
    view: Option<(
        gpui::Entity<LongDistanceHint>,
        WidgetId,
        gpui::Point<Pixels>,
    )>,
    /// Removes the hint once it faded out (Monaco keeps its layout for
    /// 200ms after it stops being visible, `_isVisibleDelayed`).
    fade_out: Option<gpui::Task<()>>,
}

impl CodeEditor {
    /// Monaco `_getLongDistanceHintState`: where the long-distance hint
    /// shows for the inline edit `ix` (with `target`, its range), and
    /// whether it's visible: always for a cross-file edit, else while the
    /// edit isn't on screen (once it is, the hint fades out). Nothing once
    /// it's jumped to, or for a hint alone.
    pub(crate) fn long_distance_hint_row(
        &mut self,
        ix: usize,
        target: Option<&Range<PointUtf16>>,
        cx: &gpui::App,
    ) -> Option<(u32, bool)> {
        if !self.options.inline_suggest.edits.show_long_distance_hint {
            return None;
        }
        let item = &self.inline.items[ix];
        let edit = self.inline_edit_item(ix)?;
        if item.jumped || self.inline.jumped_to == Some(item.id) || edit.hint_only() {
            return None;
        }
        let cursor_row = self.cursors.primary().position.row;
        if self.inline_edit_is_cross_file(ix) {
            return Some((cursor_row, true));
        }
        let id = item.id;
        let row = match self.inline.edit_view.long_distance.first_cursor_row {
            Some((cached, row)) if cached == id => row,
            _ => {
                self.inline.edit_view.long_distance.first_cursor_row = Some((id, cursor_row));
                cursor_row
            }
        };
        let in_viewport = target.is_some_and(|target| self.inline_edit_in_viewport(target, cx));
        self.layout.is_some().then_some((row, !in_viewport))
    }

    /// Monaco `LongDistancePreviewEditor._state`: the rows the hint
    /// previews. A jump or a deletion shows the original text at its row;
    /// anything else the text with the edit applied, at its first changed
    /// row. `rope` and `language` are the edited model's.
    pub(crate) fn long_distance_preview(
        &self,
        diff: Option<&InlineEditDiff>,
        jump_to: Option<PointUtf16>,
        rope: &text::Rope,
        language: Option<&str>,
        cx: &gpui::App,
    ) -> Option<Preview> {
        let context = self
            .options
            .inline_suggest
            .edits
            .long_distance_hint_context_line_count
            .min(10);
        let colors = &self.theme.colors;
        let row_count = rope.max_point().row + 1;
        let (target_row, rows, lines, highlights, highlight): (
            u32,
            Range<u32>,
            Vec<String>,
            Vec<Range<PointUtf16>>,
            Hsla,
        ) = match (jump_to, diff) {
            (Some(at), _) => {
                let rows = expand_rows_with_context(at.row, context, row_count);
                let lines = rows.clone().map(|row| rope.line(row)).collect();
                (
                    at.row,
                    rows,
                    lines,
                    Vec::new(),
                    colors.inline_edit_modified_changed_text_background(),
                )
            }
            (None, Some(diff)) => {
                let (original, modified, _) = diff.lines.first()?;
                let deletion = diff.inner.iter().all(|(_, modified)| modified.is_empty());
                if deletion {
                    let rows = expand_rows_with_context(original.start, context, row_count);
                    let lines = rows.clone().map(|row| rope.line(row)).collect();
                    let highlights = diff.inner.iter().map(|(o, _)| o.clone()).collect();
                    (
                        original.start,
                        rows,
                        lines,
                        highlights,
                        colors.inline_edit_original_changed_text_background(),
                    )
                } else {
                    let modified_count = diff.modified_row_count(row_count);
                    let rows = expand_rows_with_context(modified.start, context, modified_count);
                    let lines = rows
                        .clone()
                        .map(|row| diff.modified_document_line(rope, row))
                        .collect();
                    let highlights = diff.inner.iter().map(|(_, m)| m.clone()).collect();
                    (
                        modified.start,
                        rows,
                        lines,
                        highlights,
                        colors.inline_edit_modified_changed_text_background(),
                    )
                }
            }
            (None, None) => return None,
        };
        let target_line = lines
            .get(target_row.saturating_sub(rows.start) as usize)
            .cloned()
            .unwrap_or_default();
        let reveal = highlights
            .iter()
            .filter(|range| range.start.row <= target_row && target_row <= range.end.row)
            .map(|range| {
                let start = if range.start.row == target_row {
                    range.start.column
                } else {
                    0
                };
                let end = if range.end.row == target_row {
                    range.end.column
                } else {
                    target_line.encode_utf16().count() as u32
                };
                start..end.max(start)
            })
            .reduce(|a, b| a.start.min(b.start)..a.end.max(b.end))
            .unwrap_or(0..0);
        let indentation = (target_line.len() - target_line.trim_start().len()) as u32;
        let lines = self.colored_lines(&lines, rows.start, language, &highlights, highlight, cx);
        Some(Preview {
            rows,
            target_row,
            lines,
            reveal,
            indentation,
        })
    }

    /// Shows, moves or hides the long-distance hint.
    pub(crate) fn refresh_long_distance_hint(
        &mut self,
        state: Option<LongDistanceState>,
        tab_action: TabAction,
        cx: &mut Context<Self>,
    ) {
        let Some(state) = state else {
            self.clear_long_distance_hint(cx);
            return;
        };
        let Some(placement) = self.long_distance_placement(&state, cx) else {
            self.clear_long_distance_hint(cx);
            return;
        };
        let (position, width, scroll_left, gutter_width) = placement;
        let label: SharedString = if state.cross_file {
            "Tab to open".into()
        } else {
            "Tab to jump".into()
        };
        let arrow = if state.cross_file {
            Codicon::ArrowRight
        } else if state.hint_row < state.preview.target_row {
            Codicon::ArrowDown
        } else {
            Codicon::ArrowUp
        };
        let font = self
            .options
            .inline_suggest
            .font_family
            .clone()
            .unwrap_or_else(|| self.options.font_family.clone());
        let font_size = self.options.font_size;
        let line_height = self.line_height();
        if let Some((view, id, current)) = self.inline.edit_view.long_distance.view.clone() {
            // Visible again while fading out: it fades back in.
            if self
                .inline
                .edit_view
                .long_distance
                .fade_out
                .take()
                .is_some()
            {
                view.update(cx, |view, cx| {
                    view.opacity.set(1.);
                    cx.notify();
                });
            }
            view.update(cx, |view, cx| {
                let changed = view.tab_action != tab_action
                    || view.preview != state.preview
                    || view.location != state.location
                    || view.label != label
                    || view.arrow != arrow
                    || view.width != width
                    || view.scroll_left != scroll_left
                    || view.gutter_width != gutter_width;
                if !changed {
                    return;
                }
                view.tab_action = tab_action;
                view.preview = state.preview.clone();
                view.location = state.location.clone();
                view.label = label.clone();
                view.arrow = arrow;
                view.width = width;
                view.scroll_left = scroll_left;
                view.gutter_width = gutter_width;
                cx.notify();
            });
            if current != position {
                if let Some(overlay) = self.widgets.overlays.iter_mut().find(|w| w.id == id) {
                    overlay.position = OverlayPosition::Custom(position);
                }
                self.inline.edit_view.long_distance.view = Some((view, id, position));
                cx.notify();
            }
            return;
        }
        let editor = cx.entity().downgrade();
        // Monaco's fade-in from the opacity it has while hidden.
        let mut opacity = Transition::new(0.);
        opacity.set(1.);
        let theme = self.theme.clone();
        let view = cx.new(|_| LongDistanceHint {
            editor,
            theme,
            tab_action,
            preview: state.preview,
            location: state.location,
            label,
            arrow,
            width,
            scroll_left,
            gutter_width,
            font,
            font_size,
            line_height,
            opacity,
        });
        let id = self.add_overlay_widget(
            OverlayWidget::new(OverlayPosition::Custom(position), view.clone().into()),
            cx,
        );
        self.inline.edit_view.long_distance.view = Some((view, id, position));
    }

    /// The hint's opacity target and whether it's still easing.
    pub(crate) fn long_distance_hint_opacity(&self, cx: &gpui::App) -> Option<(f32, bool)> {
        let (view, _, _) = self.inline.edit_view.long_distance.view.as_ref()?;
        let opacity = &view.read(cx).opacity;
        Some((*opacity.target(), opacity.animating()))
    }

    /// Monaco's hint once the edit is in view (`isVisible: false`): it
    /// fades out, then goes.
    pub(crate) fn fade_out_long_distance_hint(&mut self, cx: &mut Context<Self>) {
        let state = &mut self.inline.edit_view.long_distance;
        if state.fade_out.is_some() {
            return;
        }
        let Some((view, _, _)) = state.view.clone() else {
            return;
        };
        if cx.reduce_motion() {
            self.clear_long_distance_hint(cx);
            return;
        }
        view.update(cx, |view, cx| {
            view.opacity.set(0.);
            cx.notify();
        });
        self.inline.edit_view.long_distance.fade_out = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(TRANSITION_DURATION).await;
            this.update(cx, |this, cx| {
                if this.inline.edit_view.long_distance.fade_out.is_some() {
                    this.clear_long_distance_hint(cx);
                }
            })
            .ok();
        }));
    }

    pub(crate) fn clear_long_distance_hint(&mut self, cx: &mut Context<Self>) {
        self.inline.edit_view.long_distance.fade_out = None;
        if let Some((_, id, _)) = self.inline.edit_view.long_distance.view.take() {
            self.remove_widget(id, cx);
        }
    }

    /// Monaco `_previewEditorLayoutInfo`: the widget's top-left (from the
    /// editor's), width, preview scroll and gutter width. It goes in the
    /// free space right of the lines around the hint row, nearest that
    /// row, else over the text at the right.
    fn long_distance_placement(
        &self,
        state: &LongDistanceState,
        cx: &gpui::App,
    ) -> Option<(gpui::Point<Pixels>, Pixels, Pixels, Pixels)> {
        let layout = self.layout.as_ref()?;
        let rope = self.model().read(cx).buffer().rope().clone();
        let origin = layout.bounds.origin;
        let line_height = f32::from(layout.metrics.line_height);
        let em = f32::from(layout.metrics.em_width);
        // Monaco `_lineSizesAroundHintPosition`: five rows around the hint
        // row, split where folds or zones break them apart.
        let max_row = rope.max_point().row;
        let around = state.hint_row.saturating_sub(5)..(state.hint_row + 6).min(max_row + 1);
        let mut runs: Vec<ContinuousLineSizes> = Vec::new();
        for row in around {
            let segments: Vec<_> = layout.lines.iter().filter(|line| line.row == row).collect();
            let Some(first) = segments.first() else {
                continue;
            };
            let top = f32::from(layout.line_top(first.view_line) - origin.y + layout.scroll_top);
            let width = segments
                .iter()
                .map(|line| f32::from(line.x_for_column(line.columns.end)))
                .fold(0., f32::max);
            let height = segments
                .iter()
                .map(|line| f32::from(layout.line_height_of(line.view_line)))
                .sum::<f32>();
            match runs.last_mut() {
                Some(run)
                    if run.lines.end == row
                        && (run.top + run.sizes.iter().map(|s| s.1).sum::<f32>() - top).abs()
                            < 0.5 =>
                {
                    run.lines.end = row + 1;
                    run.sizes.push((width, height));
                }
                _ => runs.push(ContinuousLineSizes {
                    lines: row..row + 1,
                    top,
                    sizes: vec![(width, height)],
                }),
            }
        }
        let first_run = runs.first()?.clone();
        let content_left = f32::from(layout.text_bounds.left() - origin.x);
        let content_width = f32::from(layout.text_bounds.size.width);
        let content_right = content_left + content_width;
        let constants = DEFAULT_LAYOUT_CONSTANTS;
        let preview_height = state.preview.lines.len() as f32 * line_height + 2.;
        let hint_row = state.hint_row;
        let padding = |row: u32| if row == hint_row { 40. } else { 20. };
        let mut outline = None;
        let mut last_context = None;
        for run in runs {
            let context = WidgetPlacementContext::new(run, content_width, padding);
            outline = context.try_find_widget_outline(
                hint_row,
                preview_height,
                content_right,
                &constants,
            );
            last_context = Some(context);
            if outline.is_some() {
                break;
            }
        }
        let overlay = outline.is_none();
        let outline = match outline {
            Some(outline) => outline,
            None => {
                let width = (f32::from(layout.bounds.size.width) - content_left)
                    .min(MAX_WIDGET_WIDTH_OVERLAY);
                let context = last_context.unwrap_or_else(|| {
                    WidgetPlacementContext::new(first_run, content_width, padding)
                });
                let vertical =
                    context.widget_vertical_outline(hint_row + 2, preview_height, &constants);
                WidgetOutline {
                    horizontal: content_right - width..content_right,
                    vertical: (vertical.start + 10.)..(vertical.end + 10.),
                }
            }
        };
        let scroll_top = f32::from(layout.scroll_top);
        // The preview's widest line and gutter.
        let digits = (state.preview.rows.end).to_string().len() as f32;
        let gutter_width = (digits + 1.) * em + 22.;
        let widest = state
            .preview
            .lines
            .iter()
            .map(|line| {
                line.iter()
                    .map(|s| s.text.encode_utf16().count())
                    .sum::<usize>()
            })
            .max()
            .unwrap_or(0) as f32
            * em;
        let content_based =
            widest + gutter_width + constants.preview_editor_margin + constants.widget_padding;
        let upper = if overlay {
            MAX_WIDGET_WIDTH_OVERLAY
        } else {
            MAX_WIDGET_WIDTH_EMPTY_SPACE
        };
        let max_widget_width = upper.min(content_based.max(constants.min_widget_width));
        let available = outline.horizontal.end - outline.horizontal.start;
        let parts = distribute_flex_box_layout(
            available,
            &[
                (
                    0.,
                    vec![FlexRule {
                        max: 10.,
                        priority: 1,
                        share: 1.,
                    }],
                ),
                (
                    50.,
                    vec![
                        FlexRule {
                            max: 150.,
                            priority: 2,
                            share: 1.,
                        },
                        FlexRule {
                            max: max_widget_width,
                            priority: 1,
                            share: 1.,
                        },
                    ],
                ),
                (
                    10.,
                    vec![FlexRule {
                        max: f32::INFINITY,
                        priority: 0,
                        share: 1.,
                    }],
                ),
            ],
        )?;
        let left = outline.horizontal.start + parts[0];
        let width = parts[1];
        let top = outline.vertical.start - scroll_top;
        // Monaco `desiredPreviewEditorScrollLeft`.
        let text_width = width
            - 2. * (constants.widget_padding
                + constants.widget_border
                + constants.preview_editor_margin)
            - gutter_width;
        let reveal = state.preview.reveal.start as f32 * em..state.preview.reveal.end as f32 * em;
        let max_reveal = text_width * 0.8;
        let reveal = reveal.start..reveal.end.min(reveal.start + max_reveal);
        let preview_scroll = scroll_to_reveal(
            state.preview.indentation as f32 * em,
            text_width.max(0.),
            reveal,
        )
        .max(0.);
        Some((
            point(px(left), px(top)),
            px(width),
            px(preview_scroll),
            px(gutter_width),
        ))
    }

    /// The long-distance hint's rows and bar text, for tests.
    #[doc(hidden)]
    pub fn inline_edit_long_distance_hint_for_tests(
        &self,
        cx: &gpui::App,
    ) -> Option<(Vec<String>, String)> {
        let (view, _, _) = self.inline.edit_view.long_distance.view.as_ref()?;
        let view = view.read(cx);
        let lines = view
            .preview
            .lines
            .iter()
            .map(|line| line.iter().map(|s| s.text.as_str()).collect())
            .collect();
        let bar = match &view.location {
            Some(location) => format!("{location} | {} {}", view.label, view.arrow.fallback()),
            None => format!("{} {}", view.label, view.arrow.fallback()),
        };
        Some((lines, bar))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Ported from Monaco's `layout.test.ts`.

    #[test]
    fn tower_heights() {
        let tower = max_tower_height_in_available_area;
        // Fits within a single area.
        assert_eq!(tower(5.0..15., &[(50., 30.)]), 30.);
        // Extends beyond the available width.
        assert_eq!(tower(0.0..60., &[(50., 30.)]), 0.);
        // Across several areas: the lowest.
        assert_eq!(tower(10.0..40., &[(20., 30.), (20., 25.), (20., 30.)]), 25.);
        assert_eq!(tower(10.0..40., &[(20., 30.), (20., 15.), (20., 30.)]), 15.);
        // At the edges.
        assert_eq!(tower(0.0..10., &[(50., 30.)]), 30.);
        assert_eq!(tower(40.0..50., &[(50., 30.)]), 30.);
        assert_eq!(tower(0.0..50., &[(50., 30.)]), 30.);
        // No areas.
        assert_eq!(tower(0.0..10., &[]), 0.);
        // Spanning exactly two, or starting at their boundary.
        assert_eq!(tower(10.0..50., &[(30., 25.), (30., 25.)]), 25.);
        assert_eq!(tower(30.0..50., &[(30., 25.), (30., 25.)]), 25.);
        let varying = [(10., 30.), (10., 15.), (10., 25.), (10., 30.), (10., 40.)];
        assert_eq!(tower(0.0..50., &varying), 15.);
        // Beyond every area.
        assert_eq!(tower(100.0..110., &[(50., 30.)]), 0.);
    }

    // Ported from Monaco's `scrollToReveal.test.ts`.

    #[test]
    fn scrolling_to_reveal() {
        let cases: [(f32, f32, Range<f32>, f32); 20] = [
            (10., 40., 20.0..30., 10.),
            (10., 40., 10.0..50., 10.),
            (20., 40., 5.0..15., 5.),
            (10., 40., 50.0..80., 40.),
            (10., 40., 20.0..100., 20.),
            (10., 40., 25.0..25., 10.),
            (10., 0., 20.0..30., 20.),
            (10., 40., 10.0..20., 10.),
            (10., 40., 40.0..50., 10.),
            (10., 40., 40.0..60., 20.),
            (20., 40., 5.0..25., 5.),
            (20., 40., 5.0..70., 5.),
            (-10., 40., 35.0..45., 5.),
            (1_000_000., 500., 1_000_600.0..1_000_700., 1_000_200.),
            (20., 40., 5.0..55., 5.),
            (10., 40., 100.0..101., 61.),
            (10., 40., 9.0..19., 9.),
            (10., 40., 45.0..51., 11.),
            (50., 1., 100.0..105., 100.),
            (20., 40., 5.0..25., 5.),
        ];
        for (current, window, range, expected) in cases {
            assert_eq!(
                scroll_to_reveal(current, window, range.clone()),
                expected,
                "{current} {window} {range:?}"
            );
        }
    }

    // Ported from Monaco's `longDistanceWidgetPlacement.test.ts`.

    const CONSTANTS: WidgetLayoutConstants = WidgetLayoutConstants {
        preview_editor_margin: 5.,
        widget_padding: 2.,
        widget_border: 1.,
        lower_bar_height: 10.,
        min_widget_width: 50.,
    };

    fn info(start: u32, sizes: &[(f32, f32)], top: f32) -> ContinuousLineSizes {
        ContinuousLineSizes {
            lines: start..start + sizes.len() as u32,
            top,
            sizes: sizes.to_vec(),
        }
    }

    #[test]
    fn available_space() {
        let context = WidgetPlacementContext::new(
            info(1, &[(100., 20.), (150., 20.), (80., 20.)], 0.),
            500.,
            |_| 0.,
        );
        let widths: Vec<f32> = context.available_space_sizes.iter().map(|s| s.0).collect();
        assert_eq!(widths, [400., 350., 420.]);
        // With end of line padding.
        let context =
            WidgetPlacementContext::new(info(1, &[(100., 20.), (150., 20.)], 0.), 500., |line| {
                line as f32 * 10.
            });
        let widths: Vec<f32> = context.available_space_sizes.iter().map(|s| s.0).collect();
        assert_eq!(widths, [390., 330.]);
        // Never negative.
        let context = WidgetPlacementContext::new(info(1, &[(600., 20.)], 0.), 500., |_| 0.);
        assert_eq!(context.available_space_sizes[0].0, 0.);
        // Heights are kept, summed and transposed.
        let context = WidgetPlacementContext::new(
            info(1, &[(100., 25.), (100., 30.), (100., 20.)], 0.),
            500.,
            |_| 0.,
        );
        let heights: Vec<f32> = context.available_space_sizes.iter().map(|s| s.1).collect();
        assert_eq!(heights, [25., 30., 20.]);
        assert_eq!(
            context.available_space_height_prefix_sums,
            [0., 25., 55., 75.]
        );
        assert_eq!(context.available_space_sizes_transposed[0], (25., 400.));
        // Empty lines leave the whole width.
        let context =
            WidgetPlacementContext::new(info(1, &[(0., 20.), (0., 20.)], 0.), 500., |_| 0.);
        assert_eq!(context.available_space_sizes[1].0, 500.);
    }

    #[test]
    fn vertical_outlines() {
        let context =
            WidgetPlacementContext::new(info(1, &[(100., 20.), (100., 20.)], 100.), 500., |_| 0.);
        // margin 5 + 2 + 1 = 8; the bar adds 10 below.
        assert_eq!(
            context.widget_vertical_outline(1, 50., &CONSTANTS),
            92.0..168.
        );
        let context =
            WidgetPlacementContext::new(info(1, &[(100., 20.), (100., 25.)], 100.), 500., |_| 0.);
        assert_eq!(
            context.widget_vertical_outline(2, 50., &CONSTANTS),
            112.0..188.
        );
        let zero = WidgetLayoutConstants {
            preview_editor_margin: 0.,
            widget_padding: 0.,
            widget_border: 0.,
            lower_bar_height: 0.,
            min_widget_width: 50.,
        };
        let context = WidgetPlacementContext::new(info(1, &[(100., 20.)], 0.), 500., |_| 0.);
        assert_eq!(context.widget_vertical_outline(1, 50., &zero), 0.0..50.);
    }

    #[test]
    fn finding_an_outline() {
        // No line has room.
        let context = WidgetPlacementContext::new(
            info(1, &[(460., 20.), (470., 20.), (480., 20.)], 0.),
            500.,
            |_| 0.,
        );
        assert_eq!(
            context.try_find_widget_outline(2, 15., 500., &CONSTANTS),
            None
        );
        // Room on the target line, ending at the content's right.
        let context = WidgetPlacementContext::new(
            info(1, &[(100., 20.), (100., 20.), (100., 20.)], 0.),
            500.,
            |_| 0.,
        );
        let outline = context
            .try_find_widget_outline(2, 15., 500., &CONSTANTS)
            .unwrap();
        assert!(outline.horizontal.end - outline.horizontal.start >= 50.);
        assert_eq!(outline.horizontal.end, 500.);
        // Searches outward from the target.
        let sizes = [
            (100., 20.),
            (460., 20.),
            (460., 20.),
            (100., 20.),
            (100., 20.),
            (100., 20.),
            (100., 20.),
        ];
        let context = WidgetPlacementContext::new(info(1, &sizes, 0.), 500., |_| 0.);
        assert!(
            context
                .try_find_widget_outline(3, 15., 500., &CONSTANTS)
                .is_some()
        );
        // Prefers lines closer to the target.
        let sizes = [
            (100., 20.),
            (100., 20.),
            (100., 20.),
            (100., 20.),
            (500., 9999.),
            (100., 20.),
            (100., 20.),
            (100., 20.),
            (100., 20.),
        ];
        let context = WidgetPlacementContext::new(info(1, &sizes, 0.), 500., |_| 0.);
        for target in 0..=4 {
            let outline = context
                .try_find_widget_outline(target, 15., 500., &CONSTANTS)
                .unwrap();
            assert!(outline.vertical.end < 9999., "{target}");
        }
        for target in 5..=10 {
            let outline = context
                .try_find_widget_outline(target, 15., 500., &CONSTANTS)
                .unwrap();
            assert!(outline.vertical.start > 9999., "{target}");
        }
    }

    #[test]
    fn flex_box_layout() {
        let rule = |max: f32, priority: i32| FlexRule {
            max,
            priority,
            share: 1.,
        };
        // Minimums first, then the highest priority grows to its max.
        let parts = [
            (0., vec![rule(10., 1)]),
            (50., vec![rule(150., 2), rule(300., 1)]),
            (10., vec![rule(f32::INFINITY, 0)]),
        ];
        assert_eq!(
            distribute_flex_box_layout(400., &parts),
            Some(vec![10., 300., 90.])
        );
        assert_eq!(
            distribute_flex_box_layout(100., &parts),
            Some(vec![0., 90., 10.])
        );
        // The minimums don't fit.
        assert_eq!(distribute_flex_box_layout(40., &parts), None);
    }

    #[test]
    fn context_rows() {
        assert_eq!(expand_rows_with_context(5, 0, 10), 5..6);
        assert_eq!(expand_rows_with_context(5, 2, 10), 3..8);
        assert_eq!(expand_rows_with_context(0, 2, 10), 0..3);
        assert_eq!(expand_rows_with_context(9, 2, 10), 7..10);
        assert_eq!(expand_rows_with_context(20, 1, 10), 8..10);
    }
}
