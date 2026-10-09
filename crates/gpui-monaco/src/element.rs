//! The [`EditorElement`]: lays out and paints only the visible view lines.

use std::ops::Range;

use gpui::{
    App, BorderStyle, Bounds, ContentMask, Corners, Edges, Element, ElementId, ElementInputHandler,
    Entity, Font, FontStyle, FontWeight, GlobalElementId, Hsla, InspectorElementId, IntoElement,
    LayoutId, PathBuilder, Pixels, ShapedLine, SharedString, Size, Style, TextAlign, TextRun,
    UnderlineStyle, Window, fill, font, point, px, quad, relative, size,
};
use text::{PointUtf16, Rope};

use crate::cursor::Selection;
use crate::editor::CodeEditor;
use crate::options::{
    CursorStyle, EditorOptions, LineNumbers, RenderLineHighlight, ShowFoldingControls, WordWrap,
};
use crate::theme::{EditorColors, Theme, TokenStyle};
use crate::view::view_lines::{ViewLine, ViewLines, WrapSettings, utf16_to_byte};

/// Width of the glyph margin when enabled (Monaco `glyphMarginWidth`).
const GLYPH_MARGIN_WIDTH: f32 = 18.;
/// Monaco `lineDecorationsWidth` default.
const LINE_DECORATIONS_WIDTH: f32 = 10.;
/// Minimum scrollbar slider length.
const MIN_SLIDER: f32 = 20.;

/// One shaped view line: a (possibly wrapped) segment of a model row.
#[derive(Clone)]
pub(crate) struct LineLayout {
    pub view_line: u32,
    pub row: u32,
    /// UTF-16 columns of the model row on this view line.
    pub columns: Range<u32>,
    pub is_last_segment: bool,
    /// X of the first column (continuation indent).
    pub indent_x: Pixels,
    pub shaped: ShapedLine,
    /// Byte index into the shaped text for each column in `columns`,
    /// inclusive of the end.
    pub display_index: Vec<usize>,
}

impl LineLayout {
    pub fn x_for_column(&self, column: u32) -> Pixels {
        let column = column.clamp(self.columns.start, self.columns.end);
        let ix = ((column - self.columns.start) as usize).min(self.display_index.len() - 1);
        self.indent_x + self.shaped.x_for_index(self.display_index[ix])
    }

    pub fn column_for_x(&self, x: Pixels) -> u32 {
        let ix = self.shaped.closest_index_for_x(x - self.indent_x);
        let col = self.display_index.partition_point(|&d| d < ix);
        let col = col.min(self.display_index.len() - 1);
        // Never land between the halves of a surrogate pair.
        let col = if col > 0 && self.display_index[col] == self.display_index[col - 1] {
            col - 1
        } else {
            col
        };
        self.columns.start + col as u32
    }

    /// Width up to the end of the text on this line.
    pub fn width(&self) -> Pixels {
        self.indent_x + self.shaped.width
    }

    /// Whether `point` displays on this line.
    pub fn contains(&self, point: PointUtf16) -> bool {
        point.row == self.row
            && self.columns.start <= point.column
            && (point.column < self.columns.end
                || (point.column == self.columns.end && self.is_last_segment))
    }
}

/// Text style shared by everything the element shapes.
#[derive(Clone)]
pub(crate) struct TextMetrics {
    pub font: Font,
    pub font_size: Pixels,
    pub line_height: Pixels,
    pub em_width: Pixels,
}

impl TextMetrics {
    pub fn new(options: &EditorOptions, window: &mut Window) -> Self {
        let font = font(options.font_family.clone());
        let font_size = options.font_size;
        let em_width = window
            .text_system()
            .shape_line("m".into(), font_size, &[run(1, &font, gpui::black())], None)
            .width;
        Self {
            font,
            font_size,
            line_height: options.resolved_line_height(),
            em_width,
        }
    }
}

fn run(len: usize, font: &Font, color: Hsla) -> TextRun {
    TextRun {
        len,
        font: font.clone(),
        color,
        background_color: None,
        underline: None,
        strikethrough: None,
    }
}

/// What to shape for one view line.
pub(crate) struct RowSegment {
    pub view_line: u32,
    pub row: u32,
    pub columns: Range<u32>,
    pub start_visible_column: u32,
    pub indent: u32,
    pub is_last_segment: bool,
}

