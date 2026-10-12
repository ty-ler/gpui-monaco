//! The [`EditorElement`]: lays out and paints only the visible view lines.

use std::ops::Range;

use gpui::{
    App, AvailableSpace, BorderStyle, Bounds, ContentMask, Corners, CursorStyle as MouseCursor,
    DispatchPhase, Edges, Element, ElementId, ElementInputHandler, Entity, Font, FontStyle,
    FontWeight, GlobalElementId, Hitbox, HitboxBehavior, Hsla, InspectorElementId, IntoElement,
    LayoutId, MouseDownEvent, MouseMoveEvent, ParentElement as _, PathBuilder, Pixels, ShapedLine,
    SharedString, Style, Styled as _, TextAlign, TextRun, UnderlineStyle, Window, fill, font,
    point, px, quad, relative, size,
};
use text::{PointUtf16, Rope};

use crate::cursor::Selection;
use crate::decorations::{DecorationId, InjectedText};
use crate::editor::CodeEditor;
use crate::options::{
    CursorStyle, EditorOptions, LineNumbers, RenderLineHighlight, ShowFoldingControls,
};
use crate::theme::{EditorColors, Theme, TokenStyle};
use crate::view::view_lines::{ViewLine, ViewLines, WrapSettings, utf16_to_byte};
use crate::widgets::{ContentWidgetPreference, OverlayPosition};

/// Minimum scrollbar slider length.
const MIN_SLIDER: f32 = 20.;
/// Monaco's `.scroll-decoration` height and shadow size.
const SCROLL_SHADOW_HEIGHT: f32 = 6.;
/// Monaco `.mtkcontrol` (`viewLines.css`): `rgb(255, 255, 255)` text.
const CONTROL_CHARACTER_FOREGROUND: Hsla = Hsla {
    h: 0.,
    s: 0.,
    l: 1.,
    a: 1.,
};
/// Monaco `.mtkcontrol` background, `rgb(150, 0, 0)`.
const CONTROL_CHARACTER_BACKGROUND: Hsla = Hsla {
    h: 0.,
    s: 1.,
    l: 150. / 255. / 2.,
    a: 1.,
};

/// One shaped view line: a (possibly wrapped) segment of a model row.
#[derive(Clone)]
pub(crate) struct LineLayout {
    pub view_line: u32,
    pub row: u32,
    /// UTF-16 columns of the model row on this view line.
    pub columns: Range<u32>,
    pub is_last_segment: bool,
    /// Columns past `columns.end` that aren't drawn (Monaco
    /// `stopRenderingLineAfter`); they map to its end.
    pub overflow: u32,
    /// X of the first column (continuation indent).
    pub indent_x: Pixels,
    pub shaped: ChunkedLine,
    /// Byte index into the shaped text for each column in `columns`,
    /// inclusive of the end.
    pub display_index: Vec<usize>,
    /// Display byte ranges of control characters, drawn in Monaco's
    /// `mtkcontrol` style.
    pub controls: Vec<Range<usize>>,
    /// Injected (decoration before/after) text.
    pub injected: Vec<InjectedLayout>,
}

/// Injected text on a laid out line.
#[derive(Clone)]
pub(crate) struct InjectedLayout {
    /// The model column it's injected at.
    pub column: u32,
    /// Its text.
    pub content: SharedString,
    /// Its host data (Monaco `attachedData`).
    pub attached_data: Option<crate::decorations::AttachedData>,
    /// Display byte range.
    pub range: Range<usize>,
    pub background: Option<Hsla>,
    pub swatch: Option<Hsla>,
    /// Its box (an inlay hint's spacers, padding and corners).
    pub text_box: Option<crate::decorations::InjectedTextBox>,
    /// The decoration it comes from, if any (Monaco's mouse target
    /// `injectedText`).
    pub decoration: Option<DecorationId>,
    /// Monaco's `mtkoverflow` "Show more" chip, styled as a button.
    pub chip: bool,
}

impl InjectedLayout {
    /// The x range of its background on `shaped` (its box less the
    /// margins), relative to the line's text start.
    pub fn background_range(&self, shaped: &ChunkedLine) -> Range<Pixels> {
        let (left, right) = self.text_box.map_or((px(0.), px(0.)), |b| b.margin);
        shaped.x_for_index(self.range.start) + left..shaped.x_for_index(self.range.end) - right
    }
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

    /// The injected text at `column`, in display order.
    fn injected_at(&self, column: u32) -> impl Iterator<Item = &InjectedLayout> {
        self.injected
            .iter()
            .filter(move |injected| injected.column == column && !injected.chip)
    }

    /// The caret's x at `column`, `injected` UTF-16 units into the
    /// injected text there (Monaco's view column: 0 is before it all).
    pub fn x_for_caret(&self, column: u32, injected: u32) -> Pixels {
        let mut group = self.injected_at(column).peekable();
        let Some(first) = group.peek() else {
            return self.x_for_column(column);
        };
        let mut index = first.range.start;
        let mut left = injected as usize;
        for text in group {
            let len = text.content.encode_utf16().count();
            if left <= len {
                let byte = utf16_to_byte(&text.content, left as u32);
                index = if left == len {
                    text.range.end
                } else {
                    text.range.start + byte
                };
                break;
            }
            left -= len;
            index = text.range.end;
        }
        self.indent_x + self.shaped.x_for_index(index)
    }

