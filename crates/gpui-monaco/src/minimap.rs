//! The minimap view part (Monaco `Minimap` and `InnerMinimap` in
//! `browser/viewParts/minimap/minimap.ts`): what each frame draws, and the
//! mouse. The geometry and rasterizing are ported in
//! [`crate::view::minimap`].
//!
//! As in Monaco the text is a canvas-like image (drawn on a background
//! thread here, and kept until the text, the colors or the lines shown
//! change), and the selections, decorations and section headers a layer
//! on top of it (quads here, redrawn every frame).

use std::sync::Arc;

use gpui::{
    Bounds, Context, Font, FontWeight, Hsla, MouseButton, MouseDownEvent, MouseMoveEvent, Pixels,
    ShapedLine, Window, font, point, px, size,
};
use text::PointUtf16;

use crate::cursor::Selection;
use crate::decorations::{MinimapPosition, MinimapSectionHeaderStyle};
use crate::editor::CodeEditor;
use crate::element::{HighlightSource, ZoneLayout, row_segment};
use crate::layout_info::MinimapLayoutInfo;
use crate::options::{EditorOptions, MinimapSize};
use crate::theme::Theme;
use crate::view::minimap::{
    CharRenderer, GUTTER_DECORATION_WIDTH, ImageData, LayoutInput, LayoutOptions, LineStyle,
    MINIMAP_GUTTER_WIDTH, MinimapLayout as SliderLayout, Rgba8, SamplingState, fit_section_header,
    render_line, x_offset_for_position,
};
use crate::view::view_lines::{ViewLines, utf16_to_byte};

/// Monaco's `.minimap canvas { opacity: .9 }`.
const CANVAS_OPACITY: f32 = 0.9;

/// Minimap lines per tile of the text canvas.
///
/// Monaco keeps one canvas the size of the minimap and, when it scrolls,
/// copies the lines it already drew (`_renderUntouchedLines`). Uploading a
/// whole new canvas image per scroll step is what costs here, so the
/// canvas is kept in tiles of whole lines instead: scrolling moves them,
/// and only a tile coming into view is drawn (on a background thread) and
/// uploaded. The pixels are the same as Monaco's canvas.
const TILE_LINES: u32 = 128;

/// The text canvas's tiles and what they were drawn for.
#[derive(Clone, Default)]
pub(crate) struct MinimapCache {
    /// Drawn tiles, by tile index; possibly for older content, shown until
    /// their redraw lands.
    tiles: Vec<Tile>,
    /// Replaced images, to free from the atlas at the next paint.
    pub stale: Vec<Arc<gpui::RenderImage>>,
    /// Tiles being drawn in the background; until one lands the old tile is
    /// shown (a frame-stale minimap is invisible, a few ms per keystroke on
    /// the main thread isn't).
    pending: Vec<(TileKey, Arc<gpui::Task<()>>)>,
    /// What the layout remembers between frames (Monaco
    /// `ComputeOptionsMemory`).
    pub layout_memory: crate::layout_info::LayoutMemory,
    /// The last frame's layout (Monaco `_lastRenderData.renderedLayout`),
    /// so a partially visible first line doesn't flicker.
    last_layout: Option<SliderLayout>,
    /// Monaco `_samplingState`, and the (view line count, minimap line
    /// count) it's for.
    pub sampling: Option<(Arc<SamplingState>, (u32, u64))>,
}

/// What the lines look like: (text, parsed text, theme, view lines,
/// render settings). Lines are redrawn once the syntax tree catches up,
/// for colors.
type ContentKey = (text::Version, Option<text::Version>, usize, u64, [u64; 4]);

/// A tile's (content, index).
type TileKey = (ContentKey, u32);

/// A drawn tile: minimap lines `index * TILE_LINES + 1` on.
#[derive(Clone)]
struct Tile {
    index: u32,
    content: ContentKey,
    image: Arc<gpui::RenderImage>,
}

/// A slider drag (Monaco `_startSliderDragging`).
#[derive(Clone, Copy, Debug)]
pub(crate) struct MinimapDrag {
    /// Where the pointer was (window coordinates); the slider's center for
    /// a click in `fill` or `fit` mode.
    initial_y: Pixels,
    initial_x: Pixels,
    /// The layout when the drag started (Monaco `initialSliderState`).
    layout: SliderLayout,
}

/// A section header to draw (Monaco `_renderSectionLabel`).
#[derive(Clone)]
pub(crate) struct SectionHeaderPaint {
    /// The background behind the label, if there's a label.
    pub band: Option<(Bounds<Pixels>, Hsla)>,
    /// The label, its origin and line height.
    pub label: Option<(ShapedLine, gpui::Point<Pixels>, Pixels)>,
    /// The separator line above it.
    pub separator: Option<(Bounds<Pixels>, Hsla)>,
}

/// What the minimap draws in a frame, over its background.
#[derive(Clone, Default)]
pub(crate) struct MinimapPaint {
    /// The text canvas's background (Monaco fills the canvas with
    /// `minimap.background`).
    pub background: Option<Hsla>,
    /// The text canvas's tiles and where they go (they may extend past the
    /// minimap).
    pub images: Vec<(Arc<gpui::RenderImage>, Bounds<Pixels>)>,
    /// Selection and decoration marks, in paint order (Monaco's
    /// decorations canvas).
    pub marks: Vec<(Bounds<Pixels>, Hsla)>,
    pub section_headers: Vec<SectionHeaderPaint>,
    /// The slider and its color (Monaco `minimapSlider.*`), faded.
    pub slider: Option<(Bounds<Pixels>, Hsla)>,
    /// The shadow left of the minimap when the text scrolls right (Monaco
    /// `minimap-shadow-visible`): the 6px band it's drawn in.
    pub shadow: Option<Bounds<Pixels>>,
}

