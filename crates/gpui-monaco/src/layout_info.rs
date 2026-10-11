//! The editor's layout (Monaco `EditorLayoutInfo`,
//! `src/vs/editor/common/config/editorOptions.ts`, MIT), as
//! [`CodeEditor::layout_info`](crate::CodeEditor::layout_info) reports it
//! after a frame.
//!
//! Each frame computes it from the validated options as Monaco's
//! `EditorLayoutInfoComputer.computeLayout` does, before laying out the
//! lines, and draws with it. Horizontal positions are relative to the
//! editor's left edge.

use gpui::{Pixels, px};

/// How the minimap renders (Monaco `RenderMinimap`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum RenderMinimap {
    /// No minimap.
    #[default]
    None,
    /// Token-colored characters (`minimap.renderCharacters`).
    Text,
    /// Blocks.
    Blocks,
}

/// Where the minimap is and how it draws (Monaco `EditorMinimapLayoutInfo`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MinimapLayoutInfo {
    /// How it renders.
    pub render_minimap: RenderMinimap,
    /// Its left edge (Monaco `minimapLeft`).
    pub minimap_left: Pixels,
    /// Its width (Monaco `minimapWidth`).
    pub minimap_width: Pixels,
    /// `minimap.size` is `Fill` or `Fit` and the lines fill its height
    /// (Monaco `minimapHeightIsEditorHeight`).
    pub minimap_height_is_editor_height: bool,
    /// There are more lines than device pixels, so it samples them (Monaco
    /// `minimapIsSampling`).
    pub minimap_is_sampling: bool,
    /// The character scale (Monaco `minimapScale`).
    pub minimap_scale: f32,
    /// Device pixels per minimap line (Monaco `minimapLineHeight`).
    pub minimap_line_height: f32,
    /// Its width in device pixels (Monaco `minimapCanvasInnerWidth`).
    pub minimap_canvas_inner_width: u32,
    /// Its height in device pixels (Monaco `minimapCanvasInnerHeight`).
    pub minimap_canvas_inner_height: u32,
    /// Its width (Monaco `minimapCanvasOuterWidth`).
    pub minimap_canvas_outer_width: Pixels,
    /// Its height (Monaco `minimapCanvasOuterHeight`).
    pub minimap_canvas_outer_height: Pixels,
}

impl Default for MinimapLayoutInfo {
    fn default() -> Self {
        Self {
            render_minimap: RenderMinimap::None,
            minimap_left: px(0.),
            minimap_width: px(0.),
            minimap_height_is_editor_height: false,
            minimap_is_sampling: false,
            minimap_scale: 1.,
            minimap_line_height: 1.,
            minimap_canvas_inner_width: 0,
            minimap_canvas_inner_height: 0,
            minimap_canvas_outer_width: px(0.),
            minimap_canvas_outer_height: px(0.),
        }
    }
}

/// Where the overview ruler is (Monaco `OverviewRulerPosition`): it covers
/// the vertical scrollbar, between its arrows.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct OverviewRulerPosition {
    /// Its width.
    pub width: Pixels,
    /// Its height.
    pub height: Pixels,
    /// Its top, from the editor's top.
    pub top: Pixels,
    /// Its right edge, from the editor's right edge.
    pub right: Pixels,
}

