//! Model decorations (Monaco `IModelDeltaDecoration`): styled ranges owned by
//! the [`TextModel`](crate::TextModel) that follow edits.
//!
//! Monaco styles decorations with CSS class names; here the options carry the
//! styles directly.

use std::ops::Range;
use std::sync::Arc;

use gpui::{EntityId, FontStyle, FontWeight, Hsla, Pixels, SharedString};
use text::{Patch, Rope, Stickiness};

use crate::providers::MarkdownString;
use crate::theme::{Theme, TokenStyle};

/// Identifies a decoration returned by `TextModel::delta_decorations`.
/// Ids are unique across models, as Monaco's are, so an id from another
/// model (e.g. one an editor showed before `set_model`) never names an
/// unrelated decoration.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct DecorationId(pub u64);

/// The next [`DecorationId`], across models.
static NEXT_DECORATION_ID: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

/// Underline drawn under the decorated text (Monaco uses CSS for squiggles).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DecorationUnderline {
    /// Underline color.
    pub color: Hsla,
    /// Draw a squiggle instead of a straight line.
    pub wavy: bool,
}

/// A decoration's color (Monaco `string | ThemeColor`): resolved, or a
/// theme color id that's looked up in the editor's theme when drawn, so it
/// follows theme changes.
#[derive(Clone, Debug, PartialEq)]
pub enum DecorationColor {
    /// A resolved color.
    Color(Hsla),
    /// A theme color id, such as `editorOverviewRuler.findMatchForeground`
    /// (Monaco `ThemeColor`).
    Theme(SharedString),
}

impl DecorationColor {
    /// The theme color `id` (Monaco `themeColorFromId`).
    pub fn theme(id: impl Into<SharedString>) -> Self {
        Self::Theme(id.into())
    }

    /// The color in `theme`: a theme color id is looked up there (Monaco
    /// `IColorTheme.getColor`); `None` when the theme has no such color.
    pub fn resolve(&self, theme: &Theme) -> Option<Hsla> {
        match self {
            Self::Color(color) => Some(*color),
            Self::Theme(id) => theme.color(id),
        }
    }

    /// Monaco's `darkColor`: `dark` in a dark theme (high contrast dark
    /// included), else `color`.
    fn pick<'a>(
        color: &'a DecorationColor,
        dark: Option<&'a DecorationColor>,
        theme: &Theme,
    ) -> &'a DecorationColor {
        match dark {
            Some(dark) if theme.kind == crate::theme::ThemeKind::Dark => dark,
            _ => color,
        }
    }
}

impl From<Hsla> for DecorationColor {
    fn from(color: Hsla) -> Self {
        Self::Color(color)
    }
}

impl From<gpui::Rgba> for DecorationColor {
    fn from(color: gpui::Rgba) -> Self {
        Self::Color(color.into())
    }
}

/// Monaco `IModelDecorationOverviewRulerOptions`: a mark in the overview
/// ruler (the vertical scrollbar's track).
#[derive(Clone, Debug, PartialEq)]
pub struct OverviewRulerDecoration {
    /// Its color (Monaco `color`).
    pub color: DecorationColor,
    /// Its color in dark themes (Monaco `darkColor`); `None` uses `color`.
    pub dark_color: Option<DecorationColor>,
    /// Its lane (Monaco `position`).
    pub position: OverviewRulerLane,
}

impl OverviewRulerDecoration {
    /// A mark of `color` in `position`'s lane.
    pub fn new(color: impl Into<DecorationColor>, position: OverviewRulerLane) -> Self {
        Self {
            color: color.into(),
            dark_color: None,
            position,
        }
    }

    /// Its color in `theme` (Monaco `getColor`, with `darkColor` in dark
    /// themes).
    pub fn resolve(&self, theme: &Theme) -> Option<Hsla> {
        DecorationColor::pick(&self.color, self.dark_color.as_ref(), theme).resolve(theme)
    }
}

/// Monaco `OverviewRulerLane`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OverviewRulerLane {
    /// Left third of the overview ruler.
    Left,
    /// Middle third of the overview ruler.
    Center,
    /// Right third of the overview ruler.
    Right,
    /// The full width of the overview ruler.
    Full,
}

/// Monaco `MinimapPosition`: where a decoration shows in the minimap.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum MinimapPosition {
    /// Over the decorated characters, with a translucent band across the
    /// line.
    #[default]
    Inline,
    /// A 2px bar in the minimap's left gutter (e.g. dirty-diff marks).
    Gutter,
}

/// Monaco `MinimapSectionHeaderStyle`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum MinimapSectionHeaderStyle {
    /// A label.
    #[default]
    Normal,
    /// A label with a separator line above it.
    Underlined,
}

/// Monaco `IModelDecorationMinimapOptions`: how a decoration shows in the
/// minimap.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct MinimapDecoration {
    /// Its color (Monaco `color`), resolved or a theme color id. `None`
    /// draws no mark, as for section headers; a marker's
    /// [`DecorationOptions::severity`] takes the theme's
    /// `minimap.{error,warning,info}Highlight` instead.
    pub color: Option<DecorationColor>,
    /// Its color in dark themes (Monaco `darkColor`); `None` uses `color`.
    pub dark_color: Option<DecorationColor>,
    /// Where it's drawn.
    pub position: MinimapPosition,
    /// Draws a section header at the decoration's first line (Monaco
    /// `sectionHeaderStyle`).
    pub section_header_style: Option<MinimapSectionHeaderStyle>,
    /// The section header's label (Monaco `sectionHeaderText`).
    pub section_header_text: Option<SharedString>,
}

impl MinimapDecoration {
    /// Its color in `theme`, if it has one (Monaco `getColor`, with
    /// `darkColor` in dark themes).
    pub fn resolve_color(&self, theme: &Theme) -> Option<Hsla> {
        DecorationColor::pick(self.color.as_ref()?, self.dark_color.as_ref(), theme).resolve(theme)
    }
}

/// Monaco `InjectedTextCursorStops`: where the cursor may stop around
/// injected text. The text isn't part of the model, so moving over it
/// doesn't change the cursor's position; the stops decide on which side of
/// the text the cursor shows.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum InjectedTextCursorStops {
    /// Before and after it (the default).
    #[default]
    Both,
    /// After it only.
    Right,
    /// Before it only.
    Left,
    /// Neither.
    None,
}

impl InjectedTextCursorStops {
    /// Monaco `hasLeftCursorStop`.
    pub(crate) fn has_left(self) -> bool {
        matches!(self, Self::Both | Self::Left)
    }

    /// Monaco `hasRightCursorStop`.
    pub(crate) fn has_right(self) -> bool {
        matches!(self, Self::Both | Self::Right)
    }
}

/// Data a host attaches to [`InjectedText`] (Monaco `attachedData`), read
/// back from mouse targets ([`MouseTarget::injected_text`](crate::MouseTarget))
/// and [`TextModel::injected_text_decorations`](crate::TextModel::injected_text_decorations).
/// Two are equal when they're the same allocation.
#[derive(Clone)]
pub struct AttachedData(pub Arc<dyn std::any::Any + Send + Sync>);

impl AttachedData {
    /// Wraps `data`.
    pub fn new(data: impl std::any::Any + Send + Sync) -> Self {
        Self(Arc::new(data))
    }

    /// The data as a `T`, if that is its type.
    pub fn downcast_ref<T: std::any::Any>(&self) -> Option<&T> {
        self.0.downcast_ref()
    }
}

impl PartialEq for AttachedData {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

impl std::fmt::Debug for AttachedData {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("AttachedData").finish_non_exhaustive()
    }
}

/// Token styles over byte ranges of [`InjectedText::content`], sorted and
/// not overlapping.
pub type InjectedTextTokens = Arc<[(Range<usize>, TokenStyle)]>;

/// Text injected before or after the decorated range (Monaco
/// `before`/`after`). It's rendered inline but isn't part of the text.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct InjectedText {
    /// The text to show.
    pub content: SharedString,
    /// Its color and font style.
    pub style: TokenStyle,
    /// Background behind the injected text.
    pub background: Option<Hsla>,
    /// A bordered square of this color centered in the text (Monaco's
    /// `colorpicker-color-decoration`); the content just makes room.
    pub swatch: Option<Hsla>,
    /// Its font; `None` uses the editor's.
    pub font_family: Option<SharedString>,
    /// Its font size; `None` uses the editor's. Smaller text is centered
    /// vertically on the line.
    pub font_size: Option<Pixels>,
    /// The inline box around the text, as Monaco's inlay hints style
    /// theirs with CSS; `None` fills the line's height (less 1px above and
    /// below) behind it, with 3px corners.
    pub text_box: Option<InjectedTextBox>,
    /// An underline under the text, e.g. the squiggle under ghost text
    /// with a warning (Monaco's `.ghost-text-decoration.warning`).
    pub underline: Option<DecorationUnderline>,
    /// Token styles over parts of the text (Monaco's internal `tokens`, as
    /// ghost text colors its suggestion like code): byte ranges of
    /// `content`, sorted and not overlapping, drawn instead of
    /// [`style`](Self::style) there.
    pub tokens: Option<InjectedTextTokens>,
    /// Where the cursor may stop around the text (Monaco `cursorStops`).
    pub cursor_stops: InjectedTextCursorStops,
    /// Host data, read back from mouse targets and
    /// [`TextModel::injected_text_decorations`](crate::TextModel::injected_text_decorations)
    /// (Monaco `attachedData`).
    pub attached_data: Option<AttachedData>,
}

