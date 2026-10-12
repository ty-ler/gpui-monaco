//! Screen reader support (Monaco's screen reader content: `screenReaderUtils.ts`
//! `SimplePagedScreenReaderStrategy`, MIT). Monaco fills a hidden textarea
//! with the pages of view lines holding the selection; here those lines
//! become AccessKit text runs under the editor's text input node.
//! Announcements are Monaco's `aria.ts` `alert` (assertive) and `status`
//! (polite) live regions.

use std::cell::RefCell;
use std::collections::HashMap;
use std::ops::Range;
use std::rc::Rc;

use gpui::accesskit::{self, NodeId, Role};
use gpui::{A11ySubtreeBuilder, Context, SharedString, Window};
use text::PointUtf16;

use crate::cursor::Selection;
use crate::editor::CodeEditor;
use crate::options::AccessibilitySupport;
use crate::view::view_lines::ViewLines;

/// Monaco `ariaLabelForScreenReaderContent` with `accessibilitySupport:
/// 'off'` (without a keybinding to turn screen reader mode on).
pub(crate) const NOT_ACCESSIBLE_LABEL: &str = "The editor is not accessible at this time.";

/// Screen reader mode (Monaco's computed `accessibilitySupport`) and
/// announcements (Monaco `aria.alert` and `aria.status`).
#[derive(Default)]
pub(crate) struct ScreenReaderState {
    /// Monaco `isDominatedByLongLines`, read when the model is attached.
    pub dominated_by_long_lines: bool,
    /// Whether the last frame was optimized for a screen reader.
    pub optimized: bool,
    /// The last announcement.
    pub announcement: Option<Announcement>,
    /// What identifies the inline suggestion announced last.
    pub announced_suggestion: Option<String>,
    /// The last screen reader content and what it was built from: the
    /// view lines' generation, the selection and the page size.
    pub content: Option<(ContentKey, Rc<ScreenReaderContent>)>,
}

type ContentKey = (u64, Selection, u32);

/// A message for screen readers, in one of Monaco's live regions.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Announcement {
    pub message: SharedString,
    /// Whether it's in the second of the region's two containers (Monaco
    /// alternates them so a repeated message is read again).
    pub second: bool,
    /// Monaco `status` (a polite live region) rather than `alert`.
    pub polite: bool,
}

/// Adds the announcement as a live region: assertive (Monaco's
/// `role="alert"` container) or, for a status, polite (Monaco's
/// `role="status"` container).
/// What an element tells assistive technology: Monaco's `role`,
/// `aria-label` and ARIA states on a widget's DOM node. GPUI makes an
/// accessibility node only for an element with an id and a role.
#[doc(hidden)]
#[derive(Clone, Debug, Default, PartialEq)]
pub struct AriaNode {
    pub role: Role,
    pub label: Option<SharedString>,
    /// `aria-checked` (a checkbox or a checked menu item).
    pub checked: Option<bool>,
    /// `aria-expanded`.
    pub expanded: Option<bool>,
    /// `aria-selected`.
    pub selected: Option<bool>,
    /// `aria-level` (a tree item's depth, from 1).
    pub level: Option<usize>,
    /// `aria-keyshortcuts`.
    pub keyshortcuts: Option<SharedString>,
    /// The focused container's active descendant (`aria-activedescendant`).
    pub active_descendant: bool,
    /// `aria-roledescription`.
    pub role_description: Option<SharedString>,
    /// `aria-autocomplete` (`None` is `"none"`).
    pub auto_complete: Option<accesskit::AutoComplete>,
}

impl AriaNode {
    pub(crate) fn new(role: Role, label: impl Into<SharedString>) -> Self {
        Self {
            role,
            label: Some(label.into()),
            ..Default::default()
        }
    }

    pub(crate) fn unlabelled(role: Role) -> Self {
        Self {
            role,
            ..Default::default()
        }
    }

    pub(crate) fn checked(mut self, checked: bool) -> Self {
        self.checked = Some(checked);
        self
    }

    pub(crate) fn expanded(mut self, expanded: bool) -> Self {
        self.expanded = Some(expanded);
        self
    }

    pub(crate) fn selected(mut self, selected: bool) -> Self {
        self.selected = Some(selected);
        self
    }

    pub(crate) fn level(mut self, level: usize) -> Self {
        self.level = Some(level);
        self
    }