/// What a frame knows that the minimap needs.
pub(crate) struct MinimapInput<'a> {
    pub options: &'a EditorOptions,
    pub info: MinimapLayoutInfo,
    pub bounds: Bounds<Pixels>,
    pub editor_bounds: Bounds<Pixels>,
    pub view_lines: &'a Arc<ViewLines>,
    pub snapshot: &'a text::BufferSnapshot,
    pub theme: &'a Arc<Theme>,
    pub zones: &'a ZoneLayout,
    pub line_height: Pixels,
    pub viewport_height: Pixels,
    pub scroll_top: Pixels,
    pub max_scroll_top: Pixels,
    pub scroll_left: Pixels,
    pub max_scroll_left: Pixels,
    /// The first and last (partially) visible view lines, 0-based.
    pub first_line: u32,
    pub last_line: u32,
    /// View zones (not the top padding) show in the viewport.
    pub whitespace_gaps: bool,
    pub tab_size: u32,
    pub wrap_column: Option<u32>,
    pub selections: &'a [Selection],
}

/// The minimap's part of a frame: its layout for the mouse, and what to
/// draw.
pub(crate) fn prepaint(
    editor: &mut CodeEditor,
    input: MinimapInput<'_>,
    window: &mut Window,
    cx: &mut Context<CodeEditor>,
) -> (crate::element::MinimapLayout, MinimapPaint) {
    let MinimapInput {
        options,
        info,
        bounds,
        editor_bounds,
        view_lines,
        snapshot,
        theme,
        zones,
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
        selections,
    } = input;
    let colors = &theme.colors;
    let pixel_ratio = f64::from(window.scale_factor());
    let view_line_count = view_lines.line_count().max(1);
    let minimap_line_height = f64::from(info.minimap_line_height);
    let inner_width = f64::from(info.minimap_canvas_inner_width);
    let inner_height = f64::from(info.minimap_canvas_inner_height);
    let outer_width = f64::from(f32::from(info.minimap_canvas_outer_width));
    let outer_height = f64::from(f32::from(info.minimap_canvas_outer_height));
    // Logical pixels per canvas pixel (the canvas is scaled to its CSS
    // size, squeezed in `fill`).
    let sx = if inner_width > 0. {
        outer_width / inner_width
    } else {
        1. / pixel_ratio
    };
    let sy = if inner_height > 0. {
        outer_height / inner_height
    } else {
        1. / pixel_ratio
    };

    // Monaco `MinimapSamplingState.compute`.
    let sampling = if info.minimap_is_sampling {
        let contained = crate::layout_info::contained_minimap_line_count(
            view_line_count,
            options.scroll_beyond_last_line,
            f64::from(f32::from(options.padding.top)),
            f64::from(f32::from(options.padding.bottom)),
            f64::from(f32::from(bounds.size.height)),
            f64::from(f32::from(line_height)),
            pixel_ratio,
        );
        let key = (view_line_count, contained.minimap_line_count.to_bits());
        match &editor.minimap_cache.sampling {
            Some((state, k)) if *k == key => Some(state.clone()),
            _ => {
                let state = Arc::new(SamplingState::compute(
                    view_line_count,
                    contained.minimap_line_count,
                ));
                editor.minimap_cache.sampling = Some((state.clone(), key));
                Some(state)
            }
        }
    } else {
        editor.minimap_cache.sampling = None;
        None
    };
    let line_count = sampling
        .as_ref()
        .map_or(view_line_count, |s| s.minimap_lines.len() as u32);
    // 1-based view line shown on 1-based minimap line `line`.
    let view_line_of = |line: u32| -> u32 {
        match &sampling {
            Some(s) => s.minimap_lines[(line.max(1) as usize - 1).min(s.minimap_lines.len() - 1)],
            None => line,
        }
    };
    let to_minimap_line = |view_line: u32| -> u32 {
        match &sampling {
            Some(s) => s.model_line_to_minimap_line(view_line),
            None => view_line,
        }
    };

    // Monaco `Minimap.render` and `MinimapLayout.create`.
    let (viewport_start, viewport_end) = (
        to_minimap_line(first_line + 1),
        to_minimap_line(last_line + 1),
    );
    let layout_options = LayoutOptions {
        pixel_ratio,
        minimap_line_height,
        canvas_inner_height: inner_height,
        line_height: f64::from(f32::from(line_height)),
        minimap_height_is_editor_height: info.minimap_height_is_editor_height,
        scroll_beyond_last_line: options.scroll_beyond_last_line,
        padding_top: f64::from(f32::from(options.padding.top)),
        padding_bottom: f64::from(f32::from(options.padding.bottom)),
        minimap_height: f64::from(f32::from(bounds.size.height)),
    };
    let layout_input = LayoutInput {
        viewport_start_line_number: viewport_start,
        viewport_end_line_number: viewport_end,
        viewport_start_line_number_vertical_offset: f64::from(f32::from(
            zones.line_y(viewport_start.saturating_sub(1), line_height),
        )),
        viewport_height: f64::from(f32::from(viewport_height)),
        viewport_contains_whitespace_gaps: whitespace_gaps,
        line_count,
        real_line_count: view_line_count,
        scroll_top: f64::from(f32::from(scroll_top)),
        scroll_height: f64::from(f32::from(max_scroll_top + viewport_height)),
    };
    let layout = SliderLayout::create(
        &layout_options,
        &layout_input,
        editor.minimap_cache.last_layout.as_ref(),
    );
    editor.minimap_cache.last_layout = Some(layout);

    // The text image (Monaco `renderLines`).
    let background = colors.minimap_background.unwrap_or(colors.background);
    let font_scale = (info.minimap_scale.max(1.)) as u32;
    let style = {
        let default_background = Rgba8::from_hsla(colors.background);
        let background = Rgba8::from_hsla(background);
        let alpha = f64::from(background.a) / 255.;
        let blend = |b: u8, d: u8| {
            ((f64::from(b) - f64::from(d)) * alpha + f64::from(d))
                .round()
                .clamp(0., 255.) as u8
        };
        LineStyle {
            text: info.render_minimap == crate::layout_info::RenderMinimap::Text,
            font_scale,
            char_width: font_scale,
            minimap_line_height,
            render_background: Rgba8 {
                r: blend(background.r, default_background.r),
                g: blend(background.g, default_background.g),
                b: blend(background.b, default_background.b),
                a: 255,
            },
            background_alpha: background.a,
            foreground_alpha: (colors.minimap_foreground_opacity.a * 255.)
                .round()
                .clamp(0., 255.) as u8,
            use_lighter_font: relative_luminance(colors.background) >= 0.5,
            tab_size,
        }
    };
    let (start, end) = (layout.start_line_number, layout.end_line_number);
    let parsed = editor
        .model()
        .read(cx)
        .syntax_snapshot()
        .map(|syntax| syntax.text.version());
    let view_key = {
        use std::hash::{Hash, Hasher};
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        view_lines.hidden().hash(&mut hasher);
        view_line_count.hash(&mut hasher);
        wrap_column.hash(&mut hasher);
        tab_size.hash(&mut hasher);
        hasher.finish()
    };
    let content: ContentKey = (
        snapshot.version(),
        parsed,
        Arc::as_ptr(theme) as usize,
        view_key,
        [
            u64::from(info.minimap_canvas_inner_width) << 32
                | u64::from(info.minimap_canvas_inner_height),
            minimap_line_height.to_bits(),
            u64::from(font_scale) << 1 | u64::from(style.text),
            sampling.as_ref().map_or(0, |s| s.sampling_ratio.to_bits()),
        ],
    );
    // The tiles covering lines `start..=end`, plus one more each way so
    // they're drawn before they scroll in; drop the others.
    let shown = (start.max(1) - 1) / TILE_LINES..=(end.max(1) - 1) / TILE_LINES;
    let tiles = shown.start().saturating_sub(1)
        ..=(shown.end() + 1).min((line_count.max(1) - 1) / TILE_LINES);
    {
        let cache = &mut editor.minimap_cache;
        let (keep, drop): (Vec<Tile>, Vec<Tile>) = std::mem::take(&mut cache.tiles)
            .into_iter()
            .partition(|tile| tiles.contains(&tile.index));
        cache.tiles = keep;
        cache.stale.extend(drop.into_iter().map(|tile| tile.image));
        cache
            .pending
            .retain(|((c, index), _)| *c == content && tiles.contains(index));
    }
    let canvas_background = Rgba8::from_hsla(background);
    let first_paint = editor.minimap_cache.tiles.is_empty();
    for index in tiles.clone() {
        let cache = &editor.minimap_cache;
        let current = cache
            .tiles
            .iter()
            .any(|tile| tile.index == index && tile.content == content);
        if current
            || cache
                .pending
                .iter()
                .any(|(key, _)| *key == (content, index))
        {
            continue;
        }
        let first = index * TILE_LINES + 1;
        let last = ((index + 1) * TILE_LINES).min(line_count);
        let lines: Vec<u32> = (first..=last).map(|line| view_line_of(line) - 1).collect();
        let source = HighlightSource::new(editor.model().read(cx));
        let build = {
            let (view_lines, snapshot, theme) =
                (view_lines.clone(), snapshot.clone(), theme.clone());
            let width = info.minimap_canvas_inner_width;
            move || {
                render_tile(
                    source.as_ref(),
                    &view_lines,
                    &snapshot,
                    &theme,
                    &lines,
                    &style,
                    width,
                    canvas_background,
                )
            }
        };
        if first_paint && shown.contains(&index) {
            // Nothing to show yet: draw it now.
            if let Some(image) = build() {
                editor.minimap_cache.tiles.push(Tile {
                    index,
                    content,
                    image,
                });
            }
            continue;
        }
        // Tiles drawn ahead of scrolling yield to the frame being drawn
        // (they'd otherwise share the CPU with it).
        let priority = if shown.contains(&index) {
            gpui::Priority::Medium
        } else {
            gpui::Priority::Low
        };
        let task = cx.spawn(async move |this, cx| {
            let image = cx
                .background_executor()
                .spawn_with_priority(priority, async move { build() })
                .await;
            this.update(cx, |editor, cx| {
                let cache = &mut editor.minimap_cache;
                let Some(ix) = cache
                    .pending
                    .iter()
                    .position(|(key, _)| *key == (content, index))
                else {
                    return;
                };
                cache.pending.remove(ix);
                if let Some(old) = cache.tiles.iter().position(|tile| tile.index == index) {
                    let old = cache.tiles.remove(old);
                    cache.stale.push(old.image);
                }
                if let Some(image) = image {
                    cache.tiles.push(Tile {
                        index,
                        content,
                        image,
                    });
                }
                cx.notify();
            })
            .ok();
        });
        editor
            .minimap_cache
            .pending
            .push(((content, index), Arc::new(task)));
    }
    let mut paint = MinimapPaint {
        // The canvas's background, at its opacity.
        background: Some(background.opacity(CANVAS_OPACITY)),
        ..Default::default()
    };
    for tile in &editor.minimap_cache.tiles {
        let first = tile.index * TILE_LINES + 1;
        let y = layout.y_for_line_number(first, minimap_line_height) * sy;
        let height = f64::from(tile.image.size(0).height.0) * sy;
        paint.images.push((
            tile.image.clone(),
            Bounds::new(
                point(bounds.left(), bounds.top() + px(y as f32)),
                size(info.minimap_canvas_outer_width, px(height as f32)),
            ),
        ));
    }

    // The decorations canvas (Monaco `renderDecorations`), in canvas
    // pixels, then mapped to the minimap.
    let to_bounds = |x: f64, y: f64, w: f64, h: f64| {
        Bounds::new(
            point(
                bounds.left() + px((x * sx) as f32),
                bounds.top() + px((y * sy) as f32),
            ),
            size(px((w * sx) as f32), px((h * sy) as f32)),
        )
    };
    let canvas = |color: Hsla| color.opacity(CANVAS_OPACITY);
    let char_width = f64::from(font_scale);
    // View line content, UTF-16, cached per line.
    let mut contents: std::collections::HashMap<u32, Vec<u16>> = Default::default();
    let mut content_of = |line: u32| -> Vec<u16> {
        contents
            .entry(line)
            .or_insert_with(|| {
                view_lines
                    .line_content(view_line_of(line) - 1)
                    .encode_utf16()
                    .collect()
            })
            .clone()
    };
    // 1-based (line, column) ranges in minimap lines.
    type LineRange = ((u32, u32), (u32, u32));
    let to_view = |p: PointUtf16, end: bool| {
        let (line, column) = view_lines.to_view(p, end);
        (line + 1, column + 1)
    };
    let to_minimap_range = |(start, end): LineRange| -> LineRange {
        match &sampling {
            Some(s) => {
                let (a, b) = s.decoration_line_range_to_minimap_line_range(start.0, end.0);
                ((a, start.1), (b, end.1))
            }
            None => (start, end),
        }
    };
    let mut highlighted_lines = vec![false; (end + 1).saturating_sub(start) as usize];
    let mut mark_line = |line: u32| -> bool {
        if line < start || line > end {
            return false;
        }
        let ix = (line - start) as usize;
        std::mem::replace(&mut highlighted_lines[ix], true)
    };
    let mut decoration_on_line = |range: LineRange, line: u32, color: Hsla, marks: &mut Vec<_>| {
        // Monaco `renderDecorationOnLine`.
        let y = layout.y_for_line_number(line, minimap_line_height);
        if y + minimap_line_height < 0. || y > inner_height {
            return;
        }
        let content = content_of(line);
        let start_column = if range.0.0 == line { range.0.1 } else { 1 };
        let end_column = if range.1.0 == line {
            range.1.1
        } else {
            content.len() as u32 + 1
        };
        let x1 = x_offset_for_position(&content, start_column, tab_size, char_width, inner_width);
        let x2 = x_offset_for_position(&content, end_column, tab_size, char_width, inner_width);
        if x2 > x1 {
            marks.push((to_bounds(x1, y, x2 - x1, minimap_line_height), color));
        }
    };

    let mut minimap_selections: Vec<LineRange> = selections
        .iter()
        .map(|s| to_minimap_range((to_view(s.start(), false), to_view(s.end(), true))))
        .collect();
    minimap_selections.sort();
    if let Some(selection_color) = colors.minimap_selection_highlight {
        // Monaco `_renderSelectionLineHighlights`: a band at half the
        // color from each selection's first line to its last line's top.
        let band = canvas(selection_color.opacity(0.5));
        let (mut y1, mut y2) = (0., 0.);
        for range in &minimap_selections {
            let Some((a, b)) = layout.intersect_with_viewport(range.0.0, range.1.0) else {
                continue;
            };
            for line in a..=b {
                mark_line(line);
            }
            let yy1 = layout.y_for_line_number(a, minimap_line_height);
            let yy2 = layout.y_for_line_number(b, minimap_line_height);
            if y2 >= yy1 {
                y2 = yy2;
            } else {
                if y2 > y1 {
                    paint.marks.push((
                        to_bounds(MINIMAP_GUTTER_WIDTH, y1, inner_width, y2 - y1),
                        band,
                    ));
                }
                y1 = yy1;
                y2 = yy2;
            }
        }
        if y2 > y1 {
            paint.marks.push((
                to_bounds(MINIMAP_GUTTER_WIDTH, y1, inner_width, y2 - y1),
                band,
            ));
        }
    }

    // Decorations with `minimap` options on the lines shown (Monaco
    // `getMinimapDecorationsInViewport`), by z-index.
    let decoration_start_row = view_lines.view_line(view_line_of(start) - 1).row;
    let decoration_end_row = view_lines.view_line(view_line_of(end.max(1)) - 1).row;
    let rope = view_lines.rope();
    let byte_range = rope.point_to_offset(text::Point::new(decoration_start_row, 0))
        ..rope.point_to_offset(text::Point::new(decoration_end_row, u32::MAX));
    // Section headers start above the lines shown (Monaco
    // `getSectionHeaderDecorationsInViewport`).
    let header_font_size = f64::from(options.minimap.section_header_font_size) * pixel_ratio;
    let header_start = (f64::from(start) - header_font_size / minimap_line_height)
        .max(1.)
        .floor() as u32;
    let header_byte_start = rope.point_to_offset(text::Point::new(
        view_lines.view_line(view_line_of(header_start) - 1).row,
        0,
    ));
    let shows_validation = editor.shows_validation_decorations();
    let editor_id = cx.entity_id();
    let mut decorations: Vec<_> = editor.model().update(cx, |model, _| {
        model
            .decorations_in_range(header_byte_start..byte_range.end, Some(editor_id))
            .into_iter()
            .filter(|d| d.options.minimap.is_some() && (shows_validation || !d.options.validation))
            .collect()
    });
    decorations.sort_by_key(|d| d.options.z_index);
    let decoration_range = |range: &std::ops::Range<usize>| -> LineRange {
        let start = rope.offset_to_point_utf16(range.start.min(rope.len()));
        let end = rope.offset_to_point_utf16(range.end.min(rope.len()));
        to_minimap_range((to_view(start, false), to_view(end, true)))
    };
    let decoration_color = |d: &crate::decorations::Decoration| -> Option<Hsla> {
        let color = match d.options.severity {
            Some(crate::MarkerSeverity::Error) => colors.minimap_error_highlight,
            Some(crate::MarkerSeverity::Warning) => colors.minimap_warning_highlight,
            Some(crate::MarkerSeverity::Info) => colors.minimap_info_highlight,
            Some(crate::MarkerSeverity::Hint) => None,
            None => d.options.minimap.as_ref()?.color,
        };
        color.filter(|c| c.a > 0.)
    };
    let mut headers: Vec<(u32, Option<String>, bool)> = Vec::new();
    let mut inline = Vec::new();
    for decoration in &decorations {
        let Some(minimap) = &decoration.options.minimap else {
            continue;
        };
        let range = decoration_range(&decoration.range);
        if let Some(style) = minimap.section_header_style {
            headers.push((
                range.0.0,
                minimap.section_header_text.as_ref().map(|t| t.to_string()),
                style == MinimapSectionHeaderStyle::Underlined,
            ));
            continue;
        }
        if decoration.range.end < byte_range.start {
            continue;
        }
        let Some(color) = decoration_color(decoration) else {
            continue;
        };
        inline.push((range, color, minimap.position));
    }
    // Monaco `_renderDecorationsLineHighlights`: one band per line, the
    // highest z-index first, none on selected lines.
    for (range, color, position) in inline.iter().rev() {
        if *position != MinimapPosition::Inline {
            continue;
        }
        let Some((a, b)) = layout.intersect_with_viewport(range.0.0, range.1.0) else {
            continue;
        };
        for line in a..=b {
            if mark_line(line) {
                continue;
            }
            let y = layout.y_for_line_number(line, minimap_line_height);
            paint.marks.push((
                to_bounds(MINIMAP_GUTTER_WIDTH, y, inner_width, minimap_line_height),
                canvas(color.opacity(0.5)),
            ));
        }
    }
    // Monaco `_renderSelectionsHighlights`.
    if let Some(selection_color) = colors.minimap_selection_highlight {
        for range in &minimap_selections {
            let Some((a, b)) = layout.intersect_with_viewport(range.0.0, range.1.0) else {
                continue;
            };
            for line in a..=b {
                decoration_on_line(*range, line, canvas(selection_color), &mut paint.marks);
            }
        }
    }
    // Monaco `_renderDecorationsHighlights`, the lowest z-index first.
    for (range, color, position) in &inline {
        let Some((a, b)) = layout.intersect_with_viewport(range.0.0, range.1.0) else {
            continue;
        };
        for line in a..=b {
            match position {
                MinimapPosition::Inline => {
                    decoration_on_line(*range, line, canvas(*color), &mut paint.marks)
                }
                MinimapPosition::Gutter => {
                    let y = layout.y_for_line_number(line, minimap_line_height);
                    paint.marks.push((
                        to_bounds(2., y, GUTTER_DECORATION_WIDTH, minimap_line_height),
                        canvas(*color),
                    ));
                }
            }
        }
    }

    // Section headers (Monaco `_renderSectionHeaders`): the built-in
    // `#region` and `MARK:` headers, and decorations' (Monaco's detector
    // adds them as decorations).
    for header in &editor.section_headers.headers {
        let line = to_minimap_line(view_lines.to_view(header.range.start, false).0 + 1);
        headers.push((
            line,
            (!header.text.is_empty()).then(|| header.text.clone()),
            header.has_separator_line,
        ));
    }
    headers.retain(|(line, _, _)| (header_start..=end).contains(line));
    headers.sort_by_key(|(line, _, _)| *line);
    let header_font_px = px((header_font_size * sy) as f32);
    let label_font = Font {
        weight: FontWeight::MEDIUM,
        ..font(".SystemUIFont")
    };
    let header_background = Hsla {
        a: 0.7,
        ..background
    };
    let header_foreground = colors.foreground;
    let max_header_width = (inner_width - MINIMAP_GUTTER_WIDTH) * sx;
    let shape = |text: &str, window: &mut Window| {
        window.text_system().shape_line(
            text.to_string().into(),
            header_font_px,
            &[gpui::TextRun {
                len: text.len(),
                font: label_font.clone(),
                color: canvas(header_foreground),
                background_color: None,
                underline: None,
                strikethrough: None,
            }],
            None,
        )
    };
    for (line, text, underlined) in headers {
        let y = layout.y_for_line_number(line, minimap_line_height) + header_font_size;
        let background_y = y - header_font_size;
        let separator_y = background_y + 2.;
        let mut header = SectionHeaderPaint {
            band: None,
            label: None,
            separator: None,
        };
        if let Some(text) = text.filter(|t| !t.is_empty()) {
            let mut shaped = shape(&text, window);
            if f64::from(f32::from(shaped.width)) > max_header_width {
                let ellipsis = shape("…", window);
                let fitted = fit_section_header(
                    &text,
                    f64::from(f32::from(shaped.width)),
                    f64::from(f32::from(ellipsis.width)),
                    max_header_width,
                );
                shaped = shape(&fitted, window);
            }
            header.band = Some((
                to_bounds(0., background_y, inner_width, header_font_size * 1.5),
                canvas(header_background),
            ));
            // The baseline is at `y`.
            let line_height = shaped.ascent + shaped.descent;
            let origin = point(
                bounds.left() + px((MINIMAP_GUTTER_WIDTH * sx) as f32),
                bounds.top() + px((y * sy) as f32) - shaped.ascent,
            );
            header.label = Some((shaped, origin, line_height));
        }
        if underlined {
            // A 0.4 canvas pixel stroke: one canvas pixel at 40%.
            header.separator = Some((
                to_bounds(0., separator_y - 0.5, inner_width, 1.),
                canvas(header_foreground.opacity(0.4)),
            ));
        }
        paint.section_headers.push(header);
    }

    // The slider (Monaco `.minimap-slider`): shown with `showSlider:
    // always`, else while the mouse is over the minimap or dragging, fading
    // over 100ms; hidden when not needed.
    let slider = Bounds::new(
        point(bounds.left(), bounds.top() + px(layout.slider_top as f32)),
        size(bounds.size.width, px(layout.slider_height as f32)),
    );
    let dragging = editor.minimap_drag.is_some();
    let (slider_opacity, fading) = editor
        .animation
        .minimap_slider
        .update(options.minimap.always_show_slider || editor.minimap_hovered || dragging);
    if fading {
        window.request_animation_frame();
    }
    if layout.slider_needed && slider_opacity > 0. {
        let color = if dragging {
            colors.minimap_slider_active
        } else if editor.minimap_slider_hovered {
            colors.minimap_slider_hover
        } else {
            colors.minimap_slider
        };
        paint.slider = Some((slider, color.opacity(slider_opacity)));
    }
    // Monaco `minimap-shadow-visible`, inside the editor.
    let shadow = Bounds::new(
        point(bounds.left() - px(6.), bounds.top()),
        size(px(6.), bounds.size.height),
    );
    if scroll_left < max_scroll_left && shadow.left() >= editor_bounds.left() {
        paint.shadow = Some(shadow);
    }

    let (opacity, fading, interactive) = editor
        .animation
        .minimap
        .update(options.minimap.autohide, editor.minimap_hovered);
    if fading {
        window.request_animation_frame();
    }
    let per_view_line = px((minimap_line_height * sy) as f32)
        / sampling.as_ref().map_or(1., |s| s.sampling_ratio as f32);
    (
        crate::element::MinimapLayout {
            bounds,
            slider,
            slider_needed: layout.slider_needed,
            layout,
            line_height: per_view_line,
            minimap_line_height: minimap_line_height as f32,
            canvas_scale_y: sy as f32,
            proportional: options.minimap.size == MinimapSize::Proportional,
            line_count,
            opacity,
            interactive,
        },
        paint,
    )
}