/// Room around [`InjectedText`] and the shape of its background (Monaco's
/// inlay hint CSS: the `paddingLeft`/`paddingRight` spacers, and the
/// `padding` option's `padding` and `border-radius`).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct InjectedTextBox {
    /// Space before and after the background (left, right).
    pub margin: (Pixels, Pixels),
    /// Space inside the background before and after the text (left,
    /// right).
    pub padding: (Pixels, Pixels),
    /// Space inside the background above and below the text: the
    /// background is the text's height (its font's ascent and descent)
    /// plus twice this.
    pub padding_y: Pixels,
    /// The background's corner radii on its left and right side.
    pub corner_radius: (Pixels, Pixels),
}

/// A lane of the glyph margin (Monaco `GlyphMarginLane`). The margin is a
/// line height wide per lane in use, so glyphs in different lanes on the
/// same line show side by side.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum GlyphMarginLane {
    /// The left lane.
    Left = 1,
    /// The center lane, the default.
    #[default]
    Center = 2,
    /// The right lane.
    Right = 3,
}

/// Where a decoration's glyph goes in the glyph margin (Monaco
/// `IModelDecorationGlyphMarginOptions`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct GlyphMarginOptions {
    /// Its lane.
    pub position: GlyphMarginLane,
    /// Keep the lane in the margin on every line, not just the
    /// decoration's (Monaco `persistLane`).
    pub persist_lane: bool,
}

/// Monaco `IModelDecorationOptions`, with styles instead of class names.
///
/// Monaco's `inlineClassNameAffectsLetterSpacing` (here and on injected
/// text) has no field: it tells Monaco's DOM renderer to measure the text
/// again, and text here is always shaped with its styles.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct DecorationOptions {
    /// Background behind the range (Monaco `className`).
    pub background: Option<Hsla>,
    /// A border around the range on each line, inside it (CSS `border`
    /// with `box-sizing: border-box`).
    pub border: Option<Hsla>,
    /// The `border`'s style: solid (the default), or dashed as Monaco's
    /// high contrast themes draw the diff editor's change borders and
    /// (for CSS `dotted`, which GPUI can't draw) find matches and word
    /// highlights.
    pub border_style: gpui::BorderStyle,
    /// The `border`'s width; `None` is 1px (Monaco's current find match,
    /// `.currentFindMatch`, has 2px).
    pub border_width: Option<Pixels>,
    /// A border of this color and width drawn just outside the range on
    /// each line, over the text around it (CSS `outline`, as on Monaco's
    /// in-place replace `valueSetReplacement`).
    pub outline: Option<(Hsla, Pixels)>,
    /// An underline under the range (e.g. diagnostic squiggles).
    pub underline: Option<DecorationUnderline>,
    /// Text color and font style for the range (Monaco `inlineClassName`).
    pub text: Option<TokenStyle>,
    /// Paint `background`/`border` across the whole line(s), under every
    /// decoration that isn't whole-line, whatever their
    /// [`z_index`](Self::z_index) (Monaco's `DecorationsOverlay` renders
    /// whole-line decorations first).
    pub is_whole_line: bool,
    /// Where the range goes on past a line break, paint `background` and
    /// `border` to the right edge of that line (Monaco
    /// `shouldFillLineOnLineBreak`, as on the diff editor's changed
    /// characters).
    pub should_fill_line_on_line_break: bool,
    /// A glyph shown in the glyph margin on every line of the range
    /// (Monaco `glyphMarginClassName`). Codicon characters
    /// ([`Codicon::char`](crate::codicons::Codicon::char)) are drawn with
    /// the codicon font. Where glyphs share a lane, the highest
    /// [`z_index`](Self::z_index) shows.
    pub glyph: Option<(SharedString, Hsla)>,
    /// The glyph margin lane [`glyph`](Self::glyph) goes in (Monaco
    /// `glyphMargin`); `None` is the center lane.
    pub glyph_margin: Option<GlyphMarginOptions>,
    /// Markdown shown when the mouse rests on the glyph margin of a row the
    /// decoration covers, in its [`glyph_margin`](Self::glyph_margin) lane
    /// (Monaco `glyphMarginHoverMessage`), one hover row per string.
    pub glyph_margin_hover_message: Vec<MarkdownString>,
    /// Markdown shown when the mouse rests on the line number of a row the
    /// decoration covers (Monaco `lineNumberHoverMessage`), one hover row
    /// per string.
    pub line_number_hover_message: Vec<MarkdownString>,
    /// Line number color on the decorated rows (Monaco
    /// `lineNumberClassName`).
    pub line_number_color: Option<Hsla>,
    /// A colored bar in the line decorations gutter area (Monaco
    /// `linesDecorationsClassName`, e.g. dirty-diff markers).
    pub gutter_bar: Option<Hsla>,
    /// A bar or text in the line decorations area, as
    /// [`gutter_bar`](Self::gutter_bar) and
    /// [`lines_decoration_text`](Self::lines_decoration_text), but only on
    /// the first view line of each wrapped line (Monaco
    /// `firstLineDecorationClassName`).
    pub first_line_decoration: Option<LinesDecoration>,
    /// Text centered in the line decorations area on every decorated line
    /// (Monaco `linesDecorationsClassName` with an icon, e.g. the diff
    /// editor's `add` and `remove` codicons). Codicon characters
    /// ([`Codicon::char`](crate::codicons::Codicon::char)) are drawn with
    /// the codicon font at 11px.
    pub lines_decoration_text: Option<(SharedString, Hsla)>,
    /// A tooltip shown when the mouse rests on what
    /// [`gutter_bar`](Self::gutter_bar),
    /// [`lines_decoration_text`](Self::lines_decoration_text) or
    /// [`first_line_decoration`](Self::first_line_decoration) draws in the
    /// line decorations area (Monaco `linesDecorationsTooltip`, the
    /// element's `title`).
    pub lines_decorations_tooltip: Option<SharedString>,
    /// A background across the whole gutter on the decorated lines (Monaco
    /// `marginClassName`).
    pub margin_background: Option<Hsla>,
    /// A mark in the overview ruler (the vertical scrollbar track) over the
    /// decorated lines (Monaco `overviewRuler`).
    pub overview_ruler: Option<OverviewRulerDecoration>,
    /// How it shows in the minimap (Monaco `minimap`).
    pub minimap: Option<MinimapDecoration>,
    /// Text shown before the range start (Monaco `before`).
    pub before: Option<InjectedText>,
    /// Text shown after the range end (Monaco `after`).
    pub after: Option<InjectedText>,
    /// Markdown shown when hovering the range, or anywhere on its lines
    /// with [`is_whole_line`](Self::is_whole_line) (Monaco `hoverMessage`),
    /// one hover row per string.
    pub hover_message: Vec<MarkdownString>,
    /// Draws the decoration when its range is empty too (Monaco
    /// `showIfCollapsed`): on an empty range, its [`before`](Self::before)
    /// and [`after`](Self::after) text only shows with this, and with it an
    /// empty or narrow [`background`](Self::background) is widened to a
    /// typical character's width (Monaco `typicalHalfwidthCharacterWidth`),
    /// centered, and a range ending at the start of a line stops at the
    /// end of the line before.
    pub show_if_collapsed: bool,
    /// Where the range goes on past a line break, widen
    /// [`background`](Self::background) by half a typical character's width
    /// past the end of that line (Monaco's `includeNewLines`, which it
    /// passes for find matches, the `findMatch` class name).
    pub include_new_lines: bool,
    /// The smallest width of [`background`](Self::background) and
    /// [`border`](Self::border) on a line (CSS `min-width`, as Monaco's
    /// `snippet-placeholder` has 2px). Otherwise an empty piece draws
    /// nothing, as Monaco's 0px wide `className` element, though its
    /// [`outline`](Self::outline) shows.
    pub min_width: Pixels,
    /// How the range grows when text is typed at its edges (Monaco `stickiness`).
    pub stickiness: Stickiness,
    /// Collapses the range to the start of an edit that replaces all of it
    /// (Monaco `collapseOnReplaceEdit`), as links and code lenses do, so
    /// pasting over a link doesn't decorate the pasted text.
    pub collapse_on_replace_edit: bool,
    /// Higher paints on top, among the whole-line decorations and among the
    /// others.
    pub z_index: i32,
    /// A diagnostic's decoration (Monaco `isForValidation`), hidden as the
    /// editor's `render_validation_decorations` says.
    pub validation: bool,
    /// Not drawn while its range lies in comment tokens (Monaco
    /// `hideInCommentTokens`, as the unicode highlighter's
    /// `includeComments: false`).
    pub hide_in_comment_tokens: bool,
    /// Not drawn while its range lies in string tokens (Monaco
    /// `hideInStringTokens`).
    pub hide_in_string_tokens: bool,
    /// A marker of this severity (Monaco's `squiggly-error` and similar
    /// class names): the squiggle, background and overview ruler mark take
    /// the editor theme's `editorError.*`, `editorWarning.*` or
    /// `editorInfo.*` colors instead of `underline`'s and `overview_ruler`'s.
    pub severity: Option<crate::MarkerSeverity>,
    /// Unnecessary (e.g. unused) code, faded when the editor's
    /// `show_unused` is on (Monaco `squiggly-inline-unnecessary`).
    pub unnecessary: bool,
    /// Deprecated code, struck through when the editor's `show_deprecated`
    /// is on (Monaco `squiggly-inline-deprecated`).
    pub deprecated: bool,
    /// The decorated lines' height, as a multiple of the editor's line
    /// height (Monaco `lineHeight`). The tallest decoration on a line wins;
    /// the editor's `allow_variable_line_heights` turns it off.
    pub line_height: Option<f32>,
    /// A bordered box around the decorated lines and the view zones above
    /// them (Monaco `blockClassName` with `blockPadding`).
    pub block: Option<BlockDecoration>,
    /// A bar at the range's start, drawn for an empty range too (Monaco's
    /// CSS `border-left`, as on the diff editor's `diff-range-empty`
    /// markers).
    pub left_border: Option<LeftBorder>,
    /// The decorated text's font family (Monaco `fontFamily`); `None` keeps
    /// the editor's. Line wrapping still measures the editor's font.
    pub font_family: Option<SharedString>,
    /// The decorated text's weight (Monaco `fontWeight`).
    pub font_weight: Option<FontWeight>,
    /// The decorated text's style, e.g. italic (Monaco `fontStyle`).
    pub font_style: Option<FontStyle>,
    /// The direction of the decorated lines' text (Monaco `textDirection`),
    /// kept for hosts and the getters. GPUI has no bidirectional text
    /// layout, so lines are still laid out left to right.
    pub text_direction: Option<TextDirection>,
}

