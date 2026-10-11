//! Editor (view) options, mirroring the subset of Monaco's `IEditorOptions`
//! implemented so far.

pub use crate::suggest::memory::SuggestSelection;
use gpui::{FontFeatures, FontWeight, Pixels, SharedString, px};

/// Monaco `matchBrackets`: which bracket pairs are highlighted.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum MatchBrackets {
    /// The pair at the cursor, or else the pair enclosing it.
    #[default]
    Always,
    /// Only the pair when the cursor is next to one of its brackets.
    Near,
    /// None.
    Never,
}

/// Monaco `lineNumbers`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum LineNumbers {
    /// Absolute line numbers (the default).
    #[default]
    On,
    /// No line numbers; the gutter has no number column.
    Off,
    /// Distance from the primary cursor's line; that line shows its absolute number.
    Relative,
    /// Every 10th line, plus the current and the last line.
    Interval,
    /// Each line's label from a host function of its 1-based line number
    /// (Monaco's `lineNumbers: (lineNumber) => string`); see
    /// [`LineNumbers::custom`].
    Custom(LineNumbersFn),
}

impl LineNumbers {
    /// Labels each line with `render(line_number)` (1-based), like Monaco's
    /// function-valued `lineNumbers`. An empty label leaves the line
    /// unnumbered.
    pub fn custom(render: impl Fn(u32) -> SharedString + Send + Sync + 'static) -> Self {
        Self::Custom(LineNumbersFn(std::sync::Arc::new(render)))
    }
}

/// A host function labelling line numbers ([`LineNumbers::Custom`]). Two
/// are equal when they're the same function.
#[derive(Clone)]
pub struct LineNumbersFn(pub std::sync::Arc<dyn Fn(u32) -> SharedString + Send + Sync>);

impl LineNumbersFn {
    /// The label of 1-based `line_number`.
    pub fn render(&self, line_number: u32) -> SharedString {
        (self.0)(line_number)
    }
}

impl PartialEq for LineNumbersFn {
    fn eq(&self, other: &Self) -> bool {
        std::sync::Arc::ptr_eq(&self.0, &other.0)
    }
}

impl Eq for LineNumbersFn {}

impl std::fmt::Debug for LineNumbersFn {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("LineNumbersFn")
    }
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

/// Monaco `wordWrapOverride1`/`wordWrapOverride2`: forces wrapping on or
/// off, or leaves it to the setting below.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum WordWrapOverride {
    /// Wrap at the viewport width.
    On,
    /// Don't wrap.
    Off,
    /// Use the setting below (the default).
    #[default]
    Inherit,
}

/// Monaco `wordBreak`: the line break rules for CJK text when wrapping.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum WordBreak {
    /// Break between CJK characters (the default).
    #[default]
    Normal,
    /// Don't break between CJK characters (CSS `keep-all`); other text
    /// wraps as with `Normal`.
    KeepAll,
}

/// Monaco `mouseStyle`: the mouse cursor over the text.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum MouseStyle {
    /// An I-beam (the default).
    #[default]
    Text,
    /// The arrow.
    Default,
    /// The copy cursor.
    Copy,
}

/// Monaco `mouseMiddleClickAction`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum MouseMiddleClickAction {
    /// What a middle click usually does: column select, or paste the
    /// primary selection with `selection_clipboard` (the default).
    #[default]
    Default,
    /// A middle click on a link opens it.
    OpenLink,
    /// A middle click acts like a Cmd/Ctrl+click: it opens links and goes
    /// to definitions.
    CtrlLeftClick,
}

/// Monaco `occurrencesHighlight`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum OccurrencesHighlight {
    /// No occurrence highlighting.
    Off,
    /// Highlight the symbol at the cursor in this editor (the default).
    #[default]
    SingleFile,
    /// Monaco also highlights across the other open editors (VS Code's
    /// experimental multi-document highlights). Here it highlights in this
    /// editor, like `SingleFile`: there are no multi-document highlight
    /// providers and editors don't share highlights.
    MultiFile,
}

/// Monaco `selectedTextMatchMode`: how Cmd/Ctrl+D, select all occurrences
/// and selection highlights match the selected text. An empty selection
/// always starts a whole-word, case-sensitive search.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SelectedTextMatchMode {
    /// The find widget's match case and whole word settings while it's
    /// open; case-sensitive otherwise (the default).
    #[default]
    FindOptions,
    /// Case-sensitive substrings, whatever the find widget says.
    CaseSensitive,
    /// Case-insensitive substrings, whatever the find widget says.
    CaseInsensitive,
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
    /// Monaco `minimap.side` (default right).
    pub side: MinimapSide,
    /// Monaco `minimap.renderCharacters`: an impression of each character
    /// rather than solid blocks (default `true`).
    pub render_characters: bool,
    /// Monaco `minimap.size` (default proportional).
    pub size: MinimapSize,
    /// Monaco `minimap.showRegionSectionHeaders`: label `#region`s
    /// (default `true`).
    pub show_region_section_headers: bool,
    /// Monaco `minimap.showMarkSectionHeaders`: label `MARK:` comments
    /// (default `true`).
    pub show_mark_section_headers: bool,
    /// Monaco `minimap.markSectionHeaderRegex`, with `label` and
    /// `separator` groups.
    pub mark_section_header_regex: SharedString,
    /// Monaco `minimap.sectionHeaderFontSize` (default 9).
    pub section_header_font_size: f32,
    /// Monaco `minimap.sectionHeaderLetterSpacing` (default 1). Not applied:
    /// GPUI text has no letter spacing.
    pub section_header_letter_spacing: f32,
    /// Monaco `minimap.autohide`: hide the minimap until the mouse is over
    /// it or the editor scrolls (default never).
    pub autohide: MinimapAutohide,
}

/// Monaco `minimap.autohide`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum MinimapAutohide {
    /// Always shown (the default).
    #[default]
    None,
    /// Shown while the mouse is over it, fading in and out.
    Mouseover,
    /// Shown while the editor scrolls and for half a second after (longer
    /// while the mouse is over it); clicks go through it while it's hidden.
    Scroll,
}

/// Monaco `minimap.side`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum MinimapSide {
    /// Right of the text.
    #[default]
    Right,
    /// Left of the gutter.
    Left,
}

/// Monaco `minimap.size`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum MinimapSize {
    /// Fixed-size lines; the minimap scrolls with long documents.
    #[default]
    Proportional,
    /// Lines stretch or shrink so the document fills the editor's height.
    Fill,
    /// Lines shrink as needed so the document never overflows it.
    Fit,
}

impl Default for MinimapOptions {
    fn default() -> Self {
        Self {
            enabled: true,
            max_column: 120,
            scale: 1.,
            always_show_slider: false,
            side: MinimapSide::Right,
            render_characters: true,
            size: MinimapSize::Proportional,
            show_region_section_headers: true,
            show_mark_section_headers: true,
            mark_section_header_regex: r"\bMARK:\s*(?<separator>-?)\s*(?<label>.*)$".into(),
            section_header_font_size: 9.,
            section_header_letter_spacing: 1.,
            autohide: MinimapAutohide::None,
        }
    }
}

/// When a scrollbar shows (Monaco `ScrollbarVisibility`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ScrollbarVisibility {
    /// While the mouse is over the editor or it just scrolled, fading out
    /// after.
    #[default]
    Auto,
    /// Always.
    Visible,
    /// Never.
    Hidden,
}

/// Monaco `scrollbar` options (the sizes are
/// [`EditorOptions::vertical_scrollbar_size`] and
/// [`EditorOptions::horizontal_scrollbar_size`]).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ScrollbarOptions {
    /// Monaco `scrollbar.vertical`.
    pub vertical: ScrollbarVisibility,
    /// Monaco `scrollbar.horizontal`.
    pub horizontal: ScrollbarVisibility,
    /// Monaco `scrollbar.useShadows`: shadows along the top and left edges
    /// once scrolled (default `true`).
    pub use_shadows: bool,
    /// Monaco `scrollbar.verticalHasArrows` (default `false`).
    pub vertical_has_arrows: bool,
    /// Monaco `scrollbar.horizontalHasArrows` (default `false`).
    pub horizontal_has_arrows: bool,
    /// Monaco `scrollbar.arrowSize` (default 11px).
    pub arrow_size: Pixels,
    /// Monaco `scrollbar.verticalSliderSize`: the slider's thickness;
    /// `None` fills the scrollbar.
    pub vertical_slider_size: Option<Pixels>,
    /// Monaco `scrollbar.horizontalSliderSize`.
    pub horizontal_slider_size: Option<Pixels>,
    /// Monaco `scrollbar.handleMouseWheel` (default `true`).
    pub handle_mouse_wheel: bool,
    /// Monaco `scrollbar.alwaysConsumeMouseWheel`: keep wheel events that
    /// can't scroll further from reaching the parent (default `true`).
    pub always_consume_mouse_wheel: bool,
    /// Monaco `scrollbar.scrollByPage`: clicking the track scrolls a page
    /// instead of jumping there (default `false`).
    pub scroll_by_page: bool,
    /// Monaco `scrollbar.ignoreHorizontalScrollbarInContentHeight`: without
    /// `scroll_beyond_last_line`, a shown horizontal scrollbar adds its
    /// height to the content height, so the last line can scroll above it;
    /// this leaves it out (default `false`).
    pub ignore_horizontal_scrollbar_in_content_height: bool,
}

