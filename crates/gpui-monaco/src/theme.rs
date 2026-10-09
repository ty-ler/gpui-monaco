//! Editor colors. The full theme system (token colors, VS Code theme JSON) is
//! built in M3; this holds the editor chrome colors Monaco's `vs` and
//! `vs-dark` themes define.

use gpui::{Hsla, Rgba, rgb, rgba};

#[derive(Clone, Debug, PartialEq)]
pub struct EditorColors {
    pub background: Hsla,
    pub foreground: Hsla,
    pub line_number: Hsla,
    pub active_line_number: Hsla,
    pub line_highlight_background: Option<Hsla>,
    pub line_highlight_border: Option<Hsla>,
    pub selection_background: Hsla,
    pub inactive_selection_background: Hsla,
    pub cursor: Hsla,
    pub gutter_background: Hsla,
    pub scrollbar_slider: Hsla,
    pub scrollbar_slider_hover: Hsla,
    pub scrollbar_slider_active: Hsla,
    pub scrollbar_shadow: Hsla,
    pub ime_underline: Hsla,
}

fn c(color: Rgba) -> Hsla {
    color.into()
}

impl EditorColors {
    /// Monaco `vs-dark`.
    pub fn vs_dark() -> Self {
        Self {
            background: c(rgb(0x1e1e1e)),
            foreground: c(rgb(0xd4d4d4)),
            line_number: c(rgb(0x858585)),
            active_line_number: c(rgb(0xc6c6c6)),
            line_highlight_background: None,
            line_highlight_border: Some(c(rgb(0x282828))),
            selection_background: c(rgb(0x264f78)),
            inactive_selection_background: c(rgb(0x3a3d41)),
            cursor: c(rgb(0xaeafad)),
            gutter_background: c(rgb(0x1e1e1e)),
            scrollbar_slider: c(rgba(0x79797966)),
            scrollbar_slider_hover: c(rgba(0x646464b3)),
            scrollbar_slider_active: c(rgba(0xbfbfbf66)),
            scrollbar_shadow: c(rgb(0x000000)),
            ime_underline: c(rgb(0xd4d4d4)),
        }
    }

    /// Monaco `vs`.
    pub fn vs() -> Self {
        Self {
            background: c(rgb(0xfffffe)),
            foreground: c(rgb(0x000000)),
            line_number: c(rgb(0x237893)),
            active_line_number: c(rgb(0x0b216f)),
            line_highlight_background: None,
            line_highlight_border: Some(c(rgb(0xeeeeee))),
            selection_background: c(rgb(0xadd6ff)),
            inactive_selection_background: c(rgb(0xe5ebf1)),
            cursor: c(rgb(0x000000)),
            gutter_background: c(rgb(0xfffffe)),
            scrollbar_slider: c(rgba(0x64646466)),
            scrollbar_slider_hover: c(rgba(0x646464b3)),
            scrollbar_slider_active: c(rgba(0x00000099)),
            scrollbar_shadow: c(rgb(0xdddddd)),
            ime_underline: c(rgb(0x000000)),
        }
    }
}

impl Default for EditorColors {
    fn default() -> Self {
        Self::vs_dark()
    }
}
