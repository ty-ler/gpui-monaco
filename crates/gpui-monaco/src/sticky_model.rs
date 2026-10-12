//! Sticky scroll's model (Monaco `StickyModelProvider`): the scopes whose
//! header lines stick to the top. Per `stickyScroll.defaultModel` it comes
//! from the document symbols (outline), the folding ranges, or indentation,
//! each falling back to the next when it has nothing, whether `folding` is
//! on or not. Which of its scopes stick, and where, is Monaco's
//! `StickyLineCandidateProvider` and `StickyScrollController`
//! ([`candidates_intersecting`], [`find_scroll_widget_state`]).
//!
//! Ports `stickyScrollModelProvider.ts`, `stickyScrollProvider.ts` and
//! `stickyScrollController.ts` (MIT).

use std::sync::Arc;
use std::time::Duration;

use gpui::{Context, Task};

use text::PointUtf16;

use crate::cursor::Selection;
use crate::editor::{CodeEditor, collect_provided_folding_ranges};
use crate::options::StickyScrollModel;
use crate::providers::{DocumentSymbol, document_symbol_providers, folding_range_providers};
use crate::view::folding::FoldRange;
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

/// What the model was computed for: the text model and its text (versions
/// start over in each model), the parsed text (for the tags query), the
/// model kind, the language, the symbol and folding range providers, and
/// whether folding is on.
type StickyKey = (
    gpui::EntityId,
    text::Version,
    Option<text::Version>,
    StickyScrollModel,
    Option<String>,
    (usize, usize),
    bool,
);

#[derive(Default)]
pub(crate) struct StickyModelState {
    key: Option<StickyKey>,
    pending: Option<StickyKey>,
    task: Option<Task<()>>,
    /// The scopes, or `None` to use the editor's folding ranges.
    elements: Option<Arc<Vec<StickyElement>>>,
    /// The folding ranges as scopes, and what they were converted from.
    fold_elements: Option<(u64, Arc<Vec<StickyElement>>)>,
    /// The keyboard-focused sticky line (Monaco `focusStickyScroll`).
    pub focused: Option<usize>,
    /// The sticky line under the mouse.
    pub hovered: Option<usize>,
    /// The mouse is over the sticky lines' gutter, which shows their
    /// folding controls with `showFoldingControls: mouseover` (Monaco
    /// `_isOnGlyphMargin`).
    pub gutter_hovered: bool,
    /// The line showing its scope's end while Shift is held over it.
    pub show_end: Option<usize>,
    /// The sticky line and part of it a mouse button went down on, for
    /// the click.
    pressed: Option<(usize, StickyPart, gpui::MouseButton)>,
}