    pub(crate) fn keyshortcuts(mut self, keys: Option<SharedString>) -> Self {
        self.keyshortcuts = keys;
        self
    }

    pub(crate) fn active_descendant(mut self, active: bool) -> Self {
        self.active_descendant = active;
        self
    }

    /// Sets the node's role, label and states on `element` (which needs an
    /// id); recorded for tests while [`record_aria_for_tests`] is on.
    pub(crate) fn apply<E: gpui::StatefulInteractiveElement>(self, element: E) -> E {
        record_aria(&self);
        let mut element = element.role(self.role);
        if let Some(label) = self.label {
            element = element.aria_label(label);
        }
        if let Some(checked) = self.checked {
            element = element.aria_toggled(if checked {
                accesskit::Toggled::True
            } else {
                accesskit::Toggled::False
            });
        }
        if let Some(expanded) = self.expanded {
            element = element.aria_expanded(expanded);
        }
        if let Some(selected) = self.selected {
            element = element.aria_selected(selected);
        }
        if let Some(level) = self.level {
            element = element.aria_level(level);
        }
        if let Some(keys) = self.keyshortcuts {
            element = element.aria_keyshortcuts(keys);
        }
        if self.active_descendant {
            element = element.aria_active_descendant();
        }
        element
    }
}

thread_local! {
    /// The nodes rendered since [`record_aria_for_tests`] (GPUI's test
    /// platform builds no accessibility tree).
    static RECORDED_ARIA: RefCell<Option<Vec<AriaNode>>> = const { RefCell::new(None) };
}

/// Records `node` while [`record_aria_for_tests`] is on.
pub(crate) fn record_aria(node: &AriaNode) {
    RECORDED_ARIA.with(|recorded| {
        if let Some(nodes) = recorded.borrow_mut().as_mut() {
            nodes.push(node.clone());
        }
    });
}

/// Starts recording the accessibility nodes rendered on this thread, for
/// tests: [`recorded_aria_for_tests`] returns them.
#[doc(hidden)]
pub fn record_aria_for_tests() {
    RECORDED_ARIA.with(|recorded| *recorded.borrow_mut() = Some(Vec::new()));
}

/// The accessibility nodes rendered since the last call or
/// [`record_aria_for_tests`], for tests.
#[doc(hidden)]
pub fn recorded_aria_for_tests() -> Vec<AriaNode> {
    RECORDED_ARIA.with(|recorded| {
        recorded
            .borrow_mut()
            .as_mut()
            .map(std::mem::take)
            .unwrap_or_default()
    })
}

pub(crate) fn build_announcement(builder: &mut A11ySubtreeBuilder, announcement: &Announcement) {
    let (key, role, live) = if announcement.polite {
        ("status", Role::Status, accesskit::Live::Polite)
    } else {
        ("announcement", Role::Alert, accesskit::Live::Assertive)
    };
    let id = builder.synthetic_node_id((key, announcement.second));
    let mut node = accesskit::Node::new(role);
    node.set_live(live);
    node.set_label(announcement.message.to_string());
    // AccessKit's macOS adapter announces a live region's value.
    node.set_value(announcement.message.to_string());
    builder.push_child(id, node);
}

/// The lines screen reader content reads (Monaco's `ISimpleModel`, which
/// the editor's view model implements).
pub(crate) trait ScreenReaderLines {
    /// How many lines there are.
    fn line_count(&self) -> u32;
    /// Line `line`'s text, without its line break.
    fn line_content(&self, line: u32) -> String;
}

impl ScreenReaderLines for ViewLines {
    fn line_count(&self) -> u32 {
        ViewLines::line_count(self)
    }

    fn line_content(&self, line: u32) -> String {
        ViewLines::line_content(self, line)
    }
}

/// Monaco's ellipsis between the start and end pages of a long selection.
const ELLIPSIS: &str = "\u{2026}";

/// What a screen reader reads (Monaco `SimplePagedScreenReaderStrategy`):
/// the page of view lines holding the selection's start and the page
/// holding its end, so at most two pages; when pages lie between them, an
/// ellipsis stands for those.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct ScreenReaderContent {
    /// The view lines read: one range, or two with the ellipsis between.
    pub ranges: Vec<Range<u32>>,
    /// The lines' text, without line breaks, in order.
    pub lines: Vec<String>,
    /// Whether the last line read has a line break after it (it isn't the
    /// last view line).
    pub trailing_line_break: bool,
    /// The selection's anchor, in view lines and view columns.
    pub anchor: PointUtf16,
    /// The selection's head, in view lines and view columns.
    pub head: PointUtf16,
}

