//! Monaco's `editorLayoutProvider.test.ts` (MIT), ported: the layout
//! `compute_layout` gives for each of its configurations.
//!
//! Generated from the Monaco test file; each test is one `doTest` call.

use super::*;
use crate::options::{
    EditorOptions, LineNumbers, MinimapOptions, MinimapSide, MinimapSize, WordWrap,
};

/// Monaco's `IEditorLayoutProviderOpts`.
struct Input {
    outer_width: f64,
    outer_height: f64,
    show_glyph_margin: bool,
    line_height: f64,
    show_line_numbers: bool,
    line_numbers_min_chars: u32,
    line_numbers_digit_count: u32,
    max_line_number: Option<u32>,
    line_decorations_width: f32,
    typical_halfwidth_character_width: f64,
    max_digit_width: f64,
    vertical_scrollbar_width: f32,
    vertical_scrollbar_has_arrows: bool,
    scrollbar_arrow_size: f32,
    horizontal_scrollbar_height: f32,
    minimap: bool,
    minimap_side: MinimapSide,
    minimap_render_characters: bool,
    minimap_max_column: u32,
    minimap_size: MinimapSize,
    pixel_ratio: f64,
    word_wrap: WordWrap,
    word_wrap_column: u32,
    word_wrap_indicator: bool,
}

impl Default for Input {
    fn default() -> Self {
        Self {
            outer_width: 1000.,
            outer_height: 800.,
            show_glyph_margin: false,
            line_height: 16.,
            show_line_numbers: false,
            line_numbers_min_chars: 0,
            line_numbers_digit_count: 1,
            max_line_number: None,
            line_decorations_width: 10.,
            typical_halfwidth_character_width: 10.,
            max_digit_width: 10.,
            vertical_scrollbar_width: 0.,
            vertical_scrollbar_has_arrows: false,
            scrollbar_arrow_size: 0.,
            horizontal_scrollbar_height: 0.,
            minimap: false,
            minimap_side: MinimapSide::Right,
            minimap_render_characters: true,
            minimap_max_column: 150,
            minimap_size: MinimapSize::Proportional,
            pixel_ratio: 1.,
            word_wrap: WordWrap::Off,
            word_wrap_column: 80,
            word_wrap_indicator: false,
        }
    }
}

/// Monaco's `computeLayoutInfo`: folding off, no padding, minimap scale 1.
fn compute(input: Input) -> LayoutInfo {
    let mut options = EditorOptions {
        glyph_margin: input.show_glyph_margin,
        line_numbers: if input.show_line_numbers {
            LineNumbers::On
        } else {
            LineNumbers::Off
        },
        line_numbers_min_chars: input.line_numbers_min_chars,
        line_decorations_width: px(input.line_decorations_width).into(),
        folding: false,
        minimap: MinimapOptions {
            enabled: input.minimap,
            size: input.minimap_size,
            side: input.minimap_side,
            render_characters: input.minimap_render_characters,
            max_column: input.minimap_max_column,
            scale: 1.,
            ..MinimapOptions::default()
        },
        vertical_scrollbar_size: px(input.vertical_scrollbar_width),
        horizontal_scrollbar_size: px(input.horizontal_scrollbar_height),
        word_wrap: input.word_wrap,
        word_wrap_column: input.word_wrap_column,
        word_wrap_indicator: input.word_wrap_indicator,
        ..EditorOptions::default()
    };
    // Monaco's test leaves `scrollBeyondLastLine` unset, so off.
    options.scroll_beyond_last_line = false;
    options.padding = Default::default();
    options.scrollbar.arrow_size = px(input.scrollbar_arrow_size);
    options.scrollbar.vertical_has_arrows = input.vertical_scrollbar_has_arrows;
    let env = LayoutEnv {
        outer_width: input.outer_width,
        outer_height: input.outer_height,
        line_height: input.line_height,
        line_numbers_digit_count: input.line_numbers_digit_count,
        typical_halfwidth_character_width: input.typical_halfwidth_character_width,
        max_digit_width: input.max_digit_width,
        pixel_ratio: input.pixel_ratio,
        view_line_count: input
            .max_line_number
            .unwrap_or(10u32.pow(input.line_numbers_digit_count) - 1),
        glyph_margin_decoration_lane_count: 1,
        is_dominated_by_long_lines: false,
        screen_reader: false,
    };
    compute_layout(&options, &env, &mut LayoutMemory::default())
}

#[allow(clippy::too_many_arguments)]
fn minimap(
    render_minimap: RenderMinimap,
    minimap_left: f32,
    minimap_width: f32,
    minimap_height_is_editor_height: bool,
    minimap_is_sampling: bool,
    minimap_scale: f32,
    minimap_line_height: f32,
    minimap_canvas_inner_width: u32,
    minimap_canvas_inner_height: u32,
    minimap_canvas_outer_width: f32,
    minimap_canvas_outer_height: f32,
) -> MinimapLayoutInfo {
    MinimapLayoutInfo {
        render_minimap,
        minimap_left: px(minimap_left),
        minimap_width: px(minimap_width),
        minimap_height_is_editor_height,
        minimap_is_sampling,
        minimap_scale,
        minimap_line_height,
        minimap_canvas_inner_width,
        minimap_canvas_inner_height,
        minimap_canvas_outer_width: px(minimap_canvas_outer_width),
        minimap_canvas_outer_height: px(minimap_canvas_outer_height),
    }
}

/// Monaco `EditorLayoutProvider 1`.
#[test]
fn monaco_editorlayoutprovider_1() {
    let actual = compute(Input {
        outer_width: 1000.0,
        outer_height: 800.0,
        show_glyph_margin: false,
        line_height: 16.0,
        show_line_numbers: false,
        line_numbers_min_chars: 0,
        line_numbers_digit_count: 1,
        line_decorations_width: 10.0,
        typical_halfwidth_character_width: 10.0,
        max_digit_width: 10.0,
        vertical_scrollbar_width: 0.0,
        horizontal_scrollbar_height: 0.0,
        scrollbar_arrow_size: 0.0,
        vertical_scrollbar_has_arrows: false,
        minimap: false,
        minimap_side: MinimapSide::Right,
        minimap_render_characters: true,
        minimap_max_column: 150,
        pixel_ratio: 1.0,
        ..Input::default()
    });
    let expected = LayoutInfo {
        width: px(1000.0),
        height: px(800.0),
        glyph_margin_left: px(0.0),
        glyph_margin_width: px(0.0),
        glyph_margin_decoration_lane_count: 1,
        line_numbers_left: px(0.0),
        line_numbers_width: px(0.0),
        decorations_left: px(0.0),
        decorations_width: px(10.0),
        content_left: px(10.0),
        content_width: px(990.0),
        minimap: minimap(
            RenderMinimap::None,
            0.0,
            0.0,
            false,
            false,
            1.0,
            1.0,
            0,
            800,
            0.0,
            800.0,
        ),
        viewport_column: 98,
        is_word_wrap_minified: false,
        is_viewport_wrapping: false,
        wrapping_column: None,
        vertical_scrollbar_width: px(0.0),
        horizontal_scrollbar_height: px(0.0),
        overview_ruler: OverviewRulerPosition {
            top: px(0.0),
            width: px(0.0),
            height: px(800.0),
            right: px(0.0),
        },
    };
    assert_eq!(actual, expected);
}

