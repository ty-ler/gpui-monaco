//! The [`EditorElement`]: lays out and paints only the visible view lines.

use std::ops::Range;

use gpui::{
    App, AvailableSpace, BorderStyle, Bounds, ContentMask, Corners, Edges, Element, ElementId,
    ElementInputHandler, Entity, Font, FontStyle, FontWeight, GlobalElementId, Hsla,
    InspectorElementId, IntoElement, LayoutId, PathBuilder, Pixels, ShapedLine, SharedString, Size,
    Style, TextAlign, TextRun, UnderlineStyle, Window, fill, font, point, px, quad, relative, size,
};
use text::{PointUtf16, Rope};

use crate::cursor::Selection;
use crate::editor::CodeEditor;
use crate::options::{
    CursorStyle, EditorOptions, LineNumbers, RenderLineHighlight, ShowFoldingControls, WordWrap,
};
use crate::theme::{EditorColors, Theme, TokenStyle};
use crate::view::view_lines::{ViewLine, ViewLines, WrapSettings, utf16_to_byte};
use crate::widgets::{ContentWidgetPreference, OverlayPosition};

/// Width of the glyph margin when enabled (Monaco `glyphMarginWidth`).
const GLYPH_MARGIN_WIDTH: f32 = 18.;
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
    /// Injected (decoration before/after) text: display byte range and
    /// background.
    /// Injected text: display range, background and swatch.
    pub injected: Vec<(Range<usize>, Option<Hsla>, Option<Hsla>)>,
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
    /// A space's width: wrapped lines' indentation.
    pub space_width: Pixels,
    /// Show control characters as their Unicode control pictures.
    pub render_control_characters: bool,
}

impl TextMetrics {
    pub fn new(options: &EditorOptions, window: &mut Window) -> Self {
        let font = Font {
            weight: options.font_weight,
            features: options.font_ligatures.features(),
            ..font(options.font_family.clone())
        };
        let font_size = options.font_size;
        let width = |text: &'static str| {
            window
                .text_system()
                .shape_line(
                    text.into(),
                    font_size,
                    &[run(1, &font, gpui::black())],
                    None,
                )
                .width
        };
        let em_width = width("m");
        let space_width = width(" ");
        Self {
            font,
            font_size,
            line_height: options.resolved_line_height(),
            em_width,
            space_width,
            render_control_characters: options.render_control_characters,
        }
    }
}

/// The editor font's character advances in columns (Monaco measures them
/// in the DOM for `wrappingStrategy: advanced`).
pub(crate) struct CharWidths {
    text_system: std::sync::Arc<gpui::TextSystem>,
    font: Font,
    font_id: gpui::FontId,
    font_size: Pixels,
    column_width: Pixels,
    ascii: [f32; 128],
    max_ascii: f32,
    other: std::sync::Mutex<std::collections::HashMap<char, f32>>,
}

impl CharWidths {
    fn new(metrics: &TextMetrics, cx: &App) -> Self {
        let text_system = cx.text_system().clone();
        let font_id = text_system.resolve_font(&metrics.font);
        let mut this = Self {
            text_system,
            font: metrics.font.clone(),
            font_id,
            font_size: metrics.font_size,
            column_width: metrics.em_width.max(px(1.)),
            ascii: [0.; 128],
            max_ascii: 0.,
            other: Default::default(),
        };
        for code in 32..128u8 {
            let columns = this.measure(code as char);
            this.ascii[code as usize] = columns;
            this.max_ascii = this.max_ascii.max(columns);
        }
        this
    }

    /// The widths for `metrics`, reusing `cached` when it's for the same
    /// font.
    pub(crate) fn for_metrics(
        cached: &Option<std::sync::Arc<Self>>,
        metrics: &TextMetrics,
        cx: &App,
    ) -> std::sync::Arc<Self> {
        match cached {
            Some(widths)
                if widths.font == metrics.font
                    && widths.font_size == metrics.font_size
                    && widths.column_width == metrics.em_width.max(px(1.)) =>
            {
                widths.clone()
            }
            _ => std::sync::Arc::new(Self::new(metrics, cx)),
        }
    }

    fn measure(&self, ch: char) -> f32 {
        let advance = self
            .text_system
            .advance(self.font_id, self.font_size, ch)
            .map_or(self.column_width, |size| size.width);
        advance / self.column_width
    }
}

impl crate::view::line_breaks::MeasureChar for CharWidths {
    fn columns(&self, ch: char) -> f32 {
        if (ch as u32) < 128 {
            return self.ascii[ch as usize];
        }
        let mut other = self.other.lock().unwrap_or_else(|e| e.into_inner());
        *other.entry(ch).or_insert_with(|| self.measure(ch))
    }