/// The part of a sticky line under the mouse (Monaco's sticky DOM nodes):
/// a click acts only when it goes down and up on the same one, as a DOM
/// `click` targets the nodes both are in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum StickyPart {
    /// The line number, outside the folding control (`.sticky-line-number`).
    Number,
    /// The folding control (`data-sticky-is-folding-icon`).
    Fold,
    /// The text (`.sticky-line-content`, `data-sticky-is-line`).
    Content,
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
/// the last row it covers. Manual and recovered regions are left out
/// (Monaco's sticky model computes its own ranges from the providers), their
/// children going to the nearest region kept around them.
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
    let kept = |range: &FoldRange| !range.manual && !range.recovered;
    let mut children = vec![Vec::new(); ranges.len()];
    let mut roots = Vec::new();
    for (ix, range) in ranges.iter().enumerate() {
        if !kept(range) {
            continue;
        }
        let mut parent = range.parent;
        while let Some(p) = parent.filter(|&p| !kept(&ranges[p])) {
            parent = ranges[p].parent;
        }
        match parent {
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

/// A sticky line candidate (Monaco `StickyLineCandidate`): a scope's
/// header row and the last row it keeps the header for, where it's drawn
/// in the widget (the heights of the headers above it) and its height.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct StickyLineCandidate {
    pub start: u32,
    pub end: u32,
    pub top: f32,
    pub height: f32,
}

/// Monaco `StickyLineCandidateProvider.getCandidateStickyLinesIntersecting`:
/// the scopes whose header could stick while rows `visible` (inclusive)
/// show, outermost first, each placed below the headers of the scopes
/// around it. Candidates inside a hidden area (`hidden`, row ranges) are
/// dropped (`filterHiddenRanges`). `line_count` is the model's.
pub(crate) fn candidates_intersecting(
    elements: &[StickyElement],
    visible: (u32, u32),
    hidden: &[std::ops::Range<u32>],
    line_count: u32,
    height_of: &dyn Fn(u32) -> f32,
) -> Vec<StickyLineCandidate> {
    struct Walk<'a> {
        range: (u32, u32),
        line_count: u32,
        height_of: &'a dyn Fn(u32) -> f32,
        result: Vec<StickyLineCandidate>,
    }
    // Monaco's line numbers are 1-based and its scopes end a line past the
    // candidate, so this works in Monaco's numbers: a scope is
    // `start + 1 ..= end + 2`.
    fn collect(walk: &mut Walk, children: &[StickyElement], top: f32, last_start: Option<u32>) {
        if children.is_empty() {
            return;
        }
        // `binarySearch` then `updateIndex`: the last child starting at or
        // before the line, else the first.
        let bound = |line: u32| {
            children
                .partition_point(|c| c.start < line)
                .saturating_sub(1)
        };
        let (lower, upper) = (bound(walk.range.0), bound(walk.range.1));
        let mut last_line = last_start;
        for child in &children[lower..=upper] {
            let start_line = child.start + 1;
            let end_line = child.end + 2;
            if end_line > start_line + 1
                && walk.range.0 <= end_line + 1
                && start_line <= walk.range.1 + 1
                && last_line != Some(start_line)
                // `textModel.isValidRange`.
                && end_line <= walk.line_count
            {
                last_line = Some(start_line);
                let height = (walk.height_of)(child.start);
                walk.result.push(StickyLineCandidate {
                    start: child.start,
                    end: child.end,
                    top,
                    height,
                });
                collect(walk, &child.children, top + height, Some(start_line));
            }
        }
    }
    let mut walk = Walk {
        range: (visible.0 + 1, visible.1 + 1),
        line_count,
        height_of,
        result: Vec::new(),
    };
    collect(&mut walk, elements, 0., None);
    let mut result = walk.result;
    result.retain(|candidate| {
        !hidden
            .iter()
            .any(|hidden| candidate.start >= hidden.start && candidate.end <= hidden.end)
    });
    result
}

/// What the sticky scroll widget shows (Monaco `StickyScrollWidgetState`):
/// the header rows, the rows after their scopes (shown with Shift held),
/// and how far the last line is pushed up as its scope scrolls out (0 or
/// negative).
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct StickyWidgetState {
    pub start_rows: Vec<u32>,
    pub end_rows: Vec<u32>,
    pub last_line_relative_position: f32,
}

/// Monaco `StickyScrollController.findScrollWidgetState`: the candidates
/// whose header has scrolled above where it would stick while their scope
/// still shows there, at most `max_lines` of them. `top_of` and
/// `bottom_of` are a row's top and bottom in the content (Monaco
/// `getTopForLineNumber` and `getBottomForLineNumber`).
pub(crate) fn find_scroll_widget_state(
    candidates: &[StickyLineCandidate],
    max_lines: usize,
    scroll_top: f32,
    top_of: &dyn Fn(u32) -> f32,
    bottom_of: &dyn Fn(u32) -> f32,
) -> StickyWidgetState {
    let mut state = StickyWidgetState::default();
    for candidate in candidates {
        let top_of_element = candidate.top;
        let bottom_of_element = top_of_element + candidate.height;
        let top_of_beginning_line = top_of(candidate.start) - scroll_top;
        let bottom_of_end_line = bottom_of(candidate.end) - scroll_top;
        if top_of_element > top_of_beginning_line && top_of_element <= bottom_of_end_line {
            state.start_rows.push(candidate.start);
            state.end_rows.push(candidate.end + 1);
            if bottom_of_element > bottom_of_end_line {
                state.last_line_relative_position = bottom_of_end_line - bottom_of_element;
            }
        }
        if state.start_rows.len() == max_lines {
            break;
        }
    }
    state
}