/// Monaco `EditorLayoutProvider 1.1`.
#[test]
fn monaco_editorlayoutprovider_1_1() {
    let actual = compute(Input {
        outer_width: 1000.0,
        outer_height: 800.0,
        show_glyph_margin: false,
        line_height: 16.0,
        show_line_numbers: false,
        line_numbers_min_chars: 0,
        line_numbers_digit_count: 1,
        line_decorations_width: 10.0,
        typical_halfwidth_character_width: 10.0,
        max_digit_width: 10.0,
        vertical_scrollbar_width: 11.0,
        horizontal_scrollbar_height: 12.0,
        scrollbar_arrow_size: 13.0,
        vertical_scrollbar_has_arrows: true,
        minimap: false,
        minimap_side: MinimapSide::Right,
        minimap_render_characters: true,
        minimap_max_column: 150,
        pixel_ratio: 1.0,
        ..Input::default()
    });
    let expected = LayoutInfo {
        width: px(1000.0),
        height: px(800.0),
        glyph_margin_left: px(0.0),
        glyph_margin_width: px(0.0),
        glyph_margin_decoration_lane_count: 1,
        line_numbers_left: px(0.0),
        line_numbers_width: px(0.0),
        decorations_left: px(0.0),
        decorations_width: px(10.0),
        content_left: px(10.0),
        content_width: px(990.0),
        minimap: minimap(
            RenderMinimap::None,
            0.0,
            0.0,
            false,
            false,
            1.0,
            1.0,
            0,
            800,
            0.0,
            800.0,
        ),
        viewport_column: 97,
        is_word_wrap_minified: false,
        is_viewport_wrapping: false,
        wrapping_column: None,
        vertical_scrollbar_width: px(11.0),
        horizontal_scrollbar_height: px(12.0),
        overview_ruler: OverviewRulerPosition {
            top: px(13.0),
            width: px(11.0),
            height: px(774.0),
            right: px(0.0),
        },
    };
    assert_eq!(actual, expected);
}

/// Monaco `EditorLayoutProvider 2`.
#[test]
fn monaco_editorlayoutprovider_2() {
    let actual = compute(Input {
        outer_width: 900.0,
        outer_height: 800.0,
        show_glyph_margin: false,
        line_height: 16.0,
        show_line_numbers: false,
        line_numbers_min_chars: 0,
        line_numbers_digit_count: 1,
        line_decorations_width: 10.0,
        typical_halfwidth_character_width: 10.0,
        max_digit_width: 10.0,
        vertical_scrollbar_width: 0.0,
        horizontal_scrollbar_height: 0.0,
        scrollbar_arrow_size: 0.0,
        vertical_scrollbar_has_arrows: false,
        minimap: false,
        minimap_side: MinimapSide::Right,
        minimap_render_characters: true,
        minimap_max_column: 150,
        pixel_ratio: 1.0,
        ..Input::default()
    });
    let expected = LayoutInfo {
        width: px(900.0),
        height: px(800.0),
        glyph_margin_left: px(0.0),
        glyph_margin_width: px(0.0),
        glyph_margin_decoration_lane_count: 1,
        line_numbers_left: px(0.0),
        line_numbers_width: px(0.0),
        decorations_left: px(0.0),
        decorations_width: px(10.0),
        content_left: px(10.0),
        content_width: px(890.0),
        minimap: minimap(
            RenderMinimap::None,
            0.0,
            0.0,
            false,
            false,
            1.0,
            1.0,
            0,
            800,
            0.0,
            800.0,
        ),
        viewport_column: 88,
        is_word_wrap_minified: false,
        is_viewport_wrapping: false,
        wrapping_column: None,
        vertical_scrollbar_width: px(0.0),
        horizontal_scrollbar_height: px(0.0),
        overview_ruler: OverviewRulerPosition {
            top: px(0.0),
            width: px(0.0),
            height: px(800.0),
            right: px(0.0),
        },
    };
    assert_eq!(actual, expected);
}

/// Monaco `EditorLayoutProvider 3`.
#[test]
fn monaco_editorlayoutprovider_3() {
    let actual = compute(Input {
        outer_width: 900.0,
        outer_height: 900.0,
        show_glyph_margin: false,
        line_height: 16.0,
        show_line_numbers: false,
        line_numbers_min_chars: 0,
        line_numbers_digit_count: 1,
        line_decorations_width: 10.0,
        typical_halfwidth_character_width: 10.0,
        max_digit_width: 10.0,
        vertical_scrollbar_width: 0.0,
        horizontal_scrollbar_height: 0.0,
        scrollbar_arrow_size: 0.0,
        vertical_scrollbar_has_arrows: false,
        minimap: false,
        minimap_side: MinimapSide::Right,
        minimap_render_characters: true,
        minimap_max_column: 150,
        pixel_ratio: 1.0,
        ..Input::default()
    });
    let expected = LayoutInfo {
        width: px(900.0),
        height: px(900.0),
        glyph_margin_left: px(0.0),
        glyph_margin_width: px(0.0),
        glyph_margin_decoration_lane_count: 1,
        line_numbers_left: px(0.0),
        line_numbers_width: px(0.0),
        decorations_left: px(0.0),
        decorations_width: px(10.0),
        content_left: px(10.0),
        content_width: px(890.0),
        minimap: minimap(
            RenderMinimap::None,
            0.0,
            0.0,
            false,
            false,
            1.0,
            1.0,
            0,
            900,
            0.0,
            900.0,
        ),
        viewport_column: 88,
        is_word_wrap_minified: false,
        is_viewport_wrapping: false,
        wrapping_column: None,
        vertical_scrollbar_width: px(0.0),
        horizontal_scrollbar_height: px(0.0),
        overview_ruler: OverviewRulerPosition {
            top: px(0.0),
            width: px(0.0),
            height: px(900.0),
            right: px(0.0),
        },
    };
    assert_eq!(actual, expected);
}