    fn max_ascii_columns(&self) -> f32 {
        self.max_ascii
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

/// The Unicode control picture for a C0 control character or DEL.
fn control_picture(ch: char) -> Option<char> {
    match ch {
        '\t' => None,
        '\u{0}'..='\u{1f}' => char::from_u32(0x2400 + ch as u32),
        '\u{7f}' => Some('\u{2421}'),
        _ => None,
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

/// Injected text being laid out: display range, style, background and
/// swatch.
type Injected = (Range<usize>, TokenStyle, Option<Hsla>, Option<Hsla>);

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
    injections: &[(u32, bool, crate::decorations::InjectedText)],
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
    // (display range, style, background) of injected text.
    let mut injected: Vec<Injected> = Vec::new();
    let mut injection_ix = 0;
    // Emits injections at `column` (`after` ones first) and returns where the
    // column itself displays: before `after` text (it ends a decorated range)
    // or after `before` text (it starts one).
    let mut inject_at =
        |column: u32, display: &mut String, injected: &mut Vec<Injected>| -> usize {
            let first = injection_ix;
            while injection_ix < injections.len() && injections[injection_ix].0 <= column {
                injection_ix += 1;
            }
            let position_before = display.len();
            let mut has_after = false;
            for is_after in [true, false] {
                for (at, after, text) in &injections[first..injection_ix] {
                    if *at == column && *after == is_after {
                        has_after |= *after;
                        let start = display.len();
                        display.push_str(&text.content);
                        injected.push((
                            start..display.len(),
                            text.style,
                            text.background,
                            text.swatch,
                        ));
                    }
                }
            }
            if has_after {
                position_before
            } else {
                display.len()
            }
        };
    let mut column = segment.columns.start;
    for (byte, ch) in line.char_indices() {
        let position = inject_at(column, &mut display, &mut injected);
        column += ch.len_utf16() as u32;
        display_index.push(position);
        for _ in 1..ch.len_utf16() {
            display_index.push(display.len());
        }
        char_starts.push((byte_start + byte, display.len()));
        if ch == '\t' {
            let spaces = tab_size - visible_column % tab_size;
            display.extend(std::iter::repeat_n(' ', spaces));
            visible_column += spaces;
        } else if let Some(picture) =
            control_picture(ch).filter(|_| metrics.render_control_characters)
        {
            // Monaco `renderControlCharacters`: `\u{7}` shows as `␇`.
            display.push(picture);
            visible_column += 1;
        } else {
            display.push(ch);
            visible_column += 1;
        }
    }
    let end_position = if segment.is_last_segment {
        inject_at(column, &mut display, &mut injected)
    } else {
        display.len()
    };
    display_index.push(end_position);
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
    for (range, _, _, _) in &injected {
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
        let style = injected
            .iter()
            .find(|(range, _, _, _)| range.start <= start && end <= range.end)
            .map(|(_, style, _, _)| *style)
            .or_else(|| {
                styled
                    .get(span_ix)
                    .filter(|(range, _)| range.start <= start)
                    .map(|(_, style)| *style)
            })
            .unwrap_or_default();
        let mut font = metrics.font.clone();
        if style.bold {
            font.weight = FontWeight::BOLD;
        }
        if style.italic {
            font.style = FontStyle::Italic;
        }
        let mut foreground = style.foreground.unwrap_or(color);
        if let Some(opacity) = style.opacity {
            foreground.a *= opacity;
        }
        let mut text_run = run(end - start, &font, foreground);
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
        indent_x: metrics.space_width * segment.indent as f32,
        shaped,
        display_index,
        injected: injected
            .into_iter()
            .map(|(range, _, background, swatch)| (range, background, swatch))
            .collect(),
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
    HighlightSource::new(model).map_or_else(Vec::new, |source| source.highlights(theme, range))
}

/// The latest highlights of a model, detached from it so they can be read on
/// a background thread.
pub(crate) struct HighlightSource {
    syntax: crate::model::SyntaxSnapshot,
    /// Edits since `syntax` was parsed, if any.
    patch: Option<text::Patch>,
}

impl HighlightSource {
    pub(crate) fn new(model: &crate::model::TextModel) -> Option<Self> {
        let syntax = model.syntax_snapshot()?;
        let buffer = model.buffer();
        let patch = (syntax.text.version() != buffer.version())
            .then(|| buffer.edits_since(syntax.text.version()));
        Some(Self { syntax, patch })
    }

    /// Styled spans in `range` of the current text, with the parsed
    /// highlights shifted through any newer edits.
    pub(crate) fn highlights(
        &self,
        theme: &Theme,
        range: Range<usize>,
    ) -> Vec<(Range<usize>, TokenStyle)> {
        let syntax = &self.syntax;
        let style = |span: &syntax::HighlightSpan| theme.style_for(span.highlight);
        let Some(patch) = &self.patch else {
            return syntax
                .highlighter
                .highlights(&syntax.state, syntax.text.rope(), range)
                .iter()
                .map(|span| (span.range.clone(), style(span)))
                .collect();
        };
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
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct ScrollbarLayout {
    /// The whole scrollbar, arrows included.
    pub bar: Bounds<Pixels>,
    /// Where the slider moves.
    pub track: Bounds<Pixels>,
    pub thumb: Bounds<Pixels>,
    /// Content pixels per track pixel.
    pub ratio: f32,
    /// The arrow buttons (start, end), with `*_has_arrows`.
    pub arrows: Option<[Bounds<Pixels>; 2]>,
    /// Whether it takes clicks (`ScrollbarVisibility::Hidden` doesn't).
    pub interactive: bool,
}

/// Where the minimap is and how it maps to content.
#[derive(Clone, Copy, Debug)]
pub(crate) struct MinimapLayout {
    pub bounds: Bounds<Pixels>,
    pub slider: Bounds<Pixels>,
    /// Content scroll pixels per slider pixel when dragging.
    pub drag_ratio: f32,
    pub line_height: Pixels,
    /// Minimap pixels scrolled off the top.
    pub scroll: Pixels,
}

/// Cached minimap blocks for one view of the minimap.
#[derive(Clone, Default)]
pub(crate) struct MinimapCache {
    key: Option<MinimapKey>,
    /// The view line `blocks` start at.
    first: u32,
    /// The rendered lines from `first` on (Monaco draws the minimap into a
    /// canvas too), with its size in device pixels.
    image: Option<(std::sync::Arc<gpui::RenderImage>, (u32, u32))>,
    /// Replaced images, to free from the atlas at the next paint.
    stale: Vec<std::sync::Arc<gpui::RenderImage>>,
    /// A rebuild running in the background; until it lands the old blocks
    /// are drawn (a frame-stale minimap is invisible, ~3ms per keystroke on
    /// the main thread is not).
    pending: Option<(MinimapKey, std::sync::Arc<gpui::Task<()>>)>,
}

/// (text, parsed text, first line, last line, theme, wrap column, render
/// settings): it's redrawn once the syntax tree catches up, for colors.
type MinimapKey = (
    text::Version,
    Option<text::Version>,
    u32,
    u32,
    usize,
    u32,
    [u32; 4],
);

/// A fold control in the gutter.
#[derive(Clone, Copy, Debug)]
pub(crate) struct FoldControl {
    pub row: u32,
    pub bounds: Bounds<Pixels>,
    pub collapsed: bool,
}

/// A sticky scroll line, for hit testing.
#[derive(Clone, Copy, Debug)]
pub(crate) struct StickyLine {
    /// The last row the scope keeps the header for.
    pub end_row: u32,
    /// The row shown (the scope's end with Shift held over it).
    pub row: u32,
    pub bounds: Bounds<Pixels>,
    /// Its folding control.
    pub fold: Option<FoldControl>,
}

/// Vertical positions with view zones: zone `i` sits after view line
/// `after` (-1 for above the first line).
#[derive(Clone, Debug, Default)]
pub(crate) struct ZoneLayout {
    /// (after view line, height), sorted by `after`.
    zones: Vec<(i64, Pixels)>,
    /// `heights_before[i]`: the total height of zones `..i`.
    heights_before: Vec<Pixels>,
    /// View lines taller (or shorter) than the line height (Monaco
    /// `LineHeightsManager`): (view line, extra height), sorted.
    lines: Vec<(u32, Pixels)>,
    /// `extra_before[i]`: the total extra height of `lines[..i]`.
    extra_before: Vec<Pixels>,
}

impl ZoneLayout {
    pub fn new(mut zones: Vec<(i64, Pixels)>) -> Self {
        zones.sort_by_key(|(after, _)| *after);
        let mut heights_before = Vec::with_capacity(zones.len() + 1);
        let mut total = px(0.);
        heights_before.push(total);
        for (_, height) in &zones {
            total += *height;
            heights_before.push(total);
        }
        Self {
            zones,
            heights_before,
            lines: Vec::new(),
            extra_before: vec![px(0.)],
        }
    }

    /// With view lines of other heights: (view line, extra height), in any
    /// order and with no duplicates.
    pub fn with_line_heights(mut self, mut lines: Vec<(u32, Pixels)>) -> Self {
        lines.sort_by_key(|(line, _)| *line);
        let mut total = px(0.);
        self.extra_before = Vec::with_capacity(lines.len() + 1);
        self.extra_before.push(total);
        for (_, extra) in &lines {
            total += *extra;
            self.extra_before.push(total);
        }
        self.lines = lines;
        self
    }

    /// The height of zones and line height changes.
    pub fn total(&self) -> Pixels {
        self.heights_before.last().copied().unwrap_or_default()
            + self.extra_before.last().copied().unwrap_or_default()
    }

    /// The height of view line `line`.
    pub fn height(&self, line: u32, line_height: Pixels) -> Pixels {
        match self.lines.binary_search_by_key(&line, |(line, _)| *line) {
            Ok(ix) => line_height + self.lines[ix].1,
            Err(_) => line_height,
        }
    }

    /// Total height of zones above view line `line` (a binary search: there
    /// may be thousands, e.g. code lenses).
    pub fn above(&self, line: u32) -> Pixels {
        let count = self
            .zones
            .partition_point(|(after, _)| *after < line as i64);
        self.heights_before.get(count).copied().unwrap_or_default()
    }

    /// Content y of the top of view line `line`.
    pub fn line_y(&self, line: u32, line_height: Pixels) -> Pixels {
        let taller = self.lines.partition_point(|(l, _)| *l < line);
        line_height * line as f32 + self.above(line) + self.extra_before[taller]
    }

    /// The last view line starting at or above content y `y`.
    pub fn line_at(&self, y: Pixels, line_height: Pixels, line_count: u32) -> u32 {
        let (mut lo, mut hi) = (0u32, line_count.saturating_sub(1));
        while lo < hi {
            let mid = (lo + hi).div_ceil(2);
            if self.line_y(mid, line_height) <= y {
                lo = mid;
            } else {
                hi = mid - 1;
            }
        }
        lo
    }
}

/// Monaco `LineHeightsManager`: the extra height of every view line a
/// decoration with a `line_height` covers (the tallest one wins).
fn decoration_line_heights(
    editor: &CodeEditor,
    view_lines: &ViewLines,
    line_height: Pixels,
    editor_id: gpui::EntityId,
    cx: &App,
) -> Vec<(u32, Pixels)> {
    let model = editor.model().read(cx);
    let decorations = model.decoration_line_heights(Some(editor_id));
    if decorations.is_empty() {
        return Vec::new();
    }
    let rope = view_lines.rope();
    let mut heights: std::collections::BTreeMap<u32, Pixels> = Default::default();
    for (range, multiplier) in decorations {
        if multiplier.is_nan() || multiplier <= 0. {
            continue;
        }
        let height = line_height * multiplier;
        let start = rope.offset_to_point_utf16(range.start.min(rope.len()));
        let end = rope.offset_to_point_utf16(range.end.min(rope.len()));
        let first = view_lines.to_view(start, false).0;
        let last = view_lines.to_view(end, true).0;
        for line in first..=last {
            let entry = heights.entry(line).or_insert(height);
            *entry = (*entry).max(height);
        }
    }
    heights
        .into_iter()
        .filter(|(_, height)| *height != line_height)
        .map(|(line, height)| (line, height - line_height))
        .collect()
}

/// Everything about the last frame that event handlers need for hit testing.
#[derive(Clone)]
pub(crate) struct EditorLayout {
    pub bounds: Bounds<Pixels>,
    pub gutter_width: Pixels,
    /// Width of the glyph margin at the gutter's left.
    pub glyph_margin_width: Pixels,
    /// Width of the line numbers after the glyph margin.
    pub line_numbers_width: Pixels,
    /// The text viewport: right of the gutter, excluding scrollbars.
    pub text_bounds: Bounds<Pixels>,
    pub metrics: TextMetrics,
    pub scroll_top: Pixels,
    pub scroll_left: Pixels,
    /// The scroll range: `scroll_top` and `scroll_left` stay within it.
    pub max_scroll: gpui::Point<Pixels>,
    pub lines: Vec<LineLayout>,
    pub fold_controls: Vec<FoldControl>,
    /// The `⋯` after each collapsed line, with its row, for clicks.
    pub fold_placeholders: Vec<(u32, Bounds<Pixels>)>,
    pub zones: ZoneLayout,
    /// Bounds of embedded views; the editor ignores clicks inside them.
    pub widget_bounds: Vec<Bounds<Pixels>>,
    /// What each of `widget_bounds` is.
    pub widget_kinds: Vec<crate::mouse_target::EmbeddedKind>,
    /// The code action lightbulb, for clicks.
    pub lightbulb: Option<Bounds<Pixels>>,
    pub minimap: Option<MinimapLayout>,
    /// Sticky scroll lines.
    pub sticky_lines: Vec<StickyLine>,
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
        self.bounds.top() + self.zones.line_y(view_line, self.metrics.line_height) - self.scroll_top
    }

    /// The height of view line `view_line` (taller with a decoration's
    /// `line_height`).
    pub fn line_height_of(&self, view_line: u32) -> Pixels {
        self.zones.height(view_line, self.metrics.line_height)
    }

    /// Where text-sized things go on view line `view_line`: centered in a
    /// taller line, as Monaco's CSS line height centers its text.
    pub fn text_top(&self, view_line: u32) -> Pixels {
        self.line_top(view_line) + (self.line_height_of(view_line) - self.metrics.line_height) / 2.
    }
}

/// The GPUI element that lays out and paints a [`CodeEditor`]. [`CodeEditor`]
/// renders one itself; use this to embed the editor in a custom element tree.
pub struct EditorElement {
    editor: Entity<CodeEditor>,
}

impl EditorElement {
    /// An element painting `editor`.
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

/// A minimap section header (Monaco `_renderSectionHeaders`).
struct MinimapSectionHeader {
    /// The translucent band behind the label.
    band: Bounds<Pixels>,
    /// The label and where it goes.
    label: Option<(ShapedLine, gpui::Point<Pixels>)>,
    /// The separator line above it.
    separator: Option<Bounds<Pixels>>,
}

/// The `·` and `→` glyphs, and where each goes (`true` for a tab).
type WhitespaceMarks = (ShapedLine, ShapedLine, Vec<(gpui::Point<Pixels>, bool)>);

pub struct PrepaintState {
    layout: EditorLayout,
    selections: Vec<Selection>,
    /// View lines with a cursor (for the current line highlight).
    cursor_view_lines: Vec<u32>,
    cursor_quads: Vec<Bounds<Pixels>>,
    gutter_numbers: Vec<(Pixels, ShapedLine, Pixels)>,
    /// (x after the text, top) of rows whose region is collapsed.
    fold_placeholders: Vec<(Pixels, Pixels)>,
    fold_highlights: Vec<Bounds<Pixels>>,
    show_fold_controls: bool,
    decoration_quads: Vec<(Bounds<Pixels>, Option<Hsla>, Option<Hsla>)>,
    decoration_underlines: Vec<(gpui::Point<Pixels>, Pixels, UnderlineStyle)>,
    gutter_bars: Vec<(Bounds<Pixels>, Hsla)>,
    glyphs: Vec<(gpui::Point<Pixels>, ShapedLine)>,
    lightbulb: Option<(Bounds<Pixels>, Hsla)>,
    ruler_marks: Vec<(Bounds<Pixels>, Hsla)>,
    /// Indent guides and rulers.
    guides: Vec<(Bounds<Pixels>, Hsla)>,
    /// Rendered whitespace.
    whitespace: Option<WhitespaceMarks>,
    embedded: Vec<Embedded>,
    section_headers: Vec<MinimapSectionHeader>,
    /// The placeholder's lines and where they go.
    placeholder: Vec<(ShapedLine, gpui::Point<Pixels>)>,
    /// Where a dragged selection would drop.
    dnd_target: Option<Bounds<Pixels>>,
    /// The minimap's image and where it goes; replaced images to free.
    minimap_image: Option<(std::sync::Arc<gpui::RenderImage>, Bounds<Pixels>)>,
    minimap_stale: Vec<std::sync::Arc<gpui::RenderImage>>,
    minimap_slider_visible: bool,
    sticky: Vec<(usize, Pixels, LineLayout, Option<ShapedLine>)>,
    /// The keyboard-focused and hovered sticky lines.
    sticky_focused: Option<usize>,
    sticky_hovered: Option<usize>,
    sticky_scroll_with_editor: bool,
    numbers_right: Pixels,
    zone_elements: Vec<gpui::AnyElement>,
    full_width_zone_elements: Vec<gpui::AnyElement>,
    widget_elements: Vec<gpui::AnyElement>,
    colors: EditorColors,
    options: EditorOptions,
    focused: bool,
    cursor_visible: bool,
    cursor_opacity: f32,
    cursor_scale: f32,
    /// Middle-click scrolling's anchor dot.
    middle_scroll_anchor: Option<gpui::Point<Pixels>>,
    /// The scrollbars' sliders: (opacity, hovered, dragged) for vertical
    /// and horizontal.
    scrollbar_looks: [(f32, bool, bool); 2],
    overview_ruler: Option<Bounds<Pixels>>,
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
        let mut state = self
            .editor
            .update(cx, |editor, cx| prepaint_editor(editor, bounds, window, cx));
        let mut widget_bounds = Vec::new();
        let mut widget_kinds = Vec::new();
        for embedded in std::mem::take(&mut state.embedded) {
            match embedded {
                Embedded::Zone {
                    view,
                    bounds: zone,
                    full_width,
                } => {
                    let mut element = view.into_any_element();
                    element.layout_as_root(
                        size(
                            AvailableSpace::Definite(zone.size.width),
                            AvailableSpace::Definite(zone.size.height),
                        ),
                        window,
                        cx,
                    );
                    element.prepaint_at(zone.origin, window, cx);
                    widget_bounds.push(zone);
                    widget_kinds.push(crate::mouse_target::EmbeddedKind::ViewZone);
                    if full_width {
                        state.full_width_zone_elements.push(element);
                    } else {
                        state.zone_elements.push(element);
                    }
                }
                Embedded::Content {
                    view,
                    anchor,
                    line_height,
                    preference,
                } => {
                    let mut element = view.into_any_element();
                    let measured = element.layout_as_root(
                        size(AvailableSpace::MinContent, AvailableSpace::MinContent),
                        window,
                        cx,
                    );
                    let x = anchor
                        .x
                        .min(bounds.right() - measured.width)
                        .max(bounds.left());
                    let Some(y) = crate::widgets::place_content_widget(
                        &preference,
                        anchor.y,
                        line_height,
                        measured.height,
                        bounds.top()..bounds.bottom(),
                    ) else {
                        continue;
                    };
                    element.prepaint_at(point(x, y), window, cx);
                    widget_bounds.push(Bounds::new(point(x, y), measured));
                    widget_kinds.push(crate::mouse_target::EmbeddedKind::ContentWidget);
                    state.widget_elements.push(element);
                }
                Embedded::Overlay { view, position } => {
                    let mut element = view.into_any_element();
                    let measured = element.layout_as_root(
                        size(AvailableSpace::MinContent, AvailableSpace::MinContent),
                        window,
                        cx,
                    );
                    let text = state.layout.text_bounds;
                    let origin = match position {
                        OverlayPosition::TopRight => {
                            point(text.right() - measured.width, bounds.top())
                        }
                        OverlayPosition::BottomRight => point(
                            text.right() - measured.width,
                            bounds.bottom() - measured.height,
                        ),
                        OverlayPosition::TopCenter => point(
                            text.left() + (text.size.width - measured.width) / 2.,
                            bounds.top(),
                        ),
                        OverlayPosition::Custom(offset) => bounds.origin + offset,
                    };
                    element.prepaint_at(origin, window, cx);
                    widget_bounds.push(Bounds::new(origin, measured));
                    widget_kinds.push(crate::mouse_target::EmbeddedKind::OverlayWidget);
                    state.widget_elements.push(element);
                }
            }
        }
        if !widget_bounds.is_empty() {
            self.editor.update(cx, |editor, _| {
                if let Some(layout) = editor.layout.as_mut() {
                    layout.widget_bounds = widget_bounds;
                    layout.widget_kinds = widget_kinds;
                }
            });
        }
        state
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
        (glyph_width + numbers_width + options.line_decorations_width + folding_width).round();

    let vbar = options.vertical_scrollbar_size;
    let hbar = options.horizontal_scrollbar_size;
    let viewport_height = bounds.size.height;
    let minimap_width = if options.minimap.enabled && !options.single_line {
        px(options.minimap.max_column as f32 * options.minimap.scale).min(bounds.size.width / 4.)
    } else {
        px(0.)
    };
    // Monaco `minimap.side: left`: the minimap comes before the gutter.
    let full_bounds = bounds;
    let minimap_left = options.minimap.side == crate::options::MinimapSide::Left;
    let bounds = if minimap_left {
        Bounds::new(
            point(bounds.left() + minimap_width, bounds.top()),
            size(bounds.size.width - minimap_width, bounds.size.height),
        )
    } else {
        bounds
    };
    let text_width =
        bounds.size.width - gutter_width - vbar - if minimap_left { px(0.) } else { minimap_width };

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
    let measure = (wrap_column.is_some()
        && options.wrapping_strategy == crate::options::WrappingStrategy::Advanced)
        .then(|| {
            let widths = CharWidths::for_metrics(&editor.char_widths, &metrics, cx);
            editor.char_widths = Some(widths.clone());
            crate::view::view_lines::MeasuredWidths(widths)
        });
    editor.sync_view_lines(
        wrap_column.map(|column| WrapSettings {
            column,
            tab_size,
            indent: options.wrapping_indent,
            keep_all: false,
            measure,
        }),
        cx,
    );
    let view_lines = editor.view_lines.clone();
    let view_line_count = view_lines.line_count().max(1);
    let snapshot = editor.snapshot.clone();

    // View zones: after the last view line of their row; hidden in folds.
    let mut zone_specs: Vec<(i64, Pixels, gpui::AnyView, bool)> = Vec::new();
    {
        let model = editor.model().clone();
        let buffer = model.read(cx).buffer();
        let key = (
            std::sync::Arc::as_ptr(&view_lines) as usize,
            buffer.version(),
            editor.widgets.zones_generation,
        );
        let cached = editor
            .widgets
            .zone_rows
            .as_ref()
            .filter(|(k, _)| *k == key)
            .map(|(_, rows)| rows.clone());
        let rows = cached.unwrap_or_else(|| {
            editor
                .widgets
                .zones
                .iter_mut()
                .map(|zone| match &mut zone.anchor {
                    None => Some(-1),
                    Some(anchor) => {
                        // Re-based on this version, so resolving stays cheap
                        // as edits pile up.
                        *anchor = buffer.refresh(anchor);
                        let row = buffer.offset_to_point(buffer.resolve(anchor)).row;
                        if view_lines.is_row_hidden(row) {
                            return None;
                        }
                        let first = view_lines.first_view_line_of_row(row);
                        let next = view_lines.first_view_line_of_row(row + 1);
                        Some(next.max(first + 1) as i64 - 1)
                    }
                })
                .collect()
        });
        for (zone, after) in editor.widgets.zones.iter().zip(&rows) {
            let Some(after) = *after else {
                continue;
            };
            let height = match zone.height {
                crate::widgets::ZoneHeight::Lines(lines) => line_height * lines,
                crate::widgets::ZoneHeight::Pixels(height) => height,
            };
            zone_specs.push((after, height, zone.view.clone(), zone.full_width));
        }
        editor.widgets.zone_rows = Some((key, rows));
    }
    // Monaco `padding.top` comes before everything, view zones above the
    // first line included.
    let mut zone_heights: Vec<(i64, Pixels)> =
        zone_specs.iter().map(|(a, h, _, _)| (*a, *h)).collect();
    if options.padding.top > px(0.) {
        zone_heights.push((-1, options.padding.top));
    }
    let zones =
        ZoneLayout::new(zone_heights).with_line_heights(if options.allow_variable_line_heights {
            decoration_line_heights(editor, &view_lines, line_height, cx.entity_id(), cx)
        } else {
            Vec::new()
        });

    let content_height = line_height * view_line_count as f32
        + zones.total()
        + options.padding.bottom
        + if options.scroll_beyond_last_line {
            (viewport_height - line_height - options.padding.bottom).max(px(0.))
        } else {
            px(0.)
        };
    let max_scroll_top = (content_height - viewport_height).max(px(0.));

    // Rough content width from the longest row; refined by shaped lines below.
    let mut content_width = if wrap_column.is_some() {
        text_width
    } else {
        metrics.em_width
            * (snapshot.summary().longest_row_chars as f32
                + options.scroll_beyond_last_column as f32)
    };
    let show_hbar = content_width > text_width;
    let text_height = if show_hbar {
        viewport_height - hbar
    } else {
        viewport_height
    };

    // Middle-click scrolling moves every frame.
    if let Some(session) = editor.animation.middle_scroll.as_mut() {
        let delta = session.frame_delta();
        editor.scroll_top += delta.y;
        editor.scroll_left += delta.x;
        editor.animation.cancel_scroll();
        window.request_animation_frame();
    }

    // Vertical autoscroll before choosing lines.
    let selections = editor.cursors.selections();
    let primary = selections[0].head;
    let (primary_view_line, _) = view_lines.to_view(primary, false);
    if let Some(offset) = editor.pending_cursor_offset.take() {
        editor.scroll_top = zones.line_y(primary_view_line, line_height) - offset;
        editor.autoscroll_requested = false;
    }
    if let Some((range, reveal)) = editor.pending_reveal.take() {
        // Monaco `revealRange`: the box of the range's lines.
        let start = view_lines.to_view(range.start, false).0;
        let end = view_lines.to_view(range.end, false).0.max(start);
        let surrounding = if options.sticky_scroll.enabled {
            options.sticky_scroll.max_line_count as f32
        } else {
            0.
        };
        let top = crate::view::reveal::scroll_top_to_reveal(
            crate::view::reveal::RevealRequest {
                viewport_top: f32::from(editor.scroll_top),
                viewport_height: f32::from(text_height),
                box_start: f32::from(zones.line_y(start, line_height)),
                box_end: f32::from(zones.line_y(end, line_height) + zones.height(end, line_height)),
                line_height: f32::from(line_height),
                surrounding_lines: surrounding,
                mouse: false,
            },
            reveal,
        );
        editor.scroll_top = px(top);
        editor.pending_reveal_x = Some(range.start);
    }
    if editor.autoscroll_requested {
        // Monaco's primary cursor reveal: `cursorSurroundingLines` (or the
        // sticky lines) of context, unless a mouse gesture moved it.
        let mouse = editor.autoscroll_from_mouse
            && options.cursor_surrounding_lines_style
                == crate::options::CursorSurroundingLinesStyle::Default;
        let surrounding =
            (options.cursor_surrounding_lines as f32).max(if options.sticky_scroll.enabled {
                options.sticky_scroll.max_line_count as f32
            } else {
                0.
            });
        let top = zones.line_y(primary_view_line, line_height);
        editor.scroll_top = px(crate::view::reveal::scroll_top_to_reveal(
            crate::view::reveal::RevealRequest {
                viewport_top: f32::from(editor.scroll_top),
                viewport_height: f32::from(text_height),
                box_start: f32::from(top),
                box_end: f32::from(top + zones.height(primary_view_line, line_height)),
                line_height: f32::from(line_height),
                surrounding_lines: if mouse { 0. } else { surrounding },
                mouse,
            },
            crate::view::reveal::RevealType::Simple,
        ));
    }
    editor.scroll_top = editor.scroll_top.clamp(px(0.), max_scroll_top);
    // Monaco `smoothScrolling`: animate to the new position.
    let (shown, scrolling) = editor
        .animation
        .scroll_position(point(px(0.), editor.scroll_top), options.smooth_scrolling);
    if scrolling {
        window.request_animation_frame();
    }
    let scroll_top = shown.y.clamp(px(0.), max_scroll_top);
    editor.last_cursor_offset = Some(zones.line_y(primary_view_line, line_height) - scroll_top);

    let first_line = zones.line_at(scroll_top, line_height, view_line_count);
    let last_line = zones.line_at(scroll_top + viewport_height, line_height, view_line_count);
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
    if let (Some(first), Some(last)) = (segments.first(), segments.last()) {
        editor.refresh_semantic_tokens(first.row..last.row + 1, cx);
    }
    let highlights = {
        let model = editor.model().read(cx);
        current_highlights(model, &theme, visible_start..visible_end)
    };
    let highlights = editor.merge_semantic_tokens(highlights, visible_start..visible_end, cx);
    let editor_id = cx.entity_id();
    let decorations = editor.model().update(cx, |model, _| {
        model.decorations_in_range(visible_start..visible_end, Some(editor_id))
    });
    let show_validation = editor.shows_validation_decorations();
    let decorations: Vec<_> = decorations
        .into_iter()
        .filter(|d| show_validation || !d.options.validation)
        .collect();
    // Decorations in model points.
    let decoration_points: Vec<(PointUtf16, PointUtf16, &crate::decorations::Decoration)> =
        decorations
            .iter()
            .map(|d| {
                (
                    snapshot.offset_to_point_utf16(d.range.start),
                    snapshot.offset_to_point_utf16(d.range.end),
                    d,
                )
            })
            .collect();
    // Monaco `bracketPairColorization`.
    editor.refresh_bracket_colors(cx);
    editor.refresh_links(cx);
    editor.refresh_ghost_text(cx);
    editor.refresh_code_lenses(cx);
    editor.refresh_sticky_model(cx);
    editor.refresh_colors(cx);
    editor.refresh_section_headers(cx);
    editor.check_unusual_line_terminators(window, cx);
    editor.refresh_unicode_highlights(cx);
    editor.resolve_visible_code_lenses(cx);
    let bracket_colors = editor.bracket_colors_in(visible_start..visible_end, cx);
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
        // Decoration text styles override syntax colors; injected text goes
        // before/after decorated ranges.
        let mut decoration_styles: Vec<(Range<usize>, TokenStyle)> = bracket_colors
            .iter()
            .filter(|(range, _)| line_range.start <= range.start && range.end <= line_range.end)
            .map(|(range, color)| {
                (
                    range.start - line_range.start..range.end - line_range.start,
                    TokenStyle {
                        foreground: Some(*color),
                        ..Default::default()
                    },
                )
            })
            .collect();
        let mut injections = Vec::new();
        for (start, end, decoration) in &decoration_points {
            if start.row > segment.row || end.row < segment.row {
                continue;
            }
            let options = &decoration.options;
            // Monaco's tag classes, when the editor shows them.
            let tag_style = TokenStyle {
                opacity: (options.unnecessary && editor.options.show_unused)
                    .then_some(colors.unnecessary_code_opacity.a),
                strikethrough: options.deprecated && editor.options.show_deprecated,
                ..Default::default()
            };
            let style = match (options.text, tag_style == TokenStyle::default()) {
                (Some(style), true) => Some(style),
                (Some(style), false) => Some(TokenStyle {
                    opacity: tag_style.opacity,
                    strikethrough: style.strikethrough || tag_style.strikethrough,
                    ..style
                }),
                (None, false) => Some(tag_style),
                (None, true) => None,
            };
            if let Some(style) = style {
                let from = if start.row == segment.row {
                    decoration.range.start - line_range.start
                } else {
                    0
                };
                let to = if end.row == segment.row {
                    decoration.range.end - line_range.start
                } else {
                    line_range.len()
                };
                if to > from {
                    decoration_styles.push((from..to, style));
                }
            }
            if let Some(before) = &options.before
                && start.row == segment.row
            {
                injections.push((start.column, false, before.clone()));
            }
            if let Some(after) = &options.after
                && end.row == segment.row
            {
                injections.push((end.column, true, after.clone()));
            }
        }
        injections.sort_by_key(|(column, _, _)| *column);
        let row_highlights = if decoration_styles.is_empty() {
            row_highlights
        } else {
            overlay_styles(&row_highlights, &decoration_styles)
        };
        let line = layout_segment(
            &snapshot,
            segment,
            tab_size,
            &metrics,
            colors.foreground,
            &row_highlights,
            &injections,
            marked_cols,
            window,
        );
        // Monaco `scrollBeyondLastColumn`.
        content_width = content_width
            .max(line.width() + metrics.em_width * options.scroll_beyond_last_column as f32);
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
        // Monaco `_computeScrollLeftToReveal`: 30px of room on the left,
        // `revealHorizontalRightPadding` on the right.
        let (left_pad, right_pad) = (px(30.), options.reveal_horizontal_right_padding);
        if primary_x - left_pad < editor.scroll_left {
            editor.scroll_left = (primary_x - left_pad).max(px(0.));
        } else if primary_x + right_pad > editor.scroll_left + text_width {
            editor.scroll_left = primary_x + right_pad - text_width;
        }
        editor.autoscroll_requested = false;
    }
    if let Some(position) = editor.pending_reveal_x.take()
        && let Some(x) = lines
            .iter()
            .find(|line| line.contains(position))
            .map(|line| line.x_for_column(position.column))
    {
        let (left_pad, right_pad) = (px(30.), options.reveal_horizontal_right_padding);
        if x - left_pad < editor.scroll_left {
            editor.scroll_left = (x - left_pad).max(px(0.));
        } else if x + right_pad > editor.scroll_left + text_width {
            editor.scroll_left = x + right_pad - text_width;
        }
    }
    editor.scroll_left = editor.scroll_left.clamp(px(0.), max_scroll_left);
    let scroll_left = editor.scroll_left;

    let text_bounds = Bounds::new(
        point(bounds.left() + gutter_width, bounds.top()),
        size(text_width, text_height),
    );

    let scrollbar_options = options.scrollbar;
    let vertical_scrollbar = (max_scroll_top > px(0.)).then(|| {
        let bar = Bounds::new(
            point(bounds.right() - vbar, bounds.top()),
            size(vbar, viewport_height),
        );
        // Monaco's arrows take `arrowSize` at each end.
        let arrow = if scrollbar_options.vertical_has_arrows {
            scrollbar_options.arrow_size.min(viewport_height / 4.)
        } else {
            px(0.)
        };
        let track = Bounds::new(
            point(bar.left(), bar.top() + arrow),
            size(vbar, viewport_height - arrow * 2.),
        );
        let thumb_height = (track.size.height * (viewport_height / content_height))
            .max(px(MIN_SLIDER))
            .min(track.size.height);
        let travel = track.size.height - thumb_height;
        let thumb_top = track.top() + travel * (scroll_top / max_scroll_top);
        // Monaco centers a `verticalSliderSize` slider in the bar.
        let slider = scrollbar_options
            .vertical_slider_size
            .unwrap_or(vbar)
            .min(vbar);
        ScrollbarLayout {
            bar,
            track,
            thumb: Bounds::new(
                point(track.left() + (vbar - slider) / 2., thumb_top),
                size(slider, thumb_height),
            ),
            ratio: max_scroll_top / travel.max(px(1.)),
            arrows: scrollbar_options.vertical_has_arrows.then(|| {
                [
                    Bounds::new(bar.origin, size(vbar, arrow)),
                    Bounds::new(point(bar.left(), bar.bottom() - arrow), size(vbar, arrow)),
                ]
            }),
            interactive: scrollbar_options.vertical != crate::options::ScrollbarVisibility::Hidden,
        }
    });
    let horizontal_scrollbar = show_hbar.then(|| {
        let bar = Bounds::new(
            point(text_bounds.left(), bounds.bottom() - hbar),
            size(text_width, hbar),
        );
        let arrow = if scrollbar_options.horizontal_has_arrows {
            scrollbar_options.arrow_size.min(text_width / 4.)
        } else {
            px(0.)
        };
        let track = Bounds::new(
            point(bar.left() + arrow, bar.top()),
            size(text_width - arrow * 2., hbar),
        );
        let thumb_width = (track.size.width * (text_width / content_width))
            .max(px(MIN_SLIDER))
            .min(track.size.width);
        let travel = track.size.width - thumb_width;
        let thumb_left = track.left()
            + if max_scroll_left > px(0.) {
                travel * (scroll_left / max_scroll_left)
            } else {
                px(0.)
            };
        let slider = scrollbar_options
            .horizontal_slider_size
            .unwrap_or(hbar)
            .min(hbar);
        ScrollbarLayout {
            bar,
            track,
            thumb: Bounds::new(
                point(thumb_left, track.top() + (hbar - slider) / 2.),
                size(thumb_width, slider),
            ),
            ratio: max_scroll_left / travel.max(px(1.)),
            arrows: scrollbar_options.horizontal_has_arrows.then(|| {
                [
                    Bounds::new(bar.origin, size(arrow, hbar)),
                    Bounds::new(point(bar.right() - arrow, bar.top()), size(arrow, hbar)),
                ]
            }),
            interactive: scrollbar_options.horizontal
                != crate::options::ScrollbarVisibility::Hidden,
        }
    });
    // Monaco reveals `auto` scrollbars whenever the view scrolls.
    if editor
        .layout
        .as_ref()
        .is_some_and(|l| l.scroll_top != scroll_top || l.scroll_left != scroll_left)
    {
        editor.animation.scrollbars.revealed_at = Some(std::time::Instant::now());
    }

    // Fold controls and placeholders.
    let mut fold_controls = Vec::new();
    let mut fold_placeholders = Vec::new();
    let mut fold_placeholder_bounds = Vec::new();
    let mut fold_highlights = Vec::new();
    if options.folding {
        let model = editor.model().clone();
        let buffer = model.read(cx).buffer();
        let collapsed = editor.folds.collapsed_rows(buffer);
        let left = bounds.left() + glyph_width + numbers_width + options.line_decorations_width;
        for line in lines.iter().filter(|line| line.columns.start == 0) {
            let top = bounds.top() + zones.line_y(line.view_line, line_height) - scroll_top;
            let height = zones.height(line.view_line, line_height);
            let is_collapsed = collapsed.iter().any(|(start, _)| *start == line.row);
            if is_collapsed || editor.folds.range_starting_at(line.row).is_some() {
                fold_controls.push(FoldControl {
                    row: line.row,
                    bounds: Bounds::new(point(left, top), size(folding_width, height)),
                    collapsed: is_collapsed,
                });
            }
        }
        for line in lines.iter().filter(|line| line.is_last_segment) {
            if collapsed.iter().any(|(start, _)| *start == line.row) {
                // Centered on taller lines, with the text.
                let top = bounds.top() + zones.line_y(line.view_line, line_height) - scroll_top
                    + (zones.height(line.view_line, line_height) - line_height) / 2.;
                let x = text_bounds.left() - scroll_left + line.width() + metrics.em_width / 2.;
                fold_placeholders.push((x, top));
                fold_placeholder_bounds.push((
                    line.row,
                    Bounds::new(
                        point(x, top + line_height * 0.15),
                        size(metrics.em_width * 2., line_height * 0.7),
                    ),
                ));
            }
        }
        // Monaco `foldingHighlight`: the collapsed regions' first lines.
        if options.folding_highlight {
            for line in lines.iter() {
                if collapsed.iter().any(|(start, _)| *start == line.row) {
                    let top = bounds.top() + zones.line_y(line.view_line, line_height) - scroll_top;
                    fold_highlights.push(Bounds::new(
                        point(text_bounds.left(), top),
                        size(
                            text_bounds.size.width,
                            zones.height(line.view_line, line_height),
                        ),
                    ));
                }
            }
        }
    }
    let show_fold_controls = match options.show_folding_controls {
        ShowFoldingControls::Always => true,
        ShowFoldingControls::Mouseover => editor.gutter_hovered,
        ShowFoldingControls::Never => false,
    };

    let mut layout = EditorLayout {
        bounds,
        gutter_width,
        glyph_margin_width: glyph_width,
        line_numbers_width: numbers_width,
        text_bounds,
        metrics: metrics.clone(),
        scroll_top,
        scroll_left,
        max_scroll: point(max_scroll_left, max_scroll_top),
        lines,
        fold_controls,
        fold_placeholders: fold_placeholder_bounds,
        zones: zones.clone(),
        widget_bounds: Vec::new(),
        widget_kinds: Vec::new(),
        lightbulb: None,
        minimap: None,
        sticky_lines: Vec::new(),
        vertical_scrollbar,
        horizontal_scrollbar,
    };

    // Minimap: token-colored characters or blocks (Monaco `minimap`), sized
    // per `minimap.size` (`EditorLayoutInfoComputer`).
    let mut minimap_image = None;
    let mut section_headers = Vec::new();
    if minimap_width > px(0.) {
        use crate::options::MinimapSize;
        let pixel_ratio = window.scale_factor();
        let minimap_options = &options.minimap;
        let base = if minimap_options.render_characters {
            2.
        } else {
            3.
        };
        let mut scale = if pixel_ratio >= 2. {
            (minimap_options.scale * 2.).round()
        } else {
            minimap_options.scale
        };
        // Device pixels per minimap line.
        let mut line_px = base * scale;
        let mut fills = false;
        let mut sample_ratio = 1.;
        if minimap_options.size != MinimapSize::Proportional {
            let height = f32::from(viewport_height);
            let lh = f32::from(line_height);
            let typical = height / lh;
            let before = (f32::from(options.padding.top) / lh).floor();
            let mut after = (f32::from(options.padding.bottom) / lh).floor();
            if options.scroll_beyond_last_line {
                after = after.max(typical - 1.);
            }
            let count = view_line_count.max(1) as f32;
            let desired = (before + count + after) / (pixel_ratio * height);
            let minimap_lines = (count / desired).floor().max(1.);
            let ratio = count / minimap_lines;
            if ratio > 1. {
                // More lines than pixels: one pixel per sampled line.
                fills = true;
                sample_ratio = ratio;
                scale = 1.;
                line_px = 1.;
            } else {
                let fit_becomes_fill = minimap_options.size == MinimapSize::Fit
                    && ((before + count + after) * line_px).ceil() > pixel_ratio * height;
                if minimap_options.size == MinimapSize::Fill || fit_becomes_fill {
                    fills = true;
                    let max_scale = scale + 1.;
                    line_px = (lh * pixel_ratio).min((1. / desired).floor().max(1.));
                    scale = max_scale.min((line_px / base).floor().max(1.));
                }
            }
        }
        let mline = px(line_px / pixel_ratio);
        let char_width = if pixel_ratio >= 2. { scale / 2. } else { scale };
        let render = MinimapRender {
            char_width: scale,
            line_height: line_px,
            characters: minimap_options.render_characters,
            max_column: (f32::from(minimap_width) / char_width.max(0.1)) as usize,
            width: (f32::from(minimap_width) * pixel_ratio).ceil() as u32,
        };
        let minimap_bounds = Bounds::new(
            if minimap_left {
                full_bounds.origin
            } else {
                point(bounds.right() - vbar - minimap_width, bounds.top())
            },
            size(minimap_width, viewport_height),
        );
        // Minimap lines, and the view line each shows.
        let minimap_line_count = (view_line_count as f32 / sample_ratio).ceil().max(1.) as u32;
        let total = mline * minimap_line_count as f32;
        let minimap_scroll = if !fills && total > viewport_height && max_scroll_top > px(0.) {
            (total - viewport_height) * (scroll_top / max_scroll_top)
        } else {
            px(0.)
        };
        let first = (minimap_scroll / mline).floor() as u32;
        let last = (((minimap_scroll + viewport_height) / mline).ceil() as u32)
            .min(minimap_line_count.saturating_sub(1));
        let lines: Vec<u32> = (first..=last)
            .map(|i| ((i as f32 * sample_ratio) as u32).min(view_line_count.saturating_sub(1)))
            .collect();
        let theme_key = std::sync::Arc::as_ptr(&theme) as usize;
        let parsed = editor
            .model()
            .read(cx)
            .syntax_snapshot()
            .map(|syntax| syntax.text.version());
        let key = (
            snapshot.version(),
            parsed,
            first,
            last,
            theme_key,
            wrap_column.unwrap_or(0),
            [
                render.char_width.to_bits(),
                render.line_height.to_bits(),
                render.characters as u32,
                sample_ratio.to_bits(),
            ],
        );
        let cache = &editor.minimap_cache;
        if cache.key != Some(key) && cache.pending.as_ref().is_none_or(|(k, _)| *k != key) {
            let source = HighlightSource::new(editor.model().read(cx));
            let build = {
                let (view_lines, snapshot, theme) =
                    (view_lines.clone(), snapshot.clone(), theme.clone());
                move || {
                    minimap_image_for(
                        source.as_ref(),
                        &view_lines,
                        &snapshot,
                        &theme,
                        &lines,
                        tab_size,
                        render,
                    )
                }
            };
            if cache.key.is_none() {
                // Nothing to show yet: build the first one now.
                let stale = std::mem::take(&mut editor.minimap_cache.stale);
                editor.minimap_cache = MinimapCache {
                    key: Some(key),
                    first,
                    image: build(),
                    stale,
                    pending: None,
                };
            } else {
                let task = cx.spawn(async move |this, cx| {
                    let blocks = cx.background_executor().spawn(async move { build() }).await;
                    this.update(cx, |editor, cx| {
                        let cache = &mut editor.minimap_cache;
                        if cache.pending.as_ref().is_some_and(|(k, _)| *k == key) {
                            cache.key = Some(key);
                            cache.first = first;
                            if let Some((old, _)) = std::mem::replace(&mut cache.image, blocks) {
                                cache.stale.push(old);
                            }
                            cache.pending = None;
                            cx.notify();
                        }
                    })
                    .ok();
                });
                editor.minimap_cache.pending = Some((key, std::sync::Arc::new(task)));
            }
        }
        let origin_y =
            minimap_bounds.top() + mline * editor.minimap_cache.first as f32 - minimap_scroll;
        minimap_image = editor.minimap_cache.image.clone().map(|(image, (w, h))| {
            (
                image,
                Bounds::new(
                    point(minimap_bounds.left(), origin_y),
                    size(px(w as f32 / pixel_ratio), px(h as f32 / pixel_ratio)),
                ),
            )
        });
        // The slider covers the visible lines, in minimap lines.
        let first_visible = zones.line_at(scroll_top, line_height, view_line_count);
        let visible_lines = (viewport_height / line_height).max(1.);
        let per_view_line = mline / sample_ratio;
        let slider = Bounds::new(
            point(
                minimap_bounds.left(),
                minimap_bounds.top() + per_view_line * first_visible as f32 - minimap_scroll,
            ),
            size(minimap_width, (per_view_line * visible_lines).max(px(2.))),
        );
        // Monaco `_renderSectionHeaders`: a label over a translucent band
        // and/or a separator line, at each header's minimap line.
        let font_size = px(minimap_options.section_header_font_size);
        let band = font_size * 1.5;
        let label_font = Font {
            weight: FontWeight::MEDIUM,
            ..font(".SystemUIFont")
        };
        for header in &editor.section_headers.headers {
            let (view_line, _) = view_lines.to_view(header.range.start, false);
            let top = minimap_bounds.top() + per_view_line * view_line as f32 - minimap_scroll;
            if top + band < minimap_bounds.top() || top > minimap_bounds.bottom() {
                continue;
            }
            let label = (!header.text.is_empty()).then(|| {
                let shape = |text: String| {
                    window.text_system().shape_line(
                        text.clone().into(),
                        font_size,
                        &[run(text.len(), &label_font, colors.foreground)],
                        None,
                    )
                };
                // Monaco's `MINIMAP_GUTTER_WIDTH` left of the label.
                let max_width = minimap_width - px(8.);
                let mut shaped = shape(header.text.clone());
                if shaped.width > max_width {
                    let chars = header.text.chars().count().max(1);
                    let average = shaped.width / chars as f32;
                    let count = ((max_width - average) / average).floor() as usize;
                    shaped = shape(crate::section_headers::fit_section_header(
                        &header.text,
                        count.saturating_sub(1),
                    ));
                }
                (
                    shaped,
                    point(
                        minimap_bounds.left() + px(8.),
                        top + (band - font_size) / 2.,
                    ),
                )
            });
            section_headers.push(MinimapSectionHeader {
                band: Bounds::new(point(minimap_bounds.left(), top), size(minimap_width, band)),
                label,
                separator: header.has_separator_line.then(|| {
                    Bounds::new(
                        point(minimap_bounds.left(), top + px(2.)),
                        size(minimap_width, px(1.)),
                    )
                }),
            });
        }
        let travel = (total.min(viewport_height) - slider.size.height).max(px(1.));
        layout.minimap = Some(MinimapLayout {
            bounds: minimap_bounds,
            slider,
            drag_ratio: max_scroll_top / travel,
            line_height: per_view_line,
            scroll: minimap_scroll,
        });
    }

    // Sticky scroll: headers of the fold ranges enclosing the top of the
    // viewport (Monaco `stickyScroll`; see `sticky_model`).
    let mut sticky_shaped: Vec<(usize, Pixels, LineLayout, Option<ShapedLine>)> = Vec::new();
    if options.sticky_scroll.enabled && !options.single_line && scroll_top > px(0.) {
        let max = options.sticky_scroll.max_line_count as usize;
        let mut sticky_rows: Vec<(u32, u32)> = Vec::new();
        while sticky_rows.len() < max {
            let probe_y = scroll_top + line_height * sticky_rows.len() as f32;
            let probe_row = view_lines
                .view_line(zones.line_at(probe_y, line_height, view_line_count))
                .row;
            // The outermost range enclosing the probe below the last header.
            let next =
                editor
                    .sticky_ranges_enclosing(probe_row)
                    .into_iter()
                    .find(|(start_row, _)| {
                        sticky_rows
                            .last()
                            .is_none_or(|(start, _)| *start < *start_row)
                    });
            match next {
                Some(range) => sticky_rows.push(range),
                None => break,
            }
        }
        let collapsed_rows = editor
            .folds
            .collapsed_rows(editor.model().read(cx).buffer());
        for (ix, (start_row, end_row)) in sticky_rows.iter().enumerate() {
            // Monaco: Shift over a sticky line shows the scope's end.
            let row = if editor.sticky.show_end == Some(ix) {
                &(end_row + 1).min(snapshot.max_point_utf16().row)
            } else {
                start_row
            };
            let mut top = line_height * ix as f32;
            // The last header slides up as its range scrolls out.
            let end_line = view_lines.first_view_line_of_row(end_row + 1).max(1) - 1;
            let end_bottom = zones.line_y(end_line, line_height) + line_height - scroll_top;
            if ix + 1 == sticky_rows.len() {
                top = top.min(end_bottom - line_height);
            }
            let segment = row_segment(&view_lines, view_lines.first_view_line_of_row(*row));
            let line_range = snapshot.line_range(*row);
            let row_highlights: Vec<(Range<usize>, TokenStyle)> = {
                let model = editor.model().read(cx);
                current_highlights(model, &theme, line_range.clone())
                    .into_iter()
                    .map(|(range, style)| {
                        (
                            range.start.max(line_range.start) - line_range.start
                                ..range.end.min(line_range.end) - line_range.start,
                            style,
                        )
                    })
                    .collect()
            };
            let shaped = layout_segment(
                &snapshot,
                &segment,
                tab_size,
                &metrics,
                colors.foreground,
                &row_highlights,
                &[],
                None,
                window,
            );
            let number = (options.line_numbers != LineNumbers::Off).then(|| {
                let label = (row + 1).to_string();
                window.text_system().shape_line(
                    label.clone().into(),
                    metrics.font_size,
                    &[run(label.len(), &metrics.font, colors.line_number)],
                    None,
                )
            });
            let fold = (options.folding && editor.folds.range_starting_at(*start_row).is_some())
                .then(|| FoldControl {
                    row: *start_row,
                    bounds: Bounds::new(
                        point(
                            bounds.left()
                                + glyph_width
                                + numbers_width
                                + options.line_decorations_width,
                            bounds.top() + top,
                        ),
                        size(folding_width, line_height),
                    ),
                    collapsed: collapsed_rows.iter().any(|(start, _)| start == start_row),
                });
            layout.sticky_lines.push(StickyLine {
                end_row: *end_row,
                row: *row,
                bounds: Bounds::new(
                    point(bounds.left(), bounds.top() + top),
                    size(gutter_width + text_width, line_height),
                ),
                fold,
            });
            sticky_shaped.push((ix, top, shaped, number));
        }
    }
    let numbers_right = bounds.left() + glyph_width + numbers_width;

    // Decoration geometry.
    let mut decoration_quads: Vec<(Bounds<Pixels>, Option<Hsla>, Option<Hsla>)> = Vec::new();
    let mut decoration_underlines: Vec<(gpui::Point<Pixels>, Pixels, UnderlineStyle)> = Vec::new();
    let mut gutter_bars: Vec<(Bounds<Pixels>, Hsla)> = Vec::new();
    let mut glyphs: Vec<(gpui::Point<Pixels>, ShapedLine)> = Vec::new();
    let mut line_number_colors: Vec<(u32, Hsla)> = Vec::new();
    let origin_x = layout.text_origin_x();
    for (start, end, decoration) in &decoration_points {
        let options = &decoration.options;
        for line in layout.lines.iter() {
            if line.row < start.row || line.row > end.row {
                continue;
            }
            let top = layout.line_top(line.view_line);
            let height = layout.line_height_of(line.view_line);
            let text_top = layout.text_top(line.view_line);
            if options.background.is_some() || options.border.is_some() {
                let range = if options.is_whole_line {
                    Some((px(0.) - layout.scroll_left, text_width + layout.scroll_left))
                } else {
                    decoration_x_range(line, *start, *end)
                };
                if let Some((from, to)) = range {
                    // Monaco gives empty ranges (e.g. a snippet's final tab
                    // stop) a 2px minimum width.
                    let bounds = Bounds::new(
                        point(origin_x + from, top),
                        size((to - from).max(px(2.)), height),
                    );
                    decoration_quads.push((bounds, options.background, options.border));
                }
            }
            // Monaco `squiggly-unnecessary`: a themed underline for unused
            // code without a squiggle.
            let underline = options.underline.or_else(|| {
                (options.unnecessary
                    && editor.options.show_unused
                    && colors.unnecessary_code_border.a > 0.)
                    .then_some(crate::decorations::DecorationUnderline {
                        color: colors.unnecessary_code_border,
                        wavy: false,
                    })
            });
            if let Some(underline) = underline
                && let Some((from, to)) = decoration_x_range(line, *start, *end)
            {
                decoration_underlines.push((
                    point(origin_x + from, text_top + line_height - px(3.)),
                    (to - from).max(metrics.em_width / 2.),
                    UnderlineStyle {
                        thickness: px(1.),
                        color: Some(underline.color),
                        wavy: underline.wavy,
                    },
                ));
            }
            if let Some(color) = options.gutter_bar {
                let left = bounds.left() + glyph_width + numbers_width + px(2.);
                gutter_bars.push((Bounds::new(point(left, top), size(px(3.), height)), color));
            }
            if line.columns.start == 0 {
                if let Some(color) = options.line_number_color {
                    line_number_colors.push((line.row, color));
                }
                if let Some((glyph, color)) = &options.glyph
                    && line.row == start.row
                    && options.glyph.is_some()
                {
                    let shaped = window.text_system().shape_line(
                        glyph.clone(),
                        metrics.font_size,
                        &[run(glyph.len(), &metrics.font, *color)],
                        None,
                    );
                    let x = bounds.left() + (glyph_width - shaped.width) / 2.;
                    glyphs.push((point(x, text_top), shaped));
                }
            }
        }
    }
    // Inlay hints follow the viewport.
    if let (Some(first), Some(last)) = (layout.lines.first(), layout.lines.last()) {
        editor.inlay_hints_viewport(first.row..last.row + 1, cx);
    }

    // The code action lightbulb (Monaco `LightBulbWidget`).
    let mut lightbulb = None;
    if let Some(bulb) = editor.code_actions.lightbulb.clone() {
        let model = editor.model().read(cx);
        let line_count = snapshot.max_point().row + 1;
        let lines = |row: u32| model.line_content(row);
        let glyph_rows: Vec<u32> = decoration_points
            .iter()
            .filter(|(_, _, d)| d.options.glyph.is_some())
            .map(|(start, _, _)| start.row)
            .collect();
        let placement = crate::code_action::place_lightbulb(
            bulb.position,
            line_count,
            &lines,
            &|row| view_lines.is_row_hidden(row),
            tab_size,
            f32::from(metrics.em_width),
            glyph_rows.contains(&bulb.position.row),
        );
        let side = line_height.min(px(16.)).max(line_height * 0.8);
        let at = |row: u32| {
            layout
                .lines
                .iter()
                .find(|line| line.row == row && line.columns.start == 0)
        };
        let bounds = match placement {
            Some(crate::code_action::LightbulbPlacement::Gutter(row)) if glyph_width > px(0.) => {
                at(row).map(|line| {
                    let top = layout.text_top(line.view_line) + (line_height - side) / 2.;
                    Bounds::new(
                        point(bounds.left() + (glyph_width - side) / 2., top),
                        size(side, side),
                    )
                })
            }
            Some(crate::code_action::LightbulbPlacement::Gutter(row))
            | Some(crate::code_action::LightbulbPlacement::Content(row, _)) => {
                let column = match placement {
                    Some(crate::code_action::LightbulbPlacement::Content(_, column)) => column,
                    _ => 0,
                };
                at(row).map(|line| {
                    let top = layout.text_top(line.view_line) + (line_height - side) / 2.;
                    Bounds::new(
                        point(origin_x + line.x_for_column(column), top),
                        size(side, side),
                    )
                })
            }
            None => None,
        };
        lightbulb = bounds.map(|bounds| {
            (
                bounds,
                crate::code_action::lightbulb_color(&colors, bulb.auto_fix),
            )
        });
    }
    layout.lightbulb = lightbulb.map(|(bounds, _)| bounds);

    // Overview ruler marks in the vertical scrollbar track.
    let mut ruler_marks: Vec<(Bounds<Pixels>, Hsla)> = Vec::new();
    let ruler = Bounds::new(
        point(bounds.right() - vbar, bounds.top()),
        size(vbar, viewport_height),
    );
    if options.overview_ruler_lanes > 0 && vbar > px(0.) {
        let track = ruler;
        let all = editor
            .model()
            .update(cx, |model, _| model.all_decorations(Some(editor_id)))
            .into_iter()
            .filter(|d| editor.shows_validation_decorations() || !d.options.validation)
            .collect::<Vec<_>>();
        let total = view_line_count.max(1) as f32;
        for decoration in all {
            let Some((color, lane)) = decoration.options.overview_ruler else {
                continue;
            };
            let row = snapshot.offset_to_point(decoration.range.start).row;
            let (view_line, _) = view_lines.to_view(PointUtf16::new(row, 0), false);
            let y = track.top() + track.size.height * (view_line as f32 / total);
            let (x, w) = overview_lane(track, options.overview_ruler_lanes, lane);
            ruler_marks.push((Bounds::new(point(x, y), size(w, px(3.))), color));
        }
        // Monaco draws the cursors across the ruler, 2px tall.
        if !options.hide_cursor_in_overview_ruler {
            for selection in &selections {
                let (view_line, _) = view_lines.to_view(selection.head, false);
                let y = track.top() + track.size.height * (view_line as f32 / total);
                ruler_marks.push((
                    Bounds::new(point(track.left(), y), size(track.size.width, px(2.))),
                    colors.cursor,
                ));
            }
        }
    }

    let show_ruler_border = options.overview_ruler_border
        && options.overview_ruler_lanes > 0
        && options.vertical_scrollbar_size > px(0.);
    // Monaco's scrollbar visibility: `auto` shows while hovered, dragged or
    // just scrolled, then fades.
    let dragging = editor.scrollbar_drag;
    let mouse = editor.animation.scrollbars.mouse_position;
    let mut scrollbar_looks = [(0., false, false); 2];
    for (ix, (bar, visibility)) in [
        (layout.vertical_scrollbar, options.scrollbar.vertical),
        (layout.horizontal_scrollbar, options.scrollbar.horizontal),
    ]
    .into_iter()
    .enumerate()
    {
        let Some(bar) = bar else {
            continue;
        };
        let dragged = match dragging {
            Some(crate::editor::ScrollbarDrag::Vertical { .. }) => ix == 0,
            Some(crate::editor::ScrollbarDrag::Horizontal { .. }) => ix == 1,
            None => false,
        };
        let opacity = match visibility {
            crate::options::ScrollbarVisibility::Visible => 1.,
            crate::options::ScrollbarVisibility::Hidden => 0.,
            crate::options::ScrollbarVisibility::Auto => {
                let (opacity, animating) = editor.animation.scrollbars.opacity(dragging.is_some());
                if animating {
                    window.request_animation_frame();
                }
                opacity
            }
        };
        let hovered = mouse.is_some_and(|m| bar.thumb.contains(&m));
        scrollbar_looks[ix] = (opacity, hovered, dragged);
    }

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
            // Monaco's cursor fills a taller line.
            let line_height = layout.line_height_of(line.view_line);
            let char_width = if head.column < line.columns.end {
                (line.x_for_column(head.column + 1) - line.x_for_column(head.column))
                    .max(metrics.em_width / 2.)
            } else {
                metrics.em_width
            };
            let style = if editor.overtype {
                options.overtype_cursor_style
            } else {
                options.cursor_style
            };
            // Monaco `cursorHeight` for the line styles, centered.
            let line_cursor_height = if options.cursor_height > px(0.) {
                options.cursor_height.min(line_height)
            } else {
                line_height
            };
            let line_y = y + (line_height - line_cursor_height) / 2.;
            let bounds = match style {
                CursorStyle::Line => Bounds::new(
                    point(x, line_y),
                    size(options.cursor_width, line_cursor_height),
                ),
                CursorStyle::LineThin => {
                    Bounds::new(point(x, line_y), size(px(1.), line_cursor_height))
                }
                CursorStyle::Block => Bounds::new(point(x, y), size(char_width, line_height)),
                CursorStyle::Underline => {
                    Bounds::new(point(x, y + line_height - px(2.)), size(char_width, px(2.)))
                }
            };
            cursor_quads.push(bounds);
        }
    }
    // Monaco `cursorSmoothCaretAnimation`: the primary cursor slides, in
    // content coordinates so scrolling doesn't animate it.
    if let Some(primary) = cursor_quads.first_mut() {
        let offset = point(
            layout.text_bounds.left() - scroll_left,
            bounds.top() - scroll_top,
        );
        let content = Bounds::new(primary.origin - offset, primary.size);
        let enabled = options.cursor_smooth_caret_animation
            != crate::options::CursorSmoothCaretAnimation::Off;
        let (shown, animating) = editor.animation.caret(content, enabled);
        *primary = Bounds::new(shown.origin + offset, shown.size);
        if animating {
            window.request_animation_frame();
        }
    }
    // Monaco's smooth, phase and expand blinking.
    let (cursor_opacity, cursor_scale) = crate::view::animation::cursor_appearance(
        editor.cursor_blinking(),
        editor.animation.blink_started.elapsed(),
    );

    // Indent and bracket pair guides (Monaco `IndentGuidesOverlay`) and
    // rulers.
    let mut guides = Vec::new();
    let bracket_guides_on = options.guides.bracket_pairs != crate::options::BracketPairGuides::Off;
    if let (Some(first), Some(last)) = (layout.lines.first(), layout.lines.last())
        && (options.guides.indentation || bracket_guides_on)
    {
        use crate::view::guides::{BracketGuide, BracketGuideOptions};
        let rope = snapshot.rope();
        let model = editor.model().read(cx);
        let ctx = crate::view::guides::GuideContext {
            tab_size,
            indent_size: model.options().indent_size,
            off_side: model
                .language()
                .and_then(|l| l.config.folding.as_ref())
                .is_some_and(|f| f.off_side),
        };
        let rows = first.row..last.row + 1;
        let counts = if options.guides.indentation {
            crate::view::guides::lines_indent_guides(rope, rows.clone(), ctx)
        } else {
            vec![0; rows.len()]
        };
        let active = (options.guides.indentation && options.guides.highlight_active_indentation)
            .then(|| {
                crate::view::guides::active_indent_guide(
                    rope,
                    primary.row,
                    rows.start,
                    last.row,
                    ctx,
                )
            });
        let bracket_guides = bracket_guides_on.then(|| {
            let pairs = editor.bracket_pairs_in_rows(rows.clone(), cx);
            crate::view::guides::lines_bracket_guides(
                rope,
                rows.clone(),
                &pairs,
                Some(primary),
                BracketGuideOptions {
                    include_inactive: options.guides.bracket_pairs
                        == crate::options::BracketPairGuides::On,
                    horizontal: options.guides.bracket_pairs_horizontal,
                    highlight_active: options.guides.highlight_active_bracket_pair,
                },
                tab_size,
            )
        });
        let origin_x = layout.text_origin_x();
        for line in &layout.lines {
            let ix = (line.row - rows.start) as usize;
            let count = counts[ix];
            let top = layout.line_top(line.view_line);
            let line_height = layout.line_height_of(line.view_line);
            // Monaco: on wrapped lines, the opening line's guide shows after
            // the bracket and the closing line's up to it.
            let in_line: Vec<&BracketGuide> = bracket_guides
                .as_ref()
                .map(|all| {
                    all[ix]
                        .iter()
                        .filter(|g| {
                            g.for_wrapped_lines_after_column
                                .is_none_or(|c| line.columns.start > c - 1)
                                && g.for_wrapped_lines_before_or_at_column
                                    .is_none_or(|c| line.columns.start < c)
                        })
                        .collect()
                })
                .unwrap_or_default();
            let draw_bracket_guide =
                |guide: &BracketGuide, guides: &mut Vec<(Bounds<Pixels>, Hsla)>| {
                    let Some(color) = colors.bracket_color(guide.level) else {
                        return;
                    };
                    let color = if guide.active {
                        color
                    } else {
                        color.opacity(0.3)
                    };
                    let left = match guide.column {
                        Some(column) => origin_x + line.x_for_column(column - 1),
                        None => origin_x + metrics.em_width * (guide.visible_column - 1) as f32,
                    };
                    let bounds = match guide.horizontal {
                        Some(horizontal) => {
                            let right = origin_x + line.x_for_column(horizontal.end_column - 1);
                            let y = if horizontal.top {
                                top
                            } else {
                                top + line_height - px(1.)
                            };
                            Bounds::new(point(left, y), size((right - left).max(px(0.)), px(1.)))
                        }
                        None => Bounds::new(point(left, top), size(px(1.), line_height)),
                    };
                    guides.push((bounds, color));
                };
            // Monaco `getGuidesByLine`: indent guides, minus those a
            // vertical bracket guide replaces, in column order.
            let mut queue = in_line.iter().copied().peekable();
            for level in 1..=count {
                let indent_column = ((level - 1) * ctx.indent_size + 1) as i64;
                while let Some(guide) = queue.next_if(|g| g.visible_column < indent_column) {
                    draw_bracket_guide(guide, &mut guides);
                }
                if queue
                    .peek()
                    .is_some_and(|g| g.visible_column == indent_column && g.horizontal.is_none())
                {
                    continue;
                }
                let is_active = in_line.is_empty()
                    && active.is_some_and(|a| {
                        a.indent == level && a.start_row <= line.row && line.row <= a.end_row
                    });
                let color = if is_active {
                    colors.indent_guide_active
                } else {
                    colors.indent_guide
                };
                let x = origin_x + metrics.em_width * ((level - 1) * ctx.indent_size) as f32;
                guides.push((Bounds::new(point(x, top), size(px(1.), line_height)), color));
            }
            for guide in queue {
                draw_bracket_guide(guide, &mut guides);
            }
        }
    }
    for ruler in &options.rulers {
        let x = layout.text_origin_x() + metrics.em_width * ruler.column as f32;
        guides.push((
            Bounds::new(
                point(x, text_bounds.top()),
                size(px(1.), text_bounds.size.height),
            ),
            ruler.color.unwrap_or(colors.ruler),
        ));
    }

    // Rendered whitespace (Monaco `renderWhitespace`).
    let whitespace =
        (options.render_whitespace != crate::options::RenderWhitespace::None).then(|| {
            let shape = |text: &'static str| {
                window.text_system().shape_line(
                    text.into(),
                    metrics.font_size,
                    &[run(text.len(), &metrics.font, colors.whitespace)],
                    None,
                )
            };
            let (dot, arrow) = (shape("\u{b7}"), shape("\u{2192}"));
            let rope = snapshot.rope();
            let mut marks = Vec::new();
            for line in &layout.lines {
                let text = rope.line(line.row);
                let selected: Vec<Range<u32>> = selections
                    .iter()
                    .filter(|s| s.start().row <= line.row && line.row <= s.end().row)
                    .map(|s| {
                        let start = if s.start().row == line.row {
                            s.start().column
                        } else {
                            0
                        };
                        let end = if s.end().row == line.row {
                            s.end().column
                        } else {
                            u32::MAX
                        };
                        start..end
                    })
                    .collect();
                let top = layout.text_top(line.view_line);
                for column in crate::view::guides::whitespace_columns(
                    &text,
                    options.render_whitespace,
                    &selected,
                ) {
                    if column < line.columns.start || column >= line.columns.end {
                        continue;
                    }
                    let is_tab = text.encode_utf16().nth(column as usize) == Some(b'\t' as u16);
                    let x = layout.text_origin_x() + line.x_for_column(column);
                    marks.push((point(x, top), is_tab));
                }
            }
            (dot, arrow, marks)
        });

    // Gutter numbers: on the first view line of each row.
    let mut gutter_numbers = Vec::new();
    if options.line_numbers != LineNumbers::Off {
        let right = bounds.left() + glyph_width + numbers_width;
        for line in layout.lines.iter().filter(|line| line.columns.start == 0) {
            let row = line.row;
            let is_active = selections.iter().any(|s| s.head.row == row);
            // Monaco `renderFinalNewline`: the empty line after a final
            // newline.
            let is_final_newline =
                row > 0 && row + 1 == snapshot.line_count() && snapshot.line_len(row) == 0;
            if is_final_newline
                && options.render_final_newline == crate::options::RenderFinalNewline::Off
            {
                continue;
            }
            let label = match options.line_numbers {
                LineNumbers::Relative if !is_active => primary.row.abs_diff(row).to_string(),
                LineNumbers::Interval if !is_active && (row + 1) % 10 != 0 && row != 0 => {
                    continue;
                }
                _ => (row + 1).to_string(),
            };
            let color = line_number_colors
                .iter()
                .find(|(r, _)| *r == row)
                .map(|(_, color)| *color)
                .unwrap_or(if is_active {
                    colors.active_line_number
                } else {
                    colors.line_number
                });
            let color = if is_final_newline
                && options.render_final_newline == crate::options::RenderFinalNewline::Dimmed
            {
                color.opacity(0.4)
            } else {
                color
            };
            let shaped = window.text_system().shape_line(
                label.clone().into(),
                metrics.font_size,
                &[run(label.len(), &metrics.font, color)],
                None,
            );
            gutter_numbers.push((layout.text_top(line.view_line), shaped, right));
        }
    }

    // Embedded views; their elements are built after this update returns so
    // host views may read the editor while rendering.
    let mut embedded = Vec::new();
    let mut zone_specs = zone_specs;
    zone_specs.sort_by_key(|(after, _, _, _)| *after);
    let mut ix = 0;
    while ix < zone_specs.len() {
        // Zones after the same line stack in insertion order.
        let after = zone_specs[ix].0;
        let group_end = ix + zone_specs[ix..].iter().take_while(|z| z.0 == after).count();
        let group_height = zone_specs[ix..group_end]
            .iter()
            .map(|(_, h, _, _)| *h)
            .fold(px(0.), |a, b| a + b);
        let mut top = layout.bounds.top() + zones.line_y((after + 1) as u32, line_height)
            - group_height
            - scroll_top;
        // Only zones on screen are rendered.
        if top > layout.bounds.bottom() || top + group_height < layout.bounds.top() {
            ix = group_end;
            continue;
        }
        for (_, height, view, full_width) in &zone_specs[ix..group_end] {
            let left = if *full_width {
                layout.bounds.left()
            } else {
                text_bounds.left()
            };
            embedded.push(Embedded::Zone {
                view: view.clone(),
                bounds: Bounds::new(
                    point(left, top),
                    size(text_bounds.left() + text_width - left, *height),
                ),
                full_width: *full_width,
            });
            top += *height;
        }
        ix = group_end;
    }
    {
        let model = editor.model().clone();
        let buffer = model.read(cx).buffer();
        for widget in &editor.widgets.content {
            let position = buffer.offset_to_point_utf16(buffer.resolve(&widget.anchor));
            let Some(line) = layout.line_for_point(position) else {
                continue;
            };
            let x = layout.text_origin_x() + line.x_for_column(position.column);
            embedded.push(Embedded::Content {
                view: widget.view.clone(),
                anchor: point(x, layout.line_top(line.view_line)),
                line_height: layout.line_height_of(line.view_line),
                preference: widget.preference.clone(),
            });
        }
    }
    for widget in &editor.widgets.overlays {
        embedded.push(Embedded::Overlay {
            view: widget.view.clone(),
            position: widget.position,
        });
    }

    // Monaco `PlaceholderTextContribution`: the placeholder while the
    // model is empty, each line cut off with an ellipsis.
    let mut placeholder = Vec::new();
    if editor.is_placeholder_visible()
        && let Some(text) = &options.placeholder
    {
        let mut wrapper = window
            .text_system()
            .line_wrapper(metrics.font.clone(), metrics.font_size);
        let top = layout.line_top(0);
        for (ix, line) in text.lines().enumerate() {
            let runs = [run(
                line.len(),
                &metrics.font,
                colors.placeholder_foreground,
            )];
            let (line, runs) = wrapper.truncate_line(
                SharedString::from(line.to_string()),
                text_bounds.size.width,
                "…",
                &runs,
                gpui::TruncateFrom::End,
            );
            let shaped = window
                .text_system()
                .shape_line(line, metrics.font_size, &runs, None);
            placeholder.push((
                shaped,
                point(text_bounds.left(), top + line_height * ix as f32),
            ));
        }
    }
    // Monaco's `dnd-target`: a 2px dotted line where the drag would drop.
    let dnd_target = editor.dnd.target.and_then(|position| {
        let line = layout.line_for_point(position)?;
        let x = layout.text_origin_x() + line.x_for_column(position.column);
        Some(Bounds::new(
            point(x, layout.text_top(line.view_line)),
            size(px(2.), line_height),
        ))
    });
    let focused = editor.focus_handle.is_focused(window);
    editor.layout = Some(layout.clone());
    PrepaintState {
        layout,
        selections,
        cursor_view_lines,
        cursor_quads,
        gutter_numbers,
        fold_placeholders,
        fold_highlights,
        show_fold_controls,
        decoration_quads,
        decoration_underlines,
        gutter_bars,
        glyphs,
        lightbulb,
        ruler_marks,
        guides,
        whitespace,
        embedded,
        minimap_image,
        section_headers,
        placeholder,
        dnd_target,
        minimap_stale: std::mem::take(&mut editor.minimap_cache.stale),
        minimap_slider_visible: options.minimap.always_show_slider || editor.minimap_hovered,
        sticky: sticky_shaped,
        sticky_focused: editor.sticky.focused,
        sticky_hovered: editor.sticky.hovered,
        sticky_scroll_with_editor: options.sticky_scroll.scroll_with_editor,
        numbers_right,
        zone_elements: Vec::new(),
        full_width_zone_elements: Vec::new(),
        widget_elements: Vec::new(),
        colors,
        options,
        focused,
        cursor_visible: editor.cursor_visible,
        cursor_opacity,
        cursor_scale,
        middle_scroll_anchor: editor.animation.middle_scroll.map(|s| s.anchor),
        scrollbar_looks,
        overview_ruler: show_ruler_border.then_some(ruler),
    }
}

fn paint_editor(state: &mut PrepaintState, window: &mut Window, cx: &mut App) {
    let mut zone_elements = std::mem::take(&mut state.zone_elements);
    let mut full_width_zone_elements = std::mem::take(&mut state.full_width_zone_elements);
    let mut widget_elements = std::mem::take(&mut state.widget_elements);
    let state = &*state;
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
        if !has_selection
            && (highlight_line || highlight_gutter)
            && (state.focused || !state.options.render_line_highlight_only_when_focus)
        {
            let top = layout.line_top(state.cursor_view_lines[0]);
            let left = if highlight_gutter {
                bounds.left()
            } else {
                layout.text_bounds.left()
            };
            let line_bounds = Bounds::new(
                point(left, top),
                size(
                    layout.text_bounds.right() - left,
                    layout.line_height_of(state.cursor_view_lines[0]),
                ),
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
        // Monaco `foldingHighlight` (`editor.foldBackground`).
        for highlight in &state.fold_highlights {
            window.paint_quad(fill(*highlight, colors.fold_background));
        }

        // Gutter numbers.
        for (top, shaped, right) in &state.gutter_numbers {
            let origin = point(*right - shaped.width, *top);
            shaped
                .paint(origin, line_height, TextAlign::Left, None, window, cx)
                .ok();
        }

        for (bounds, color) in &state.gutter_bars {
            window.paint_quad(fill(*bounds, *color));
        }
        for (origin, shaped) in &state.glyphs {
            shaped
                .paint(*origin, line_height, TextAlign::Left, None, window, cx)
                .ok();
        }
        // Monaco shows the lightbulb only while the editor has focus.
        if state.focused
            && let Some((bulb, color)) = state.lightbulb
        {
            paint_lightbulb(bulb, color, window);
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
                for (bounds, background, border) in &state.decoration_quads {
                    window.paint_quad(quad(
                        *bounds,
                        Corners::default(),
                        background.unwrap_or(gpui::transparent_black()),
                        Edges::all(if border.is_some() { px(1.) } else { px(0.) }),
                        border.unwrap_or(gpui::transparent_black()),
                        BorderStyle::Solid,
                    ));
                }
                // Injected text backgrounds.
                for line in &layout.lines {
                    for (range, background, swatch) in &line.injected {
                        if let Some(swatch) = swatch {
                            // Monaco: a 0.8em square with a 0.1em border,
                            // centered in its 1.2em of room.
                            let from = line.shaped.x_for_index(range.start);
                            let to = line.shaped.x_for_index(range.end);
                            let side = (layout.metrics.font_size * 0.8).round();
                            let top = layout.text_top(line.view_line);
                            window.paint_quad(quad(
                                Bounds::new(
                                    point(
                                        origin_x + line.indent_x + from + (to - from - side) / 2.,
                                        top + (line_height - side) / 2.,
                                    ),
                                    size(side, side),
                                ),
                                Corners::default(),
                                *swatch,
                                Edges::all(px(1.)),
                                colors.color_decoration_border,
                                BorderStyle::Solid,
                            ));
                        }
                        if let Some(background) = background {
                            let from = line.shaped.x_for_index(range.start);
                            let to = line.shaped.x_for_index(range.end);
                            let top = layout.text_top(line.view_line);
                            window.paint_quad(quad(
                                Bounds::new(
                                    point(origin_x + line.indent_x + from, top + px(1.)),
                                    size(to - from, line_height - px(2.)),
                                ),
                                Corners::all(px(3.)),
                                *background,
                                Edges::default(),
                                gpui::transparent_black(),
                                BorderStyle::Solid,
                            ));
                        }
                    }
                }
                for (bounds, color) in &state.guides {
                    window.paint_quad(fill(*bounds, *color));
                }
                for selection in &state.selections {
                    if selection.is_empty() {
                        continue;
                    }
                    paint_selection(
                        layout,
                        selection,
                        origin_x,
                        selection_color,
                        state.options.rounded_selection,
                        window,
                    );
                }

                for (shaped, origin) in &state.placeholder {
                    shaped
                        .paint(*origin, line_height, TextAlign::Left, None, window, cx)
                        .ok();
                }
                for line in &layout.lines {
                    let top = layout.text_top(line.view_line);
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

                if let Some((dot, arrow, marks)) = &state.whitespace {
                    for (origin, is_tab) in marks {
                        let glyph = if *is_tab { arrow } else { dot };
                        glyph
                            .paint(*origin, line_height, TextAlign::Left, None, window, cx)
                            .ok();
                    }
                }

                for element in &mut zone_elements {
                    element.paint(window, cx);
                }

                for (origin, width, style) in &state.decoration_underlines {
                    window.paint_underline(*origin, *width, style);
                }

                // Folded region placeholder ("⋯").
                for (x, top) in &state.fold_placeholders {
                    paint_fold_placeholder(*x, *top, layout, colors, window);
                }

                if let Some(target) = state.dnd_target {
                    // Dotted: 2px dashes with 2px gaps.
                    let mut y = target.top();
                    while y < target.bottom() {
                        let dash = Bounds::new(
                            point(target.left(), y),
                            size(target.size.width, px(2.).min(target.bottom() - y)),
                        );
                        window.paint_quad(fill(dash, colors.cursor));
                        y += px(4.);
                    }
                }
                if state.focused && state.cursor_visible {
                    for cursor in &state.cursor_quads {
                        let height = cursor.size.height * state.cursor_scale;
                        let bounds = Bounds::new(
                            point(
                                cursor.left(),
                                cursor.top() + (cursor.size.height - height) / 2.,
                            ),
                            size(cursor.size.width, height),
                        );
                        window
                            .paint_quad(fill(bounds, colors.cursor.opacity(state.cursor_opacity)));
                    }
                }
            },
        );

        // Zones spanning the gutter, clipped to the text area's height.
        let text = layout.text_bounds;
        let full_width_area = Bounds::from_corners(
            point(bounds.left(), text.top()),
            point(text.right(), text.bottom()),
        );
        window.with_content_mask(
            Some(ContentMask {
                bounds: full_width_area,
            }),
            |window| {
                for element in &mut full_width_zone_elements {
                    element.paint(window, cx);
                }
            },
        );

        if !state.sticky.is_empty() {
            let background = colors.sticky_scroll_background.unwrap_or(colors.background);
            let height = state
                .sticky
                .iter()
                .map(|(_, top, _, _)| *top + line_height)
                .fold(px(0.), |a, b| a.max(b));
            let area = Bounds::new(
                bounds.origin,
                size(layout.text_bounds.right() - bounds.left(), height),
            );
            // Monaco `stickyScroll.scrollWithEditor`: the lines follow the
            // horizontal scroll.
            let origin_x = if state.sticky_scroll_with_editor {
                layout.text_origin_x()
            } else {
                layout.text_bounds.left()
            };
            window.with_content_mask(Some(ContentMask { bounds: area }), |window| {
                window.paint_quad(fill(area, background));
                for (ix, top, line, number) in &state.sticky {
                    let y = bounds.top() + *top;
                    let content = Bounds::new(
                        point(layout.text_bounds.left(), y),
                        size(layout.text_bounds.size.width, line_height),
                    );
                    // Hovered and keyboard-focused lines (Monaco
                    // `editorStickyScrollHover.background`, focus outline).
                    let focused = state.sticky_focused == Some(*ix);
                    if focused || state.sticky_hovered == Some(*ix) {
                        window.paint_quad(fill(content, colors.sticky_scroll_hover_background));
                    }
                    if focused {
                        window.paint_quad(quad(
                            content,
                            Corners::default(),
                            gpui::transparent_black(),
                            Edges::all(px(1.)),
                            colors.focus_border,
                            BorderStyle::Solid,
                        ));
                    }
                    if let Some(number) = number {
                        number
                            .paint(
                                point(state.numbers_right - number.width, y),
                                line_height,
                                TextAlign::Left,
                                None,
                                window,
                                cx,
                            )
                            .ok();
                    }
                    window.with_content_mask(
                        Some(ContentMask {
                            bounds: Bounds::new(
                                point(layout.text_bounds.left(), area.top()),
                                size(layout.text_bounds.size.width, area.size.height),
                            ),
                        }),
                        |window| {
                            line.shaped
                                .paint(
                                    point(origin_x + line.indent_x, y),
                                    line_height,
                                    TextAlign::Left,
                                    None,
                                    window,
                                    cx,
                                )
                                .ok();
                        },
                    );
                }
                // Folding controls (Monaco shows them as in the gutter).
                for sticky in &layout.sticky_lines {
                    if let Some(control) = &sticky.fold
                        && (control.collapsed || state.show_fold_controls)
                    {
                        paint_chevron(control, colors.line_number, window);
                    }
                }
                let shadow = Bounds::new(
                    point(area.left(), area.bottom() - px(1.)),
                    size(area.size.width, px(1.)),
                );
                window.paint_quad(fill(shadow, colors.scrollbar_shadow.opacity(0.6)));
            });
        }

        for element in &mut widget_elements {
            element.paint(window, cx);
        }

        // Monaco's `scroll-editor-on-middle-click-dot`: 5px, half opaque.
        if let Some(anchor) = state.middle_scroll_anchor {
            let dot = Bounds::new(anchor - point(px(3.5), px(3.5)), size(px(7.), px(7.)));
            window.paint_quad(quad(
                dot,
                Corners::all(px(3.5)),
                colors.foreground.opacity(0.5),
                Edges::all(px(1.)),
                colors.background.opacity(0.5),
                BorderStyle::Solid,
            ));
        }

        for (bounds, color) in &state.ruler_marks {
            window.paint_quad(fill(*bounds, *color));
        }
        if let Some(ruler) = state.overview_ruler {
            window.paint_quad(fill(
                Bounds::new(ruler.origin, size(px(1.), ruler.size.height)),
                colors.overview_ruler_border,
            ));
        }
        for (ix, scrollbar) in [layout.vertical_scrollbar, layout.horizontal_scrollbar]
            .into_iter()
            .enumerate()
        {
            let Some(scrollbar) = scrollbar else {
                continue;
            };
            let (opacity, hovered, dragged) = state.scrollbar_looks[ix];
            if opacity <= 0. {
                continue;
            }
            let color = if dragged {
                colors.scrollbar_slider_active
            } else if hovered {
                colors.scrollbar_slider_hover
            } else {
                colors.scrollbar_slider
            };
            window.paint_quad(fill(scrollbar.thumb, color.opacity(opacity)));
            if let Some(arrows) = scrollbar.arrows {
                for (end, arrow) in arrows.into_iter().enumerate() {
                    paint_scrollbar_arrow(
                        arrow,
                        ix == 0,
                        end == 0,
                        colors.line_number.opacity(opacity),
                        window,
                    );
                }
            }
        }
        // Free replaced minimap images from the atlas.
        for image in &state.minimap_stale {
            window.drop_image(image.clone()).ok();
        }
        if let Some(minimap) = layout.minimap {
            // Its own background, also where it sits left of the gutter.
            window.paint_quad(fill(
                minimap.bounds,
                colors.minimap_background.unwrap_or(colors.background),
            ));
            window.with_content_mask(
                Some(ContentMask {
                    bounds: minimap.bounds,
                }),
                |window| {
                    if let Some((image, image_bounds)) = &state.minimap_image {
                        window
                            .paint_image(
                                minimap.bounds,
                                *image_bounds,
                                Corners::default(),
                                image.clone(),
                                0,
                                false,
                            )
                            .ok();
                    }
                    for MinimapSectionHeader {
                        band,
                        label,
                        separator,
                    } in &state.section_headers
                    {
                        if let Some((label, origin)) = label {
                            window.paint_quad(fill(*band, colors.background.opacity(0.7)));
                            label
                                .paint(*origin, band.size.height, TextAlign::Left, None, window, cx)
                                .ok();
                        }
                        if let Some(separator) = separator {
                            window.paint_quad(fill(*separator, colors.foreground.opacity(0.4)));
                        }
                    }
                    if state.minimap_slider_visible {
                        window.paint_quad(fill(minimap.slider, colors.minimap_slider));
                    }
                },
            );
        }
        // Monaco `scrollbar.useShadows`: thin shadows under the top edge
        // and along the text's left edge once scrolled.
        if state.options.scrollbar.use_shadows {
            if layout.scroll_top > px(0.) {
                let shadow = Bounds::new(
                    point(bounds.left(), bounds.top()),
                    Size::new(bounds.size.width, px(1.)),
                );
                window.paint_quad(fill(shadow, colors.scrollbar_shadow.opacity(0.6)));
            }
            if layout.scroll_left > px(0.) {
                let shadow = Bounds::new(
                    point(layout.text_bounds.left(), bounds.top()),
                    Size::new(px(1.), layout.text_bounds.size.height),
                );
                window.paint_quad(fill(shadow, colors.scrollbar_shadow.opacity(0.6)));
            }
        }
    });
}

/// The x and width of an overview ruler `lane` with `lanes` lanes (Monaco
/// `DecorationsOverviewRuler` settings: one lane is the whole width, two
/// split it in halves with the center on the left).
fn overview_lane(
    track: Bounds<Pixels>,
    lanes: u32,
    lane: crate::decorations::OverviewRulerLane,
) -> (Pixels, Pixels) {
    use crate::decorations::OverviewRulerLane as Lane;
    let width = track.size.width;
    match (lanes, lane) {
        (_, Lane::Full) | (1, _) => (track.left(), width),
        (2, Lane::Left | Lane::Center) => (track.left(), width / 2.),
        (2, Lane::Right) => (track.left() + width / 2., width / 2.),
        (_, Lane::Left) => (track.left(), width / 3.),
        (_, Lane::Center) => (track.left() + width / 3., width / 3.),
        (_, Lane::Right) => (track.left() + width * 2. / 3., width / 3.),
    }
}

/// A scrollbar arrow: a small triangle pointing out of the bar.
fn paint_scrollbar_arrow(
    bounds: Bounds<Pixels>,
    vertical: bool,
    start: bool,
    color: Hsla,
    window: &mut Window,
) {
    let center = bounds.center();
    let r = bounds.size.width.min(bounds.size.height) * 0.3;
    let points = match (vertical, start) {
        (true, true) => [
            point(center.x, center.y - r),
            point(center.x + r, center.y + r / 2.),
            point(center.x - r, center.y + r / 2.),
        ],
        (true, false) => [
            point(center.x, center.y + r),
            point(center.x - r, center.y - r / 2.),
            point(center.x + r, center.y - r / 2.),
        ],
        (false, true) => [
            point(center.x - r, center.y),
            point(center.x + r / 2., center.y - r),
            point(center.x + r / 2., center.y + r),
        ],
        (false, false) => [
            point(center.x + r, center.y),
            point(center.x - r / 2., center.y + r),
            point(center.x - r / 2., center.y - r),
        ],
    };
    let mut path = PathBuilder::fill();
    path.move_to(points[0]);
    path.line_to(points[1]);
    path.line_to(points[2]);
    path.close();
    if let Ok(path) = path.build() {
        window.paint_path(path, color);
    }
}

/// A lightbulb: a round bulb over a small base.
fn paint_lightbulb(bounds: Bounds<Pixels>, color: Hsla, window: &mut Window) {
    let h = bounds.size.height;
    let center = point(
        bounds.left() + bounds.size.width / 2.,
        bounds.top() + h * 0.4,
    );
    let radius = h * 0.3;
    let mut path = PathBuilder::fill();
    for i in 0..=16 {
        let angle = std::f32::consts::TAU * i as f32 / 16.;
        let p = point(
            center.x + radius * angle.cos(),
            center.y + radius * angle.sin(),
        );
        if i == 0 {
            path.move_to(p);
        } else {
            path.line_to(p);
        }
    }
    path.close();
    if let Ok(path) = path.build() {
        window.paint_path(path, color);
    }
    let base = Bounds::new(
        point(center.x - h * 0.13, bounds.top() + h * 0.7),
        size(h * 0.26, h * 0.16),
    );
    window.paint_quad(fill(base, color));
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
    rounded: bool,
    window: &mut Window,
) {
    let mut rows: Vec<Bounds<Pixels>> = Vec::new();
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
            rows.push(Bounds::new(
                point(origin_x + from, layout.line_top(line.view_line)),
                size(to - from, layout.line_height_of(line.view_line)),
            ));
        }
    }
    // Monaco `roundedSelection`: 3px on the outer corners.
    for (ix, bounds) in rows.iter().enumerate() {
        let span = |b: &Bounds<Pixels>| (b.left(), b.right());
        let adjacent = |other: Option<&Bounds<Pixels>>| {
            other
                .filter(|o| {
                    // Touching rows, whatever their heights.
                    (o.bottom() - bounds.top()).abs() <= px(0.5)
                        || (bounds.bottom() - o.top()).abs() <= px(0.5)
                })
                .map(span)
        };
        let corners = if rounded {
            let [tl, tr, br, bl] = crate::view::animation::rounded_corners(
                span(bounds),
                adjacent(ix.checked_sub(1).and_then(|i| rows.get(i))),
                adjacent(rows.get(ix + 1)),
            );
            let r = |round: bool| if round { px(3.) } else { px(0.) };
            Corners {
                top_left: r(tl),
                top_right: r(tr),
                bottom_right: r(br),
                bottom_left: r(bl),
            }
        } else {
            Corners::default()
        };
        window.paint_quad(quad(
            *bounds,
            corners,
            color,
            Edges::default(),
            gpui::transparent_black(),
            BorderStyle::Solid,
        ));
    }
}