/// Whether a keystroke stops at a focused sticky line. Monaco focuses its
/// DOM node, so typing and the text's own keys (which need
/// `editorTextFocus`) don't reach the text; its keys (Up, Down, Enter,
/// Escape), function keys and shortcuts with Ctrl, Alt or Cmd go on.
pub(crate) fn sticky_swallows(keystroke: &gpui::Keystroke) -> bool {
    let modifiers = &keystroke.modifiers;
    if modifiers.control || modifiers.alt || modifiers.platform || modifiers.function {
        return false;
    }
    let key = keystroke.key.as_str();
    let sticky_key = !modifiers.shift && matches!(key, "up" | "down" | "enter" | "escape");
    let function_key =
        key.len() > 1 && key.starts_with('f') && key[1..].chars().all(|c| c.is_ascii_digit());
    !sticky_key && !function_key
}

/// Monaco's cap on the sticky lines: a quarter of the editor's height
/// (`_onDidResize`), and `stickyScroll.maxLineCount`.
pub(crate) fn max_sticky_lines(height: f32, line_height: f32, max_line_count: u32) -> usize {
    let theoretical_lines = height / line_height.max(1.);
    ((theoretical_lines * 0.25).round().max(0.) as usize).min(max_line_count as usize)
}

impl CodeEditor {
    /// The sticky scroll model: the computed one, else the folding ranges
    /// (Monaco's folding provider model, which runs the providers itself,
    /// so manual and recovered regions aren't scopes), converted once per
    /// change.
    pub(crate) fn sticky_elements(&mut self) -> Arc<Vec<StickyElement>> {
        if let Some(elements) = &self.sticky.elements {
            return elements.clone();
        }
        let key = self.folds.generation();
        if let Some((cached, elements)) = &self.sticky.fold_elements
            && *cached == key
        {
            return elements.clone();
        }
        let elements = Arc::new(from_fold_ranges(self.folds.ranges()));
        self.sticky.fold_elements = Some((key, elements.clone()));
        elements
    }

    /// Whether the sticky scroll model is for the current text (Monaco
    /// `_renderStickyScroll` renders only when the candidate provider's
    /// version is the model's): the computed model's text, else the
    /// folding ranges'. Neither computed yet counts as current.
    pub(crate) fn sticky_model_is_current(&self, cx: &gpui::App) -> bool {
        let version = if self.sticky.elements.is_some() {
            self.sticky.key.as_ref().map(|key| key.1)
        } else {
            self.folds.ranges_version()
        };
        version.is_none_or(|version| version == self.model().read(cx).buffer().version())
    }

