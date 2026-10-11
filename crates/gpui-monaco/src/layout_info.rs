//! The editor's layout (Monaco `EditorLayoutInfo`,
//! `src/vs/editor/common/config/editorOptions.ts`, MIT), as
//! [`CodeEditor::layout_info`](crate::CodeEditor::layout_info) reports it
//! after a frame.
//!
//! Monaco computes it from the options before rendering
//! (`EditorLayoutInfoComputer`); here it's read off the last frame, so the
//! values are the ones actually drawn. Horizontal positions are relative to
//! the editor's left edge.

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
    /// `glyphMarginDecorationLaneCount`): there's one.
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
    /// Wrapping was forced for a screen reader (Monaco
    /// `isWordWrapMinified`); never here.
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