/// Shapes one view line. `highlights` are styled spans in row-relative byte
/// offsets, sorted and non-overlapping. `marked` is an IME composition range
/// in UTF-16 columns of the row, drawn underlined.
#[allow(clippy::too_many_arguments)]
pub(crate) fn layout_segment(
    rope: &Rope,
    segment: &RowSegment,
    tab_size: u32,
    metrics: &TextMetrics,
    color: Hsla,
    highlights: &[(Range<usize>, TokenStyle)],
    marked: Option<(Range<u32>, Hsla)>,
    window: &mut Window,
) -> LineLayout {
    let full = rope.line(segment.row);
    let byte_start = utf16_to_byte(&full, segment.columns.start);
    let byte_end = utf16_to_byte(&full, segment.columns.end);
    let line = &full[byte_start..byte_end];
    let tab_size = tab_size.max(1) as usize;
    let mut display = String::with_capacity(line.len());
    let mut display_index = Vec::with_capacity(line.len() + 1);
    // (row byte offset, display offset) for each char start.
    let mut char_starts = Vec::with_capacity(line.len());
    let mut visible_column = segment.start_visible_column as usize;
    for (byte, ch) in line.char_indices() {
        for _ in 0..ch.len_utf16() {
            display_index.push(display.len());
        }
        char_starts.push((byte_start + byte, display.len()));
        if ch == '\t' {
            let spaces = tab_size - visible_column % tab_size;
            display.extend(std::iter::repeat_n(' ', spaces));
            visible_column += spaces;
        } else {
            display.push(ch);
            visible_column += 1;
        }
    }
    display_index.push(display.len());
    let display_len = display.len();
    let to_display = |byte: usize| -> usize {
        if byte >= byte_end {
            return display_len;
        }
        if byte <= byte_start {
            return 0;
        }
        let ix = char_starts.partition_point(|&(b, _)| b < byte);
        char_starts.get(ix).map_or(display_len, |&(_, d)| d)
    };
    let column_to_display = |column: u32| -> usize {
        let column = column.clamp(segment.columns.start, segment.columns.end);
        display_index[((column - segment.columns.start) as usize).min(display_index.len() - 1)]
    };

    let marked_display = marked.as_ref().and_then(|(range, underline)| {
        let start = column_to_display(range.start);
        let end = column_to_display(range.end);
        (end > start).then_some((start..end, *underline))
    });
    let styled: Vec<(Range<usize>, TokenStyle)> = highlights
        .iter()
        .map(|(range, style)| (to_display(range.start)..to_display(range.end), *style))
        .filter(|(range, _)| !range.is_empty())
        .collect();
    let mut bounds = vec![0, display_len];
    for (range, _) in &styled {
        bounds.extend([range.start, range.end]);
    }
    if let Some((range, _)) = &marked_display {
        bounds.extend([range.start, range.end]);
    }
    bounds.sort_unstable();
    bounds.dedup();

    let mut runs = Vec::new();
    let mut span_ix = 0;
    for pair in bounds.windows(2) {
        let (start, end) = (pair[0], pair[1]);
        if start >= end || end > display_len {
            continue;
        }
        while span_ix < styled.len() && styled[span_ix].0.end <= start {
            span_ix += 1;
        }
        let style = styled
            .get(span_ix)
            .filter(|(range, _)| range.start <= start)
            .map(|(_, style)| *style)
            .unwrap_or_default();
        let mut font = metrics.font.clone();
        if style.bold {
            font.weight = FontWeight::BOLD;
        }
        if style.italic {
            font.style = FontStyle::Italic;
        }
        let mut text_run = run(end - start, &font, style.foreground.unwrap_or(color));
        let is_marked = marked_display
            .as_ref()
            .is_some_and(|(range, _)| range.start <= start && end <= range.end);
        if is_marked || style.underline {
            text_run.underline = Some(UnderlineStyle {
                color: Some(
                    marked_display
                        .as_ref()
                        .filter(|_| is_marked)
                        .map_or(text_run.color, |(_, underline)| *underline),
                ),
                thickness: px(1.),
                wavy: false,
            });
        }
        if style.strikethrough {
            text_run.strikethrough = Some(gpui::StrikethroughStyle {
                thickness: px(1.),
                color: Some(text_run.color),
            });
        }
        runs.push(text_run);
    }
    let shaped = window.text_system().shape_line(
        SharedString::from(display),
        metrics.font_size,
        &runs,
        None,
    );
    LineLayout {
        view_line: segment.view_line,
        row: segment.row,
        columns: segment.columns.clone(),
        is_last_segment: segment.is_last_segment,
        indent_x: metrics.em_width * segment.indent as f32,
        shaped,
        display_index,
    }
}

