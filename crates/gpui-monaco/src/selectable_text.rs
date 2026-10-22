//! Selectable text in rendered markdown (hovers, suggest details,
//! parameter hints, messages), as the browser lets Monaco's be selected.
//!
//! GPUI (0.3.8, and zed main) has no selectable text outside inputs, so
//! this composes it from `StyledText`'s layout: each text block of a
//! rendered markdown ([`SelectableBlock`]) registers its laid-out
//! [`TextLayout`] with the markdown's [`SelectionRoot`], in document
//! order. The root keeps the selection across frames (its element state),
//! selects on drag, double click (a word) and triple click (the block),
//! and paints the selection under the text, a quad per wrapped line.
//!
//! Cmd/Ctrl+C copies a shown selection before anything else handles the
//! keystroke, wherever the focus is (the editor, or the hover a click
//! focused), as the browser's copy takes the page's selection.

use std::cell::RefCell;
use std::ops::Range;
use std::rc::{Rc, Weak};

use gpui::{
    AnyElement, App, Bounds, CursorStyle, Element, ElementId, GlobalElementId, Hitbox,
    HitboxBehavior, Hsla, InspectorElementId, IntoElement, LayoutId, MouseButton, MouseDownEvent,
    MouseMoveEvent, MouseUpEvent, Pixels, Point, SharedString, TextLayout, Window, fill, point, px,
    size,
};
use unicode_segmentation::UnicodeSegmentation as _;

/// A laid-out text block of this frame.
pub(crate) struct Block {
    layout: TextLayout,
    text: SharedString,
}

/// The blocks of a markdown this frame, in document order.
pub(crate) type Blocks = Rc<RefCell<Vec<Block>>>;

/// A position in the text: (block, byte offset in its text).
type Position = (usize, usize);

#[derive(Default)]
struct SelectionState {
    anchor: Position,
    head: Position,
    dragging: bool,
    /// The selected text, kept for copying.
    text: String,
}

impl SelectionState {
    fn range(&self) -> Range<Position> {
        if self.anchor <= self.head {
            self.anchor..self.head
        } else {
            self.head..self.anchor
        }
    }

    fn is_empty(&self) -> bool {
        self.anchor == self.head
    }

    fn update_text(&mut self, blocks: &[Block]) {
        let range = self.range();
        let mut parts = Vec::new();
        for (ix, block) in blocks
            .iter()
            .enumerate()
            .take(range.end.0 + 1)
            .skip(range.start.0)
        {
            let start = if ix == range.start.0 {
                range.start.1
            } else {
                0
            };
            let end = if ix == range.end.0 {
                range.end.1
            } else {
                block.text.len()
            };
            parts.push(block.text.get(start..end).unwrap_or_default());
        }
        self.text = parts.join("\n");
    }
}

thread_local! {
    /// The selection last made, while its markdown is shown.
    static ACTIVE: RefCell<Weak<RefCell<SelectionState>>> = const { RefCell::new(Weak::new()) };
}

/// The selected text of a shown markdown, if there's a selection: what
/// Cmd/Ctrl+C copies.
fn selected_text() -> Option<String> {
    ACTIVE.with(|active| {
        let state = active.borrow().upgrade()?;
        let state = state.borrow();
        (!state.is_empty() && !state.text.is_empty()).then(|| state.text.clone())
    })
}

/// Marks an app whose keystrokes [`intercept_copy`] watches.
struct InterceptingCopy;

impl gpui::Global for InterceptingCopy {}

/// Has Cmd/Ctrl+C copy a shown selection, before the keystroke's actions.
fn intercept_copy(cx: &mut App) {
    if cx.has_global::<InterceptingCopy>() {
        return;
    }
    cx.set_global(InterceptingCopy);
    cx.intercept_keystrokes(|event, _, cx| {
        let keystroke = &event.keystroke;
        let modifiers = keystroke.modifiers;
        if keystroke.key == "c"
            && modifiers.secondary()
            && !modifiers.shift
            && !modifiers.alt
            && let Some(text) = selected_text()
        {
            cx.write_to_clipboard(gpui::ClipboardItem::new_string(text));
            cx.stop_propagation();
        }
    })
    .detach();
}

/// The root of a selectable markdown: `child` holds its
/// [`SelectableBlock`]s, which register with `blocks`.
pub(crate) struct SelectionRoot {
    id: ElementId,
    child: AnyElement,
    blocks: Blocks,
    color: Hsla,
}

impl SelectionRoot {
    pub(crate) fn new(
        id: impl Into<ElementId>,
        child: AnyElement,
        blocks: Blocks,
        color: Hsla,
    ) -> Self {
        Self {
            id: id.into(),
            child,
            blocks,
            color,
        }
    }
}