impl Default for ScrollbarOptions {
    fn default() -> Self {
        Self {
            vertical: ScrollbarVisibility::Auto,
            horizontal: ScrollbarVisibility::Auto,
            use_shadows: true,
            vertical_has_arrows: false,
            horizontal_has_arrows: false,
            arrow_size: px(11.),
            vertical_slider_size: None,
            horizontal_slider_size: None,
            handle_mouse_wheel: true,
            always_consume_mouse_wheel: true,
            scroll_by_page: false,
            ignore_horizontal_scrollbar_in_content_height: false,
        }
    }
}

/// Monaco `cursorBlinking`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum CursorBlinking {
    /// On and off every 500ms.
    #[default]
    Blink,
    /// Fades in and out.
    Smooth,
    /// Fades, staying on longer.
    Phase,
    /// Shrinks and grows vertically.
    Expand,
    /// Doesn't blink.
    Solid,
}

/// Monaco `cursorSmoothCaretAnimation`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum CursorSmoothCaretAnimation {
    /// Off.
    #[default]
    Off,
    /// Only for explicit moves (keys, clicks), not typing.
    Explicit,
    /// Always.
    On,
}

/// Monaco `cursorSurroundingLinesStyle`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum CursorSurroundingLinesStyle {
    /// Keyboard and API moves only.
    #[default]
    Default,
    /// Mouse moves too.
    All,
}

/// Monaco `padding`.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct EditorPadding {
    /// Above the first line.
    pub top: Pixels,
    /// Below the last line.
    pub bottom: Pixels,
}

/// Monaco `find` options.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FindEditorOptions {
    /// Jump to matches while typing the search string (Monaco
    /// `find.cursorMoveOnType`, default `true`).
    pub cursor_move_on_type: bool,
    /// Where opening find takes its search string from (Monaco
    /// `find.seedSearchStringFromSelection`, default always).
    pub seed_search_string_from_selection: SeedSearchString,
    /// When opening find turns on find in selection (Monaco
    /// `find.autoFindInSelection`, default never).
    pub auto_find_in_selection: AutoFindInSelection,
    /// Scroll the text down so the widget doesn't cover the first line
    /// (Monaco `find.addExtraSpaceOnTop`, default `true`).
    pub add_extra_space_on_top: bool,
    /// Wrap around at the first and last match (Monaco `find.loop`,
    /// default `true`).
    pub loop_: bool,
    /// Remember search strings for Up and Down in the find input (Monaco
    /// `find.history`, default on: Monaco's `workspace`, shared by every
    /// editor).
    pub history: bool,
    /// The same for the replace input (Monaco `find.replaceHistory`).
    pub replace_history: bool,
    /// Close the widget when Find Next/Previous from the find input lands
    /// on a match (Monaco `find.closeOnResult`, default `false`).
    pub close_on_result: bool,
    /// Search as the find input is typed in (Monaco `find.findOnType`,
    /// default `true`); when off, Enter searches.
    pub find_on_type: bool,
}

impl Default for FindEditorOptions {
    fn default() -> Self {
        Self {
            cursor_move_on_type: true,
            seed_search_string_from_selection: SeedSearchString::Always,
            auto_find_in_selection: AutoFindInSelection::Never,
            add_extra_space_on_top: true,
            loop_: true,
            history: true,
            replace_history: true,
            close_on_result: false,
            find_on_type: true,
        }
    }
}

/// Monaco `find.seedSearchStringFromSelection`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SeedSearchString {
    /// Never.
    Never,
    /// From the selection, or the word at the cursor.
    #[default]
    Always,
    /// Only from a selection.
    Selection,
}

/// Monaco `find.autoFindInSelection`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum AutoFindInSelection {
    /// Never.
    #[default]
    Never,
    /// Whenever there's a selection.
    Always,
    /// When the selection spans lines.
    Multiline,
}

/// Monaco `stickyScroll` options.
#[derive(Clone, Debug, PartialEq)]
pub struct StickyScrollOptions {
    /// Whether sticky scroll is on (Monaco `stickyScroll.enabled`, default `true`).
    pub enabled: bool,
    /// Most header lines pinned at once (Monaco `stickyScroll.maxLineCount`, default 5).
    pub max_line_count: u32,
    /// Where the scopes come from (Monaco `stickyScroll.defaultModel`,
    /// default outline).
    pub default_model: StickyScrollModel,
    /// Scroll the sticky lines with the editor's horizontal scroll (Monaco
    /// `stickyScroll.scrollWithEditor`, default `true`).
    pub scroll_with_editor: bool,
}

impl Default for StickyScrollOptions {
    fn default() -> Self {
        Self {
            enabled: true,
            max_line_count: 5,
            default_model: StickyScrollModel::Outline,
            scroll_with_editor: true,
        }
    }
}

/// Monaco `stickyScroll.defaultModel`. Each falls back to the next when it
/// has no scopes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum StickyScrollModel {
    /// The document symbols (`outlineModel`), from providers or the
    /// language's tags query.
    #[default]
    Outline,
    /// The folding ranges (`foldingProviderModel`).
    FoldingProvider,
    /// Indentation (`indentationModel`).
    Indentation,
}

/// Which whitespace is drawn as `·` and `→` (Monaco `renderWhitespace`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum RenderWhitespace {
    /// None.
    None,
    /// All except single spaces between words.
    Boundary,
    /// Only whitespace inside selections (the default).
    #[default]
    Selection,
    /// Only trailing whitespace.
    Trailing,
    /// All whitespace.
    All,
}

/// How the last line's number shows when the file ends with a newline
/// (Monaco `renderFinalNewline`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum RenderFinalNewline {
    /// Numbered like any line (the default).
    #[default]
    On,
    /// No line number.
    Off,
    /// A dimmed line number.
    Dimmed,
}

/// A vertical ruler (Monaco `IRulerOption`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Ruler {
    /// The visible column it's drawn at.
    pub column: u32,
    /// Its color; `None` uses the theme's `editorRuler.foreground`.
    pub color: Option<gpui::Hsla>,
}

/// Monaco `guides`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GuideOptions {
    /// Draw indent guides (Monaco `guides.indentation`, default `true`).
    pub indentation: bool,
    /// Highlight the guide of the block with the cursor (Monaco
    /// `guides.highlightActiveIndentation`, default on).
    pub highlight_active_indentation: HighlightActiveIndentation,
    /// Vertical bracket pair guides (Monaco `guides.bracketPairs`, default
    /// off).
    pub bracket_pairs: BracketPairGuides,
    /// Horizontal bracket pair guides (Monaco
    /// `guides.bracketPairsHorizontal`, default for the active pair).
    pub bracket_pairs_horizontal: BracketPairGuides,
    /// Highlight the guides of the pair around the cursor (Monaco
    /// `guides.highlightActiveBracketPair`, default `true`).
    pub highlight_active_bracket_pair: bool,
}

impl Default for GuideOptions {
    fn default() -> Self {
        Self {
            indentation: true,
            highlight_active_indentation: HighlightActiveIndentation::On,
            bracket_pairs: BracketPairGuides::Off,
            bracket_pairs_horizontal: BracketPairGuides::Active,
            highlight_active_bracket_pair: true,
        }
    }
}

/// Whether the active indent guide is highlighted (Monaco
/// `guides.highlightActiveIndentation`: `true`, `"always"` or `false`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum HighlightActiveIndentation {
    /// Not highlighted.
    Off,
    /// Highlighted, except on lines with bracket pair guides (the
    /// default).
    #[default]
    On,
    /// Highlighted even on lines with bracket pair guides.
    Always,
}

/// Monaco `bracketPairColorization`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BracketPairColorizationOptions {
    /// Color brackets by nesting level (Monaco `enabled`, default `true`).
    pub enabled: bool,
    /// Each kind of bracket counts its own nesting level, so `(` inside `[`
    /// gets the first color (Monaco `independentColorPoolPerBracketType`,
    /// default `false`). Bracket pair guides are colored the same way.
    pub independent_color_pool_per_bracket_type: bool,
}