/// Monaco `SimplePagedScreenReaderStrategy.fromEditorSelection` without
/// `trimLongText` (Monaco only trims while it doesn't know whether a screen
/// reader is attached): `anchor` and `head` are view positions, and pages
/// are `lines_per_page` view lines.
pub(crate) fn screen_reader_content(
    lines: &impl ScreenReaderLines,
    anchor: PointUtf16,
    head: PointUtf16,
    lines_per_page: u32,
) -> ScreenReaderContent {
    let lines_per_page = lines_per_page.max(1);
    let count = lines.line_count().max(1);
    let page = |line: u32| line.min(count - 1) / lines_per_page;
    let page_range = |page: u32| page * lines_per_page..((page + 1) * lines_per_page).min(count);
    let start_page = page(anchor.row.min(head.row));
    let end_page = page(anchor.row.max(head.row));
    // Monaco takes the whole selection when it ends on the start page or
    // the next; otherwise the two pages' parts of it, joined by `…`.
    let ranges: Vec<Range<u32>> = if end_page <= start_page + 1 {
        std::iter::once(page_range(start_page).start..page_range(end_page).end).collect()
    } else {
        vec![page_range(start_page), page_range(end_page)]
    };
    let end = ranges.last().map_or(0, |range| range.end);
    ScreenReaderContent {
        lines: ranges
            .iter()
            .flat_map(|range| range.clone().map(|line| lines.line_content(line)))
            .collect(),
        ranges,
        trailing_line_break: end < count,
        anchor,
        head,
    }
}

/// The character index of UTF-16 `column` in `line` (AccessKit counts
/// characters).
fn char_index(line: &str, column: u32) -> usize {
    let mut units = 0;
    for (ix, ch) in line.chars().enumerate() {
        if units >= column as usize {
            return ix;
        }
        units += ch.len_utf16();
    }
    line.chars().count()
}

/// The UTF-16 column of character `index` in `line`.
fn utf16_column(line: &str, index: usize) -> u32 {
    line.chars().take(index).map(|c| c.len_utf16() as u32).sum()
}

/// What a text run of the screen reader content shows.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum A11yRun {
    /// A view line.
    Line(u32),
    /// The ellipsis before view line `.0`.
    Ellipsis(u32),
}

/// Text run node ids to what they show, filled while building the tree,
/// for actions coming back from assistive technology.
pub(crate) type RunRows = Rc<RefCell<HashMap<NodeId, A11yRun>>>;

impl ScreenReaderContent {
    /// The text runs: each line, with its line break as in a textarea
    /// (the last one only when more lines follow), and the ellipsis
    /// between the pages.
    fn runs(&self) -> Vec<(A11yRun, String)> {
        let mut runs = Vec::with_capacity(self.lines.len() + 1);
        let mut texts = self.lines.iter();
        let mut left = self.lines.len();
        for (ix, range) in self.ranges.iter().enumerate() {
            if ix > 0 {
                runs.push((A11yRun::Ellipsis(range.start), ELLIPSIS.to_string()));
            }
            for line in range.clone() {
                let mut text = texts.next().cloned().unwrap_or_default();
                left -= 1;
                if left > 0 || self.trailing_line_break {
                    text.push('\n');
                }
                runs.push((A11yRun::Line(line), text));
            }
        }
        runs
    }

    /// The run index and character index of view position `point`, which
    /// is on a line read (else the content's end).
    fn position(&self, point: PointUtf16) -> (usize, usize) {
        let mut run = 0;
        let mut first_line = 0;
        for (ix, range) in self.ranges.iter().enumerate() {
            if ix > 0 {
                run += 1;
            }
            if range.contains(&point.row) {
                let offset = (point.row - range.start) as usize;
                let line = &self.lines[first_line + offset];
                return (run + offset, char_index(line, point.column));
            }
            run += range.len();
            first_line += range.len();
        }
        let last = self.lines.last().map_or(0, |line| line.chars().count());
        (run.saturating_sub(1), last)
    }