    /// The column and UTF-16 offset into its injected text closest to `x`
    /// (Monaco's view position under the mouse, before normalizing); the
    /// offset is `None` off injected text.
    pub fn position_for_x(&self, x: Pixels) -> (u32, Option<u32>) {
        if self.injected.is_empty() {
            return (self.column_for_x(x), None);
        }
        let ix = self.shaped.closest_index_for_x(x - self.indent_x);
        let hit = self
            .injected
            .iter()
            .find(|text| !text.chip && text.range.start <= ix && ix <= text.range.end);
        let Some(hit) = hit else {
            return (self.column_for_x(x), None);
        };
        let mut offset = 0;
        for text in self.injected_at(hit.column) {
            if std::ptr::eq(text, hit) {
                let mut end = (ix - text.range.start).min(text.content.len());
                while !text.content.is_char_boundary(end) {
                    end -= 1;
                }
                offset += text.content[..end].encode_utf16().count() as u32;
                break;
            }
            offset += text.content.encode_utf16().count() as u32;
        }
        (hit.column, Some(offset))
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
                || (point.column <= self.columns.end + self.overflow && self.is_last_segment))
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
    /// The widest digit's width (Monaco `maxDigitWidth`), for the line
    /// numbers' width.
    pub max_digit_width: Pixels,
    /// Show control characters as their Unicode control pictures.
    pub render_control_characters: bool,
    /// Monaco `effectiveFullwidthCharacterWidth: 'twoCells'`: the cell
    /// full-width characters are centered in (two spaces wide), with a
    /// monospace font.
    pub full_width_cell: Option<Pixels>,
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
        // Monaco `FontInfo.maxDigitWidth`: the widest of the digits.
        let digits = window.text_system().shape_line(
            "0123456789".into(),
            font_size,
            &[run(10, &font, gpui::black())],
            None,
        );
        let max_digit_width = (0..10)
            .map(|i| digits.x_for_index(i + 1) - digits.x_for_index(i))
            .fold(px(0.), Pixels::max);
        // Monaco `FontInfo.isMonospace`: the typical characters are equally
        // wide.
        let two_cells = options.fullwidth_character_width
            == crate::options::FullwidthCharacterWidth::TwoCells
            && ["i", "W", "1", ".", "_"]
                .into_iter()
                .all(|text| (width(text) - em_width).abs() < px(0.01))
            && (space_width - em_width).abs() < px(0.01);
        Self {
            font,
            font_size,
            line_height: options.resolved_line_height(),
            em_width,
            space_width,
            max_digit_width,
            render_control_characters: options.render_control_characters,
            full_width_cell: two_cells.then_some(space_width * 2.),
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
    /// Full-width characters' columns with `fullwidthCharacterWidth:
    /// 'twoCells'`.
    full_width_columns: Option<f32>,
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
            full_width_columns: metrics
                .full_width_cell
                .map(|cell| cell / metrics.em_width.max(px(1.))),
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
                    && widths.column_width == metrics.em_width.max(px(1.))
                    && widths.full_width_columns.is_some() == metrics.full_width_cell.is_some() =>
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
        if let Some(columns) = self.full_width_columns
            && crate::cursor::strings::is_full_width_character(ch as u32)
        {
            return columns;
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

/// Monaco `isControlCharacter` (`viewLineRenderer.ts`): C0 controls other
/// than tab, DEL, and the Unicode directional formatting characters (LRE,
/// RLE, PDF, LRO, RLO, LRI, RLI, FSI, PDI, LRM, RLM and ALM).
pub(crate) fn is_control_character(ch: char) -> bool {
    matches!(
        ch,
        '\u{0}'..='\u{8}'
            | '\u{a}'..='\u{1f}'
            | '\u{7f}'
            | '\u{202a}'..='\u{202e}'
            | '\u{2066}'..='\u{2069}'
            | '\u{200e}'..='\u{200f}'
            | '\u{61c}'
    )
}

/// What Monaco's `_renderLine` draws instead of a character.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Substitute {
    /// A C0 control or DEL as its Unicode control picture (`␇`), one
    /// column wide, in `mtkcontrol` style.
    Picture(char),
    /// A directional formatting character as the text `[U+202E]`, eight
    /// columns wide, in `mtkcontrol` style, so it can't reorder the line.
    Hex(u32),
    /// The BOM, line and paragraph separators and NEL as U+FFFD (whatever
    /// `renderControlCharacters` says).
    Replacement,
}

impl Substitute {
    /// Columns it takes (Monaco's `producedCharacters`).
    pub(crate) fn columns(self) -> usize {
        match self {
            Substitute::Hex(_) => 8,
            Substitute::Picture(_) | Substitute::Replacement => 1,
        }
    }
}

/// How Monaco renders `ch` (`viewLineRenderer.ts` `_renderLine`), if not as
/// itself: with `render_control_characters`, control characters (see
/// [`is_control_character`]) show as control pictures or `[U+XXXX]`.
pub(crate) fn substitute(ch: char, render_control_characters: bool) -> Option<Substitute> {
    match ch {
        '\u{feff}' | '\u{2028}' | '\u{2029}' | '\u{85}' => Some(Substitute::Replacement),
        _ if !render_control_characters || !is_control_character(ch) => None,
        '\u{0}'..='\u{1f}' => char::from_u32(0x2400 + ch as u32).map(Substitute::Picture),
        '\u{7f}' => Some(Substitute::Picture('\u{2421}')),
        _ => Some(Substitute::Hex(ch as u32)),
    }
}

/// Long lines are shaped in chunks this big (bytes, ending at a
/// delimiter): an edit reshapes its chunk, and GPUI's layout cache keeps
/// the others.
const SHAPE_CHUNK: usize = 512;

/// A line's text shaped in chunks laid end to end (one chunk for most
/// lines), with [`ShapedLine`]'s API in display byte offsets.
#[derive(Clone)]
pub(crate) struct ChunkedLine {
    /// (display byte offset, x, shaped text) of each chunk.
    chunks: Vec<(usize, Pixels, ShapedLine)>,
    /// Room left before each chunk's text (an injected text box's margin
    /// and padding); empty when no chunk has any.
    leads: Vec<Pixels>,
    /// The total width.
    pub width: Pixels,
    /// Full-width characters fitted to two cells (Monaco `twoCells`): their
    /// display byte range and the cell's left edge, in order.
    cells: Vec<(Range<usize>, Pixels)>,
    /// The cells' width.
    cell_width: Pixels,
}

/// Injected text shaped on its own: in its own font size (centered on the
/// line when painted), or with room before and after it.
struct ShapeBox {
    range: Range<usize>,
    size: Option<Pixels>,
    lead: Pixels,
    trail: Pixels,
}

impl ChunkedLine {
    /// Shapes `display` in `font_size`, except the `boxes`, each shaped on
    /// its own in its size, with its room before and after. A box's
    /// [`Self::x_for_index`] range includes that room.
    fn shape(
        display: String,
        font_size: Pixels,
        full_width_cell: Option<Pixels>,
        boxes: &[ShapeBox],
        runs: &[TextRun],
        window: &mut Window,
    ) -> Self {
        let mut bounds = vec![0];
        if display.len() > SHAPE_CHUNK * 2 {
            // Content-defined: the first delimiter after `SHAPE_CHUNK`
            // bytes, so chunks after an edit come back unchanged.
            let bytes = display.as_bytes();
            let mut at = SHAPE_CHUNK;
            while at < display.len() {
                let end = bytes[at..]
                    .iter()
                    .position(|b| matches!(b, b' ' | b';' | b',' | b'}' | b')' | b'>'))
                    .map_or(display.len(), |i| at + i + 1);
                if end >= display.len() {
                    break;
                }
                bounds.push(end);
                at = end + SHAPE_CHUNK;
            }
        }
        bounds.push(display.len());
        if !boxes.is_empty() {
            for shape_box in boxes {
                bounds.extend([shape_box.range.start, shape_box.range.end]);
            }
            bounds.sort_unstable();
            bounds.dedup();
        }
        let mut chunks = Vec::with_capacity(bounds.len() - 1);
        let mut leads = Vec::new();
        let mut cells = Vec::new();
        let mut x = px(0.);
        let mut run_ix = 0;
        let mut run_offset = 0;
        for pair in bounds.windows(2) {
            let (start, end) = (pair[0], pair[1]);
            // The runs covering `start..end`.
            let mut chunk_runs = Vec::new();
            let mut pos = start;
            while pos < end && run_ix < runs.len() {
                let run = &runs[run_ix];
                let run_end = run_offset + run.len;
                let take = run_end.min(end) - pos;
                chunk_runs.push(TextRun {
                    len: take,
                    ..run.clone()
                });
                pos += take;
                if run_end <= end {
                    run_offset = run_end;
                    run_ix += 1;
                }
            }
            let shape_box = boxes
                .iter()
                .find(|b| b.range.start <= start && end <= b.range.end);
            let size = shape_box.and_then(|b| b.size).unwrap_or(font_size);
            let (lead, trail) = shape_box.map_or((px(0.), px(0.)), |b| {
                (
                    if b.range.start == start {
                        b.lead
                    } else {
                        px(0.)
                    },
                    if b.range.end == end { b.trail } else { px(0.) },
                )
            });
            if lead > px(0.) && leads.is_empty() {
                leads = vec![px(0.); chunks.len()];
            }
            if !leads.is_empty() {
                leads.push(lead);
            }
            x += lead;
            let mut shaped = window.text_system().shape_line(
                SharedString::from(display[start..end].to_string()),
                size,
                &chunk_runs,
                None,
            );
            if let Some(cell) = full_width_cell.filter(|_| size == font_size) {
                for (range, left) in fit_full_width_to_cells(&mut shaped, cell) {
                    cells.push((start + range.start..start + range.end, x + left));
                }
            }
            let width = shaped.width;
            chunks.push((start, x, shaped));
            x += width + trail;
        }
        Self {
            chunks,
            leads,
            width: x,
            cells,
            cell_width: full_width_cell.unwrap_or_default(),
        }
    }

    /// The two-cell box of the full-width character at display byte
    /// `index`, if fitted.
    fn cell_at(&self, index: usize) -> Option<&(Range<usize>, Pixels)> {
        let ix = self.cells.partition_point(|(range, _)| range.start < index);
        self.cells.get(ix).filter(|(range, _)| range.start == index)
    }

    /// The chunk holding display byte `index`.
    fn chunk(&self, index: usize) -> usize {
        self.chunks
            .partition_point(|(start, _, _)| *start <= index)
            .saturating_sub(1)
    }

    pub fn x_for_index(&self, index: usize) -> Pixels {
        // Monaco `getColumnLeft` with `useTwoCellFullwidthCharacters`: a
        // centered full-width character starts at its cell's left edge.
        if let Some((_, left)) = self.cell_at(index) {
            return *left;
        }
        let ix = self.chunk(index);
        let (start, x, shaped) = &self.chunks[ix];
        // A box's range takes in the room before and after its text.
        if index == *start
            && let Some(lead) = self.leads.get(ix)
        {
            return *x - *lead;
        }
        if ix + 1 == self.chunks.len() && index - start >= shaped.len {
            return self.width;
        }
        *x + shaped.x_for_index(index - start)
    }

    /// The height of the text at display byte `index` (its font's ascent
    /// and descent), which painting centers on the line.
    pub fn text_height(&self, index: usize) -> Pixels {
        let (_, _, shaped) = &self.chunks[self.chunk(index)];
        shaped.ascent + shaped.descent
    }

    pub fn closest_index_for_x(&self, x: Pixels) -> usize {
        // In a two-cell box, the nearer edge.
        let cell = self
            .cells
            .partition_point(|(_, left)| *left <= x)
            .checked_sub(1)
            .and_then(|ix| self.cells.get(ix))
            .filter(|(_, left)| x < *left + self.cell_width);
        if let Some((range, left)) = cell {
            return if x < *left + self.cell_width / 2. {
                range.start
            } else {
                range.end
            };
        }
        let ix = self
            .chunks
            .partition_point(|(_, chunk_x, _)| *chunk_x <= x)
            .saturating_sub(1);
        let (start, chunk_x, shaped) = &self.chunks[ix];
        start + shaped.closest_index_for_x(x - *chunk_x)
    }

    /// Paints just the glyphs of display bytes `range` in `color`, where
    /// [`Self::paint`] would put them (the character inside a block
    /// cursor).
    pub fn paint_glyphs(
        &self,
        range: Range<usize>,
        origin: gpui::Point<Pixels>,
        line_height: Pixels,
        color: Hsla,
        window: &mut Window,
    ) {
        for (start, x, shaped) in &self.chunks {
            if *start >= range.end || *start + shaped.len <= range.start {
                continue;
            }
            // As `ShapedLine::paint` places them.
            let padding_top = (line_height - shaped.ascent - shaped.descent) / 2.;
            let baseline = point(origin.x + *x, origin.y + padding_top + shaped.ascent);
            for run in &shaped.runs {
                for glyph in &run.glyphs {
                    if !range.contains(&(start + glyph.index)) {
                        continue;
                    }
                    let at = baseline + glyph.position;
                    if glyph.is_emoji {
                        window
                            .paint_emoji(at, run.font_id, glyph.id, shaped.font_size)
                            .ok();
                    } else {
                        window
                            .paint_glyph(at, run.font_id, glyph.id, shaped.font_size, color)
                            .ok();
                    }
                }
            }
        }
    }

    pub fn paint(
        &self,
        origin: gpui::Point<Pixels>,
        line_height: Pixels,
        align: TextAlign,
        align_width: Option<Pixels>,
        window: &mut Window,
        cx: &mut App,
    ) -> anyhow::Result<()> {
        for (_, x, shaped) in &self.chunks {
            shaped.paint(
                point(origin.x + *x, origin.y),
                line_height,
                align,
                align_width,
                window,
                cx,
            )?;
        }
        Ok(())
    }
}

/// Monaco `useTwoCellFullwidthCharacters` (`splitFullWidthCharacters` and
/// the `mtkfullwidth` parts in `viewLineRenderer.ts`): moves each
/// full-width character's glyph to the middle of a `cell`-wide box and the
/// glyphs after it along. Returns the boxes as (byte range, left edge).
/// Lines with right-to-left glyphs, and characters drawn with several
/// glyphs, are left alone, as Monaco leaves RTL parts and multi code point
/// characters.
fn fit_full_width_to_cells(shaped: &mut ShapedLine, cell: Pixels) -> Vec<(Range<usize>, Pixels)> {
    use crate::cursor::strings::is_full_width_character;
    let text = shaped.text.clone();
    if !text.chars().any(|ch| is_full_width_character(ch as u32)) {
        return Vec::new();
    }
    // The glyphs in visual order: (run, glyph, x, byte index).
    let mut glyphs: Vec<(usize, usize, Pixels, usize)> = shaped
        .runs
        .iter()
        .enumerate()
        .flat_map(|(r, run)| {
            run.glyphs
                .iter()
                .enumerate()
                .map(move |(g, glyph)| (r, g, glyph.position.x, glyph.index))
        })
        .collect();
    glyphs.sort_by(|a, b| f32::from(a.2).total_cmp(&f32::from(b.2)));
    if glyphs.windows(2).any(|w| w[1].3 < w[0].3) {
        return Vec::new();
    }
    let mut runs = shaped.runs.clone();
    let mut cells = Vec::new();
    let mut shift = px(0.);
    for (k, &(r, g, x, index)) in glyphs.iter().enumerate() {
        let next = glyphs.get(k + 1);
        let next_x = next.map_or(shaped.width, |n| n.2);
        let next_index = next.map_or(text.len(), |n| n.3);
        let ch = text[index..].chars().next();
        let fits = ch.filter(|ch| {
            is_full_width_character(*ch as u32) && next_index == index + ch.len_utf8()
        });
        let position = &mut runs[r].glyphs[g].position.x;
        if let Some(ch) = fits {
            let advance = next_x - x;
            let left = x + shift;
            *position = left + (cell - advance) / 2.;
            cells.push((index..index + ch.len_utf8(), left));
            shift += cell - advance;
        } else {
            *position = x + shift;
        }
    }
    if cells.is_empty() {
        return cells;
    }
    let layout = gpui::LineLayout {
        font_size: shaped.font_size,
        width: shaped.width + shift,
        ascent: shaped.ascent,
        descent: shaped.descent,
        runs,
        len: shaped.len,
    };
    // `ShapedLine` derefs to its `Arc<LineLayout>`.
    **shaped = std::sync::Arc::new(layout);
    cells
}

/// Monaco `renderOverflowingCharCount`.
fn overflow_size(units: u32) -> String {
    let n = units as f64;
    if units < 1024 {
        format!("{units} chars")
    } else if units < 1024 * 1024 {
        format!("{:.1} KB", n / 1024.)
    } else {
        format!("{:.1} MB", n / 1024. / 1024.)
    }
}

/// What to shape for one view line.
#[derive(Clone)]
pub(crate) struct RowSegment {
    pub view_line: u32,
    pub row: u32,
    pub columns: Range<u32>,
    pub start_visible_column: u32,
    pub indent: u32,
    pub is_last_segment: bool,
    /// UTF-16 units past Monaco's `stopRenderingLineAfter`, not shown.
    pub overflow: u32,
}

/// Injected text being laid out: display range and its injection.
type Injected<'a> = (Range<usize>, &'a Injection);

/// Text to inject on a row: UTF-16 column, whether it's `after` a range
/// (else `before` one), the text, and its decoration.
pub(crate) type Injection = (u32, bool, InjectedText, Option<DecorationId>);

/// Shapes one view line. `highlights` are styled spans in row-relative byte
/// offsets, sorted and non-overlapping. `marked` is an IME composition range
/// in UTF-16 columns of the row, drawn underlined 1px, with the IME's
/// selected part (its active clause) 2px (Monaco
/// `edit-context-composition-secondary` / `-primary`).
#[allow(clippy::too_many_arguments)]
pub(crate) fn layout_segment<'a>(
    rope: &Rope,
    segment: &RowSegment,
    tab_size: u32,
    metrics: &TextMetrics,
    color: Hsla,
    highlights: &[(Range<usize>, TokenStyle)],
    fonts: &[(Range<usize>, DecorationFont)],
    injections: &'a [Injection],
    marked: Option<(Range<u32>, Option<Range<u32>>, Hsla)>,
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
    // (display range, injection) of injected text.
    let mut injected: Vec<Injected> = Vec::new();
    // Display ranges of control characters (Monaco `mtkcontrol`).
    let mut controls: Vec<Range<usize>> = Vec::new();
    let mut injection_ix = 0;
    // Emits injections at `column` (`after` ones first) and returns where the
    // column itself displays: before `after` text (it ends a decorated range)
    // or after `before` text (it starts one).
    let mut inject_at =
        |column: u32, display: &mut String, injected: &mut Vec<Injected<'a>>| -> usize {
            let first = injection_ix;
            while injection_ix < injections.len() && injections[injection_ix].0 <= column {
                injection_ix += 1;
            }
            let position_before = display.len();
            let mut has_after = false;
            for is_after in [true, false] {
                for injection in &injections[first..injection_ix] {
                    let (at, after, text, _) = injection;
                    if *at == column && *after == is_after {
                        has_after |= *after;
                        let start = display.len();
                        display.push_str(&text.content);
                        injected.push((start..display.len(), injection));
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
        } else if let Some(substitute) = substitute(ch, metrics.render_control_characters) {
            // Monaco `renderControlCharacters`: `\u{7}` shows as `␇` and
            // U+202E as `[U+202E]`, in `mtkcontrol` style.
            let start = display.len();
            match substitute {
                Substitute::Picture(picture) => display.push(picture),
                Substitute::Hex(code) => {
                    use std::fmt::Write as _;
                    write!(display, "[U+{code:04X}]").ok();
                }
                Substitute::Replacement => display.push('\u{fffd}'),
            }
            if substitute != Substitute::Replacement {
                controls.push(start..display.len());
            }
            visible_column += substitute.columns();
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

    let marked_display = marked.as_ref().and_then(|(range, _, underline)| {
        let start = column_to_display(range.start);
        let end = column_to_display(range.end);
        (end > start).then_some((start..end, *underline))
    });
    let active_display = marked.as_ref().and_then(|(_, active, _)| {
        let active = active.as_ref()?;
        let start = column_to_display(active.start);
        let end = column_to_display(active.end);
        (end > start).then_some(start..end)
    });
    let styled: Vec<(Range<usize>, TokenStyle)> = highlights
        .iter()
        .map(|(range, style)| (to_display(range.start)..to_display(range.end), *style))
        .filter(|(range, _)| !range.is_empty())
        .collect();
    let fonts: Vec<(Range<usize>, &DecorationFont)> = fonts
        .iter()
        .map(|(range, font)| (to_display(range.start)..to_display(range.end), font))
        .filter(|(range, _)| !range.is_empty())
        .collect();
    let mut bounds = vec![0, display_len];
    for (range, _) in &styled {
        bounds.extend([range.start, range.end]);
    }
    for (range, _) in &fonts {
        bounds.extend([range.start, range.end]);
    }
    for (range, (_, _, text, _)) in &injected {
        bounds.extend([range.start, range.end]);
        // Monaco's injected text `tokens`.
        for (token, _) in text.tokens.iter().flat_map(|tokens| tokens.iter()) {
            bounds.extend([
                (range.start + token.start).min(range.end),
                (range.start + token.end).min(range.end),
            ]);
        }
    }
    for range in &controls {
        bounds.extend([range.start, range.end]);
    }
    if let Some((range, _)) = &marked_display {
        bounds.extend([range.start, range.end]);
    }
    if let Some(range) = &active_display {
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
        let injected_at = injected
            .iter()
            .find(|(range, _)| range.start <= start && end <= range.end);
        let injected_text = injected_at.map(|(_, (_, _, text, _))| text);
        let style = injected_at
            .map(|(range, (_, _, text, _))| {
                injected_text_style(text, start - range.start..end - range.start)
            })
            .or_else(|| {
                styled
                    .get(span_ix)
                    .filter(|(range, _)| range.start <= start)
                    .map(|(_, style)| *style)
            })
            .unwrap_or_default();
        let mut font = metrics.font.clone();
        if let Some(family) = injected_text.and_then(|text| text.font_family.clone()) {
            font.family = family;
        }
        // A decoration's font (Monaco `fontFamily`, `fontWeight`,
        // `fontStyle`); token styles' bold and italic still apply.
        if injected_text.is_none()
            && let Some((_, decoration_font)) = fonts
                .iter()
                .rev()
                .find(|(range, _)| range.start <= start && end <= range.end)
        {
            if let Some(family) = &decoration_font.family {
                font.family = family.clone();
            }
            if let Some(weight) = decoration_font.weight {
                font.weight = weight;
            }
            if let Some(style) = decoration_font.style {
                font.style = style;
            }
        }
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
        // Monaco `.mtkcontrol`: white on dark red, replacing the token's
        // class.
        let is_control = injected_text.is_none()
            && controls
                .iter()
                .any(|range| range.start <= start && end <= range.end);
        if is_control {
            font = metrics.font.clone();
            foreground = CONTROL_CHARACTER_FOREGROUND;
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
                thickness: if is_marked
                    && active_display
                        .as_ref()
                        .is_some_and(|range| range.start <= start && end <= range.end)
                {
                    px(2.)
                } else {
                    px(1.)
                },
                wavy: false,
            });
        }
        if let Some(underline) = injected_text.and_then(|text| text.underline) {
            text_run.underline = Some(UnderlineStyle {
                color: Some(underline.color),
                thickness: px(1.),
                wavy: underline.wavy,
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
    // Injected text in its own font size, or with room around it, is
    // shaped on its own.
    let boxes: Vec<ShapeBox> = injected
        .iter()
        .filter_map(|(range, (_, _, text, _))| {
            let size = text.font_size.filter(|size| *size != metrics.font_size);
            let (lead, trail) = text.text_box.map_or((px(0.), px(0.)), |b| {
                (b.margin.0 + b.padding.0, b.padding.1 + b.margin.1)
            });
            (!range.is_empty() && (size.is_some() || lead > px(0.) || trail > px(0.))).then(|| {
                ShapeBox {
                    range: range.clone(),
                    size,
                    lead,
                    trail,
                }
            })
        })
        .collect();
    let shaped = ChunkedLine::shape(
        display,
        metrics.font_size,
        metrics.full_width_cell,
        &boxes,
        &runs,
        window,
    );
    LineLayout {
        view_line: segment.view_line,
        row: segment.row,
        columns: segment.columns.clone(),
        is_last_segment: segment.is_last_segment,
        overflow: segment.overflow,
        indent_x: metrics.space_width * segment.indent as f32,
        shaped,
        display_index,
        controls,
        injected: injected
            .into_iter()
            .map(|(range, injection)| {
                let (column, _, text, decoration) = injection;
                InjectedLayout {
                    column: *column,
                    content: text.content.clone(),
                    attached_data: text.attached_data.clone(),
                    range,
                    background: text.background,
                    swatch: text.swatch,
                    text_box: text.text_box,
                    decoration: *decoration,
                    // The overflow chip is always the last injection.
                    chip: segment.overflow > 0
                        && injections
                            .last()
                            .is_some_and(|last| std::ptr::eq(last, injection)),
                }
            })
            .collect(),
    }
}

/// Monaco's `ghost-text-hidden` (`opacity: 0; font-size: 0`): `segment`
/// shown only up to UTF-16 column `from` of its row, as the end of the row
/// it holds `from`'s injected text; a later segment shows nothing. `None`
/// leaves a segment before `from` as it is.
pub(crate) fn hide_segment_from(segment: &RowSegment, from: u32) -> Option<RowSegment> {
    let holds = segment.columns.start <= from
        && (from < segment.columns.end || (from == segment.columns.end && segment.is_last_segment));
    if holds {
        return Some(RowSegment {
            columns: segment.columns.start..from,
            is_last_segment: true,
            overflow: 0,
            ..segment.clone()
        });
    }
    (segment.columns.start > from).then(|| RowSegment {
        columns: segment.columns.start..segment.columns.start,
        is_last_segment: false,
        overflow: 0,
        ..segment.clone()
    })
}

/// Monaco `_renderNormalDecoration` with `showIfCollapsed`: a range
/// narrower than a typical (half-width) character is widened to that,
/// around its center (rounded, and not left of the line's start).
pub(crate) fn widen_collapsed(from: Pixels, to: Pixels, half_width: Pixels) -> (Pixels, Pixels) {
    if to - from >= half_width {
        return (from, to);
    }
    let center = ((from + to) / 2.).round();
    let from = (center - half_width / 2.).round().max(px(0.));
    (from, from + half_width)
}

/// The style of bytes `range` of injected `text`: the token there (Monaco's
/// injected text `tokens`), else the text's style.
pub(crate) fn injected_text_style(text: &InjectedText, range: Range<usize>) -> TokenStyle {
    text.tokens
        .iter()
        .flat_map(|tokens| tokens.iter())
        .find(|(token, _)| token.start <= range.start && range.end <= token.end)
        .map_or(text.style, |(_, style)| *style)
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
        overflow: 0,
    }
}

/// A decoration's font over part of a row (Monaco's `fontFamily`,
/// `fontWeight` and `fontStyle` decoration options), in row-relative bytes.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct DecorationFont {
    pub family: Option<SharedString>,
    pub weight: Option<FontWeight>,
    pub style: Option<FontStyle>,
}

/// A row's text styles, injected text and fonts from its decorations (see
/// [`decoration_styles_and_injections`]).
pub(crate) type RowDecorations = (
    Vec<(Range<usize>, TokenStyle)>,
    Vec<Injection>,
    Vec<(Range<usize>, DecorationFont)>,
);

/// Row `row`'s inline decorations (Monaco `LineDecoration`s): the bracket
/// pair colors and decoration text styles over the syntax colors, in
/// row-relative bytes, the injected text before and after decorated
/// ranges, by column (see [`row_injections`]), and the decorations' fonts.
/// `line_range` is the row's byte range.
pub(crate) fn decoration_styles_and_injections(
    row: u32,
    line_range: Range<usize>,
    bracket_colors: &[(Range<usize>, Hsla)],
    decoration_points: &[(PointUtf16, PointUtf16, &crate::decorations::Decoration)],
    options: &EditorOptions,
    colors: &EditorColors,
) -> RowDecorations {
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
    let mut fonts = Vec::new();
    let mut injections = Vec::new();
    for (start, end, decoration) in decoration_points {
        if start.row > row || end.row < row {
            continue;
        }
        push_injections(row, *start, *end, decoration, &mut injections);
        let decoration_options = &decoration.options;
        let row_range = || {
            let from = if start.row == row {
                decoration.range.start - line_range.start
            } else {
                0
            };
            let to = if end.row == row {
                decoration.range.end - line_range.start
            } else {
                line_range.len()
            };
            from..to
        };
        // Monaco's `fontFamily`, `fontWeight` and `fontStyle`.
        if decoration_options.has_font() {
            let range = row_range();
            if !range.is_empty() {
                fonts.push((
                    range,
                    DecorationFont {
                        family: decoration_options.font_family.clone(),
                        weight: decoration_options.font_weight,
                        style: decoration_options.font_style,
                    },
                ));
            }
        }
        // Monaco's tag classes, when the editor shows them.
        let tag_style = TokenStyle {
            opacity: (decoration_options.unnecessary && options.show_unused)
                .then_some(colors.unnecessary_code_opacity.a),
            strikethrough: decoration_options.deprecated && options.show_deprecated,
            ..Default::default()
        };
        let style = match (decoration_options.text, tag_style == TokenStyle::default()) {
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
            let range = row_range();
            if !range.is_empty() {
                decoration_styles.push((range, style));
            }
        }
    }
    injections.sort_by_key(|(column, _, _, _)| *column);
    (decoration_styles, injections, fonts)
}

/// The injected text on row `row` (Monaco `LineInjectedText.fromDecorations`),
/// by column: a decoration's `before` text at its start and `after` text at
/// its end, on an empty range only with `show_if_collapsed`. At one column
/// it's drawn `after` texts first, then `before` ones, in this order.
pub(crate) fn row_injections(
    row: u32,
    decoration_points: &[(PointUtf16, PointUtf16, &crate::decorations::Decoration)],
) -> Vec<Injection> {
    let mut injections = Vec::new();
    for (start, end, decoration) in decoration_points {
        if start.row <= row && row <= end.row {
            push_injections(row, *start, *end, decoration, &mut injections);
        }
    }
    injections.sort_by_key(|(column, _, _, _)| *column);
    injections
}

/// Adds `decoration`'s injected text on `row` (see [`row_injections`]),
/// unsorted.
fn push_injections(
    row: u32,
    start: PointUtf16,
    end: PointUtf16,
    decoration: &crate::decorations::Decoration,
    injections: &mut Vec<Injection>,
) {
    let options = &decoration.options;
    if (options.before.is_none() && options.after.is_none())
        || !options.shows_injected_text(&decoration.range)
    {
        return;
    }
    if let Some(before) = &options.before
        && start.row == row
    {
        injections.push((start.column, false, before.clone(), Some(decoration.id)));
    }
    if let Some(after) = &options.after
        && end.row == row
    {
        injections.push((end.column, true, after.clone(), Some(decoration.id)));
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
    /// Monaco `maxTokenizationLineLength`.
    max_line_length: u32,
}

impl HighlightSource {
    pub(crate) fn new(model: &crate::model::TextModel) -> Option<Self> {
        let syntax = model.syntax_snapshot()?;
        let buffer = model.buffer();
        let patch = (syntax.text.version() != buffer.version())
            .then(|| buffer.edits_since(syntax.text.version()));
        Some(Self {
            syntax,
            patch,
            max_line_length: model.options().max_tokenization_line_length,
        })
    }

    /// Styled spans in `range` of the current text, with the parsed
    /// highlights shifted through any newer edits.
    pub(crate) fn highlights(
        &self,
        theme: &Theme,
        range: Range<usize>,
    ) -> Vec<(Range<usize>, TokenStyle)> {
        let syntax = &self.syntax;
        let old_range = match &self.patch {
            None => range,
            Some(patch) => {
                let inverse = patch.invert();
                inverse.transform_offset(range.start, text::Bias::Left)
                    ..inverse.transform_offset(range.end, text::Bias::Right)
            }
        };
        // Monaco's tokens carry their language (`keyword.json`): a span's is
        // the deepest visible injection at its start, else the root's.
        let tree = syntax.state.syntax();
        let root = tree
            .and_then(|tree| tree.root())
            .map(|layer| &*layer.language.id);
        let injections: Vec<_> = tree
            .map(|tree| {
                tree.layers()
                    .iter()
                    .filter(|layer| {
                        layer.depth > 0
                            && !layer.language.hidden
                            && layer
                                .ranges
                                .iter()
                                .any(|r| r.start <= old_range.end && old_range.start <= r.end)
                    })
                    .collect()
            })
            .unwrap_or_default();
        let style = |span: &syntax::HighlightSpan| {
            let offset = span.range.start;
            let language = injections
                .iter()
                .filter(|layer| {
                    layer
                        .ranges
                        .iter()
                        .any(|r| r.start <= offset && offset < r.end)
                })
                .max_by_key(|layer| layer.depth)
                .map(|layer| &*layer.language.id)
                .or(root);
            theme.style_for_language(span.highlight, language)
        };
        let mut result = Vec::new();
        // Monaco `maxTokenizationLineLength`: long lines stay plain, and
        // aren't even queried.
        for old_range in
            crate::model::tokenized_ranges(syntax.text.rope(), old_range, self.max_line_length)
        {
            for span in syntax
                .highlighter
                .highlights(&syntax.state, syntax.text.rope(), old_range)
            {
                let range = match &self.patch {
                    None => span.range.clone(),
                    Some(patch) => {
                        patch.transform_offset(span.range.start, text::Bias::Right)
                            ..patch.transform_offset(span.range.end, text::Bias::Left)
                    }
                };
                if range.start < range.end {
                    result.push((range, style(&span)));
                }
            }
        }
        result
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

/// Where the minimap is and how it maps to content, for the mouse.
#[derive(Clone, Copy, Debug)]
pub(crate) struct MinimapLayout {
    pub bounds: Bounds<Pixels>,
    pub slider: Bounds<Pixels>,
    /// Monaco `sliderNeeded`: the slider shows and takes the mouse.
    pub slider_needed: bool,
    /// Monaco `MinimapLayout`.
    pub layout: crate::view::minimap::MinimapLayout,
    /// Logical pixels per view line.
    pub line_height: Pixels,
    /// Canvas pixels per minimap line (Monaco `minimapLineHeight`).
    pub minimap_line_height: f32,
    /// Logical pixels per canvas pixel, vertically.
    pub canvas_scale_y: f32,
    /// `minimap.size: Proportional`.
    pub proportional: bool,
    /// Minimap lines (sampled lines when sampling).
    pub line_count: u32,
    /// Monaco `minimap.autohide`: its opacity, and whether it takes the
    /// mouse (not while hidden with `scroll`).
    pub opacity: f32,
    pub interactive: bool,
}

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
    /// The row after the scope (Monaco's `endLineNumbers`), which Shift
    /// shows and Shift+click reveals.
    pub end_row: u32,
    /// The row shown (the scope's end with Shift held over it).
    pub row: u32,
    /// Where it shows, clipped to the widget.
    pub bounds: Bounds<Pixels>,
    /// Its folding control.
    pub fold: Option<FoldControl>,
    /// The x its text starts at (it follows the horizontal scroll with
    /// `stickyScroll.scrollWithEditor`).
    pub origin_x: Pixels,
}

/// Something to draw in the glyph margin (Monaco `GlyphRenderRequest`): a
/// decoration's glyph or a glyph margin widget (by index).
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct GlyphRequest {
    pub view_line: u32,
    pub lane_index: usize,
    pub z_index: i32,
    pub glyph: Option<(SharedString, Hsla)>,
    pub widget: Option<usize>,
}

/// Monaco `GlyphMarginWidgets._collectSortedGlyphRenderRequests` and
/// `prepareRender`: per view line and lane, the request with the highest
/// z-index wins, a widget before a decoration, then the first glyph.
pub(crate) fn resolve_glyph_requests(mut requests: Vec<GlyphRequest>) -> Vec<GlyphRequest> {
    requests.sort_by(|a, b| {
        (a.view_line, a.lane_index)
            .cmp(&(b.view_line, b.lane_index))
            .then(b.z_index.cmp(&a.z_index))
            .then(b.widget.is_some().cmp(&a.widget.is_some()))
            .then_with(|| {
                let text = |r: &GlyphRequest| r.glyph.as_ref().map(|(text, _)| text.clone());
                text(a).cmp(&text(b))
            })
    });
    requests.dedup_by(|b, a| a.view_line == b.view_line && a.lane_index == b.lane_index);
    requests
}

/// Whether view line `line` shows part of `start..end` (inclusive): Monaco
/// converts a decoration's range to view lines, so a glyph shows on every
/// line of its range, wrapped segments included, but not on a wrapped
/// line's later segments for a range that ends before them.
pub(crate) fn segment_intersects(line: &LineLayout, start: PointUtf16, end: PointUtf16) -> bool {
    if line.row < start.row || line.row > end.row {
        return false;
    }
    if line.row == end.row && line.columns.start > 0 && line.columns.start >= end.column {
        return false;
    }
    !(line.row == start.row && line.columns.end < start.column)
}

/// One box of the current line highlight (Monaco
/// `CurrentLineHighlightOverlay` in the text area and
/// `CurrentLineMarginHighlightOverlay` in the gutter).
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct LineHighlight {
    /// The view line.
    pub view_line: u32,
    /// The gutter's box, not the text area's.
    pub margin: bool,
    pub bounds: Bounds<Pixels>,
    pub background: Option<Hsla>,
    /// The border's color and widths (on the cursor's own view line).
    pub border: Option<(Hsla, Edges<Pixels>)>,
}

/// Vertical positions with view zones: zone `i` sits after view line
/// `after` (-1 for above the first line).
#[derive(Clone, Debug, Default, PartialEq)]
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

    /// Total height of the zones right after view line `after` (-1: above
    /// the first line).
    pub fn heights_after(&self, after: i64) -> Pixels {
        let start = self.zones.partition_point(|(a, _)| *a < after);
        let end = self.zones.partition_point(|(a, _)| *a <= after);
        self.heights_before[end] - self.heights_before[start]
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
    /// The lines' and view zones' height, without scrolling beyond the
    /// last line (Monaco `getContentHeight`).
    pub content_height: Pixels,
    /// The content's width, the vertical scrollbar included (Monaco
    /// `getContentWidth`).
    pub content_width: Pixels,
    /// Monaco `EditorLayoutInfo`.
    pub layout_info: crate::layout_info::LayoutInfo,
    /// The top scroll shadow's box (Monaco `.scroll-decoration`), when
    /// scrolled with `scrollbar.useShadows`.
    pub scroll_decoration: Option<Bounds<Pixels>>,
    pub lines: Vec<LineLayout>,
    pub fold_controls: Vec<FoldControl>,
    /// The `⋯` after each collapsed line, with its row, for clicks.
    pub fold_placeholders: Vec<(u32, Bounds<Pixels>)>,
    pub zones: ZoneLayout,
    /// The visible bounds of embedded views; the editor ignores clicks
    /// inside them.
    pub widget_bounds: Vec<Bounds<Pixels>>,
    /// What each of `widget_bounds` is.
    pub widget_kinds: Vec<crate::mouse_target::EmbeddedKind>,
    /// The view of each of `widget_bounds`, to tell which widget it is.
    pub widget_views: Vec<gpui::EntityId>,
    /// Whether the wheel over each of `widget_bounds` leaves the editor
    /// alone (overlay and overflowing widgets, zone widgets).
    pub widget_captures_wheel: Vec<bool>,
    /// The code action lightbulb, for clicks.
    pub lightbulb: Option<Bounds<Pixels>>,
    pub minimap: Option<MinimapLayout>,
    /// What the minimap draws.
    pub minimap_paint: Option<std::sync::Arc<crate::minimap::MinimapPaint>>,
    /// Sticky scroll lines, topmost first (earlier lines draw over the
    /// pushed last one).
    pub sticky_lines: Vec<StickyLine>,
    /// The sticky lines' shaped text, for clicks.
    pub sticky_layouts: Vec<LineLayout>,
    pub vertical_scrollbar: Option<ScrollbarLayout>,
    pub horizontal_scrollbar: Option<ScrollbarLayout>,
    /// The line number labels drawn, by row (for tests).
    pub line_number_labels: Vec<(u32, String)>,
    /// The cursors drawn, and whether they're outlined (for tests).
    pub cursors: (Vec<Bounds<Pixels>>, bool),
    /// The rows and left edges of the `wordWrapIndicator` glyphs drawn (for
    /// tests).
    pub wrap_indicators: Vec<(u32, Pixels)>,
    /// The current line highlight's boxes.
    pub line_highlights: Vec<LineHighlight>,
    /// The line numbers' rows, left edges and colors (for tests).
    pub line_number_styles: Vec<(u32, Pixels, Hsla)>,
    /// The glyph margin's glyphs and widgets drawn, as (view line, lane
    /// index) (for tests).
    pub glyphs_drawn: Vec<(u32, usize)>,
    /// The line decorations area's bars drawn (for tests).
    pub gutter_bars: Vec<(Bounds<Pixels>, Hsla)>,
}

impl EditorLayout {
    /// The sticky line at `position`: the topmost, as the pushed last line
    /// slides under the others.
    pub fn sticky_line_at(&self, position: gpui::Point<Pixels>) -> Option<usize> {
        self.sticky_lines
            .iter()
            .position(|line| line.bounds.contains(&position))
    }

    /// The text position on sticky line `ix` under `position`, if it's over
    /// the line's text area (Monaco `getEditorPositionFromNode`).
    pub fn sticky_position(&self, ix: usize, position: gpui::Point<Pixels>) -> Option<PointUtf16> {
        let line = self.sticky_lines.get(ix)?;
        let text = self.sticky_layouts.get(ix)?;
        if position.x < self.text_bounds.left() || !line.bounds.contains(&position) {
            return None;
        }
        Some(PointUtf16::new(
            line.row,
            text.column_for_x(position.x - line.origin_x),
        ))
    }

    /// Visible view lines that fit fully, for page up/down.
    pub fn page_rows(&self) -> u32 {
        ((self.text_bounds.size.height / self.metrics.line_height).floor() as u32).max(1)
    }

    /// The "Show more" chips (Monaco `.mtkoverflow`) in window coordinates.
    pub fn overflow_chips(&self) -> impl Iterator<Item = Bounds<Pixels>> + '_ {
        self.lines.iter().flat_map(move |line| {
            let top = self.text_top(line.view_line);
            let origin_x = self.text_origin_x() + line.indent_x;
            line.injected
                .iter()
                .filter(|i| i.chip)
                .map(move |injected| {
                    let from = line.shaped.x_for_index(injected.range.start);
                    let to = line.shaped.x_for_index(injected.range.end);
                    Bounds::new(
                        point(origin_x + from, top),
                        size(to - from, self.metrics.line_height),
                    )
                })
        })
    }

    /// The laid-out view line showing `point`, if visible.
    pub fn line_for_point(&self, point: PointUtf16) -> Option<&LineLayout> {
        self.lines.iter().find(|line| line.contains(point))
    }

    /// [`Self::line_for_point`], where a left `affinity` puts a point at a
    /// wrap boundary at the end of the previous view line (Monaco
    /// `convertModelPositionToViewPosition` with `PositionAffinity.Left`).
    pub fn line_for_point_with_affinity(
        &self,
        point: PointUtf16,
        affinity: crate::widgets::PositionAffinity,
    ) -> Option<&LineLayout> {
        if affinity.prefers_previous()
            && let Some(line) = self.lines.iter().find(|line| {
                line.row == point.row
                    && !line.is_last_segment
                    && line.columns.end == point.column
                    && line.columns.start < point.column
            })
        {
            return Some(line);
        }
        self.line_for_point(point)
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

/// Where each rendered whitespace mark's character starts (on its line's
/// text top), and whether it's a tab.
type WhitespaceMarks = Vec<(gpui::Point<Pixels>, bool)>;

/// One cursor to paint (Monaco `ViewCursorRenderData`).
struct CursorPaint {
    /// Its box, after `cursorSmoothCaretAnimation`.
    bounds: Bounds<Pixels>,
    /// `editorCursor.foreground` (or the multi-cursor color).
    color: Hsla,
    /// `editorCursor.background`: the character inside a block cursor,
    /// and the cursor's sides over a selection in high contrast themes.
    text_color: Hsla,
    /// The character inside it (Monaco's `textContent`): its view line,
    /// display byte range, and how far the box is from where the
    /// character is (while sliding).
    glyph: Option<(u32, Range<usize>, gpui::Point<Pixels>)>,
}

/// Monaco `Color.opposite`: each RGB channel inverted.
fn opposite(color: Hsla) -> Hsla {
    let rgba = gpui::Rgba::from(color);
    gpui::Rgba {
        r: 1. - rgba.r,
        g: 1. - rgba.g,
        b: 1. - rgba.b,
        a: rgba.a,
    }
    .into()
}

pub struct PrepaintState {
    layout: EditorLayout,
    selections: Vec<Selection>,
    cursors: Vec<CursorPaint>,
    /// Monaco `block-outline`: the cursors are outlined, not filled.
    cursor_outline: bool,
    /// The line numbers: top, text, left.
    gutter_numbers: Vec<(Pixels, ShapedLine, Pixels)>,
    /// (x after the text, top) of rows whose region is collapsed.
    fold_placeholders: Vec<(Pixels, Pixels)>,
    fold_highlights: Vec<Bounds<Pixels>>,
    /// The expanded regions' folding controls' opacity (collapsed ones always
    /// show).
    fold_controls_opacity: f32,
    decoration_quads: Vec<(Bounds<Pixels>, Option<Hsla>, Option<Hsla>)>,
    /// Block decorations' boxes, colors and border widths.
    block_borders: Vec<(Bounds<Pixels>, Hsla, Pixels)>,
    decoration_underlines: Vec<(gpui::Point<Pixels>, Pixels, UnderlineStyle)>,
    /// The IME composition's boxes in high contrast themes.
    composition_boxes: Vec<Bounds<Pixels>>,
    gutter_bars: Vec<(Bounds<Pixels>, Hsla)>,
    /// Monaco `marginClassName` backgrounds.
    margin_backgrounds: Vec<(Bounds<Pixels>, Hsla)>,
    glyphs: Vec<(gpui::Point<Pixels>, ShapedLine)>,
    lightbulb: Option<(Bounds<Pixels>, Hsla, crate::codicons::Codicon)>,
    /// The lightbulb's title (none in the gutter).
    lightbulb_title: Option<SharedString>,
    /// The lightbulb's hover target: the pointer and its title.
    lightbulb_tooltip: Option<gpui::AnyElement>,
    /// Indent and bracket pair guides.
    guides: Vec<(Bounds<Pixels>, Hsla)>,
    /// Monaco `rulers`.
    rulers: Vec<(Bounds<Pixels>, Hsla)>,
    /// Rendered whitespace.
    whitespace: Option<WhitespaceMarks>,
    /// The `wordWrapIndicator` glyph and where it goes.
    wrap_indicators: Option<(ShapedLine, Vec<gpui::Point<Pixels>>)>,
    embedded: Vec<Embedded>,
    /// The placeholder's lines and where they go.
    placeholder: Vec<(ShapedLine, gpui::Point<Pixels>)>,
    /// Where a dragged selection would drop.
    dnd_target: Option<Bounds<Pixels>>,
    /// Replaced minimap images to free.
    minimap_stale: Vec<std::sync::Arc<gpui::RenderImage>>,
    sticky: Vec<(usize, Pixels, LineLayout, Option<ShapedLine>)>,
    /// The sticky scroll widget's height (Monaco `_setHeight`).
    sticky_height: Pixels,
    /// The keyboard-focused and hovered sticky lines.
    sticky_focused: Option<usize>,
    sticky_hovered: Option<usize>,
    /// Every sticky folding control shows, not just the collapsed ones.
    sticky_show_fold_controls: bool,
    numbers_right: Pixels,
    zone_elements: Vec<gpui::AnyElement>,
    full_width_zone_elements: Vec<gpui::AnyElement>,
    /// View zones' margin parts, painted in the gutter.
    zone_margin_elements: Vec<gpui::AnyElement>,
    /// Content widgets, painted in the text area under the sticky scroll.
    content_elements: Vec<gpui::AnyElement>,
    /// Glyph margin widgets, painted in the gutter under the sticky scroll.
    glyph_elements: Vec<gpui::AnyElement>,
    /// The hovered folding control's tooltip.
    fold_tooltip: Option<gpui::AnyElement>,
    /// Overlay widgets, painted last, clipped to the editor.
    overlay_elements: Vec<gpui::AnyElement>,
    /// The editor's own hitbox (see `EditorElement::prepaint`).
    hitbox: Option<Hitbox>,
    /// Pointers over parts of the editor other than the text.
    cursor_areas: Vec<(MouseCursor, Hitbox)>,
    /// Whether to set the gutter's, scrollbars', minimap's and folding
    /// controls' `cursor_areas`: not while dragging the selection to move
    /// or copy it, which shows `mouseStyle` over the whole editor.
    cursor_areas_enabled: bool,
    /// The pointer for the whole window during a drag that captured it
    /// ([`CodeEditor::drag_mouse_cursor`]).
    drag_cursor: Option<MouseCursor>,
    /// Where the overflowing widgets are, in window coordinates.
    overflow_bounds: Vec<Bounds<Pixels>>,
    /// Widgets whose clicks don't focus the editor (see `paint`).
    widgets_keeping_focus: Vec<Bounds<Pixels>>,
    colors: EditorColors,
    /// A high contrast theme (Monaco's `hc-black`/`hc-light` rules).
    high_contrast: bool,
    options: EditorOptions,
    focused: bool,
    cursor_visible: bool,
    cursor_opacity: f32,
    cursor_scale: f32,
    /// Middle-click scrolling's anchor dot.
    middle_scroll_anchor: Option<gpui::Point<Pixels>>,
    /// The mouse over the editor, for hover styles.
    mouse_position: Option<gpui::Point<Pixels>>,
    /// The scrollbars' sliders: (opacity, hovered, dragged) for vertical
    /// and horizontal.
    scrollbar_looks: [(f32, bool, bool); 2],
    overview_ruler: Option<crate::view::overview_ruler::RulerPaint>,
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
        // The editor's own hitbox: the mouse is over the editor itself, not
        // something painted above it (an overflowing widget, a host popup).
        state.hitbox = Some(window.insert_hitbox(bounds, HitboxBehavior::Normal));
        // Monaco's pointer: the I-beam over the text only; the arrow over
        // the gutter, scrollbars, minimap and widgets, a hand over folding
        // controls. Kept while drag-selecting text (the I-beam isn't shown
        // over them); a drag that captures the pointer shows its own over
        // all of them (`drag_cursor`).
        let mut cursor_areas = Vec::new();
        if state.cursor_areas_enabled {
            let layout = &state.layout;
            let gutter = Bounds::from_corners(
                layout.bounds.origin,
                point(layout.text_bounds.left(), layout.bounds.bottom()),
            );
            let mut arrows = vec![gutter];
            arrows.extend(
                [layout.vertical_scrollbar, layout.horizontal_scrollbar]
                    .into_iter()
                    .flatten()
                    .map(|bar| bar.bar),
            );
            arrows.extend(layout.minimap.map(|minimap| minimap.bounds));
            let hands: Vec<_> = layout
                .fold_controls
                .iter()
                .map(|control| control.bounds)
                .chain(layout.fold_placeholders.iter().map(|(_, bounds)| *bounds))
                // Monaco `.mtkoverflow`'s `cursor: pointer`.
                .chain(layout.overflow_chips())
                .collect();
            for area in arrows {
                cursor_areas.push((
                    MouseCursor::Arrow,
                    window.insert_hitbox(area, HitboxBehavior::Normal),
                ));
            }
            for area in hands {
                cursor_areas.push((
                    MouseCursor::PointingHand,
                    window.insert_hitbox(area, HitboxBehavior::Normal),
                ));
            }
        }
        let text = state.layout.text_bounds;
        let editor_area = state.layout.bounds;
        // Monaco: zones and content widgets are in the scrollable text area,
        // under the sticky scroll and the scrollbars, so those get the
        // clicks there; parts scrolled out of the editor take none. GPUI
        // clips hit tests to the content mask a hitbox was recorded with.
        let sticky_bottom = state
            .layout
            .sticky_lines
            .iter()
            .map(|line| line.bounds.bottom())
            .fold(text.top(), Pixels::max)
            .min(text.bottom());
        let below_sticky = |area: Bounds<Pixels>| {
            Bounds::from_corners(
                point(area.left(), area.top().max(sticky_bottom)),
                area.bottom_right(),
            )
        };
        let text_hits = below_sticky(text);
        let full_width_hits = below_sticky(Bounds::from_corners(
            point(editor_area.left(), text.top()),
            text.bottom_right(),
        ));
        let gutter_hits = below_sticky(Bounds::from_corners(
            point(editor_area.left(), text.top()),
            point(text.left(), text.bottom()),
        ));
        let viewport = Bounds::new(gpui::Point::default(), window.viewport_size());
        let mut widgets = WidgetHits::default();
        let mut overlays = Vec::new();
        // Monaco `typicalFullwidthCharacterWidth`, for secondary positions.
        let wide_char = state
            .layout
            .metrics
            .full_width_cell
            .unwrap_or(state.layout.metrics.em_width * 2.);
        for embedded in std::mem::take(&mut state.embedded) {
            widgets.view = Some(embedded.view_id());
            match embedded {
                Embedded::Zone {
                    view,
                    bounds: zone,
                    full_width,
                    captures_wheel,
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
                    let clip = if full_width {
                        full_width_hits
                    } else {
                        text_hits
                    };
                    // Zone widgets (peek) take the arrow; plain zones are
                    // part of the text.
                    let visible = zone.intersect(&clip);
                    if captures_wheel && !visible.is_empty() {
                        cursor_areas.push((
                            MouseCursor::Arrow,
                            window.insert_hitbox(visible, HitboxBehavior::Normal),
                        ));
                    }
                    window.with_content_mask(Some(ContentMask { bounds: clip }), |window| {
                        element.prepaint_at(zone.origin, window, cx)
                    });
                    widgets.push(
                        visible,
                        crate::mouse_target::EmbeddedKind::ViewZone,
                        captures_wheel,
                    );
                    if full_width {
                        state.full_width_zone_elements.push(element);
                    } else {
                        state.zone_elements.push(element);
                    }
                }
                Embedded::ZoneMargin {
                    view,
                    bounds: margin,
                } => {
                    let mut element = view.into_any_element();
                    element.layout_as_root(
                        size(
                            AvailableSpace::Definite(margin.size.width),
                            AvailableSpace::Definite(margin.size.height),
                        ),
                        window,
                        cx,
                    );
                    window.with_content_mask(
                        Some(ContentMask {
                            bounds: gutter_hits,
                        }),
                        |window| element.prepaint_at(margin.origin, window, cx),
                    );
                    widgets.push(
                        margin.intersect(&gutter_hits),
                        crate::mouse_target::EmbeddedKind::ViewZoneMargin,
                        false,
                    );
                    state.zone_margin_elements.push(element);
                }
                Embedded::Content {
                    view,
                    anchor,
                    secondary_x,
                    line_height,
                    preference,
                    overflow: Some((layer, room)),
                } => {
                    // Monaco `_layoutBoxInPage`: placed in the window, with
                    // some clearance, and told the room there is.
                    let top = viewport.top() + crate::widgets::OVERFLOW_VERTICAL_PADDING;
                    let bottom = viewport.bottom() - crate::widgets::OVERFLOW_VERTICAL_PADDING;
                    room.set(crate::widgets::OverflowRoom {
                        above: anchor.y - top,
                        below: bottom - (anchor.y + line_height),
                        width: viewport.size.width
                            - crate::widgets::OVERFLOW_HORIZONTAL_PADDING * 2.,
                        right: viewport.right()
                            - crate::widgets::OVERFLOW_HORIZONTAL_PADDING
                            - anchor.x,
                    });
                    // Monaco's `maxWidth` for overflowing widgets: the
                    // window's width.
                    let (element, measured) =
                        layout_capped(&view, Some(&room), viewport.size.width, window, cx);
                    // The part placed against the line: the whole widget, or
                    // the box it asked for (the suggest list, with its
                    // details around it), on the side it asked for.
                    let asked = room.anchor();
                    let (preference, anchor_box) = match asked {
                        Some(asked) => (vec![asked.preference], asked.anchor_box),
                        None => (preference, Bounds::new(point(px(0.), px(0.)), measured)),
                    };
                    let anchor_x =
                        reduce_anchor_x(anchor.x, secondary_x, anchor_box.size.width, wide_char);
                    // `_layoutHorizontalSegmentInPage`.
                    let min_x = (viewport.left() + crate::widgets::OVERFLOW_HORIZONTAL_PADDING)
                        .max(bounds.left() - anchor_box.size.width);
                    let max_x = (bounds.right() + anchor_box.size.width)
                        .min(viewport.right() - crate::widgets::OVERFLOW_HORIZONTAL_PADDING);
                    let x = anchor_x.min(max_x - anchor_box.size.width).max(min_x);
                    let Some(y) = crate::widgets::place_content_widget(
                        &preference,
                        anchor.y,
                        line_height,
                        anchor_box.size.height,
                        top..bottom,
                    ) else {
                        continue;
                    };
                    let origin = point(x, y) - anchor_box.origin;
                    // The parts that take the mouse: the whole widget, or
                    // the anchored part and the one beside it.
                    let (part, beside) = match asked.and_then(|asked| asked.beside) {
                        Some(beside) => (anchor_box, Some(beside)),
                        None => (Bounds::new(point(px(0.), px(0.)), measured), None),
                    };
                    for part in std::iter::once(part).chain(beside) {
                        widgets.push_overflowing(
                            Bounds::new(origin + part.origin, part.size).intersect(&viewport),
                            crate::mouse_target::EmbeddedKind::ContentWidget,
                            layer,
                        );
                    }
                    window.defer_draw(element, origin, OVERFLOW_PRIORITY + layer, None);
                }
                Embedded::Content {
                    view,
                    anchor,
                    secondary_x,
                    line_height,
                    preference,
                    overflow: None,
                } => {
                    // Monaco's `maxWidth`: the text area's width.
                    let (mut element, measured) =
                        layout_capped(&view, None, text.size.width, window, cx);
                    let anchor_x =
                        reduce_anchor_x(anchor.x, secondary_x, measured.width, wide_char);
                    // Monaco `_layoutBoxInViewport`: kept in the text area.
                    let x = anchor_x.min(text.right() - measured.width).max(text.left());
                    let Some(y) = crate::widgets::place_content_widget(
                        &preference,
                        anchor.y,
                        line_height,
                        measured.height,
                        text.top()..text.bottom(),
                    ) else {
                        continue;
                    };
                    let visible = Bounds::new(point(x, y), measured).intersect(&text_hits);
                    if !visible.is_empty() {
                        cursor_areas.push((
                            MouseCursor::Arrow,
                            window.insert_hitbox(visible, HitboxBehavior::Normal),
                        ));
                    }
                    window.with_content_mask(Some(ContentMask { bounds: text_hits }), |window| {
                        element.prepaint_at(point(x, y), window, cx)
                    });
                    widgets.push(
                        visible,
                        crate::mouse_target::EmbeddedKind::ContentWidget,
                        false,
                    );
                    state.content_elements.push(element);
                }
                Embedded::Glyph {
                    view,
                    bounds: glyph,
                } => {
                    let mut element = view.into_any_element();
                    element.layout_as_root(
                        size(
                            AvailableSpace::Definite(glyph.size.width),
                            AvailableSpace::Definite(glyph.size.height),
                        ),
                        window,
                        cx,
                    );
                    window.with_content_mask(
                        Some(ContentMask {
                            bounds: gutter_hits,
                        }),
                        |window| element.prepaint_at(glyph.origin, window, cx),
                    );
                    widgets.push(
                        glyph.intersect(&gutter_hits),
                        crate::mouse_target::EmbeddedKind::GlyphMarginWidget,
                        false,
                    );
                    state.glyph_elements.push(element);
                }
                Embedded::Overlay {
                    view,
                    position,
                    stack_ordinal,
                    overflow,
                } => {
                    let mut element = match overflow {
                        Some(_) => overflowing(view, None),
                        None => view.into_any_element(),
                    };
                    // A self-placing widget gets the editor's size.
                    let available = if position == OverlayPosition::Manual {
                        size(
                            AvailableSpace::Definite(bounds.size.width),
                            AvailableSpace::Definite(bounds.size.height),
                        )
                    } else {
                        size(AvailableSpace::MinContent, AvailableSpace::MinContent)
                    };
                    let measured = element.layout_as_root(available, window, cx);
                    overlays.push(PendingOverlay {
                        view: widgets.view,
                        element,
                        measured,
                        position,
                        stack_ordinal,
                        overflow,
                    });
                }
            }
        }
        // Overlay widgets, placed together as they stack (Monaco
        // `OverlayWidgets.render`).
        let layout_info = &state.layout.layout_info;
        let origins = crate::widgets::place_overlay_widgets(
            &overlays
                .iter()
                .map(|overlay| (overlay.position, overlay.stack_ordinal))
                .collect::<Vec<_>>(),
            &overlays
                .iter()
                .map(|overlay| overlay.measured)
                .collect::<Vec<_>>(),
            bounds,
            text,
            layout_info.vertical_scrollbar_width * 2. + layout_info.minimap.minimap_width,
            layout_info.horizontal_scrollbar_height * 2.,
        );
        for (overlay, mut origin) in overlays.into_iter().zip(origins) {
            let PendingOverlay {
                view,
                mut element,
                measured,
                overflow,
                ..
            } = overlay;
            widgets.view = view;
            if let Some(layer) = overflow {
                // Kept inside the window.
                origin.x = origin
                    .x
                    .min(viewport.right() - measured.width)
                    .max(viewport.left());
                origin.y = origin
                    .y
                    .min(viewport.bottom() - measured.height)
                    .max(viewport.top());
                widgets.push_overflowing(
                    Bounds::new(origin, measured).intersect(&viewport),
                    crate::mouse_target::EmbeddedKind::OverlayWidget,
                    layer,
                );
                window.defer_draw(element, origin, OVERFLOW_PRIORITY + layer, None);
                continue;
            }
            let visible = Bounds::new(origin, measured).intersect(&bounds);
            if !visible.is_empty() {
                cursor_areas.push((
                    MouseCursor::Arrow,
                    window.insert_hitbox(visible, HitboxBehavior::Normal),
                ));
            }
            window.with_content_mask(Some(ContentMask { bounds }), |window| {
                element.prepaint_at(origin, window, cx)
            });
            widgets.push(
                visible,
                crate::mouse_target::EmbeddedKind::OverlayWidget,
                true,
            );
            state.overlay_elements.push(element);
        }
        // Monaco's `linesDecorationsTooltip` on the folding control under
        // the mouse.
        let mouse = window.mouse_position();
        if let Some(control) = state
            .layout
            .fold_controls
            .iter()
            .find(|control| control.bounds.contains(&mouse))
            .copied()
            .filter(|_| state.layout.bounds.contains(&mouse))
        {
            let text = if control.collapsed {
                "Click to expand the range."
            } else {
                "Click to collapse the range."
            };
            use gpui::{InteractiveElement as _, StatefulInteractiveElement as _, Styled as _};
            let mut element = gpui::div()
                .id(("folding-control", control.row as usize))
                .w(control.bounds.size.width)
                .h(control.bounds.size.height)
                .tooltip(crate::tooltip::tooltip(text, &state.colors))
                .into_any_element();
            element.layout_as_root(
                size(
                    AvailableSpace::Definite(control.bounds.size.width),
                    AvailableSpace::Definite(control.bounds.size.height),
                ),
                window,
                cx,
            );
            element.prepaint_at(control.bounds.origin, window, cx);
            state.fold_tooltip = Some(element);
        }
        // Monaco's `.lightBulbWidget:hover { cursor: pointer }` and its
        // title, while the editor has focus (when it shows).
        if state.focused
            && let Some((bulb, _, _)) = state.lightbulb
            && bulb.contains(&mouse)
        {
            use gpui::prelude::FluentBuilder as _;
            use gpui::{InteractiveElement as _, StatefulInteractiveElement as _, Styled as _};
            let mut element = gpui::div()
                .id("lightbulb")
                .w(bulb.size.width)
                .h(bulb.size.height)
                .cursor_pointer()
                .when_some(state.lightbulb_title.clone(), |element, title| {
                    element.tooltip(crate::tooltip::tooltip(title, &state.colors))
                })
                .into_any_element();
            element.layout_as_root(
                size(
                    AvailableSpace::Definite(bulb.size.width),
                    AvailableSpace::Definite(bulb.size.height),
                ),
                window,
                cx,
            );
            element.prepaint_at(bulb.origin, window, cx);
            state.lightbulb_tooltip = Some(element);
        }
        state.cursor_areas = cursor_areas;
        state.widgets_keeping_focus = widgets
            .bounds
            .iter()
            .zip(&widgets.kinds)
            .zip(&widgets.captures_wheel)
            .filter(|((_, kind), captures)| {
                use crate::mouse_target::EmbeddedKind;
                match kind {
                    EmbeddedKind::ContentWidget | EmbeddedKind::OverlayWidget => true,
                    EmbeddedKind::ViewZone => **captures,
                    EmbeddedKind::ViewZoneMargin | EmbeddedKind::GlyphMarginWidget => false,
                }
            })
            .map(|((bounds, _), _)| *bounds)
            .collect();
        state.overflow_bounds = widgets.finish();
        self.editor.update(cx, |editor, _| {
            if let Some(layout) = editor.layout.as_mut() {
                layout.widget_bounds = widgets.bounds;
                layout.widget_kinds = widgets.kinds;
                layout.widget_views = widgets.views;
                layout.widget_captures_wheel = widgets.captures_wheel;
            }
        });
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
        // Mouse moves: over the editor (as a `div`'s `on_mouse_move`), over
        // its overflowing widgets (which block the editor's hitbox but are
        // still the editor's, as for the hover's mouse tracking), and
        // anywhere in the window while a drag that started in the editor
        // goes on (Monaco's `GlobalPointerMoveMonitor`). Registered before
        // the widgets paint, so their listeners run first.
        if let Some(hitbox) = state.hitbox.clone() {
            let editor = self.editor.clone();
            let overflow = state.overflow_bounds.clone();
            window.on_mouse_event(move |event: &MouseMoveEvent, phase, window, cx| {
                if phase != DispatchPhase::Bubble {
                    return;
                }
                let over_overflow = overflow.iter().any(|b| b.contains(&event.position));
                editor.update(cx, |editor, cx| {
                    if hitbox.is_hovered(window) || over_overflow || editor.is_mouse_dragging() {
                        editor.editor_mouse_move(event, window, cx);
                    }
                });
            });
        }
        // GPUI focuses a focusable element under any mouse down; Monaco
        // doesn't focus the editor for clicks (of any button) on its
        // widgets, which keep focus themselves. Registered after the
        // editor's root, so it runs first.
        let keep_focus = std::mem::take(&mut state.widgets_keeping_focus);
        if !keep_focus.is_empty() {
            window.on_mouse_event(move |event: &MouseDownEvent, phase, window, _| {
                if phase == DispatchPhase::Bubble
                    && keep_focus.iter().any(|b| b.contains(&event.position))
                {
                    window.prevent_default();
                }
            });
        }
        for (cursor, hitbox) in &state.cursor_areas {
            window.set_cursor_style(*cursor, hitbox);
        }
        {
            let editor = self.editor.read(cx);
            let mut painted = editor.painted_cursor_areas.borrow_mut();
            painted.clear();
            painted.extend(
                state
                    .cursor_areas
                    .iter()
                    .map(|(cursor, hitbox)| (*cursor, hitbox.bounds)),
            );
        }
        crate::pointer_capture::paint_captured_cursor(state.drag_cursor, window);
        paint_editor(state, window, cx);
    }
}

/// Monaco `View._computeGlyphMarginLanes`: the lanes the glyph decorations
/// and glyph margin widgets use, line by line. Without any picking a lane
/// it's the one center lane, without looking at the decorations.
fn glyph_margin_lanes(
    editor: &CodeEditor,
    cx: &gpui::Context<CodeEditor>,
) -> crate::view::glyph_lanes::GlyphMarginLanesModel {
    use crate::decorations::GlyphMarginLane;
    let mut lanes = crate::view::glyph_lanes::GlyphMarginLanesModel::default();
    let model = editor.model().read(cx);
    let set = model.decoration_set();
    let widgets_pick_lanes = editor
        .widgets
        .glyphs
        .iter()
        .any(|widget| widget.lane != GlyphMarginLane::Center);
    if !set.has_glyph_lanes() && !widgets_pick_lanes {
        return lanes;
    }
    let buffer = model.buffer();
    let editor_id = cx.entity_id();
    let mut glyphs: Vec<(u32, u32, GlyphMarginLane, bool)> = set
        .iter()
        .filter(|d| d.options.glyph.is_some() && d.owner.is_none_or(|o| o == editor_id))
        .map(|d| {
            let options = d.options.glyph_margin.unwrap_or_default();
            (
                buffer.offset_to_point(d.range.start).row,
                buffer.offset_to_point(d.range.end).row,
                options.position,
                options.persist_lane,
            )
        })
        .collect();
    glyphs.extend(editor.widgets.glyphs.iter().map(|widget| {
        let row = buffer.offset_to_point(buffer.resolve(&widget.anchor)).row;
        (row, row, widget.lane, false)
    }));
    glyphs.sort_by_key(|(start, end, _, _)| (*start, *end));
    lanes.reset(glyphs.iter().map(|g| g.1).max().unwrap_or(0));
    for (start, end, lane, persist) in glyphs {
        lanes.push(lane, start..=end, persist);
    }
    lanes
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

    // Monaco `accessibilitySupport`: optimized for a screen reader, a model
    // dominated by long lines wraps at the viewport, measured and without
    // continuation indent, so the wrapping matches the screen reader's.
    let screen_reader = editor.screen_reader_optimized(window);
    // Monaco `EditorLayoutInfoComputer.computeLayout`, from the view lines
    // as they are before this frame's update, as Monaco's are: the gutter
    // is [glyph margin][line numbers][line decorations + folding], then
    // the minimap's share, and where lines wrap.
    let f64_of = |pixels: Pixels| f64::from(f32::from(pixels));
    let glyph_lanes = glyph_margin_lanes(editor, cx);
    let mut layout_info = crate::layout_info::compute_layout(
        &options,
        &crate::layout_info::LayoutEnv {
            outer_width: f64_of(bounds.size.width),
            outer_height: f64_of(bounds.size.height),
            line_height: f64_of(line_height),
            line_numbers_digit_count: line_count.to_string().len() as u32,
            typical_halfwidth_character_width: f64_of(metrics.em_width),
            max_digit_width: f64_of(metrics.max_digit_width),
            pixel_ratio: f64::from(window.scale_factor()),
            view_line_count: editor.view_lines.line_count().max(1),
            glyph_margin_decoration_lane_count: glyph_lanes.required_lanes() as u32,
            is_dominated_by_long_lines: editor.screen_reader.dominated_by_long_lines,
            screen_reader,
        },
        &mut editor.minimap_cache.layout_memory,
    );
    let numbers_width = layout_info.line_numbers_width;
    let glyph_width = layout_info.glyph_margin_width;
    // The folding controls are the line decorations' last 16px.
    let folding_width =
        if options.folding && options.show_folding_controls != ShowFoldingControls::Never {
            px(16.)
        } else {
            px(0.)
        };
    let line_decorations_width = layout_info.decorations_width - folding_width;
    let gutter_width = layout_info.content_left - layout_info.glyph_margin_left;

    let vbar = options.vertical_scrollbar_size;
    let hbar = options.horizontal_scrollbar_size;
    let viewport_height = bounds.size.height;
    let minimap_width = layout_info.minimap.minimap_width;
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
    let text_width = (bounds.size.width
        - gutter_width
        - vbar
        - if minimap_left { px(0.) } else { minimap_width })
    .max(px(0.));

    // Bring the view lines up to date: edits, wrapping, folds. Monaco
    // doesn't wrap models too large to tokenize (`IdentityLinesCollection`).
    let too_large = editor.model().read(cx).is_too_large_for_tokenization();
    if too_large {
        layout_info.is_word_wrap_minified = false;
        layout_info.is_viewport_wrapping = false;
        layout_info.wrapping_column = None;
    }
    let wrapping_strategy = if screen_reader {
        crate::options::WrappingStrategy::Advanced
    } else {
        options.wrapping_strategy
    };
    let wrapping_indent = if screen_reader {
        crate::view::line_breaks::WrappingIndent::None
    } else {
        options.wrapping_indent
    };
    let wrap_column = layout_info.wrapping_column;
    let measure = (wrap_column.is_some()
        && wrapping_strategy == crate::options::WrappingStrategy::Advanced)
        .then(|| {
            let widths = CharWidths::for_metrics(&editor.char_widths, &metrics, cx);
            editor.char_widths = Some(widths.clone());
            crate::view::view_lines::MeasuredWidths(widths)
        });
    let classifier = editor.wrap_classifier();
    editor.sync_view_lines(
        wrap_column.map(|column| WrapSettings {
            column,
            tab_size,
            indent: wrapping_indent,
            keep_all: options.word_break == crate::options::WordBreak::KeepAll,
            classifier,
            wrap_on_escaped_line_feeds: options.wrap_on_escaped_line_feeds,
            measure,
        }),
        cx,
    );
    let view_lines = editor.view_lines.clone();
    let view_line_count = view_lines.line_count().max(1);
    let snapshot = editor.snapshot.clone();

    // Code lens zones follow font and zoom changes (Monaco `updateHeight`).
    editor.update_code_lens_heights();
    // View zones (Monaco `ViewZones._computeWhitespaceProps`): after the
    // last view line of their row, or the one showing `afterColumn`;
    // hidden when the position after them is folded away, unless
    // `showInHiddenAreas`.
    let mut zone_specs: Vec<ZoneSpec> = Vec::new();
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
            // Re-based on this version, so resolving stays cheap as edits
            // pile up.
            let offsets: Vec<Option<usize>> = editor
                .widgets
                .zones
                .iter_mut()
                .map(|zone| {
                    let anchor = zone.anchor.as_mut()?;
                    *anchor = buffer.refresh(anchor);
                    Some(buffer.resolve(anchor))
                })
                .collect();
            // Rows in one forward pass over the text (there can be tens of
            // thousands of zones, e.g. code lenses).
            let mut order: Vec<usize> = (0..offsets.len())
                .filter(|&i| offsets[i].is_some())
                .collect();
            order.sort_by_key(|&i| offsets[i]);
            let sorted: Vec<usize> = order.iter().filter_map(|&i| offsets[i]).collect();
            let mut zone_rows = vec![0u32; offsets.len()];
            for (&i, point) in order.iter().zip(buffer.rope().offsets_to_points(&sorted)) {
                zone_rows[i] = point.row;
            }
            let rope = buffer.rope();
            let max_row = rope.max_point().row;
            offsets
                .iter()
                .zip(zone_rows)
                .zip(&editor.widgets.zones)
                .map(|((offset, row), zone)| {
                    // No anchor: above the first line.
                    let Some(offset) = *offset else {
                        return Some(-1);
                    };
                    // The zone's column: its anchor's with `afterColumn`,
                    // else the line's end. Measured only when needed: this
                    // runs for every zone after each edit.
                    let column = zone
                        .after_column
                        .then(|| buffer.offset_to_point_utf16(offset).column);
                    // The position right after the zone: the next line's
                    // start after a line's end (the last line's end at the
                    // end of the text).
                    let at_line_end = column.is_none_or(|c| c >= rope.line_len_utf16(row));
                    let before_row = if at_line_end {
                        (row + 1).min(max_row)
                    } else {
                        row
                    };
                    if !zone.show_in_hidden_areas && view_lines.is_row_hidden(before_row) {
                        return None;
                    }
                    if !zone.after_column && !view_lines.is_row_hidden(row) {
                        let first = view_lines.first_view_line_of_row(row);
                        let next = view_lines.first_view_line_of_row(row + 1);
                        return Some(next.max(first + 1) as i64 - 1);
                    }
                    let column = column.unwrap_or_else(|| rope.line_len_utf16(row));
                    let point = PointUtf16::new(row, column);
                    let prefer_previous = zone.after_column_affinity.prefers_previous();
                    Some(view_lines.to_view(point, prefer_previous).0 as i64)
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
            zone_specs.push(ZoneSpec {
                after,
                ordinal: zone.ordinal,
                height,
                view: zone.view.clone(),
                margin_view: zone.margin_view.clone(),
                full_width: zone.full_width,
                captures_wheel: zone.captures_wheel,
            });
        }
        editor.widgets.zone_rows = Some((key, rows));
    }
    // Monaco `padding.top` comes before everything, view zones above the
    // first line included.
    let mut zone_heights: Vec<(i64, Pixels)> = zone_specs
        .iter()
        .map(|zone| (zone.after, zone.height))
        .collect();
    if options.padding.top > px(0.) {
        zone_heights.push((-1, options.padding.top));
    }
    let zones =
        ZoneLayout::new(zone_heights).with_line_heights(if options.allow_variable_line_heights {
            decoration_line_heights(editor, &view_lines, line_height, cx.entity_id(), cx)
        } else {
            Vec::new()
        });

    // Rough content width from the longest row; refined by shaped lines below.
    let mut content_width = if wrap_column.is_some() {
        text_width
    } else {
        metrics.em_width
            * (snapshot
                .summary()
                .longest_row_chars
                // Lines stop rendering after the limit.
                .min(options.stop_rendering_line_after.unwrap_or(u32::MAX)) as f32
                + options.scroll_beyond_last_column as f32)
    };
    // Monaco `minWidthInPx` and `getMinContentWidthInPx`: view zones and
    // overlay widgets that need a wider scroll range.
    for zone in &editor.widgets.zones {
        content_width = content_width.max(zone.min_width);
    }
    for widget in &editor.widgets.overlays {
        content_width = content_width.max(widget.min_content_width);
    }
    let show_hbar = content_width > text_width;

    let lines_height =
        line_height * view_line_count as f32 + zones.total() + options.padding.bottom;
    // Monaco `ViewLayout._getContentHeight`: scrolling beyond the last
    // line, or else room for a shown horizontal scrollbar under it (unless
    // `ignoreHorizontalScrollbarInContentHeight`).
    let hbar_height = if show_hbar
        && options.scrollbar.horizontal != crate::options::ScrollbarVisibility::Hidden
    {
        hbar
    } else {
        px(0.)
    };
    let lines_height = if options.scroll_beyond_last_line
        || options
            .scrollbar
            .ignore_horizontal_scrollbar_in_content_height
    {
        lines_height
    } else {
        lines_height + hbar_height
    };
    let content_height = lines_height
        + if options.scroll_beyond_last_line {
            (viewport_height - line_height - options.padding.bottom).max(px(0.))
        } else {
            px(0.)
        };
    let max_scroll_top = (content_height - viewport_height).max(px(0.));
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
    // The cursors' view positions (Monaco `CursorState.viewState`).
    let cursor_views = editor.cursor_view_positions();
    let primary_view_line = cursor_views[0].row;
    if let Some(offset) = editor.pending_cursor_offset.take() {
        editor.scroll_top = zones.line_y(primary_view_line, line_height) - offset;
        editor.autoscroll_requested = false;
    }
    // Monaco `restoreViewState`: back to the first visible position.
    if let Some((position, delta_top)) = editor.pending_scroll_anchor.take() {
        let position = view_lines
            .rope()
            .clip_point_utf16(position, text::Bias::Left);
        let line = view_lines.to_view(position, false).0;
        editor.scroll_top = zones.line_y(line, line_height) - delta_top;
    }
    if let Some(request) = editor.pending_reveal.take() {
        // Monaco `_computeScrollTopToRevealRange`: the box of the range's
        // lines, or of every cursor's.
        let start = request
            .ranges
            .iter()
            .map(|range| view_lines.to_view(range.start, false).0)
            .min()
            .unwrap_or(0);
        let end = request
            .ranges
            .iter()
            .map(|range| view_lines.to_view(range.end, false).0)
            .max()
            .unwrap_or(0)
            .max(start);
        let surrounding =
            (options.cursor_surrounding_lines as f32).max(if options.sticky_scroll.enabled {
                options.sticky_scroll.max_line_count as f32
            } else {
                0.
            });
        let reveal_request = crate::view::reveal::RevealRequest {
            viewport_top: f32::from(editor.scroll_top),
            viewport_height: f32::from(text_height),
            box_start: f32::from(zones.line_y(start, line_height)),
            box_end: f32::from(zones.line_y(end, line_height) + zones.height(end, line_height)),
            line_height: f32::from(line_height),
            surrounding_lines: surrounding,
            mouse: request.minimal
                && options.cursor_surrounding_lines_style
                    == crate::options::CursorSurroundingLinesStyle::Default,
            minimal: request.minimal,
        };
        let top = if request.cursors {
            crate::view::reveal::scroll_top_to_reveal_cursors(reveal_request, request.reveal)
        } else {
            Some(crate::view::reveal::scroll_top_to_reveal(
                reveal_request,
                request.reveal,
            ))
        };
        // Monaco aborts revealing cursors that don't fit, horizontally too.
        if let Some(top) = top {
            let top = px(top).clamp(px(0.), max_scroll_top);
            // Monaco `onRevealRangeRequest`: a line or less jumps.
            let current = editor
                .animation
                .animated_scroll_position()
                .map_or(editor.scroll_top, |position| position.y);
            let scroll_type = if (current - top).abs() <= line_height {
                crate::view::reveal::ScrollType::Immediate
            } else {
                request.scroll_type
            };
            editor.apply_scroll_type(scroll_type);
            editor.scroll_top = top;
            editor.pending_reveal_x = None;
            if request.horizontal {
                let multi_line = !request.cursors
                    && request
                        .ranges
                        .first()
                        .is_some_and(|range| range.start.row != range.end.row);
                if multi_line {
                    // Two or more lines: scroll to the start of the lines.
                    editor.scroll_left = px(0.);
                } else {
                    editor.pending_reveal_x = Some(crate::editor::HorizontalReveal {
                        ranges: request.ranges,
                        cursors: request.cursors,
                        minimal: request.minimal,
                    });
                }
            }
        }
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
                minimal: editor.autoscroll_minimal,
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
        .map(|view_line| {
            let mut segment = row_segment(&view_lines, view_line);
            // Monaco `stopRenderingLineAfter`: very long lines are cut off.
            if let Some(limit) = options.stop_rendering_line_after {
                let len = segment.columns.end - segment.columns.start;
                if len > limit {
                    segment.overflow = len - limit;
                    segment.columns.end = segment.columns.start + limit;
                    segment.is_last_segment = true;
                }
            }
            segment
        })
        .collect();

    let marked = editor.marked_range_points();
    let marked_active = editor.marked_selection_points();
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
    // Monaco `ghost-text-hidden`: the rows whose end moved below their
    // ghost text, as of the decorations above.
    let ghost_hidden = editor.ghost_text_hidden_rows();
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
    editor.refresh_post_edit(cx);
    editor.resolve_visible_code_lenses(cx);
    // Only the drawn parts of lines (long ones stop rendering early).
    let mut rendered: Vec<Range<usize>> = Vec::new();
    for segment in &segments {
        let start =
            snapshot.point_utf16_to_offset(PointUtf16::new(segment.row, segment.columns.start));
        let end = snapshot.point_utf16_to_offset(PointUtf16::new(segment.row, segment.columns.end));
        match rendered.last_mut() {
            // Adjacent segments, or the next row after a line break.
            Some(last) if start <= last.end + 2 => last.end = last.end.max(end),
            _ => rendered.push(start..end),
        }
    }
    let bracket_colors: Vec<_> = rendered
        .iter()
        .flat_map(|range| editor.bracket_colors_in(range.clone(), cx))
        .collect();
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
        // High contrast themes box the composition instead (below).
        // The composition's columns on this row (it may span rows), with
        // the IME's selected part.
        let row_columns = |range: &Range<PointUtf16>| {
            (range.start.row <= segment.row && segment.row <= range.end.row).then(|| {
                let start = if range.start.row == segment.row {
                    range.start.column
                } else {
                    0
                };
                let end = if range.end.row == segment.row {
                    range.end.column
                } else {
                    u32::MAX
                };
                start..end
            })
        };
        let marked_cols = marked
            .as_ref()
            .filter(|_| !editor.theme.high_contrast)
            .and_then(|range| {
                let active = marked_active.as_ref().and_then(row_columns);
                row_columns(range).map(|columns| (columns, active, colors.ime_underline))
            });
        // Decoration text styles override syntax colors; injected text goes
        // before/after decorated ranges.
        let (decoration_styles, mut injections, fonts) = decoration_styles_and_injections(
            segment.row,
            line_range.clone(),
            &bracket_colors,
            &decoration_points,
            &editor.options,
            &colors,
        );
        // Monaco's `mtkoverflow`: "Show more (1.9 MB)" where it stops.
        if segment.overflow > 0 {
            injections.retain(|(column, _, _, _)| *column <= segment.columns.end);
            injections.push((
                segment.columns.end,
                true,
                InjectedText {
                    content: format!("Show more ({})", overflow_size(segment.overflow)).into(),
                    style: TokenStyle {
                        foreground: Some(colors.button_foreground),
                        ..Default::default()
                    },
                    background: Some(colors.button_background),
                    swatch: None,
                    ..Default::default()
                },
                None,
            ));
        }
        let row_highlights = if decoration_styles.is_empty() {
            row_highlights
        } else {
            overlay_styles(&row_highlights, &decoration_styles)
        };
        // Monaco's `inline-selected-text`: selected text in
        // `editor.selectionForeground` (high-contrast themes).
        let row_highlights = match colors.selection_foreground {
            Some(foreground) => {
                let rope = snapshot.rope();
                let mut selected: Vec<(Range<usize>, TokenStyle)> = selections
                    .iter()
                    .filter(|s| {
                        !s.is_empty() && s.start().row <= segment.row && segment.row <= s.end().row
                    })
                    .map(|s| {
                        let start = if s.start().row == segment.row {
                            rope.point_utf16_to_offset(s.start()) - line_range.start
                        } else {
                            0
                        };
                        let end = if s.end().row == segment.row {
                            rope.point_utf16_to_offset(s.end()) - line_range.start
                        } else {
                            line_range.len()
                        };
                        (
                            start..end,
                            TokenStyle {
                                foreground: Some(foreground),
                                ..Default::default()
                            },
                        )
                    })
                    .collect();
                if selected.is_empty() {
                    row_highlights
                } else {
                    selected.sort_by_key(|(range, _)| range.start);
                    overlay_styles(&row_highlights, &selected)
                }
            }
            None => row_highlights,
        };
        let hidden_segment;
        let segment = match ghost_hidden
            .iter()
            .find(|(row, _)| *row == segment.row)
            .and_then(|(_, from)| hide_segment_from(segment, *from))
        {
            Some(hidden) => {
                hidden_segment = hidden;
                &hidden_segment
            }
            None => segment,
        };
        let line = layout_segment(
            &snapshot,
            segment,
            tab_size,
            &metrics,
            colors.foreground,
            &row_highlights,
            &fonts,
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
    if std::mem::take(&mut editor.autoscroll_vertical_only) {
        editor.autoscroll_requested = false;
    }
    if editor.autoscroll_requested {
        let primary_x = lines
            .iter()
            .find(|line| line.view_line == primary_view_line)
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
    if let Some(request) = editor.pending_reveal_x.take()
        && let Some(left) = scroll_left_to_reveal(
            &request,
            &lines,
            editor.scroll_left,
            text_width,
            options.reveal_horizontal_right_padding,
        )
    {
        editor.scroll_left = left;
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
        // Monaco's arrows take `arrowSize` at each end, as
        // `layoutInfo.overviewRuler` assumes.
        let arrow = if scrollbar_options.vertical_has_arrows {
            scrollbar_options.arrow_size
        } else {
            px(0.)
        };
        let track = Bounds::new(
            point(bar.left(), bar.top() + arrow),
            size(vbar, (viewport_height - arrow * 2.).max(px(0.))),
        );
        let thumb_height = slider_size(track.size.height, viewport_height, content_height);
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
            scrollbar_options.arrow_size
        } else {
            px(0.)
        };
        let track = Bounds::new(
            point(bar.left() + arrow, bar.top()),
            size((text_width - arrow * 2.).max(px(0.)), hbar),
        );
        let thumb_width = slider_size(track.size.width, text_width, content_width);
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
        editor.animation.scrollbars.revealed_at = Some(web_time::Instant::now());
        // Monaco `minimap.autohide: scroll` shows it while scrolling.
        editor.animation.minimap.scrolled_at = Some(web_time::Instant::now());
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
        // Monaco draws the control as a line decoration (`.cldr`): the
        // whole line decorations column, its codicon centered, with
        // `folding.css`'s 2px left margin.
        let left = bounds.left() + glyph_width + numbers_width + px(2.);
        let control_width = line_decorations_width + folding_width;
        // Monaco `showFoldingControls: never` has no controls, collapsed
        // regions' included.
        for line in lines
            .iter()
            .filter(|line| line.columns.start == 0 && folding_width > px(0.))
        {
            let top = bounds.top() + zones.line_y(line.view_line, line_height) - scroll_top;
            let height = zones.height(line.view_line, line_height);
            let is_collapsed = collapsed.iter().any(|(start, _)| *start == line.row);
            if is_collapsed || editor.folds.range_starting_at(line.row).is_some() {
                fold_controls.push(FoldControl {
                    row: line.row,
                    bounds: Bounds::new(point(left, top), size(control_width, height)),
                    collapsed: is_collapsed,
                });
            }
        }
        for line in lines.iter().filter(|line| line.is_last_segment) {
            if collapsed.iter().any(|(start, _)| *start == line.row) {
                // Centered on taller lines, with the text.
                let top = bounds.top() + zones.line_y(line.view_line, line_height) - scroll_top
                    + (zones.height(line.view_line, line_height) - line_height) / 2.;
                // Monaco `.inline-folded:after`: "⋯" after the text, with
                // a 0.2em margin each side.
                let margin = metrics.font_size * 0.2;
                let x = text_bounds.left() - scroll_left + line.width() + margin;
                fold_placeholders.push((x, top));
                fold_placeholder_bounds.push((
                    line.row,
                    Bounds::new(point(x, top), size(metrics.em_width, line_height)),
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
    // Monaco `folding.css`: the expanded regions' controls show with
    // `always`, else fade in over 0.5s while the mouse is over the gutter.
    let fold_controls_opacity = match options.show_folding_controls {
        ShowFoldingControls::Always => 1.,
        ShowFoldingControls::Mouseover => {
            let (opacity, fading) = editor.animation.folding.update(editor.gutter_hovered);
            if fading {
                window.request_animation_frame();
            }
            opacity
        }
        ShowFoldingControls::Never => 0.,
    };

    // Monaco `ScrollDecorationViewPart`, with `scrollbar.useShadows`: a
    // shadow along the top once scrolled, across the editor except a
    // vertical scrollbar right of the minimap.
    let scroll_decoration = (options.scrollbar.use_shadows && scroll_top > px(0.)).then(|| {
        let minimap = layout_info.minimap;
        let minimap_right = minimap.render_minimap != crate::layout_info::RenderMinimap::None
            && !(minimap.minimap_width > px(0.) && minimap.minimap_left == px(0.));
        let right = if minimap_right {
            bounds.right() - layout_info.vertical_scrollbar_width
        } else {
            bounds.right()
        };
        Bounds::new(
            bounds.origin,
            size(right - bounds.left(), px(SCROLL_SHADOW_HEIGHT)),
        )
    });
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
        content_height: lines_height,
        // Monaco counts the vertical scrollbar in (`_computeContentWidth`).
        content_width: content_width + vbar,
        layout_info,
        scroll_decoration,
        lines,
        fold_controls,
        fold_placeholders: fold_placeholder_bounds,
        zones: zones.clone(),
        widget_bounds: Vec::new(),
        widget_kinds: Vec::new(),
        widget_views: Vec::new(),
        widget_captures_wheel: Vec::new(),
        lightbulb: None,
        minimap: None,
        minimap_paint: None,
        sticky_lines: Vec::new(),
        sticky_layouts: Vec::new(),
        vertical_scrollbar,
        horizontal_scrollbar,
        line_number_labels: Vec::new(),
        cursors: (Vec::new(), false),
        wrap_indicators: Vec::new(),
        line_highlights: Vec::new(),
        line_number_styles: Vec::new(),
        glyphs_drawn: Vec::new(),
        gutter_bars: Vec::new(),
    };

    // Minimap (Monaco `Minimap`), sized per `minimap.size`
    // (`EditorLayoutInfoComputer`).
    if minimap_width > px(0.) {
        let minimap_bounds = Bounds::new(
            if minimap_left {
                full_bounds.origin
            } else {
                point(bounds.right() - vbar - minimap_width, bounds.top())
            },
            size(minimap_width, viewport_height),
        );
        // View zones in the viewport, besides the top padding (Monaco
        // `whitespaceViewportData`).
        let whitespace_gaps = zone_specs.iter().any(|zone| {
            zone.height > px(0.)
                && zone.after + 1 >= first_line as i64
                && zone.after <= last_line as i64
        });
        let (minimap, paint) = crate::minimap::prepaint(
            editor,
            crate::minimap::MinimapInput {
                options: &options,
                info: layout.layout_info.minimap,
                bounds: minimap_bounds,
                editor_bounds: full_bounds,
                view_lines: &view_lines,
                snapshot: &snapshot,
                theme: &theme,
                zones: &zones,
                line_height,
                viewport_height,
                scroll_top,
                max_scroll_top,
                scroll_left,
                max_scroll_left,
                first_line,
                last_line,
                whitespace_gaps,
                tab_size,
                wrap_column,
                selections: &selections,
            },
            window,
            cx,
        );
        layout.minimap = Some(minimap);
        layout.minimap_paint = Some(std::sync::Arc::new(paint));
    }

    // Sticky scroll: headers of the fold ranges enclosing the top of the
    // viewport (Monaco `stickyScroll`; see `sticky_model`).
    let mut sticky_shaped: Vec<(usize, Pixels, LineLayout, Option<ShapedLine>)> = Vec::new();
    let mut sticky_height = px(0.);
    // Monaco: no sticky scroll for models too large to tokenize.
    if options.sticky_scroll.enabled && !options.single_line && !too_large && scroll_top > px(0.) {
        let elements = editor.sticky_elements();
        let first_row = view_lines
            .view_line(zones.line_at(scroll_top, line_height, view_line_count))
            .row;
        let last_row = view_lines
            .view_line(zones.line_at(scroll_top + viewport_height, line_height, view_line_count))
            .row;
        let line_count = snapshot.line_count();
        let lh = f32::from(line_height);
        let candidates = crate::sticky_model::candidates_intersecting(
            &elements,
            (first_row, last_row),
            view_lines.hidden(),
            line_count,
            &|_| lh,
        );
        // Monaco `getTopForLineNumber` and `getBottomForLineNumber`: a
        // hidden row is where the line it's folded into ends.
        let view_line_count = view_lines.line_count();
        let top_of = |row: u32| {
            let line = view_lines.first_view_line_of_row(row);
            let line = if view_lines.is_row_hidden(row) {
                line.saturating_sub(1)
            } else {
                line.min(view_line_count.saturating_sub(1))
            };
            f32::from(zones.line_y(line, line_height))
        };
        let bottom_of = |row: u32| {
            let line = view_lines.first_view_line_of_row(row + 1).saturating_sub(1);
            f32::from(zones.line_y(line, line_height) + zones.height(line, line_height))
        };
        let state = crate::sticky_model::find_scroll_widget_state(
            &candidates,
            crate::sticky_model::max_sticky_lines(
                f32::from(viewport_height),
                lh,
                options.sticky_scroll.max_line_count,
            ),
            f32::from(scroll_top),
            &top_of,
            &bottom_of,
        );
        let collapsed_rows = editor
            .folds
            .collapsed_rows(editor.model().read(cx).buffer());
        let editor_id = cx.entity_id();
        let show_validation = editor.shows_validation_decorations();
        let count = state.start_rows.len();
        // Monaco's sticky widget clips its lines to its height, which the
        // pushed last line shrinks (`_renderRootNode`).
        sticky_height = line_height * count as f32 + px(state.last_line_relative_position);
        for (ix, start_row) in state.start_rows.iter().enumerate() {
            // Monaco: Shift over a sticky line shows the scope's end.
            let row = match editor.sticky.show_end {
                Some(end) if end == ix => state.end_rows[ix],
                _ => *start_row,
            };
            if row >= line_count {
                continue;
            }
            let collapsed = collapsed_rows.iter().any(|(start, _)| *start == row);
            // Monaco `_updatePosition`: the last line slides up under the
            // ones above as its scope scrolls out (a collapsed one a pixel
            // less).
            let top = if ix + 1 == count {
                line_height * ix as f32
                    + px(state.last_line_relative_position)
                    + if collapsed { px(1.) } else { px(0.) }
            } else {
                line_height * ix as f32
            };
            let segment = row_segment(&view_lines, view_lines.first_view_line_of_row(row));
            let line_range = snapshot.line_range(row);
            // Monaco renders sticky lines with their inline decorations and
            // injected text (`RenderedStickyLine`), so they line up with the
            // real lines.
            let (row_highlights, injections, fonts) = {
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
                let row_decorations: Vec<_> = editor
                    .model()
                    .update(cx, |model, _| {
                        model.decorations_in_range(line_range.clone(), Some(editor_id))
                    })
                    .into_iter()
                    .filter(|d| show_validation || !d.options.validation)
                    .collect();
                let points: Vec<_> = row_decorations
                    .iter()
                    .map(|d| {
                        (
                            snapshot.offset_to_point_utf16(d.range.start),
                            snapshot.offset_to_point_utf16(d.range.end),
                            d,
                        )
                    })
                    .collect();
                let brackets = editor.bracket_colors_in(line_range.clone(), cx);
                let (styles, injections, fonts) = decoration_styles_and_injections(
                    row,
                    line_range.clone(),
                    &brackets,
                    &points,
                    &editor.options,
                    &colors,
                );
                let row_highlights = if styles.is_empty() {
                    row_highlights
                } else {
                    overlay_styles(&row_highlights, &styles)
                };
                (row_highlights, injections, fonts)
            };
            let shaped = layout_segment(
                &snapshot,
                &segment,
                tab_size,
                &metrics,
                colors.foreground,
                &row_highlights,
                &fonts,
                &injections,
                None,
                window,
            );
            // Monaco's sticky line numbers: absolute (every 10th with
            // `interval`), relative, or none for `off` and custom labels.
            let label = match &options.line_numbers {
                LineNumbers::On => Some((row + 1).to_string()),
                LineNumbers::Interval if (row + 1) % 10 == 0 => Some((row + 1).to_string()),
                LineNumbers::Relative => Some(primary.row.abs_diff(row).to_string()),
                _ => None,
            };
            let number = label.map(|label| {
                window.text_system().shape_line(
                    label.clone().into(),
                    metrics.font_size,
                    &[run(label.len(), &metrics.font, colors.line_number)],
                    None,
                )
            });
            // Monaco `_renderFoldingIconForLine`: with folding on and
            // `showFoldingControls` not `never`, on a folding range's
            // header, 26px wide after the line numbers (with a 2px margin).
            let fold = (folding_width > px(0.) && editor.folds.range_starting_at(row).is_some())
                .then(|| FoldControl {
                    row,
                    bounds: Bounds::new(
                        point(
                            bounds.left() + glyph_width + numbers_width + px(2.),
                            bounds.top() + top,
                        ),
                        size(px(26.), line_height),
                    ),
                    collapsed,
                });
            // The pushed last line only shows above the widget's bottom.
            let line_bounds = Bounds::new(
                point(bounds.left(), bounds.top() + top),
                size(gutter_width + text_width, line_height),
            );
            let line_bounds = Bounds::from_corners(
                line_bounds.origin,
                point(
                    line_bounds.right(),
                    line_bounds.bottom().min(bounds.top() + sticky_height),
                ),
            );
            layout.sticky_lines.push(StickyLine {
                end_row: state.end_rows[ix],
                row,
                bounds: line_bounds,
                fold,
                origin_x: if options.sticky_scroll.scroll_with_editor {
                    layout.text_origin_x()
                } else {
                    text_bounds.left()
                },
            });
            layout.sticky_layouts.push(shaped.clone());
            sticky_shaped.push((ix, top, shaped, number));
        }
        // Monaco keeps the focused sticky line focused while it shows, else
        // focuses the last one (`_renderStickyScroll`).
        if let Some(focused) = editor.sticky.focused {
            let focused_row = editor
                .layout
                .as_ref()
                .and_then(|l| l.sticky_lines.get(focused))
                .map(|line| line.row);
            editor.sticky.focused = if layout.sticky_lines.is_empty() {
                None
            } else {
                Some(
                    focused_row
                        .and_then(|row| layout.sticky_lines.iter().position(|l| l.row == row))
                        .unwrap_or(layout.sticky_lines.len() - 1),
                )
            };
        }
    }
    let numbers_right = bounds.left() + glyph_width + numbers_width;

    // Decoration geometry.
    let mut decoration_quads: Vec<(Bounds<Pixels>, Option<Hsla>, Option<Hsla>)> = Vec::new();
    // Monaco `BlockDecorations`: from above the first line's view zones to
    // after the last line's.
    let mut block_borders: Vec<(Bounds<Pixels>, Hsla, Pixels)> = Vec::new();
    for (start, end, decoration) in &decoration_points {
        let Some(block) = decoration.options.block else {
            continue;
        };
        let first = view_lines.first_view_line_of_row(start.row);
        let end_line = view_lines.first_view_line_of_row(end.row + 1);
        if end_line <= first {
            continue;
        }
        let top = if first == 0 {
            px(0.)
        } else {
            layout.zones.line_y(first - 1, line_height)
                + layout.zones.height(first - 1, line_height)
        };
        let bottom = layout.zones.line_y(end_line, line_height);
        let [padding_top, padding_right, padding_bottom, padding_left] = block.padding;
        let text = layout.text_bounds;
        block_borders.push((
            Bounds::from_corners(
                point(
                    text.left() - padding_left,
                    bounds.top() + top - layout.scroll_top - padding_top,
                ),
                point(
                    text.right() + padding_right,
                    bounds.top() + bottom - layout.scroll_top + padding_bottom,
                ),
            ),
            block.border,
            block.border_width,
        ));
    }
    let mut decoration_underlines: Vec<(gpui::Point<Pixels>, Pixels, UnderlineStyle)> = Vec::new();
    // Monaco `nativeEditContext.css`: high contrast themes border the IME
    // composition all round rather than underlining it.
    let composition_boxes: Vec<Bounds<Pixels>> = match &marked {
        Some(range) if editor.theme.high_contrast => layout
            .lines
            .iter()
            .filter_map(|line| {
                let (from, to) = decoration_x_range(line, range.start, range.end)?;
                (to > from).then(|| {
                    Bounds::new(
                        point(
                            layout.text_origin_x() + from,
                            layout.line_top(line.view_line),
                        ),
                        size(to - from, layout.line_height_of(line.view_line)),
                    )
                })
            })
            .collect(),
        _ => Vec::new(),
    };
    let mut gutter_bars: Vec<(Bounds<Pixels>, Hsla)> = Vec::new();
    let mut margin_backgrounds: Vec<(Bounds<Pixels>, Hsla)> = Vec::new();
    let options_line_decorations_width = line_decorations_width;
    let mut glyphs: Vec<(gpui::Point<Pixels>, ShapedLine)> = Vec::new();
    let mut glyph_requests: Vec<GlyphRequest> = Vec::new();
    let mut line_number_colors: Vec<(u32, Hsla)> = Vec::new();
    let origin_x = layout.text_origin_x();
    for (start, end, decoration) in &decoration_points {
        let options = &decoration.options;
        // Monaco `_renderNormalDecorations`: with `showIfCollapsed`, a range
        // ending at the start of a line ends at the end of the line before.
        let end = &if options.show_if_collapsed && end.column == 0 && end.row > start.row {
            PointUtf16::new(end.row - 1, snapshot.line_len_utf16(end.row - 1))
        } else {
            *end
        };
        for line in layout.lines.iter() {
            if line.row < start.row || line.row > end.row {
                continue;
            }
            let top = layout.line_top(line.view_line);
            let height = layout.line_height_of(line.view_line);
            let text_top = layout.text_top(line.view_line);
            // A marker's `editorError.background` and the like.
            let background = options.background.or_else(|| {
                options
                    .severity
                    .and_then(|severity| colors.marker_background(severity))
            });
            if background.is_some() || options.border.is_some() {
                let range = if options.is_whole_line {
                    Some((px(0.) - layout.scroll_left, text_width + layout.scroll_left))
                } else {
                    // Monaco `shouldFillLineOnLineBreak`: a range going on
                    // past the line break fills to the line's end.
                    decoration_x_range(line, *start, *end).map(|(from, to)| {
                        if options.should_fill_line_on_line_break && end.row > line.row {
                            (from, to.max(text_width + layout.scroll_left))
                        } else {
                            (from, to)
                        }
                    })
                };
                if let Some((mut from, mut to)) = range {
                    if options.show_if_collapsed && !options.is_whole_line {
                        (from, to) = widen_collapsed(from, to, metrics.em_width);
                    }
                    // Monaco gives empty ranges (e.g. a snippet's final tab
                    // stop) a 2px minimum width.
                    let bounds = Bounds::new(
                        point(origin_x + from, top),
                        size((to - from).max(px(2.)), height),
                    );
                    decoration_quads.push((bounds, background, options.border));
                }
            }
            // Monaco's `border-left` on an inline decoration: at its start.
            if let Some(border) = options.left_border
                && line.contains(*start)
            {
                let x = origin_x + line.x_for_column(start.column) + border.offset;
                decoration_quads.push((
                    Bounds::new(point(x, top), size(border.width, height)),
                    Some(border.color),
                    None,
                ));
            }
            // Monaco `squiggly-unnecessary`: a themed underline for unused
            // code without a squiggle.
            // A marker's squiggle is in the theme's color.
            let underline = options.underline.map(|underline| match options.severity {
                Some(severity) => crate::decorations::DecorationUnderline {
                    color: colors.marker_color(severity),
                    ..underline
                },
                None => underline,
            });
            let underline = underline.or_else(|| {
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
            // Monaco `firstLineDecorationClassName`: in the line decorations
            // area too, on a wrapped line's first view line only.
            let first_line = options
                .first_line_decoration
                .as_ref()
                .filter(|_| line.columns.start == 0);
            let bars = options
                .gutter_bar
                .into_iter()
                .chain(first_line.and_then(|first| first.bar));
            for color in bars {
                let left = bounds.left() + glyph_width + numbers_width + px(2.);
                gutter_bars.push((Bounds::new(point(left, top), size(px(3.), height)), color));
            }
            if let Some(color) = options.margin_background {
                margin_backgrounds.push((
                    Bounds::new(point(bounds.left(), top), size(gutter_width, height)),
                    color,
                ));
            }
            let texts = options
                .lines_decoration_text
                .iter()
                .chain(first_line.and_then(|first| first.text.as_ref()));
            for (text, color) in texts {
                // Codicons (the diff editor's `add` / `remove` signs) are
                // 11px, as Monaco's `.insert-sign` / `.delete-sign`.
                let (text, font) = crate::codicons::decoration_text(text, cx);
                let (font, font_size) = match font {
                    Some(font) => (font, px(11.)),
                    None => (metrics.font.clone(), metrics.font_size),
                };
                let shaped = window.text_system().shape_line(
                    text.clone(),
                    font_size,
                    &[run(text.len(), &font, *color)],
                    None,
                );
                let area = options_line_decorations_width;
                let x = bounds.left() + glyph_width + numbers_width + (area - shaped.width) / 2.;
                glyphs.push((point(x, text_top), shaped));
            }
            if line.columns.start == 0
                && let Some(color) = options.line_number_color
            {
                line_number_colors.push((line.row, color));
            }
            // Monaco `_collectDecorationBasedGlyphRenderRequest`: on every
            // view line of the range.
            if let Some((glyph, color)) = &options.glyph
                && segment_intersects(line, *start, *end)
            {
                let lane = options.glyph_margin.unwrap_or_default().position;
                glyph_requests.push(GlyphRequest {
                    view_line: line.view_line,
                    lane_index: glyph_lanes.lane_index(line.row, lane),
                    z_index: options.z_index,
                    glyph: Some((glyph.clone(), *color)),
                    widget: None,
                });
            }
        }
    }
    // Inlay hints follow the viewport.
    if let (Some(first), Some(last)) = (layout.lines.first(), layout.lines.last()) {
        editor.inlay_hints_viewport(first.row..last.row + 1, cx);
    }

    // The code action lightbulb (Monaco `LightBulbWidget`).
    let mut lightbulb = None;
    let mut lightbulb_title = None;
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
            let icon = crate::code_action::lightbulb_icon(&bulb);
            (
                bounds,
                crate::code_action::lightbulb_color(&colors, icon),
                icon,
            )
        });
        // Monaco's `LightBulbWidget` title, with the Auto Fix and Quick Fix
        // keys; the gutter lightbulb (a glyph decoration) has none.
        let in_gutter = matches!(
            placement,
            Some(crate::code_action::LightbulbPlacement::Gutter(_)) if glyph_width > px(0.)
        );
        // Only under the mouse, where the title can show.
        let hovered = lightbulb
            .as_ref()
            .is_some_and(|(bounds, _, _)| bounds.contains(&window.mouse_position()));
        if hovered && !in_gutter {
            let keys = |action: &dyn gpui::Action| {
                crate::keybinding_label::command_label(action, None, &editor.focus_handle, window)
            };
            let preferred = keys(&crate::actions::AutoFix);
            let quick_fix = keys(&crate::actions::QuickFix);
            lightbulb_title = Some(SharedString::from(crate::code_action::lightbulb_title(
                &bulb,
                preferred.as_deref(),
                quick_fix.as_deref(),
            )));
        }
    }
    layout.lightbulb = lightbulb.map(|(bounds, _, _)| bounds);

    // Monaco `DecorationsOverviewRuler`: a canvas under the vertical
    // scrollbar, between its arrows (`layoutInfo.overviewRuler`), mapping
    // content offsets by `canvasHeight / scrollHeight` so marks line up
    // with the slider.
    let overview_ruler = {
        let position = layout.layout_info.overview_ruler;
        (options.overview_ruler_lanes > 0 && position.width > px(0.)).then(|| {
            use crate::view::overview_ruler as ruler;
            let bounds = Bounds::new(
                point(
                    bounds.right() - position.right - position.width,
                    bounds.top() + position.top,
                ),
                size(position.width, position.height),
            );
            let pixel_ratio = window.scale_factor();
            let geometry = ruler::RulerGeometry::new(
                f32::from(position.width),
                f32::from(position.height),
                pixel_ratio,
                // Monaco `getScrollHeight`: at least the viewport.
                f32::from(content_height.max(viewport_height)),
                f32::from(line_height),
                options.overview_ruler_lanes,
            );
            let offset_of = |line: u32| f32::from(zones.line_y(line, line_height));
            let show_validation = editor.shows_validation_decorations();
            let marker_colors = [
                colors.overview_ruler_error,
                colors.overview_ruler_warning,
                colors.overview_ruler_info,
            ];
            let model = editor.model().clone();
            // Theme color ids resolve against the current theme.
            let theme = editor.theme.clone();
            let key = (
                model.entity_id(),
                model.read(cx).decorations_version(),
                view_lines.generation(),
                show_validation,
                marker_colors,
                std::sync::Arc::as_ptr(&theme) as usize,
            );
            let mut cache = std::mem::take(&mut editor.overview_ruler_cache);
            // Monaco `getAllOverviewRulerDecorations`: grouped by color, in
            // view lines.
            let groups = cache.groups(key, || {
                let decorations =
                    model.update(cx, |model, _| model.all_decorations(Some(editor_id)));
                let mut groups = ruler::DecorationGroups::default();
                for decoration in decorations {
                    if !show_validation && decoration.options.validation {
                        continue;
                    }
                    let Some(ruler) = &decoration.options.overview_ruler else {
                        continue;
                    };
                    let lane = ruler.position;
                    // A marker's mark is in the theme's color; a theme
                    // color id is looked up in the theme.
                    let color = match decoration.options.severity {
                        Some(severity) => colors
                            .marker_overview_color(severity)
                            .or_else(|| ruler.resolve(&theme)),
                        None => ruler.resolve(&theme),
                    };
                    let Some(color) = color else {
                        continue;
                    };
                    let view_line = |offset: usize| {
                        view_lines
                            .to_view(snapshot.offset_to_point_utf16(offset), false)
                            .0
                    };
                    groups.accept(
                        color,
                        decoration.options.z_index,
                        view_line(decoration.range.start),
                        view_line(decoration.range.end),
                        ruler::lane_bits(lane),
                    );
                }
                groups.into_sorted()
            });
            let marks = cache.marks(&groups, &zones, &geometry, offset_of);
            // The cursors at 0.7 alpha: `editorCursor.foreground`, or
            // `editorMultiCursor.primary/secondary.foreground` with several,
            // sorted by position.
            let cursors = if options.hide_cursor_in_overview_ruler {
                Vec::new()
            } else {
                let mut cursors: Vec<(PointUtf16, Hsla)> = cursor_views
                    .iter()
                    .enumerate()
                    .map(|(ix, view)| {
                        let color = if cursor_views.len() == 1 {
                            colors.cursor
                        } else if ix == 0 {
                            colors.multi_cursor_primary_foreground
                        } else {
                            colors.multi_cursor_secondary_foreground
                        };
                        (*view, color.opacity(0.7))
                    })
                    .collect();
                cursors.sort_by_key(|(view, _)| *view);
                let cursors: Vec<(u32, Hsla)> = cursors
                    .into_iter()
                    .map(|(view, color)| (view.row, color))
                    .collect();
                ruler::render_cursors(&cursors, &geometry, offset_of)
            };
            let paint = ruler::RulerPaint {
                bounds,
                pixel_ratio,
                // `editorOverviewRuler.background`, else the editor's with
                // the minimap on the right.
                background: colors.overview_ruler_background.or((options.minimap.enabled
                    && options.minimap.side == crate::options::MinimapSide::Right)
                    .then_some(colors.background)),
                canvas_size: (geometry.canvas_width, geometry.canvas_height),
                marks,
                cursors,
                border: options
                    .overview_ruler_border
                    .then_some(colors.overview_ruler_border),
            };
            cache.painted = Some(paint.clone());
            editor.overview_ruler_cache = cache;
            paint
        })
    };

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

    // Cursors (Monaco `ViewCursors`), at their view positions: a cursor
    // moved to the end of a wrapped segment stays there.
    let cursor_style = if editor.overtype {
        options.overtype_cursor_style
    } else {
        options.cursor_style
    };
    let em_width = metrics.em_width;
    // `editorCursor.*`, or `editorMultiCursor.primary.*`/`secondary.*` with
    // several cursors; the character inside is in the background color,
    // else the caret's opposite.
    let cursor_colors = |ix: usize| {
        let (foreground, background) = if selections.len() == 1 {
            (colors.cursor, colors.cursor_background)
        } else if ix == 0 {
            (
                colors.multi_cursor_primary_foreground,
                colors
                    .multi_cursor_primary_background
                    .or(colors.cursor_background),
            )
        } else {
            (
                colors.multi_cursor_secondary_foreground,
                colors
                    .multi_cursor_secondary_background
                    .or(colors.cursor_background),
            )
        };
        (
            foreground,
            background.unwrap_or_else(|| opposite(foreground)),
        )
    };
    let rope = snapshot.rope();
    let cursor_injected = editor.cursor_injected_offsets();
    let mut cursor_view_lines = Vec::with_capacity(selections.len());
    // (bounds, the character inside, colors) of each cursor on screen.
    let mut cursor_targets = Vec::with_capacity(selections.len());
    let mut cursor_parts = Vec::with_capacity(selections.len());
    for (ix, (selection, view)) in selections.iter().zip(&cursor_views).enumerate() {
        cursor_view_lines.push(view.row);
        let Some(line) = layout.view_line(view.row) else {
            cursor_targets.push(None);
            cursor_parts.push(None);
            continue;
        };
        let column = selection
            .head
            .column
            .clamp(line.columns.start, line.columns.end);
        // Monaco's view column: before injected text there, or as far into
        // it as the cursor stops let a move or click take it.
        let injected = cursor_injected.get(ix).copied().unwrap_or(0);
        let x = layout.text_origin_x() + line.x_for_caret(column, injected);
        let y = layout.line_top(line.view_line);
        // Monaco's cursor fills a taller line.
        let line_height = layout.line_height_of(line.view_line);
        // Monaco `_getGraphemeAwarePosition`: the character after the
        // cursor on its view line, if any.
        let next = (column < line.columns.end).then(|| {
            let ix = (column - line.columns.start) as usize;
            // A surrogate pair's halves display at the same index.
            let len = if line.display_index.get(ix + 1) == line.display_index.get(ix) {
                2
            } else {
                1
            };
            let end = (column + len).min(line.columns.end);
            let is_tab = rope
                .chars_at(rope.point_utf16_to_offset(PointUtf16::new(line.row, column)))
                .next()
                == Some('\t');
            (end, is_tab)
        });
        // Monaco: a tab, the line end or a zero-width character are a
        // half-width character wide.
        let char_width = match next {
            Some((end, false)) => {
                let width = line.x_for_column(end) - line.x_for_column(column);
                if width < px(1.) { em_width } else { width }
            }
            _ => em_width,
        };
        // Monaco `cursorHeight` for the line styles, centered.
        let line_cursor_height = if options.cursor_height > px(0.) {
            options.cursor_height.min(line_height)
        } else {
            line_height
        };
        let line_y = y + (line_height - line_cursor_height) / 2.;
        let mut shows_character = false;
        let bounds = match cursor_style {
            CursorStyle::Line => {
                let width = options.line_cursor_width(em_width);
                // Monaco draws the character inside a bar wider than 2px.
                shows_character = width > px(2.);
                // Monaco shifts a bar of 2px or more 1px left, between the
                // characters (`paddingLeft`).
                let left = if width >= px(2.) && x - layout.text_origin_x() >= px(1.) {
                    x - px(1.)
                } else {
                    x
                };
                Bounds::new(point(left, line_y), size(width, line_cursor_height))
            }
            CursorStyle::LineThin => {
                Bounds::new(point(x, line_y), size(px(1.), line_cursor_height))
            }
            CursorStyle::Block => {
                shows_character = true;
                Bounds::new(point(x, y), size(char_width, line_height))
            }
            CursorStyle::BlockOutline => Bounds::new(point(x, y), size(char_width, line_height)),
            CursorStyle::Underline => {
                Bounds::new(point(x, y + line_height - px(2.)), size(char_width, px(2.)))
            }
            // Monaco: a 2px box with a 1px bottom border.
            CursorStyle::UnderlineThin => {
                Bounds::new(point(x, y + line_height - px(1.)), size(char_width, px(1.)))
            }
        };
        // The character, drawn in the cursor's background color (not a tab,
        // which shows nothing).
        let glyph = next
            .filter(|(_, is_tab)| shows_character && !is_tab)
            .map(|(end, _)| {
                let start = line.display_index[(column - line.columns.start) as usize];
                let end = line.display_index[(end - line.columns.start) as usize];
                (line.view_line, start..end)
            });
        cursor_targets.push(Some(bounds));
        cursor_parts.push(Some((glyph, cursor_colors(ix))));
    }
    // Monaco `cursorSmoothCaretAnimation`: every cursor slides, in content
    // coordinates so scrolling doesn't animate them.
    let offset = point(
        layout.text_bounds.left() - scroll_left,
        bounds.top() - scroll_top,
    );
    let content: Vec<Option<Bounds<Pixels>>> = cursor_targets
        .iter()
        .map(|target| target.map(|b| Bounds::new(b.origin - offset, b.size)))
        .collect();
    let enabled =
        options.cursor_smooth_caret_animation != crate::options::CursorSmoothCaretAnimation::Off;
    let (shown, animating) = editor.animation.carets(&content, enabled);
    if animating {
        window.request_animation_frame();
    }
    let mut cursors = Vec::with_capacity(cursor_parts.len());
    for ((shown, target), parts) in shown.into_iter().zip(&cursor_targets).zip(cursor_parts) {
        let (Some(shown), Some(target), Some((glyph, (color, text_color)))) =
            (shown, target, parts)
        else {
            continue;
        };
        let bounds = Bounds::new(shown.origin + offset, shown.size);
        cursors.push(CursorPaint {
            bounds,
            color,
            text_color,
            glyph: glyph.map(|(line, range)| (line, range, bounds.origin - target.origin)),
        });
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
        let highlight_active = options.guides.highlight_active_indentation;
        let active = (options.guides.indentation
            && highlight_active != crate::options::HighlightActiveIndentation::Off)
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
        // Monaco `_maxIndentLeft`: the wrapping column's x.
        let max_indent_left = wrap_column.map(|column| metrics.em_width * column as f32);
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
                    // `editorBracketPairGuide.*`, else the bracket color.
                    let Some((inactive, active)) = colors.bracket_pair_guide_colors(guide.level)
                    else {
                        return;
                    };
                    let color = if guide.active { active } else { inactive };
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
                    // Monaco `_maxIndentLeft`: none past the wrapping column.
                    if max_indent_left.is_none_or(|max| left - origin_x <= max) {
                        guides.push((bounds, color));
                    }
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
                // Monaco: lines with bracket guides don't highlight the
                // active indent guide, unless `always`.
                let is_active = (in_line.is_empty()
                    || highlight_active == crate::options::HighlightActiveIndentation::Always)
                    && active.is_some_and(|a| {
                        a.indent == level && a.start_row <= line.row && line.row <= a.end_row
                    });
                // `editorIndentGuide.background*` by level.
                let Some((inactive, active)) = colors.indent_guide_colors(level - 1) else {
                    continue;
                };
                let color = if is_active { active } else { inactive };
                let x = origin_x + metrics.em_width * ((level - 1) * ctx.indent_size) as f32;
                if max_indent_left.is_some_and(|max| x - origin_x > max) {
                    continue;
                }
                guides.push((Bounds::new(point(x, top), size(px(1.), line_height)), color));
            }
            for guide in queue {
                draw_bracket_guide(guide, &mut guides);
            }
        }
    }
    // Monaco `Rulers`: above the content overlays (selections, guides,
    // decorations), below the text.
    let mut rulers = Vec::new();
    for ruler in &options.rulers {
        let x = layout.text_origin_x() + metrics.em_width * ruler.column as f32;
        rulers.push((
            Bounds::new(
                point(x, text_bounds.top()),
                size(px(1.), text_bounds.size.height),
            ),
            ruler.color.unwrap_or(colors.ruler),
        ));
    }

    // Rendered whitespace (Monaco `renderWhitespace`, drawn as
    // `WhitespaceOverlay`'s default SVG: a dot per space, an arrow per tab).
    let whitespace =
        (options.render_whitespace != crate::options::RenderWhitespace::None).then(|| {
            let rope = snapshot.rope();
            let mut marks = Vec::new();
            let mut text = String::new();
            let mut text_row = None;
            for line in &layout.lines {
                if text_row != Some(line.row) {
                    text = rope.line(line.row);
                    text_row = Some(line.row);
                }
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
                let segment = &text[utf16_to_byte(&text, line.columns.start)
                    ..utf16_to_byte(&text, line.columns.end)];
                let faux_indent = (line.indent_x / metrics.space_width).round() as u32;
                let top = layout.text_top(line.view_line);
                let columns = crate::view::guides::whitespace_columns(
                    segment,
                    line.columns.start,
                    faux_indent,
                    !line.is_last_segment,
                    options.render_whitespace,
                    &selected,
                );
                // The tabs among them, by offset in the segment.
                let mut tabs = segment
                    .encode_utf16()
                    .enumerate()
                    .filter(|(_, c)| *c == b'\t' as u16)
                    .map(|(ix, _)| ix as u32);
                let mut next_tab = tabs.next();
                for column in columns {
                    let offset = column - line.columns.start;
                    while next_tab.is_some_and(|tab| tab < offset) {
                        next_tab = tabs.next();
                    }
                    let is_tab = next_tab == Some(offset);
                    let x = layout.text_origin_x() + line.x_for_column(column);
                    marks.push((point(x, top), is_tab));
                }
            }
            marks
        });

    // Monaco `WordWrapIndicatorOverlay`: `↩` at the wrapping column of each
    // view line that continues on the next one.
    let wrap_indicators = wrap_column
        .filter(|_| options.word_wrap_indicator)
        .map(|column| {
            let text = "\u{21a9}";
            let glyph = window.text_system().shape_line(
                text.into(),
                metrics.font_size,
                &[run(text.len(), &metrics.font, colors.word_wrap_indicator)],
                None,
            );
            let left = metrics.em_width * column as f32;
            let lines: Vec<&LineLayout> = layout
                .lines
                .iter()
                .filter(|line| !line.is_last_segment)
                .collect();
            layout.wrap_indicators = lines.iter().map(|line| (line.row, left)).collect();
            let marks = lines
                .iter()
                .map(|line| {
                    point(
                        layout.text_origin_x() + left,
                        layout.text_top(line.view_line),
                    )
                })
                .collect();
            (glyph, marks)
        });

    // Gutter numbers: on the first view line of each row.
    let mut gutter_numbers = Vec::new();
    if options.line_numbers != LineNumbers::Off {
        let right = bounds.left() + glyph_width + numbers_width;
        for line in layout.lines.iter().filter(|line| line.columns.start == 0) {
            let row = line.row;
            // Monaco `LineNumbersOverlay`: only the primary cursor's line is
            // active.
            let is_active = row == primary.row;
            // Monaco `renderFinalNewline`: the last view line, when empty.
            let is_final_newline =
                line.view_line + 1 == view_line_count && snapshot.line_len(row) == 0;
            if is_final_newline
                && options.render_final_newline == crate::options::RenderFinalNewline::Off
            {
                continue;
            }
            // Monaco `_getLineRenderLineNumber`.
            let label = match &options.line_numbers {
                LineNumbers::Custom(render) => render.render(row + 1).to_string(),
                LineNumbers::Relative if !is_active => primary.row.abs_diff(row).to_string(),
                LineNumbers::Interval
                    if !is_active && (row + 1) % 10 != 0 && row + 1 != snapshot.line_count() =>
                {
                    continue;
                }
                _ => (row + 1).to_string(),
            };
            if label.is_empty() {
                continue;
            }
            let color = line_number_colors
                .iter()
                .find(|(r, _)| *r == row)
                .map(|(_, color)| *color)
                .unwrap_or(if is_active {
                    colors.active_line_number
                } else {
                    colors.line_number
                });
            // Monaco's `dimmed-line-number`: `editorLineNumber.dimmedForeground`,
            // else the line number color at 40%, over the active color.
            let color = if is_final_newline
                && options.render_final_newline == crate::options::RenderFinalNewline::Dimmed
            {
                colors
                    .line_number_dimmed
                    .unwrap_or(colors.line_number.opacity(0.4))
            } else {
                color
            };
            let shaped = window.text_system().shape_line(
                label.clone().into(),
                metrics.font_size,
                &[run(label.len(), &metrics.font, color)],
                None,
            );
            // Right-aligned, but for the relative mode's current line
            // (`.relative-current-line-number`).
            let x = if options.line_numbers == LineNumbers::Relative && is_active {
                right - numbers_width
            } else {
                right - shaped.width
            };
            layout.line_number_styles.push((row, x, color));
            gutter_numbers.push((layout.text_top(line.view_line), shaped, x));
            layout.line_number_labels.push((row, label));
        }
    }

    // Embedded views; their elements are built after this update returns so
    // host views may read the editor while rendering.
    let mut embedded = Vec::new();
    let mut zone_specs = zone_specs;
    // Monaco `LinesLayout`: zones after the same view line by ordinal, ties
    // in insertion order (a stable sort).
    zone_specs.sort_by_key(|zone| (zone.after, zone.ordinal));
    // Monaco's zones layer is in the scrolled `linesContent`, as wide as
    // the scroll width (`ViewZones.render`); the margin part is the
    // gutter's width and doesn't scroll horizontally.
    let zone_width = layout.content_width.max(text_width);
    let margin_width = text_bounds.left() - layout.bounds.left();
    let mut ix = 0;
    while ix < zone_specs.len() {
        let after = zone_specs[ix].after;
        let group_end = ix
            + zone_specs[ix..]
                .iter()
                .take_while(|z| z.after == after)
                .count();
        let group_height = zone_specs[ix..group_end]
            .iter()
            .map(|zone| zone.height)
            .fold(px(0.), |a, b| a + b);
        let mut top = layout.bounds.top() + zones.line_y((after + 1) as u32, line_height)
            - group_height
            - scroll_top;
        // Only zones on screen are rendered.
        if top > layout.bounds.bottom() || top + group_height < layout.bounds.top() {
            ix = group_end;
            continue;
        }
        for zone in &zone_specs[ix..group_end] {
            // Zone widgets (peek, the problem zone) are overlay widgets in
            // Monaco: they don't scroll horizontally.
            let bounds = if zone.full_width {
                Bounds::from_corners(
                    point(layout.bounds.left(), top),
                    point(text_bounds.left() + text_width, top + zone.height),
                )
            } else if zone.captures_wheel {
                Bounds::new(
                    point(text_bounds.left(), top),
                    size(text_width, zone.height),
                )
            } else {
                Bounds::new(
                    point(text_bounds.left() - scroll_left, top),
                    size(zone_width, zone.height),
                )
            };
            embedded.push(Embedded::Zone {
                view: zone.view.clone(),
                bounds,
                full_width: zone.full_width,
                captures_wheel: zone.captures_wheel,
            });
            if let Some(margin) = &zone.margin_view
                && !zone.full_width
                && margin_width > px(0.)
            {
                embedded.push(Embedded::ZoneMargin {
                    view: margin.clone(),
                    bounds: Bounds::new(
                        point(layout.bounds.left(), top),
                        size(margin_width, zone.height),
                    ),
                });
            }
            top += zone.height;
        }
        ix = group_end;
    }
    {
        let model = editor.model().clone();
        let buffer = model.read(cx).buffer();
        // Monaco glyph margin widgets: on their line's first view line.
        let glyph_width = layout.glyph_margin_width;
        if glyph_width > px(0.) {
            for (ix, widget) in editor.widgets.glyphs.iter().enumerate() {
                let row = buffer.offset_to_point(buffer.resolve(&widget.anchor)).row;
                let Some(line) = layout
                    .lines
                    .iter()
                    .find(|line| line.row == row && line.columns.start == 0)
                else {
                    continue;
                };
                glyph_requests.push(GlyphRequest {
                    view_line: line.view_line,
                    lane_index: glyph_lanes.lane_index(row, widget.lane),
                    z_index: widget.z_index,
                    glyph: None,
                    widget: Some(ix),
                });
            }
        }
        // Monaco `GlyphMarginWidgets.prepareRender`: one glyph per line and
        // lane, the highest z-index's, widgets first; a lane is a line
        // height wide.
        let lane_width = (glyph_width / glyph_lanes.required_lanes() as f32).round();
        let mut glyphs_drawn = Vec::new();
        for request in resolve_glyph_requests(glyph_requests) {
            let Some(line) = layout.view_line(request.view_line) else {
                continue;
            };
            let left = layout.bounds.left() + line_height * request.lane_index as f32;
            glyphs_drawn.push((request.view_line, request.lane_index));
            match (request.widget, request.glyph) {
                (Some(ix), _) => embedded.push(Embedded::Glyph {
                    view: editor.widgets.glyphs[ix].view.clone(),
                    bounds: Bounds::new(
                        point(left, layout.line_top(line.view_line)),
                        size(lane_width, layout.line_height_of(line.view_line)),
                    ),
                }),
                (None, Some((glyph, color))) => {
                    let (glyph, font) = crate::codicons::decoration_text(&glyph, cx);
                    let font = font.unwrap_or_else(|| metrics.font.clone());
                    let shaped = window.text_system().shape_line(
                        glyph.clone(),
                        metrics.font_size,
                        &[run(glyph.len(), &font, color)],
                        None,
                    );
                    let x = left + (lane_width - shaped.width) / 2.;
                    glyphs.push((point(x, layout.text_top(line.view_line)), shaped));
                }
                (None, None) => {}
            }
        }
        layout.glyphs_drawn = glyphs_drawn;
        // Widgets across the whole gutter (the inline edit indicator), over
        // its glyphs, whenever there is a gutter (Monaco's
        // `InlineEditsGutterIndicator` overlay).
        let info = &layout.layout_info;
        let gutter_width = info.decorations_left + info.decorations_width - info.glyph_margin_left;
        if gutter_width > px(0.) {
            for widget in &editor.widgets.gutter {
                let row = buffer.offset_to_point(buffer.resolve(&widget.anchor)).row;
                let Some(line) = layout
                    .lines
                    .iter()
                    .find(|line| line.row == row && line.columns.start == 0)
                else {
                    continue;
                };
                embedded.push(Embedded::Glyph {
                    view: widget.view.clone(),
                    bounds: Bounds::new(
                        point(
                            layout.bounds.left() + info.glyph_margin_left,
                            layout.line_top(line.view_line),
                        ),
                        size(gutter_width, layout.line_height_of(line.view_line)),
                    ),
                });
            }
        }
        // Monaco `ContentWidget._getAnchorsCoordinates`: hidden without a
        // position or preference, or when its line isn't laid out.
        for widget in &editor.widgets.content {
            // Against a box in the window (Monaco's `IAnchor`): below or
            // above it as below or above a line.
            if let Some(rect) = widget.window_anchor.filter(|_| widget.allow_overflow) {
                embedded.push(Embedded::Content {
                    view: widget.view.clone(),
                    anchor: rect.origin,
                    secondary_x: None,
                    line_height: rect.size.height,
                    preference: widget.preference.clone(),
                    overflow: Some((widget.layer, widget.room.clone())),
                });
                continue;
            }
            let Some(anchor) = widget
                .anchor
                .as_ref()
                .filter(|_| !widget.preference.is_empty())
            else {
                continue;
            };
            let affinity = widget.affinity.unwrap_or_default();
            let position = buffer.offset_to_point_utf16(buffer.resolve(anchor));
            let Some(line) = layout.line_for_point_with_affinity(position, affinity) else {
                continue;
            };
            // Left-aligned before injected text at a line's start.
            let x_for = |line: &LineLayout, column: u32| {
                if column == 0 && affinity == crate::widgets::PositionAffinity::LeftOfInjectedText {
                    layout.text_origin_x()
                } else {
                    layout.text_origin_x() + line.x_for_column(column)
                }
            };
            let x = x_for(line, position.column);
            // The secondary position counts only on the same view line.
            let secondary_x = widget.secondary.as_ref().and_then(|secondary| {
                let secondary = buffer.offset_to_point_utf16(buffer.resolve(secondary));
                let secondary_line = layout.line_for_point_with_affinity(secondary, affinity)?;
                (secondary_line.view_line == line.view_line)
                    .then(|| x_for(secondary_line, secondary.column))
            });
            embedded.push(Embedded::Content {
                view: widget.view.clone(),
                anchor: point(x, layout.line_top(line.view_line)),
                secondary_x,
                line_height: layout.line_height_of(line.view_line),
                preference: widget.preference.clone(),
                overflow: widget
                    .allow_overflow
                    .then(|| (widget.layer, widget.room.clone())),
            });
        }
    }
    for widget in &editor.widgets.overlays {
        embedded.push(Embedded::Overlay {
            view: widget.view.clone(),
            position: widget.position,
            stack_ordinal: widget.stack_ordinal,
            overflow: widget.allow_overflow.then_some(widget.layer),
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
    layout.line_highlights = current_line_highlights(
        &layout,
        &options,
        &colors,
        editor.theme.high_contrast,
        &selections,
        &cursor_view_lines,
        focused,
    );
    layout.cursors = (
        cursors.iter().map(|cursor| cursor.bounds).collect(),
        cursor_style == CursorStyle::BlockOutline,
    );
    layout.gutter_bars = gutter_bars.clone();
    editor.layout = Some(layout.clone());
    editor.report_layout(cx);
    // Monaco `RenameWidget.afterRender(null)`: the box's line scrolled out
    // of view, so rename is cancelled.
    if editor.rename_out_of_view(cx) {
        let entity = cx.entity();
        window.defer(cx, move |_, cx| {
            entity.update(cx, |editor, cx| {
                if editor.rename_out_of_view(cx) {
                    editor.cancel_rename(true, cx);
                }
            });
        });
    }
    // Shift+F10 revealed the cursor: the menu goes below it now.
    if editor.context_menu.show_at_cursor {
        let entity = cx.entity();
        window.defer(cx, move |window, cx| {
            entity.update(cx, |editor, cx| {
                editor.show_pending_context_menu(window, cx)
            });
        });
    }
    PrepaintState {
        layout,
        selections,
        cursors,
        cursor_outline: cursor_style == CursorStyle::BlockOutline,
        gutter_numbers,
        fold_placeholders,
        fold_highlights,
        fold_controls_opacity,
        decoration_quads,
        block_borders,
        decoration_underlines,
        composition_boxes,
        gutter_bars,
        margin_backgrounds,
        glyphs,
        lightbulb,
        lightbulb_title,
        lightbulb_tooltip: None,
        guides,
        rulers,
        whitespace,
        wrap_indicators,
        embedded,
        placeholder,
        dnd_target,
        minimap_stale: std::mem::take(&mut editor.minimap_cache.stale),
        sticky: sticky_shaped,
        sticky_height,
        sticky_focused: editor.sticky.focused,
        sticky_hovered: editor.sticky.hovered,
        // Monaco: hovering the sticky gutter shows every folding control
        // there with `mouseover`; else the collapsed ones (and all with
        // `always`).
        sticky_show_fold_controls: editor.sticky.gutter_hovered
            || options.show_folding_controls == ShowFoldingControls::Always,
        numbers_right,
        zone_elements: Vec::new(),
        full_width_zone_elements: Vec::new(),
        zone_margin_elements: Vec::new(),
        content_elements: Vec::new(),
        glyph_elements: Vec::new(),
        fold_tooltip: None,
        overlay_elements: Vec::new(),
        hitbox: None,
        cursor_areas: Vec::new(),
        cursor_areas_enabled: editor.dnd.target.is_none() && !editor.dnd.copy,
        drag_cursor: editor.drag_mouse_cursor(),
        overflow_bounds: Vec::new(),
        widgets_keeping_focus: Vec::new(),
        colors,
        high_contrast: editor.theme.high_contrast,
        options,
        focused,
        cursor_visible: editor.cursor_visible,
        cursor_opacity,
        cursor_scale,
        middle_scroll_anchor: editor.animation.middle_scroll.map(|s| s.anchor),
        mouse_position: editor.animation.scrollbars.mouse_position,
        scrollbar_looks,
        overview_ruler,
    }
}

fn paint_editor(state: &mut PrepaintState, window: &mut Window, cx: &mut App) {
    let mut zone_elements = std::mem::take(&mut state.zone_elements);
    let mut full_width_zone_elements = std::mem::take(&mut state.full_width_zone_elements);
    let mut zone_margin_elements = std::mem::take(&mut state.zone_margin_elements);
    let mut content_elements = std::mem::take(&mut state.content_elements);
    let mut glyph_elements = std::mem::take(&mut state.glyph_elements);
    let mut fold_tooltip = state.fold_tooltip.take();
    let mut lightbulb_tooltip = state.lightbulb_tooltip.take();
    let mut overlay_elements = std::mem::take(&mut state.overlay_elements);
    let state = &*state;
    let layout = &state.layout;
    let colors = &state.colors;
    let bounds = layout.bounds;
    let line_height = layout.metrics.line_height;

    window.paint_quad(fill(bounds, colors.background));
    let gutter = Bounds::new(bounds.origin, size(layout.gutter_width, bounds.size.height));
    window.paint_quad(fill(gutter, colors.gutter_background));

    window.with_content_mask(Some(ContentMask { bounds }), |window| {
        paint_current_line_highlight(state, window);
        for (bounds, color) in &state.margin_backgrounds {
            window.paint_quad(fill(*bounds, *color));
        }
        // Monaco `foldingHighlight` (`editor.foldBackground`).
        for highlight in &state.fold_highlights {
            window.paint_quad(fill(*highlight, colors.fold_background));
        }

        // Gutter numbers.
        for (top, shaped, x) in &state.gutter_numbers {
            let origin = point(*x, *top);
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
            && let Some((bulb, color, icon)) = state.lightbulb
        {
            paint_lightbulb(bulb, color, icon, window, cx);
        }
        if let Some(element) = &mut lightbulb_tooltip {
            element.paint(window, cx);
        }

        // Fold controls (`editorGutter.foldingControlForeground`, 140% of
        // the font size): always for collapsed regions, otherwise per
        // option.
        for control in &layout.fold_controls {
            let opacity = if control.collapsed {
                1.
            } else {
                state.fold_controls_opacity
            };
            if opacity > 0. {
                paint_chevron(
                    control,
                    colors.folding_control_foreground.opacity(opacity),
                    layout.metrics.font_size * 1.4,
                    window,
                    cx,
                );
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
                // Monaco's content overlays, bottom to top (`view.ts`):
                // selections, indent guides, decorations, whitespace and the
                // wrap indicator; then the rulers, the text and the cursors.
                // High contrast themes have square selections.
                paint_selections(
                    layout,
                    &state.selections,
                    origin_x,
                    selection_color,
                    colors.background,
                    state.options.rounded_selection && !state.high_contrast,
                    window,
                );
                for (bounds, color) in &state.guides {
                    window.paint_quad(fill(*bounds, *color));
                }
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
                if let Some(marks) = &state.whitespace {
                    paint_whitespace(marks, layout, colors.whitespace, window);
                }
                if let Some((glyph, marks)) = &state.wrap_indicators {
                    for origin in marks {
                        glyph
                            .paint(*origin, line_height, TextAlign::Left, None, window, cx)
                            .ok();
                    }
                }
                for (bounds, color) in &state.rulers {
                    window.paint_quad(fill(*bounds, *color));
                }

                // The view lines: injected text and control character
                // backgrounds, then the text.
                for line in &layout.lines {
                    let top = layout.text_top(line.view_line);
                    for range in &line.controls {
                        let from = line.shaped.x_for_index(range.start);
                        let to = line.shaped.x_for_index(range.end);
                        window.paint_quad(fill(
                            Bounds::new(
                                point(origin_x + line.indent_x + from, top),
                                size(to - from, line_height),
                            ),
                            CONTROL_CHARACTER_BACKGROUND,
                        ));
                    }
                    for injected in &line.injected {
                        let InjectedLayout {
                            range,
                            background,
                            swatch,
                            chip,
                            text_box,
                            ..
                        } = injected;
                        let from = line.shaped.x_for_index(range.start);
                        let to = line.shaped.x_for_index(range.end);
                        if let Some(swatch) = swatch {
                            // Monaco: a 0.8em square with a 0.1em border,
                            // centered in its 1.2em of room.
                            let side = (layout.metrics.font_size * 0.8).round();
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
                        let bounds = Bounds::new(
                            point(origin_x + line.indent_x + from, top + px(1.)),
                            size(to - from, line_height - px(2.)),
                        );
                        if *chip {
                            // Monaco `.mtkoverflow`: `button.background`
                            // (`button.hoverBackground` hovered), a 1px
                            // `contrastBorder` and a 2px radius.
                            let hovered = state.mouse_position.is_some_and(|m| bounds.contains(&m));
                            let background = if hovered {
                                colors.button_hover_background
                            } else {
                                background.unwrap_or(colors.button_background)
                            };
                            window.paint_quad(quad(
                                bounds,
                                Corners::all(px(2.)),
                                background,
                                Edges::all(if colors.contrast_border.is_some() {
                                    px(1.)
                                } else {
                                    px(0.)
                                }),
                                colors.contrast_border.unwrap_or(gpui::transparent_black()),
                                BorderStyle::Solid,
                            ));
                        } else if let Some(background) = background {
                            let (bounds, corners) = match text_box {
                                // Monaco's inline box: the text's height
                                // plus its padding, centered like the text.
                                Some(text_box) => {
                                    let x = injected.background_range(&line.shaped);
                                    let height = line.shaped.text_height(range.start)
                                        + text_box.padding_y * 2.;
                                    let (left, right) = text_box.corner_radius;
                                    (
                                        Bounds::new(
                                            point(
                                                origin_x + line.indent_x + x.start,
                                                top + (line_height - height) / 2.,
                                            ),
                                            size(x.end - x.start, height),
                                        ),
                                        Corners {
                                            top_left: left,
                                            bottom_left: left,
                                            top_right: right,
                                            bottom_right: right,
                                        },
                                    )
                                }
                                None => (bounds, Corners::all(px(3.))),
                            };
                            window.paint_quad(quad(
                                bounds,
                                corners,
                                *background,
                                Edges::default(),
                                gpui::transparent_black(),
                                BorderStyle::Solid,
                            ));
                        }
                    }
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

                for element in &mut zone_elements {
                    element.paint(window, cx);
                }

                for (origin, width, style) in &state.decoration_underlines {
                    window.paint_underline(*origin, *width, style);
                }
                // High contrast themes box the IME composition (Monaco
                // `nativeEditContext.css`).
                for bounds in &state.composition_boxes {
                    window.paint_quad(quad(
                        *bounds,
                        Corners::default(),
                        gpui::transparent_black(),
                        Edges::all(px(1.)),
                        colors.ime_underline,
                        BorderStyle::Solid,
                    ));
                }

                // Folded region placeholder ("⋯").
                for (x, top) in &state.fold_placeholders {
                    paint_fold_placeholder(*x, *top, layout, colors, window, cx);
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
                    paint_cursors(state, window);
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
                for (bounds, color, width) in &state.block_borders {
                    window.paint_quad(quad(
                        *bounds,
                        Corners::default(),
                        gpui::transparent_black(),
                        Edges::all(*width),
                        *color,
                        BorderStyle::Solid,
                    ));
                }
            },
        );

        // Content and glyph margin widgets: in the text area and the
        // gutter, under the sticky scroll (Monaco's `linesContent` and
        // margin).
        let text = layout.text_bounds;
        window.with_content_mask(Some(ContentMask { bounds: text }), |window| {
            for element in &mut content_elements {
                element.paint(window, cx);
            }
        });
        let gutter = Bounds::from_corners(
            point(bounds.left(), text.top()),
            point(text.left(), text.bottom()),
        );
        window.with_content_mask(Some(ContentMask { bounds: gutter }), |window| {
            for element in &mut zone_margin_elements {
                element.paint(window, cx);
            }
            for element in &mut glyph_elements {
                element.paint(window, cx);
            }
            if let Some(element) = &mut fold_tooltip {
                element.paint(window, cx);
            }
        });

        // Monaco's view parts in order: the scrollbars with the overview
        // ruler, the scroll shadow, then overlay widgets (the sticky scroll
        // among them) and the minimap.
        if let Some(ruler) = &state.overview_ruler {
            ruler.paint(window);
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
        // Monaco `ScrollDecorationViewPart`'s `.scroll-decoration`:
        // `box-shadow: scrollbar.shadow 0 6px 6px -6px inset`.
        if let Some(decoration) = layout.scroll_decoration {
            window.paint_inset_shadows(
                decoration,
                Corners::default(),
                &[gpui::BoxShadow {
                    color: colors.scrollbar_shadow,
                    offset: point(px(0.), px(SCROLL_SHADOW_HEIGHT)),
                    blur_radius: px(SCROLL_SHADOW_HEIGHT),
                    spread_radius: px(-SCROLL_SHADOW_HEIGHT),
                    inset: true,
                }],
            );
        }

        if !state.sticky.is_empty() && state.sticky_height > px(0.) {
            let background = colors.sticky_scroll_background.unwrap_or(colors.background);
            let gutter_background = colors.sticky_scroll_gutter_background.unwrap_or(background);
            let area = Bounds::new(
                bounds.origin,
                size(
                    layout.text_bounds.right() - bounds.left(),
                    state.sticky_height,
                ),
            );
            // Monaco `stickyScroll.css`: `box-shadow: editorStickyScroll.shadow
            // 0 4px 2px -2px` under the widget, then a 1px
            // `editorStickyScroll.border` bottom border.
            window.with_content_mask(Some(ContentMask { bounds }), |window| {
                if colors.sticky_scroll_shadow.a > 0. {
                    window.paint_drop_shadows(
                        area,
                        Corners::default(),
                        &[gpui::BoxShadow {
                            color: colors.sticky_scroll_shadow,
                            offset: point(px(0.), px(4.)),
                            blur_radius: px(2.),
                            spread_radius: px(-2.),
                            inset: false,
                        }],
                    );
                }
                if let Some(border) = colors.sticky_scroll_border {
                    window.paint_quad(fill(
                        Bounds::new(
                            point(area.left(), area.bottom()),
                            size(area.size.width, px(1.)),
                        ),
                        border,
                    ));
                }
            });
            window.with_content_mask(Some(ContentMask { bounds: area }), |window| {
                // Monaco's last line has z-index 0 under the others' 1, and
                // every line is opaque, so a pushed last line slides under
                // its parent instead of drawing over it.
                let order = state
                    .sticky
                    .len()
                    .checked_sub(1)
                    .into_iter()
                    .chain(0..state.sticky.len().saturating_sub(1));
                for k in order {
                    let (ix, top, line, number) = &state.sticky[k];
                    let y = bounds.top() + *top;
                    // `editorStickyScrollGutter.background` behind the
                    // number, `editorStickyScroll.background` behind the
                    // text.
                    window.paint_quad(fill(
                        Bounds::new(
                            point(area.left(), y),
                            size(layout.text_bounds.left() - area.left(), line_height),
                        ),
                        gutter_background,
                    ));
                    let content = Bounds::new(
                        point(layout.text_bounds.left(), y),
                        size(layout.text_bounds.size.width, line_height),
                    );
                    window.paint_quad(fill(content, background));
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
                            let origin_x = layout
                                .sticky_lines
                                .get(k)
                                .map_or(layout.text_bounds.left(), |l| l.origin_x);
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
                    // Its folding control (Monaco `StickyFoldingIcon`): the
                    // codicon at its 16px.
                    if let Some(control) = layout.sticky_lines.get(k).and_then(|l| l.fold.as_ref())
                        && (control.collapsed || state.sticky_show_fold_controls)
                    {
                        paint_chevron(
                            control,
                            colors.folding_control_foreground,
                            px(16.),
                            window,
                            cx,
                        );
                    }
                }
            });
        }

        // Overlay widgets, above everything in the editor. Overflowing
        // widgets are deferred draws, above the rest of the window too.
        for element in &mut overlay_elements {
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

        // Free replaced minimap images from the atlas.
        for image in &state.minimap_stale {
            window.drop_image(image.clone()).ok();
        }
        if let (Some(minimap), Some(paint)) = (layout.minimap, layout.minimap_paint.as_ref()) {
            // The editor's background, also where it sits left of the gutter;
            // the canvas (with `minimap.background`) goes over it.
            window.paint_quad(fill(minimap.bounds, colors.background));
            // Monaco `minimap-shadow-visible`: a 6px shadow left of it while
            // the text scrolls right.
            if let Some(band) = paint.shadow {
                window.with_content_mask(Some(ContentMask { bounds: band }), |window| {
                    window.paint_drop_shadows(
                        minimap.bounds,
                        Corners::default(),
                        &[gpui::BoxShadow {
                            color: colors.scrollbar_shadow.opacity(minimap.opacity),
                            offset: point(px(0.), px(0.)),
                            blur_radius: px(6.),
                            spread_radius: px(0.),
                            inset: false,
                        }],
                    );
                });
            }
            window.with_content_mask(
                Some(ContentMask {
                    bounds: minimap.bounds,
                }),
                |window| {
                    if let Some(background) = paint.background {
                        window.paint_quad(fill(minimap.bounds, background));
                    }
                    for (image, image_bounds) in &paint.images {
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
                    for (bounds, color) in &paint.marks {
                        window.paint_quad(fill(*bounds, *color));
                    }
                    for header in &paint.section_headers {
                        if let Some((band, color)) = header.band {
                            window.paint_quad(fill(band, color));
                        }
                        if let Some((label, origin, line_height)) = &header.label {
                            label
                                .paint(*origin, *line_height, TextAlign::Left, None, window, cx)
                                .ok();
                        }
                        if let Some((separator, color)) = header.separator {
                            window.paint_quad(fill(separator, color));
                        }
                    }
                    if let Some((slider, color)) = paint.slider {
                        window.paint_quad(fill(slider, color));
                    }
                    // `minimap.autohide`: fade it into the editor's
                    // background (GPUI images have no opacity).
                    if minimap.opacity < 1. {
                        window.paint_quad(fill(
                            minimap.bounds,
                            colors.background.opacity(1. - minimap.opacity),
                        ));
                    }
                },
            );
        }
    });
}

/// A slider's length on a track of `track` pixels showing `visible` of
/// `scroll` pixels (Monaco `ScrollbarState._computeValues`:
/// `round(max(MINIMUM_SLIDER_SIZE, floor(visible * track / scroll)))`), kept
/// within the track.
fn slider_size(track: Pixels, visible: Pixels, scroll: Pixels) -> Pixels {
    let track = f32::from(track).max(0.);
    let size = (f32::from(visible) * track / f32::from(scroll).max(1.))
        .floor()
        .max(MIN_SLIDER)
        .round();
    px(size.min(track))
}

/// Monaco `ViewLines._computeScrollLeftToReveal`: the scroll left showing
/// the request's ranges on the laid-out `lines`, or `None` when one isn't
/// laid out, or cursors span lines or don't fit.
fn scroll_left_to_reveal(
    request: &crate::editor::HorizontalReveal,
    lines: &[LineLayout],
    scroll_left: Pixels,
    viewport_width: Pixels,
    right_padding: Pixels,
) -> Option<Pixels> {
    // Monaco `ViewLines.HORIZONTAL_EXTRA_PX`.
    const HORIZONTAL_EXTRA_PX: Pixels = px(30.);
    let x_of = |point: PointUtf16| {
        lines
            .iter()
            .find(|line| line.contains(point))
            .map(|line| line.x_for_column(point.column))
    };
    let mut box_start = Pixels::MAX;
    let mut box_end = px(0.);
    for range in &request.ranges {
        if request.cursors && range.start.row != range.end.row {
            return None;
        }
        let start = x_of(range.start)?;
        let end = x_of(range.end).unwrap_or(start);
        box_start = box_start.min(start.min(end)).round();
        box_end = box_end.max(start.max(end)).round();
    }
    if !request.minimal {
        box_start = (box_start - HORIZONTAL_EXTRA_PX).max(px(0.));
        box_end += right_padding;
    }
    if request.cursors && box_end - box_start > viewport_width {
        return None;
    }
    Some(px(crate::view::reveal::minimum_scrolling(
        f32::from(scroll_left),
        f32::from(scroll_left + viewport_width),
        f32::from(box_start),
        f32::from(box_end),
        false,
        false,
    )))
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

/// Paints a shaped codicon centered in `bounds`.
pub(crate) fn paint_icon(
    shaped: ShapedLine,
    bounds: Bounds<Pixels>,
    size: Pixels,
    window: &mut Window,
    cx: &mut App,
) {
    let origin = point(
        bounds.center().x - shaped.width / 2.,
        bounds.center().y - size / 2.,
    );
    shaped
        .paint(origin, size, TextAlign::Left, None, window, cx)
        .ok();
}

/// The lightbulb (Monaco `LightBulbWidget`): its 16px codicon, or without
/// the codicon font a round bulb over a small base.
pub(crate) fn paint_lightbulb(
    bounds: Bounds<Pixels>,
    color: Hsla,
    icon: crate::codicons::Codicon,
    window: &mut Window,
    cx: &mut App,
) {
    let icon_size = bounds.size.height.min(px(16.));
    if let Some(shaped) = crate::codicons::shape_icon(icon, icon_size, color, window, cx) {
        paint_icon(shaped, bounds, icon_size, window, cx);
        return;
    }
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

/// Monaco's current line highlight (`AbstractLineHighlightOverlay`), as
/// boxes: every cursor's view line, and with viewport wrapping every view
/// line of its model line, gets a gutter box (`renderLineHighlight`
/// `gutter` or `all`, selection or not) and a text area box (`line` or
/// `all`, only while every selection is empty). The cursor's own view
/// line also gets the border (`current-line-exact`), 1px in high contrast
/// themes; the gutter box has none while the focused editor's
/// `editor.lineHighlightBackground` fills it, and none on its right in
/// `all` mode (`current-line-margin-both`). Unfocused, the boxes take
/// `editor.inactiveLineHighlightBackground`.
fn current_line_highlights(
    layout: &EditorLayout,
    options: &EditorOptions,
    colors: &EditorColors,
    high_contrast: bool,
    selections: &[Selection],
    cursor_view_lines: &[u32],
    focused: bool,
) -> Vec<LineHighlight> {
    let mode = options.render_line_highlight;
    let shown = !options.render_line_highlight_only_when_focus || focused;
    let in_margin = matches!(mode, RenderLineHighlight::Gutter | RenderLineHighlight::All) && shown;
    let in_content = matches!(mode, RenderLineHighlight::Line | RenderLineHighlight::All)
        && selections.iter().all(|s| s.is_empty())
        && shown;
    if !in_margin && !in_content {
        return Vec::new();
    }
    let mut exact: Vec<u32> = cursor_view_lines.to_vec();
    exact.sort_unstable();
    exact.dedup();
    // Monaco `_wordWrap` is `isViewportWrapping`: the cursor's whole model
    // line is highlighted.
    let wrapped_rows: Vec<u32> = if layout.layout_info.is_viewport_wrapping {
        let mut rows: Vec<u32> = exact
            .iter()
            .filter_map(|line| layout.view_line(*line).map(|l| l.row))
            .collect();
        rows.sort_unstable();
        rows.dedup();
        rows
    } else {
        Vec::new()
    };
    let background = if focused {
        colors.line_highlight_background
    } else {
        colors.inactive_line_highlight_background
    };
    let border_width = if high_contrast { px(1.) } else { px(2.) };
    let bounds = layout.bounds;
    let text = layout.text_bounds;
    // `max(scrollWidth, contentWidth)` from the content's start.
    let content_width = layout.content_width.max(layout.layout_info.content_width);
    let mut boxes = Vec::new();
    for line in &layout.lines {
        let is_exact = exact.binary_search(&line.view_line).is_ok();
        if !is_exact && wrapped_rows.binary_search(&line.row).is_err() {
            continue;
        }
        let top = layout.line_top(line.view_line);
        let height = layout.line_height_of(line.view_line);
        let border = line_highlight_border(is_exact, colors, border_width);
        if in_margin {
            let margin_border = border
                .filter(|_| !(focused && colors.line_highlight_background.is_some()))
                .map(|(color, edges)| {
                    let right = if in_content { px(0.) } else { edges.right };
                    (color, Edges { right, ..edges })
                });
            boxes.push(LineHighlight {
                view_line: line.view_line,
                margin: true,
                bounds: Bounds::new(
                    point(bounds.left(), top),
                    size(text.left() - bounds.left(), height),
                ),
                background,
                border: margin_border,
            });
        }
        if in_content {
            boxes.push(LineHighlight {
                view_line: line.view_line,
                margin: false,
                bounds: Bounds::new(
                    point(layout.text_origin_x(), top),
                    size(content_width, height),
                ),
                background,
                border,
            });
        }
    }
    boxes
}

/// The current line highlight's border on the cursor's own view line.
fn line_highlight_border(
    exact: bool,
    colors: &EditorColors,
    width: Pixels,
) -> Option<(Hsla, Edges<Pixels>)> {
    colors
        .line_highlight_border
        .filter(|_| exact)
        .map(|color| (color, Edges::all(width)))
}

/// Paints [`current_line_highlights`], the text area's boxes clipped to it
/// (the vertical scrollbar included, as Monaco's run under it).
fn paint_current_line_highlight(state: &PrepaintState, window: &mut Window) {
    let layout = &state.layout;
    if layout.line_highlights.is_empty() {
        return;
    }
    let text = layout.text_bounds;
    let content_right = (text.left() + layout.layout_info.content_width).min(layout.bounds.right());
    let content_mask = Bounds::from_corners(
        point(text.left(), layout.bounds.top()),
        point(content_right.max(text.right()), layout.bounds.bottom()),
    );
    for highlight in &layout.line_highlights {
        let paint = |window: &mut Window| {
            if let Some(background) = highlight.background {
                window.paint_quad(fill(highlight.bounds, background));
            }
            if let Some((color, edges)) = highlight.border {
                window.paint_quad(quad(
                    highlight.bounds,
                    Corners::default(),
                    gpui::transparent_black(),
                    edges,
                    color,
                    BorderStyle::Solid,
                ));
            }
        };
        if highlight.margin {
            paint(window);
        } else {
            window.with_content_mask(
                Some(ContentMask {
                    bounds: content_mask,
                }),
                paint,
            );
        }
    }
}

/// Monaco's folded-region placeholder (`.inline-folded:after` in
/// `folding.css`): a plain "⋯" (U+22EF) in the editor font, in
/// `editor.foldPlaceholderForeground` (unset in high contrast themes,
/// where it takes the text color).
fn paint_fold_placeholder(
    x: Pixels,
    top: Pixels,
    layout: &EditorLayout,
    colors: &EditorColors,
    window: &mut Window,
    cx: &mut App,
) {
    const ELLIPSIS: &str = "\u{22EF}";
    let color = colors
        .fold_placeholder_foreground
        .unwrap_or(colors.foreground);
    let shaped = window.text_system().shape_line(
        ELLIPSIS.into(),
        layout.metrics.font_size,
        &[run(ELLIPSIS.len(), &layout.metrics.font, color)],
        None,
    );
    shaped
        .paint(
            point(x, top),
            layout.metrics.line_height,
            TextAlign::Left,
            None,
            window,
            cx,
        )
        .ok();
}

/// A folding control (Monaco `foldingDecorations.ts`): the `chevron-down`
/// codicon when expanded and `chevron-right` when collapsed, `icon_size`
/// big and centered (`folding.css`); without the codicon font, a drawn
/// chevron.
fn paint_chevron(
    control: &FoldControl,
    color: Hsla,
    icon_size: Pixels,
    window: &mut Window,
    cx: &mut App,
) {
    let icon = if control.collapsed {
        crate::codicons::Codicon::ChevronRight
    } else {
        crate::codicons::Codicon::ChevronDown
    };
    let icon_size = icon_size.min(control.bounds.size.height);
    if let Some(shaped) = crate::codicons::shape_icon(icon, icon_size, color, window, cx) {
        paint_icon(shaped, control.bounds, icon_size, window, cx);
        return;
    }
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

/// Monaco `WhitespaceOverlay` with its default SVG rendering
/// (`_renderCircle`, `_renderArrow`): a dot a seventh of a space wide in the
/// middle of each space, and an arrow 0.8 spaces long from the start of each
/// tab, both centered vertically.
fn paint_whitespace(
    marks: &[(gpui::Point<Pixels>, bool)],
    layout: &EditorLayout,
    color: Hsla,
    window: &mut Window,
) {
    let space_width = f32::from(layout.metrics.space_width);
    let line_height = f32::from(layout.metrics.line_height);
    let radius = space_width / 7.;
    let mut arrows = PathBuilder::fill();
    let mut has_arrows = false;
    for (origin, is_tab) in marks {
        let (left, top) = (f32::from(origin.x), f32::from(origin.y));
        let cy = top + line_height / 2.;
        if *is_tab {
            // Monaco's ten-point arrow: a shaft `strokeWidth` thick and a
            // head, mirrored about the middle of the line.
            let stroke = space_width / 7.;
            let p2x = 100. / 125. * space_width;
            let p1 = (0., stroke / 2.);
            let p2 = (p2x, p1.1);
            let p3 = (p2.0 - 0.2 * p2.0, p2.1 + 0.2 * p2.0);
            let p4 = (p3.0 + 0.1 * p2.0, p3.1 + 0.1 * p2.0);
            let p5 = (p4.0 + 0.35 * p2.0, p4.1 - 0.35 * p2.0);
            let points = [
                p1,
                p2,
                p3,
                p4,
                p5,
                (p5.0, -p5.1),
                (p4.0, -p4.1),
                (p3.0, -p3.1),
                (p2.0, -p2.1),
                (p1.0, -p1.1),
            ];
            let at = |(x, y): (f32, f32)| point(px(left + x), px(cy + y));
            arrows.move_to(at(points[0]));
            for p in &points[1..] {
                arrows.line_to(at(*p));
            }
            arrows.close();
            has_arrows = true;
        } else {
            let cx = left + space_width / 2.;
            window.paint_quad(quad(
                Bounds::new(
                    point(px(cx - radius), px(cy - radius)),
                    size(px(radius * 2.), px(radius * 2.)),
                ),
                Corners::all(px(radius)),
                color,
                Edges::default(),
                gpui::transparent_black(),
                BorderStyle::Solid,
            ));
        }
    }
    if has_arrows && let Ok(path) = arrows.build() {
        window.paint_path(path, color);
    }
}

/// Paints the cursors (Monaco `ViewCursors`): each box in its color (or
/// outlined), blinking, with the character inside a block cursor drawn in
/// the cursor's background color, and in high contrast themes 1px sides in
/// that color while there's a selection.
fn paint_cursors(state: &PrepaintState, window: &mut Window) {
    let layout = &state.layout;
    let has_selection = state.selections.iter().any(|s| !s.is_empty());
    for cursor in &state.cursors {
        let height = cursor.bounds.size.height * state.cursor_scale;
        let bounds = Bounds::new(
            point(
                cursor.bounds.left(),
                cursor.bounds.top() + (cursor.bounds.size.height - height) / 2.,
            ),
            size(cursor.bounds.size.width, height),
        );
        let color = cursor.color.opacity(state.cursor_opacity);
        let text_color = cursor.text_color.opacity(state.cursor_opacity);
        if state.cursor_outline {
            window.paint_quad(quad(
                bounds,
                Corners::default(),
                gpui::transparent_black(),
                Edges::all(px(1.)),
                color,
                BorderStyle::Solid,
            ));
        } else {
            window.paint_quad(fill(bounds, color));
        }
        if let Some((view_line, range, shift)) = &cursor.glyph
            && let Some(line) = layout.view_line(*view_line)
        {
            let origin = point(
                layout.text_origin_x() + line.indent_x,
                layout.text_top(line.view_line),
            ) + *shift;
            window.with_content_mask(Some(ContentMask { bounds }), |window| {
                line.shaped.paint_glyphs(
                    range.clone(),
                    origin,
                    layout.metrics.line_height,
                    text_color,
                    window,
                );
            });
        }
        if state.high_contrast && has_selection {
            for x in [bounds.left(), bounds.right() - px(1.)] {
                window.paint_quad(fill(
                    Bounds::new(point(x, bounds.top()), size(px(1.), bounds.size.height)),
                    text_color,
                ));
            }
        }
    }
}

/// Monaco `SelectionsOverlay`: each selection's rectangle on each view
/// line, extended by a character where it continues on the next model line,
/// with `roundedSelection`'s outer corners rounded and its inner corners
/// joined by reverse-rounded pieces, and 1px insets between several
/// selections. Every selection's inner-corner pieces are painted before the
/// rectangles, as Monaco renders them.
fn paint_selections(
    layout: &EditorLayout,
    selections: &[Selection],
    origin_x: Pixels,
    color: Hsla,
    background: Hsla,
    rounded: bool,
    window: &mut Window,
) {
    use crate::view::selections::{CornerStyle, ROUNDED_PIECE_WIDTH, corner_styles, insets};
    // Monaco `typicalHalfwidthCharacterWidth`, added where the selection
    // continues on the next model line.
    let eol_width = layout.metrics.em_width;
    let multiple = selections.len() > 1;
    let radius = px(3.);
    let piece = px(ROUNDED_PIECE_WIDTH);
    // (bounds, corners) of every selection's rectangles, for the second
    // pass.
    let mut rects: Vec<(Bounds<Pixels>, Corners<Pixels>)> = Vec::new();
    let mut rows: Vec<(Pixels, Pixels, &LineLayout)> = Vec::new();
    for selection in selections.iter().filter(|s| !s.is_empty()) {
        rows.clear();
        let start: PointUtf16 = selection.start();
        let end: PointUtf16 = selection.end();
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
            rows.push((from, to.max(from), line));
        }
        let styles = (rounded && !rows.is_empty()).then(|| {
            let spans: Vec<(f32, f32)> = rows
                .iter()
                .map(|(from, to, _)| (f32::from(*from), f32::from(*to)))
                .collect();
            corner_styles(&spans, f32::from(eol_width))
        });
        for (ix, (from, to, line)) in rows.iter().enumerate() {
            let (top, bottom) = insets(ix, rows.len(), multiple);
            let line_top = layout.line_top(line.view_line);
            let bounds = Bounds::from_corners(
                point(origin_x + *from, line_top + px(top)),
                point(
                    origin_x + *to,
                    line_top + layout.line_height_of(line.view_line) - px(bottom),
                ),
            );
            let Some(styles) = styles.as_ref().map(|styles| styles[ix]) else {
                rects.push((bounds, Corners::default()));
                continue;
            };
            let round = |style: CornerStyle| {
                if style == CornerStyle::Extern {
                    radius
                } else {
                    px(0.)
                }
            };
            let is_intern = |style: CornerStyle| style == CornerStyle::Intern;
            // Reverse-rounded corners: a selection-colored piece outside
            // the rectangle, then the editor background over it with the
            // opposite corner rounded.
            for (left, top_style, bottom_style, start_side) in [
                (
                    bounds.left() - piece,
                    styles.start_top,
                    styles.start_bottom,
                    true,
                ),
                (bounds.right(), styles.end_top, styles.end_bottom, false),
            ] {
                if !is_intern(top_style) && !is_intern(bottom_style) {
                    continue;
                }
                let piece_bounds =
                    Bounds::new(point(left, bounds.top()), size(piece, bounds.size.height));
                window.paint_quad(fill(piece_bounds, color));
                let r = |intern: bool| if intern { radius } else { px(0.) };
                let corners = if start_side {
                    Corners {
                        top_right: r(is_intern(top_style)),
                        bottom_right: r(is_intern(bottom_style)),
                        ..Corners::default()
                    }
                } else {
                    Corners {
                        top_left: r(is_intern(top_style)),
                        bottom_left: r(is_intern(bottom_style)),
                        ..Corners::default()
                    }
                };
                window.paint_quad(quad(
                    piece_bounds,
                    corners,
                    background,
                    Edges::default(),
                    gpui::transparent_black(),
                    BorderStyle::Solid,
                ));
            }
            rects.push((
                bounds,
                Corners {
                    top_left: round(styles.start_top),
                    bottom_left: round(styles.start_bottom),
                    top_right: round(styles.end_top),
                    bottom_right: round(styles.end_bottom),
                },
            ));
        }
    }
    for (bounds, corners) in rects {
        if bounds.size.width <= px(0.) {
            continue;
        }
        window.paint_quad(quad(
            bounds,
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
    // A sweep: each list's first span that hasn't ended yet, so long lines
    // with many spans stay linear.
    let (mut base_ix, mut top_ix) = (0, 0);
    let covering = |spans: &[(Range<usize>, TokenStyle)], ix: &mut usize, start, end| {
        while *ix < spans.len() && spans[*ix].0.end <= start {
            *ix += 1;
        }
        // The last of the spans from here that cover it (later ones win).
        spans[*ix..]
            .iter()
            .take_while(|(range, _)| range.start <= start)
            .filter(|(range, _)| end <= range.end)
            .last()
            .map(|(_, style)| *style)
    };
    for pair in bounds.windows(2) {
        let (start, end) = (pair[0], pair[1]);
        let base_style = covering(base, &mut base_ix, start, end);
        let top_style = covering(top, &mut top_ix, start, end);
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
        captures_wheel: bool,
    },
    /// A zone's margin part (Monaco `marginDomNode`), in the gutter.
    ZoneMargin {
        view: gpui::AnyView,
        bounds: Bounds<Pixels>,
    },
    Content {
        view: gpui::AnyView,
        /// Top-left of the anchored character's line.
        anchor: gpui::Point<Pixels>,
        /// The secondary position's x, on the same line.
        secondary_x: Option<Pixels>,
        /// The line's height.
        line_height: Pixels,
        preference: Vec<ContentWidgetPreference>,
        /// When it overflows the editor: its layer and room.
        overflow: Option<(usize, crate::widgets::OverflowRoomCell)>,
    },
    Overlay {
        view: gpui::AnyView,
        position: OverlayPosition,
        stack_ordinal: Option<i32>,
        /// When it overflows the editor: its layer.
        overflow: Option<usize>,
    },
    Glyph {
        view: gpui::AnyView,
        bounds: Bounds<Pixels>,
    },
}

impl Embedded {
    /// The embedded view's entity, which identifies the widget.
    fn view_id(&self) -> gpui::EntityId {
        match self {
            Self::Zone { view, .. }
            | Self::ZoneMargin { view, .. }
            | Self::Content { view, .. }
            | Self::Overlay { view, .. }
            | Self::Glyph { view, .. } => view.entity_id(),
        }
    }
}

/// A view zone as laid out this frame.
pub(crate) struct ZoneSpec {
    /// The view line it's after (-1: above the first line).
    after: i64,
    /// Monaco's ordinal among zones after the same view line.
    ordinal: i64,
    height: Pixels,
    view: gpui::AnyView,
    margin_view: Option<gpui::AnyView>,
    full_width: bool,
    captures_wheel: bool,
}

/// Monaco `ContentWidget._reduceAnchorCoordinates`: the anchor's x moved
/// toward the secondary position's, keeping a full-width character's width
/// of a `width` wide widget over the primary position.
fn reduce_anchor_x(
    primary: Pixels,
    secondary: Option<Pixels>,
    width: Pixels,
    char: Pixels,
) -> Pixels {
    let Some(secondary) = secondary else {
        return primary;
    };
    if secondary < primary {
        secondary.max(primary - width + char)
    } else {
        secondary.min(primary + width - char)
    }
}

/// Where overflowing widgets paint among GPUI's deferred draws: above
/// GPUI's usual popovers (priority 1), then by their layer.
const OVERFLOW_PRIORITY: usize = 2;

/// An overflowing widget, wrapped to block the mouse from what's under it
/// (Monaco's overflow widgets are outside the editor's scrollable element,
/// so neither the editor nor the page under them gets their wheel or
/// clicks), with the arrow pointer. A widget that asks for a part beside
/// its anchored one in `room` (the suggest widget's details panel) blocks
/// it under those two parts only, and the rest of its box lets the mouse
/// through (Monaco's suggest list and details are separate widgets).
fn overflowing(
    view: gpui::AnyView,
    room: Option<crate::widgets::OverflowRoomCell>,
) -> gpui::AnyElement {
    // Under the view, so it blocks only what's under the widget. Read once
    // the view has rendered and asked for its parts.
    let blocker = gpui::canvas(
        move |bounds, window, _| {
            let asked = room
                .and_then(|room| room.anchor())
                .and_then(|asked| Some((asked.anchor_box, asked.beside?)));
            let (part, beside) = match asked {
                Some((anchor_box, beside)) => (anchor_box, Some(beside)),
                None => (Bounds::new(point(px(0.), px(0.)), bounds.size), None),
            };
            std::iter::once(part)
                .chain(beside)
                .map(|part| {
                    let part = Bounds::new(bounds.origin + part.origin, part.size);
                    window.insert_hitbox(part, HitboxBehavior::BlockMouse)
                })
                .collect::<Vec<_>>()
        },
        |_, hitboxes, window, _| {
            for hitbox in &hitboxes {
                window.set_cursor_style(MouseCursor::Arrow, hitbox);
            }
        },
    )
    .absolute()
    .size_full();
    gpui::div()
        .relative()
        .child(blocker)
        .child(view)
        .into_any_element()
}

/// Lays out a content widget's view at its min-content size, again at
/// `max_width` when that's wider (Monaco's `maxWidth` on the widget's
/// node). With `overflow`, its room, it wraps it as an overflowing widget.
fn layout_capped(
    view: &gpui::AnyView,
    overflow: Option<&crate::widgets::OverflowRoomCell>,
    max_width: Pixels,
    window: &mut Window,
    cx: &mut App,
) -> (gpui::AnyElement, gpui::Size<Pixels>) {
    let element = |view: &gpui::AnyView| match overflow {
        Some(room) => overflowing(view.clone(), Some(room.clone())),
        None => view.clone().into_any_element(),
    };
    let mut measured_element = element(view);
    let measured = measured_element.layout_as_root(
        size(AvailableSpace::MinContent, AvailableSpace::MinContent),
        window,
        cx,
    );
    if measured.width <= max_width.max(px(0.)) {
        return (measured_element, measured);
    }
    let mut capped = element(view);
    let measured = capped.layout_as_root(
        size(
            AvailableSpace::Definite(max_width.max(px(0.))),
            AvailableSpace::MinContent,
        ),
        window,
        cx,
    );
    (capped, measured)
}

/// An overlay widget laid out, waiting to be placed with the others.
struct PendingOverlay {
    view: Option<gpui::EntityId>,
    element: gpui::AnyElement,
    measured: gpui::Size<Pixels>,
    position: OverlayPosition,
    stack_ordinal: Option<i32>,
    overflow: Option<usize>,
}

/// The visible parts of the embedded views, for the editor's hit testing,
/// bottom-most first.
#[derive(Default)]
struct WidgetHits {
    bounds: Vec<Bounds<Pixels>>,
    kinds: Vec<crate::mouse_target::EmbeddedKind>,
    views: Vec<gpui::EntityId>,
    captures_wheel: Vec<bool>,
    /// The overflowing widgets, their layers and views; above the rest.
    overflowing: Vec<(
        Bounds<Pixels>,
        crate::mouse_target::EmbeddedKind,
        usize,
        gpui::EntityId,
    )>,
    /// The view of the widget being pushed.
    view: Option<gpui::EntityId>,
}

impl WidgetHits {
    fn push(
        &mut self,
        visible: Bounds<Pixels>,
        kind: crate::mouse_target::EmbeddedKind,
        captures_wheel: bool,
    ) {
        if visible.is_empty() {
            return;
        }
        self.bounds.push(visible);
        self.kinds.push(kind);
        self.views
            .push(self.view.unwrap_or(gpui::EntityId::from(0u64)));
        self.captures_wheel.push(captures_wheel);
    }

    /// An overflowing content or overlay widget, painted at `layer`.
    fn push_overflowing(
        &mut self,
        visible: Bounds<Pixels>,
        kind: crate::mouse_target::EmbeddedKind,
        layer: usize,
    ) {
        if !visible.is_empty() {
            let view = self.view.unwrap_or(gpui::EntityId::from(0u64));
            self.overflowing.push((visible, kind, layer, view));
        }
    }

    /// Puts the overflowing widgets last, in paint order; returns their
    /// bounds.
    fn finish(&mut self) -> Vec<Bounds<Pixels>> {
        self.overflowing.sort_by_key(|(_, _, layer, _)| *layer);
        for (bounds, kind, _, view) in &self.overflowing {
            self.bounds.push(*bounds);
            self.kinds.push(*kind);
            self.views.push(*view);
            self.captures_wheel.push(true);
        }
        self.overflowing
            .iter()
            .map(|(bounds, _, _, _)| *bounds)
            .collect()
    }
}

#[cfg(test)]
mod fold_control_tests {
    use gpui::TestAppContext;

    use crate::CodeEditor;
    use crate::options::ShowFoldingControls;

    /// Monaco's `linesDecorationsTooltip` on the folding controls.
    #[gpui::test]
    fn folding_controls_have_tooltips(cx: &mut TestAppContext) {
        cx.update(crate::init);
        let (editor, cx) = cx.add_window_view(|window, cx| {
            let mut editor = CodeEditor::with_text("a {\n    b\n    c\n}\n", window, cx);
            let mut options = editor.options().clone();
            options.show_folding_controls = ShowFoldingControls::Always;
            editor.update_options(options, cx);
            editor
        });
        cx.run_until_parked();
        cx.update(|window, _| window.refresh());
        cx.run_until_parked();
        let control = editor.read_with(cx, |e, _| e.fold_controls_for_tests()[0].1);
        cx.simulate_mouse_move(control.center(), None, gpui::Modifiers::none());
        cx.update(|window, _| window.refresh());
        cx.run_until_parked();
        cx.simulate_mouse_move(
            control.center() + gpui::point(gpui::px(1.), gpui::px(0.)),
            None,
            gpui::Modifiers::none(),
        );
        cx.executor()
            .advance_clock(std::time::Duration::from_secs(1));
        cx.run_until_parked();
        cx.update(|window, _| window.refresh());
        cx.run_until_parked();
        assert!(crate::tooltip::RENDERED.with(|r| {
            r.borrow()
                .iter()
                .any(|text| text == "Click to collapse the range.")
        }));
    }
}

#[cfg(test)]
mod decoration_option_tests {
    //! Monaco's `showIfCollapsed`, injected text `tokens` and the
    //! decoration fonts, as the line layout reads them.

    use std::sync::Arc;

    use gpui::{FontStyle, FontWeight};
    use text::PointUtf16;

    use super::*;
    use crate::decorations::{Decoration, DecorationId, DecorationOptions};

    fn decoration(id: u64, range: Range<usize>, options: DecorationOptions) -> Decoration {
        Decoration {
            id: DecorationId(id),
            range,
            options: Arc::new(options),
            owner: None,
        }
    }

    fn points(decorations: &[Decoration]) -> Vec<(PointUtf16, PointUtf16, &Decoration)> {
        decorations
            .iter()
            .map(|d| {
                (
                    PointUtf16::new(0, d.range.start as u32),
                    PointUtf16::new(0, d.range.end as u32),
                    d,
                )
            })
            .collect()
    }

    fn text(content: &str) -> Option<InjectedText> {
        Some(InjectedText {
            content: content.to_string().into(),
            ..Default::default()
        })
    }

    /// Monaco `getInjectedTextInInterval`: an empty range's injected text
    /// shows only with `showIfCollapsed`; a non-empty range's always does.
    #[test]
    fn injected_text_on_empty_ranges_needs_show_if_collapsed() {
        let decorations = [
            decoration(
                1,
                2..2,
                DecorationOptions {
                    before: text("hidden"),
                    ..Default::default()
                },
            ),
            decoration(
                2,
                3..3,
                DecorationOptions {
                    after: text("shown"),
                    show_if_collapsed: true,
                    ..Default::default()
                },
            ),
            decoration(
                3,
                4..6,
                DecorationOptions {
                    before: text("ranged"),
                    ..Default::default()
                },
            ),
        ];
        let injections = row_injections(0, &points(&decorations));
        let contents: Vec<(u32, &str)> = injections
            .iter()
            .map(|(column, _, text, _)| (*column, text.content.as_ref()))
            .collect();
        assert_eq!(contents, [(3, "shown"), (4, "ranged")]);
    }

    /// Monaco's `fontFamily`, `fontWeight` and `fontStyle` become font
    /// spans over the decorated text.
    #[test]
    fn decoration_fonts() {
        let decorations = [decoration(
            1,
            1..4,
            DecorationOptions {
                font_family: Some("Courier".into()),
                font_weight: Some(FontWeight::BOLD),
                font_style: Some(FontStyle::Italic),
                ..Default::default()
            },
        )];
        let theme = Theme::vs_dark();
        let (_, _, fonts) = decoration_styles_and_injections(
            0,
            0..8,
            &[],
            &points(&decorations),
            &EditorOptions::default(),
            &theme.colors,
        );
        assert_eq!(
            fonts,
            [(
                1..4,
                DecorationFont {
                    family: Some("Courier".into()),
                    weight: Some(FontWeight::BOLD),
                    style: Some(FontStyle::Italic),
                }
            )]
        );
    }

    /// Monaco `_renderNormalDecoration`: with `showIfCollapsed`, an empty or
    /// narrow range is a typical character wide, centered.
    #[test]
    fn collapsed_ranges_widen() {
        use gpui::px;
        assert_eq!(
            widen_collapsed(px(20.), px(20.), px(8.)),
            (px(16.), px(24.))
        );
        assert_eq!(
            widen_collapsed(px(20.), px(22.), px(8.)),
            (px(17.), px(25.))
        );
        // Wide enough already.
        assert_eq!(
            widen_collapsed(px(20.), px(30.), px(8.)),
            (px(20.), px(30.))
        );
        // Not past the line's start.
        assert_eq!(widen_collapsed(px(0.), px(0.), px(8.)), (px(0.), px(8.)));
    }

    /// Monaco's injected text `tokens`: each part of the text in its
    /// token's style, the rest in the text's.
    #[test]
    fn injected_text_tokens() {
        let red = TokenStyle {
            foreground: Some(gpui::red()),
            ..Default::default()
        };
        let bold = TokenStyle {
            bold: true,
            ..Default::default()
        };
        let text = InjectedText {
            content: "let x".into(),
            style: bold,
            tokens: Some(Arc::from([(0..3, red)])),
            ..Default::default()
        };
        assert_eq!(injected_text_style(&text, 0..3), red);
        assert_eq!(injected_text_style(&text, 1..2), red);
        assert_eq!(injected_text_style(&text, 3..5), bold);
    }
}
