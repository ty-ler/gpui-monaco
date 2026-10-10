//! Sticky scroll's model (Monaco `StickyModelProvider`): the scopes whose
//! header lines stick to the top. Per `stickyScroll.defaultModel` it comes
//! from the document symbols (outline), the folding ranges, or indentation,
//! each falling back to the next when it has nothing.
//!
//! Ports `stickyScrollModelProvider.ts` (MIT).

use std::time::Duration;

use gpui::{Context, Task};

use text::PointUtf16;

use crate::cursor::Selection;
use crate::editor::CodeEditor;
use crate::options::StickyScrollModel;
use crate::providers::{DocumentSymbol, document_symbol_providers};
use crate::view::folding::{FoldRange, compute_indent_ranges};
use crate::view::reveal::RevealType;

/// Monaco's `Delayer` before recomputing after an edit.
const UPDATE_DELAY: Duration = Duration::from_millis(300);

/// A scope (Monaco `StickyElement`): its header row and the last row it
/// keeps the header for, then the scopes inside it, sorted by start.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct StickyElement {
    pub start: u32,
    pub end: u32,
    pub children: Vec<StickyElement>,
}

/// What the model was computed for: the text, the parsed text (for the
/// tags query), the model kind, the language and the symbol providers.
type StickyKey = (
    text::Version,
    Option<text::Version>,
    StickyScrollModel,
    Option<String>,
    usize,
);

#[derive(Default)]
pub(crate) struct StickyModelState {
    key: Option<StickyKey>,
    pending: Option<StickyKey>,
    task: Option<Task<()>>,
    /// The scopes, or `None` to use the folding ranges.
    elements: Option<Vec<StickyElement>>,
    /// The keyboard-focused sticky line (Monaco `focusStickyScroll`).
    pub focused: Option<usize>,
    /// The sticky line under the mouse.
    pub hovered: Option<usize>,
    /// The line showing its scope's end while Shift is held over it.
    pub show_end: Option<usize>,
}

/// Monaco's comparator: by start, longer first.
fn sort_elements(elements: &mut [StickyElement]) {
    elements.sort_by_key(|e| (e.start, std::cmp::Reverse(e.end)));
}

/// Monaco `_stickyModelFromOutlineElement`: a symbol's scope runs from its
/// name's line to the line before its end. Single-line children are
/// dropped, and children sharing the parent's header line give way to
/// their own children (`previous_start` is the header line above).
fn from_symbol(symbol: &DocumentSymbol, previous_start: u32) -> StickyElement {
    let mut children = Vec::new();
    for child in &symbol.children {
        let child_start = child.selection_range.start.row;
        if child_start == child.range.end.row {
            continue;
        }
        if child_start != previous_start {
            children.push(from_symbol(child, child_start));
        } else {
            children.extend(
                child
                    .children
                    .iter()
                    .map(|sub| from_symbol(sub, child_start)),
            );
        }
    }
    sort_elements(&mut children);
    StickyElement {
        start: symbol.selection_range.start.row,
        // Monaco's candidates end a line before the range does.
        end: symbol.range.end.row.saturating_sub(1),
        children,
    }
}

/// Monaco `_stickyModelFromOutlineModel` (for one provider's symbols).
pub(crate) fn from_symbols(symbols: &[DocumentSymbol]) -> Vec<StickyElement> {
    let mut sorted: Vec<&DocumentSymbol> = symbols.iter().collect();
    sorted.sort_by_key(|s| (s.range.start.row, std::cmp::Reverse(s.range.end.row)));
    sorted
        .into_iter()
        .map(|symbol| from_symbol(symbol, symbol.selection_range.start.row))
        .collect()
}

/// Folding ranges (nested by `parent`) as scopes; a range's end is already
/// the last row it covers.
pub(crate) fn from_fold_ranges(ranges: &[FoldRange]) -> Vec<StickyElement> {
    fn build(ranges: &[FoldRange], children: &[Vec<usize>], ix: usize) -> StickyElement {
        StickyElement {
            start: ranges[ix].start_row,
            end: ranges[ix].end_row,
            children: children[ix]
                .iter()
                .map(|&child| build(ranges, children, child))
                .collect(),
        }
    }
    let mut children = vec![Vec::new(); ranges.len()];
    let mut roots = Vec::new();
    for (ix, range) in ranges.iter().enumerate() {
        match range.parent {
            Some(parent) => children[parent].push(ix),
            None => roots.push(ix),
        }
    }
    roots
        .into_iter()
        .map(|ix| build(ranges, &children, ix))
        .collect()
}