/// The editor's layout (Monaco `EditorLayoutInfo`).
///
/// Left to right: the glyph margin, the line numbers, the line decorations
/// (with the folding controls), then the content, which the vertical
/// scrollbar overlays at its right. The minimap sits before the glyph
/// margin with `minimap.side: Left`, else before the vertical scrollbar.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct LayoutInfo {
    /// The editor's whole width.
    pub width: Pixels,
    /// The editor's whole height.
    pub height: Pixels,
    /// The glyph margin's left edge.
    pub glyph_margin_left: Pixels,
    /// The glyph margin's width (0 when `glyph_margin` is off).
    pub glyph_margin_width: Pixels,
    /// The glyph margin's decoration lanes (Monaco
    /// `glyphMarginDecorationLaneCount`): the most lanes glyph decorations
    /// and glyph margin widgets use on one line, at least one.
    pub glyph_margin_decoration_lane_count: u32,
    /// The line numbers' left edge.
    pub line_numbers_left: Pixels,
    /// The line numbers' width (0 when `line_numbers` is `Off`).
    pub line_numbers_width: Pixels,
    /// The line decorations' left edge.
    pub decorations_left: Pixels,
    /// The line decorations' width, the folding controls included.
    pub decorations_width: Pixels,
    /// The text area's left edge.
    pub content_left: Pixels,
    /// The text area's width, the vertical scrollbar included (Monaco
    /// `contentWidth`).
    pub content_width: Pixels,
    /// The minimap.
    pub minimap: MinimapLayoutInfo,
    /// Typical characters fitting on a line (Monaco `viewportColumn`).
    pub viewport_column: u32,
    /// Wrapping at the viewport was forced for a screen reader, because the
    /// model is mostly long lines (Monaco `isWordWrapMinified`).
    pub is_word_wrap_minified: bool,
    /// Lines wrap at the viewport's width (`word_wrap` `On` or `Bounded`).
    pub is_viewport_wrapping: bool,
    /// The column lines wrap at, if they wrap (Monaco `wrappingColumn`, -1
    /// for none).
    pub wrapping_column: Option<u32>,
    /// The vertical scrollbar's width.
    pub vertical_scrollbar_width: Pixels,
    /// The horizontal scrollbar's height (Monaco reports the option, shown
    /// or not).
    pub horizontal_scrollbar_height: Pixels,
    /// The overview ruler.
    pub overview_ruler: OverviewRulerPosition,
}

/// What [`compute_layout`] needs besides the options (Monaco
/// `EditorLayoutInfoComputerEnv`).
#[derive(Clone, Copy, Debug)]
pub(crate) struct LayoutEnv {
    /// The editor's width.
    pub outer_width: f64,
    /// The editor's height.
    pub outer_height: f64,
    /// The line height.
    pub line_height: f64,
    /// Digits of the last line number.
    pub line_numbers_digit_count: u32,
    /// A typical character's width (Monaco `typicalHalfwidthCharacterWidth`).
    pub typical_halfwidth_character_width: f64,
    /// The widest digit's width.
    pub max_digit_width: f64,
    /// Device pixels per pixel.
    pub pixel_ratio: f64,
    /// View lines (wrapped and folded).
    pub view_line_count: u32,
    /// The glyph margin's lanes.
    pub glyph_margin_decoration_lane_count: u32,
    /// The model is mostly long lines (Monaco `isDominatedByLongLines`).
    pub is_dominated_by_long_lines: bool,
    /// The editor is optimized for a screen reader (Monaco
    /// `accessibilitySupport === Enabled`).
    pub screen_reader: bool,
}

/// The minimap's layout inputs (Monaco `IMinimapLayoutInput`).
#[derive(Clone, Debug, PartialEq)]
struct MinimapLayoutInput {
    outer_width: f64,
    outer_height: f64,
    line_height: f64,
    typical_halfwidth_character_width: f64,
    pixel_ratio: f64,
    scroll_beyond_last_line: bool,
    padding_top: f64,
    padding_bottom: f64,
    minimap: crate::options::MinimapOptions,
    minimap_enabled: bool,
    vertical_scrollbar_width: f64,
    view_line_count: u32,
    remaining_width: f64,
    is_viewport_wrapping: bool,
}

impl MinimapLayoutInput {
    /// Monaco's `couldUseMemory` test: the same input but for the outer
    /// width, view line count and remaining width.
    fn is_stable_with(&self, other: &Self) -> bool {
        let (a, b) = (&self.minimap, &other.minimap);
        self.outer_height == other.outer_height
            && self.line_height == other.line_height
            && self.typical_halfwidth_character_width == other.typical_halfwidth_character_width
            && self.pixel_ratio == other.pixel_ratio
            && self.scroll_beyond_last_line == other.scroll_beyond_last_line
            && self.padding_top == other.padding_top
            && self.padding_bottom == other.padding_bottom
            && self.minimap_enabled == other.minimap_enabled
            && a.side == b.side
            && a.size == b.size
            && a.always_show_slider == b.always_show_slider
            && a.render_characters == b.render_characters
            && a.max_column == b.max_column
            && a.scale == b.scale
            && self.vertical_scrollbar_width == other.vertical_scrollbar_width
            && self.is_viewport_wrapping == other.is_viewport_wrapping
    }
}