    /// The content as one string, and the selection's anchor and head
    /// offsets in it in characters (Monaco's `TextAreaState`).
    pub(crate) fn text_area_state(&self) -> (String, usize, usize) {
        let runs = self.runs();
        let offset = |(run, ch): (usize, usize)| -> usize {
            runs[..run]
                .iter()
                .map(|(_, text)| text.chars().count())
                .sum::<usize>()
                + ch
        };
        let value = runs.iter().map(|(_, text)| text.as_str()).collect();
        (
            value,
            offset(self.position(self.anchor)),
            offset(self.position(self.head)),
        )
    }

    /// Adds a text run per line and for the ellipsis, and the selection,
    /// to the editor's node.
    pub(crate) fn build(&self, builder: &mut A11ySubtreeBuilder, rows: &RunRows) {
        let mut ids = rows.borrow_mut();
        ids.clear();
        let runs = self.runs();
        let mut run_ids = Vec::with_capacity(runs.len());
        for (what, text) in runs {
            let id = match what {
                A11yRun::Line(line) => builder.synthetic_node_id(line),
                A11yRun::Ellipsis(line) => builder.synthetic_node_id(("ellipsis", line)),
            };
            let mut run = accesskit::Node::new(Role::TextRun);
            run.set_character_lengths(text.chars().map(|c| c.len_utf8() as u8).collect::<Vec<_>>());
            run.set_value(text);
            builder.push_child(id, run);
            ids.insert(id, what);
            run_ids.push(id);
        }
        if run_ids.is_empty() {
            return;
        }
        let position = |point: PointUtf16| {
            let (run, character_index) = self.position(point);
            accesskit::TextPosition {
                node: run_ids[run.min(run_ids.len() - 1)],
                character_index,
            }
        };
        builder
            .parent_node()
            .set_text_selection(accesskit::TextSelection {
                anchor: position(self.anchor),
                focus: position(self.head),
            });
    }
}

/// The editor selection for an AccessKit text selection over the text runs
/// in `rows`: the model position of a view line's character, or of the
/// start of the line after the ellipsis.
fn selection_from_a11y(
    lines: &ViewLines,
    rows: &HashMap<NodeId, A11yRun>,
    selection: &accesskit::TextSelection,
) -> Option<Selection> {
    let point = |position: &accesskit::TextPosition| {
        let (line, column) = match *rows.get(&position.node)? {
            A11yRun::Line(line) => {
                let text = lines.line_content(line);
                (line, utf16_column(&text, position.character_index))
            }
            A11yRun::Ellipsis(line) => (line, lines.line_min_column(line)),
        };
        Some(lines.to_model(line, column))
    };
    Some(Selection::new(
        point(&selection.anchor)?,
        point(&selection.focus)?,
    ))
}

impl CodeEditor {
    /// The editor's own node (Monaco's textarea in
    /// `TextAreaEditContext`): a text input (or Monaco `setAriaOptions({
    /// role })`'s role) labelled by `ariaLabelForScreenReaderContent`, with
    /// the role description "editor" and `aria-autocomplete` "both", "none"
    /// when read-only, or "list" while `setAriaOptions` points it at an
    /// active descendant. A widget input is a plain `<input>` in Monaco, so
    /// it has neither.
    pub(crate) fn root_aria(&self) -> AriaNode {
        let role = self
            .aria_options
            .role
            .unwrap_or(if self.options.single_line {
                Role::TextInput
            } else {
                Role::MultilineTextInput
            });
        let label = if self.options.accessibility_support == AccessibilitySupport::Off {
            NOT_ACCESSIBLE_LABEL.into()
        } else {
            self.options.aria_label.clone()
        };
        let mut aria = AriaNode::new(role, label);
        if !self.widget_input {
            aria.role_description = Some("editor".into());
            aria.auto_complete = if self.aria_options.active_descendant.is_some() {
                Some(accesskit::AutoComplete::List)
            } else if self.options.read_only {
                None
            } else {
                Some(accesskit::AutoComplete::Both)
            };
        }
        aria
    }