/// The scopes starting above `row` that still cover it, outermost first.
pub(crate) fn enclosing(elements: &[StickyElement], row: u32) -> Vec<(u32, u32)> {
    let mut chain = Vec::new();
    let mut level = elements;
    'levels: loop {
        // Scopes are sorted by start; the enclosing one starts last before
        // `row` among those that reach it.
        let end = level.partition_point(|e| e.start < row);
        for element in level[..end].iter().rev() {
            if element.end >= row {
                if chain
                    .last()
                    .is_none_or(|(start, _)| *start != element.start)
                {
                    chain.push((element.start, element.end));
                }
                level = &element.children;
                continue 'levels;
            }
        }
        return chain;
    }
}

impl CodeEditor {
    /// The sticky headers' scopes enclosing `row`, outermost first.
    pub(crate) fn sticky_ranges_enclosing(&self, row: u32) -> Vec<(u32, u32)> {
        match &self.sticky.elements {
            Some(elements) => enclosing(elements, row),
            None => self
                .folds
                .ranges_enclosing(row)
                .into_iter()
                .map(|range| (range.start_row, range.end_row))
                .collect(),
        }
    }

    /// Recomputes the model when the text or the providers changed: right
    /// away the first time, after Monaco's delay otherwise. Called before
    /// painting.
    pub(crate) fn refresh_sticky_model(&mut self, cx: &mut Context<Self>) {
        let options = &self.options.sticky_scroll;
        if !options.enabled || self.options.single_line {
            if self.sticky.key.is_some() || self.sticky.focused.is_some() {
                self.sticky = StickyModelState::default();
            }
            return;
        }
        let model = self.model().read(cx);
        let language = model.language_id().map(|l| l.to_string());
        let providers = document_symbol_providers(cx, language.as_deref()).len();
        let parsed = model.syntax_snapshot().map(|syntax| syntax.text.version());
        let key: StickyKey = (
            model.buffer().version(),
            parsed,
            options.default_model,
            language,
            providers,
        );
        if self.sticky.key.as_ref() == Some(&key) || self.sticky.pending.as_ref() == Some(&key) {
            return;
        }
        // Only the first request is immediate, so typing doesn't recompute
        // on every keystroke.
        let delay = if self.sticky.key.is_some() || self.sticky.pending.is_some() {
            UPDATE_DELAY
        } else {
            Duration::ZERO
        };
        self.sticky.pending = Some(key.clone());
        let default_model = key.2;
        self.sticky.task = Some(cx.spawn(async move |this, cx| {
            if !delay.is_zero() {
                cx.background_executor().timer(delay).await;
            }
            let mut elements = None;
            if default_model == StickyScrollModel::Outline {
                let Ok(symbols) = this.update(cx, |this, cx| this.document_symbols(cx)) else {
                    return;
                };
                let symbols = symbols.await;
                // Monaco: an empty outline isn't a model.
                if !symbols.is_empty() {
                    elements = Some(from_symbols(&symbols));
                }
            }
            if elements.is_none() && default_model == StickyScrollModel::Indentation {
                let Ok((rope, tab_size)) = this.update(cx, |this, cx| {
                    let model = this.model().read(cx);
                    (model.buffer().rope().clone(), model.options().tab_size)
                }) else {
                    return;
                };
                let ranges = cx
                    .background_executor()
                    .spawn(async move { compute_indent_ranges(&rope, tab_size, false, None) })
                    .await;
                elements = Some(from_fold_ranges(&ranges));
            }
            this.update(cx, |this, cx| {
                this.sticky.pending = None;
                this.sticky.task = None;
                if this.model().read(cx).buffer().version() != key.0 {
                    // Edited meanwhile: the next paint asks again.
                    cx.notify();
                    return;
                }
                this.sticky.key = Some(key);
                if this.sticky.elements != elements {
                    this.sticky.elements = elements;
                    cx.notify();
                }
            })
            .ok();
        }));
    }
}

