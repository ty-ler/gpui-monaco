//! Model decorations (Monaco `IModelDeltaDecoration`): styled ranges owned by
//! the [`TextModel`](crate::TextModel) that follow edits.
//!
//! Monaco styles decorations with CSS class names; here the options carry the
//! styles directly.

use std::ops::Range;
use std::sync::Arc;

use gpui::{EntityId, Hsla, SharedString};
use text::{Patch, Stickiness};

use crate::theme::TokenStyle;

/// Identifies a decoration returned by `TextModel::delta_decorations`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct DecorationId(pub u64);

/// Underline drawn under the decorated text (Monaco uses CSS for squiggles).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DecorationUnderline {
    /// Underline color.
    pub color: Hsla,
    /// Draw a squiggle instead of a straight line.
    pub wavy: bool,
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
}

/// Monaco `IModelDecorationOptions`, with styles instead of class names.
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
    /// A glyph shown in the glyph margin on the start row (Monaco
    /// `glyphMarginClassName`).
    pub glyph: Option<(SharedString, Hsla)>,
    /// Markdown shown when the mouse rests on the start row's glyph margin
    /// or line number (Monaco `glyphMarginHoverMessage`).
    pub glyph_margin_hover_message: Option<SharedString>,
    /// Line number color on the decorated rows (Monaco
    /// `lineNumberClassName`).
    pub line_number_color: Option<Hsla>,
    /// A colored bar in the line decorations gutter area (Monaco
    /// `linesDecorationsClassName`, e.g. dirty-diff markers).
    pub gutter_bar: Option<Hsla>,
    /// Text centered in the line decorations area on every decorated line
    /// (Monaco `linesDecorationsClassName` with an icon, e.g. the diff
    /// editor's `+` and `-`).
    pub lines_decoration_text: Option<(SharedString, Hsla)>,
    /// A background across the whole gutter on the decorated lines (Monaco
    /// `marginClassName`).
    pub margin_background: Option<Hsla>,
    /// A mark in the overview ruler (the vertical scrollbar track) at the start row
    /// (Monaco `overviewRuler`).
    pub overview_ruler: Option<(Hsla, OverviewRulerLane)>,
    /// Text shown before the range start (Monaco `before`).
    pub before: Option<InjectedText>,
    /// Text shown after the range end (Monaco `after`).
    pub after: Option<InjectedText>,
    /// Shown when hovering the range.
    pub hover_message: Option<SharedString>,
    /// How the range grows when text is typed at its edges (Monaco `stickiness`).
    pub stickiness: Stickiness,
    /// Higher paints on top.
    pub z_index: i32,
    /// A diagnostic's decoration (Monaco `isForValidation`), hidden as the
    /// editor's `render_validation_decorations` says.
    pub validation: bool,
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
}

/// Monaco's block decoration: a box spanning the content width around
/// whole lines.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BlockDecoration {
    /// The border's color.
    pub border: Hsla,
    /// The border's width.
    pub border_width: gpui::Pixels,
    /// Monaco `blockPadding`: top, right, bottom and left, outside the
    /// lines.
    pub padding: [gpui::Pixels; 4],
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
    next_id: u64,
    /// Length of the longest decoration, to bound range queries.
    max_len: usize,
    sorted: bool,
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
            self.next_id += 1;
            let id = DecorationId(self.next_id);
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
        ids
    }

    /// Replaces the options of decoration `id`, keeping its range and id
    /// (Monaco `changeDecorationOptions`). Returns whether it exists.
    pub fn change_options(&mut self, id: DecorationId, options: DecorationOptions) -> bool {
        match self.decorations.iter_mut().find(|d| d.id == id) {
            Some(decoration) => {
                decoration.options = Arc::new(options);
                true
            }
            None => false,
        }
    }

    /// Removes every decoration.
    pub fn clear(&mut self) {
        self.decorations.clear();
        self.max_len = 0;
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