/// Monaco `EditorLayoutProvider 4`.
#[test]
fn monaco_editorlayoutprovider_4() {
    let actual = compute(Input {
        outer_width: 900.0,
        outer_height: 900.0,
        show_glyph_margin: false,
        line_height: 16.0,
        show_line_numbers: false,
        line_numbers_min_chars: 5,
        line_numbers_digit_count: 1,
        line_decorations_width: 10.0,
        typical_halfwidth_character_width: 10.0,
        max_digit_width: 10.0,
        vertical_scrollbar_width: 0.0,
        horizontal_scrollbar_height: 0.0,
        scrollbar_arrow_size: 0.0,
        vertical_scrollbar_has_arrows: false,
        minimap: false,
        minimap_side: MinimapSide::Right,
        minimap_render_characters: true,
        minimap_max_column: 150,
        pixel_ratio: 1.0,
        ..Input::default()
    });
    let expected = LayoutInfo {
        width: px(900.0),
        height: px(900.0),
        glyph_margin_left: px(0.0),
        glyph_margin_width: px(0.0),
        glyph_margin_decoration_lane_count: 1,
        line_numbers_left: px(0.0),
        line_numbers_width: px(0.0),
        decorations_left: px(0.0),
        decorations_width: px(10.0),
        content_left: px(10.0),
        content_width: px(890.0),
        minimap: minimap(
            RenderMinimap::None,
            0.0,
            0.0,
            false,
            false,
            1.0,
            1.0,
            0,
            900,
            0.0,
            900.0,
        ),
        viewport_column: 88,
        is_word_wrap_minified: false,
        is_viewport_wrapping: false,
        wrapping_column: None,
        vertical_scrollbar_width: px(0.0),
        horizontal_scrollbar_height: px(0.0),
        overview_ruler: OverviewRulerPosition {
            top: px(0.0),
            width: px(0.0),
            height: px(900.0),
            right: px(0.0),
        },
    };
    assert_eq!(actual, expected);
}

/// Monaco `EditorLayoutProvider 5`.
#[test]
fn monaco_editorlayoutprovider_5() {
    let actual = compute(Input {
        outer_width: 900.0,
        outer_height: 900.0,
        show_glyph_margin: false,
        line_height: 16.0,
        show_line_numbers: true,
        line_numbers_min_chars: 5,
        line_numbers_digit_count: 1,
        line_decorations_width: 10.0,
        typical_halfwidth_character_width: 10.0,
        max_digit_width: 10.0,
        vertical_scrollbar_width: 0.0,
        horizontal_scrollbar_height: 0.0,
        scrollbar_arrow_size: 0.0,
        vertical_scrollbar_has_arrows: false,
        minimap: false,
        minimap_side: MinimapSide::Right,
        minimap_render_characters: true,
        minimap_max_column: 150,
        pixel_ratio: 1.0,
        ..Input::default()
    });
    let expected = LayoutInfo {
        width: px(900.0),
        height: px(900.0),
        glyph_margin_left: px(0.0),
        glyph_margin_width: px(0.0),
        glyph_margin_decoration_lane_count: 1,
        line_numbers_left: px(0.0),
        line_numbers_width: px(50.0),
        decorations_left: px(50.0),
        decorations_width: px(10.0),
        content_left: px(60.0),
        content_width: px(840.0),
        minimap: minimap(
            RenderMinimap::None,
            0.0,
            0.0,
            false,
            false,
            1.0,
            1.0,
            0,
            900,
            0.0,
            900.0,
        ),
        viewport_column: 83,
        is_word_wrap_minified: false,
        is_viewport_wrapping: false,
        wrapping_column: None,
        vertical_scrollbar_width: px(0.0),
        horizontal_scrollbar_height: px(0.0),
        overview_ruler: OverviewRulerPosition {
            top: px(0.0),
            width: px(0.0),
            height: px(900.0),
            right: px(0.0),
        },
    };
    assert_eq!(actual, expected);
}

/// Monaco `EditorLayoutProvider 6`.
#[test]
fn monaco_editorlayoutprovider_6() {
    let actual = compute(Input {
        outer_width: 900.0,
        outer_height: 900.0,
        show_glyph_margin: false,
        line_height: 16.0,
        show_line_numbers: true,
        line_numbers_min_chars: 5,
        line_numbers_digit_count: 5,
        line_decorations_width: 10.0,
        typical_halfwidth_character_width: 10.0,
        max_digit_width: 10.0,
        vertical_scrollbar_width: 0.0,
        horizontal_scrollbar_height: 0.0,
        scrollbar_arrow_size: 0.0,
        vertical_scrollbar_has_arrows: false,
        minimap: false,
        minimap_side: MinimapSide::Right,
        minimap_render_characters: true,
        minimap_max_column: 150,
        pixel_ratio: 1.0,
        ..Input::default()
    });
    let expected = LayoutInfo {
        width: px(900.0),
        height: px(900.0),
        glyph_margin_left: px(0.0),
        glyph_margin_width: px(0.0),
        glyph_margin_decoration_lane_count: 1,
        line_numbers_left: px(0.0),
        line_numbers_width: px(50.0),
        decorations_left: px(50.0),
        decorations_width: px(10.0),
        content_left: px(60.0),
        content_width: px(840.0),
        minimap: minimap(
            RenderMinimap::None,
            0.0,
            0.0,
            false,
            false,
            1.0,
            1.0,
            0,
            900,
            0.0,
            900.0,
        ),
        viewport_column: 83,
        is_word_wrap_minified: false,
        is_viewport_wrapping: false,
        wrapping_column: None,
        vertical_scrollbar_width: px(0.0),
        horizontal_scrollbar_height: px(0.0),
        overview_ruler: OverviewRulerPosition {
            top: px(0.0),
            width: px(0.0),
            height: px(900.0),
            right: px(0.0),
        },
    };
    assert_eq!(actual, expected);
}

/// Monaco `EditorLayoutProvider 7`.
#[test]
fn monaco_editorlayoutprovider_7() {
    let actual = compute(Input {
        outer_width: 900.0,
        outer_height: 900.0,
        show_glyph_margin: false,
        line_height: 16.0,
        show_line_numbers: true,
        line_numbers_min_chars: 5,
        line_numbers_digit_count: 6,
        line_decorations_width: 10.0,
        typical_halfwidth_character_width: 10.0,
        max_digit_width: 10.0,
        vertical_scrollbar_width: 0.0,
        horizontal_scrollbar_height: 0.0,
        scrollbar_arrow_size: 0.0,
        vertical_scrollbar_has_arrows: false,
        minimap: false,
        minimap_side: MinimapSide::Right,
        minimap_render_characters: true,
        minimap_max_column: 150,
        pixel_ratio: 1.0,
        ..Input::default()
    });
    let expected = LayoutInfo {
        width: px(900.0),
        height: px(900.0),
        glyph_margin_left: px(0.0),
        glyph_margin_width: px(0.0),
        glyph_margin_decoration_lane_count: 1,
        line_numbers_left: px(0.0),
        line_numbers_width: px(60.0),
        decorations_left: px(60.0),
        decorations_width: px(10.0),
        content_left: px(70.0),
        content_width: px(830.0),
        minimap: minimap(
            RenderMinimap::None,
            0.0,
            0.0,
            false,
            false,
            1.0,
            1.0,
            0,
            900,
            0.0,
            900.0,
        ),
        viewport_column: 82,
        is_word_wrap_minified: false,
        is_viewport_wrapping: false,
        wrapping_column: None,
        vertical_scrollbar_width: px(0.0),
        horizontal_scrollbar_height: px(0.0),
        overview_ruler: OverviewRulerPosition {
            top: px(0.0),
            width: px(0.0),
            height: px(900.0),
            right: px(0.0),
        },
    };
    assert_eq!(actual, expected);
}