// ----- the widget (Monaco `StickyScrollController`) --------------------------------

impl CodeEditor {
    /// Scrolls so sticky line `ix`'s row sits where it's shown and moves
    /// the cursor there (Monaco `_revealPosition` from a sticky line).
    fn reveal_sticky_row(&mut self, row: u32, ix: usize, cx: &mut Context<Self>) {
        let Some(layout) = self.layout.clone() else {
            return;
        };
        let line = self.view_lines.first_view_line_of_row(row);
        let line_height = layout.metrics.line_height;
        self.scroll_top =
            (layout.zones.line_y(line, line_height) - line_height * ix as f32).max(gpui::px(0.));
        self.sticky.focused = None;
        self.set_selections(&[Selection::collapsed(PointUtf16::new(row, 0))], cx);
        self.autoscroll_requested = false;
        cx.notify();
    }

    /// A click on sticky line `ix` (Monaco's sticky scroll click handler):
    /// the folding control folds, Shift reveals the scope's end, otherwise
    /// it goes to the line.
    pub(crate) fn sticky_click(
        &mut self,
        ix: usize,
        event: &gpui::MouseDownEvent,
        cx: &mut Context<Self>,
    ) {
        let Some(line) = self
            .layout
            .as_ref()
            .and_then(|l| l.sticky_lines.get(ix).copied())
        else {
            return;
        };
        let modifiers = event.modifiers;
        if modifiers.control || modifiers.alt || modifiers.platform {
            return;
        }
        if event.button != gpui::MouseButton::Left {
            return;
        }
        if modifiers.shift {
            // The scope's last line, centered if outside the viewport.
            let end = PointUtf16::new(
                (line.end_row + 1).min(self.snapshot.max_point_utf16().row),
                0,
            );
            self.sticky.focused = None;
            self.reveal_range(end..end, RevealType::CenterIfOutsideViewport, cx);
            self.set_selections(&[Selection::collapsed(end)], cx);
            return;
        }
        if let Some(control) = line.fold
            && control.bounds.contains(&event.position)
        {
            self.click_fold(control.row, true, false, false, cx);
            // Keep the header where it was.
            self.reveal_sticky_row(control.row, ix, cx);
            return;
        }
        self.reveal_sticky_row(line.row, ix, cx);
    }

    /// Tracks the sticky line under the mouse; with Shift held it shows its
    /// scope's end (Monaco `_onMouseMoveOrKeyDown`).
    pub(crate) fn sticky_mouse_move(
        &mut self,
        position: Option<gpui::Point<gpui::Pixels>>,
        shift: bool,
        cx: &mut Context<Self>,
    ) {
        let hovered = match position {
            Some(position) => self.layout.as_ref().and_then(|layout| {
                layout.sticky_lines.iter().position(|line| {
                    line.bounds.contains(&position) && position.x >= layout.text_bounds.left()
                })
            }),
            None => self.sticky.hovered,
        };
        let show_end = hovered.filter(|_| shift);
        if hovered != self.sticky.hovered || show_end != self.sticky.show_end {
            self.sticky.hovered = hovered;
            self.sticky.show_end = show_end;
            cx.notify();
        }
    }

    pub(crate) fn sticky_mouse_leave(&mut self, cx: &mut Context<Self>) {
        if self.sticky.hovered.is_some() || self.sticky.show_end.is_some() {
            self.sticky.hovered = None;
            self.sticky.show_end = None;
            cx.notify();
        }
    }

    fn sticky_line_count(&self) -> usize {
        self.layout.as_ref().map_or(0, |l| l.sticky_lines.len())
    }

    /// Focuses the last sticky line for the keyboard (Monaco
    /// `editor.action.focusStickyScroll`).
    pub fn focus_sticky_scroll(&mut self, cx: &mut Context<Self>) {
        let count = self.sticky_line_count();
        if count > 0 {
            self.sticky.focused = Some(count - 1);
            cx.notify();
        }
    }