impl IntoElement for SelectionRoot {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

impl Element for SelectionRoot {
    type RequestLayoutState = ();
    type PrepaintState = Hitbox;

    fn id(&self) -> Option<ElementId> {
        Some(self.id.clone())
    }

    fn source_location(&self) -> Option<&'static std::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, ()) {
        (self.child.request_layout(window, cx), ())
    }

    fn prepaint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) -> Hitbox {
        self.blocks.borrow_mut().clear();
        self.child.prepaint(window, cx);
        window.insert_hitbox(bounds, HitboxBehavior::Normal)
    }

    fn paint(
        &mut self,
        id: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        _: Bounds<Pixels>,
        _: &mut (),
        hitbox: &mut Hitbox,
        window: &mut Window,
        cx: &mut App,
    ) {
        let Some(id) = id else {
            self.child.paint(window, cx);
            return;
        };
        let state =
            window.with_element_state(id, |state: Option<Rc<RefCell<SelectionState>>>, _| {
                let state = state.unwrap_or_default();
                (state.clone(), state)
            });
        // The selection under the text.
        {
            let selection = state.borrow();
            if !selection.is_empty() {
                let blocks = self.blocks.borrow();
                let range = selection.range();
                for (ix, block) in blocks
                    .iter()
                    .enumerate()
                    .take(range.end.0 + 1)
                    .skip(range.start.0)
                {
                    let start = if ix == range.start.0 {
                        range.start.1
                    } else {
                        0
                    };
                    let end = if ix == range.end.0 {
                        range.end.1
                    } else {
                        block.text.len()
                    };
                    let past_end = ix != range.end.0;
                    for bounds in selection_bounds(&block.layout, &block.text, start..end, past_end)
                    {
                        window.paint_quad(fill(bounds, self.color));
                    }
                }
            }
        }
        self.child.paint(window, cx);

        let (blocks, hitbox) = (self.blocks.clone(), hitbox.clone());
        let down_state = state.clone();
        window.on_mouse_event(move |event: &MouseDownEvent, phase, window, cx| {
            if !phase.capture() || event.button != MouseButton::Left {
                return;
            }
            let mut state = down_state.borrow_mut();
            if !hitbox.is_hovered(window) {
                // A click elsewhere clears the selection.
                if !state.is_empty() {
                    state.head = state.anchor;
                    state.text.clear();
                    window.refresh();
                }
                return;
            }
            let blocks = blocks.borrow();
            let Some(position) = position_at(&blocks, event.position) else {
                return;
            };
            let text = &blocks[position.0].text;
            let (anchor, head) = match event.click_count {
                2 => {
                    let word = word_at(text, position.1);
                    ((position.0, word.start), (position.0, word.end))
                }
                n if n >= 3 => ((position.0, 0), (position.0, text.len())),
                _ if event.modifiers.shift && !state.is_empty() => (state.anchor, position),
                _ => (position, position),
            };
            state.anchor = anchor;
            state.head = head;
            state.dragging = event.click_count <= 1;
            state.update_text(&blocks);
            ACTIVE.with(|active| *active.borrow_mut() = Rc::downgrade(&down_state));
            intercept_copy(cx);
            window.refresh();
        });
        let (blocks, move_state) = (self.blocks.clone(), state.clone());
        window.on_mouse_event(move |event: &MouseMoveEvent, phase, window, _| {
            if !phase.capture() {
                return;
            }
            let mut state = move_state.borrow_mut();
            if !state.dragging {
                return;
            }
            if event.pressed_button != Some(MouseButton::Left) {
                state.dragging = false;
                return;
            }
            let blocks = blocks.borrow();
            if let Some(position) = position_at(&blocks, event.position)
                && position != state.head
            {
                state.head = position;
                state.update_text(&blocks);
                window.refresh();
            }
        });
        window.on_mouse_event(move |event: &MouseUpEvent, phase, _, _| {
            if phase.capture() && event.button == MouseButton::Left {
                state.borrow_mut().dragging = false;
            }
        });
    }
}

/// A selectable text block: `child` shows `text`, laid out by `layout`
/// (a `StyledText`'s).
pub(crate) struct SelectableBlock {
    child: AnyElement,
    layout: TextLayout,
    text: SharedString,
    blocks: Blocks,
}

impl SelectableBlock {
    pub(crate) fn new(
        child: AnyElement,
        layout: TextLayout,
        text: SharedString,
        blocks: Blocks,
    ) -> Self {
        Self {
            child,
            layout,
            text,
            blocks,
        }
    }
}

impl IntoElement for SelectableBlock {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

impl Element for SelectableBlock {
    type RequestLayoutState = ();
    type PrepaintState = Hitbox;

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static std::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, ()) {
        (self.child.request_layout(window, cx), ())
    }