    /// The sticky scroll's lines for screen readers (Monaco's
    /// `sticky-widget-lines`, a `list` of `listitem`s), where the last
    /// frame drew them. They're painted by the editor element, so these
    /// are empty elements over them.
    pub(crate) fn sticky_scroll_aria(&mut self) -> Option<gpui::AnyElement> {
        use gpui::{InteractiveElement as _, IntoElement as _, ParentElement as _, Styled as _};
        let layout = self.layout.as_ref()?;
        self.aria_sticky_rows = layout.sticky_lines.iter().map(|line| line.row).collect();
        let first = layout.sticky_lines.first()?;
        // The root's top is the banner's.
        let banner = self
            .banner
            .as_ref()
            .map_or(gpui::px(0.), |(_, height)| *height);
        let origin = layout.bounds.origin - gpui::point(gpui::px(0.), banner);
        let top = layout.bounds.top();
        let bottom = layout
            .sticky_lines
            .iter()
            .map(|line| line.bounds.bottom())
            .fold(first.bounds.bottom(), gpui::Pixels::max);
        let items = layout.sticky_lines.iter().enumerate().map(|(ix, line)| {
            AriaNode::new(Role::ListItem, self.snapshot.line(line.row))
                .apply(gpui::div().id(("sticky-line-aria", ix)))
                .absolute()
                .left(line.bounds.left() - layout.bounds.left())
                .top(line.bounds.top() - top)
                .w(line.bounds.size.width)
                .h(line.bounds.size.height)
        });
        Some(
            AriaNode::unlabelled(Role::List)
                .apply(gpui::div().id("sticky-scroll-aria"))
                .absolute()
                .left(layout.bounds.left() - origin.x)
                .top(top - origin.y)
                .w(layout.bounds.size.width)
                .h(bottom - top)
                .children(items)
                .into_any_element(),
        )
    }

    /// What the editor's own node tells assistive technology, for tests
    /// (GPUI's test platform builds no accessibility tree).
    #[doc(hidden)]
    pub fn aria_for_tests(&self) -> AriaNode {
        self.root_aria()
    }

    /// Whether the editor runs optimized for screen readers (Monaco's
    /// computed `accessibilitySupport` is `Enabled`): `On`, or `Auto` while
    /// assistive technology is connected.
    pub(crate) fn screen_reader_optimized(&self, window: &Window) -> bool {
        match self.options.accessibility_support {
            AccessibilitySupport::On => true,
            AccessibilitySupport::Off => false,
            AccessibilitySupport::Auto => window.is_a11y_active(),
        }
    }

    /// Whether the last frame was optimized for screen readers (see
    /// [`EditorOptions::accessibility_support`](crate::EditorOptions::accessibility_support)).
    pub fn is_screen_reader_optimized(&self) -> bool {
        self.screen_reader.optimized
    }

    /// Has screen readers read `message` now (Monaco `aria.alert`): an
    /// assertive live region under the editor's node, which interrupts
    /// what they're reading.
    pub fn announce(&mut self, message: impl Into<SharedString>, cx: &mut Context<Self>) {
        self.set_announcement(message.into(), false);
        cx.notify();
    }

    /// Has screen readers read `message` once they're done with what
    /// they're reading (Monaco `aria.status`): a polite live region under
    /// the editor's node.
    pub fn announce_status(&mut self, message: impl Into<SharedString>, cx: &mut Context<Self>) {
        self.set_announcement(message.into(), true);
        cx.notify();
    }

    fn set_announcement(&mut self, message: SharedString, polite: bool) {
        // Monaco uses the region's second container when the first already
        // says it.
        let second = self
            .screen_reader
            .announcement
            .as_ref()
            .is_some_and(|last| last.polite == polite && last.message == message && !last.second);
        self.screen_reader.announcement = Some(Announcement {
            message,
            second,
            polite,
        });
    }

    /// The last announcement, for tests.
    #[doc(hidden)]
    pub fn last_announcement_for_tests(&self) -> Option<SharedString> {
        self.screen_reader
            .announcement
            .as_ref()
            .map(|announcement| announcement.message.clone())
    }

    /// Whether the last announcement was a polite status
    /// ([`Self::announce_status`]), for tests.
    #[doc(hidden)]
    pub fn last_announcement_is_status_for_tests(&self) -> bool {
        self.screen_reader
            .announcement
            .as_ref()
            .is_some_and(|announcement| announcement.polite)
    }

    /// The announcement for the accessibility tree.
    pub(crate) fn announcement(&self) -> Option<Announcement> {
        self.screen_reader.announcement.clone()
    }