/// The segment shown on `view_line`.
pub(crate) fn row_segment(view_lines: &ViewLines, view_line: u32) -> RowSegment {
    let ViewLine { row, segment } = view_lines.view_line(view_line);
    let info = view_lines.segment_info(ViewLine { row, segment });
    RowSegment {
        view_line,
        row,
        columns: info.columns,
        start_visible_column: info.start_visible_column,
        indent: info.indent,
        is_last_segment: info.is_last,
    }
}

/// Highlight spans for byte `range` of the current text, using the model's
/// latest (possibly older) syntax state mapped through the edits since.
pub(crate) fn current_highlights(
    model: &crate::model::TextModel,
    theme: &Theme,
    range: Range<usize>,
) -> Vec<(Range<usize>, TokenStyle)> {
    let Some(syntax) = model.syntax_snapshot() else {
        return Vec::new();
    };
    let buffer = model.buffer();
    let style = |span: &syntax::HighlightSpan| theme.style_for(span.highlight);
    if syntax.text.version() == buffer.version() {
        return syntax
            .highlighter
            .highlights(&syntax.state, syntax.text.rope(), range)
            .iter()
            .map(|span| (span.range.clone(), style(span)))
            .collect();
    }
    let patch = buffer.edits_since(syntax.text.version());
    let inverse = patch.invert();
    let old_range = inverse.transform_offset(range.start, text::Bias::Left)
        ..inverse.transform_offset(range.end, text::Bias::Right);
    syntax
        .highlighter
        .highlights(&syntax.state, syntax.text.rope(), old_range)
        .iter()
        .filter_map(|span| {
            let start = patch.transform_offset(span.range.start, text::Bias::Right);
            let end = patch.transform_offset(span.range.end, text::Bias::Left);
            (start < end).then(|| (start..end, style(span)))
        })
        .collect()
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct ScrollbarLayout {
    pub track: Bounds<Pixels>,
    pub thumb: Bounds<Pixels>,
    /// Content pixels per track pixel.
    pub ratio: f32,
}

/// A fold control in the gutter.
#[derive(Clone, Copy, Debug)]
pub(crate) struct FoldControl {
    pub row: u32,
    pub bounds: Bounds<Pixels>,
    pub collapsed: bool,
}

/// Everything about the last frame that event handlers need for hit testing.
#[derive(Clone)]
pub(crate) struct EditorLayout {
    pub bounds: Bounds<Pixels>,
    pub gutter_width: Pixels,
    /// The text viewport: right of the gutter, excluding scrollbars.
    pub text_bounds: Bounds<Pixels>,
    pub metrics: TextMetrics,
    pub scroll_top: Pixels,
    pub scroll_left: Pixels,
    pub lines: Vec<LineLayout>,
    pub fold_controls: Vec<FoldControl>,
    pub vertical_scrollbar: Option<ScrollbarLayout>,
    pub horizontal_scrollbar: Option<ScrollbarLayout>,
}

impl EditorLayout {
    /// Visible view lines that fit fully, for page up/down.
    pub fn page_rows(&self) -> u32 {
        ((self.text_bounds.size.height / self.metrics.line_height).floor() as u32).max(1)
    }

    /// The laid-out view line showing `point`, if visible.
    pub fn line_for_point(&self, point: PointUtf16) -> Option<&LineLayout> {
        self.lines.iter().find(|line| line.contains(point))
    }

    pub fn view_line(&self, view_line: u32) -> Option<&LineLayout> {
        let first = self.lines.first()?.view_line;
        self.lines.get(view_line.checked_sub(first)? as usize)
    }

    /// X of the start of text (column 0) in window coordinates.
    pub fn text_origin_x(&self) -> Pixels {
        self.text_bounds.left() - self.scroll_left
    }

    pub fn line_top(&self, view_line: u32) -> Pixels {
        self.bounds.top() + self.metrics.line_height * view_line as f32 - self.scroll_top
    }
}

pub struct EditorElement {
    editor: Entity<CodeEditor>,
}

impl EditorElement {
    pub fn new(editor: Entity<CodeEditor>) -> Self {
        Self { editor }
    }
}

impl IntoElement for EditorElement {
    type Element = Self;

    fn into_element(self) -> Self {
        self
    }
}

pub struct PrepaintState {
    layout: EditorLayout,
    selections: Vec<Selection>,
    /// View lines with a cursor (for the current line highlight).
    cursor_view_lines: Vec<u32>,
    cursor_quads: Vec<Bounds<Pixels>>,
    gutter_numbers: Vec<(Pixels, ShapedLine, Pixels)>,
    /// (x after the text, top) of rows whose region is collapsed.
    fold_placeholders: Vec<(Pixels, Pixels)>,
    show_fold_controls: bool,
    colors: EditorColors,
    options: EditorOptions,
    focused: bool,
    cursor_visible: bool,
}

impl Element for EditorElement {
    type RequestLayoutState = ();
    type PrepaintState = PrepaintState;

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, ()) {
        let mut style = Style::default();
        style.size.width = relative(1.).into();
        style.size.height = relative(1.).into();
        (window.request_layout(style, [], cx), ())
    }

    fn prepaint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) -> PrepaintState {
        self.editor
            .update(cx, |editor, cx| prepaint_editor(editor, bounds, window, cx))
    }

    fn paint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut (),
        state: &mut PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        let focus_handle = self.editor.read(cx).focus_handle.clone();
        window.handle_input(
            &focus_handle,
            ElementInputHandler::new(bounds, self.editor.clone()),
            cx,
        );
        paint_editor(state, window, cx);
    }
}