/// X range of `start..end` (model points) on `line`, or `None` if it doesn't
/// intersect. Ranges continuing past the line end stop at the text end.
fn decoration_x_range(
    line: &LineLayout,
    start: PointUtf16,
    end: PointUtf16,
) -> Option<(Pixels, Pixels)> {
    if line.row < start.row || line.row > end.row {
        return None;
    }
    let from_col = if line.row == start.row {
        start.column
    } else {
        0
    };
    let to_col = if line.row == end.row {
        end.column
    } else {
        u32::MAX
    };
    if to_col < line.columns.start || from_col > line.columns.end {
        return None;
    }
    if from_col == to_col && !(line.columns.start <= from_col && from_col <= line.columns.end) {
        return None;
    }
    let from = line.x_for_column(from_col.max(line.columns.start));
    let to = if to_col > line.columns.end {
        line.width()
    } else {
        line.x_for_column(to_col)
    };
    Some((from, to.max(from)))
}

/// Overlays `top` styles on `base` (both sorted, non-overlapping byte
/// ranges): `top`'s foreground replaces, font flags combine.
fn overlay_styles(
    base: &[(Range<usize>, TokenStyle)],
    top: &[(Range<usize>, TokenStyle)],
) -> Vec<(Range<usize>, TokenStyle)> {
    let mut bounds: Vec<usize> = base
        .iter()
        .chain(top.iter())
        .flat_map(|(range, _)| [range.start, range.end])
        .collect();
    bounds.sort_unstable();
    bounds.dedup();
    let mut result: Vec<(Range<usize>, TokenStyle)> = Vec::new();
    for pair in bounds.windows(2) {
        let (start, end) = (pair[0], pair[1]);
        let covering = |spans: &[(Range<usize>, TokenStyle)]| {
            spans
                .iter()
                .rev()
                .find(|(range, _)| range.start <= start && end <= range.end)
                .map(|(_, style)| *style)
        };
        let base_style = covering(base);
        let top_style = covering(top);
        let style = match (base_style, top_style) {
            (None, None) => continue,
            (Some(style), None) | (None, Some(style)) => style,
            (Some(base), Some(top)) => TokenStyle {
                foreground: top.foreground.or(base.foreground),
                bold: base.bold || top.bold,
                italic: base.italic || top.italic,
                underline: base.underline || top.underline,
                strikethrough: base.strikethrough || top.strikethrough,
                opacity: match (base.opacity, top.opacity) {
                    (Some(a), Some(b)) => Some(a * b),
                    (a, b) => a.or(b),
                },
            },
        };
        match result.last_mut() {
            Some((range, last)) if range.end == start && *last == style => range.end = end,
            _ => result.push((start..end, style)),
        }
    }
    result
}

