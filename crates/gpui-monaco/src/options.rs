//! Editor (view) options, mirroring the subset of Monaco's `IEditorOptions`
//! implemented so far.

use gpui::{Pixels, SharedString, px};

/// Monaco `lineNumbers`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum LineNumbers {
    /// Absolute line numbers (the default).
    #[default]
    On,
    /// No line numbers; the gutter has no number column.
    Off,
    /// Distance from the primary cursor's line; that line shows its absolute number.
    Relative,
    /// Every 10th line, plus the current line.
    Interval,
}

/// Monaco `renderLineHighlight`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum RenderLineHighlight {
    /// No current-line highlight.
    None,
    /// Highlight only the gutter of the current line.
    Gutter,
    /// Highlight the text area of the current line (the default).
    #[default]
    Line,
    /// Highlight both the gutter and the text area of the current line.
    All,
}

/// Monaco `wordWrap`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum WordWrap {
    /// No wrapping (the default).
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
    /// Always show the fold chevrons.
    Always,
    /// Show the fold chevrons while the mouse is over the gutter (the default).
    #[default]
    Mouseover,
    /// Never show the fold chevrons.
    Never,
}

/// Monaco `minimap` options.
#[derive(Clone, Debug, PartialEq)]
pub struct MinimapOptions {
    /// Whether the minimap is shown (Monaco `minimap.enabled`, default `true`). Never shown
    /// for [`EditorOptions::single_line`] editors.
    pub enabled: bool,
    /// Columns rendered (Monaco `minimap.maxColumn`).
    pub max_column: u32,
    /// 1 or 2: pixels per character column and half the line height.
    pub scale: f32,
    /// Monaco `minimap.showSlider`: `true` always, `false` on mouse over.
    pub always_show_slider: bool,
}

impl Default for MinimapOptions {
    fn default() -> Self {
        Self {
            enabled: true,
            max_column: 120,
            scale: 1.,
            always_show_slider: false,
        }
    }
}

/// Monaco `stickyScroll` options.
#[derive(Clone, Debug, PartialEq)]
pub struct StickyScrollOptions {
    /// Whether sticky scroll is on (Monaco `stickyScroll.enabled`, default `true`).
    pub enabled: bool,
    /// Most header lines pinned at once (Monaco `stickyScroll.maxLineCount`, default 5).
    pub max_line_count: u32,
}

impl Default for StickyScrollOptions {
    fn default() -> Self {
        Self {
            enabled: true,
            max_line_count: 5,
        }
    }
}

/// Which modifier adds a cursor on click (Monaco `multiCursorModifier`); the
/// other one opens definitions.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum MultiCursorModifier {
    /// Alt+click adds a cursor; Cmd/Ctrl+click goes to the definition.
    #[default]
    Alt,
    /// Cmd/Ctrl+click adds a cursor; Alt+click goes to the definition.
    CtrlCmd,
}

/// Monaco `cursorStyle`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum CursorStyle {
    /// A vertical bar [`EditorOptions::cursor_width`] pixels wide (the default).
    #[default]
    Line,
    /// A box covering the character after the cursor.
    Block,
    /// A 2px bar under the character after the cursor.
    Underline,
    /// A 1px vertical bar.
    LineThin,
}