/// Monaco `Color.getRelativeLuminance`, which `MinimapTokensColorTracker`
/// uses to pick the lighter glyphs on light backgrounds.
fn relative_luminance(color: Hsla) -> f64 {
    let rgba = gpui::Rgba::from(color);
    let component = |c: f32| {
        let c = f64::from(c);
        if c <= 0.03928 {
            c / 12.92
        } else {
            ((c + 0.055) / 1.055).powf(2.4)
        }
    };
    0.2126 * component(rgba.r) + 0.7152 * component(rgba.g) + 0.0722 * component(rgba.b)
}

/// The pixel bytes a canvas pixel ends up as: BGRA for GPUI, with the
/// canvas's 0.9 opacity.
fn to_gpui_pixels(rows: &mut [u8]) {
    for pixel in rows.as_chunks_mut::<4>().0 {
        pixel.swap(0, 2);
        pixel[3] = (f32::from(pixel[3]) * CANVAS_OPACITY).round() as u8;
    }
}

/// Monaco `renderLines` for one tile: the 0-based view `lines`, one
/// minimap line each, on the canvas `background`.
#[allow(clippy::too_many_arguments)]
fn render_tile(
    highlights: Option<&HighlightSource>,
    view_lines: &ViewLines,
    snapshot: &text::BufferSnapshot,
    theme: &Theme,
    lines: &[u32],
    style: &LineStyle,
    width: u32,
    background: Rgba8,
) -> Option<Arc<gpui::RenderImage>> {
    let height = (lines.len() as f64 * style.minimap_line_height).ceil() as u32;
    if width == 0 || height == 0 {
        return None;
    }
    let renderer = CharRenderer::create(style.font_scale);
    let mut image = ImageData::filled(width, height, background);
    let default_color = Rgba8::from_hsla(theme.colors.foreground);
    let (first, last) = (*lines.first()?, *lines.last()?);
    let first_row = view_lines.view_line(first).row;
    let last_row = view_lines.view_line(last).row;
    // Contiguous lines share one highlight query; sampled ones get their
    // own.
    let contiguous = (last - first) as usize + 1 == lines.len();
    let row_range = |first_row: u32, last_row: u32| {
        snapshot.point_to_offset(text::Point::new(first_row, 0))
            ..snapshot.point_to_offset(text::Point::new(last_row, u32::MAX))
    };
    // Each span's color, converted once.
    let colored = |spans: Vec<(std::ops::Range<usize>, crate::theme::TokenStyle)>| {
        spans
            .into_iter()
            .map(|(range, style)| (range, style.foreground.map(Rgba8::from_hsla)))
            .collect::<Vec<_>>()
    };
    let shared = match (contiguous, highlights) {
        (true, Some(source)) => colored(source.highlights(theme, row_range(first_row, last_row))),
        _ => Vec::new(),
    };
    let mut content = Vec::new();
    let mut line_colors = Vec::new();
    let mut dy = 0.;
    for &view_line in lines {
        let segment = row_segment(view_lines, view_line);
        let line_highlights;
        let spans: &[(std::ops::Range<usize>, Option<Rgba8>)] = if contiguous {
            &shared
        } else {
            line_highlights = highlights.map_or_else(Vec::new, |source| {
                colored(source.highlights(theme, row_range(segment.row, segment.row)))
            });
            &line_highlights
        };
        let full = snapshot.line(segment.row);
        let line_start = snapshot.point_to_offset(text::Point::new(segment.row, 0));
        let byte_start = utf16_to_byte(&full, segment.columns.start);
        let byte_end = utf16_to_byte(&full, segment.columns.end);
        content.clear();
        line_colors.clear();
        // Continuation lines start with their indentation (Monaco's view
        // line content).
        for _ in 0..segment.indent {
            content.push(u16::from(b' '));
            line_colors.push(default_color);
        }
        let mut next = spans.partition_point(|(r, _)| r.end <= line_start + byte_start);
        let mut buf = [0u16; 2];
        for (byte, ch) in full[byte_start..byte_end].char_indices() {
            let offset = line_start + byte_start + byte;
            while spans.get(next).is_some_and(|(r, _)| r.end <= offset) {
                next += 1;
            }
            let color = spans
                .get(next)
                .filter(|(r, _)| r.start <= offset)
                .and_then(|(_, color)| *color)
                .unwrap_or(default_color);
            for unit in ch.encode_utf16(&mut buf) {
                content.push(*unit);
                line_colors.push(color);
            }
        }
        render_line(&mut image, &renderer, style, dy, &content, |i| {
            line_colors[i]
        });
        dy += style.minimap_line_height;
    }
    to_gpui_pixels(&mut image.data);
    let buffer = image::RgbaImage::from_raw(width, height, image.data)?;
    Some(Arc::new(gpui::RenderImage::new(vec![image::Frame::new(
        buffer,
    )])))
}

