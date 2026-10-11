//! Model decorations (Monaco `IModelDeltaDecoration`): styled ranges owned by
//! the [`TextModel`](crate::TextModel) that follow edits.
//!
//! Monaco styles decorations with CSS class names; here the options carry the
//! styles directly.

use std::ops::Range;
use std::sync::Arc;

use gpui::{EntityId, FontStyle, FontWeight, Hsla, Pixels, SharedString};
use text::{Patch, Stickiness};

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
    /// 1px border around the range on each line.
    pub border: Option<Hsla>,
    /// An underline under the range (e.g. diagnostic squiggles).
    pub underline: Option<DecorationUnderline>,
    /// Text color and font style for the range (Monaco `inlineClassName`).
    pub text: Option<TokenStyle>,
    /// Paint `background`/`border` across the whole line(s).
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
    /// Markdown shown when the mouse rests on the start row's glyph margin
    /// or line number (Monaco `glyphMarginHoverMessage`).
    pub glyph_margin_hover_message: Option<SharedString>,
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
    /// Shown when hovering the range.
    pub hover_message: Option<SharedString>,
    /// Draws the decoration when its range is empty too (Monaco
    /// `showIfCollapsed`): on an empty range, its [`before`](Self::before)
    /// and [`after`](Self::after) text only shows with this, and with it an
    /// empty or narrow [`background`](Self::background) is widened to a
    /// typical character's width (Monaco `typicalHalfwidthCharacterWidth`),
    /// centered, and a range ending at the start of a line stops at the
    /// end of the line before.
    pub show_if_collapsed: bool,
    /// How the range grows when text is typed at its edges (Monaco `stickiness`).
    pub stickiness: Stickiness,
    /// Higher paints on top.
    pub z_index: i32,
    /// A diagnostic's decoration (Monaco `isForValidation`), hidden as the
    /// editor's `render_validation_decorations` says.
    pub validation: bool,
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
}