/// A border Monaco draws `dotted` in high contrast themes (find matches,
/// word and selection highlights), dashed there as GPUI has no dotted
/// style, and solid otherwise.
pub(crate) fn dotted_in_high_contrast(high_contrast: bool) -> gpui::BorderStyle {
    if high_contrast {
        gpui::BorderStyle::Dashed
    } else {
        gpui::BorderStyle::Solid
    }
}

impl DecorationOptions {
    /// Whether its injected text shows on `range`: always on a non-empty
    /// range, and on an empty one with
    /// [`show_if_collapsed`](Self::show_if_collapsed) (Monaco
    /// `getInjectedTextInInterval`).
    pub(crate) fn shows_injected_text(&self, range: &Range<usize>) -> bool {
        self.show_if_collapsed || !range.is_empty()
    }

    /// Whether it injects `before` or `after` text.
    pub(crate) fn has_injected_text(&self) -> bool {
        self.before.is_some() || self.after.is_some()
    }

    /// Whether it changes the decorated text's font.
    pub(crate) fn has_font(&self) -> bool {
        self.font_family.is_some() || self.font_weight.is_some() || self.font_style.is_some()
    }
}

/// What a decoration draws in the line decorations area, between the line
/// numbers and the text (Monaco's `linesDecorationsClassName` and
/// `firstLineDecorationClassName` with a bar or an icon).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct LinesDecoration {
    /// A 3px colored bar, as [`DecorationOptions::gutter_bar`].
    pub bar: Option<Hsla>,
    /// Centered text, as [`DecorationOptions::lines_decoration_text`].
    pub text: Option<(SharedString, Hsla)>,
}

/// A bar at a decoration's start ([`DecorationOptions::left_border`]).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LeftBorder {
    /// Its color.
    pub color: Hsla,
    /// Its width.
    pub width: Pixels,
    /// How far it's moved right of the range's start (CSS `margin-left`;
    /// negative moves left).
    pub offset: Pixels,
}

/// Monaco's block decoration: a box spanning the content width around
/// whole lines.
///
/// By default it runs from the top of the view zones above the first
/// decorated line to the bottom of those below the last; an empty range
/// covers only the zones above its line (Monaco's collapsed block).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct BlockDecoration {
    /// The border's color.
    pub border: Hsla,
    /// The border's width.
    pub border_width: Pixels,
    /// Monaco `blockPadding`: top, right, bottom and left, outside the
    /// lines.
    pub padding: [Pixels; 4],
    /// Monaco `blockIsAfterEnd`: the box covers just the view zones below
    /// the range's last line (meant for an empty range).
    pub is_after_end: bool,
    /// Monaco `blockDoesNotCollapse`: an empty range's box still covers its
    /// line, not only the zones above it.
    pub does_not_collapse: bool,
}

/// The direction of a decorated line's text (Monaco `TextDirection`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum TextDirection {
    /// Left to right (Monaco `LTR`).
    #[default]
    Ltr,
    /// Right to left (Monaco `RTL`).
    Rtl,
}

/// A decoration as stored in the model.
#[derive(Clone, Debug)]
pub struct Decoration {
    /// Its id.
    pub id: DecorationId,
    /// The decorated byte range in the current text.
    pub range: Range<usize>,
    /// How it's drawn.
    pub options: Arc<DecorationOptions>,
    /// `None` shows in every editor; `Some(editor)` only in that editor.
    pub owner: Option<EntityId>,
}

/// All decorations of a model, kept in byte offsets and moved through every
/// edit.
#[derive(Clone, Debug, Default)]
pub struct DecorationSet {
    decorations: Vec<Decoration>,
    /// Length of the longest decoration, to bound range queries.
    max_len: usize,
    sorted: bool,
    /// Bumped on every change, so views can cache what they derive.
    version: u64,
    /// Bumped when a decoration with injected text is added, removed or
    /// changed (not moved by an edit): the view lines' injected text
    /// (Monaco's `ModelInjectedTextChangedEvent`).
    injected_text_version: u64,
    /// How many decorations have injected text.
    injected_text_count: usize,
    /// The byte range of each recent injected text change, with the
    /// `injected_text_version` it made, moved through edits since (Monaco's
    /// `ModelInjectedTextChangedEvent` line numbers). The oldest drop off.
    injected_text_changes: std::collections::VecDeque<(u64, Range<usize>)>,
    /// Whether a glyph picks its glyph margin lane, computed when asked
    /// after a change.
    has_glyph_lanes: std::cell::Cell<Option<bool>>,
    /// Each decoration's position in `decorations`, built when a lookup by
    /// id needs it and dropped when decorations are added, removed or
    /// reordered (see [`DecorationSet::get`]).
    index: std::cell::OnceCell<std::collections::HashMap<DecorationId, usize>>,
}

/// How many decorations [`DecorationSet::get`] scans before it looks ids up
/// in an index instead.
const INDEX_THRESHOLD: usize = 32;

impl DecorationSet {
    /// The number of decorations, for all editors.
    pub fn len(&self) -> usize {
        self.decorations.len()
    }

    /// Whether there are no decorations.
    pub fn is_empty(&self) -> bool {
        self.decorations.is_empty()
    }

    /// A number that changes whenever a decoration is added, removed,
    /// changed or moved.
    pub fn version(&self) -> u64 {
        self.version
    }

    /// A number that changes whenever a decoration with `before` or
    /// `after` text is added, removed or changed; edits move injected text
    /// without changing it.
    pub(crate) fn injected_text_version(&self) -> u64 {
        self.injected_text_version
    }

    /// Whether any decoration has `before` or `after` text.
    pub(crate) fn has_injected_text(&self) -> bool {
        self.injected_text_count > 0
    }

    /// The byte ranges (in the current text) of the injected text changes
    /// since `injected_text_version` was `version`, or `None` when they
    /// aren't all known (too many, or the decorations were cleared).
    pub(crate) fn injected_text_changes_since(&self, version: u64) -> Option<Vec<Range<usize>>> {
        if version == self.injected_text_version {
            return Some(Vec::new());
        }
        let first = self
            .injected_text_changes
            .iter()
            .position(|(v, _)| *v == version + 1)?;
        Some(
            self.injected_text_changes
                .range(first..)
                .map(|(_, range)| range.clone())
                .collect(),
        )
    }

    /// Counts `options`' injected text at `range` in or out.
    fn note_injected_text(
        &mut self,
        options: &DecorationOptions,
        range: Range<usize>,
        added: bool,
    ) {
        if options.has_injected_text() {
            self.note_injected_text_at(range, added);
        }
    }