    /// Monaco `InlineCompletionsController`'s
    /// `playAccessibilitySignalAndReadSuggestion`, run each frame: once
    /// another inline suggestion shows (and the suggest widget is closed),
    /// screen readers read its ghost text, or where the next edit
    /// suggestion is, with `screen_reader_announce_inline_suggestion`.
    pub(crate) fn announce_inline_suggestion(&mut self, cx: &mut Context<Self>) {
        if !self.screen_reader.optimized || self.is_suggest_visible() {
            return;
        }
        let suggestion = if let Some((row, text)) = self.ghost_text_for_screen_reader(cx) {
            Some((format!("ghost {row} {text}"), text))
        } else if let Some((ix, edits)) = self.inline_edit_state(cx) {
            let line = edits.first().map_or(0, |(range, _)| range.start.row) + 1;
            let message = if self.tab_should_accept_inline_edit(cx) {
                format!("Next edit suggestion available on line {line}, press Tab to accept")
            } else if self.tab_should_jump_to_inline_edit(cx) {
                format!("Next edit suggestion available on line {line}, press Tab to jump")
            } else {
                format!("Next edit suggestion available on line {line}")
            };
            Some((format!("edit {ix} {edits:?}"), message))
        } else {
            None
        };
        let key = suggestion.as_ref().map(|(key, _)| key.clone());
        if key == self.screen_reader.announced_suggestion {
            return;
        }
        self.screen_reader.announced_suggestion = key;
        if let Some((_, message)) = suggestion
            && self.options.screen_reader_announce_inline_suggestion
            && !message.is_empty()
        {
            self.set_announcement(message.into(), false);
        }
    }

    /// What the screen reader gets for the primary selection, in view
    /// lines; kept until the view lines, the selection or the page size
    /// change (as Monaco's rich content compares its state).
    pub(crate) fn screen_reader_content(&mut self) -> Rc<ScreenReaderContent> {
        let selection = self.cursors.primary().selection();
        let page_size = self.options.accessibility_page_size;
        let key = (self.view_lines.generation(), selection, page_size);
        if let Some((cached, content)) = &self.screen_reader.content
            && *cached == key
        {
            return content.clone();
        }
        let lines = &*self.view_lines;
        let view = |point: PointUtf16| {
            let (line, column) = lines.to_view(point, false);
            PointUtf16::new(line, column)
        };
        let content = Rc::new(screen_reader_content(
            lines,
            view(selection.anchor),
            view(selection.head),
            page_size,
        ));
        self.screen_reader.content = Some((key, content.clone()));
        content
    }

    /// The screen reader content as Monaco's `TextAreaState`: the text,
    /// and the selection's anchor and head offsets in it (in characters).
    #[doc(hidden)]
    pub fn screen_reader_content_for_tests(&mut self) -> (String, usize, usize) {
        self.screen_reader_content().text_area_state()
    }