impl CodeEditor {
    /// Monaco `InnerMinimap`'s pointer down: on the slider it starts a
    /// drag; elsewhere a `proportional` minimap reveals the clicked line in
    /// the center, and a `fill` or `fit` one drags from the slider's center.
    pub(crate) fn minimap_mouse_down(
        &mut self,
        minimap: crate::element::MinimapLayout,
        event: &MouseDownEvent,
        cx: &mut Context<Self>,
    ) {
        if event.button != MouseButton::Left {
            return;
        }
        let start_drag = |this: &mut Self, initial_y: Pixels, cx: &mut Context<Self>| {
            this.minimap_drag = Some(MinimapDrag {
                initial_y,
                initial_x: event.position.x,
                layout: minimap.layout,
            });
            if event.position.y != initial_y {
                this.minimap_drag_to(event.position, cx);
            }
            cx.notify();
        };
        if minimap.slider_needed && minimap.slider.contains(&event.position) {
            start_drag(self, event.position.y, cx);
            return;
        }
        if !minimap.proportional {
            // Pretend the click was in the slider's center.
            start_drag(self, minimap.slider.center().y, cx);
            return;
        }
        let offset_y = f32::from(event.position.y - minimap.bounds.top());
        let line_index =
            (offset_y / minimap.canvas_scale_y / minimap.minimap_line_height).floor() as i64;
        let line = (line_index + i64::from(minimap.layout.start_line_number)
            - i64::from(minimap.layout.top_padding_line_count))
        .clamp(1, i64::from(minimap.line_count.max(1))) as u32;
        // Monaco `revealLineNumber`: the sampled line's view line, centered
        // smoothly.
        let view_line = match &self.minimap_cache.sampling {
            Some((state, _)) => {
                state.minimap_lines[(line as usize - 1).min(state.minimap_lines.len() - 1)]
            }
            None => line,
        } - 1;
        let point = self.view_lines.to_model(view_line, 0);
        self.request_reveal(
            crate::editor::PendingReveal {
                ranges: vec![point..point],
                cursors: false,
                reveal: crate::view::reveal::RevealType::Center,
                horizontal: false,
                minimal: false,
                scroll_type: crate::view::reveal::ScrollType::Smooth,
            },
            cx,
        );
    }