    /// Counts injected text at `range` in or out.
    fn note_injected_text_at(&mut self, range: Range<usize>, added: bool) {
        /// How many changes are kept: editors catch up every frame, and
        /// gather everything again after more.
        const KEPT_CHANGES: usize = 64;
        if added {
            self.injected_text_count += 1;
        } else {
            self.injected_text_count -= 1;
        }
        self.injected_text_version += 1;
        if self.injected_text_changes.len() == KEPT_CHANGES {
            self.injected_text_changes.pop_front();
        }
        self.injected_text_changes
            .push_back((self.injected_text_version, range));
    }

    /// Removes `old` and adds `new`, returning the new ids in order.
    pub fn delta(
        &mut self,
        owner: Option<EntityId>,
        old: &[DecorationId],
        new: impl IntoIterator<Item = (Range<usize>, DecorationOptions)>,
    ) -> Vec<DecorationId> {
        if !old.is_empty() {
            let mut removed = Vec::new();
            // Replacing many decorations (thousands of markers) would be
            // quadratic with a scan of `old` per decoration.
            let old_set: Option<std::collections::HashSet<DecorationId>> =
                (old.len() > INDEX_THRESHOLD).then(|| old.iter().copied().collect());
            let is_old = |id: &DecorationId| match &old_set {
                Some(set) => set.contains(id),
                None => old.contains(id),
            };
            self.decorations.retain(|d| {
                let keep = !is_old(&d.id);
                if !keep && d.options.has_injected_text() {
                    removed.push(d.range.clone());
                }
                keep
            });
            for range in removed {
                self.note_injected_text_at(range, false);
            }
        }
        let mut ids = Vec::new();
        for (range, options) in new {
            let id =
                DecorationId(NEXT_DECORATION_ID.fetch_add(1, std::sync::atomic::Ordering::Relaxed));
            let range = range.start.min(range.end)..range.end.max(range.start);
            self.max_len = self.max_len.max(range.len());
            self.note_injected_text(&options, range.clone(), true);
            self.decorations.push(Decoration {
                id,
                range,
                options: Arc::new(options),
                owner,
            });
            ids.push(id);
        }
        self.sorted = false;
        self.version += 1;
        self.has_glyph_lanes.set(None);
        self.index.take();
        ids
    }

    /// Replaces the options of decoration `id`, keeping its range and id
    /// (Monaco `changeDecorationOptions`). Returns whether it exists.
    pub fn change_options(&mut self, id: DecorationId, options: DecorationOptions) -> bool {
        match self.position(id) {
            Some(ix) => {
                let old = std::mem::replace(&mut self.decorations[ix].options, Arc::new(options));
                let new = self.decorations[ix].options.clone();
                let range = self.decorations[ix].range.clone();
                self.note_injected_text(&old, range.clone(), false);
                self.note_injected_text(&new, range, true);
                self.version += 1;
                self.has_glyph_lanes.set(None);
                true
            }
            None => false,
        }
    }

    /// Removes every decoration, returning what they affected, or `None`
    /// when there were none.
    pub fn clear(&mut self) -> Option<crate::events::DecorationsChange> {
        if self.decorations.is_empty() {
            return None;
        }
        let change = self
            .decorations
            .iter()
            .fold(crate::events::DecorationsChange::default(), |change, d| {
                change.with(&d.options)
            });
        self.decorations.clear();
        self.index.take();
        self.max_len = 0;
        self.version += 1;
        if self.injected_text_count > 0 {
            // A change of its own, not logged: views gather it all again.
            self.injected_text_count = 0;
            self.injected_text_version += 1;
            self.injected_text_changes.clear();
        }
        self.has_glyph_lanes.set(None);
        Some(change)
    }

    /// Removes the decorations `owner` owns (Monaco
    /// `removeAllDecorationsWithOwnerId`), returning what they affected, or
    /// `None` when there were none.
    pub fn remove_owned_by(&mut self, owner: EntityId) -> Option<crate::events::DecorationsChange> {
        let mut change = None;
        let mut injected_text_removed = Vec::new();
        self.has_glyph_lanes.set(None);
        self.decorations.retain(|d| {
            if d.owner == Some(owner) {
                change = Some(
                    change
                        .unwrap_or(crate::events::DecorationsChange::default())
                        .with(&d.options),
                );
                if d.options.has_injected_text() {
                    injected_text_removed.push(d.range.clone());
                }
                false
            } else {
                true
            }
        });
        for range in injected_text_removed {
            self.note_injected_text_at(range, false);
        }
        if change.is_some() {
            self.version += 1;
            self.index.take();
        }
        change
    }

    /// Every decoration, in no particular order.
    pub fn iter(&self) -> impl Iterator<Item = &Decoration> {
        self.decorations.iter()
    }

    /// The decoration with `id`, if it still exists.
    pub fn get(&self, id: DecorationId) -> Option<&Decoration> {
        self.position(id).map(|ix| &self.decorations[ix])
    }

    /// Where the decoration with `id` is in `decorations`: a scan of a few,
    /// and otherwise an index of all of them, kept until they're added,
    /// removed or reordered, so looking up each of thousands of markers'
    /// decorations isn't quadratic.
    fn position(&self, id: DecorationId) -> Option<usize> {
        if self.decorations.len() <= INDEX_THRESHOLD {
            return self.decorations.iter().position(|d| d.id == id);
        }
        self.index
            .get_or_init(|| {
                self.decorations
                    .iter()
                    .enumerate()
                    .map(|(ix, d)| (d.id, ix))
                    .collect()
            })
            .get(&id)
            .copied()
    }

    fn ensure_sorted(&mut self) {
        if !self.sorted {
            let key = |d: &Decoration| (d.range.start, d.range.end, d.id);
            // Edits mostly shift decorations without reordering them; the
            // index stays valid then.
            if !self.decorations.is_sorted_by_key(key) {
                self.decorations.sort_by_key(key);
                self.index.take();
            }
            self.sorted = true;
        }
    }

    /// Moves every decoration through `patch`, which turned `old` into
    /// `new`, as Monaco's `nodeAcceptEdit` does (see
    /// [`DecorationSet::apply_patch_forcing`]).
    pub fn apply_patch(&mut self, patch: &Patch, old: &Rope, new: &Rope) {
        self.apply_patch_forcing(patch, old, new, &[]);
    }

    /// Moves every decoration through `patch`, which turned `old` into
    /// `new` (Monaco `nodeAcceptEdit`, for each of the patch's edits):
    ///
    /// - An edge before an edit stays and one after it shifts by the
    ///   edit's change in length.
    /// - An edge at the start of a replace stays, and one at an insertion
    ///   point follows its stickiness.
    /// - An edge inside a replace stays as far into the new text as it was
    ///   into the old while that's within both (moving past the common
    ///   length's end only as its stickiness says, and staying when more is
    ///   deleted than inserted), and otherwise moves to the new text's end.
    ///   Monaco counts UTF-16 code units; an edge that would land inside a
    ///   character moves to that character's end.
    /// - A decoration with [`DecorationOptions::collapse_on_replace_edit`]
    ///   that an edit covers collapses to the edit's start.
    ///
    /// Edges inside or at the edges of one of the `force` ranges (old byte
    /// ranges of the patch's edits) move past that edit's new text (Monaco
    /// `forceMoveMarkers`).
    pub fn apply_patch_forcing(
        &mut self,
        patch: &Patch,
        old: &Rope,
        new: &Rope,
        force: &[Range<usize>],
    ) {
        if patch.is_empty() {
            return;
        }
        let edits = patch.edits();
        let forced = |offset: usize| {
            force
                .iter()
                .find(|range| range.start <= offset && offset <= range.end)
                .map(|range| patch.transform_offset(range.end, text::Bias::Right))
        };
        let mut spans = ReplacedSpans::new(edits, old, new);
        let mut max_len = 0;
        for decoration in &mut self.decorations {
            let range = &decoration.range;
            let ix = edits.partition_point(|edit| edit.old.end < range.start);
            let next = edits.get(ix);
            let (start, end) = if force.is_empty()
                && next.is_none_or(|edit| range.end < edit.old.start)
            {
                // Wholly between two edits: both edges shift alike.
                let (old_end, new_end) = ix
                    .checked_sub(1)
                    .map_or((0, 0), |ix| (edits[ix].old.end, edits[ix].new.end));
                (
                    range.start - old_end + new_end,
                    range.end - old_end + new_end,
                )
            } else if let Some(edit) = next.filter(|edit| {
                decoration.options.collapse_on_replace_edit
                    && edit.old.start <= range.start
                    && range.end <= edit.old.end
            }) {
                (edit.new.start, edit.new.start)
            } else {
                let (start_bias, end_bias) = decoration.options.stickiness.biases();
                let start = forced(range.start).unwrap_or_else(|| {
                    spans.accept_edit(range.start, start_bias == text::Bias::Left)
                });
                let end = forced(range.end)
                    .unwrap_or_else(|| spans.accept_edit(range.end, end_bias == text::Bias::Left));
                (start, end.max(start))
            };
            decoration.range = start..end;
            max_len = max_len.max(end - start);
        }
        for (_, range) in &mut self.injected_text_changes {
            let start = patch.transform_offset(range.start, text::Bias::Left);
            let end = patch
                .transform_offset(range.end, text::Bias::Right)
                .max(start);
            *range = start..end;
        }
        self.max_len = max_len;
        self.sorted = false;
        self.version += 1;
    }

