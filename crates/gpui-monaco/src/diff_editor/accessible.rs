//! The accessible diff viewer (Monaco `accessibleDiffViewer.ts`, MIT): a
//! keyboard-driven list of each change's lines, for screen reader users.
//! F7 and Shift+F7 move between changes, Up and Down between lines, Enter
//! or Space reveals the line in the editor and Escape closes it.

use gpui::{
    App, Context, Entity, FocusHandle, Focusable, InteractiveElement as _, IntoElement,
    KeyDownEvent, ParentElement as _, Render, ScrollHandle, SharedString,
    StatefulInteractiveElement as _, Styled as _, WeakEntity, Window, div, px,
};
use text::PointUtf16;

use crate::codicons::{Codicon, icon as codicon};
use crate::cursor::Selection;
use crate::diff::{LineRange, LineRangeMapping};
use crate::editor::CodeEditor;
use crate::view::reveal::RevealType;

use super::DiffEditor;

/// Unchanged lines shown around each group (Monaco
/// `viewElementGroupLineMargin`).
const LINE_MARGIN: usize = 3;

/// A row of the viewer (Monaco `ViewElement`), with 1-based line numbers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ViewElement {
    /// The group's summary row.
    Header,
    Unchanged {
        original: usize,
        modified: usize,
    },
    /// A deleted line; `modified` is where its change starts.
    Deleted {
        original: usize,
        modified: usize,
    },
    Added {
        modified: usize,
    },
}

/// Changes close together, with the lines around them (Monaco
/// `ViewElementGroup`).
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct ViewElementGroup {
    pub range: LineRangeMapping,
    pub lines: Vec<ViewElement>,
}

/// Monaco `computeViewElementGroups`: changes less than two margins apart
/// form a group, shown with `LINE_MARGIN` lines of context.
pub(crate) fn compute_view_element_groups(
    diffs: &[LineRangeMapping],
    original_line_count: usize,
    modified_line_count: usize,
) -> Vec<ViewElementGroup> {
    let mut result = Vec::new();
    let mut start = 0;
    while start < diffs.len() {
        let mut end = start + 1;
        while end < diffs.len()
            && diffs[end].modified.start_line_number
                < diffs[end - 1].modified.end_line_number_exclusive + 2 * LINE_MARGIN
        {
            end += 1;
        }
        let group = &diffs[start..end];
        start = end;
        let (first, last) = (&group[0], &group[group.len() - 1]);
        let original_full = LineRange::new(
            first
                .original
                .start_line_number
                .saturating_sub(LINE_MARGIN)
                .max(1),
            (last.original.end_line_number_exclusive + LINE_MARGIN).min(original_line_count + 1),
        );
        let modified_full = LineRange::new(
            first
                .modified
                .start_line_number
                .saturating_sub(LINE_MARGIN)
                .max(1),
            (last.modified.end_line_number_exclusive + LINE_MARGIN).min(modified_line_count + 1),
        );
        let mut lines = vec![ViewElement::Header];
        // Monaco `forEachAdjacent`: the unchanged lines before, between and
        // after the changes, and each change's lines.
        for i in 0..=group.len() {
            let a = i.checked_sub(1).map(|i| &group[i]);
            let b = group.get(i);
            let original_start = a.map_or(original_full.start_line_number, |a| {
                a.original.end_line_number_exclusive
            });
            let original_end = b.map_or(original_full.end_line_number_exclusive, |b| {
                b.original.start_line_number
            });
            let modified_start = a.map_or(modified_full.start_line_number, |a| {
                a.modified.end_line_number_exclusive
            });
            for original in original_start..original_end {
                lines.push(ViewElement::Unchanged {
                    original,
                    modified: modified_start + (original - original_start),
                });
            }
            if let Some(b) = b {
                for original in b.original.start_line_number..b.original.end_line_number_exclusive {
                    lines.push(ViewElement::Deleted {
                        original,
                        modified: b.modified.start_line_number,
                    });
                }
                for modified in b.modified.start_line_number..b.modified.end_line_number_exclusive {
                    lines.push(ViewElement::Added { modified });
                }
            }
        }
        result.push(ViewElementGroup {
            range: LineRangeMapping::new(
                first.original.join(&last.original),
                first.modified.join(&last.modified),
            ),
            lines,
        });
    }
    result
}

