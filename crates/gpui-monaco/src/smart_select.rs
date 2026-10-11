//! Expand and shrink selection (Monaco `SmartSelectController`,
//! `src/vs/editor/contrib/smartSelect/browser/smartSelect.ts`, MIT).

use std::ops::Range;

use gpui::{Context, Task};
use text::PointUtf16;

use crate::cursor::Selection;
use crate::cursor::smart_select::{
    bracket_selection_ranges, merge_selection_ranges, word_selection_ranges,
};
use crate::editor::CodeEditor;
use crate::providers::selection_range_providers;

/// Monaco `SelectionRanges`: one cursor's ranges and where it is in them.
#[derive(Clone)]
struct SelectionRanges {
    index: usize,
    ranges: Vec<Range<PointUtf16>>,
}

impl SelectionRanges {
    /// Steps out (or in), skipping ranges equal to the current one.
    fn step(&self, forward: bool) -> Self {
        let mut index = self.index;
        loop {
            let next = if forward {
                index + 1
            } else {
                match index.checked_sub(1) {
                    Some(next) => next,
                    None => {
                        return Self {
                            index,
                            ..self.clone()
                        };
                    }
                }
            };
            if next >= self.ranges.len() {
                return Self {
                    index,
                    ranges: self.ranges.clone(),
                };
            }
            index = next;
            if self.ranges[index] != self.ranges[self.index] {
                return Self {
                    index,
                    ranges: self.ranges.clone(),
                };
            }
        }
    }
}

#[derive(Default)]
pub(crate) struct SmartSelectState {
    state: Option<Vec<SelectionRanges>>,
    /// The selections are being set by smart select itself.
    ignore_change: bool,
    task: Option<Task<()>>,
}

impl CodeEditor {
    /// Expands each selection to the next larger syntax node, bracket pair,
    /// word or line (Monaco `editor.action.smartSelect.expand`).
    pub fn expand_selection(&mut self, cx: &mut Context<Self>) {
        self.smart_select(true, cx);
    }

    /// Shrinks each selection back (Monaco
    /// `editor.action.smartSelect.shrink`).
    pub fn shrink_selection(&mut self, cx: &mut Context<Self>) {
        self.smart_select(false, cx);
    }

    /// Monaco `SmartSelectController.run`.
    fn smart_select(&mut self, forward: bool, cx: &mut Context<Self>) {
        if self.smart_select.state.is_some() {
            self.apply_smart_select(forward, cx);
            return;
        }
        let selections = self.cursors.selections();
        let positions: Vec<PointUtf16> = selections.iter().map(|s| s.head).collect();
        let model = self.model().read(cx);
        let context = model.provider_context();
        let tasks: Vec<_> = selection_range_providers(cx, self.provider_target(cx))
            .into_iter()
            .map(|provider| provider.provide_selection_ranges(&context, &positions, cx))
            .collect();
        self.smart_select.task = Some(cx.spawn(async move |this, cx| {
            let mut provided: Vec<Vec<Range<PointUtf16>>> = vec![Vec::new(); positions.len()];
            let mut any_provider = false;
            for task in tasks {
                match task.await {
                    // Monaco ignores results that don't match the positions.
                    Ok(ranges) if ranges.len() == positions.len() => {
                        any_provider = true;
                        for (into, ranges) in provided.iter_mut().zip(ranges) {
                            into.extend(ranges);
                        }
                    }
                    Ok(_) => {}
                    Err(err) => log::warn!("selection ranges failed: {err:#}"),
                }
            }
            this.update(cx, |this, cx| {
                this.smart_select.task = None;
                if this.cursors.selections() != selections {
                    return;
                }
                let state = this.compute_selection_ranges(&selections, provided, any_provider, cx);
                this.smart_select.state = Some(state);
                this.apply_smart_select(forward, cx);
            })
            .ok();
        }));
    }

    /// Monaco `provideSelectionRanges` per cursor: providers' ranges (else
    /// the syntax tree's, else brackets') plus word ranges, merged, inside
    /// the selection, with the selection itself first.
    fn compute_selection_ranges(
        &self,
        selections: &[Selection],
        provided: Vec<Vec<Range<PointUtf16>>>,
        any_provider: bool,
        cx: &gpui::App,
    ) -> Vec<SelectionRanges> {
        let model = self.model().read(cx);
        let rope = self.snapshot.rope().clone();
        let options = self.options.smart_select;
        let word_regex = model
            .language()
            .and_then(|language| language.config.word_pattern.clone());
        let word_regex = match word_regex.as_ref().and_then(syntax::Pattern::regex) {
            Some(regex) => regex.clone(),
            None => crate::cursor::in_place_replace::default_word_regex().clone(),
        };
        let syntax = model.syntax_snapshot();
        selections
            .iter()
            .zip(provided)
            .map(|(selection, mut raw)| {
                let position = selection.head;
                if !any_provider {
                    let syntax_ranges = syntax.as_ref().and_then(|syntax| {
                        let offset = rope.point_utf16_to_offset(position);
                        syntax.highlighter.selection_ranges(&syntax.state, offset)
                    });
                    match syntax_ranges {
                        Some(ranges)
                            if syntax
                                .as_ref()
                                .is_some_and(|s| s.text.version() == self.snapshot.version()) =>
                        {
                            raw.extend(ranges.into_iter().map(|range| {
                                rope.offset_to_point_utf16(range.start)
                                    ..rope.offset_to_point_utf16(range.end)
                            }));
                        }
                        _ => {
                            let brackets = self.with_bracket_context(cx, |rope, config, skip| {
                                bracket_selection_ranges(rope, config, position, skip)
                            });
                            let brackets = brackets.unwrap_or_else(|| {
                                bracket_selection_ranges(
                                    &rope,
                                    &syntax::LanguageConfiguration::default(),
                                    position,
                                    &|_| false,
                                )
                            });
                            raw.extend(brackets);
                        }
                    }
                }
                raw.extend(word_selection_ranges(
                    &rope,
                    &word_regex,
                    position,
                    options.select_subwords,
                ));
                let mut ranges = merge_selection_ranges(
                    &rope,
                    raw,
                    position,
                    options.select_leading_and_trailing_whitespace,
                );
                // Only ranges around the selection, after the selection.
                let (start, end) = (selection.start(), selection.end());
                ranges.retain(|r| r.start <= start && end <= r.end);
                ranges.insert(0, start..end);
                SelectionRanges { index: 0, ranges }
            })
            .collect()
    }

    fn apply_smart_select(&mut self, forward: bool, cx: &mut Context<Self>) {
        let Some(state) = self.smart_select.state.as_mut() else {
            return;
        };
        for ranges in state.iter_mut() {
            *ranges = ranges.step(forward);
        }
        let selections: Vec<Selection> = state
            .iter()
            .map(|s| {
                let range = &s.ranges[s.index];
                Selection::new(range.start, range.end)
            })
            .collect();
        self.smart_select.ignore_change = true;
        self.set_selections(&selections, cx);
        self.smart_select.ignore_change = false;
    }

    /// The cursor moved: forget the ranges (Monaco listens to cursor
    /// changes).
    pub(crate) fn smart_select_cursor_changed(&mut self) {
        if !self.smart_select.ignore_change {
            self.smart_select.state = None;
        }
    }
}