    /// Follows a slider drag (Monaco's `handlePointerMove`). Returns whether
    /// a drag is going on.
    pub(crate) fn minimap_mouse_move(
        &mut self,
        event: &MouseMoveEvent,
        cx: &mut Context<Self>,
    ) -> bool {
        if self.minimap_drag.is_none() {
            return false;
        }
        if event.pressed_button != Some(MouseButton::Left) {
            self.minimap_drag = None;
            cx.notify();
            return false;
        }
        self.minimap_drag_to(event.position, cx);
        true
    }

    fn minimap_drag_to(&mut self, position: gpui::Point<Pixels>, cx: &mut Context<Self>) {
        let Some(drag) = self.minimap_drag else {
            return;
        };
        let mut scroll_top = drag
            .layout
            .desired_scroll_top_from_delta(f64::from(f32::from(position.y - drag.initial_y)));
        // On Windows the scroll position snaps back once the pointer wanders
        // far from the minimap.
        if cfg!(windows)
            && let Some(minimap) = self.layout.as_ref().and_then(|l| l.minimap)
        {
            let distance = [
                (position.x - drag.initial_x).abs(),
                (position.x - minimap.bounds.left()).abs(),
                (position.x - minimap.bounds.right()).abs(),
            ]
            .into_iter()
            .fold(Pixels::MAX, Pixels::min);
            if f32::from(distance) > crate::view::minimap::POINTER_DRAG_RESET_DISTANCE {
                scroll_top = drag.layout.scroll_top;
            }
        }
        if !scroll_top.is_finite() {
            return;
        }
        let left = self.scroll_left;
        self.set_scroll_position(point(left, px(scroll_top as f32)), cx);
    }