    /// Decorations intersecting `range` (inclusive of touching empty ones)
    /// visible to `editor`, sorted by z-index then start.
    pub fn in_range(&mut self, range: Range<usize>, editor: Option<EntityId>) -> Vec<Decoration> {
        self.ensure_sorted();
        let from = range.start.saturating_sub(self.max_len);
        let first = self.decorations.partition_point(|d| d.range.start < from);
        let mut result: Vec<Decoration> = self.decorations[first..]
            .iter()
            .take_while(|d| d.range.start <= range.end)
            .filter(|d| d.range.end >= range.start)
            .filter(|d| d.owner.is_none() || d.owner == editor)
            .cloned()
            .collect();
        result.sort_by_key(|d| (d.options.z_index, d.range.start));
        result
    }

    /// The ranges and line height multipliers of the decorations with a
    /// `line_height`, as seen by `editor`.
    pub fn line_heights(&self, editor: Option<EntityId>) -> Vec<(Range<usize>, f32)> {
        self.decorations
            .iter()
            .filter(|d| d.owner.is_none() || d.owner == editor)
            .filter_map(|d| Some((d.range.clone(), d.options.line_height?)))
            .collect()
    }

    /// Whether a glyph picks its glyph margin lane, so the margin may need
    /// more than one (most models' don't, which skips Monaco's per-frame
    /// `_computeGlyphMarginLanes`).
    pub(crate) fn has_glyph_lanes(&self) -> bool {
        if let Some(has) = self.has_glyph_lanes.get() {
            return has;
        }
        let has = self
            .decorations
            .iter()
            .any(|d| d.options.glyph.is_some() && d.options.glyph_margin.is_some());
        self.has_glyph_lanes.set(Some(has));
        has
    }

    /// All decorations visible to `editor` (for the overview ruler).
    pub fn all(&mut self, editor: Option<EntityId>) -> Vec<Decoration> {
        self.ensure_sorted();
        self.decorations
            .iter()
            .filter(|d| d.owner.is_none() || d.owner == editor)
            .cloned()
            .collect()
    }
}

/// Moves a decoration edge at `offset` through `patch`, which turned `old`
/// into `new`, as [`DecorationSet::apply_patch`] does; `bias` is the edge's
/// side from [`Stickiness::biases`]. For ranges kept outside the model, such
/// as the editor's auto-closed characters.
pub(crate) fn move_edge(
    patch: &Patch,
    old: &Rope,
    new: &Rope,
    offset: usize,
    bias: text::Bias,
) -> usize {
    ReplacedSpans::new(patch.edits(), old, new).accept_edit(offset, bias == text::Bias::Left)
}

/// A patch's edits, which turned `old` into `new`, measuring a replace in
/// UTF-16 code units only once an edge falls strictly inside it, and then
/// only once: edges outside every replace, and every edge of a pure
/// insertion or deletion, never touch the ropes.
struct ReplacedSpans<'a> {
    edits: &'a [text::Edit],
    old: &'a Rope,
    new: &'a Rope,
    /// Per edit, once measured: [`Utf16Span`].
    utf16: Vec<Option<Utf16Span>>,
}

/// Where a replace's old and new text start, and their lengths, in UTF-16
/// code units.
#[derive(Clone, Copy)]
struct Utf16Span {
    old_start: usize,
    new_start: usize,
    deleted: usize,
    inserted: usize,
}

impl<'a> ReplacedSpans<'a> {
    fn new(edits: &'a [text::Edit], old: &'a Rope, new: &'a Rope) -> Self {
        Self {
            edits,
            old,
            new,
            utf16: Vec::new(),
        }
    }

    fn utf16(&mut self, ix: usize) -> Utf16Span {
        if self.utf16.is_empty() {
            self.utf16.resize(self.edits.len(), None);
        }
        let (edits, old, new) = (self.edits, self.old, self.new);
        *self.utf16[ix].get_or_insert_with(|| {
            let edit = &edits[ix];
            let old_start = old.offset_to_offset_utf16(edit.old.start).0;
            let new_start = new.offset_to_offset_utf16(edit.new.start).0;
            Utf16Span {
                old_start,
                new_start,
                deleted: old.offset_to_offset_utf16(edit.old.end).0 - old_start,
                inserted: new.offset_to_offset_utf16(edit.new.end).0 - new_start,
            }
        })
    }

    /// Monaco `nodeAcceptEdit` for an edge at `offset` that's
    /// `stick_to_previous` (its stickiness), without `forceMoveMarkers`.
    fn accept_edit(&mut self, offset: usize, stick_to_previous: bool) -> usize {
        let edits = self.edits;
        let ix = edits.partition_point(|edit| edit.old.end < offset);
        let Some(edit) = edits.get(ix).filter(|edit| edit.old.start <= offset) else {
            // Before every edit from `ix` on: shift by the edits before it.
            return match ix.checked_sub(1).map(|ix| &edits[ix]) {
                Some(prev) => offset - prev.old.end + prev.new.end,
                None => offset,
            };
        };
        if offset == edit.old.start {
            // Monaco: `ForceStay` at the start of a replace; the stickiness
            // decides at an insertion point.
            return if edit.old_len() > 0 || stick_to_previous {
                edit.new.start
            } else {
                edit.new.end
            };
        }
        if edit.new_len() == 0 {
            return edit.new.start;
        }
        // Inside the replaced text: Monaco measures in UTF-16 code units.
        let span = self.utf16(ix);
        let into = self.old.offset_to_offset_utf16(offset).0 - span.old_start;
        if !stays_in_common_prefix(into, span.deleted, span.inserted, stick_to_previous) {
            return edit.new.end;
        }
        let new = self.new;
        let target = span.new_start + into;
        let at = new.offset_utf16_to_offset(text::OffsetUtf16(target));
        if new.offset_to_offset_utf16(at).0 < target {
            // Inside a surrogate pair: never split the character.
            at + new.chars_at(at).next().map_or(0, char::len_utf8)
        } else {
            at
        }
    }
}

/// Monaco `nodeAcceptEdit`'s second check: whether an edge `into` units
/// into a replace of `deleted` units by `inserted` (`0 < into <= deleted`)
/// stays where it is rather than moving to the end of the new text. It
/// stays within the first `min(deleted, inserted)` units, and at that
/// boundary when more is deleted than inserted (`ForceStay`) or its
/// stickiness says so.
fn stays_in_common_prefix(
    into: usize,
    deleted: usize,
    inserted: usize,
    stick_to_previous: bool,
) -> bool {
    let common = deleted.min(inserted);
    into < common || (into == common && common > 0 && (deleted > inserted || stick_to_previous))
}

/// Stickiness re-exported for decoration options.
pub use text::Stickiness as TrackedRangeStickiness;

#[cfg(test)]
mod tests {
    use super::*;
    use text::Buffer;

    /// Applies `edits` to `buffer` and moves `set` through them, forcing
    /// the edits in `force` past their decorations.
    fn edit(
        buffer: &mut Buffer,
        set: &mut DecorationSet,
        edits: &[(Range<usize>, &str)],
        force: &[Range<usize>],
    ) {
        let old = buffer.rope().clone();
        let patch = buffer.edit(edits.iter().cloned()).unwrap();
        set.apply_patch_forcing(&patch, &old, buffer.rope(), force);
    }

    fn with_stickiness(stickiness: Stickiness) -> DecorationOptions {
        DecorationOptions {
            stickiness,
            ..Default::default()
        }
    }

    #[test]
    fn decorations_follow_edits_with_stickiness() {
        let mut buffer = Buffer::new("hello world");
        let mut set = DecorationSet::default();
        let ids = set.delta(
            None,
            &[],
            [
                (6..11, DecorationOptions::default()),
                (
                    6..11,
                    DecorationOptions {
                        stickiness: Stickiness::NeverGrowsWhenTypingAtEdges,
                        ..Default::default()
                    },
                ),
            ],
        );
        edit(&mut buffer, &mut set, &[(6..6, ">"), (11..11, "<")], &[]);
        assert_eq!(set.get(ids[0]).unwrap().range, 6..13);
        assert_eq!(set.get(ids[1]).unwrap().range, 7..12);
        let found = set.in_range(0..3, None);
        assert!(found.is_empty());
        assert_eq!(set.in_range(12..12, None).len(), 2);
        set.delta(None, &ids[..1], []);
        assert_eq!(set.len(), 1);
    }