impl Default for BracketPairColorizationOptions {
    fn default() -> Self {
        Self {
            enabled: true,
            independent_color_pool_per_bracket_type: false,
        }
    }
}

/// Which bracket pairs get guides (Monaco's `true`, `"active"` and
/// `false`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum BracketPairGuides {
    /// None.
    #[default]
    Off,
    /// Only the pair around the cursor.
    Active,
    /// Every pair.
    On,
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

/// Monaco `hover.enabled`: when the mouse shows hovers.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum HoverEnabled {
    /// Always.
    #[default]
    On,
    /// Never (Show or Focus Hover still works).
    Off,
    /// While Cmd/Ctrl, or Alt with `multi_cursor_modifier: CtrlCmd`, is
    /// held (the opposite of the multi-cursor modifier).
    OnKeyboardModifier,
}

/// Monaco `hover`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HoverOptions {
    /// When the mouse shows hovers (default `On`).
    pub enabled: HoverEnabled,
    /// How long the mouse rests before the hover shows (default 300ms).
    pub delay: std::time::Duration,
    /// The hover stays while the mouse moves over it (default `true`).
    pub sticky: bool,
    /// With `sticky`, how long the hover waits before reacting to the
    /// mouse leaving its word, so the mouse can reach it (default 300ms).
    pub hiding_delay: std::time::Duration,
    /// Prefer showing the hover above the line (default `true`).
    pub above: bool,
    /// Show long line warning hovers, when rendering stops
    /// (`stop_rendering_line_after`) or tokenization is skipped
    /// (`max_tokenization_line_length`) (default `true`).
    pub show_long_line_warning: bool,
}

impl Default for HoverOptions {
    fn default() -> Self {
        Self {
            enabled: HoverEnabled::On,
            delay: std::time::Duration::from_millis(300),
            sticky: true,
            hiding_delay: std::time::Duration::from_millis(300),
            above: true,
            show_long_line_warning: true,
        }
    }
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
    /// A 1px outline of the box [`Block`](Self::Block) fills.
    BlockOutline,
    /// A 1px bar under the character after the cursor.
    UnderlineThin,
}

/// Monaco `acceptSuggestionOnEnter`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum AcceptSuggestionOnEnter {
    /// Enter accepts.
    #[default]
    On,
    /// Enter accepts unless the suggestion would change nothing (the word
    /// is already typed) in a session that opened by itself.
    Smart,
    /// Only Tab accepts.
    Off,
}

/// Monaco `suggest.insertMode`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SuggestInsertMode {
    /// Insert before the rest of the word (the item's `range`).
    #[default]
    Insert,
    /// Replace the rest of the word too (the item's `replace_range`).
    Replace,
}

/// Monaco `quickSuggestions`: where suggestions open while typing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct QuickSuggestions {
    /// In code.
    pub other: bool,
    /// In comments.
    pub comments: bool,
    /// In strings.
    pub strings: bool,
}

impl QuickSuggestions {
    /// Everywhere.
    pub const ON: Self = Self {
        other: true,
        comments: true,
        strings: true,
    };
    /// Nowhere.
    pub const OFF: Self = Self {
        other: false,
        comments: false,
        strings: false,
    };
}

impl Default for QuickSuggestions {
    /// Monaco's default: in code only.
    fn default() -> Self {
        Self {
            other: true,
            comments: false,
            strings: false,
        }
    }
}

/// Monaco `wordBasedSuggestions`: completions from words in documents,
/// offered when no other provider has any.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum WordBasedSuggestions {
    /// None.
    Off,
    /// Monaco's default: like `MatchingDocuments`, but off while an inline
    /// completions provider serves the language (and `inline_suggest` is
    /// enabled).
    #[default]
    OffWithInlineSuggestions,
    /// Words of this document.
    CurrentDocument,
    /// Words of open documents in the same language.
    MatchingDocuments,
    /// Words of all open documents.
    AllDocuments,
}

/// When the inline suggestion toolbar shows (Monaco
/// `inlineSuggest.showToolbar`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum InlineSuggestShowToolbar {
    /// Whenever an inline suggestion shows.
    Always,
    /// While the mouse is over an inline suggestion (the default).
    #[default]
    OnHover,
    /// Never.
    Never,
}

/// How ghost text relates to the text at the cursor (Monaco
/// `inlineSuggest.mode`, also `suggest.previewMode`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum InlineSuggestMode {
    /// Only text appended at the cursor.
    Prefix,
    /// Insertions anywhere in the rest of the word.
    Subword,
    /// Insertions in the rest of the word, but none before the cursor.
    #[default]
    SubwordSmart,
}

/// Monaco `inlineSuggest`.
#[derive(Clone, Debug, PartialEq)]
pub struct InlineSuggestOptions {
    /// `enabled`: show inline completions (on).
    pub enabled: bool,
    /// `mode`.
    pub mode: InlineSuggestMode,
    /// `suppressSuggestions`: keep the suggest widget from opening by itself
    /// while ghost text shows (off).
    pub suppress_suggestions: bool,
    /// `keepOnBlur`: keep ghost text when the editor loses focus (off).
    pub keep_on_blur: bool,
    /// `minShowDelay`: show results no sooner than this after the request.
    pub min_show_delay: std::time::Duration,
    /// `suppressInSnippetMode`: no ghost text inside a snippet (on).
    pub suppress_in_snippet_mode: bool,
    /// `showToolbar`: when the toolbar with previous/next and accept shows
    /// (`OnHover`).
    pub show_toolbar: InlineSuggestShowToolbar,
    /// `syntaxHighlightingEnabled`: color ghost text like code (on).
    pub syntax_highlighting_enabled: bool,
    /// `fontFamily`: the ghost text's font; `None` is the editor's
    /// (Monaco `'default'`).
    pub font_family: Option<SharedString>,
    /// `edits`: inline edits (next edit suggestions).
    pub edits: InlineSuggestEditsOptions,
}

impl Default for InlineSuggestOptions {
    fn default() -> Self {
        Self {
            enabled: true,
            mode: InlineSuggestMode::SubwordSmart,
            suppress_suggestions: false,
            keep_on_blur: false,
            min_show_delay: std::time::Duration::ZERO,
            suppress_in_snippet_mode: true,
            show_toolbar: InlineSuggestShowToolbar::OnHover,
            syntax_highlighting_enabled: true,
            font_family: None,
            edits: InlineSuggestEditsOptions::default(),
        }
    }
}

/// Whether showing an inline edit may move the text to make room (Monaco
/// `inlineSuggest.edits.allowCodeShifting`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum InlineEditsCodeShifting {
    /// Insertions show in place: inserted lines push the text down (the
    /// default).
    #[default]
    Always,
    /// Text inserted within a line shows in place; inserted lines show in a
    /// box instead.
    Horizontal,
    /// The text never moves: insertions show in a box.
    Never,
}

/// Whether larger inline edits may show beside the text (Monaco
/// `inlineSuggest.edits.renderSideBySide`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum InlineEditsRenderSideBySide {
    /// Beside the text when it fits, else below it (the default).
    #[default]
    Auto,
    /// Always below the text.
    Never,
}

/// Monaco `inlineSuggest.edits`: how inline edits (next edit suggestions)
/// show.
#[derive(Clone, Debug, PartialEq)]
pub struct InlineSuggestEditsOptions {
    /// `enabled`: ask providers for inline edits and show them (on).
    pub enabled: bool,
    /// `showCollapsed`: show only a marker at the edit until jumping to it
    /// (off).
    pub show_collapsed: bool,
    /// `renderSideBySide`.
    pub render_side_by_side: InlineEditsRenderSideBySide,
    /// `allowCodeShifting`.
    pub allow_code_shifting: InlineEditsCodeShifting,
    /// `showLongDistanceHint`: while the inline edit is off screen (or in
    /// another file), preview it in a box near the cursor (on).
    pub show_long_distance_hint: bool,
    /// `longDistanceHintContextLineCount`: lines shown above and below the
    /// target line in that preview, 0 to 10 (0).
    pub long_distance_hint_context_line_count: u32,
}

impl Default for InlineSuggestEditsOptions {
    fn default() -> Self {
        Self {
            enabled: true,
            show_collapsed: false,
            render_side_by_side: InlineEditsRenderSideBySide::Auto,
            allow_code_shifting: InlineEditsCodeShifting::Always,
            show_long_distance_hint: true,
            long_distance_hint_context_line_count: 0,
        }
    }
}

/// Monaco `semanticHighlighting.enabled`: whether semantic tokens color
/// the text.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SemanticHighlighting {
    /// Always.
    On,
    /// Never.
    Off,
    /// As the theme says (`Theme::semantic_highlighting`; off for Monaco's
    /// built-in themes, as in standalone Monaco).
    #[default]
    ConfiguredByTheme,
}

