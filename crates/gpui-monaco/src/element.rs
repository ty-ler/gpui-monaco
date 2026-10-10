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
    pub injected: Vec<(Range<usize>, Option<Hsla>)>,
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
    /// Show control characters as their Unicode control pictures.
    pub render_control_characters: bool,
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
            render_control_characters: options.render_control_characters,
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
    let mut injected: Vec<(Range<usize>, TokenStyle, Option<Hsla>)> = Vec::new();
    let mut injection_ix = 0;
    // Emits injections at `column` (`after` ones first) and returns where the
    // column itself displays: before `after` text (it ends a decorated range)
    // or after `before` text (it starts one).
    let mut inject_at = |column: u32,
                         display: &mut String,
                         injected: &mut Vec<(Range<usize>, TokenStyle, Option<Hsla>)>|
     -> usize {
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
                    injected.push((start..display.len(), text.style, text.background));
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
    for (range, _, _) in &injected {
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
            .find(|(range, _, _)| range.start <= start && end <= range.end)
            .map(|(_, style, _)| *style)
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
        injected: injected
            .into_iter()
            .map(|(range, _, background)| (range, background))
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
    pub track: Bounds<Pixels>,
    pub thumb: Bounds<Pixels>,
    /// Content pixels per track pixel.
    pub ratio: f32,
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
    /// Blocks relative to the minimap's first drawn line.
    blocks: Vec<(Bounds<Pixels>, Hsla)>,
    /// A rebuild running in the background; until it lands the old blocks
    /// are drawn (a frame-stale minimap is invisible, ~3ms per keystroke on
    /// the main thread is not).
    pending: Option<(MinimapKey, std::sync::Arc<gpui::Task<()>>)>,
}

type MinimapKey = (text::Version, u32, u32, usize, u32);

/// A fold control in the gutter.
#[derive(Clone, Copy, Debug)]
pub(crate) struct FoldControl {
    pub row: u32,
    pub bounds: Bounds<Pixels>,
    pub collapsed: bool,
}

/// Vertical positions with view zones: zone `i` sits after view line
/// `after` (-1 for above the first line).
#[derive(Clone, Debug, Default)]
pub(crate) struct ZoneLayout {
    /// (after view line, height), sorted by `after`.
    zones: Vec<(i64, Pixels)>,
}

impl ZoneLayout {
    pub fn new(mut zones: Vec<(i64, Pixels)>) -> Self {
        zones.sort_by_key(|(after, _)| *after);
        Self { zones }
    }

    pub fn total(&self) -> Pixels {
        self.zones
            .iter()
            .map(|(_, h)| *h)
            .fold(px(0.), |a, b| a + b)
    }

    /// Total height of zones above view line `line`.
    pub fn above(&self, line: u32) -> Pixels {
        self.zones
            .iter()
            .take_while(|(after, _)| *after < line as i64)
            .map(|(_, h)| *h)
            .fold(px(0.), |a, b| a + b)
    }

    /// Content y of the top of view line `line`.
    pub fn line_y(&self, line: u32, line_height: Pixels) -> Pixels {
        line_height * line as f32 + self.above(line)
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
    pub zones: ZoneLayout,
    /// Bounds of embedded views; the editor ignores clicks inside them.
    pub widget_bounds: Vec<Bounds<Pixels>>,
    /// What each of `widget_bounds` is.
    pub widget_kinds: Vec<crate::mouse_target::EmbeddedKind>,
    /// The code action lightbulb, for clicks.
    pub lightbulb: Option<Bounds<Pixels>>,
    pub minimap: Option<MinimapLayout>,
    /// Sticky scroll lines: (model row, bounds).
    pub sticky_lines: Vec<(u32, Bounds<Pixels>)>,
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
    minimap_blocks: Vec<(Bounds<Pixels>, Hsla)>,
    minimap_slider_visible: bool,
    sticky: Vec<(u32, Pixels, LineLayout, Option<ShapedLine>)>,
    numbers_right: Pixels,
    zone_elements: Vec<gpui::AnyElement>,
    full_width_zone_elements: Vec<gpui::AnyElement>,
    widget_elements: Vec<gpui::AnyElement>,
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
        let mut state = self
            .editor
            .update(cx, |editor, cx| prepaint_editor(editor, bounds, window, cx));
        let line_height = state.layout.metrics.line_height;
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
    let text_width = bounds.size.width - gutter_width - vbar - minimap_width;

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

    // View zones: after the last view line of their row; hidden in folds.
    let mut zone_specs: Vec<(i64, Pixels, gpui::AnyView, bool)> = Vec::new();
    {
        let model = editor.model().clone();
        let buffer = model.read(cx).buffer();
        for zone in &editor.widgets.zones {
            let after = match &zone.anchor {
                None => -1,
                Some(anchor) => {
                    let row = buffer.offset_to_point(buffer.resolve(anchor)).row;
                    if view_lines.is_row_hidden(row) {
                        continue;
                    }
                    let first = view_lines.first_view_line_of_row(row);
                    let next = view_lines.first_view_line_of_row(row + 1);
                    next.max(first + 1) as i64 - 1
                }
            };
            let height = match zone.height {
                crate::widgets::ZoneHeight::Lines(lines) => line_height * lines,
                crate::widgets::ZoneHeight::Pixels(height) => height,
            };
            zone_specs.push((after, height, zone.view.clone(), zone.full_width));
        }
    }
    let zones = ZoneLayout::new(zone_specs.iter().map(|(a, h, _, _)| (*a, *h)).collect());

    let content_height = line_height * view_line_count as f32
        + zones.total()
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
                box_end: f32::from(zones.line_y(end, line_height) + line_height),
                line_height: f32::from(line_height),
                surrounding_lines: surrounding,
            },
            reveal,
        );
        editor.scroll_top = px(top);
        editor.pending_reveal_x = Some(range.start);
    }
    if editor.autoscroll_requested {
        let top = zones.line_y(primary_view_line, line_height);
        let bottom = top + line_height;
        if top < editor.scroll_top {
            editor.scroll_top = top;
        } else if bottom > editor.scroll_top + text_height {
            editor.scroll_top = bottom - text_height;
        }
    }
    editor.scroll_top = editor.scroll_top.clamp(px(0.), max_scroll_top);
    let scroll_top = editor.scroll_top;
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
    let highlights = {
        let model = editor.model().read(cx);
        current_highlights(model, &theme, visible_start..visible_end)
    };
    let editor_id = cx.entity_id();
    let decorations = editor.model().update(cx, |model, _| {
        model.decorations_in_range(visible_start..visible_end, Some(editor_id))
    });
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
            if let Some(style) = options.text {
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
    if let Some(position) = editor.pending_reveal_x.take()
        && let Some(x) = lines
            .iter()
            .find(|line| line.contains(position))
            .map(|line| line.x_for_column(position.column))
    {
        let padding = metrics.em_width * 2.;
        if x < editor.scroll_left {
            editor.scroll_left = (x - padding).max(px(0.));
        } else if x + padding > editor.scroll_left + text_width {
            editor.scroll_left = x + padding - text_width;
        }
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
        let model = editor.model().clone();
        let buffer = model.read(cx).buffer();
        let collapsed = editor.folds.collapsed_rows(buffer);
        let left = bounds.left() + glyph_width + numbers_width + options.line_decorations_width;
        for line in lines.iter().filter(|line| line.columns.start == 0) {
            let top = bounds.top() + zones.line_y(line.view_line, line_height) - scroll_top;
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
                let top = bounds.top() + zones.line_y(line.view_line, line_height) - scroll_top;
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
        zones: zones.clone(),
        widget_bounds: Vec::new(),
        widget_kinds: Vec::new(),
        lightbulb: None,
        minimap: None,
        sticky_lines: Vec::new(),
        vertical_scrollbar,
        horizontal_scrollbar,
    };

    // Minimap: token-colored blocks, `scale` px per column and 2*scale px
    // per view line, scrolled proportionally (Monaco `minimap`).
    let mut minimap_blocks = Vec::new();
    if minimap_width > px(0.) {
        let scale = options.minimap.scale;
        let mline = px(2. * scale);
        let minimap_bounds = Bounds::new(
            point(bounds.right() - vbar - minimap_width, bounds.top()),
            size(minimap_width, viewport_height),
        );
        let total = mline * view_line_count as f32;
        let minimap_scroll = if total > viewport_height && max_scroll_top > px(0.) {
            (total - viewport_height) * (scroll_top / max_scroll_top)
        } else {
            px(0.)
        };
        let first = (minimap_scroll / mline).floor() as u32;
        let last = (((minimap_scroll + viewport_height) / mline).ceil() as u32)
            .min(view_line_count.saturating_sub(1));
        let theme_key = std::sync::Arc::as_ptr(&theme) as usize;
        let key = (
            snapshot.version(),
            first,
            last,
            theme_key,
            wrap_column.unwrap_or(0),
        );
        let cache = &editor.minimap_cache;
        if cache.key != Some(key) && cache.pending.as_ref().is_none_or(|(k, _)| *k != key) {
            let source = HighlightSource::new(editor.model().read(cx));
            let build = {
                let (view_lines, snapshot, theme, options) = (
                    view_lines.clone(),
                    snapshot.clone(),
                    theme.clone(),
                    options.clone(),
                );
                move || {
                    minimap_blocks_for(
                        source.as_ref(),
                        &view_lines,
                        &snapshot,
                        &theme,
                        first..last + 1,
                        tab_size,
                        &options,
                    )
                }
            };
            if cache.key.is_none() {
                // Nothing to show yet: build the first one now.
                editor.minimap_cache = MinimapCache {
                    key: Some(key),
                    first,
                    blocks: build(),
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
                            cache.blocks = blocks;
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
        minimap_blocks = editor
            .minimap_cache
            .blocks
            .iter()
            .map(|(b, color)| {
                (
                    Bounds::new(
                        point(minimap_bounds.left() + b.origin.x, origin_y + b.origin.y),
                        b.size,
                    ),
                    *color,
                )
            })
            .collect();
        let first_visible = zones.line_at(scroll_top, line_height, view_line_count);
        let visible_lines = (viewport_height / line_height).max(1.);
        let slider = Bounds::new(
            point(
                minimap_bounds.left(),
                minimap_bounds.top() + mline * first_visible as f32 - minimap_scroll,
            ),
            size(minimap_width, mline * visible_lines),
        );
        let travel = (total.min(viewport_height) - slider.size.height).max(px(1.));
        layout.minimap = Some(MinimapLayout {
            bounds: minimap_bounds,
            slider,
            drag_ratio: max_scroll_top / travel,
            line_height: mline,
            scroll: minimap_scroll,
        });
    }

    // Sticky scroll: headers of the fold ranges enclosing the top of the
    // viewport (Monaco `stickyScroll`, folding-provider model).
    let mut sticky_shaped: Vec<(u32, Pixels, LineLayout, Option<ShapedLine>)> = Vec::new();
    if options.sticky_scroll.enabled && !options.single_line && scroll_top > px(0.) {
        let max = options.sticky_scroll.max_line_count as usize;
        let mut sticky_rows: Vec<(u32, u32)> = Vec::new();
        while sticky_rows.len() < max {
            let probe_y = scroll_top + line_height * sticky_rows.len() as f32;
            let probe_row = view_lines
                .view_line(zones.line_at(probe_y, line_height, view_line_count))
                .row;
            // The outermost range enclosing the probe below the last header.
            let next = editor
                .folds
                .ranges_enclosing(probe_row)
                .into_iter()
                .find(|range| {
                    sticky_rows
                        .last()
                        .is_none_or(|(start, _)| *start < range.start_row)
                });
            match next {
                Some(range) => sticky_rows.push((range.start_row, range.end_row)),
                None => break,
            }
        }
        for (ix, (row, end_row)) in sticky_rows.iter().enumerate() {
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
            layout.sticky_lines.push((
                *row,
                Bounds::new(
                    point(bounds.left(), bounds.top() + top),
                    size(gutter_width + text_width, line_height),
                ),
            ));
            sticky_shaped.push((*row, top, shaped, number));
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
            if options.background.is_some() || options.border.is_some() {
                let range = if options.is_whole_line {
                    Some((px(0.) - layout.scroll_left, text_width + layout.scroll_left))
                } else {
                    decoration_x_range(line, *start, *end)
                };
                if let Some((from, to)) = range {
                    let bounds = Bounds::new(
                        point(origin_x + from, top),
                        size((to - from).max(px(1.)), line_height),
                    );
                    decoration_quads.push((bounds, options.background, options.border));
                }
            }
            if let Some(underline) = options.underline
                && let Some((from, to)) = decoration_x_range(line, *start, *end)
            {
                decoration_underlines.push((
                    point(origin_x + from, top + line_height - px(3.)),
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
                gutter_bars.push((
                    Bounds::new(point(left, top), size(px(3.), line_height)),
                    color,
                ));
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
                    glyphs.push((point(x, top), shaped));
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
                    let top = layout.line_top(line.view_line) + (line_height - side) / 2.;
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
                    let top = layout.line_top(line.view_line) + (line_height - side) / 2.;
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
    if let Some(track) = layout.vertical_scrollbar.map(|bar| bar.track).or_else(|| {
        Some(Bounds::new(
            point(bounds.right() - vbar, bounds.top()),
            size(vbar, viewport_height),
        ))
    }) {
        let all = editor
            .model()
            .update(cx, |model, _| model.all_decorations(Some(editor_id)));
        let total = view_line_count.max(1) as f32;
        for decoration in all {
            let Some((color, lane)) = decoration.options.overview_ruler else {
                continue;
            };
            let row = snapshot.offset_to_point(decoration.range.start).row;
            let (view_line, _) = view_lines.to_view(PointUtf16::new(row, 0), false);
            let y = track.top() + track.size.height * (view_line as f32 / total);
            let third = track.size.width / 3.;
            let (x, w) = match lane {
                crate::decorations::OverviewRulerLane::Left => (track.left(), third),
                crate::decorations::OverviewRulerLane::Center => (track.left() + third, third),
                crate::decorations::OverviewRulerLane::Right => (track.left() + third * 2., third),
                crate::decorations::OverviewRulerLane::Full => (track.left(), track.size.width),
            };
            ruler_marks.push((Bounds::new(point(x, y), size(w, px(3.))), color));
        }
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
            let bounds = match style {
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

    // Indent guides (Monaco `IndentGuidesOverlay`) and rulers.
    let mut guides = Vec::new();
    if let (Some(first), Some(last)) = (layout.lines.first(), layout.lines.last())
        && options.guides.indentation
    {
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
        let counts = crate::view::guides::lines_indent_guides(rope, rows.clone(), ctx);
        let active = options.guides.highlight_active_indentation.then(|| {
            crate::view::guides::active_indent_guide(rope, primary.row, rows.start, last.row, ctx)
        });
        let origin_x = layout.text_origin_x();
        for line in &layout.lines {
            let count = counts[(line.row - rows.start) as usize];
            let top = layout.line_top(line.view_line);
            for level in 1..=count {
                let x = origin_x + metrics.em_width * ((level - 1) * ctx.indent_size) as f32;
                let is_active = active.is_some_and(|a| {
                    a.indent == level && a.start_row <= line.row && line.row <= a.end_row
                });
                let color = if is_active {
                    colors.indent_guide_active
                } else {
                    colors.indent_guide
                };
                guides.push((Bounds::new(point(x, top), size(px(1.), line_height)), color));
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
                let top = layout.line_top(line.view_line);
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
            gutter_numbers.push((layout.line_top(line.view_line), shaped, right));
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
        decoration_quads,
        decoration_underlines,
        gutter_bars,
        glyphs,
        lightbulb,
        ruler_marks,
        guides,
        whitespace,
        embedded,
        minimap_blocks,
        minimap_slider_visible: options.minimap.always_show_slider || editor.minimap_hovered,
        sticky: sticky_shaped,
        numbers_right,
        zone_elements: Vec::new(),
        full_width_zone_elements: Vec::new(),
        widget_elements: Vec::new(),
        colors,
        options,
        focused,
        cursor_visible: editor.cursor_visible,
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
                    for (range, background) in &line.injected {
                        if let Some(background) = background {
                            let from = line.shaped.x_for_index(range.start);
                            let to = line.shaped.x_for_index(range.end);
                            let top = layout.line_top(line.view_line);
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

                if state.focused && state.cursor_visible {
                    for cursor in &state.cursor_quads {
                        window.paint_quad(fill(*cursor, colors.cursor));
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
            window.with_content_mask(Some(ContentMask { bounds: area }), |window| {
                window.paint_quad(fill(area, background));
                for (_, top, line, number) in &state.sticky {
                    let y = bounds.top() + *top;
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
                                    point(layout.text_origin_x() + line.indent_x, y),
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

        for (bounds, color) in &state.ruler_marks {
            window.paint_quad(fill(*bounds, *color));
        }
        for scrollbar in [layout.vertical_scrollbar, layout.horizontal_scrollbar]
            .into_iter()
            .flatten()
        {
            window.paint_quad(fill(scrollbar.thumb, colors.scrollbar_slider));
        }
        if let Some(minimap) = layout.minimap {
            if let Some(background) = colors.minimap_background {
                window.paint_quad(fill(minimap.bounds, background));
            }
            window.with_content_mask(
                Some(ContentMask {
                    bounds: minimap.bounds,
                }),
                |window| {
                    for (block, color) in &state.minimap_blocks {
                        window.paint_quad(fill(*block, *color));
                    }
                    if state.minimap_slider_visible {
                        window.paint_quad(fill(minimap.slider, colors.minimap_slider));
                    }
                },
            );
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
        preference: Vec<ContentWidgetPreference>,
    },
    Overlay {
        view: gpui::AnyView,
        position: OverlayPosition,
    },
}

/// Minimap blocks for view lines `lines`, relative to the first one.
#[allow(clippy::too_many_arguments)]
fn minimap_blocks_for(
    highlights: Option<&HighlightSource>,
    view_lines: &ViewLines,
    snapshot: &text::BufferSnapshot,
    theme: &Theme,
    lines: Range<u32>,
    tab_size: u32,
    options: &EditorOptions,
) -> Vec<(Bounds<Pixels>, Hsla)> {
    let scale = options.minimap.scale;
    let mline = 2. * scale;
    let max_column = options.minimap.max_column as usize;
    let foreground = theme.colors.foreground.opacity(0.6);
    let first_row = view_lines.view_line(lines.start).row;
    let last_row = view_lines.view_line(lines.end.saturating_sub(1)).row;
    let range = snapshot.point_to_offset(text::Point::new(first_row, 0))
        ..snapshot.point_to_offset(text::Point::new(last_row, u32::MAX));
    let highlights = highlights.map_or_else(Vec::new, |source| source.highlights(theme, range));
    let tab_size = tab_size.max(1) as usize;
    let mut blocks = Vec::new();
    // Highlights are sorted and offsets only increase, so walk them with a
    // cursor.
    let mut next_highlight = 0;
    for (ix, view_line) in lines.enumerate() {
        let segment = row_segment(view_lines, view_line);
        let full = snapshot.line(segment.row);
        let line_start = snapshot.point_to_offset(text::Point::new(segment.row, 0));
        let byte_start = utf16_to_byte(&full, segment.columns.start);
        let byte_end = utf16_to_byte(&full, segment.columns.end);
        let y = px(mline * ix as f32);
        let mut column = segment.indent as usize + segment.start_visible_column as usize
            - if segment.columns.start > 0 {
                segment.start_visible_column as usize
            } else {
                0
            };
        let mut run: Option<(usize, usize, Hsla)> = None;
        let flush = |run: &mut Option<(usize, usize, Hsla)>,
                     blocks: &mut Vec<(Bounds<Pixels>, Hsla)>| {
            if let Some((start, end, color)) = run.take() {
                let start = start.min(max_column);
                let end = end.min(max_column);
                if end > start {
                    blocks.push((
                        Bounds::new(
                            point(px(start as f32 * scale), y),
                            size(px((end - start) as f32 * scale), px(mline * 0.8)),
                        ),
                        color,
                    ));
                }
            }
        };
        for (byte, ch) in full[byte_start..byte_end].char_indices() {
            if column >= max_column {
                break;
            }
            let width = if ch == '\t' {
                tab_size - column % tab_size
            } else {
                1
            };
            if ch.is_whitespace() {
                flush(&mut run, &mut blocks);
            } else {
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
                match run.as_mut() {
                    Some((_, end, run_color)) if *run_color == color && *end == column => {
                        *end += width
                    }
                    _ => {
                        flush(&mut run, &mut blocks);
                        run = Some((column, column + width, color));
                    }
                }
            }
            column += width;
        }
        flush(&mut run, &mut blocks);
    }
    blocks
}
