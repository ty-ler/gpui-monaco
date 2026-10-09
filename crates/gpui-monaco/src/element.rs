//! The [`EditorElement`]: lays out and paints only the visible rows.

use std::ops::Range;

use gpui::{
    App, BorderStyle, Bounds, ContentMask, Corners, Edges, Element, ElementId, ElementInputHandler,
    Entity, Font, GlobalElementId, Hsla, InspectorElementId, IntoElement, LayoutId, Pixels,
    ShapedLine, SharedString, Size, Style, TextAlign, TextRun, UnderlineStyle, Window, fill, font,
    point, px, quad, relative, size,
};
use text::{BufferSnapshot, PointUtf16, Rope};

use crate::cursor::Selection;
use crate::editor::CodeEditor;
use crate::options::{CursorStyle, EditorOptions, LineNumbers, RenderLineHighlight};
use crate::theme::EditorColors;

/// Width of the glyph margin when enabled (Monaco `glyphMarginWidth`).
const GLYPH_MARGIN_WIDTH: f32 = 18.;
/// Monaco `lineDecorationsWidth` default.
const LINE_DECORATIONS_WIDTH: f32 = 10.;
/// Minimum scrollbar slider length (Monaco `minimumSliderSize`... rounded).
const MIN_SLIDER: f32 = 20.;

/// One shaped buffer row and the mapping from UTF-16 columns to positions in
/// the shaped (tab-expanded) text.
#[derive(Clone)]
pub(crate) struct LineLayout {
    pub row: u32,
    pub shaped: ShapedLine,
    /// `display_index[col]` is the byte index into the shaped text for UTF-16
    /// column `col`, for `col` in `0..=line_len_utf16`.
    pub display_index: Vec<usize>,
}

impl LineLayout {
    pub fn x_for_column(&self, column: u32) -> Pixels {
        let ix = (column as usize).min(self.display_index.len() - 1);
        self.shaped.x_for_index(self.display_index[ix])
    }

    pub fn column_for_x(&self, x: Pixels) -> u32 {
        let ix = self.shaped.closest_index_for_x(x);
        let col = self.display_index.partition_point(|&d| d < ix);
        let col = col.min(self.display_index.len() - 1);
        // Never land between the halves of a surrogate pair.
        if col > 0 && self.display_index[col] == self.display_index[col - 1] {
            (col - 1) as u32
        } else {
            col as u32
        }
    }