/// An embedded view waiting to be laid out.
pub(crate) enum Embedded {
    Zone {
        view: gpui::AnyView,
        bounds: Bounds<Pixels>,
        /// Painted over the gutter, outside the text area's clip.
        full_width: bool,
    },
    Content {
        view: gpui::AnyView,
        /// Top-left of the anchored character's line.
        anchor: gpui::Point<Pixels>,
        /// The line's height.
        line_height: Pixels,
        preference: Vec<ContentWidgetPreference>,
    },
    Overlay {
        view: gpui::AnyView,
        position: OverlayPosition,
    },
}

/// Minimap blocks for view lines `lines`, relative to the first one.
#[allow(clippy::too_many_arguments)]
/// How the minimap draws (Monaco's minimap layout: char width and line
/// height in device pixels, and `renderCharacters`).
#[derive(Clone, Copy, Debug, PartialEq)]
struct MinimapRender {
    char_width: f32,
    line_height: f32,
    /// Monaco `renderCharacters`: a two-pixel-row impression of each
    /// character rather than solid blocks.
    characters: bool,
    max_column: usize,
    /// The image's width in device pixels.
    width: u32,
}

/// How dark a character is in the top and bottom halves of its minimap
/// cell, standing in for Monaco's prerendered minimap glyphs.
fn minimap_glyph(ch: char) -> (f32, f32) {
    match ch {
        'g' | 'j' | 'p' | 'q' | 'y' => (0.45, 0.9),
        'b' | 'd' | 'f' | 'h' | 'k' | 'l' | 't' | 'i' => (0.75, 0.7),
        c if c.is_lowercase() => (0.35, 0.8),
        c if c.is_uppercase() || c.is_ascii_digit() => (0.8, 0.75),
        '.' | ',' | '_' => (0., 0.6),
        ':' | ';' => (0.35, 0.5),
        '-' | '=' | '+' | '~' | '*' => (0.4, 0.3),
        '\'' | '"' | '`' | '^' => (0.6, 0.),
        _ => (0.6, 0.6),
    }
}