    /// The sticky headers' scopes enclosing `row`, outermost first.
    pub(crate) fn sticky_ranges_enclosing(&mut self, row: u32) -> Vec<(u32, u32)> {
        enclosing(&self.sticky_elements(), row)
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
        let providers = (
            document_symbol_providers(cx, self.provider_target(cx)).len(),
            folding_range_providers(cx, self.provider_target(cx)).len(),
        );
        let parsed = model.syntax_snapshot().map(|syntax| syntax.text.version());
        let key: StickyKey = (
            self.model().entity_id(),
            model.buffer().version(),
            parsed,
            options.default_model,
            language,
            providers,
            self.options.folding,
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
        let default_model = key.3;
        let folding = key.6;
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
                    elements = Some(Arc::new(from_symbols(&symbols)));
                }
            }
            // Monaco's folding provider model, then the indentation model
            // (`IndentRangeProvider`, with the language's `offSide` and
            // markers). With folding on, the editor's folding ranges are
            // those (`None` here); with it off, sticky scroll computes its
            // own.
            let indentation = default_model == StickyScrollModel::Indentation;
            if elements.is_none() && (indentation || !folding) {
                let strategy = if indentation {
                    crate::options::FoldingStrategy::Indentation
                } else {
                    crate::options::FoldingStrategy::Auto
                };
                let Ok((snapshot, _, providers, compute)) =
                    this.update(cx, |this, cx| this.fold_inputs(strategy, cx))
                else {
                    return;
                };
                let provided =
                    collect_provided_folding_ranges(providers, snapshot.rope().line_count()).await;
                let (ranges, _) = cx
                    .background_executor()
                    .spawn(async move { compute(provided) })
                    .await;
                elements = Some(Arc::new(from_fold_ranges(&ranges)));
            }
            this.update(cx, |this, cx| {
                this.sticky.pending = None;
                this.sticky.task = None;
                if this.model().entity_id() != key.0
                    || this.model().read(cx).buffer().version() != key.1
                {
                    // Edited (or another model) meanwhile: the next paint
                    // asks again.
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
    /// Monaco `StickyScrollController._revealPosition` (a sticky line's
    /// click, `goToFocused`): a plain `revealPosition` (the least scrolling
    /// that shows it, with the cursor's surrounding lines), then the cursor
    /// goes to `position`.
    fn reveal_sticky_position(&mut self, position: PointUtf16, cx: &mut Context<Self>) {
        self.sticky.focused = None;
        // `setSelection` doesn't reveal in Monaco: the reveal below replaces
        // the cursor's.
        self.set_selections(&[Selection::collapsed(position)], cx);
        self.reveal_position(position, cx);
    }

    /// A mouse down on sticky line `ix`: Monaco acts on the click, so this
    /// waits for the mouse up on the same part of the same line
    /// ([`Self::sticky_mouse_up`]). The text takes the keyboard back
    /// (Monaco focuses the editor on a sticky mouse down).
    pub(crate) fn sticky_mouse_down(
        &mut self,
        ix: usize,
        event: &gpui::MouseDownEvent,
        cx: &mut Context<Self>,
    ) {
        let Some(part) = self
            .layout
            .as_ref()
            .and_then(|layout| layout.sticky_part_at(ix, event.position))
        else {
            return;
        };
        self.sticky.pressed = Some((ix, part, event.button));
        if self.sticky.focused.take().is_some() {
            cx.notify();
        }
    }

    /// The mouse up after [`Self::sticky_mouse_down`]: a click if it's on
    /// the same part of the same line.
    pub(crate) fn sticky_mouse_up(
        &mut self,
        position: gpui::Point<gpui::Pixels>,
        button: gpui::MouseButton,
        modifiers: gpui::Modifiers,
        cx: &mut Context<Self>,
    ) {
        let Some((ix, part, pressed)) = self.sticky.pressed.take() else {
            return;
        };
        if pressed == button
            && self.layout.as_ref().is_some_and(|layout| {
                layout.sticky_line_at(position) == Some(ix)
                    && layout.sticky_part_at(ix, position) == Some(part)
            })
        {
            self.sticky_click(ix, position, button, modifiers, cx);
        }
    }

    /// A click on sticky line `ix` (Monaco's sticky scroll click handler
    /// and go-to-definition gesture): Cmd/Ctrl follows the definition link
    /// under the mouse, Shift reveals the row after the scope, the folding
    /// control folds or unfolds without moving the cursor, and a click on
    /// the text moves the cursor to the clicked column (column 0 past the
    /// text's end), revealed as by `revealPosition`. A click on the line
    /// number does nothing (Monaco acts only on `data-sticky-is-line`).
    pub(crate) fn sticky_click(
        &mut self,
        ix: usize,
        position: gpui::Point<gpui::Pixels>,
        button: gpui::MouseButton,
        modifiers: gpui::Modifiers,
        cx: &mut Context<Self>,
    ) {
        let Some(layout) = self.layout.clone() else {
            return;
        };
        let Some(line) = layout.sticky_lines.get(ix).copied() else {
            return;
        };
        if button != gpui::MouseButton::Left {
            return;
        }
        let text_position = layout.sticky_position(ix, position);
        if self.is_definition_modifier(&modifiers) {
            if let Some(text_position) = text_position {
                self.sticky.focused = None;
                let side = self.is_side_by_side_modifier(&modifiers);
                self.follow_definition_link(text_position, side, cx);
            }
            return;
        }
        if modifiers.control || modifiers.alt || modifiers.platform {
            return;
        }
        if modifiers.shift {
            // The row after the scope, centered if outside the viewport.
            let end = PointUtf16::new(line.end_row.min(self.snapshot.max_point_utf16().row), 0);
            self.sticky.focused = None;
            self.reveal_range(end..end, RevealType::CenterIfOutsideViewport, cx);
            self.set_selections(&[Selection::collapsed(end)], cx);
            return;
        }
        if let Some(control) = line.fold
            && control.bounds.contains(&position)
        {
            self.toggle_sticky_fold(control.row, ix, cx);
            return;
        }
        if layout.sticky_part_at(ix, position) != Some(StickyPart::Content) {
            return;
        }
        let target = text_position.unwrap_or(PointUtf16::new(line.row, 0));
        self.reveal_sticky_position(target, cx);
    }

    /// Monaco `_toggleFoldingRegionForLine`: folds or unfolds the region,
    /// keeping its header where it shows (`scrollTop = top(header) -
    /// lineHeight * index + 1`), without touching the cursor.
    fn toggle_sticky_fold(&mut self, row: u32, ix: usize, cx: &mut Context<Self>) {
        let Some(layout) = self.layout.clone() else {
            return;
        };
        self.click_fold(row, true, false, false, cx);
        let line = self.view_lines.first_view_line_of_row(row);
        let line_height = layout.metrics.line_height;
        // Monaco `setScrollTop`, immediate: a smooth scroll stops.
        self.animation.cancel_scroll();
        self.scroll_top = (layout.zones.line_y(line, line_height) - line_height * ix as f32
            + gpui::px(1.))
        .max(gpui::px(0.));
        self.autoscroll_requested = false;
        self.emit_scroll_changed(cx);
        cx.notify();
    }

    /// Tracks the sticky line under the mouse; with Shift held it shows its
    /// scope's end (Monaco `_onMouseMoveOrKeyDown`).
    pub(crate) fn sticky_mouse_move(
        &mut self,
        position: Option<gpui::Point<gpui::Pixels>>,
        shift: bool,
        cx: &mut Context<Self>,
    ) {
        let (hovered, gutter_hovered) = match position {
            Some(position) => self.layout.as_ref().map_or((None, false), |layout| {
                let line = layout.sticky_line_at(position);
                let in_text = position.x >= layout.text_bounds.left();
                (line.filter(|_| in_text), line.is_some() && !in_text)
            }),
            None => (self.sticky.hovered, self.sticky.gutter_hovered),
        };
        let show_end = hovered.filter(|_| shift);
        if hovered != self.sticky.hovered
            || show_end != self.sticky.show_end
            || gutter_hovered != self.sticky.gutter_hovered
        {
            self.sticky.hovered = hovered;
            self.sticky.show_end = show_end;
            self.sticky.gutter_hovered = gutter_hovered;
            cx.notify();
        }
    }

    pub(crate) fn sticky_mouse_leave(&mut self, cx: &mut Context<Self>) {
        if self.sticky.hovered.is_some()
            || self.sticky.show_end.is_some()
            || self.sticky.gutter_hovered
        {
            self.sticky.hovered = None;
            self.sticky.show_end = None;
            self.sticky.gutter_hovered = false;
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
            self.reveal_sticky_position(PointUtf16::new(line.row, 0), cx);
        }
    }

    /// Monaco `selectEditor` (Escape while focused): back to the text.
    pub(crate) fn sticky_select_editor(&mut self, cx: &mut Context<Self>) {
        self.sticky.focused = None;
        cx.notify();
    }

    /// Monaco `editor.action.toggleStickyScroll`.
    pub fn toggle_sticky_scroll(&mut self, cx: &mut Context<Self>) {
        let mut options = self.options().clone();
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

    // ----- ported from Monaco's `stickyScroll.test.ts` (0-based rows) -----

    const LINE_HEIGHT: f32 = 19.;

    /// `documentSymbolProviderForTestModel`, for its 14-line text.
    fn test_model_symbols() -> Vec<DocumentSymbol> {
        vec![
            symbol("foo", 0, (0, 2), vec![]),
            symbol(
                "TestClass",
                6,
                (3, 11),
                vec![symbol(
                    "functionOfClass",
                    8,
                    (7, 11),
                    vec![symbol("function1", 9, (9, 10), vec![])],
                )],
            ),
            symbol(
                "bar",
                12,
                (12, 13),
                vec![symbol("insideBar", 12, (12, 12), vec![])],
            ),
        ]
    }

    /// Monaco's `findScrollWidgetState` in a 500px editor with rows
    /// `hidden`, at `scroll_top`.
    fn start_rows_at(
        elements: &[StickyElement],
        line_count: u32,
        hidden: &[std::ops::Range<u32>],
        scroll_top: f32,
    ) -> Vec<u32> {
        let visible_rows: Vec<u32> = (0..line_count)
            .filter(|row| !hidden.iter().any(|h| h.contains(row)))
            .collect();
        // A hidden row is where the visible row above it is.
        let view_line = |row: u32| visible_rows.iter().rposition(|r| *r <= row).unwrap_or(0) as f32;
        let first = visible_rows[((scroll_top / LINE_HEIGHT) as usize).min(visible_rows.len() - 1)];
        let candidates = candidates_intersecting(
            elements,
            (first, line_count - 1),
            hidden,
            line_count,
            &|_| LINE_HEIGHT,
        );
        find_scroll_widget_state(
            &candidates,
            max_sticky_lines(500., LINE_HEIGHT, 5),
            scroll_top,
            &|row| view_line(row) * LINE_HEIGHT,
            &|row| (view_line(row) + 1.) * LINE_HEIGHT,
        )
        .start_rows
    }

    #[test]
    fn get_candidate_sticky_lines_intersecting() {
        let elements = from_symbols(&test_model_symbols());
        let candidate = |start, end, top| StickyLineCandidate {
            start,
            end,
            top,
            height: LINE_HEIGHT,
        };
        let at = |visible| candidates_intersecting(&elements, visible, &[], 14, &|_| LINE_HEIGHT);
        assert_eq!(at((0, 3)), vec![candidate(0, 1, 0.)]);
        assert_eq!(
            at((7, 9)),
            vec![candidate(6, 10, 0.), candidate(8, 10, LINE_HEIGHT)]
        );
        assert_eq!(
            at((9, 12)),
            vec![candidate(6, 10, 0.), candidate(8, 10, LINE_HEIGHT)]
        );
    }

    #[test]
    fn issue_157180_render_the_line_of_the_scope_definition() {
        let elements = from_symbols(&test_model_symbols());
        let at = |lines: f32| start_rows_at(&elements, 14, &[], lines * LINE_HEIGHT + 1.);
        assert_eq!(at(0.), vec![0]);
        assert_eq!(at(1.), vec![0]);
        assert_eq!(at(4.), Vec::<u32>::new());
        assert_eq!(at(8.), vec![6, 8]);
        assert_eq!(at(9.), vec![6, 8]);
        // The inner scope ended above where its header would stick, so it
        // isn't shown at all (it used to be drawn over the outer one).
        assert_eq!(at(10.), vec![6]);
    }

    #[test]
    fn issue_156268_no_sticky_lines_in_a_folded_region() {
        let elements = from_symbols(&test_model_symbols());
        let hidden = [1..2, 9..11];
        let at = |lines: f32| start_rows_at(&elements, 14, &hidden, lines * LINE_HEIGHT + 1.);
        assert_eq!(at(0.), vec![0]);
        assert_eq!(at(1.), Vec::<u32>::new());
        assert_eq!(at(6.), vec![6, 8]);
        assert_eq!(at(7.), vec![6]);
        assert_eq!(at(10.), Vec::<u32>::new());
    }

    #[test]
    fn issue_159271_child_scope_starting_on_the_parent_line() {
        let symbols = vec![symbol(
            "TestClass",
            0,
            (0, 4),
            vec![symbol(
                "foo",
                0,
                (0, 3),
                vec![symbol("bar", 1, (1, 3), vec![])],
            )],
        )];
        let elements = from_symbols(&symbols);
        let at = |lines: f32| start_rows_at(&elements, 6, &[], lines * LINE_HEIGHT + 1.);
        assert_eq!(at(0.), vec![0, 1]);
        assert_eq!(at(1.), vec![0, 1]);
        assert_eq!(at(2.), vec![0]);
        assert_eq!(at(3.), vec![0]);
        assert_eq!(at(4.), Vec::<u32>::new());
    }

    #[test]
    fn the_last_line_is_pushed_up_as_its_scope_scrolls_out() {
        let elements = from_symbols(&test_model_symbols());
        let candidates = candidates_intersecting(&elements, (9, 13), &[], 14, &|_| LINE_HEIGHT);
        // `functionOfClass` ends on row 10: at 9 lines + 14px, 24px of it
        // is left, so its header (second, 19px down) goes 14px up.
        let state = find_scroll_widget_state(
            &candidates,
            5,
            9. * LINE_HEIGHT + 14.,
            &|row| row as f32 * LINE_HEIGHT,
            &|row| (row + 1) as f32 * LINE_HEIGHT,
        );
        assert_eq!(state.start_rows, vec![6, 8]);
        assert_eq!(state.end_rows, vec![11, 11]);
        assert_eq!(state.last_line_relative_position, -14.);
    }

    #[test]
    fn sticky_lines_take_at_most_a_quarter_of_the_height() {
        assert_eq!(max_sticky_lines(500., 19., 5), 5);
        assert_eq!(max_sticky_lines(200., 20., 5), 3);
        assert_eq!(max_sticky_lines(100., 20., 5), 1);
        assert_eq!(max_sticky_lines(30., 20., 5), 0);
    }
}