    /// AccessKit's `SetTextSelection`: assistive technology moved the
    /// caret or selection.
    pub(crate) fn a11y_set_selection(
        &mut self,
        selection: &accesskit::TextSelection,
        rows: &RunRows,
        cx: &mut Context<Self>,
    ) {
        let found = selection_from_a11y(&self.view_lines, &rows.borrow(), selection);
        if let Some(selection) = found {
            self.set_selections(&[selection], cx);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use text::Rope;

    impl ScreenReaderLines for Vec<&str> {
        fn line_count(&self) -> u32 {
            self.len() as u32
        }

        fn line_content(&self, line: u32) -> String {
            self[line as usize].to_string()
        }
    }

    /// Monaco `textAreaState.test.ts`'s `testPagedScreenReaderStrategy`:
    /// pages of 10 lines, a selection from 1-based (line, column) `start`
    /// to `end`, and the expected text and selection offsets.
    fn paged(text: &str, start: (u32, u32), end: (u32, u32), expected: (&str, usize, usize)) {
        let lines: Vec<&str> = text.split('\n').collect();
        let p = |(line, column): (u32, u32)| PointUtf16::new(line - 1, column - 1);
        let content = screen_reader_content(&lines, p(start), p(end), 10);
        let (value, anchor, head) = content.text_area_state();
        assert_eq!((value.as_str(), anchor, head), expected);
    }

    const LINES: &str = "L1\nL2\nL3\nL4\nL5\nL6\nL7\nL8\nL9\nL10\nL11\nL12\nL13\nL14\nL15\nL16\nL17\nL18\nL19\nL20\nL21";

    #[test]
    fn paged_simple() {
        paged("Hello world!", (1, 13), (1, 13), ("Hello world!", 12, 12));
        paged("Hello world!", (1, 1), (1, 1), ("Hello world!", 0, 0));
        paged("Hello world!", (1, 1), (1, 6), ("Hello world!", 0, 5));
    }

    #[test]
    fn paged_multiline() {
        let text = "Hello world!\nHow are you?";
        paged(text, (1, 1), (1, 1), ("Hello world!\nHow are you?", 0, 0));
        paged(text, (2, 1), (2, 1), ("Hello world!\nHow are you?", 13, 13));
    }

    #[test]
    fn paged_page() {
        let first = "L1\nL2\nL3\nL4\nL5\nL6\nL7\nL8\nL9\nL10\n";
        let second = "L11\nL12\nL13\nL14\nL15\nL16\nL17\nL18\nL19\nL20\n";
        paged(LINES, (1, 1), (1, 1), (first, 0, 0));
        paged(LINES, (11, 1), (11, 1), (second, 0, 0));
        paged(LINES, (12, 1), (12, 1), (second, 4, 4));
        paged(LINES, (21, 1), (21, 1), ("L21", 0, 0));
    }

    #[test]
    fn paged_selections_span_at_most_two_pages() {
        // A selection ending on the next page reads both pages whole.
        let content = screen_reader_content(
            &LINES.split('\n').collect::<Vec<_>>(),
            PointUtf16::new(8, 0),
            PointUtf16::new(12, 1),
            10,
        );
        assert_eq!(content.ranges, vec![0..20]);
        // Select All in 10,000 lines: the first and last pages, with an
        // ellipsis for the selection's middle (Monaco's `…`).
        let text: String = (1..=10_000).map(|i| format!("L{i}\n")).collect();
        let lines: Vec<&str> = text.trim_end().split('\n').collect();
        let content =
            screen_reader_content(&lines, PointUtf16::new(0, 0), PointUtf16::new(9_999, 6), 10);
        assert_eq!(content.ranges, vec![0..10, 9_990..10_000]);
        let (value, anchor, head) = content.text_area_state();
        assert!(value.starts_with("L1\nL2\n"));
        assert!(value.contains("L10\n\u{2026}L9991\n"));
        assert!(value.ends_with("L10000"));
        assert_eq!((anchor, head), (0, value.chars().count()));
        // Reversed, the anchor is the end.
        let content =
            screen_reader_content(&lines, PointUtf16::new(9_999, 6), PointUtf16::new(0, 0), 10);
        let (value, anchor, head) = content.text_area_state();
        assert_eq!((anchor, head), (value.chars().count(), 0));
    }

    #[test]
    fn selections_from_assistive_technology() {
        let lines = ViewLines::new(Rope::from("ab\nc😀d\n"), 0);
        let rows: HashMap<NodeId, A11yRun> = [
            (NodeId(7), A11yRun::Line(0)),
            (NodeId(8), A11yRun::Line(1)),
            (NodeId(9), A11yRun::Ellipsis(1)),
        ]
        .into_iter()
        .collect();
        let position = |node, character_index| accesskit::TextPosition {
            node: NodeId(node),
            character_index,
        };
        let selection = accesskit::TextSelection {
            anchor: position(7, 1),
            focus: position(8, 2),
        };
        assert_eq!(
            selection_from_a11y(&lines, &rows, &selection),
            Some(Selection::new(PointUtf16::new(0, 1), PointUtf16::new(1, 3)))
        );
        // The ellipsis is the start of the line after it.
        let selection = accesskit::TextSelection {
            anchor: position(9, 1),
            focus: position(9, 1),
        };
        assert_eq!(
            selection_from_a11y(&lines, &rows, &selection),
            Some(Selection::collapsed(PointUtf16::new(1, 0)))
        );
        // Unknown nodes are ignored.
        let selection = accesskit::TextSelection {
            anchor: position(10, 0),
            focus: position(10, 0),
        };
        assert_eq!(selection_from_a11y(&lines, &rows, &selection), None);
    }

    #[test]
    fn columns_and_characters() {
        assert_eq!(char_index("a😀b", 3), 2);
        assert_eq!(utf16_column("a😀b", 2), 3);
        assert_eq!(char_index("ab", 9), 2);
    }
}