    pub fn width(&self) -> Pixels {
        self.shaped.width
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

/// Shapes one buffer row. `marked` is an IME composition range in UTF-16
/// columns on this row, drawn underlined.
pub(crate) fn layout_row(
    rope: &Rope,
    row: u32,
    tab_size: u32,
    metrics: &TextMetrics,
    color: Hsla,
    marked: Option<(Range<u32>, Hsla)>,
    window: &mut Window,
) -> LineLayout {
    let line = rope.line(row);
    let tab_size = tab_size.max(1) as usize;
    let mut display = String::with_capacity(line.len());
    let mut display_index = Vec::with_capacity(line.len() + 1);
    let mut visible_column = 0usize;
    for ch in line.chars() {
        for _ in 0..ch.len_utf16() {
            display_index.push(display.len());
        }
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

    let mut runs = Vec::new();
    match marked {
        Some((range, underline)) if !range.is_empty() => {
            let start = display_index[(range.start as usize).min(display_index.len() - 1)];
            let end = display_index[(range.end as usize).min(display_index.len() - 1)];
            if start > 0 {
                runs.push(run(start, &metrics.font, color));
            }
            if end > start {
                let mut marked_run = run(end - start, &metrics.font, color);
                marked_run.underline = Some(UnderlineStyle {
                    color: Some(underline),
                    thickness: px(1.),
                    wavy: false,
                });
                runs.push(marked_run);
            }
            if display.len() > end {
                runs.push(run(display.len() - end, &metrics.font, color));
            }
        }
        _ => {
            if !display.is_empty() {
                runs.push(run(display.len(), &metrics.font, color));
            }
        }
    }
    let shaped = window.text_system().shape_line(
        SharedString::from(display),
        metrics.font_size,
        &runs,
        None,
    );
    LineLayout {
        row,
        shaped,
        display_index,
    }
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct ScrollbarLayout {
    pub track: Bounds<Pixels>,
    pub thumb: Bounds<Pixels>,
    /// Content pixels per track pixel.
    pub ratio: f32,
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
    pub vertical_scrollbar: Option<ScrollbarLayout>,
    pub horizontal_scrollbar: Option<ScrollbarLayout>,
}

impl EditorLayout {
    /// Visible rows that fit fully, for page up/down.
    pub fn page_rows(&self) -> u32 {
        ((self.text_bounds.size.height / self.metrics.line_height).floor() as u32).max(1)
    }

    pub fn line(&self, row: u32) -> Option<&LineLayout> {
        let first = self.lines.first()?.row;
        self.lines.get(row.checked_sub(first)? as usize)
    }

    /// X of the start of text (column 0) in window coordinates.
    pub fn text_origin_x(&self) -> Pixels {
        self.text_bounds.left() - self.scroll_left
    }

    pub fn row_top(&self, row: u32) -> Pixels {
        self.bounds.top() + self.metrics.line_height * row as f32 - self.scroll_top
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
    primary_row: u32,
    cursor_quads: Vec<Bounds<Pixels>>,
    gutter_numbers: Vec<(Pixels, ShapedLine, Pixels)>,
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
    let colors = editor.colors.clone();
    let metrics = TextMetrics::new(&options, window);
    let line_height = metrics.line_height;
    let snapshot: BufferSnapshot = editor.snapshot.clone();
    let tab_size = editor.cursor_config(cx).tab_size;
    let line_count = snapshot.line_count();

    // Gutter: [glyph margin][line numbers][line decorations].
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
    let gutter_width = (glyph_width + numbers_width + px(LINE_DECORATIONS_WIDTH)).round();

    let vbar = options.vertical_scrollbar_size;
    let hbar = options.horizontal_scrollbar_size;
    let viewport_height = bounds.size.height;
    let content_height = line_height * line_count as f32
        + if options.scroll_beyond_last_line {
            (viewport_height - line_height).max(px(0.))
        } else {
            px(0.)
        };
    let max_scroll_top = (content_height - viewport_height).max(px(0.));

    // Rough content width from the longest row; refined by shaped rows below.
    let longest = snapshot.summary().longest_row_chars as f32;
    let text_width = bounds.size.width - gutter_width - vbar;
    let mut content_width = metrics.em_width * (longest + 2.);
    let show_hbar = content_width > text_width;
    let text_height = if show_hbar {
        viewport_height - hbar
    } else {
        viewport_height
    };

    // Vertical autoscroll before choosing rows.
    let selections = editor.cursors.selections();
    let primary = selections[0].head;
    if editor.autoscroll_requested {
        let top = line_height * primary.row as f32;
        let bottom = top + line_height;
        if top < editor.scroll_top {
            editor.scroll_top = top;
        } else if bottom > editor.scroll_top + text_height {
            editor.scroll_top = bottom - text_height;
        }
    }
    editor.scroll_top = editor.scroll_top.clamp(px(0.), max_scroll_top);
    let scroll_top = editor.scroll_top;

    let first_row = (scroll_top / line_height).floor() as u32;
    let last_row = (((scroll_top + viewport_height) / line_height).ceil() as u32)
        .min(line_count.saturating_sub(1));

    let marked = editor.marked_range_points();
    let mut lines = Vec::with_capacity((last_row + 1).saturating_sub(first_row) as usize);
    for row in first_row..=last_row {
        let marked_cols = marked.as_ref().and_then(|range| {
            (range.start.row == row && range.end.row == row)
                .then_some((range.start.column..range.end.column, colors.ime_underline))
        });
        let line = layout_row(
            &snapshot,
            row,
            tab_size,
            &metrics,
            colors.foreground,
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
            .find(|line| line.row == primary.row)
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

    let layout = EditorLayout {
        bounds,
        gutter_width,
        text_bounds,
        metrics: metrics.clone(),
        scroll_top,
        scroll_left,
        lines,
        vertical_scrollbar,
        horizontal_scrollbar,
    };

    // Cursors.
    let mut cursor_quads = Vec::new();
    for selection in &selections {
        let head = selection.head;
        if let Some(line) = layout.line(head.row) {
            let x = layout.text_origin_x() + line.x_for_column(head.column);
            let y = layout.row_top(head.row);
            let char_width = if head.column < line.display_index.len() as u32 - 1 {
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

    // Gutter numbers.
    let mut gutter_numbers = Vec::new();
    if options.line_numbers != LineNumbers::Off {
        let right = bounds.left() + glyph_width + numbers_width;
        for line in &layout.lines {
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
            gutter_numbers.push((layout.row_top(row), shaped, right));
        }
    }

    let focused = editor.focus_handle.is_focused(window);
    editor.layout = Some(layout.clone());
    PrepaintState {
        layout,
        selections,
        primary_row: primary.row,
        cursor_quads,
        gutter_numbers,
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
            let top = layout.row_top(state.primary_row);
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
                    let origin = point(origin_x, layout.row_top(line.row));
                    line.shaped
                        .paint(origin, line_height, TextAlign::Left, None, window, cx)
                        .ok();
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
        let from = if row == start.row {
            line.x_for_column(start.column)
        } else {
            px(0.)
        };
        let to = if row == end.row {
            line.x_for_column(end.column)
        } else {
            line.width() + eol_width
        };
        if to > from {
            let bounds = Bounds::new(
                point(origin_x + from, layout.row_top(row)),
                size(to - from, layout.metrics.line_height),
            );
            window.paint_quad(fill(bounds, color));
        }
    }
}