impl DecorationOptions {
    /// Whether its injected text shows on `range`: always on a non-empty
    /// range, and on an empty one with
    /// [`show_if_collapsed`](Self::show_if_collapsed) (Monaco
    /// `getInjectedTextInInterval`).
    pub(crate) fn shows_injected_text(&self, range: &Range<usize>) -> bool {
        self.show_if_collapsed || !range.is_empty()
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
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BlockDecoration {
    /// The border's color.
    pub border: Hsla,
    /// The border's width.
    pub border_width: Pixels,
    /// Monaco `blockPadding`: top, right, bottom and left, outside the
    /// lines.
    pub padding: [Pixels; 4],
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
    /// Whether a glyph picks its glyph margin lane, computed when asked
    /// after a change.
    has_glyph_lanes: std::cell::Cell<Option<bool>>,
}

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

    /// Removes `old` and adds `new`, returning the new ids in order.
    pub fn delta(
        &mut self,
        owner: Option<EntityId>,
        old: &[DecorationId],
        new: impl IntoIterator<Item = (Range<usize>, DecorationOptions)>,
    ) -> Vec<DecorationId> {
        if !old.is_empty() {
            self.decorations.retain(|d| !old.contains(&d.id));
        }
        let mut ids = Vec::new();
        for (range, options) in new {
            let id =
                DecorationId(NEXT_DECORATION_ID.fetch_add(1, std::sync::atomic::Ordering::Relaxed));
            let range = range.start.min(range.end)..range.end.max(range.start);
            self.max_len = self.max_len.max(range.len());
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
        ids
    }

    /// Replaces the options of decoration `id`, keeping its range and id
    /// (Monaco `changeDecorationOptions`). Returns whether it exists.
    pub fn change_options(&mut self, id: DecorationId, options: DecorationOptions) -> bool {
        match self.decorations.iter_mut().find(|d| d.id == id) {
            Some(decoration) => {
                decoration.options = Arc::new(options);
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
        self.max_len = 0;
        self.version += 1;
        self.has_glyph_lanes.set(None);
        Some(change)
    }

    /// Removes the decorations `owner` owns (Monaco
    /// `removeAllDecorationsWithOwnerId`), returning what they affected, or
    /// `None` when there were none.
    pub fn remove_owned_by(&mut self, owner: EntityId) -> Option<crate::events::DecorationsChange> {
        let mut change = None;
        self.has_glyph_lanes.set(None);
        self.decorations.retain(|d| {
            if d.owner == Some(owner) {
                change = Some(
                    change
                        .unwrap_or(crate::events::DecorationsChange::default())
                        .with(&d.options),
                );
                false
            } else {
                true
            }
        });
        if change.is_some() {
            self.version += 1;
        }
        change
    }

    /// Every decoration, in no particular order.
    pub fn iter(&self) -> impl Iterator<Item = &Decoration> {
        self.decorations.iter()
    }

    /// The decoration with `id`, if it still exists.
    pub fn get(&self, id: DecorationId) -> Option<&Decoration> {
        self.decorations.iter().find(|d| d.id == id)
    }

    fn ensure_sorted(&mut self) {
        if !self.sorted {
            self.decorations
                .sort_by_key(|d| (d.range.start, d.range.end, d.id));
            self.sorted = true;
        }
    }

    /// Moves every decoration through `patch`, honoring stickiness.
    pub fn apply_patch(&mut self, patch: &Patch) {
        if patch.is_empty() {
            return;
        }
        let mut max_len = 0;
        for decoration in &mut self.decorations {
            let (start_bias, end_bias) = decoration.options.stickiness.biases();
            let start = patch.transform_offset(decoration.range.start, start_bias);
            let end = patch
                .transform_offset(decoration.range.end, end_bias)
                .max(start);
            decoration.range = start..end;
            max_len = max_len.max(end - start);
        }
        self.max_len = max_len;
        self.sorted = false;
        self.version += 1;
    }

    /// [`DecorationSet::apply_patch`], except that decoration edges inside
    /// or at the edges of one of the `force` ranges (old byte ranges of
    /// the patch's edits) move past that edit's new text (Monaco
    /// `forceMoveMarkers`).
    pub fn apply_patch_forcing(&mut self, patch: &Patch, force: &[Range<usize>]) {
        if force.is_empty() {
            return self.apply_patch(patch);
        }
        let forced = |offset: usize| {
            force
                .iter()
                .find(|range| range.start <= offset && offset <= range.end)
                .map(|range| patch.transform_offset(range.end, text::Bias::Right))
        };
        let old: Vec<Range<usize>> = self.decorations.iter().map(|d| d.range.clone()).collect();
        self.apply_patch(patch);
        let mut max_len = 0;
        for (decoration, old) in self.decorations.iter_mut().zip(old) {
            let start = forced(old.start).unwrap_or(decoration.range.start);
            let end = forced(old.end).unwrap_or(decoration.range.end).max(start);
            decoration.range = start..end;
            max_len = max_len.max(end - start);
        }
        self.max_len = max_len;
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

/// Stickiness re-exported for decoration options.
pub use text::Stickiness as TrackedRangeStickiness;

#[cfg(test)]
mod tests {
    use super::*;
    use text::Buffer;

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
        let patch = buffer.edit([(6..6, ">"), (11..11, "<")]).unwrap();
        set.apply_patch(&patch);
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
        let patch = buffer.edit([(5..6, "__")]).unwrap();
        set.apply_patch_forcing(&patch, std::slice::from_ref(&(5..6)));
        assert_eq!(set.get(ids[0]).unwrap().range, 0..7);
        assert_eq!(set.get(ids[1]).unwrap().range, 7..12);
        // Not forced: they keep to their text.
        let patch = buffer.edit([(7..7, "+")]).unwrap();
        set.apply_patch_forcing(&patch, &[]);
        assert_eq!(set.get(ids[1]).unwrap().range, 8..13);
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
}