/// Monaco `EditorLayoutProvider 8`.
#[test]
fn monaco_editorlayoutprovider_8() {
    let actual = compute(Input {
        outer_width: 900.0,
        outer_height: 900.0,
        show_glyph_margin: false,
        line_height: 16.0,
        show_line_numbers: true,
        line_numbers_min_chars: 5,
        line_numbers_digit_count: 6,
        line_decorations_width: 10.0,
        typical_halfwidth_character_width: 5.0,
        max_digit_width: 5.0,
        vertical_scrollbar_width: 0.0,
        horizontal_scrollbar_height: 0.0,
        scrollbar_arrow_size: 0.0,
        vertical_scrollbar_has_arrows: false,
        minimap: false,
        minimap_side: MinimapSide::Right,
        minimap_render_characters: true,
        minimap_max_column: 150,
        pixel_ratio: 1.0,
        ..Input::default()
    });
    let expected = LayoutInfo {
        width: px(900.0),
        height: px(900.0),
        glyph_margin_left: px(0.0),
        glyph_margin_width: px(0.0),
        glyph_margin_decoration_lane_count: 1,
        line_numbers_left: px(0.0),
        line_numbers_width: px(30.0),
        decorations_left: px(30.0),
        decorations_width: px(10.0),
        content_left: px(40.0),
        content_width: px(860.0),
        minimap: minimap(
            RenderMinimap::None,
            0.0,
            0.0,
            false,
            false,
            1.0,
            1.0,
            0,
            900,
            0.0,
            900.0,
        ),
        viewport_column: 171,
        is_word_wrap_minified: false,
        is_viewport_wrapping: false,
        wrapping_column: None,
        vertical_scrollbar_width: px(0.0),
        horizontal_scrollbar_height: px(0.0),
        overview_ruler: OverviewRulerPosition {
            top: px(0.0),
            width: px(0.0),
            height: px(900.0),
            right: px(0.0),
        },
    };
    assert_eq!(actual, expected);
}

/// Monaco `EditorLayoutProvider 8 - rounds floats`.
#[test]
fn monaco_editorlayoutprovider_8_rounds_floats() {
    let actual = compute(Input {
        outer_width: 900.0,
        outer_height: 900.0,
        show_glyph_margin: false,
        line_height: 16.0,
        show_line_numbers: true,
        line_numbers_min_chars: 5,
        line_numbers_digit_count: 6,
        line_decorations_width: 10.0,
        typical_halfwidth_character_width: 5.05,
        max_digit_width: 5.05,
        vertical_scrollbar_width: 0.0,
        horizontal_scrollbar_height: 0.0,
        scrollbar_arrow_size: 0.0,
        vertical_scrollbar_has_arrows: false,
        minimap: false,
        minimap_side: MinimapSide::Right,
        minimap_render_characters: true,
        minimap_max_column: 150,
        pixel_ratio: 1.0,
        ..Input::default()
    });
    let expected = LayoutInfo {
        width: px(900.0),
        height: px(900.0),
        glyph_margin_left: px(0.0),
        glyph_margin_width: px(0.0),
        glyph_margin_decoration_lane_count: 1,
        line_numbers_left: px(0.0),
        line_numbers_width: px(30.0),
        decorations_left: px(30.0),
        decorations_width: px(10.0),
        content_left: px(40.0),
        content_width: px(860.0),
        minimap: minimap(
            RenderMinimap::None,
            0.0,
            0.0,
            false,
            false,
            1.0,
            1.0,
            0,
            900,
            0.0,
            900.0,
        ),
        viewport_column: 169,
        is_word_wrap_minified: false,
        is_viewport_wrapping: false,
        wrapping_column: None,
        vertical_scrollbar_width: px(0.0),
        horizontal_scrollbar_height: px(0.0),
        overview_ruler: OverviewRulerPosition {
            top: px(0.0),
            width: px(0.0),
            height: px(900.0),
            right: px(0.0),
        },
    };
    assert_eq!(actual, expected);
}

/// Monaco `EditorLayoutProvider 9 - render minimap`.
#[test]
fn monaco_editorlayoutprovider_9_render_minimap() {
    let actual = compute(Input {
        outer_width: 1000.0,
        outer_height: 800.0,
        show_glyph_margin: false,
        line_height: 16.0,
        show_line_numbers: false,
        line_numbers_min_chars: 0,
        line_numbers_digit_count: 1,
        line_decorations_width: 10.0,
        typical_halfwidth_character_width: 10.0,
        max_digit_width: 10.0,
        vertical_scrollbar_width: 0.0,
        horizontal_scrollbar_height: 0.0,
        scrollbar_arrow_size: 0.0,
        vertical_scrollbar_has_arrows: false,
        minimap: true,
        minimap_side: MinimapSide::Right,
        minimap_render_characters: true,
        minimap_max_column: 150,
        pixel_ratio: 1.0,
        ..Input::default()
    });
    let expected = LayoutInfo {
        width: px(1000.0),
        height: px(800.0),
        glyph_margin_left: px(0.0),
        glyph_margin_width: px(0.0),
        glyph_margin_decoration_lane_count: 1,
        line_numbers_left: px(0.0),
        line_numbers_width: px(0.0),
        decorations_left: px(0.0),
        decorations_width: px(10.0),
        content_left: px(10.0),
        content_width: px(893.0),
        minimap: minimap(
            RenderMinimap::Text,
            903.0,
            97.0,
            false,
            false,
            1.0,
            2.0,
            97,
            800,
            97.0,
            800.0,
        ),
        viewport_column: 89,
        is_word_wrap_minified: false,
        is_viewport_wrapping: false,
        wrapping_column: None,
        vertical_scrollbar_width: px(0.0),
        horizontal_scrollbar_height: px(0.0),
        overview_ruler: OverviewRulerPosition {
            top: px(0.0),
            width: px(0.0),
            height: px(800.0),
            right: px(0.0),
        },
    };
    assert_eq!(actual, expected);
}