/// Monaco `renderValidationDecorations`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum RenderValidationDecorations {
    /// Only while the editor is editable (not read-only).
    #[default]
    Editable,
    /// Always.
    On,
    /// Never.
    Off,
}

/// Monaco `tabCompletion`: what Tab does without the suggest widget.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum TabCompletion {
    /// Tab indents.
    #[default]
    Off,
    /// At the end of a word, Tab inserts the best completion; Tab and
    /// Shift+Tab then cycle through the other suggestions.
    On,
    /// Only snippet prefixes complete (Monaco's `onlySnippets`). Standalone
    /// Monaco has no snippet prefixes (VS Code's workbench completes its user
    /// snippets), so like Monaco this behaves like `Off`.
    OnlySnippets,
}

/// Monaco `suggest.selectionMode`: whether a suggestion is focused when
/// the widget opens by itself (an explicit request, like Ctrl+Space, always
/// focuses one).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SuggestSelectionMode {
    /// Always focus a suggestion.
    #[default]
    Always,
    /// Never: Enter and Tab then type as usual until a suggestion is
    /// focused with the arrow keys or Ctrl+Space (Monaco `focusSuggestion`).
    Never,
    /// Only when a trigger character opened the widget.
    WhenTriggerCharacter,
    /// Only when typing a word opened the widget (quick suggestions), or
    /// once more is typed after a trigger character.
    WhenQuickSuggestion,
}

/// Monaco `editor.suggest`.
#[derive(Clone, Debug, PartialEq)]
pub struct SuggestOptions {
    /// `insertMode`.
    pub insert_mode: SuggestInsertMode,
    /// `filterGraceful`: tolerate small typos when filtering.
    pub filter_graceful: bool,
    /// `matchOnWordStartOnly`: the first character must match a word start.
    pub match_on_word_start_only: bool,
    /// `showDeprecated`.
    pub show_deprecated: bool,
    /// Kinds hidden from the list (Monaco's `show*` options set to false,
    /// e.g. `showWords: false` hides `Text`).
    pub hidden_kinds: Vec<crate::providers::CompletionItemKind>,
    /// `localityBonus`: among equal matches, prefer words used in the
    /// innermost bracket block around the cursor.
    pub locality_bonus: bool,
    /// `fontSize`: the widget's font size; 0 uses the editor's.
    pub font_size: f32,
    /// `lineHeight`: the widget's row height; 0 uses the editor's.
    pub line_height: f32,
    /// `showIcons`: kind icons next to labels.
    pub show_icons: bool,
    /// `showStatusBar`: the insert/replace and details hints under the list.
    pub show_status_bar: bool,
    /// `showInlineDetails`: details at the right of the focused row.
    pub show_inline_details: bool,
    /// `preview`: show the selected suggestion as ghost text (off).
    pub preview: bool,
    /// `previewMode`: how the preview relates to the typed text.
    pub preview_mode: InlineSuggestMode,
    /// `selectionMode`: whether a suggestion is focused when the widget
    /// opens by itself.
    pub selection_mode: SuggestSelectionMode,
    /// `snippetsPreventQuickSuggestions`: no quick suggestions while a
    /// snippet is active (off).
    pub snippets_prevent_quick_suggestions: bool,
}

impl Default for SuggestOptions {
    fn default() -> Self {
        Self {
            insert_mode: SuggestInsertMode::Insert,
            filter_graceful: true,
            match_on_word_start_only: true,
            show_deprecated: true,
            hidden_kinds: Vec::new(),
            locality_bonus: false,
            font_size: 0.,
            line_height: 0.,
            show_icons: true,
            show_status_bar: false,
            show_inline_details: true,
            preview: false,
            preview_mode: InlineSuggestMode::SubwordSmart,
            selection_mode: SuggestSelectionMode::Always,
            snippets_prevent_quick_suggestions: false,
        }
    }
}

/// What opens the color picker from a swatch (Monaco
/// `colorDecoratorsActivatedOn`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ColorDecoratorsActivatedOn {
    /// Hovering or clicking.
    #[default]
    ClickAndHover,
    /// Hovering.
    Hover,
    /// Clicking.
    Click,
}

/// When the built-in color finder (hex, `rgb()`, `hsl()` and the like)
/// decorates colors (Monaco `defaultColorDecorators`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum DefaultColorDecorators {
    /// Only when no color provider found any.
    #[default]
    Auto,
    /// Always, alongside providers.
    Always,
    /// Never.
    Never,
}

/// Where folding ranges come from (Monaco `foldingStrategy`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum FoldingStrategy {
    /// Folding range providers, else the syntax tree (`folds.scm`), else
    /// indentation.
    #[default]
    Auto,
    /// Indentation (and region markers) only.
    Indentation,
}

/// When inlay hints show (Monaco `inlayHints.enabled`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum InlayHintsMode {
    /// Always.
    #[default]
    On,
    /// Hidden while Ctrl+Alt is held.
    OnUnlessPressed,
    /// Shown only while Ctrl+Alt is held.
    OffUnlessPressed,
    /// Never (nothing is requested).
    Off,
}

/// Monaco `inlayHints`.
#[derive(Clone, Debug, PartialEq)]
pub struct InlayHintsOptions {
    /// `enabled`.
    pub enabled: InlayHintsMode,
    /// `padding`: pad the hint's background.
    pub padding: bool,
    /// `maximumLength`: hint text per line, beyond which it's cut with
    /// `…`; 0 for no limit.
    pub maximum_length: usize,
    /// `fontSize`: the hints' font size; 0, under 5 or above the editor's
    /// uses the editor's.
    pub font_size: f32,
    /// `fontFamily`: the hints' font; `None` uses the editor's.
    pub font_family: Option<SharedString>,
}

impl Default for InlayHintsOptions {
    fn default() -> Self {
        Self {
            enabled: InlayHintsMode::On,
            padding: false,
            maximum_length: 43,
            font_size: 0.,
            font_family: None,
        }
    }
}

/// Monaco `parameterHints`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ParameterHintsOptions {
    /// `enabled`: open parameter hints while typing (on). They still open
    /// when requested (Cmd/Ctrl+Shift+Space).
    pub enabled: bool,
    /// `cycle`: going past the last or first signature wraps around
    /// rather than closing the hints (on).
    pub cycle: bool,
}

impl Default for ParameterHintsOptions {
    fn default() -> Self {
        Self {
            enabled: true,
            cycle: true,
        }
    }
}

/// When the lightbulb asks for code actions (Monaco
/// `ShowLightbulbIconMode`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum LightbulbMode {
    /// Never.
    Off,
    /// When the cursor is on or next to code, or text is selected.
    #[default]
    OnCode,
    /// Always.
    On,
}

/// What symbol navigation does with several results (Monaco
/// `GoToLocationValues`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum GoToLocation {
    /// Show them in a peek view.
    #[default]
    Peek,
    /// Go to the first and show them all in a peek view.
    GotoAndPeek,
    /// Go to the first; F4 goes to the next.
    Goto,
}

/// Monaco `peekWidgetDefaultFocus`: what has focus when a peek view
/// opens.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PeekWidgetDefaultFocus {
    /// The results list.
    #[default]
    Tree,
    /// The preview editor.
    Editor,
}

/// A symbol navigation command.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum LocationKind {
    /// Go to definition (F12).
    Definition,
    /// Go to declaration.
    Declaration,
    /// Go to type definition.
    TypeDefinition,
    /// Go to implementations (Cmd/Ctrl+F12).
    Implementation,
    /// Go to references (Shift+F12).
    References,
}

/// Monaco `gotoLocation`: what each navigation command does with several
/// results, and what it runs instead when its only result is where the
/// cursor already is.
#[derive(Clone, Debug, PartialEq)]
pub struct GotoLocationOptions {
    /// `multipleDefinitions`.
    pub multiple_definitions: GoToLocation,
    /// `multipleTypeDefinitions`.
    pub multiple_type_definitions: GoToLocation,
    /// `multipleDeclarations`.
    pub multiple_declarations: GoToLocation,
    /// `multipleImplementations`.
    pub multiple_implementations: GoToLocation,
    /// `multipleReferences`.
    pub multiple_references: GoToLocation,
    /// `alternativeDefinitionCommand` (go to references by default).
    pub alternative_definition_command: Option<LocationKind>,
    /// `alternativeTypeDefinitionCommand` (go to references by default).
    pub alternative_type_definition_command: Option<LocationKind>,
    /// `alternativeDeclarationCommand` (go to references by default).
    pub alternative_declaration_command: Option<LocationKind>,
    /// `alternativeImplementationCommand` (none by default).
    pub alternative_implementation_command: Option<LocationKind>,
    /// `alternativeReferenceCommand` (none by default).
    pub alternative_reference_command: Option<LocationKind>,
}