    /// Whether a sticky line has keyboard focus.
    pub fn is_sticky_scroll_focused(&self) -> bool {
        self.sticky.focused.is_some()
    }

    /// Monaco `focusNext`/`focusPrevious` (Down and Up while focused).
    pub(crate) fn sticky_focus_move(&mut self, next: bool, cx: &mut Context<Self>) {
        let count = self.sticky_line_count();
        if let Some(ix) = self.sticky.focused {
            let ix = if next {
                (ix + 1).min(count.saturating_sub(1))
            } else {
                ix.saturating_sub(1)
            };
            self.sticky.focused = Some(ix);
            cx.notify();
        }
    }

    /// Monaco `goToFocused` (Enter while focused).
    pub(crate) fn sticky_go_to_focused(&mut self, cx: &mut Context<Self>) {
        let Some(ix) = self.sticky.focused else {
            return;
        };
        if let Some(line) = self
            .layout
            .as_ref()
            .and_then(|l| l.sticky_lines.get(ix).copied())
        {
            self.reveal_sticky_row(line.row, ix, cx);
        }
    }

    /// Monaco `selectEditor` (Escape while focused): back to the text.
    pub(crate) fn sticky_select_editor(&mut self, cx: &mut Context<Self>) {
        self.sticky.focused = None;
        cx.notify();
    }

    /// Monaco `editor.action.toggleStickyScroll`.
    pub fn toggle_sticky_scroll(&mut self, cx: &mut Context<Self>) {
        let mut options = self.options.clone();
        options.sticky_scroll.enabled = !options.sticky_scroll.enabled;
        self.update_options(options, cx);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use text::PointUtf16;

    fn symbol(
        name: &str,
        name_row: u32,
        rows: (u32, u32),
        children: Vec<DocumentSymbol>,
    ) -> DocumentSymbol {
        DocumentSymbol {
            name: name.to_string().into(),
            range: PointUtf16::new(rows.0, 0)..PointUtf16::new(rows.1, 1),
            selection_range: PointUtf16::new(name_row, 0)..PointUtf16::new(name_row, 3),
            children,
            ..Default::default()
        }
    }

    #[test]
    fn outline_scopes_follow_monaco() {
        // A doc comment above `impl` (range from row 0, name on row 1);
        // a method; a one-line constant; a field sharing the header line
        // whose child is kept.
        let symbols = vec![
            symbol("main", 20, (20, 22), vec![]),
            symbol(
                "Impl",
                1,
                (0, 12),
                vec![
                    symbol("len", 3, (2, 5), vec![]),
                    symbol("ONE", 6, (6, 6), vec![]),
                    symbol(
                        "same",
                        1,
                        (1, 11),
                        vec![symbol("inner", 8, (8, 10), vec![])],
                    ),
                ],
            ),
        ];
        let elements = from_symbols(&symbols);
        assert_eq!(elements.len(), 2);
        assert_eq!((elements[0].start, elements[0].end), (1, 11));
        let children: Vec<_> = elements[0]
            .children
            .iter()
            .map(|c| (c.start, c.end))
            .collect();
        assert_eq!(children, vec![(3, 4), (8, 9)]);
        assert_eq!((elements[1].start, elements[1].end), (20, 21));

        assert_eq!(enclosing(&elements, 1), vec![]);
        assert_eq!(enclosing(&elements, 4), vec![(1, 11), (3, 4)]);
        assert_eq!(enclosing(&elements, 5), vec![(1, 11)]);
        assert_eq!(enclosing(&elements, 9), vec![(1, 11), (8, 9)]);
        assert_eq!(enclosing(&elements, 12), vec![]);
        assert_eq!(enclosing(&elements, 21), vec![(20, 21)]);
    }

    #[test]
    fn fold_ranges_nest_by_parent() {
        let range = |start_row, end_row, parent| FoldRange {
            start_row,
            end_row,
            parent,
            ..Default::default()
        };
        let elements =
            from_fold_ranges(&[range(0, 9, None), range(2, 4, Some(0)), range(12, 14, None)]);
        assert_eq!(enclosing(&elements, 3), vec![(0, 9), (2, 4)]);
        assert_eq!(enclosing(&elements, 13), vec![(12, 14)]);
    }
}
