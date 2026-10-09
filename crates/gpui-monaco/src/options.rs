//! Editor (view) options, mirroring the subset of Monaco's `IEditorOptions`
//! implemented so far.

use gpui::{Pixels, SharedString, px};

/// Monaco `lineNumbers`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum LineNumbers {
    #[default]
    On,
    Off,
    Relative,
    /// Every 10th line, plus the current line.
    Interval,
}

/// Monaco `renderLineHighlight`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum RenderLineHighlight {
    None,
    Gutter,
    #[default]
    Line,
    All,
}

/// Monaco `wordWrap`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum WordWrap {
    #[default]
    Off,
    /// Wrap at the viewport width.
    On,
    /// Wrap at `word_wrap_column`.
    WordWrapColumn,
    /// Wrap at the smaller of the viewport width and `word_wrap_column`.
    Bounded,
}

/// Monaco `showFoldingControls`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ShowFoldingControls {
    Always,
    #[default]
    Mouseover,
    Never,
}

/// Monaco `cursorStyle`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum CursorStyle {
    #[default]
    Line,
    Block,
    Underline,
    LineThin,
}

#[derive(Clone, Debug, PartialEq)]
pub struct EditorOptions {
    pub font_family: SharedString,
    pub font_size: Pixels,
    /// `None` computes it from the font size like Monaco
    /// (`GOLDEN_LINE_HEIGHT_RATIO`, 1.5 on macOS and 1.35 elsewhere).
    pub line_height: Option<Pixels>,
    pub line_numbers: LineNumbers,
    pub line_numbers_min_chars: u32,
    /// Monaco `lineDecorationsWidth`: space between line numbers and text.
    pub line_decorations_width: Pixels,
    pub glyph_margin: bool,
    pub render_line_highlight: RenderLineHighlight,
    pub cursor_style: CursorStyle,
    pub cursor_width: Pixels,
    pub cursor_blinking: bool,
    pub scroll_beyond_last_line: bool,
    pub read_only: bool,
    pub word_separators: SharedString,
    pub vertical_scrollbar_size: Pixels,
    pub horizontal_scrollbar_size: Pixels,
    /// Pixels scrolled per wheel "line" step is the line height; this scales it
    /// (Monaco `mouseWheelScrollSensitivity`).
    pub mouse_wheel_scroll_sensitivity: f32,
    pub empty_selection_clipboard: bool,
    pub sticky_tab_stops: bool,
    pub word_wrap: WordWrap,
    pub word_wrap_column: u32,
    pub wrapping_indent: crate::view::line_breaks::WrappingIndent,
    pub folding: bool,
    pub show_folding_controls: ShowFoldingControls,
    /// A one-line input (like Monaco's find input): Enter, Tab and Escape
    /// propagate to the parent and pasted line breaks become spaces.
    pub single_line: bool,
}

impl Default for EditorOptions {
    fn default() -> Self {
        Self {
            font_family: if cfg!(target_os = "macos") {
                "Menlo".into()
            } else if cfg!(target_os = "windows") {
                "Consolas".into()
            } else {
                "Droid Sans Mono".into()
            },
            font_size: if cfg!(target_os = "macos") {
                px(12.)
            } else {
                px(14.)
            },
            line_height: None,
            line_numbers: LineNumbers::On,
            line_numbers_min_chars: 5,
            line_decorations_width: px(10.),
            glyph_margin: true,
            render_line_highlight: RenderLineHighlight::Line,
            cursor_style: CursorStyle::Line,
            cursor_width: px(2.),
            cursor_blinking: true,
            scroll_beyond_last_line: true,
            read_only: false,
            word_separators: "`~!@#$%^&*()-=+[{]}\\|;:'\",.<>/?".into(),
            vertical_scrollbar_size: px(14.),
            horizontal_scrollbar_size: px(12.),
            mouse_wheel_scroll_sensitivity: 1.0,
            empty_selection_clipboard: true,
            sticky_tab_stops: false,
            word_wrap: WordWrap::Off,
            word_wrap_column: 80,
            wrapping_indent: crate::view::line_breaks::WrappingIndent::Same,
            folding: true,
            show_folding_controls: ShowFoldingControls::Mouseover,
            single_line: false,
        }
    }
}

impl EditorOptions {
    /// Options for a one-line input box: no gutter, scrollbars or line
    /// highlight.
    pub fn single_line_input() -> Self {
        Self {
            line_numbers: LineNumbers::Off,
            glyph_margin: false,
            folding: false,
            render_line_highlight: RenderLineHighlight::None,
            scroll_beyond_last_line: false,
            vertical_scrollbar_size: px(0.),
            horizontal_scrollbar_size: px(0.),
            line_decorations_width: px(2.),
            single_line: true,
            ..Self::default()
        }
    }
}

impl EditorOptions {
    pub fn resolved_line_height(&self) -> Pixels {
        self.line_height.unwrap_or_else(|| {
            let ratio = if cfg!(target_os = "macos") { 1.5 } else { 1.35 };
            px((f32::from(self.font_size) * ratio).round())
        })
    }
}