/// Options for one [`CodeEditor`](crate::CodeEditor) view (Monaco `IEditorOptions`).
///
/// The model-level options (tab size, insert spaces, …) live in
/// [`ModelOptions`](crate::ModelOptions).
#[derive(Clone, Debug, PartialEq)]
pub struct EditorOptions {
    /// Monaco `fontFamily`. Defaults to Menlo on macOS, Consolas on Windows and
    /// Droid Sans Mono elsewhere.
    pub font_family: SharedString,
    /// Monaco `fontSize`. Defaults to 12px on macOS and 14px elsewhere.
    pub font_size: Pixels,
    /// `None` computes it from the font size like Monaco
    /// (`GOLDEN_LINE_HEIGHT_RATIO`, 1.5 on macOS and 1.35 elsewhere).
    pub line_height: Option<Pixels>,
    /// Monaco `lineNumbers`.
    pub line_numbers: LineNumbers,
    /// Monaco `lineNumbersMinChars`: the number column is at least this many digits
    /// wide (default 5).
    pub line_numbers_min_chars: u32,
    /// Monaco `lineDecorationsWidth`: space between line numbers and text.
    pub line_decorations_width: Pixels,
    /// Monaco `glyphMargin`: show the gutter column for glyph decorations (default
    /// `true`).
    pub glyph_margin: bool,
    /// Monaco `renderLineHighlight`.
    pub render_line_highlight: RenderLineHighlight,
    /// Monaco `cursorStyle`.
    pub cursor_style: CursorStyle,
    /// Monaco `cursorWidth`: width of the [`CursorStyle::Line`] cursor (default 2px).
    pub cursor_width: Pixels,
    /// Whether the cursor blinks (default `true`). Monaco's `cursorBlinking` has more
    /// styles; `false` is `solid`.
    pub cursor_blinking: bool,
    /// Monaco `scrollBeyondLastLine`: allow scrolling the last line to the top of the
    /// viewport (default `true`).
    pub scroll_beyond_last_line: bool,
    /// Monaco `readOnly`: ignore edits from typing, IME and editing commands (default
    /// `false`).
    pub read_only: bool,
    /// Monaco `wordSeparators`: characters that end a word for word navigation,
    /// double-click selection and whole-word search.
    pub word_separators: SharedString,
    /// Monaco `scrollbar.verticalScrollbarSize` (default 14px; 0 hides it).
    pub vertical_scrollbar_size: Pixels,
    /// Monaco `scrollbar.horizontalScrollbarSize` (default 12px; 0 hides it).
    pub horizontal_scrollbar_size: Pixels,
    /// Pixels scrolled per wheel "line" step is the line height; this scales it
    /// (Monaco `mouseWheelScrollSensitivity`).
    pub mouse_wheel_scroll_sensitivity: f32,
    /// Monaco `emptySelectionClipboard`: copy and cut with an empty selection act on
    /// the whole line (default `true`).
    pub empty_selection_clipboard: bool,
    /// Monaco `stickyTabStops`: move the cursor over indentation spaces as if they were
    /// tabs (default `false`).
    pub sticky_tab_stops: bool,
    /// Monaco `wordWrap`.
    pub word_wrap: WordWrap,
    /// Monaco `wordWrapColumn`, in characters (default 80); used by
    /// [`WordWrap::WordWrapColumn`] and [`WordWrap::Bounded`].
    pub word_wrap_column: u32,
    /// Monaco `wrappingIndent`: indentation of wrapped continuation lines (default
    /// same as the first line).
    pub wrapping_indent: crate::view::line_breaks::WrappingIndent,
    /// Monaco `folding`: enable code folding and its gutter column (default `true`).
    pub folding: bool,
    /// Monaco `showFoldingControls`.
    pub show_folding_controls: ShowFoldingControls,
    /// Monaco `autoClosingBrackets` / `autoClosingQuotes` /
    /// `autoClosingComments`.
    pub auto_closing_brackets: crate::cursor::AutoClosingStrategy,
    /// Monaco `autoClosingQuotes`.
    pub auto_closing_quotes: crate::cursor::AutoClosingStrategy,
    /// Monaco `autoClosingComments`.
    pub auto_closing_comments: crate::cursor::AutoClosingStrategy,
    /// Monaco `autoClosingOvertype` / `autoClosingDelete`.
    pub auto_closing_overtype: crate::cursor::AutoClosingEditStrategy,
    /// Monaco `autoClosingDelete`.
    pub auto_closing_delete: crate::cursor::AutoClosingEditStrategy,
    /// Monaco `autoSurround`.
    pub auto_surround: crate::cursor::AutoSurroundStrategy,
    /// Monaco `autoIndent`.
    pub auto_indent: crate::cursor::AutoIndent,
    /// Monaco `matchBrackets`: highlight the bracket pair at the cursor.
    pub match_brackets: bool,
    /// Monaco `quickSuggestions`: suggest while typing in code (not in
    /// strings or comments).
    pub quick_suggestions: bool,
    /// Monaco `minimap`.
    pub minimap: MinimapOptions,
    /// Monaco `stickyScroll`.
    pub sticky_scroll: StickyScrollOptions,
    /// Monaco `occurrencesHighlight`: highlight the occurrences of the symbol
    /// at the cursor (default `true`).
    pub occurrences_highlight: bool,
    /// Monaco `occurrencesHighlightDelay`: how long the cursor rests before
    /// occurrences are highlighted (default 250ms).
    pub occurrences_highlight_delay: std::time::Duration,
    /// Monaco `selectionHighlight`: highlight other occurrences of the
    /// selected text (default `true`).
    pub selection_highlight: bool,
    /// Monaco `selectionHighlightMaxLength`: longer selections aren't
    /// highlighted (default 200 UTF-16 units; 0 for no limit).
    pub selection_highlight_max_length: u32,
    /// Monaco `selectionHighlightMultiline`: also highlight multi-line
    /// selections (default `false`).
    pub selection_highlight_multiline: bool,
    /// Monaco `multiCursorModifier`.
    pub multi_cursor_modifier: MultiCursorModifier,
    /// Monaco `multiCursorPaste`: how pasting with several cursors splits
    /// the text.
    pub multi_cursor_paste: crate::cursor::MultiCursorPaste,
    /// Monaco `multiCursorLimit`: the most cursors at once (default 10000).
    pub multi_cursor_limit: usize,
    /// Monaco `multiCursorMergeOverlapping`: merge overlapping selections
    /// (default `true`).
    pub multi_cursor_merge_overlapping: bool,
    /// Monaco `columnSelection`: mouse drags and Shift+arrows make column
    /// (box) selections.
    pub column_selection: bool,
    /// Monaco `overtypeCursorStyle`: the cursor shape in overtype mode.
    pub overtype_cursor_style: CursorStyle,
    /// Monaco `overtypeOnPaste`: in overtype mode, pasting overtypes too.
    pub overtype_on_paste: bool,
    /// Monaco `comments`: how comment toggling inserts and removes tokens.
    pub comments: crate::cursor::CommentOptions,
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
            auto_closing_brackets: Default::default(),
            auto_closing_quotes: Default::default(),
            auto_closing_comments: Default::default(),
            auto_closing_overtype: Default::default(),
            auto_closing_delete: Default::default(),
            auto_surround: Default::default(),
            auto_indent: Default::default(),
            match_brackets: true,
            quick_suggestions: true,
            minimap: MinimapOptions::default(),
            sticky_scroll: StickyScrollOptions::default(),
            comments: Default::default(),
            overtype_cursor_style: CursorStyle::Block,
            occurrences_highlight: true,
            occurrences_highlight_delay: std::time::Duration::from_millis(250),
            selection_highlight: true,
            selection_highlight_max_length: 200,
            selection_highlight_multiline: false,
            multi_cursor_modifier: MultiCursorModifier::Alt,
            multi_cursor_paste: Default::default(),
            multi_cursor_limit: 10_000,
            multi_cursor_merge_overlapping: true,
            column_selection: false,
            overtype_on_paste: true,
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
            minimap: MinimapOptions {
                enabled: false,
                ..MinimapOptions::default()
            },
            sticky_scroll: StickyScrollOptions {
                enabled: false,
                ..StickyScrollOptions::default()
            },
            ..Self::default()
        }
    }
}

impl EditorOptions {
    /// The line height in pixels: [`line_height`](Self::line_height), or the font size
    /// times 1.5 on macOS and 1.35 elsewhere, rounded.
    pub fn resolved_line_height(&self) -> Pixels {
        self.line_height.unwrap_or_else(|| {
            let ratio = if cfg!(target_os = "macos") { 1.5 } else { 1.35 };
            px((f32::from(self.font_size) * ratio).round())
        })
    }
}