/// Monaco `EditorLayoutProvider 9 - render minimap with pixelRatio = 2`.
#[test]
fn monaco_editorlayoutprovider_9_render_minimap_with_pixelratio_2() {
    let actual = compute(Input {
        outer_width: 1000.0,
        outer_height: 800.0,
        show_glyph_margin: false,
        line_height: 16.0,
        show_line_numbers: false,
        line_numbers_min_chars: 0,
        line_numbers_digit_count: 1,
        line_decorations_width: 10.0,
        typical_halfwidth_character_width: 10.0,
        max_digit_width: 10.0,
        vertical_scrollbar_width: 0.0,
        horizontal_scrollbar_height: 0.0,
        scrollbar_arrow_size: 0.0,
        vertical_scrollbar_has_arrows: false,
        minimap: true,
        minimap_side: MinimapSide::Right,
        minimap_render_characters: true,
        minimap_max_column: 150,
        pixel_ratio: 2.0,
        ..Input::default()
    });
    let expected = LayoutInfo {
        width: px(1000.0),
        height: px(800.0),
        glyph_margin_left: px(0.0),
        glyph_margin_width: px(0.0),
        glyph_margin_decoration_lane_count: 1,
        line_numbers_left: px(0.0),
        line_numbers_width: px(0.0),
        decorations_left: px(0.0),
        decorations_width: px(10.0),
        content_left: px(10.0),
        content_width: px(893.0),
        minimap: minimap(
            RenderMinimap::Text,
            903.0,
            97.0,
            false,
            false,
            2.0,
            4.0,
            194,
            1600,
            97.0,
            800.0,
        ),
        viewport_column: 89,
        is_word_wrap_minified: false,
        is_viewport_wrapping: false,
        wrapping_column: None,
        vertical_scrollbar_width: px(0.0),
        horizontal_scrollbar_height: px(0.0),
        overview_ruler: OverviewRulerPosition {
            top: px(0.0),
            width: px(0.0),
            height: px(800.0),
            right: px(0.0),
        },
    };
    assert_eq!(actual, expected);
}

/// Monaco `EditorLayoutProvider 9 - render minimap with pixelRatio = 4`.
#[test]
fn monaco_editorlayoutprovider_9_render_minimap_with_pixelratio_4() {
    let actual = compute(Input {
        outer_width: 1000.0,
        outer_height: 800.0,
        show_glyph_margin: false,
        line_height: 16.0,
        show_line_numbers: false,
        line_numbers_min_chars: 0,
        line_numbers_digit_count: 1,
        line_decorations_width: 10.0,
        typical_halfwidth_character_width: 10.0,
        max_digit_width: 10.0,
        vertical_scrollbar_width: 0.0,
        horizontal_scrollbar_height: 0.0,
        scrollbar_arrow_size: 0.0,
        vertical_scrollbar_has_arrows: false,
        minimap: true,
        minimap_side: MinimapSide::Right,
        minimap_render_characters: true,
        minimap_max_column: 150,
        pixel_ratio: 4.0,
        ..Input::default()
    });
    let expected = LayoutInfo {
        width: px(1000.0),
        height: px(800.0),
        glyph_margin_left: px(0.0),
        glyph_margin_width: px(0.0),
        glyph_margin_decoration_lane_count: 1,
        line_numbers_left: px(0.0),
        line_numbers_width: px(0.0),
        decorations_left: px(0.0),
        decorations_width: px(10.0),
        content_left: px(10.0),
        content_width: px(935.0),
        minimap: minimap(
            RenderMinimap::Text,
            945.0,
            55.0,
            false,
            false,
            2.0,
            4.0,
            220,
            3200,
            55.0,
            800.0,
        ),
        viewport_column: 93,
        is_word_wrap_minified: false,
        is_viewport_wrapping: false,
        wrapping_column: None,
        vertical_scrollbar_width: px(0.0),
        horizontal_scrollbar_height: px(0.0),
        overview_ruler: OverviewRulerPosition {
            top: px(0.0),
            width: px(0.0),
            height: px(800.0),
            right: px(0.0),
        },
    };
    assert_eq!(actual, expected);
}

/// Monaco `EditorLayoutProvider 10 - render minimap to left`.
#[test]
fn monaco_editorlayoutprovider_10_render_minimap_to_left() {
    let actual = compute(Input {
        outer_width: 1000.0,
        outer_height: 800.0,
        show_glyph_margin: false,
        line_height: 16.0,
        show_line_numbers: false,
        line_numbers_min_chars: 0,
        line_numbers_digit_count: 1,
        line_decorations_width: 10.0,
        typical_halfwidth_character_width: 10.0,
        max_digit_width: 10.0,
        vertical_scrollbar_width: 0.0,
        horizontal_scrollbar_height: 0.0,
        scrollbar_arrow_size: 0.0,
        vertical_scrollbar_has_arrows: false,
        minimap: true,
        minimap_side: MinimapSide::Left,
        minimap_render_characters: true,
        minimap_max_column: 150,
        pixel_ratio: 4.0,
        ..Input::default()
    });
    let expected = LayoutInfo {
        width: px(1000.0),
        height: px(800.0),
        glyph_margin_left: px(55.0),
        glyph_margin_width: px(0.0),
        glyph_margin_decoration_lane_count: 1,
        line_numbers_left: px(55.0),
        line_numbers_width: px(0.0),
        decorations_left: px(55.0),
        decorations_width: px(10.0),
        content_left: px(65.0),
        content_width: px(935.0),
        minimap: minimap(
            RenderMinimap::Text,
            0.0,
            55.0,
            false,
            false,
            2.0,
            4.0,
            220,
            3200,
            55.0,
            800.0,
        ),
        viewport_column: 93,
        is_word_wrap_minified: false,
        is_viewport_wrapping: false,
        wrapping_column: None,
        vertical_scrollbar_width: px(0.0),
        horizontal_scrollbar_height: px(0.0),
        overview_ruler: OverviewRulerPosition {
            top: px(0.0),
            width: px(0.0),
            height: px(800.0),
            right: px(0.0),
        },
    };
    assert_eq!(actual, expected);
}

/// Monaco `EditorLayoutProvider 11 - minimap mode cover without sampling`.
#[test]
fn monaco_editorlayoutprovider_11_minimap_mode_cover_without_sampling() {
    let actual = compute(Input {
        outer_width: 1000.0,
        outer_height: 800.0,
        show_glyph_margin: false,
        line_height: 16.0,
        show_line_numbers: false,
        line_numbers_min_chars: 0,
        line_numbers_digit_count: 3,
        max_line_number: Some(120),
        line_decorations_width: 10.0,
        typical_halfwidth_character_width: 10.0,
        max_digit_width: 10.0,
        vertical_scrollbar_width: 0.0,
        horizontal_scrollbar_height: 0.0,
        scrollbar_arrow_size: 0.0,
        vertical_scrollbar_has_arrows: false,
        minimap: true,
        minimap_side: MinimapSide::Right,
        minimap_render_characters: true,
        minimap_max_column: 150,
        minimap_size: MinimapSize::Fill,
        pixel_ratio: 2.0,
        ..Input::default()
    });
    let expected = LayoutInfo {
        width: px(1000.0),
        height: px(800.0),
        glyph_margin_left: px(0.0),
        glyph_margin_width: px(0.0),
        glyph_margin_decoration_lane_count: 1,
        line_numbers_left: px(0.0),
        line_numbers_width: px(0.0),
        decorations_left: px(0.0),
        decorations_width: px(10.0),
        content_left: px(10.0),
        content_width: px(893.0),
        minimap: minimap(
            RenderMinimap::Text,
            903.0,
            97.0,
            true,
            false,
            3.0,
            13.0,
            291,
            1560,
            97.0,
            800.0,
        ),
        viewport_column: 89,
        is_word_wrap_minified: false,
        is_viewport_wrapping: false,
        wrapping_column: None,
        vertical_scrollbar_width: px(0.0),
        horizontal_scrollbar_height: px(0.0),
        overview_ruler: OverviewRulerPosition {
            top: px(0.0),
            width: px(0.0),
            height: px(800.0),
            right: px(0.0),
        },
    };
    assert_eq!(actual, expected);
}