impl Default for GotoLocationOptions {
    fn default() -> Self {
        Self {
            multiple_definitions: GoToLocation::Peek,
            multiple_type_definitions: GoToLocation::Peek,
            multiple_declarations: GoToLocation::Peek,
            multiple_implementations: GoToLocation::Peek,
            multiple_references: GoToLocation::Peek,
            alternative_definition_command: Some(LocationKind::References),
            alternative_type_definition_command: Some(LocationKind::References),
            alternative_declaration_command: Some(LocationKind::References),
            alternative_implementation_command: None,
            alternative_reference_command: None,
        }
    }
}

impl GotoLocationOptions {
    /// What `kind` does with several results.
    pub fn multiple(&self, kind: LocationKind) -> GoToLocation {
        match kind {
            LocationKind::Definition => self.multiple_definitions,
            LocationKind::Declaration => self.multiple_declarations,
            LocationKind::TypeDefinition => self.multiple_type_definitions,
            LocationKind::Implementation => self.multiple_implementations,
            LocationKind::References => self.multiple_references,
        }
    }

    /// What `kind` runs instead when its only result is at the cursor.
    pub fn alternative(&self, kind: LocationKind) -> Option<LocationKind> {
        match kind {
            LocationKind::Definition => self.alternative_definition_command,
            LocationKind::Declaration => self.alternative_declaration_command,
            LocationKind::TypeDefinition => self.alternative_type_definition_command,
            LocationKind::Implementation => self.alternative_implementation_command,
            LocationKind::References => self.alternative_reference_command,
        }
    }
}

/// Monaco `unicodeHighlight`: which characters get a warning border.
///
/// Monaco's `inUntrustedWorkspace` defaults resolve as in a trusted
/// workspace, as the standalone editor does.
#[derive(Clone, Debug, PartialEq)]
pub struct UnicodeHighlightOptions {
    /// Every character outside basic ASCII (default `false`).
    pub non_basic_ascii: bool,
    /// Invisible characters, such as zero-width spaces (default `true`).
    pub invisible_characters: bool,
    /// Characters that look like ASCII ones, such as Cyrillic `а` (default
    /// `true`).
    pub ambiguous_characters: bool,
    /// Also in comments (default `false`).
    pub include_comments: bool,
    /// Also in strings (default `true`).
    pub include_strings: bool,
    /// Characters never highlighted.
    pub allowed_characters: Vec<char>,
    /// Locales whose common characters aren't ambiguous. `_os` and
    /// `_vscode` (the defaults) stand for the system's locale.
    pub allowed_locales: Vec<SharedString>,
}

impl Default for UnicodeHighlightOptions {
    fn default() -> Self {
        Self {
            non_basic_ascii: false,
            invisible_characters: true,
            ambiguous_characters: true,
            include_comments: false,
            include_strings: true,
            allowed_characters: Vec::new(),
            allowed_locales: vec!["_os".into(), "_vscode".into()],
        }
    }
}

/// When to show the button that switches between drop or paste edits
/// (Monaco `showDropSelector`/`showPasteSelector`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ShowEditSelector {
    /// After the drop or paste (`afterDrop`, `afterPaste`).
    #[default]
    AfterEdit,
    /// Never.
    Never,
}

/// Monaco `dropIntoEditor`.
#[derive(Clone, Debug, PartialEq)]
pub struct DropIntoEditorOptions {
    /// Accept drops (default `true`).
    pub enabled: bool,
    /// Monaco `showDropSelector`.
    pub show_drop_selector: ShowEditSelector,
}

impl Default for DropIntoEditorOptions {
    fn default() -> Self {
        Self {
            enabled: true,
            show_drop_selector: ShowEditSelector::AfterEdit,
        }
    }
}

/// Monaco `pasteAs`.
#[derive(Clone, Debug, PartialEq)]
pub struct PasteAsOptions {
    /// Let paste edit providers handle pastes (default `true`).
    pub enabled: bool,
    /// Monaco `showPasteSelector`.
    pub show_paste_selector: ShowEditSelector,
}

impl Default for PasteAsOptions {
    fn default() -> Self {
        Self {
            enabled: true,
            show_paste_selector: ShowEditSelector::AfterEdit,
        }
    }
}

/// Monaco `unusualLineTerminators`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum UnusualLineTerminators {
    /// Remove them as they appear.
    Auto,
    /// Leave them.
    Off,
    /// Ask whether to remove them.
    #[default]
    Prompt,
}

/// Monaco `wrappingStrategy`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum WrappingStrategy {
    /// Every character is a column wide (two for full-width ones): fast,
    /// and right for monospace fonts.
    #[default]
    Simple,
    /// Characters are measured in the editor font, so proportional fonts
    /// wrap at the edge.
    Advanced,
}

/// Monaco `fullwidthCharacterWidth`: how wide full-width (CJK) characters
/// are drawn.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum FullwidthCharacterWidth {
    /// The font's own width (the default).
    #[default]
    Font,
    /// Centered in exactly two character cells, so columns line up. Only
    /// with a monospace font (Monaco `effectiveFullwidthCharacterWidth`).
    TwoCells,
}

/// Monaco `accessibilitySupport`: whether the editor runs optimized for
/// screen readers.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum AccessibilitySupport {
    /// Optimized while assistive technology is connected to the window (the
    /// default; Monaco asks the platform).
    #[default]
    Auto,
    /// Always optimized for screen readers.
    On,
    /// Assume no screen reader is attached.
    Off,
}

/// Monaco `fontLigatures`: the font's OpenType features.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum FontLigatures {
    /// `"liga" off, "calt" off`.
    #[default]
    Off,
    /// `"liga" on, "calt" on`.
    On,
    /// A CSS `font-feature-settings` value, such as `"calt", "ss01" on,
    /// "zero" 1`.
    Settings(SharedString),
}

impl FontLigatures {
    /// The features to shape text with.
    pub fn features(&self) -> FontFeatures {
        let settings = match self {
            Self::Off => vec![("liga".into(), 0), ("calt".into(), 0)],
            Self::On => vec![("liga".into(), 1), ("calt".into(), 1)],
            Self::Settings(settings) => parse_font_feature_settings(settings),
        };
        FontFeatures(std::sync::Arc::new(settings))
    }
}

/// Parses CSS `font-feature-settings`: comma-separated quoted four-letter
/// tags, each optionally followed by `on`, `off` or a number (1 if
/// missing). Invalid entries are skipped.
fn parse_font_feature_settings(settings: &str) -> Vec<(String, u32)> {
    settings
        .split(',')
        .filter_map(|entry| {
            let entry = entry.trim();
            let quote = entry.chars().next().filter(|c| *c == '"' || *c == '\'')?;
            let rest = &entry[1..];
            let end = rest.find(quote)?;
            let tag = &rest[..end];
            if tag.len() != 4 || !tag.is_ascii() {
                return None;
            }
            let value = match rest[end + 1..].trim() {
                "" | "on" => 1,
                "off" => 0,
                number => number.parse().ok()?,
            };
            Some((tag.to_string(), value))
        })
        .collect()
}

#[cfg(test)]
mod font_feature_tests {
    use super::*;