    /// Monaco `nodeAcceptEdit` with `forceMoveMarkers`: edges inside or at
    /// the edges of the edit move past its text, whatever the stickiness.
    #[test]
    fn forced_edits_move_decorations_past_their_text() {
        let mut buffer = Buffer::new("hello world");
        let mut set = DecorationSet::default();
        let never_grows = || DecorationOptions {
            stickiness: Stickiness::NeverGrowsWhenTypingAtEdges,
            ..Default::default()
        };
        let ids = set.delta(None, &[], [(0..5, never_grows()), (6..11, never_grows())]);
        edit(
            &mut buffer,
            &mut set,
            &[(5..6, "__")],
            std::slice::from_ref(&(5..6)),
        );
        assert_eq!(set.get(ids[0]).unwrap().range, 0..7);
        assert_eq!(set.get(ids[1]).unwrap().range, 7..12);
        // Not forced: they keep to their text.
        edit(&mut buffer, &mut set, &[(7..7, "+")], &[]);
        assert_eq!(set.get(ids[1]).unwrap().range, 8..13);
    }

    /// Decorations wholly before, between or after a patch's edits shift by
    /// the edits before them, as edges moved one at a time would.
    #[test]
    fn decorations_between_edits_shift() {
        let mut buffer = Buffer::new("0123456789abcdefghij");
        let mut set = DecorationSet::default();
        let ranges = [0..2, 4..4, 7..9, 9..9, 12..15, 18..20];
        let ids = set.delta(
            None,
            &[],
            ranges
                .iter()
                .map(|range| (range.clone(), DecorationOptions::default())),
        );
        let old = buffer.rope().clone();
        let patch = buffer
            .edit([(2..4, "\u{1f600}\u{1f600}"), (10..10, "+"), (15..18, "")])
            .unwrap();
        set.apply_patch(&patch, &old, buffer.rope());
        let moved: Vec<_> = ids
            .iter()
            .map(|id| set.get(*id).unwrap().range.clone())
            .collect();
        // 0..2 and 12..15 end where a replace starts (`ForceStay`), 4..4
        // sits inside the first replace (2 of 2 units in, the emoji pair is
        // 4) and 18..20 starts at a deletion's end.
        assert_eq!(moved, [0..2, 6..10, 13..15, 15..15, 19..22, 22..24]);
    }