/// Monaco `EditorLayoutProvider 12 - minimap mode cover with sampling`.
#[test]
fn monaco_editorlayoutprovider_12_minimap_mode_cover_with_sampling() {
    let actual = compute(Input {
        outer_width: 1000.0,
        outer_height: 800.0,
        show_glyph_margin: false,
        line_height: 16.0,
        show_line_numbers: false,
        line_numbers_min_chars: 0,
        line_numbers_digit_count: 4,
        max_line_number: Some(2500),
        line_decorations_width: 10.0,
        typical_halfwidth_character_width: 10.0,
        max_digit_width: 10.0,
        vertical_scrollbar_width: 0.0,
        horizontal_scrollbar_height: 0.0,
        scrollbar_arrow_size: 0.0,
        vertical_scrollbar_has_arrows: false,
        minimap: true,
        minimap_side: MinimapSide::Right,
        minimap_render_characters: true,
        minimap_max_column: 150,
        minimap_size: MinimapSize::Fill,
        pixel_ratio: 2.0,
        ..Input::default()
    });
    let expected = LayoutInfo {
        width: px(1000.0),
        height: px(800.0),
        glyph_margin_left: px(0.0),
        glyph_margin_width: px(0.0),
        glyph_margin_decoration_lane_count: 1,
        line_numbers_left: px(0.0),
        line_numbers_width: px(0.0),
        decorations_left: px(0.0),
        decorations_width: px(10.0),
        content_left: px(10.0),
        content_width: px(935.0),
        minimap: minimap(
            RenderMinimap::Text,
            945.0,
            55.0,
            true,
            true,
            1.0,
            1.0,
            110,
            1600,
            55.0,
            800.0,
        ),
        viewport_column: 93,
        is_word_wrap_minified: false,
        is_viewport_wrapping: false,
        wrapping_column: None,
        vertical_scrollbar_width: px(0.0),
        horizontal_scrollbar_height: px(0.0),
        overview_ruler: OverviewRulerPosition {
            top: px(0.0),
            width: px(0.0),
            height: px(800.0),
            right: px(0.0),
        },
    };
    assert_eq!(actual, expected);
}

/// Monaco `EditorLayoutProvider 13 - minimap mode contain without sampling`.
#[test]
fn monaco_editorlayoutprovider_13_minimap_mode_contain_without_sampling() {
    let actual = compute(Input {
        outer_width: 1000.0,
        outer_height: 800.0,
        show_glyph_margin: false,
        line_height: 16.0,
        show_line_numbers: false,
        line_numbers_min_chars: 0,
        line_numbers_digit_count: 3,
        max_line_number: Some(120),
        line_decorations_width: 10.0,
        typical_halfwidth_character_width: 10.0,
        max_digit_width: 10.0,
        vertical_scrollbar_width: 0.0,
        horizontal_scrollbar_height: 0.0,
        scrollbar_arrow_size: 0.0,
        vertical_scrollbar_has_arrows: false,
        minimap: true,
        minimap_side: MinimapSide::Right,
        minimap_render_characters: true,
        minimap_max_column: 150,
        minimap_size: MinimapSize::Fit,
        pixel_ratio: 2.0,
        ..Input::default()
    });
    let expected = LayoutInfo {
        width: px(1000.0),
        height: px(800.0),
        glyph_margin_left: px(0.0),
        glyph_margin_width: px(0.0),
        glyph_margin_decoration_lane_count: 1,
        line_numbers_left: px(0.0),
        line_numbers_width: px(0.0),
        decorations_left: px(0.0),
        decorations_width: px(10.0),
        content_left: px(10.0),
        content_width: px(893.0),
        minimap: minimap(
            RenderMinimap::Text,
            903.0,
            97.0,
            false,
            false,
            2.0,
            4.0,
            194,
            1600,
            97.0,
            800.0,
        ),
        viewport_column: 89,
        is_word_wrap_minified: false,
        is_viewport_wrapping: false,
        wrapping_column: None,
        vertical_scrollbar_width: px(0.0),
        horizontal_scrollbar_height: px(0.0),
        overview_ruler: OverviewRulerPosition {
            top: px(0.0),
            width: px(0.0),
            height: px(800.0),
            right: px(0.0),
        },
    };
    assert_eq!(actual, expected);
}

/// Monaco `EditorLayoutProvider 14 - minimap mode contain with sampling`.
#[test]
fn monaco_editorlayoutprovider_14_minimap_mode_contain_with_sampling() {
    let actual = compute(Input {
        outer_width: 1000.0,
        outer_height: 800.0,
        show_glyph_margin: false,
        line_height: 16.0,
        show_line_numbers: false,
        line_numbers_min_chars: 0,
        line_numbers_digit_count: 4,
        max_line_number: Some(2500),
        line_decorations_width: 10.0,
        typical_halfwidth_character_width: 10.0,
        max_digit_width: 10.0,
        vertical_scrollbar_width: 0.0,
        horizontal_scrollbar_height: 0.0,
        scrollbar_arrow_size: 0.0,
        vertical_scrollbar_has_arrows: false,
        minimap: true,
        minimap_side: MinimapSide::Right,
        minimap_render_characters: true,
        minimap_max_column: 150,
        minimap_size: MinimapSize::Fit,
        pixel_ratio: 2.0,
        ..Input::default()
    });
    let expected = LayoutInfo {
        width: px(1000.0),
        height: px(800.0),
        glyph_margin_left: px(0.0),
        glyph_margin_width: px(0.0),
        glyph_margin_decoration_lane_count: 1,
        line_numbers_left: px(0.0),
        line_numbers_width: px(0.0),
        decorations_left: px(0.0),
        decorations_width: px(10.0),
        content_left: px(10.0),
        content_width: px(935.0),
        minimap: minimap(
            RenderMinimap::Text,
            945.0,
            55.0,
            true,
            true,
            1.0,
            1.0,
            110,
            1600,
            55.0,
            800.0,
        ),
        viewport_column: 93,
        is_word_wrap_minified: false,
        is_viewport_wrapping: false,
        wrapping_column: None,
        vertical_scrollbar_width: px(0.0),
        horizontal_scrollbar_height: px(0.0),
        overview_ruler: OverviewRulerPosition {
            top: px(0.0),
            width: px(0.0),
            height: px(800.0),
            right: px(0.0),
        },
    };
    assert_eq!(actual, expected);
}