fn prepaint_editor(
    editor: &mut CodeEditor,
    bounds: Bounds<Pixels>,
    window: &mut Window,
    cx: &mut gpui::Context<CodeEditor>,
) -> PrepaintState {
    let options = editor.options.clone();
    let theme = editor.theme.clone();
    let colors = theme.colors.clone();
    let metrics = TextMetrics::new(&options, window);
    let line_height = metrics.line_height;
    let tab_size = editor.cursor_config(cx).tab_size;
    let line_count = editor.snapshot.line_count();

    // Gutter: [glyph margin][line numbers][line decorations + folding].
    let digits = line_count.to_string().len() as u32;
    let number_chars = digits.max(options.line_numbers_min_chars);
    let numbers_width = if options.line_numbers == LineNumbers::Off {
        px(0.)
    } else {
        metrics.em_width * number_chars as f32
    };
    let glyph_width = if options.glyph_margin {
        px(GLYPH_MARGIN_WIDTH)
    } else {
        px(0.)
    };
    let folding_width = if options.folding {
        line_height.min(px(18.))
    } else {
        px(0.)
    };
    let gutter_width =
        (glyph_width + numbers_width + px(LINE_DECORATIONS_WIDTH) + folding_width).round();

    let vbar = options.vertical_scrollbar_size;
    let hbar = options.horizontal_scrollbar_size;
    let viewport_height = bounds.size.height;
    let text_width = bounds.size.width - gutter_width - vbar;

    // Bring the view lines up to date: edits, wrapping, folds.
    let wrap_column = {
        let viewport_columns = ((text_width - metrics.em_width) / metrics.em_width)
            .floor()
            .max(1.) as u32;
        match options.word_wrap {
            WordWrap::Off => None,
            WordWrap::On => Some(viewport_columns),
            WordWrap::WordWrapColumn => Some(options.word_wrap_column.max(1)),
            WordWrap::Bounded => Some(viewport_columns.min(options.word_wrap_column.max(1))),
        }
    };
    editor.sync_view_lines(
        wrap_column.map(|column| WrapSettings {
            column,
            tab_size,
            indent: options.wrapping_indent,
            keep_all: false,
        }),
        cx,
    );
    let view_lines = editor.view_lines.clone();
    let view_line_count = view_lines.line_count().max(1);
    let snapshot = editor.snapshot.clone();

    let content_height = line_height * view_line_count as f32
        + if options.scroll_beyond_last_line {
            (viewport_height - line_height).max(px(0.))
        } else {
            px(0.)
        };
    let max_scroll_top = (content_height - viewport_height).max(px(0.));

    // Rough content width from the longest row; refined by shaped lines below.
    let mut content_width = if wrap_column.is_some() {
        text_width
    } else {
        metrics.em_width * (snapshot.summary().longest_row_chars as f32 + 2.)
    };
    let show_hbar = content_width > text_width;
    let text_height = if show_hbar {
        viewport_height - hbar
    } else {
        viewport_height
    };

    // Vertical autoscroll before choosing lines.
    let selections = editor.cursors.selections();
    let primary = selections[0].head;
    let (primary_view_line, _) = view_lines.to_view(primary, false);
    if editor.autoscroll_requested {
        let top = line_height * primary_view_line as f32;
        let bottom = top + line_height;
        if top < editor.scroll_top {
            editor.scroll_top = top;
        } else if bottom > editor.scroll_top + text_height {
            editor.scroll_top = bottom - text_height;
        }
    }
    editor.scroll_top = editor.scroll_top.clamp(px(0.), max_scroll_top);
    let scroll_top = editor.scroll_top;

    let first_line = (scroll_top / line_height).floor() as u32;
    let last_line = (((scroll_top + viewport_height) / line_height).ceil() as u32)
        .min(view_line_count.saturating_sub(1));
    let segments: Vec<RowSegment> = (first_line..=last_line)
        .map(|view_line| row_segment(&view_lines, view_line))
        .collect();

    let marked = editor.marked_range_points();
    let (visible_start, visible_end) = match (segments.first(), segments.last()) {
        (Some(first), Some(last)) => (
            snapshot.point_to_offset(text::Point::new(first.row, 0)),
            snapshot.point_to_offset(text::Point::new(last.row, u32::MAX)),
        ),
        _ => (0, 0),
    };
    let highlights = {
        let model = editor.model().read(cx);
        current_highlights(model, &theme, visible_start..visible_end)
    };
    let mut highlight_ix = 0;
    let mut lines = Vec::with_capacity(segments.len());
    for segment in &segments {
        let line_range = snapshot.line_range(segment.row);
        while highlight_ix < highlights.len() && highlights[highlight_ix].0.end <= line_range.start
        {
            highlight_ix += 1;
        }
        let row_highlights: Vec<(Range<usize>, TokenStyle)> = highlights[highlight_ix..]
            .iter()
            .take_while(|(range, _)| range.start < line_range.end)
            .map(|(range, style)| {
                let start = range.start.max(line_range.start) - line_range.start;
                let end = range.end.min(line_range.end) - line_range.start;
                (start..end, *style)
            })
            .collect();
        let marked_cols = marked.as_ref().and_then(|range| {
            (range.start.row == segment.row && range.end.row == segment.row)
                .then_some((range.start.column..range.end.column, colors.ime_underline))
        });
        let line = layout_segment(
            &snapshot,
            segment,
            tab_size,
            &metrics,
            colors.foreground,
            &row_highlights,
            marked_cols,
            window,
        );
        content_width = content_width.max(line.width() + metrics.em_width * 2.);
        lines.push(line);
    }

    // Horizontal autoscroll to the primary cursor.
    let max_scroll_left = (content_width - text_width).max(px(0.));
    if editor.autoscroll_requested {
        let primary_x = lines
            .iter()
            .find(|line| line.contains(primary))
            .map(|line| line.x_for_column(primary.column))
            .unwrap_or(px(0.));
        let padding = metrics.em_width * 2.;
        if primary_x < editor.scroll_left {
            editor.scroll_left = (primary_x - padding).max(px(0.));
        } else if primary_x + padding > editor.scroll_left + text_width {
            editor.scroll_left = primary_x + padding - text_width;
        }
        editor.autoscroll_requested = false;
    }
    editor.scroll_left = editor.scroll_left.clamp(px(0.), max_scroll_left);
    let scroll_left = editor.scroll_left;

    let text_bounds = Bounds::new(
        point(bounds.left() + gutter_width, bounds.top()),
        size(text_width, text_height),
    );

    let vertical_scrollbar = (max_scroll_top > px(0.)).then(|| {
        let track = Bounds::new(
            point(bounds.right() - vbar, bounds.top()),
            size(vbar, viewport_height),
        );
        let thumb_height =
            (viewport_height * (viewport_height / content_height)).max(px(MIN_SLIDER));
        let travel = viewport_height - thumb_height;
        let thumb_top = track.top() + travel * (scroll_top / max_scroll_top);
        ScrollbarLayout {
            track,
            thumb: Bounds::new(point(track.left(), thumb_top), size(vbar, thumb_height)),
            ratio: max_scroll_top / travel.max(px(1.)),
        }
    });
    let horizontal_scrollbar = show_hbar.then(|| {
        let track = Bounds::new(
            point(text_bounds.left(), bounds.bottom() - hbar),
            size(text_width, hbar),
        );
        let thumb_width = (text_width * (text_width / content_width)).max(px(MIN_SLIDER));
        let travel = text_width - thumb_width;
        let thumb_left = track.left()
            + if max_scroll_left > px(0.) {
                travel * (scroll_left / max_scroll_left)
            } else {
                px(0.)
            };
        ScrollbarLayout {
            track,
            thumb: Bounds::new(point(thumb_left, track.top()), size(thumb_width, hbar)),
            ratio: max_scroll_left / travel.max(px(1.)),
        }
    });

    // Fold controls and placeholders.
    let mut fold_controls = Vec::new();
    let mut fold_placeholders = Vec::new();
    if options.folding {
        let buffer = editor.model().read(cx).buffer().clone();
        let collapsed = editor.folds.collapsed_rows(&buffer);
        let left = bounds.left() + glyph_width + numbers_width + px(LINE_DECORATIONS_WIDTH);
        for line in lines.iter().filter(|line| line.columns.start == 0) {
            let top = bounds.top() + line_height * line.view_line as f32 - scroll_top;
            let is_collapsed = collapsed.iter().any(|(start, _)| *start == line.row);
            if is_collapsed || editor.folds.range_starting_at(line.row).is_some() {
                fold_controls.push(FoldControl {
                    row: line.row,
                    bounds: Bounds::new(point(left, top), size(folding_width, line_height)),
                    collapsed: is_collapsed,
                });
            }
        }
        for line in lines.iter().filter(|line| line.is_last_segment) {
            if collapsed.iter().any(|(start, _)| *start == line.row) {
                let top = bounds.top() + line_height * line.view_line as f32 - scroll_top;
                let x = text_bounds.left() - scroll_left + line.width() + metrics.em_width / 2.;
                fold_placeholders.push((x, top));
            }
        }
    }
    let show_fold_controls = match options.show_folding_controls {
        ShowFoldingControls::Always => true,
        ShowFoldingControls::Mouseover => editor.gutter_hovered,
        ShowFoldingControls::Never => false,
    };

    let layout = EditorLayout {
        bounds,
        gutter_width,
        text_bounds,
        metrics: metrics.clone(),
        scroll_top,
        scroll_left,
        lines,
        fold_controls,
        vertical_scrollbar,
        horizontal_scrollbar,
    };

    // Cursors.
    let mut cursor_quads = Vec::new();
    let mut cursor_view_lines = Vec::new();
    for selection in &selections {
        let head = selection.head;
        let (view_line, _) = view_lines.to_view(head, false);
        cursor_view_lines.push(view_line);
        if let Some(line) = layout
            .view_line(view_line)
            .filter(|line| line.contains(head))
        {
            let x = layout.text_origin_x() + line.x_for_column(head.column);
            let y = layout.line_top(line.view_line);
            let char_width = if head.column < line.columns.end {
                (line.x_for_column(head.column + 1) - line.x_for_column(head.column))
                    .max(metrics.em_width / 2.)
            } else {
                metrics.em_width
            };
            let bounds = match options.cursor_style {
                CursorStyle::Line => {
                    Bounds::new(point(x, y), size(options.cursor_width, line_height))
                }
                CursorStyle::LineThin => Bounds::new(point(x, y), size(px(1.), line_height)),
                CursorStyle::Block => Bounds::new(point(x, y), size(char_width, line_height)),
                CursorStyle::Underline => {
                    Bounds::new(point(x, y + line_height - px(2.)), size(char_width, px(2.)))
                }
            };
            cursor_quads.push(bounds);
        }
    }

    // Gutter numbers: on the first view line of each row.
    let mut gutter_numbers = Vec::new();
    if options.line_numbers != LineNumbers::Off {
        let right = bounds.left() + glyph_width + numbers_width;
        for line in layout.lines.iter().filter(|line| line.columns.start == 0) {
            let row = line.row;
            let is_active = selections.iter().any(|s| s.head.row == row);
            let label = match options.line_numbers {
                LineNumbers::Relative if !is_active => primary.row.abs_diff(row).to_string(),
                LineNumbers::Interval if !is_active && (row + 1) % 10 != 0 && row != 0 => {
                    continue;
                }
                _ => (row + 1).to_string(),
            };
            let color = if is_active {
                colors.active_line_number
            } else {
                colors.line_number
            };
            let shaped = window.text_system().shape_line(
                label.clone().into(),
                metrics.font_size,
                &[run(label.len(), &metrics.font, color)],
                None,
            );
            gutter_numbers.push((layout.line_top(line.view_line), shaped, right));
        }
    }

    let focused = editor.focus_handle.is_focused(window);
    editor.layout = Some(layout.clone());
    PrepaintState {
        layout,
        selections,
        cursor_view_lines,
        cursor_quads,
        gutter_numbers,
        fold_placeholders,
        show_fold_controls,
        colors,
        options,
        focused,
        cursor_visible: editor.cursor_visible,
    }
}