    /// Monaco's `nodeAcceptEdit` test (`intervalTree.test.ts`): each row is
    /// a decoration and its stickiness, an edit replacing a range with that
    /// many characters, whether it forces markers to move, and where the
    /// decoration ends up.
    #[test]
    fn node_accept_edit() {
        use Stickiness::{
            AlwaysGrowsWhenTypingAtEdges as A, GrowsOnlyWhenTypingAfter as F,
            GrowsOnlyWhenTypingBefore as B, NeverGrowsWhenTypingAtEdges as N,
        };
        type Row = (
            &'static str,
            Range<usize>,
            Stickiness,
            Range<usize>,
            usize,
            bool,
            Range<usize>,
        );
        #[rustfmt::skip]
        let rows: &[Row] = &[
            ("A.000", 0..0, A, 0..0, 0, false, 0..0),
            ("A.001", 0..0, N, 0..0, 0, false, 0..0),
            ("A.002", 0..0, B, 0..0, 0, false, 0..0),
            ("A.003", 0..0, F, 0..0, 0, false, 0..0),
            ("A.004", 0..0, A, 0..0, 0, true, 0..0),
            ("A.005", 0..0, N, 0..0, 0, true, 0..0),
            ("A.006", 0..0, B, 0..0, 0, true, 0..0),
            ("A.007", 0..0, F, 0..0, 0, true, 0..0),
            ("A.008", 0..0, A, 0..0, 1, false, 0..1),
            ("A.009", 0..0, N, 0..0, 1, false, 1..1),
            ("A.010", 0..0, B, 0..0, 1, false, 0..0),
            ("A.011", 0..0, F, 0..0, 1, false, 1..1),
            ("A.012", 0..0, A, 0..0, 1, true, 1..1),
            ("A.013", 0..0, N, 0..0, 1, true, 1..1),
            ("A.014", 0..0, B, 0..0, 1, true, 1..1),
            ("A.015", 0..0, F, 0..0, 1, true, 1..1),
            ("B.000", 0..5, A, 0..0, 0, false, 0..5),
            ("B.001", 0..5, N, 0..0, 0, false, 0..5),
            ("B.002", 0..5, B, 0..0, 0, false, 0..5),
            ("B.003", 0..5, F, 0..0, 0, false, 0..5),
            ("B.004", 0..5, A, 0..0, 0, true, 0..5),
            ("B.005", 0..5, N, 0..0, 0, true, 0..5),
            ("B.006", 0..5, B, 0..0, 0, true, 0..5),
            ("B.007", 0..5, F, 0..0, 0, true, 0..5),
            ("B.008", 0..5, A, 0..0, 1, false, 0..6),
            ("B.009", 0..5, N, 0..0, 1, false, 1..6),
            ("B.010", 0..5, B, 0..0, 1, false, 0..6),
            ("B.011", 0..5, F, 0..0, 1, false, 1..6),
            ("B.012", 0..5, A, 0..0, 1, true, 1..6),
            ("B.013", 0..5, N, 0..0, 1, true, 1..6),
            ("B.014", 0..5, B, 0..0, 1, true, 1..6),
            ("B.015", 0..5, F, 0..0, 1, true, 1..6),
            ("B.016", 0..5, A, 2..2, 1, false, 0..6),
            ("B.017", 0..5, N, 2..2, 1, false, 0..6),
            ("B.018", 0..5, B, 2..2, 1, false, 0..6),
            ("B.019", 0..5, F, 2..2, 1, false, 0..6),
            ("B.020", 0..5, A, 2..2, 1, true, 0..6),
            ("B.021", 0..5, N, 2..2, 1, true, 0..6),
            ("B.022", 0..5, B, 2..2, 1, true, 0..6),
            ("B.023", 0..5, F, 2..2, 1, true, 0..6),
            ("B.024", 0..5, A, 5..5, 1, false, 0..6),
            ("B.025", 0..5, N, 5..5, 1, false, 0..5),
            ("B.026", 0..5, B, 5..5, 1, false, 0..5),
            ("B.027", 0..5, F, 5..5, 1, false, 0..6),
            ("B.028", 0..5, A, 5..5, 1, true, 0..6),
            ("B.029", 0..5, N, 5..5, 1, true, 0..6),
            ("B.030", 0..5, B, 5..5, 1, true, 0..6),
            ("B.031", 0..5, F, 5..5, 1, true, 0..6),
            ("B.032", 5..10, A, 4..5, 2, false, 5..11),
            ("B.033", 5..10, N, 4..5, 2, false, 6..11),
            ("B.034", 5..10, B, 4..5, 2, false, 5..11),
            ("B.035", 5..10, F, 4..5, 2, false, 6..11),
            ("B.036", 5..10, A, 4..5, 2, true, 6..11),
            ("B.037", 5..10, N, 4..5, 2, true, 6..11),
            ("B.038", 5..10, B, 4..5, 2, true, 6..11),
            ("B.039", 5..10, F, 4..5, 2, true, 6..11),
            ("B.040", 5..10, A, 3..5, 1, false, 4..9),
            ("B.041", 5..10, N, 3..5, 1, false, 4..9),
            ("B.042", 5..10, B, 3..5, 1, false, 4..9),
            ("B.043", 5..10, F, 3..5, 1, false, 4..9),
            ("B.044", 5..10, A, 3..5, 1, true, 4..9),
            ("B.045", 5..10, N, 3..5, 1, true, 4..9),
            ("B.046", 5..10, B, 3..5, 1, true, 4..9),
            ("B.047", 5..10, F, 3..5, 1, true, 4..9),
            ("B.048", 5..10, A, 4..6, 3, false, 5..11),
            ("B.049", 5..10, N, 4..6, 3, false, 5..11),
            ("B.050", 5..10, B, 4..6, 3, false, 5..11),
            ("B.051", 5..10, F, 4..6, 3, false, 5..11),
            ("B.052", 5..10, A, 4..6, 3, true, 7..11),
            ("B.053", 5..10, N, 4..6, 3, true, 7..11),
            ("B.054", 5..10, B, 4..6, 3, true, 7..11),
            ("B.055", 5..10, F, 4..6, 3, true, 7..11),
            ("B.056", 5..10, A, 4..6, 1, false, 5..9),
            ("B.057", 5..10, N, 4..6, 1, false, 5..9),
            ("B.058", 5..10, B, 4..6, 1, false, 5..9),
            ("B.059", 5..10, F, 4..6, 1, false, 5..9),
            ("B.060", 5..10, A, 4..6, 1, true, 5..9),
            ("B.061", 5..10, N, 4..6, 1, true, 5..9),
            ("B.062", 5..10, B, 4..6, 1, true, 5..9),
            ("B.063", 5..10, F, 4..6, 1, true, 5..9),
            ("B.064", 5..10, A, 5..6, 2, false, 5..11),
            ("B.065", 5..10, N, 5..6, 2, false, 5..11),
            ("B.066", 5..10, B, 5..6, 2, false, 5..11),
            ("B.067", 5..10, F, 5..6, 2, false, 5..11),
            ("B.068", 5..10, A, 5..6, 2, true, 7..11),
            ("B.069", 5..10, N, 5..6, 2, true, 7..11),
            ("B.070", 5..10, B, 5..6, 2, true, 7..11),
            ("B.071", 5..10, F, 5..6, 2, true, 7..11),
            ("B.072", 5..10, A, 5..7, 1, false, 5..9),
            ("B.073", 5..10, N, 5..7, 1, false, 5..9),
            ("B.074", 5..10, B, 5..7, 1, false, 5..9),
            ("B.075", 5..10, F, 5..7, 1, false, 5..9),
            ("B.076", 5..10, A, 5..7, 1, true, 6..9),
            ("B.077", 5..10, N, 5..7, 1, true, 6..9),
            ("B.078", 5..10, B, 5..7, 1, true, 6..9),
            ("B.079", 5..10, F, 5..7, 1, true, 6..9),
            ("B.080", 5..10, A, 9..10, 2, false, 5..11),
            ("B.081", 5..10, N, 9..10, 2, false, 5..10),
            ("B.082", 5..10, B, 9..10, 2, false, 5..10),
            ("B.083", 5..10, F, 9..10, 2, false, 5..11),
            ("B.084", 5..10, A, 9..10, 2, true, 5..11),
            ("B.085", 5..10, N, 9..10, 2, true, 5..11),
            ("B.086", 5..10, B, 9..10, 2, true, 5..11),
            ("B.087", 5..10, F, 9..10, 2, true, 5..11),
            ("B.088", 5..10, A, 8..10, 1, false, 5..9),
            ("B.089", 5..10, N, 8..10, 1, false, 5..9),
            ("B.090", 5..10, B, 8..10, 1, false, 5..9),
            ("B.091", 5..10, F, 8..10, 1, false, 5..9),
            ("B.092", 5..10, A, 8..10, 1, true, 5..9),
            ("B.093", 5..10, N, 8..10, 1, true, 5..9),
            ("B.094", 5..10, B, 8..10, 1, true, 5..9),
            ("B.095", 5..10, F, 8..10, 1, true, 5..9),
            ("B.096", 5..10, A, 9..11, 3, false, 5..10),
            ("B.097", 5..10, N, 9..11, 3, false, 5..10),
            ("B.098", 5..10, B, 9..11, 3, false, 5..10),
            ("B.099", 5..10, F, 9..11, 3, false, 5..10),
            ("B.100", 5..10, A, 9..11, 3, true, 5..12),
            ("B.101", 5..10, N, 9..11, 3, true, 5..12),
            ("B.102", 5..10, B, 9..11, 3, true, 5..12),
            ("B.103", 5..10, F, 9..11, 3, true, 5..12),
            ("B.104", 5..10, A, 9..11, 1, false, 5..10),
            ("B.105", 5..10, N, 9..11, 1, false, 5..10),
            ("B.106", 5..10, B, 9..11, 1, false, 5..10),
            ("B.107", 5..10, F, 9..11, 1, false, 5..10),
            ("B.108", 5..10, A, 9..11, 1, true, 5..10),
            ("B.109", 5..10, N, 9..11, 1, true, 5..10),
            ("B.110", 5..10, B, 9..11, 1, true, 5..10),
            ("B.111", 5..10, F, 9..11, 1, true, 5..10),
            ("B.112", 5..10, A, 10..11, 3, false, 5..10),
            ("B.113", 5..10, N, 10..11, 3, false, 5..10),
            ("B.114", 5..10, B, 10..11, 3, false, 5..10),
            ("B.115", 5..10, F, 10..11, 3, false, 5..10),
            ("B.116", 5..10, A, 10..11, 3, true, 5..13),
            ("B.117", 5..10, N, 10..11, 3, true, 5..13),
            ("B.118", 5..10, B, 10..11, 3, true, 5..13),
            ("B.119", 5..10, F, 10..11, 3, true, 5..13),
            ("B.120", 5..10, A, 10..12, 1, false, 5..10),
            ("B.121", 5..10, N, 10..12, 1, false, 5..10),
            ("B.122", 5..10, B, 10..12, 1, false, 5..10),
            ("B.123", 5..10, F, 10..12, 1, false, 5..10),
            ("B.124", 5..10, A, 10..12, 1, true, 5..11),
            ("B.125", 5..10, N, 10..12, 1, true, 5..11),
            ("B.126", 5..10, B, 10..12, 1, true, 5..11),
            ("B.127", 5..10, F, 10..12, 1, true, 5..11),
            ("B.128", 5..10, A, 4..5, 0, false, 4..9),
            ("B.129", 5..10, N, 4..5, 0, false, 4..9),
            ("B.130", 5..10, B, 4..5, 0, false, 4..9),
            ("B.131", 5..10, F, 4..5, 0, false, 4..9),
            ("B.132", 5..10, A, 4..5, 0, true, 4..9),
            ("B.133", 5..10, N, 4..5, 0, true, 4..9),
            ("B.134", 5..10, B, 4..5, 0, true, 4..9),
            ("B.135", 5..10, F, 4..5, 0, true, 4..9),
            ("B.136", 5..10, A, 4..6, 0, false, 4..8),
            ("B.137", 5..10, N, 4..6, 0, false, 4..8),
            ("B.138", 5..10, B, 4..6, 0, false, 4..8),
            ("B.139", 5..10, F, 4..6, 0, false, 4..8),
            ("B.140", 5..10, A, 4..6, 0, true, 4..8),
            ("B.141", 5..10, N, 4..6, 0, true, 4..8),
            ("B.142", 5..10, B, 4..6, 0, true, 4..8),
            ("B.143", 5..10, F, 4..6, 0, true, 4..8),
            ("B.144", 5..10, A, 5..6, 0, false, 5..9),
            ("B.145", 5..10, N, 5..6, 0, false, 5..9),
            ("B.146", 5..10, B, 5..6, 0, false, 5..9),
            ("B.147", 5..10, F, 5..6, 0, false, 5..9),
            ("B.148", 5..10, A, 5..6, 0, true, 5..9),
            ("B.149", 5..10, N, 5..6, 0, true, 5..9),
            ("B.150", 5..10, B, 5..6, 0, true, 5..9),
            ("B.151", 5..10, F, 5..6, 0, true, 5..9),
            ("B.152", 5..10, A, 9..10, 0, false, 5..9),
            ("B.153", 5..10, N, 9..10, 0, false, 5..9),
            ("B.154", 5..10, B, 9..10, 0, false, 5..9),
            ("B.155", 5..10, F, 9..10, 0, false, 5..9),
            ("B.156", 5..10, A, 9..10, 0, true, 5..9),
            ("B.157", 5..10, N, 9..10, 0, true, 5..9),
            ("B.158", 5..10, B, 9..10, 0, true, 5..9),
            ("B.159", 5..10, F, 9..10, 0, true, 5..9),
            ("B.160", 5..10, A, 9..11, 0, false, 5..9),
            ("B.161", 5..10, N, 9..11, 0, false, 5..9),
            ("B.162", 5..10, B, 9..11, 0, false, 5..9),
            ("B.163", 5..10, F, 9..11, 0, false, 5..9),
            ("B.164", 5..10, A, 9..11, 0, true, 5..9),
            ("B.165", 5..10, N, 9..11, 0, true, 5..9),
            ("B.166", 5..10, B, 9..11, 0, true, 5..9),
            ("B.167", 5..10, F, 9..11, 0, true, 5..9),
            ("B.168", 5..10, A, 10..11, 0, false, 5..10),
            ("B.169", 5..10, N, 10..11, 0, false, 5..10),
            ("B.170", 5..10, B, 10..11, 0, false, 5..10),
            ("B.171", 5..10, F, 10..11, 0, false, 5..10),
            ("B.172", 5..10, A, 10..11, 0, true, 5..10),
            ("B.173", 5..10, N, 10..11, 0, true, 5..10),
            ("B.174", 5..10, B, 10..11, 0, true, 5..10),
            ("B.175", 5..10, F, 10..11, 0, true, 5..10),
            ("B.176", 5..10, A, 5..10, 3, false, 5..8),
            ("B.177", 5..10, N, 5..10, 3, false, 5..8),
            ("B.178", 5..10, B, 5..10, 3, false, 5..8),
            ("B.179", 5..10, F, 5..10, 3, false, 5..8),
            ("B.180", 5..10, A, 5..10, 3, true, 8..8),
            ("B.181", 5..10, N, 5..10, 3, true, 8..8),
            ("B.182", 5..10, B, 5..10, 3, true, 8..8),
            ("B.183", 5..10, F, 5..10, 3, true, 8..8),
            ("B.184", 5..10, A, 5..10, 7, false, 5..12),
            ("B.185", 5..10, N, 5..10, 7, false, 5..10),
            ("B.186", 5..10, B, 5..10, 7, false, 5..10),
            ("B.187", 5..10, F, 5..10, 7, false, 5..12),
            ("B.188", 5..10, A, 5..10, 7, true, 12..12),
            ("B.189", 5..10, N, 5..10, 7, true, 12..12),
            ("B.190", 5..10, B, 5..10, 7, true, 12..12),
            ("B.191", 5..10, F, 5..10, 7, true, 12..12),
        ];
        for (name, range, stickiness, replaced, inserted, force, expected) in rows.iter().cloned() {
            let mut buffer = Buffer::new(&"x".repeat(20));
            let mut set = DecorationSet::default();
            let ids = set.delta(None, &[], [(range, with_stickiness(stickiness))]);
            let text = "y".repeat(inserted);
            let force = if force {
                vec![replaced.clone()]
            } else {
                Vec::new()
            };
            edit(&mut buffer, &mut set, &[(replaced, &text)], &force);
            assert_eq!(set.get(ids[0]).unwrap().range, expected, "{name}");
        }
    }