    /// The minimap's slider in the last frame: its bounds, whether it's
    /// needed, and its color if drawn; for tests.
    #[doc(hidden)]
    pub fn minimap_slider_for_tests(&self) -> Option<(Bounds<Pixels>, bool, Option<Hsla>)> {
        let layout = self.layout.as_ref()?;
        let minimap = layout.minimap?;
        Some((
            minimap.slider,
            minimap.slider_needed,
            layout
                .minimap_paint
                .as_ref()
                .and_then(|p| p.slider.map(|(_, c)| c)),
        ))
    }

    /// The first and last minimap lines drawn (1-based) and the empty
    /// lines above them in the last frame (Monaco `MinimapLayout`), for
    /// tests.
    #[doc(hidden)]
    pub fn minimap_lines_for_tests(&self) -> Option<(u32, u32, u32)> {
        let minimap = self.layout.as_ref()?.minimap?;
        Some((
            minimap.layout.start_line_number,
            minimap.layout.end_line_number,
            minimap.layout.top_padding_line_count,
        ))
    }

    /// The selection and decoration marks the minimap drew in the last
    /// frame, for tests.
    #[doc(hidden)]
    pub fn minimap_marks_for_tests(&self) -> Vec<(Bounds<Pixels>, Hsla)> {
        self.layout
            .as_ref()
            .and_then(|l| l.minimap_paint.as_ref())
            .map_or_else(Vec::new, |p| p.marks.clone())
    }