/// Monaco's ARIA label for a changed line count.
fn lines_changed(lines: usize) -> String {
    match lines {
        0 => "no lines changed".into(),
        1 => "1 line changed".into(),
        n => format!("{n} lines changed"),
    }
}

/// The viewer (Monaco `AccessibleDiffViewer`'s view model and view).
pub(crate) struct AccessibleDiffViewer {
    diff: WeakEntity<DiffEditor>,
    original: Entity<CodeEditor>,
    modified: Entity<CodeEditor>,
    groups: Vec<ViewElementGroup>,
    current_group: usize,
    current_element: usize,
    /// Monaco `canClose`: false with `only_show_accessible_diff_viewer`.
    can_close: bool,
    focus_handle: FocusHandle,
    scroll: ScrollHandle,
}

impl Focusable for AccessibleDiffViewer {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl AccessibleDiffViewer {
    pub(crate) fn new(
        diff: WeakEntity<DiffEditor>,
        original: Entity<CodeEditor>,
        modified: Entity<CodeEditor>,
        can_close: bool,
        cx: &mut Context<Self>,
    ) -> Self {
        Self {
            diff,
            original,
            modified,
            groups: Vec::new(),
            current_group: 0,
            current_element: 0,
            can_close,
            focus_handle: cx.focus_handle(),
            scroll: ScrollHandle::new(),
        }
    }

    /// New changes: the groups again, starting at the one at or after the
    /// modified editor's cursor.
    pub(crate) fn set_diffs(&mut self, diffs: &[LineRangeMapping], cx: &mut Context<Self>) {
        let original_count = self.original.read(cx).model().read(cx).line_count() as usize;
        let modified_count = self.modified.read(cx).model().read(cx).line_count() as usize;
        self.groups = compute_view_element_groups(diffs, original_count, modified_count);
        let line = self.modified.read(cx).selections()[0].head.row as usize + 1;
        if let Some(ix) = self
            .groups
            .iter()
            .position(|g| line < g.range.modified.end_line_number_exclusive)
        {
            self.current_group = ix;
        }
        self.current_group = self.current_group.min(self.groups.len().saturating_sub(1));
        self.current_element = 0;
        cx.notify();
    }

    fn current(&self) -> Option<ViewElement> {
        self.groups
            .get(self.current_group)?
            .lines
            .get(self.current_element)
            .copied()
    }

    /// Monaco `_goToGroupDelta`: cycles through the groups.
    pub(crate) fn go_to_group_delta(&mut self, delta: isize, cx: &mut Context<Self>) {
        let len = self.groups.len() as isize;
        if len <= 1 {
            return;
        }
        self.current_group = (self.current_group as isize + delta).rem_euclid(len) as usize;
        self.current_element = 0;
        self.scroll.scroll_to_item(0);
        self.element_changed(cx);
    }

    /// Monaco `_goToLineDelta`: moves within the group, stopping at its
    /// ends.
    fn go_to_line_delta(&mut self, delta: isize, cx: &mut Context<Self>) {
        let Some(group) = self.groups.get(self.current_group) else {
            return;
        };
        if group.lines.len() <= 1 {
            return;
        }
        self.current_element = (self.current_element as isize + delta)
            .clamp(0, group.lines.len() as isize - 1) as usize;
        self.scroll.scroll_to_item(self.current_element);
        self.element_changed(cx);
    }

    /// Monaco's "select lines in editor": the modified cursor follows the
    /// current line, so editor commands like revert apply to it.
    fn element_changed(&mut self, cx: &mut Context<Self>) {
        // Monaco's "play audio-cue for diff".
        let signal = match self.current() {
            Some(ViewElement::Deleted { .. }) => Some(super::AccessibilitySignal::DiffLineDeleted),
            Some(ViewElement::Added { .. }) => Some(super::AccessibilitySignal::DiffLineInserted),
            _ => None,
        };
        if let (Some(signal), Some(diff)) = (signal, self.diff.upgrade()) {
            cx.defer(move |cx| diff.update(cx, |diff, cx| diff.play_signal(signal, cx)));
        }
        let line = match self.current() {
            Some(ViewElement::Unchanged { modified, .. })
            | Some(ViewElement::Deleted { modified, .. })
            | Some(ViewElement::Added { modified }) => modified,
            _ => {
                cx.notify();
                return;
            }
        };
        let point = PointUtf16::new(line as u32 - 1, 0);
        self.modified.update(cx, |editor, cx| {
            editor.set_selections(&[Selection::collapsed(point)], cx)
        });
        cx.notify();
    }