fn paint_editor(state: &mut PrepaintState, window: &mut Window, cx: &mut App) {
    let layout = &state.layout;
    let colors = &state.colors;
    let bounds = layout.bounds;
    let line_height = layout.metrics.line_height;

    window.paint_quad(fill(bounds, colors.background));
    let gutter = Bounds::new(bounds.origin, size(layout.gutter_width, bounds.size.height));
    window.paint_quad(fill(gutter, colors.gutter_background));

    let has_selection = state.selections.iter().any(|s| !s.is_empty());
    let highlight_line = matches!(
        state.options.render_line_highlight,
        RenderLineHighlight::Line | RenderLineHighlight::All
    );
    let highlight_gutter = matches!(
        state.options.render_line_highlight,
        RenderLineHighlight::Gutter | RenderLineHighlight::All
    );

    window.with_content_mask(Some(ContentMask { bounds }), |window| {
        // Current line highlight (Monaco only draws it without a selection).
        if !has_selection && (highlight_line || highlight_gutter) {
            let top = layout.line_top(state.cursor_view_lines[0]);
            let left = if highlight_gutter {
                bounds.left()
            } else {
                layout.text_bounds.left()
            };
            let line_bounds = Bounds::new(
                point(left, top),
                size(layout.text_bounds.right() - left, line_height),
            );
            if let Some(background) = colors.line_highlight_background {
                window.paint_quad(fill(line_bounds, background));
            }
            if let Some(border) = colors.line_highlight_border {
                window.paint_quad(quad(
                    line_bounds,
                    Corners::default(),
                    gpui::transparent_black(),
                    Edges::all(px(2.)),
                    border,
                    BorderStyle::Solid,
                ));
            }
        }

        // Gutter numbers.
        for (top, shaped, right) in &state.gutter_numbers {
            let origin = point(*right - shaped.width, *top);
            shaped
                .paint(origin, line_height, TextAlign::Left, None, window, cx)
                .ok();
        }

        // Fold controls: always for collapsed regions, otherwise per option.
        for control in &layout.fold_controls {
            if control.collapsed || state.show_fold_controls {
                paint_chevron(control, colors.line_number, window);
            }
        }

        window.with_content_mask(
            Some(ContentMask {
                bounds: layout.text_bounds,
            }),
            |window| {
                let selection_color = if state.focused {
                    colors.selection_background
                } else {
                    colors.inactive_selection_background
                };
                let origin_x = layout.text_origin_x();
                for selection in &state.selections {
                    if selection.is_empty() {
                        continue;
                    }
                    paint_selection(layout, selection, origin_x, selection_color, window);
                }

                for line in &layout.lines {
                    let top = layout.line_top(line.view_line);
                    line.shaped
                        .paint(
                            point(origin_x + line.indent_x, top),
                            line_height,
                            TextAlign::Left,
                            None,
                            window,
                            cx,
                        )
                        .ok();
                }

                // Folded region placeholder ("⋯").
                for (x, top) in &state.fold_placeholders {
                    paint_fold_placeholder(*x, *top, layout, colors, window);
                }

                if state.focused && state.cursor_visible {
                    for cursor in &state.cursor_quads {
                        window.paint_quad(fill(*cursor, colors.cursor));
                    }
                }
            },
        );

        for scrollbar in [layout.vertical_scrollbar, layout.horizontal_scrollbar]
            .into_iter()
            .flatten()
        {
            window.paint_quad(fill(scrollbar.thumb, colors.scrollbar_slider));
        }
        // Monaco draws a thin shadow under the top edge once scrolled.
        if layout.scroll_top > px(0.) {
            let shadow = Bounds::new(
                point(bounds.left(), bounds.top()),
                Size::new(bounds.size.width, px(1.)),
            );
            window.paint_quad(fill(shadow, colors.scrollbar_shadow.opacity(0.6)));
        }
    });
}