    /// The minimap's section header labels in the last frame, and whether
    /// each has a separator line; for tests.
    #[doc(hidden)]
    pub fn minimap_section_headers_for_tests(&self) -> Vec<(String, bool)> {
        self.layout
            .as_ref()
            .and_then(|l| l.minimap_paint.as_ref())
            .map_or_else(Vec::new, |p| {
                p.section_headers
                    .iter()
                    .map(|h| {
                        (
                            h.label
                                .as_ref()
                                .map_or_else(String::new, |(l, _, _)| l.text.to_string()),
                            h.separator.is_some(),
                        )
                    })
                    .collect()
            })
    }

    /// The drawn tiles of the minimap's text canvas: each one's first
    /// minimap line and an identity for its image; for tests.
    #[doc(hidden)]
    pub fn minimap_tiles_for_tests(&self) -> Vec<(u32, usize)> {
        let mut tiles: Vec<_> = self
            .minimap_cache
            .tiles
            .iter()
            .map(|tile| {
                (
                    tile.index * TILE_LINES + 1,
                    Arc::as_ptr(&tile.image) as usize,
                )
            })
            .collect();
        tiles.sort();
        tiles
    }

    /// Whether the minimap's shadow showed in the last frame, for tests.
    #[doc(hidden)]
    pub fn minimap_shadow_for_tests(&self) -> bool {
        self.layout
            .as_ref()
            .and_then(|l| l.minimap_paint.as_ref())
            .is_some_and(|p| p.shadow.is_some())
    }
}