    /// Monaco `revealCurrentElementInEditor`: closes the viewer and shows
    /// the line in its editor.
    fn reveal_current(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.can_close {
            return;
        }
        let current = self.current();
        let (editor, line) = match current {
            Some(ViewElement::Deleted { original, .. }) => (self.original.clone(), Some(original)),
            Some(ViewElement::Unchanged { modified, .. })
            | Some(ViewElement::Added { modified }) => (self.modified.clone(), Some(modified)),
            _ => (self.modified.clone(), None),
        };
        self.close_viewer(cx);
        if let Some(line) = line {
            let point = PointUtf16::new(line as u32 - 1, 0);
            editor.update(cx, |editor, cx| {
                editor.set_selections(&[Selection::collapsed(point)], cx);
                editor.reveal_range(point..point, RevealType::Simple, cx);
            });
        }
        window.focus(&editor.read(cx).focus_handle(cx), cx);
    }

    /// Monaco `close`: hides the viewer and focuses the modified editor.
    fn close(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.can_close {
            return;
        }
        self.close_viewer(cx);
        window.focus(&self.modified.read(cx).focus_handle(cx), cx);
    }

    fn close_viewer(&self, cx: &mut Context<Self>) {
        if let Some(diff) = self.diff.upgrade() {
            cx.defer(move |cx| {
                diff.update(cx, |diff, cx| diff.set_accessible_viewer_visible(false, cx))
            });
        }
    }

    fn on_key_down(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let keystroke = &event.keystroke;
        let modifiers = keystroke.modifiers;
        let plain = !modifiers.shift && !modifiers.control && !modifiers.platform && !modifiers.alt;
        // Monaco also accepts Ctrl/Cmd and Alt with the arrows and Escape.
        let arrow_modifiers = !modifiers.shift;
        match keystroke.key.as_str() {
            "down" if arrow_modifiers => self.go_to_line_delta(1, cx),
            "up" if arrow_modifiers => self.go_to_line_delta(-1, cx),
            "escape" => self.close(window, cx),
            "enter" | "space" if plain => self.reveal_current(window, cx),
            _ => return,
        }
        cx.stop_propagation();
    }

    /// The groups, for tests.
    pub(crate) fn state_for_tests(&self) -> (usize, usize, usize) {
        (self.groups.len(), self.current_group, self.current_element)
    }