/// Monaco `issue #31312: When wrapping, leave 2px for the cursor`.
#[test]
fn monaco_issue_31312_when_wrapping_leave_2px_for_the_cursor() {
    let actual = compute(Input {
        outer_width: 1201.0,
        outer_height: 422.0,
        show_glyph_margin: true,
        line_height: 30.0,
        show_line_numbers: true,
        line_numbers_min_chars: 3,
        line_numbers_digit_count: 1,
        line_decorations_width: 26.0,
        typical_halfwidth_character_width: 12.04296875,
        max_digit_width: 12.04296875,
        vertical_scrollbar_width: 14.0,
        horizontal_scrollbar_height: 10.0,
        scrollbar_arrow_size: 11.0,
        vertical_scrollbar_has_arrows: false,
        minimap: true,
        minimap_side: MinimapSide::Right,
        minimap_render_characters: true,
        minimap_max_column: 120,
        pixel_ratio: 2.0,
        ..Input::default()
    });
    let expected = LayoutInfo {
        width: px(1201.0),
        height: px(422.0),
        glyph_margin_left: px(0.0),
        glyph_margin_width: px(30.0),
        glyph_margin_decoration_lane_count: 1,
        line_numbers_left: px(30.0),
        line_numbers_width: px(36.0),
        decorations_left: px(66.0),
        decorations_width: px(26.0),
        content_left: px(92.0),
        content_width: px(1018.0),
        minimap: minimap(
            RenderMinimap::Text,
            1096.0,
            91.0,
            false,
            false,
            2.0,
            4.0,
            182,
            844,
            91.0,
            422.0,
        ),
        viewport_column: 83,
        is_word_wrap_minified: false,
        is_viewport_wrapping: false,
        wrapping_column: None,
        vertical_scrollbar_width: px(14.0),
        horizontal_scrollbar_height: px(10.0),
        overview_ruler: OverviewRulerPosition {
            top: px(0.0),
            width: px(14.0),
            height: px(422.0),
            right: px(0.0),
        },
    };
    assert_eq!(actual, expected);
}

/// Monaco `leaves the rightmost column for the word wrap indicator when
/// wrapping to the viewport`.
#[test]
fn monaco_leaves_the_rightmost_column_for_the_word_wrap_indicator() {
    let wrapping_of = |word_wrap, word_wrap_indicator, word_wrap_column| {
        let info = compute(Input {
            word_wrap,
            word_wrap_indicator,
            word_wrap_column,
            ..Input::default()
        });
        (info.viewport_column, info.wrapping_column)
    };
    assert_eq!(wrapping_of(WordWrap::Off, true, 40), (98, None));
    assert_eq!(wrapping_of(WordWrap::On, false, 40), (98, Some(98)));
    assert_eq!(wrapping_of(WordWrap::On, true, 40), (98, Some(97)));
    assert_eq!(wrapping_of(WordWrap::Bounded, false, 40), (98, Some(40)));
    assert_eq!(wrapping_of(WordWrap::Bounded, true, 40), (98, Some(40)));
    assert_eq!(
        wrapping_of(WordWrap::WordWrapColumn, true, 40),
        (98, Some(40))
    );
    assert_eq!(
        wrapping_of(WordWrap::WordWrapColumn, true, 120),
        (98, Some(120))
    );
}

/// The options and environment of a default editor 1000x800 wide, with
/// 18px lines and 7px characters, and a glyph margin (VS Code's default).
fn default_layout(change: impl FnOnce(&mut EditorOptions)) -> LayoutInfo {
    let mut options = EditorOptions {
        glyph_margin: true,
        ..EditorOptions::default()
    };
    change(&mut options);
    let env = LayoutEnv {
        outer_width: 1000.,
        outer_height: 800.,
        line_height: 18.,
        line_numbers_digit_count: 3,
        typical_halfwidth_character_width: 7.,
        max_digit_width: 7.,
        pixel_ratio: 1.,
        view_line_count: 100,
        glyph_margin_decoration_lane_count: 1,
        is_dominated_by_long_lines: false,
        screen_reader: false,
    };
    compute_layout(&options.validated(), &env, &mut LayoutMemory::default())
}

#[test]
fn folding_controls_add_16px_unless_never_shown() {
    use crate::options::ShowFoldingControls;
    // Glyph margin (a line height), 5 digits of 7px, 10px of decorations
    // and 16px of folding controls.
    let info = default_layout(|_| {});
    assert_eq!(info.glyph_margin_width, px(18.));
    assert_eq!(info.line_numbers_width, px(35.));
    assert_eq!(info.decorations_width, px(26.));
    assert_eq!(info.content_left, px(79.));
    for show in [ShowFoldingControls::Always, ShowFoldingControls::Mouseover] {
        let info = default_layout(|o| o.show_folding_controls = show);
        assert_eq!(info.decorations_width, px(26.));
    }
    let info = default_layout(|o| o.show_folding_controls = ShowFoldingControls::Never);
    assert_eq!(info.decorations_width, px(10.));
    let info = default_layout(|o| o.folding = false);
    assert_eq!(info.decorations_width, px(10.));
}

#[test]
fn glyph_margin_is_a_line_height_per_lane() {
    let options = EditorOptions {
        folding: false,
        glyph_margin: true,
        ..EditorOptions::default()
    };
    let mut env = LayoutEnv {
        outer_width: 1000.,
        outer_height: 800.,
        line_height: 24.,
        line_numbers_digit_count: 1,
        typical_halfwidth_character_width: 7.,
        max_digit_width: 7.,
        pixel_ratio: 1.,
        view_line_count: 1,
        glyph_margin_decoration_lane_count: 1,
        is_dominated_by_long_lines: false,
        screen_reader: false,
    };
    let info = compute_layout(&options, &env, &mut LayoutMemory::default());
    assert_eq!(info.glyph_margin_width, px(24.));
    env.glyph_margin_decoration_lane_count = 2;
    let info = compute_layout(&options, &env, &mut LayoutMemory::default());
    assert_eq!(info.glyph_margin_width, px(48.));
    assert_eq!(info.line_numbers_left, px(48.));
}