/// What the minimap layout remembers between frames (Monaco
/// `ComputeOptionsMemory`), to break the loop between `minimap.size: Fill`
/// or `Fit`, the minimap's width and viewport wrapping.
#[derive(Clone, Debug, Default)]
pub(crate) struct LayoutMemory {
    stable_minimap_layout_input: Option<MinimapLayoutInput>,
    stable_fit_remaining_width: f64,
    stable_fit_max_minimap_scale: f64,
}

/// Monaco `MINIMAP_GUTTER_WIDTH`.
const MINIMAP_GUTTER_WIDTH: f64 = 8.;

/// The view lines the minimap shows when it fits the editor (Monaco
/// `EditorLayoutInfoComputer.computeContainedMinimapLineCount`).
#[derive(Clone, Copy, Debug)]
pub(crate) struct ContainedMinimapLineCount {
    pub typical_viewport_line_count: f64,
    pub extra_lines_before_first_line: f64,
    pub extra_lines_beyond_last_line: f64,
    pub desired_ratio: f64,
    pub minimap_line_count: f64,
}

/// Monaco `EditorLayoutInfoComputer.computeContainedMinimapLineCount`.
pub(crate) fn contained_minimap_line_count(
    view_line_count: u32,
    scroll_beyond_last_line: bool,
    padding_top: f64,
    padding_bottom: f64,
    height: f64,
    line_height: f64,
    pixel_ratio: f64,
) -> ContainedMinimapLineCount {
    let view_line_count = f64::from(view_line_count);
    let typical_viewport_line_count = height / line_height;
    let extra_lines_before_first_line = (padding_top / line_height).floor();
    let mut extra_lines_beyond_last_line = (padding_bottom / line_height).floor();
    if scroll_beyond_last_line {
        extra_lines_beyond_last_line =
            extra_lines_beyond_last_line.max(typical_viewport_line_count - 1.);
    }
    let desired_ratio =
        (extra_lines_before_first_line + view_line_count + extra_lines_beyond_last_line)
            / (pixel_ratio * height);
    let minimap_line_count = (view_line_count / desired_ratio).floor();
    ContainedMinimapLineCount {
        typical_viewport_line_count,
        extra_lines_before_first_line,
        extra_lines_beyond_last_line,
        desired_ratio,
        minimap_line_count,
    }
}