    pub(crate) fn current_for_tests(&self) -> Option<ViewElement> {
        self.current()
    }
}

impl Render for AccessibleDiffViewer {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let modified_editor = self.modified.read(cx);
        let colors = modified_editor.colors().clone();
        let options = modified_editor.options().clone();
        let line_height = options.resolved_line_height();
        let numbers_width = |editor: &CodeEditor| {
            editor
                .layout
                .as_ref()
                .map_or(px(40.), |l| l.glyph_margin_width + l.line_numbers_width)
        };
        let original_numbers_width = numbers_width(self.original.read(cx));
        let modified_numbers_width = px(10.) + numbers_width(modified_editor);
        let original_rope = self
            .original
            .read(cx)
            .model()
            .read(cx)
            .buffer()
            .rope()
            .clone();
        let modified_rope = modified_editor.model().read(cx).buffer().rope().clone();
        let tab_size = modified_editor.model().read(cx).options().tab_size.max(1) as usize;
        let line_text = |rope: &text::Rope, line: usize| {
            if line >= 1 && line <= rope.line_count() as usize {
                rope.line(line as u32 - 1)
            } else {
                String::new()
            }
        };
        let group_count = self.groups.len();
        let em_width = modified_editor
            .layout
            .as_ref()
            .map_or(px(8.), |l| l.metrics.em_width);
        // Rows are as wide as the longest line, so the list scrolls
        // sideways (Monaco's `DomScrollableElement`).
        let longest = self
            .groups
            .get(self.current_group)
            .map(|group| {
                group
                    .lines
                    .iter()
                    .map(|element| match *element {
                        ViewElement::Unchanged { modified, .. }
                        | ViewElement::Added { modified } => line_text(&modified_rope, modified),
                        ViewElement::Deleted { original, .. } => {
                            line_text(&original_rope, original)
                        }
                        ViewElement::Header => String::new(),
                    })
                    .map(|text| {
                        crate::colorize::expand_tabs(&text, tab_size)
                            .0
                            .chars()
                            .count()
                    })
                    .max()
                    .unwrap_or(0)
            })
            .unwrap_or(0);
        let row_width = original_numbers_width
            + modified_numbers_width
            + px(10. + 14. + 20.)
            + em_width * longest as f32;
        let mut rows = Vec::new();
        if let Some(group) = self.groups.get(self.current_group) {
            for (ix, element) in group.lines.iter().enumerate() {
                let selected = ix == self.current_element;
                let row = match *element {
                    ViewElement::Header => {
                        let r = &group.range;
                        let label = format!(
                            "Difference {} of {}: original line {}, {}, modified line {}, {}",
                            self.current_group + 1,
                            group_count,
                            r.original.start_line_number,
                            lines_changed(r.original.len()),
                            r.modified.start_line_number,
                            lines_changed(r.modified.len()),
                        );
                        // e.g. `1/10: @@ -504,7 +517,7 @@`
                        let summary = format!(
                            "{}/{}: @@ -{},{} +{},{} @@",
                            self.current_group + 1,
                            group_count,
                            r.original.start_line_number,
                            r.original.len(),
                            r.modified.start_line_number,
                            r.modified.len(),
                        );
                        div()
                            .id(("diff-review-row", ix))
                            .h(line_height)
                            .pl(px(10.))
                            .flex()
                            .items_center()
                            .role(gpui::Role::ListItem)
                            .aria_label(label)
                            .child(summary)
                    }
                    element => {
                        let (original, modified, kind) = match element {
                            ViewElement::Unchanged { original, modified } => {
                                (Some(original), Some(modified), 0)
                            }
                            ViewElement::Deleted { original, .. } => (Some(original), None, -1),
                            ViewElement::Added { modified } => (None, Some(modified), 1),
                            ViewElement::Header => unreachable!(),
                        };
                        let content = match modified {
                            Some(line) if kind >= 0 => line_text(&modified_rope, line),
                            _ => line_text(&original_rope, original.unwrap_or(1)),
                        };
                        let spoken = if content.is_empty() {
                            "blank".to_string()
                        } else {
                            content.clone()
                        };
                        let label = match (kind, original, modified) {
                            (0, Some(o), Some(m)) if o == m => {
                                format!("{spoken} unchanged line {o}")
                            }
                            (0, Some(o), Some(m)) => {
                                format!("{spoken} original line {o} modified line {m}")
                            }
                            (1, _, Some(m)) => format!("+ {spoken} modified line {m}"),
                            (_, Some(o), _) => format!("- {spoken} original line {o}"),
                            _ => spoken,
                        };
                        let (text, _) = crate::colorize::expand_tabs(&content, tab_size);
                        let (row_background, number_background, sign) = match kind {
                            1 => (
                                Some(colors.diff_inserted_line_background),
                                Some(colors.diff_inserted_text_background),
                                Some(Codicon::Add),
                            ),
                            -1 => (
                                Some(colors.diff_removed_line_background),
                                Some(colors.diff_removed_text_background),
                                Some(Codicon::Remove),
                            ),
                            _ => (None, None, None),
                        };
                        let number = |line: Option<usize>, width| {
                            let mut cell = div()
                                .w(width)
                                .flex_none()
                                .flex()
                                .justify_end()
                                .text_color(colors.line_number)
                                .child(line.map_or(String::new(), |l| l.to_string()));
                            if let Some(background) = number_background {
                                cell = cell.bg(background);
                            }
                            cell
                        };
                        let mut row = div()
                            .id(("diff-review-row", ix))
                            .h(line_height)
                            .flex()
                            .flex_row()
                            .items_center()
                            .whitespace_nowrap()
                            .role(gpui::Role::ListItem)
                            .aria_label(label)
                            .child(number(original, original_numbers_width))
                            .child(number(modified, modified_numbers_width).pr(px(10.)))
                            .child(
                                div()
                                    .w(px(14.))
                                    .flex_none()
                                    .flex()
                                    .items_center()
                                    .children(sign.map(|sign| codicon(sign).size(px(14.)))),
                            )
                            .child(text);
                        if let Some(background) = row_background {
                            row = row.bg(background);
                        }
                        row
                    }
                };
                let row = row.min_w(row_width).flex_none();
                let row = if selected {
                    row.border_1().border_color(colors.focus_border)
                } else {
                    row.border_1().border_color(gpui::transparent_black())
                };
                rows.push(row.on_click(cx.listener(move |this, _, _, cx| {
                    this.current_element = ix;
                    this.element_changed(cx);
                })));
            }
        }
        div()
            .id("accessible-diff-viewer")
            .key_context("AccessibleDiffViewer")
            .track_focus(&self.focus_handle)
            .on_key_down(cx.listener(Self::on_key_down))
            .absolute()
            .size_full()
            // Monaco hides the editors (`visibility: hidden`) while the
            // viewer shows: nothing reaches them through it.
            .occlude()
            .on_scroll_wheel(|_, _, cx| cx.stop_propagation())
            .bg(colors.background)
            .text_color(colors.foreground)
            .font_family(options.font_family.clone())
            .text_size(options.font_size)
            .line_height(line_height)
            .child(
                div()
                    .id("diff-review-table")
                    .size_full()
                    .flex()
                    .flex_col()
                    .overflow_scroll()
                    .track_scroll(&self.scroll)
                    .role(gpui::Role::List)
                    .aria_label(SharedString::from(
                        "Accessible Diff Viewer. Use arrow up and down to navigate.",
                    ))
                    .children(rows),
            )
            .when(self.can_close, |viewer| {
                viewer.child(
                    div()
                        .id("diff-review-close")
                        .tooltip(crate::tooltip::tooltip("Close", &colors))
                        .absolute()
                        .top(px(2.))
                        .right(px(10.))
                        .px_1()
                        .cursor_pointer()
                        .rounded(px(3.))
                        .hover(|style| style.bg(colors.widget_border))
                        .role(gpui::Role::Button)
                        .aria_label(SharedString::from("Close"))
                        .on_click(cx.listener(|this, _, window, cx| this.close(window, cx)))
                        // `accessible-diff-viewer-close`: `close`.
                        .child(codicon(Codicon::Close)),
                )
            })
    }
}