#[test]
fn line_decorations_width_in_characters() {
    use crate::options::LineDecorationsWidth;
    // Monaco `"1.5ch"` with 7px characters: 10.5px, truncated.
    let info = default_layout(|o| {
        o.folding = false;
        o.line_decorations_width = LineDecorationsWidth::Chars(1.5);
    });
    assert_eq!(info.decorations_width, px(10.));
    let info = default_layout(|o| {
        o.folding = false;
        o.line_decorations_width = LineDecorationsWidth::Chars(4.);
    });
    assert_eq!(info.decorations_width, px(28.));
    // Clamped to 1000px.
    let info = default_layout(|o| {
        o.folding = false;
        o.line_decorations_width = LineDecorationsWidth::Chars(1000.);
    });
    assert_eq!(info.decorations_width, px(1000.));
}

#[test]
fn minimap_takes_its_share_of_the_width() {
    // Monaco: ((remaining - vbar - 2) * charWidth) / (typical + charWidth)
    // plus the 8px gutter: (921 - 14 - 2) * 1 / 8 = 113, so 121px...
    let info = default_layout(|o| o.minimap.max_column = 200);
    assert_eq!(info.minimap.minimap_width, px(121.));
    assert_eq!(info.content_width, px(921. - 121.));
    assert_eq!(info.minimap.minimap_left, px(1000. - 121. - 14.));
    // ...capped at `max_column` characters (120 by default).
    let info = default_layout(|_| {});
    assert_eq!(info.minimap.minimap_width, px(120.));
    let info = default_layout(|o| o.minimap.max_column = 40);
    assert_eq!(info.minimap.minimap_width, px(40.));
    // Scale 2 at pixel ratio 1: two pixels per column, (905 * 2) / 9 + 8.
    let info = default_layout(|o| o.minimap.scale = 2.);
    assert_eq!(info.minimap.minimap_width, px(209.));
    // A single-line editor has none.
    let info = default_layout(|o| o.single_line = true);
    assert_eq!(info.minimap.render_minimap, RenderMinimap::None);
    assert_eq!(info.minimap.minimap_width, px(0.));
}

#[test]
fn fill_remembers_its_scale_while_wrapping() {
    // Monaco's `ComputeOptionsMemory`: with `fill` and viewport wrapping,
    // a narrower editor keeps the scale it found.
    let options = EditorOptions {
        minimap: MinimapOptions {
            size: MinimapSize::Fill,
            ..MinimapOptions::default()
        },
        word_wrap: WordWrap::On,
        ..EditorOptions::default()
    };
    let env = LayoutEnv {
        outer_width: 1000.,
        outer_height: 800.,
        line_height: 16.,
        line_numbers_digit_count: 2,
        typical_halfwidth_character_width: 10.,
        max_digit_width: 10.,
        pixel_ratio: 2.,
        view_line_count: 20,
        glyph_margin_decoration_lane_count: 1,
        is_dominated_by_long_lines: false,
        screen_reader: false,
    };
    let mut memory = LayoutMemory::default();
    let first = compute_layout(&options, &env, &mut memory);
    assert!(first.minimap.minimap_height_is_editor_height);
    let narrower = LayoutEnv {
        outer_width: 990.,
        view_line_count: 400,
        ..env
    };
    let second = compute_layout(&options, &narrower, &mut memory);
    assert!(second.minimap.minimap_scale <= first.minimap.minimap_scale);
    assert_eq!(
        second.minimap.minimap_scale,
        compute_layout(&options, &narrower, &mut memory)
            .minimap
            .minimap_scale
    );
}

/// Monaco `editorConfiguration.test.ts`'s `TestWrappingConfiguration`: a
/// 1000x100 editor with the default options (VS Code's, so with a glyph
/// margin) and `TestConfiguration`'s font (19px lines, 10px characters),
/// returning `(isViewportWrapping, wrappingColumn)`.
fn wrapping(change: impl FnOnce(&mut EditorOptions)) -> (bool, Option<u32>) {
    let mut options = EditorOptions {
        glyph_margin: true,
        ..EditorOptions::default()
    };
    change(&mut options);
    let env = LayoutEnv {
        outer_width: 1000.,
        outer_height: 100.,
        line_height: 19.,
        line_numbers_digit_count: 1,
        typical_halfwidth_character_width: 10.,
        max_digit_width: 10.,
        pixel_ratio: 1.,
        view_line_count: 1,
        glyph_margin_decoration_lane_count: 1,
        is_dominated_by_long_lines: false,
        screen_reader: false,
    };
    let info = compute_layout(&options.validated(), &env, &mut LayoutMemory::default());
    (info.is_viewport_wrapping, info.wrapping_column)
}

/// Monaco `wordWrap default`, `wordWrap off` and `wordWrap off does not use
/// wordWrapColumn`.
#[test]
fn monaco_word_wrap_off() {
    assert_eq!(wrapping(|_| {}), (false, None));
    assert_eq!(wrapping(|o| o.word_wrap = WordWrap::Off), (false, None));
    assert_eq!(
        wrapping(|o| {
            o.word_wrap = WordWrap::Off;
            o.word_wrap_column = 10;
        }),
        (false, None)
    );
}

/// Monaco `wordWrap on`, `wordWrap on without minimap` and `wordWrap on
/// does not use wordWrapColumn`.
#[test]
fn monaco_word_wrap_on() {
    assert_eq!(wrapping(|o| o.word_wrap = WordWrap::On), (true, Some(80)));
    assert_eq!(
        wrapping(|o| {
            o.word_wrap = WordWrap::On;
            o.minimap.enabled = false;
        }),
        (true, Some(88))
    );
    assert_eq!(
        wrapping(|o| {
            o.word_wrap = WordWrap::On;
            o.word_wrap_column = 10;
        }),
        (true, Some(80))
    );
}

/// Monaco `wordWrap wordWrapColumn uses default wordWrapColumn`, `... uses
/// wordWrapColumn` and `... validates wordWrapColumn` (0 here, -1 there).
#[test]
fn monaco_word_wrap_column() {
    let column = |column| {
        wrapping(|o| {
            o.word_wrap = WordWrap::WordWrapColumn;
            if let Some(column) = column {
                o.word_wrap_column = column;
            }
        })
    };
    assert_eq!(column(None), (false, Some(80)));
    assert_eq!(column(Some(100)), (false, Some(100)));
    assert_eq!(column(Some(0)), (false, Some(1)));
}

/// Monaco `wordWrap bounded uses default wordWrapColumn`, `... uses
/// wordWrapColumn` and `... validates wordWrapColumn`.
#[test]
fn monaco_word_wrap_bounded() {
    let bounded = |column| {
        wrapping(|o| {
            o.word_wrap = WordWrap::Bounded;
            if let Some(column) = column {
                o.word_wrap_column = column;
            }
        })
    };
    assert_eq!(bounded(None), (true, Some(80)));
    assert_eq!(bounded(Some(40)), (true, Some(40)));
    assert_eq!(bounded(Some(0)), (true, Some(1)));
}