    #[test]
    fn parses_font_feature_settings() {
        assert_eq!(
            parse_font_feature_settings(r#""calt", 'ss01' on, "zero" 2, "liga" off, "bad", "x" 1"#),
            vec![
                ("calt".to_string(), 1),
                ("ss01".to_string(), 1),
                ("zero".to_string(), 2),
                ("liga".to_string(), 0),
            ]
        );
        assert_eq!(
            FontLigatures::Off.features().tag_value_list(),
            &[("liga".to_string(), 0), ("calt".to_string(), 0)]
        );
    }
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
    /// Monaco `fontWeight` (default normal). Bold tokens stay bold.
    pub font_weight: FontWeight,
    /// Monaco `fontLigatures` (default off).
    pub font_ligatures: FontLigatures,
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
    /// Monaco `cursorBlinking`: how the cursor blinks (default blink).
    pub cursor_blinking: CursorBlinking,
    /// Monaco `cursorSmoothCaretAnimation`: animate cursor moves (default
    /// off).
    pub cursor_smooth_caret_animation: CursorSmoothCaretAnimation,
    /// Monaco `cursorHeight`: the line cursor's height, at most the line
    /// height; 0 for the line height (default 0).
    pub cursor_height: Pixels,
    /// Monaco `cursorSurroundingLines`: lines kept visible above and below
    /// the cursor when it moves (default 0).
    pub cursor_surrounding_lines: u32,
    /// Monaco `cursorSurroundingLinesStyle`: whether mouse moves keep them
    /// too (default no).
    pub cursor_surrounding_lines_style: CursorSurroundingLinesStyle,
    /// Monaco `smoothScrolling`: animate scrolling from reveals, cursor
    /// moves and mouse wheels (default `false`).
    pub smooth_scrolling: bool,
    /// Monaco `scrollBeyondLastColumn`: columns of space past the longest
    /// line (default 4).
    pub scroll_beyond_last_column: u32,
    /// Monaco `revealHorizontalRightPadding`: space kept right of a revealed
    /// position (default 15px).
    pub reveal_horizontal_right_padding: Pixels,
    /// Monaco `fastScrollSensitivity`: wheel speed with Alt held (default
    /// 5).
    pub fast_scroll_sensitivity: f32,
    /// Monaco `scrollPredominantAxis`: a diagonal wheel or trackpad scroll
    /// moves along its main axis only (default `true`).
    pub scroll_predominant_axis: bool,
    /// Monaco `inertialScroll`: trackpad scrolling keeps going and slows
    /// down after the fingers lift, for platforms that don't do it
    /// themselves (default `false`).
    pub inertial_scroll: bool,
    /// Monaco `scrollOnMiddleClick`: a middle click starts scrolling toward
    /// the mouse until the next click (default `false`).
    pub scroll_on_middle_click: bool,
    /// Monaco `padding`: space above the first line and below the last.
    pub padding: EditorPadding,
    /// Monaco `roundedSelection`: selections have rounded corners (default
    /// `true`).
    pub rounded_selection: bool,
    /// Monaco `renderLineHighlightOnlyWhenFocus` (default `false`).
    pub render_line_highlight_only_when_focus: bool,
    /// Monaco `scrollBeyondLastLine`: allow scrolling the last line to the top of the
    /// viewport (default `true`).
    pub scroll_beyond_last_line: bool,
    /// Monaco `allowVariableLineHeights`: decorations may change line
    /// heights (default `true`).
    pub allow_variable_line_heights: bool,
    /// Monaco `readOnly`: ignore edits from typing, IME and editing commands (default
    /// `false`).
    pub read_only: bool,
    /// Monaco `readOnlyMessage`: markdown shown when typing into a
    /// read-only editor. `None` shows "Cannot edit in read-only editor"
    /// ("input" for single-line editors).
    pub read_only_message: Option<SharedString>,
    /// Monaco `placeholder`: shown while the model is empty.
    pub placeholder: Option<SharedString>,
    /// Monaco `unusualLineTerminators`: remove LS and PS characters
    /// (`Auto`), ask first (`Prompt`, the default) or leave them (`Off`).
    pub unusual_line_terminators: UnusualLineTerminators,
    /// Monaco `unicodeHighlight`.
    pub unicode_highlight: UnicodeHighlightOptions,
    /// Monaco `dragAndDrop`: drag the selection to move it, or copy it with
    /// Alt (Ctrl elsewhere) (default `true`).
    pub drag_and_drop: bool,
    /// Monaco `doubleClickSelectsBlock`: double-clicking just inside
    /// brackets or quotes selects their content (default `true`).
    pub double_click_selects_block: bool,
    /// Monaco `dropIntoEditor`: files dropped on the editor are inserted by
    /// the drop edit providers.
    pub drop_into_editor: DropIntoEditorOptions,
    /// Monaco `pasteAs`: paste edit providers may decide how the clipboard
    /// is pasted.
    pub paste_as: PasteAsOptions,
    /// Monaco `selectionClipboard` (Linux): selections go to the primary
    /// selection, and a middle click pastes it (default `true`).
    pub selection_clipboard: bool,
    /// Monaco `autoIndentOnPaste`: re-indent pasted lines by the language's
    /// indentation rules, with `auto_indent: Full` (default `false`).
    pub auto_indent_on_paste: bool,
    /// Monaco `autoIndentOnPasteWithinString`: also when the paste starts or
    /// ends in a string (default `true`).
    pub auto_indent_on_paste_within_string: bool,
    /// Monaco `wordSeparators`: characters that end a word for word navigation,
    /// double-click selection and whole-word search.
    pub word_separators: SharedString,
    /// Monaco `scrollbar.verticalScrollbarSize` (default 14px; 0 hides it).
    pub vertical_scrollbar_size: Pixels,
    /// Monaco `scrollbar.horizontalScrollbarSize` (default 12px; 0 hides it).
    pub horizontal_scrollbar_size: Pixels,
    /// The rest of Monaco `scrollbar`.
    pub scrollbar: ScrollbarOptions,
    /// Monaco `overviewRulerLanes`: lanes in the overview ruler, 0 to hide
    /// its marks (default 3).
    pub overview_ruler_lanes: u32,
    /// Monaco `overviewRulerBorder`: a border on its left (default
    /// `true`).
    pub overview_ruler_border: bool,
    /// Monaco `hideCursorInOverviewRuler` (default `false`).
    pub hide_cursor_in_overview_ruler: bool,
    /// Pixels scrolled per wheel "line" step is the line height; this scales it
    /// (Monaco `mouseWheelScrollSensitivity`).
    pub mouse_wheel_scroll_sensitivity: f32,
    /// Monaco `mouseWheelZoom`: the wheel zooms the font while Cmd (macOS)
    /// or Ctrl is held (default `false`). See
    /// [`set_editor_zoom_level`](crate::set_editor_zoom_level).
    pub mouse_wheel_zoom: bool,
    /// Monaco `emptySelectionClipboard`: copy and cut with an empty selection act on
    /// the whole line (default `true`).
    pub empty_selection_clipboard: bool,
    /// Monaco `stickyTabStops`: move the cursor over indentation spaces as if they were
    /// tabs (default `false`).
    pub sticky_tab_stops: bool,
    /// Monaco `useTabStops`: Backspace in indentation deletes to the
    /// previous indent stop, and Tab/Shift+Tab snap to indent stops
    /// (default `true`).
    pub use_tab_stops: bool,
    /// Monaco `trimWhitespaceOnDelete`: deleting a line break also deletes
    /// the next line's leading whitespace (default `false`).
    pub trim_whitespace_on_delete: bool,
    /// Monaco `wordWrap`.
    pub word_wrap: WordWrap,
    /// Monaco `wordWrapOverride1`: overrides `word_wrap` (default inherit).
    /// The diff editor sets it from its `diff_word_wrap`.
    pub word_wrap_override1: WordWrapOverride,
    /// Monaco `wordWrapOverride2`: overrides `word_wrap_override1` (default
    /// inherit).
    pub word_wrap_override2: WordWrapOverride,
    /// Monaco `wordWrapIndicator`: a `↩` at the wrapping column of each
    /// soft-wrapped line, which then wraps a column earlier to leave room
    /// for it (default `false`). Only while wrapping.
    pub word_wrap_indicator: bool,
    /// Monaco `wordWrapColumn`, in characters (default 80); used by
    /// [`WordWrap::WordWrapColumn`] and [`WordWrap::Bounded`].
    pub word_wrap_column: u32,
    /// Monaco `wordWrapBreakBeforeCharacters`: wrapping may break before
    /// these characters.
    pub word_wrap_break_before_characters: SharedString,
    /// Monaco `wordWrapBreakAfterCharacters`: wrapping may break after these
    /// characters.
    pub word_wrap_break_after_characters: SharedString,
    /// Monaco `wordBreak`: whether wrapping breaks between CJK characters
    /// (default normal).
    pub word_break: WordBreak,
    /// Monaco `wrapOnEscapedLineFeeds`: while wrapping, a line with a quote
    /// also wraps after each literal `\n` (default `false`).
    pub wrap_on_escaped_line_feeds: bool,
    /// Monaco `wrappingIndent`: indentation of wrapped continuation lines (default
    /// same as the first line).
    pub wrapping_indent: crate::view::line_breaks::WrappingIndent,
    /// Monaco `wrappingStrategy`: wrap by monospace columns (`Simple`, the
    /// default) or by measured widths (`Advanced`, for proportional fonts).
    pub wrapping_strategy: WrappingStrategy,
    /// Monaco `fullwidthCharacterWidth`: draw full-width characters with the
    /// font's width or centered in two cells (default font). Two cells only
    /// apply with a monospace font.
    pub fullwidth_character_width: FullwidthCharacterWidth,
    /// Monaco `folding`: enable code folding and its gutter column (default `true`).
    pub folding: bool,
    /// Monaco `foldingStrategy`.
    pub folding_strategy: FoldingStrategy,
    /// Monaco `foldingMaximumRegions`: provider and syntax ranges beyond
    /// this many are dropped, the most deeply nested first.
    pub folding_maximum_regions: usize,
    /// Monaco `foldingHighlight`: shade collapsed regions' first lines.
    pub folding_highlight: bool,
    /// Monaco `unfoldOnClickAfterEndOfLine`: a click after the end of a
    /// collapsed line unfolds it (a click on its `⋯` always does).
    pub unfold_on_click_after_end_of_line: bool,
    /// Monaco `foldingImportsByDefault`: collapse import ranges when a
    /// model is first shown.
    pub folding_imports_by_default: bool,
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
    /// Monaco `matchBrackets`: highlight the bracket pair at the cursor
    /// (default `Always`).
    pub match_brackets: MatchBrackets,
    /// Monaco `quickSuggestions`: where suggestions open while typing.
    pub quick_suggestions: QuickSuggestions,
    /// Monaco `quickSuggestionsDelay`: how long after typing they open.
    pub quick_suggestions_delay: std::time::Duration,
    /// Monaco `suggestOnTriggerCharacters`.
    pub suggest_on_trigger_characters: bool,
    /// Monaco `acceptSuggestionOnEnter`.
    pub accept_suggestion_on_enter: AcceptSuggestionOnEnter,
    /// `tabCompletion`.
    pub tab_completion: TabCompletion,
    /// Monaco `acceptSuggestionOnCommitCharacter`.
    pub accept_suggestion_on_commit_character: bool,
    /// Monaco `suggestSelection`.
    pub suggest_selection: SuggestSelection,
    /// Monaco `wordBasedSuggestions`.
    pub word_based_suggestions: WordBasedSuggestions,
    /// Monaco `snippetSuggestions` (`None` hides snippets).
    pub snippet_suggestions: Option<crate::suggest::SnippetSortOrder>,
    /// Monaco `suggest`.
    pub suggest: SuggestOptions,
    /// Monaco `gotoLocation`.
    pub goto_location: GotoLocationOptions,
    /// Monaco `peekWidgetDefaultFocus`.
    pub peek_widget_default_focus: PeekWidgetDefaultFocus,
    /// Monaco `definitionLinkOpensInPeek`: Cmd/Ctrl+click on a symbol
    /// shows its definitions in a peek view instead of going there
    /// (default `false`).
    pub definition_link_opens_in_peek: bool,
    /// Monaco `lightbulb.enabled`.
    pub lightbulb: LightbulbMode,
    /// Monaco `inlayHints`.
    pub inlay_hints: InlayHintsOptions,
    /// Monaco `parameterHints`.
    pub parameter_hints: ParameterHintsOptions,
    /// Monaco `smartSelect`.
    pub smart_select: crate::cursor::smart_select::SmartSelectOptions,
    /// Monaco `formatOnType`: format with the on-type formatter when one of
    /// its trigger characters is typed.
    pub format_on_type: bool,
    /// Monaco `formatOnPaste`: format pasted text with the range formatter.
    pub format_on_paste: bool,
    /// Monaco `minimap`.
    pub minimap: MinimapOptions,
    /// Monaco `stickyScroll`.
    pub sticky_scroll: StickyScrollOptions,
    /// Monaco `find`.
    pub find: FindEditorOptions,
    /// Monaco `bracketPairColorization`: color brackets by nesting level.
    pub bracket_pair_colorization: BracketPairColorizationOptions,
    /// Monaco `renderWhitespace`.
    pub render_whitespace: RenderWhitespace,
    /// Monaco `renderControlCharacters`: show control characters as visible
    /// symbols such as `␇` (default `true`).
    pub render_control_characters: bool,
    /// Monaco `renderFinalNewline`.
    pub render_final_newline: RenderFinalNewline,
    /// Monaco `guides`.
    pub guides: GuideOptions,
    /// Monaco `rulers`: vertical lines at visible columns.
    pub rulers: Vec<Ruler>,
    /// Monaco `occurrencesHighlight`: highlight the occurrences of the symbol
    /// at the cursor (default single file).
    pub occurrences_highlight: OccurrencesHighlight,
    /// Monaco `occurrencesHighlightFromAllProviders`: ask every matching
    /// document highlight provider at once and merge their answers instead
    /// of taking the first one; the textual highlights are only used when
    /// no other provider matches (default `false`).
    pub occurrences_highlight_from_all_providers: bool,
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
    /// Monaco `selectedTextMatchMode`: how occurrences of the selected text
    /// match (default the find widget's options).
    pub selected_text_match_mode: SelectedTextMatchMode,
    /// Monaco `multiCursorModifier`.
    pub multi_cursor_modifier: MultiCursorModifier,
    /// Monaco `selectOnLineNumbers`: clicking a line number selects its
    /// line, and dragging selects lines (default `true`). Clicks elsewhere
    /// in the gutter never select.
    pub select_on_line_numbers: bool,
    /// Monaco `mouseStyle`: the mouse cursor over the text (default text).
    pub mouse_style: MouseStyle,
    /// Monaco `mouseMiddleClickAction` (default the usual middle click).
    pub mouse_middle_click_action: MouseMiddleClickAction,
    /// Monaco `hover`.
    pub hover: HoverOptions,
    /// Monaco `multiCursorPaste`: how pasting with several cursors splits
    /// the text.
    pub multi_cursor_paste: crate::cursor::MultiCursorPaste,
    /// Monaco `inlineSuggest`.
    pub inline_suggest: InlineSuggestOptions,
    /// Monaco `codeLens`: show code lenses (on).
    pub code_lens: bool,
    /// Monaco `codeLensFontFamily`; `None` uses the editor's.
    pub code_lens_font_family: Option<SharedString>,
    /// Monaco `codeLensFontSize`; 0 (or under 5) uses 90% of the editor's.
    pub code_lens_font_size: f32,
    /// Monaco `linkedEditing`: edit linked ranges (like HTML tag pairs)
    /// together (off).
    pub linked_editing: bool,
    /// Monaco `semanticHighlighting.enabled`.
    pub semantic_highlighting: SemanticHighlighting,
    /// Monaco `links`: detect links and follow them with Cmd/Ctrl+click.
    pub links: bool,
    /// Monaco `contextmenu`: right-click and Shift+F10 show the editor's
    /// context menu.
    pub context_menu: bool,
    /// Monaco `colorDecorators`: a swatch before each color in the text,
    /// which opens the color picker.
    pub color_decorators: bool,
    /// Monaco `colorDecoratorsLimit`: at most this many swatches (500).
    pub color_decorators_limit: usize,
    /// Monaco `colorDecoratorsActivatedOn`: what opens the picker from a
    /// swatch.
    pub color_decorators_activated_on: ColorDecoratorsActivatedOn,
    /// Monaco `defaultColorDecorators`: when the built-in CSS color finder
    /// runs.
    pub default_color_decorators: DefaultColorDecorators,
    /// Monaco `showUnused`: fade code diagnostics tag unnecessary.
    pub show_unused: bool,
    /// Monaco `showDeprecated`: strike through code diagnostics tag
    /// deprecated.
    pub show_deprecated: bool,
    /// Monaco `renderValidationDecorations`: when diagnostics are drawn.
    pub render_validation_decorations: RenderValidationDecorations,
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
    /// Monaco `stopRenderingLineAfter`: only this many UTF-16 units of a
    /// line are drawn, then a "Show more" chip (default 10 000; `None`
    /// draws everything).
    pub stop_rendering_line_after: Option<u32>,
    /// Monaco `ariaLabel`: the editor's name for screen readers (default
    /// "Editor content").
    pub aria_label: SharedString,
    /// Monaco `ariaRequired`: report the editor as a required input.
    pub aria_required: bool,
    /// Monaco `accessibilityPageSize`: how many lines a screen reader can
    /// read at once around the selection (default 500).
    pub accessibility_page_size: u32,
    /// Monaco `accessibilitySupport`: run optimized for screen readers
    /// (default auto: while assistive technology is connected). Optimized,
    /// wrapping is measured (`wrapping_strategy: Advanced`) without
    /// continuation indent, and a model dominated by long lines wraps at the
    /// viewport unless `word_wrap_override1` says otherwise. Off, the screen
    /// reader gets no text, only a label saying the editor isn't accessible.
    pub accessibility_support: AccessibilitySupport,
    /// Monaco `screenReaderAnnounceInlineSuggestion`: screen readers announce
    /// ghost text when it shows (default `true`).
    pub screen_reader_announce_inline_suggestion: bool,
    /// Monaco `tabFocusMode`: Tab moves focus instead of inserting a tab
    /// (also on for every editor with [`set_tab_focus_mode`](crate::set_tab_focus_mode)).
    pub tab_focus_mode: bool,
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
            font_weight: FontWeight::NORMAL,
            font_ligatures: FontLigatures::Off,
            line_numbers: LineNumbers::On,
            line_numbers_min_chars: 5,
            line_decorations_width: px(10.),
            glyph_margin: true,
            render_line_highlight: RenderLineHighlight::Line,
            cursor_style: CursorStyle::Line,
            cursor_width: px(2.),
            cursor_blinking: CursorBlinking::Blink,
            cursor_smooth_caret_animation: CursorSmoothCaretAnimation::Off,
            cursor_height: px(0.),
            cursor_surrounding_lines: 0,
            cursor_surrounding_lines_style: CursorSurroundingLinesStyle::Default,
            smooth_scrolling: false,
            scroll_beyond_last_column: 4,
            reveal_horizontal_right_padding: px(15.),
            fast_scroll_sensitivity: 5.,
            scroll_predominant_axis: true,
            inertial_scroll: false,
            scroll_on_middle_click: false,
            padding: EditorPadding::default(),
            rounded_selection: true,
            render_line_highlight_only_when_focus: false,
            scroll_beyond_last_line: true,
            allow_variable_line_heights: true,
            read_only: false,
            read_only_message: None,
            placeholder: None,
            unusual_line_terminators: UnusualLineTerminators::Prompt,
            unicode_highlight: UnicodeHighlightOptions::default(),
            drag_and_drop: true,
            double_click_selects_block: true,
            drop_into_editor: DropIntoEditorOptions::default(),
            paste_as: PasteAsOptions::default(),
            selection_clipboard: true,
            auto_indent_on_paste: false,
            auto_indent_on_paste_within_string: true,
            word_separators: "`~!@#$%^&*()-=+[{]}\\|;:'\",.<>/?".into(),
            vertical_scrollbar_size: px(14.),
            horizontal_scrollbar_size: px(12.),
            scrollbar: ScrollbarOptions::default(),
            overview_ruler_lanes: 3,
            overview_ruler_border: true,
            hide_cursor_in_overview_ruler: false,
            mouse_wheel_scroll_sensitivity: 1.0,
            mouse_wheel_zoom: false,
            empty_selection_clipboard: true,
            sticky_tab_stops: false,
            use_tab_stops: true,
            trim_whitespace_on_delete: false,
            word_wrap: WordWrap::Off,
            word_wrap_override1: WordWrapOverride::Inherit,
            word_wrap_override2: WordWrapOverride::Inherit,
            word_wrap_indicator: false,
            word_wrap_column: 80,
            word_wrap_break_before_characters: crate::view::line_breaks::DEFAULT_BREAK_BEFORE
                .into(),
            word_wrap_break_after_characters: crate::view::line_breaks::DEFAULT_BREAK_AFTER.into(),
            word_break: WordBreak::Normal,
            wrap_on_escaped_line_feeds: false,
            wrapping_indent: crate::view::line_breaks::WrappingIndent::Same,
            wrapping_strategy: WrappingStrategy::Simple,
            fullwidth_character_width: FullwidthCharacterWidth::Font,
            folding: true,
            folding_strategy: FoldingStrategy::Auto,
            folding_maximum_regions: 5000,
            folding_highlight: true,
            unfold_on_click_after_end_of_line: false,
            folding_imports_by_default: false,
            show_folding_controls: ShowFoldingControls::Mouseover,
            single_line: false,
            stop_rendering_line_after: Some(10_000),
            aria_label: "Editor content".into(),
            aria_required: false,
            accessibility_page_size: 500,
            tab_focus_mode: false,
            accessibility_support: AccessibilitySupport::Auto,
            screen_reader_announce_inline_suggestion: true,
            auto_closing_brackets: Default::default(),
            auto_closing_quotes: Default::default(),
            auto_closing_comments: Default::default(),
            auto_closing_overtype: Default::default(),
            auto_closing_delete: Default::default(),
            auto_surround: Default::default(),
            auto_indent: Default::default(),
            match_brackets: MatchBrackets::Always,
            quick_suggestions: QuickSuggestions::default(),
            quick_suggestions_delay: std::time::Duration::from_millis(10),
            suggest_on_trigger_characters: true,
            accept_suggestion_on_enter: AcceptSuggestionOnEnter::On,
            tab_completion: TabCompletion::Off,
            accept_suggestion_on_commit_character: true,
            suggest_selection: Default::default(),
            word_based_suggestions: WordBasedSuggestions::OffWithInlineSuggestions,
            snippet_suggestions: Some(crate::suggest::SnippetSortOrder::Inline),
            suggest: SuggestOptions::default(),
            goto_location: GotoLocationOptions::default(),
            peek_widget_default_focus: PeekWidgetDefaultFocus::Tree,
            definition_link_opens_in_peek: false,
            lightbulb: LightbulbMode::OnCode,
            inlay_hints: InlayHintsOptions::default(),
            parameter_hints: ParameterHintsOptions::default(),
            smart_select: Default::default(),
            format_on_type: false,
            format_on_paste: false,
            minimap: MinimapOptions::default(),
            sticky_scroll: StickyScrollOptions::default(),
            find: FindEditorOptions::default(),
            comments: Default::default(),
            overtype_cursor_style: CursorStyle::Block,
            bracket_pair_colorization: BracketPairColorizationOptions::default(),
            render_whitespace: RenderWhitespace::Selection,
            render_control_characters: true,
            render_final_newline: RenderFinalNewline::On,
            guides: GuideOptions::default(),
            rulers: Vec::new(),
            occurrences_highlight: OccurrencesHighlight::SingleFile,
            occurrences_highlight_from_all_providers: false,
            occurrences_highlight_delay: std::time::Duration::from_millis(250),
            selection_highlight: true,
            selection_highlight_max_length: 200,
            selection_highlight_multiline: false,
            selected_text_match_mode: SelectedTextMatchMode::FindOptions,
            multi_cursor_modifier: MultiCursorModifier::Alt,
            select_on_line_numbers: true,
            mouse_style: MouseStyle::Text,
            mouse_middle_click_action: MouseMiddleClickAction::Default,
            hover: HoverOptions::default(),
            multi_cursor_paste: Default::default(),
            inline_suggest: InlineSuggestOptions::default(),
            code_lens: true,
            code_lens_font_family: None,
            code_lens_font_size: 0.,
            linked_editing: false,
            semantic_highlighting: SemanticHighlighting::ConfiguredByTheme,
            links: true,
            context_menu: true,
            color_decorators: true,
            color_decorators_limit: 500,
            color_decorators_activated_on: ColorDecoratorsActivatedOn::ClickAndHover,
            default_color_decorators: DefaultColorDecorators::Auto,
            show_unused: true,
            show_deprecated: true,
            render_validation_decorations: RenderValidationDecorations::Editable,
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
            color_decorators: false,
            context_menu: false,
            ..Self::default()
        }
    }
}