/// Monaco `EditorLayoutInfoComputer._computeMinimapLayout`.
fn compute_minimap_layout(
    input: MinimapLayoutInput,
    memory: &mut LayoutMemory,
) -> MinimapLayoutInfo {
    use crate::options::{MinimapSide, MinimapSize};
    let outer_width = input.outer_width;
    let outer_height = input.outer_height;
    let pixel_ratio = input.pixel_ratio;

    if !input.minimap_enabled {
        return MinimapLayoutInfo {
            render_minimap: RenderMinimap::None,
            minimap_left: px(0.),
            minimap_width: px(0.),
            minimap_height_is_editor_height: false,
            minimap_is_sampling: false,
            minimap_scale: 1.,
            minimap_line_height: 1.,
            minimap_canvas_inner_width: 0,
            minimap_canvas_inner_height: (pixel_ratio * outer_height).floor().max(0.) as u32,
            minimap_canvas_outer_width: px(0.),
            minimap_canvas_outer_height: px(outer_height as f32),
        };
    }

    // Monaco reuses the memory when only the view line count and the
    // remaining width changed.
    let could_use_memory = memory
        .stable_minimap_layout_input
        .as_ref()
        .is_some_and(|stable| input.is_stable_with(stable));

    let line_height = input.line_height;
    let typical_halfwidth_character_width = input.typical_halfwidth_character_width;
    let minimap_render_characters = input.minimap.render_characters;
    let configured_scale = f64::from(input.minimap.scale);
    let mut minimap_scale = if pixel_ratio >= 2. {
        (configured_scale * 2.).round()
    } else {
        configured_scale
    };
    let minimap_max_column = f64::from(input.minimap.max_column);
    let minimap_size = input.minimap.size;
    let vertical_scrollbar_width = input.vertical_scrollbar_width;
    let view_line_count = input.view_line_count;
    let remaining_width = input.remaining_width;
    let is_viewport_wrapping = input.is_viewport_wrapping;

    let base_char_height = if minimap_render_characters { 2. } else { 3. };
    let mut minimap_canvas_inner_height = (pixel_ratio * outer_height).floor();
    let minimap_canvas_outer_height = minimap_canvas_inner_height / pixel_ratio;
    let mut minimap_height_is_editor_height = false;
    let mut minimap_is_sampling = false;
    let mut minimap_line_height = base_char_height * minimap_scale;
    let mut minimap_char_width = minimap_scale / pixel_ratio;
    let mut minimap_width_multiplier: f64 = 1.;

    if minimap_size != MinimapSize::Proportional {
        let contained = contained_minimap_line_count(
            view_line_count,
            input.scroll_beyond_last_line,
            input.padding_top,
            input.padding_bottom,
            outer_height,
            line_height,
            pixel_ratio,
        );
        // The ratio is left out of the layout, so sampling doesn't change
        // it all the time.
        let ratio = f64::from(view_line_count) / contained.minimap_line_count;

        if ratio > 1. {
            minimap_height_is_editor_height = true;
            minimap_is_sampling = true;
            minimap_scale = 1.;
            minimap_line_height = 1.;
            minimap_char_width = minimap_scale / pixel_ratio;
        } else {
            let mut fit_becomes_fill = false;
            let mut max_minimap_scale = minimap_scale + 1.;
            let lines = contained.extra_lines_before_first_line
                + f64::from(view_line_count)
                + contained.extra_lines_beyond_last_line;
            let stable_fit = is_viewport_wrapping
                && could_use_memory
                && remaining_width <= memory.stable_fit_remaining_width;

            if minimap_size == MinimapSize::Fit {
                let effective_minimap_height = (lines * minimap_line_height).ceil();
                if stable_fit {
                    // Viewport wrapping loops back on the minimap's width
                    // (the view line count changes it, and it changes the
                    // viewport): once at a smaller scale, stay there.
                    fit_becomes_fill = true;
                    max_minimap_scale = memory.stable_fit_max_minimap_scale;
                } else {
                    fit_becomes_fill = effective_minimap_height > minimap_canvas_inner_height;
                }
            }

            if minimap_size == MinimapSize::Fill || fit_becomes_fill {
                minimap_height_is_editor_height = true;
                let configured_minimap_scale = minimap_scale;
                minimap_line_height =
                    (line_height * pixel_ratio).min((1. / contained.desired_ratio).floor().max(1.));
                if stable_fit {
                    max_minimap_scale = memory.stable_fit_max_minimap_scale;
                }
                minimap_scale =
                    max_minimap_scale.min((minimap_line_height / base_char_height).floor().max(1.));
                if minimap_scale > configured_minimap_scale {
                    minimap_width_multiplier = (minimap_scale / configured_minimap_scale).min(2.);
                }
                minimap_char_width = minimap_scale / pixel_ratio / minimap_width_multiplier;
                minimap_canvas_inner_height =
                    (contained.typical_viewport_line_count.max(lines) * minimap_line_height).ceil();
                if is_viewport_wrapping {
                    // Remembered for next time.
                    memory.stable_fit_remaining_width = remaining_width;
                    memory.stable_fit_max_minimap_scale = minimap_scale;
                    memory.stable_minimap_layout_input = Some(input.clone());
                } else {
                    memory.stable_minimap_layout_input = None;
                    memory.stable_fit_remaining_width = 0.;
                }
            }
        }
    }

    // The minimap takes a share of the remaining width proportional to its
    // characters' width, so it shows about the columns the viewport does:
    // ((remainingWidth - verticalScrollbarWidth - 2) * minimapCharWidth) /
    // (typicalHalfwidthCharacterWidth + minimapCharWidth), leaving 2px for
    // the cursor.
    let minimap_max_width = (minimap_max_column * minimap_char_width).floor();
    let minimap_width = minimap_max_width.min(
        (((remaining_width - vertical_scrollbar_width - 2.) * minimap_char_width)
            / (typical_halfwidth_character_width + minimap_char_width))
            .floor()
            .max(0.)
            + MINIMAP_GUTTER_WIDTH,
    );

    let minimap_canvas_inner_width = (pixel_ratio * minimap_width).floor();
    let minimap_canvas_outer_width = minimap_canvas_inner_width / pixel_ratio;
    let minimap_canvas_inner_width =
        (minimap_canvas_inner_width * minimap_width_multiplier).floor();

    let render_minimap = if minimap_render_characters {
        RenderMinimap::Text
    } else {
        RenderMinimap::Blocks
    };
    let minimap_left = if input.minimap.side == MinimapSide::Left {
        0.
    } else {
        outer_width - minimap_width - vertical_scrollbar_width
    };

    MinimapLayoutInfo {
        render_minimap,
        minimap_left: px(minimap_left as f32),
        minimap_width: px(minimap_width as f32),
        minimap_height_is_editor_height,
        minimap_is_sampling,
        minimap_scale: minimap_scale as f32,
        minimap_line_height: minimap_line_height as f32,
        minimap_canvas_inner_width: minimap_canvas_inner_width.max(0.) as u32,
        minimap_canvas_inner_height: minimap_canvas_inner_height.max(0.) as u32,
        minimap_canvas_outer_width: px(minimap_canvas_outer_width as f32),
        minimap_canvas_outer_height: px(minimap_canvas_outer_height as f32),
    }
}