fn minimap_image_for(
    highlights: Option<&HighlightSource>,
    view_lines: &ViewLines,
    snapshot: &text::BufferSnapshot,
    theme: &Theme,
    lines: &[u32],
    tab_size: u32,
    render: MinimapRender,
) -> Option<(std::sync::Arc<gpui::RenderImage>, (u32, u32))> {
    let width = render.width;
    // Device pixels per character and per line.
    let char_px = render.char_width.max(1.).round() as u32;
    let line_px = render.line_height.max(1.).round() as u32;
    let height = line_px * lines.len() as u32;
    if width == 0 || height == 0 {
        return None;
    }
    let mut pixels = vec![0u8; (width * height * 4) as usize];
    let mut fill = |x: u32, y: u32, w: u32, h: u32, color: Hsla| {
        let rgba = gpui::Rgba::from(color);
        let bgra = [
            (rgba.b * 255.) as u8,
            (rgba.g * 255.) as u8,
            (rgba.r * 255.) as u8,
            (rgba.a * 255.) as u8,
        ];
        for row in y..(y + h).min(height) {
            let start = ((row * width + x.min(width)) * 4) as usize;
            let end = ((row * width + (x + w).min(width)) * 4) as usize;
            for pixel in pixels[start..end].as_chunks_mut::<4>().0 {
                pixel.copy_from_slice(&bgra);
            }
        }
    };
    let max_column = render.max_column;
    let foreground = theme.colors.foreground.opacity(0.6);
    let (Some(first), Some(last)) = (lines.first(), lines.last()) else {
        return None;
    };
    let first_row = view_lines.view_line(*first).row;
    let last_row = view_lines.view_line(*last).row;
    // Contiguous lines share one highlight query; sampled ones get their own.
    let contiguous = (*last - *first) as usize + 1 == lines.len();
    let row_range = |first_row: u32, last_row: u32| {
        snapshot.point_to_offset(text::Point::new(first_row, 0))
            ..snapshot.point_to_offset(text::Point::new(last_row, u32::MAX))
    };
    let shared = if contiguous {
        highlights.map_or_else(Vec::new, |source| {
            source.highlights(theme, row_range(first_row, last_row))
        })
    } else {
        Vec::new()
    };
    let tab_size = tab_size.max(1) as usize;
    for (ix, view_line) in lines.iter().copied().enumerate() {
        let segment = row_segment(view_lines, view_line);
        let line_highlights;
        let highlights: &[(Range<usize>, TokenStyle)] = if contiguous {
            &shared
        } else {
            line_highlights = highlights.map_or_else(Vec::new, |source| {
                source.highlights(theme, row_range(segment.row, segment.row))
            });
            &line_highlights
        };
        let mut next_highlight = 0;
        let full = snapshot.line(segment.row);
        let line_start = snapshot.point_to_offset(text::Point::new(segment.row, 0));
        let byte_start = utf16_to_byte(&full, segment.columns.start);
        let byte_end = utf16_to_byte(&full, segment.columns.end);
        let y = line_px * ix as u32;
        let mut column = segment.indent as usize + segment.start_visible_column as usize
            - if segment.columns.start > 0 {
                segment.start_visible_column as usize
            } else {
                0
            };
        for (byte, ch) in full[byte_start..byte_end].char_indices() {
            if column >= max_column {
                break;
            }
            let span = if ch == '\t' {
                tab_size - column % tab_size
            } else {
                1
            };
            if !ch.is_whitespace() {
                let offset = line_start + byte_start + byte;
                while highlights
                    .get(next_highlight)
                    .is_some_and(|(r, _)| r.end <= offset)
                {
                    next_highlight += 1;
                }
                let color = highlights
                    .get(next_highlight)
                    .filter(|(r, _)| r.start <= offset)
                    .and_then(|(_, style)| style.foreground)
                    .map_or(foreground, |c| c.opacity(0.75));
                let x = column as u32 * char_px;
                let w = span as u32 * char_px;
                if render.characters && line_px >= 2 {
                    // Monaco's glyphs, as two bands of darkness.
                    let (top, bottom) = minimap_glyph(ch);
                    let half = line_px / 2;
                    fill(x, y, w, half, color.opacity(top));
                    fill(x, y + half, w, line_px - half, color.opacity(bottom));
                } else if render.characters {
                    let (top, bottom) = minimap_glyph(ch);
                    fill(x, y, w, 1, color.opacity((top + bottom) / 2.));
                } else {
                    // Blocks leave a gap between lines.
                    let h = (line_px - (line_px / 3).max(1)).max(1);
                    fill(x, y, w, h, color);
                }
            }
            column += span;
        }
    }
    let buffer = image::RgbaImage::from_raw(width, height, pixels)?;
    Some((
        std::sync::Arc::new(gpui::RenderImage::new(vec![image::Frame::new(buffer)])),
        (width, height),
    ))
}