    /// Edges inside a replace stay as far into the new text as they were
    /// into the old, up to the shorter of the two (audit 4 A.1): an
    /// equal-length replace (Transform to Uppercase) keeps a decoration on
    /// its word, whatever its stickiness.
    #[test]
    fn edges_inside_a_replace_keep_their_place() {
        for stickiness in [
            Stickiness::AlwaysGrowsWhenTypingAtEdges,
            Stickiness::NeverGrowsWhenTypingAtEdges,
            Stickiness::GrowsOnlyWhenTypingBefore,
            Stickiness::GrowsOnlyWhenTypingAfter,
        ] {
            let mut buffer = Buffer::new("foo bar baz");
            let mut set = DecorationSet::default();
            let ids = set.delta(None, &[], [(4..7, with_stickiness(stickiness))]);
            edit(&mut buffer, &mut set, &[(0..11, "FOO BAR BAZ")], &[]);
            assert_eq!(set.get(ids[0]).unwrap().range, 4..7, "{stickiness:?}");
        }
        // Shorter text: edges past its end move to it.
        let mut buffer = Buffer::new("foo bar baz");
        let mut set = DecorationSet::default();
        let ids = set.delta(None, &[], [(2..9, DecorationOptions::default())]);
        edit(&mut buffer, &mut set, &[(0..11, "abcde")], &[]);
        assert_eq!(set.get(ids[0]).unwrap().range, 2..5);
    }

    /// Monaco measures the common length in UTF-16 code units, and the
    /// port keeps byte offsets: an edge keeps its place in characters and
    /// never lands inside one.
    #[test]
    fn edges_inside_a_replace_count_utf16_units() {
        // `\u{e9}` is 2 bytes and 1 unit: the edge 1 unit in stays after
        // the first accented letter.
        let mut buffer = Buffer::new("abcd");
        let mut set = DecorationSet::default();
        let ids = set.delta(None, &[], [(1..3, DecorationOptions::default())]);
        edit(
            &mut buffer,
            &mut set,
            &[(0..4, "\u{e9}\u{e8}\u{ea}\u{eb}")],
            &[],
        );
        assert_eq!(set.get(ids[0]).unwrap().range, 2..6);
        // An emoji is 4 bytes and 2 units: 1 unit in would split it, so the
        // edge moves to its end.
        let mut buffer = Buffer::new("abcd");
        let mut set = DecorationSet::default();
        let ids = set.delta(None, &[], [(1..3, DecorationOptions::default())]);
        edit(&mut buffer, &mut set, &[(0..4, "\u{1f600}\u{1f600}")], &[]);
        assert_eq!(set.get(ids[0]).unwrap().range, 4..8);
        // From UTF-16 units back to bytes: 2 units into the old text is
        // past the emoji.
        let mut buffer = Buffer::new("\u{1f600}xy");
        let mut set = DecorationSet::default();
        let ids = set.delta(None, &[], [(4..5, DecorationOptions::default())]);
        edit(&mut buffer, &mut set, &[(0..6, "abcd")], &[]);
        assert_eq!(set.get(ids[0]).unwrap().range, 2..3);
    }

    /// Monaco `issue #41492: URL highlighting persists after pasting over
    /// url` (`modelDecorations.test.ts`): `collapseOnReplaceEdit`.
    #[test]
    fn collapse_on_replace_edit() {
        let mut buffer = Buffer::new("My First Line");
        let mut set = DecorationSet::default();
        let collapsing = |stickiness| DecorationOptions {
            stickiness,
            collapse_on_replace_edit: true,
            ..Default::default()
        };
        let ids = set.delta(
            None,
            &[],
            [(1..13, collapsing(Stickiness::NeverGrowsWhenTypingAtEdges))],
        );
        let text = "Some new text that is longer than the previous one";
        edit(&mut buffer, &mut set, &[(0..13, text)], &[]);
        assert_eq!(set.get(ids[0]).unwrap().range, 0..0);
        // An edit inside it moves its edges as usual.
        let mut buffer = Buffer::new("My First Line");
        let mut set = DecorationSet::default();
        let ids = set.delta(
            None,
            &[],
            [(3..8, collapsing(Stickiness::AlwaysGrowsWhenTypingAtEdges))],
        );
        edit(&mut buffer, &mut set, &[(4..6, "IR")], &[]);
        assert_eq!(set.get(ids[0]).unwrap().range, 3..8);
    }

    /// Ids are unique across models (Monaco's are global), so one from
    /// another model never names an unrelated decoration.
    #[test]
    fn ids_are_unique_across_sets() {
        let mut a = DecorationSet::default();
        let mut b = DecorationSet::default();
        let first = a.delta(None, &[], [(0..1, DecorationOptions::default())]);
        let second = b.delta(None, &[], [(0..1, DecorationOptions::default())]);
        assert_ne!(first, second);
        assert!(b.get(first[0]).is_none());
        assert!(a.clear().is_some());
        assert!(a.clear().is_none());
    }

    #[test]
    fn owner_filtering() {
        let mut set = DecorationSet::default();
        let editor = EntityId::from(7u64);
        set.delta(Some(editor), &[], [(0..1, DecorationOptions::default())]);
        set.delta(None, &[], [(0..1, DecorationOptions::default())]);
        assert_eq!(set.in_range(0..1, Some(editor)).len(), 2);
        assert_eq!(set.in_range(0..1, None).len(), 1);
    }

    /// Looking decorations up by id among many (an index past
    /// `INDEX_THRESHOLD`) finds each after it's added, after edits, sorting,
    /// removals and option changes, and nothing for removed ids.
    #[test]
    fn lookups_by_id_among_many_decorations() {
        let count = INDEX_THRESHOLD * 4;
        let text = "x".repeat(count * 2);
        let mut buffer = Buffer::new(&text);
        let mut set = DecorationSet::default();
        let editor = EntityId::from(7u64);
        // Added out of order, so sorting reorders them.
        let mut ids = set.delta(
            None,
            &[],
            (0..count).rev().map(|i| {
                (
                    i * 2..i * 2 + 1,
                    with_stickiness(Stickiness::NeverGrowsWhenTypingAtEdges),
                )
            }),
        );
        ids.reverse();
        let check = |set: &DecorationSet, ids: &[DecorationId], shift: usize| {
            for (i, id) in ids.iter().enumerate() {
                let start = i * 2 + shift;
                assert_eq!(
                    set.get(*id).map(|d| d.range.clone()),
                    Some(start..start + 1)
                );
            }
        };
        check(&set, &ids, 0);
        // Sorting reorders the decorations.
        assert_eq!(set.in_range(0..1, None).len(), 1);
        check(&set, &ids, 0);
        // An edit before them all shifts them without reordering.
        edit(&mut buffer, &mut set, &[(0..0, "ab")], &[]);
        assert_eq!(set.in_range(2..3, None).len(), 1);
        check(&set, &ids, 2);
        // Removing many and adding one.
        let removed: Vec<DecorationId> = ids.drain(..count / 2).collect();
        let owned = set.delta(
            Some(editor),
            &removed,
            [(0..1, DecorationOptions::default())],
        );
        assert!(removed.iter().all(|id| set.get(*id).is_none()));
        assert_eq!(set.get(owned[0]).map(|d| d.range.clone()), Some(0..1));
        for (i, id) in ids.iter().enumerate() {
            let start = (i + count / 2) * 2 + 2;
            assert_eq!(
                set.get(*id).map(|d| d.range.clone()),
                Some(start..start + 1)
            );
        }
        // Changing options keeps the place.
        let options = DecorationOptions {
            z_index: 3,
            ..with_stickiness(Stickiness::NeverGrowsWhenTypingAtEdges)
        };
        assert!(set.change_options(ids[0], options));
        assert_eq!(set.get(ids[0]).unwrap().options.z_index, 3);
        // Removing an editor's decorations.
        assert!(set.remove_owned_by(editor).is_some());
        assert!(set.get(owned[0]).is_none());
        assert!(set.get(ids[1]).is_some());
        assert!(set.clear().is_some());
        assert!(ids.iter().all(|id| set.get(*id).is_none()));
    }
}