use gpui::prelude::FluentBuilder as _;

#[cfg(test)]
mod tests {
    use super::*;

    fn mapping(original: (usize, usize), modified: (usize, usize)) -> LineRangeMapping {
        LineRangeMapping::new(
            LineRange::new(original.0, original.1),
            LineRange::new(modified.0, modified.1),
        )
    }

    #[test]
    fn groups_show_changes_with_context() {
        // Line 5 replaced with two lines, in a 10-line text.
        let groups = compute_view_element_groups(&[mapping((5, 6), (5, 7))], 10, 11);
        assert_eq!(groups.len(), 1);
        use ViewElement::*;
        assert_eq!(
            groups[0].lines,
            vec![
                Header,
                Unchanged {
                    original: 2,
                    modified: 2
                },
                Unchanged {
                    original: 3,
                    modified: 3
                },
                Unchanged {
                    original: 4,
                    modified: 4
                },
                Deleted {
                    original: 5,
                    modified: 5
                },
                Added { modified: 5 },
                Added { modified: 6 },
                Unchanged {
                    original: 6,
                    modified: 7
                },
                Unchanged {
                    original: 7,
                    modified: 8
                },
                Unchanged {
                    original: 8,
                    modified: 9
                },
            ]
        );
        assert_eq!(groups[0].range.original.to_string(), "[5,6)");
        assert_eq!(groups[0].range.modified.to_string(), "[5,7)");
    }

    #[test]
    fn nearby_changes_share_a_group() {
        let diffs = [
            mapping((2, 3), (2, 3)),
            // Less than 6 lines after the first: same group.
            mapping((7, 8), (7, 8)),
            // Far away: its own group, its context clipped at the end.
            mapping((30, 31), (30, 31)),
        ];
        let groups = compute_view_element_groups(&diffs, 31, 31);
        assert_eq!(groups.len(), 2);
        // Context before line 2 starts at line 1, and unchanged lines 3-6
        // sit between the changes.
        let unchanged = groups[0]
            .lines
            .iter()
            .filter(|l| matches!(l, ViewElement::Unchanged { .. }))
            .count();
        assert_eq!(unchanged, 1 + 4 + 3);
        assert_eq!(
            groups[1].lines.last(),
            Some(&ViewElement::Unchanged {
                original: 31,
                modified: 31
            })
        );
    }
}