fn paint_fold_placeholder(
    x: Pixels,
    top: Pixels,
    layout: &EditorLayout,
    colors: &EditorColors,
    window: &mut Window,
) {
    let line_height = layout.metrics.line_height;
    let width = layout.metrics.em_width * 2.;
    let box_bounds = Bounds::new(
        point(x, top + line_height * 0.15),
        size(width, line_height * 0.7),
    );
    window.paint_quad(quad(
        box_bounds,
        Corners::all(px(3.)),
        colors.selection_background.opacity(0.4),
        Edges::default(),
        gpui::transparent_black(),
        BorderStyle::Solid,
    ));
    let dot = px(2.);
    for i in 0..3 {
        let x = box_bounds.left() + width * (0.25 + 0.25 * i as f32) - dot / 2.;
        let y = box_bounds.center().y - dot / 2.;
        window.paint_quad(quad(
            Bounds::new(point(x, y), size(dot, dot)),
            Corners::all(dot / 2.),
            colors.foreground.opacity(0.7),
            Edges::default(),
            gpui::transparent_black(),
            BorderStyle::Solid,
        ));
    }
}

/// A chevron: pointing down when expanded, right when collapsed.
fn paint_chevron(control: &FoldControl, color: Hsla, window: &mut Window) {
    let center = control.bounds.center();
    let half = control.bounds.size.width.min(control.bounds.size.height) * 0.18;
    let mut path = PathBuilder::stroke(px(1.5));
    if control.collapsed {
        path.move_to(point(center.x - half / 2., center.y - half));
        path.line_to(point(center.x + half / 2., center.y));
        path.line_to(point(center.x - half / 2., center.y + half));
    } else {
        path.move_to(point(center.x - half, center.y - half / 2.));
        path.line_to(point(center.x, center.y + half / 2.));
        path.line_to(point(center.x + half, center.y - half / 2.));
    }
    if let Ok(path) = path.build() {
        window.paint_path(path, color);
    }
}

fn paint_selection(
    layout: &EditorLayout,
    selection: &Selection,
    origin_x: Pixels,
    color: Hsla,
    window: &mut Window,
) {
    let start: PointUtf16 = selection.start();
    let end: PointUtf16 = selection.end();
    let eol_width = layout.metrics.em_width / 2.;
    for line in &layout.lines {
        let row = line.row;
        if row < start.row || row > end.row {
            continue;
        }
        // Selected columns of this row, clipped to this view line.
        let sel_start = if row == start.row { start.column } else { 0 };
        let sel_end = if row == end.row { end.column } else { u32::MAX };
        if sel_end < line.columns.start || sel_start > line.columns.end {
            continue;
        }
        let from = line.x_for_column(sel_start.max(line.columns.start));
        let to = if sel_end > line.columns.end {
            // Continues past this view line.
            line.width()
                + if line.is_last_segment {
                    eol_width
                } else {
                    px(0.)
                }
        } else {
            line.x_for_column(sel_end)
        };
        if to > from {
            let bounds = Bounds::new(
                point(origin_x + from, layout.line_top(line.view_line)),
                size(to - from, layout.metrics.line_height),
            );
            window.paint_quad(fill(bounds, color));
        }
    }
}