/// The layout Monaco's `EditorLayoutInfoComputer.computeLayout` gives
/// `options` (validated) in `env`.
///
/// Left to right: the glyph margin (a line height per lane), the line
/// numbers (the widest digit times the digits of the last line number, at
/// least `line_numbers_min_chars`, rounded), the line decorations
/// (`line_decorations_width`, plus 16px for the folding controls unless
/// they're never shown) and the content. The minimap takes its share of
/// the rest, before the glyph margin with `minimap.side: Left`.
pub(crate) fn compute_layout(
    options: &crate::options::EditorOptions,
    env: &LayoutEnv,
    memory: &mut LayoutMemory,
) -> LayoutInfo {
    use crate::options::{LineNumbers, ShowFoldingControls, WordWrap};
    // Monaco works in whole pixels here (`| 0`).
    let outer_width = env.outer_width.trunc();
    let outer_height = env.outer_height.trunc();
    let line_height = env.line_height.trunc();
    let typical_halfwidth_character_width = env.typical_halfwidth_character_width;

    let (word_wrap, is_word_wrap_minified) =
        options.wrap_mode(env.screen_reader, env.is_dominated_by_long_lines);

    let show_line_numbers = options.line_numbers != LineNumbers::Off;
    let vertical_scrollbar_width = f64::from(f32::from(options.vertical_scrollbar_size));
    let scrollbar_arrow_size = f64::from(f32::from(options.scrollbar.arrow_size));
    let horizontal_scrollbar_height = f64::from(f32::from(options.horizontal_scrollbar_size));

    let mut line_decorations_width = f64::from(f32::from(
        options
            .line_decorations_width
            .resolve(px(typical_halfwidth_character_width as f32)),
    ));
    if options.folding && options.show_folding_controls != ShowFoldingControls::Never {
        line_decorations_width += 16.;
    }

    let line_numbers_width = if show_line_numbers {
        let digit_count = env
            .line_numbers_digit_count
            .max(options.line_numbers_min_chars);
        (f64::from(digit_count) * env.max_digit_width).round()
    } else {
        0.
    };

    let glyph_margin_width = if options.glyph_margin {
        line_height * f64::from(env.glyph_margin_decoration_lane_count)
    } else {
        0.
    };

    let mut glyph_margin_left = 0.;
    let mut line_numbers_left = glyph_margin_left + glyph_margin_width;
    let mut decorations_left = line_numbers_left + line_numbers_width;
    let mut content_left = decorations_left + line_decorations_width;

    let remaining_width =
        outer_width - glyph_margin_width - line_numbers_width - line_decorations_width;

    let is_viewport_wrapping = matches!(word_wrap, WordWrap::On | WordWrap::Bounded);
    let mut wrapping_column =
        (word_wrap == WordWrap::WordWrapColumn).then_some(options.word_wrap_column.max(1));

    let minimap = compute_minimap_layout(
        MinimapLayoutInput {
            outer_width,
            outer_height,
            line_height,
            typical_halfwidth_character_width,
            pixel_ratio: env.pixel_ratio,
            scroll_beyond_last_line: options.scroll_beyond_last_line,
            padding_top: f64::from(f32::from(options.padding.top)),
            padding_bottom: f64::from(f32::from(options.padding.bottom)),
            minimap: options.minimap.clone(),
            minimap_enabled: options.minimap.enabled && !options.single_line,
            vertical_scrollbar_width,
            view_line_count: env.view_line_count,
            remaining_width,
            is_viewport_wrapping,
        },
        memory,
    );
    let minimap_width = f64::from(f32::from(minimap.minimap_width));

    if minimap.render_minimap != RenderMinimap::None && minimap.minimap_left == px(0.) {
        // The minimap is on the left: everything else moves right.
        glyph_margin_left += minimap_width;
        line_numbers_left += minimap_width;
        decorations_left += minimap_width;
        content_left += minimap_width;
    }
    let content_width = remaining_width - minimap_width;

    // Leaving 2px for the cursor after the last character.
    let viewport_column = ((content_width - vertical_scrollbar_width - 2.)
        / typical_halfwidth_character_width)
        .floor()
        .max(1.) as u32;

    let vertical_arrow_size = if options.scrollbar.vertical_has_arrows {
        scrollbar_arrow_size
    } else {
        0.
    };

    if is_viewport_wrapping {
        // The rightmost column is left for the word wrap indicator, so it
        // overlaps neither the text nor the vertical scrollbar.
        let mut column = viewport_column
            .saturating_sub(options.word_wrap_indicator as u32)
            .max(1);
        if word_wrap == WordWrap::Bounded {
            column = column.min(options.word_wrap_column.max(1));
        }
        wrapping_column = Some(column);
    }

    let pixels = |value: f64| px(value as f32);
    LayoutInfo {
        width: pixels(outer_width),
        height: pixels(outer_height),
        glyph_margin_left: pixels(glyph_margin_left),
        glyph_margin_width: pixels(glyph_margin_width),
        glyph_margin_decoration_lane_count: env.glyph_margin_decoration_lane_count,
        line_numbers_left: pixels(line_numbers_left),
        line_numbers_width: pixels(line_numbers_width),
        decorations_left: pixels(decorations_left),
        decorations_width: pixels(line_decorations_width),
        content_left: pixels(content_left),
        content_width: pixels(content_width),
        minimap,
        viewport_column,
        is_word_wrap_minified,
        is_viewport_wrapping,
        wrapping_column,
        vertical_scrollbar_width: pixels(vertical_scrollbar_width),
        horizontal_scrollbar_height: pixels(horizontal_scrollbar_height),
        overview_ruler: OverviewRulerPosition {
            top: pixels(vertical_arrow_size),
            width: pixels(vertical_scrollbar_width),
            height: pixels(outer_height - 2. * vertical_arrow_size),
            right: px(0.),
        },
    }
}

#[cfg(test)]
mod tests;
