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
    ParentElement as _, PathBuilder, Pixels, Render, SharedString, Styled as _, StyledText,
    Subscription, Task, WeakEntity, Window, canvas, div, point, px,
};
use text::{Point, PointUtf16};

use crate::cursor::Selection;
use crate::decorations::{DecorationId, DecorationOptions, OverviewRulerLane};
use crate::diff::{DetailedLineRangeMapping, DiffOptions, LineRange, LinesDiff, compute_diff};
use crate::editor::{CodeEditor, CodeEditorEvent};
use crate::model::{TextModel, TextModelEvent};
use crate::theme::EditorColors;
use crate::widgets::{ViewZone, WidgetId, ZoneHeight};

gpui::actions!(
    diff_editor,
    [
        /// Goes to the next change (Monaco `goToDiff('next')`, Alt+F5).
        GoToNextDiff,
        /// Goes to the previous change (Monaco `goToDiff('previous')`,
        /// Shift+Alt+F5).
        GoToPreviousDiff,
    ]
);

/// The key context of the diff editor.
pub const DIFF_EDITOR_KEY_CONTEXT: &str = "DiffEditor";

/// Binds the diff editor's keys (called by [`crate::init`]).
pub(crate) fn bind_keys(cx: &mut App) {
    cx.bind_keys([
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
            cx.subscribe(&self.original, |this, _, event: &CodeEditorEvent, cx| {
                if matches!(event, CodeEditorEvent::ScrollChanged) {
                    this.sync_scroll(true, cx);
                }
            }),
            cx.subscribe(&self.modified, |this, _, event: &CodeEditorEvent, cx| {
                if matches!(event, CodeEditorEvent::ScrollChanged) {
                    this.sync_scroll(false, cx);
                }
            }),
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
            || options.show_moves != self.options.show_moves;
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
        self.options = options;
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
    }

    /// Decorates both sides and aligns them with view zones.
    fn apply_diff(&mut self, cx: &mut Context<Self>) {
        let Some(diff) = self.diff.clone() else {
            return;
        };
        let colors = self.modified.read(cx).colors().clone();
        let (original_decorations, modified_decorations) =
            diff_decorations(&diff.changes, &self.options, &colors);
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
        let original_extra = self.original.read(cx).wrap_extra_heights();
        let modified_extra = self.modified.read(cx).wrap_extra_heights();
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
            let view = cx.new(|_| DiagonalFill { color: fill });
            let id = editor.update(cx, |editor, cx| {
                editor.add_view_zone(
                    ViewZone {
                        after_line: after as u32,
                        height: ZoneHeight::Pixels(px(height)),
                        view: view.into(),
                    },
                    cx,
                )
            });
            ids.push(id);
        }
        self.zones = zones;
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
        let position = source.read(cx).scroll_position();
        if target.read(cx).scroll_position() != position {
            target.update(cx, |editor, cx| editor.set_scroll_position(position, cx));
        }
        self.syncing_scroll = false;
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
        let bounds = self.bounds.clone();
        let ratio = self.ratio.clamp(0.1, 0.9);
        let border = self.modified.read(cx).colors().widget_border;
        let resizable = self.options.enable_split_view_resizing;
        div()
            .key_context(DIFF_EDITOR_KEY_CONTEXT)
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
            .child(div().h_full().flex_1().child(self.modified.clone()))
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