impl EditorOptions {
    /// `word_wrap` after `word_wrap_override2` and `word_wrap_override1`
    /// (Monaco `EditorLayoutInfoComputer.computeLayout`); an `On` override
    /// wraps at the viewport width.
    pub fn effective_word_wrap(&self) -> WordWrap {
        let override1 = match self.word_wrap_override2 {
            WordWrapOverride::Inherit => self.word_wrap_override1,
            forced => forced,
        };
        match override1 {
            WordWrapOverride::Inherit => self.word_wrap,
            WordWrapOverride::On => WordWrap::On,
            WordWrapOverride::Off => WordWrap::Off,
        }
    }

    /// Monaco `EditorLayoutInfoComputer`'s wrapping with
    /// `accessibilitySupport`: [`effective_word_wrap`](Self::effective_word_wrap),
    /// except that optimized for a `screen_reader`, a model
    /// `dominated_by_long_lines` wraps at the viewport unless
    /// `word_wrap_override1` (or `word_wrap_override2`) is set. The second
    /// value says it was forced (Monaco `isWordWrapMinified`).
    pub(crate) fn wrap_mode(
        &self,
        screen_reader: bool,
        dominated_by_long_lines: bool,
    ) -> (WordWrap, bool) {
        let override1 = match self.word_wrap_override2 {
            WordWrapOverride::Inherit => self.word_wrap_override1,
            forced => forced,
        };
        if screen_reader && dominated_by_long_lines && override1 == WordWrapOverride::Inherit {
            return (WordWrap::On, true);
        }
        (self.effective_word_wrap(), false)
    }

    /// The line height in pixels: [`line_height`](Self::line_height), or the font size
    /// times 1.5 on macOS and 1.35 elsewhere, rounded.
    pub fn resolved_line_height(&self) -> Pixels {
        self.line_height.unwrap_or_else(|| {
            let ratio = if cfg!(target_os = "macos") { 1.5 } else { 1.35 };
            px((f32::from(self.font_size) * ratio).round())
        })
    }
}