    fn prepaint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) -> Hitbox {
        let hitbox = window.insert_hitbox(bounds, HitboxBehavior::Normal);
        self.child.prepaint(window, cx);
        // Laid out now (a text the child left out isn't).
        self.blocks.borrow_mut().push(Block {
            layout: self.layout.clone(),
            text: self.text.clone(),
        });
        hitbox
    }

    fn paint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        _: Bounds<Pixels>,
        _: &mut (),
        hitbox: &mut Hitbox,
        window: &mut Window,
        cx: &mut App,
    ) {
        // Links set the pointer over this.
        window.set_cursor_style(CursorStyle::IBeam, hitbox);
        self.child.paint(window, cx);
    }
}

/// The text position nearest `point`: in the block whose lines span its
/// height, else the nearest block above or below.
fn position_at(blocks: &[Block], point: Point<Pixels>) -> Option<Position> {
    let mut best: Option<(Pixels, Position)> = None;
    for (ix, block) in blocks.iter().enumerate() {
        let bounds = block.layout.bounds();
        let distance = if point.y < bounds.top() {
            bounds.top() - point.y
        } else if point.y > bounds.bottom() {
            point.y - bounds.bottom()
        } else {
            px(0.)
        };
        let offset = if point.y < bounds.top() {
            0
        } else if point.y > bounds.bottom() {
            block.text.len()
        } else {
            match block.layout.index_for_position(point) {
                Ok(ix) | Err(ix) => ix.min(block.text.len()),
            }
        };
        if best.is_none_or(|(best, _)| distance < best) {
            best = Some((distance, (ix, offset)));
        }
        if distance == px(0.) {
            break;
        }
    }
    best.map(|(_, position)| position)
}

/// The word (or run of other characters) at byte `offset` of `text`, as
/// a double click selects.
fn word_at(text: &str, offset: usize) -> Range<usize> {
    let mut start = 0;
    for (ix, word) in text.split_word_bound_indices() {
        let end = ix + word.len();
        if offset < end || end == text.len() {
            return ix..end;
        }
        start = end;
    }
    start..text.len()
}

/// The selection's rectangles over `range` of a block: one per wrapped
/// line. `past_end`: the selection goes on past the range's end, as
/// past a line end, so that line's selection shows a sliver after it.
fn selection_bounds(
    layout: &TextLayout,
    text: &str,
    range: Range<usize>,
    past_end: bool,
) -> Vec<Bounds<Pixels>> {
    let mut result = Vec::new();
    let line_height = layout.line_height();
    let sliver = line_height * 0.3;
    let left = layout.bounds().left();
    let mut line_start = 0;
    for line in layout.line_layouts() {
        let line_len = line.unwrapped_layout.len;
        let line_end = line_start + line_len;
        let (start, end) = (range.start.max(line_start), range.end.min(line_end));
        // A selection over the line's end includes its line break.
        let over_break = range.end > line_end || (past_end && range.end == line_end);
        if start <= end && !(start == end && !over_break) {
            // Visual lines start at each wrap boundary.
            let unwrapped = &line.unwrapped_layout;
            let mut starts: Vec<usize> = vec![0];
            starts.extend(
                line.wrap_boundaries
                    .iter()
                    .map(|b| unwrapped.runs[b.run_ix].glyphs[b.glyph_ix].index),
            );
            for (row, &row_start) in starts.iter().enumerate() {
                let row_end = starts.get(row + 1).copied().unwrap_or(line_len);
                let (s, e) = (
                    (start - line_start).max(row_start),
                    (end - line_start).min(row_end),
                );
                let last_row = row + 1 == starts.len();
                if s > e || (s == e && !(last_row && over_break)) {
                    continue;
                }
                let origin_x = unwrapped.x_for_index(row_start);
                let x0 = unwrapped.x_for_index(s) - origin_x;
                let mut x1 = unwrapped.x_for_index(e) - origin_x;
                if last_row && over_break && e == row_end {
                    x1 += sliver;
                }
                let y = layout
                    .position_for_index(line_start + row_start)
                    .map_or(layout.bounds().top(), |p| p.y);
                result.push(Bounds::new(point(left + x0, y), size(x1 - x0, line_height)));
            }
        }
        // Lines are separated by one `\n` byte.
        line_start = line_end + 1;
        if line_start > text.len() {
            break;
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn double_click_selects_the_word() {
        let text = "let foo_bar = 1;";
        assert_eq!(&text[word_at(text, 5)], "foo_bar");
        assert_eq!(&text[word_at(text, 0)], "let");
        assert_eq!(&text[word_at(text, 3)], " ");
        assert_eq!(&text[word_at(text, text.len())], ";");
    }
}
