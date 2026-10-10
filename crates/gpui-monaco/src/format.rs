//! Formatting: format document and selection, format on type and on paste.
//!
//! Ports Monaco's `src/vs/editor/contrib/format/browser/format.ts`,
//! `formatActions.ts` and `formattingEdit.ts`, and the worker's
//! `computeMoreMinimalEdits` (`editorWebWorker.ts`) (MIT).

use std::ops::Range;

use gpui::{Context, Task};
use text::{Bias, PointUtf16, Rope};

use crate::editor::CodeEditor;
use crate::providers::{
    FormattingOptions, TextEdit, document_formatting_providers,
    document_range_formatting_providers, on_type_formatting_providers,
};
use crate::view::reveal::RevealType;

/// Monaco `EditorWorker._diffLimit`: longer edits aren't diffed.
const DIFF_LIMIT: usize = 100_000;

/// Monaco `computeMoreMinimalEdits`: replaces each edit with the
/// character-level differences between the text it replaces and its new
/// text, so cursors and decorations in unchanged text stay put. Adjacent
/// edits are merged first and no-op edits dropped.
pub fn minimal_edits(rope: &Rope, mut edits: Vec<TextEdit>) -> Vec<TextEdit> {
    edits.sort_by_key(|(range, _)| range.start);
    // Merge edits that touch.
    let mut merged: Vec<TextEdit> = Vec::with_capacity(edits.len());
    for (range, text) in edits {
        match merged.last_mut() {
            Some((last, last_text)) if last.end == range.start => {
                last.end = range.end;
                last_text.push_str(&text);
            }
            _ => merged.push((range, text)),
        }
    }
    let mut result = Vec::new();
    for (range, text) in merged {
        if range.start == range.end && text.is_empty() {
            continue;
        }
        let start = rope.point_utf16_to_offset(rope.clip_point_utf16(range.start, Bias::Left));
        let end = rope.point_utf16_to_offset(rope.clip_point_utf16(range.end, Bias::Left));
        let original = rope.text_for_range(start..end);
        let text = normalize_line_breaks(&text);
        if original == text {
            continue;
        }
        if original.len().max(text.len()) > DIFF_LIMIT {
            result.push((range, text));
            continue;
        }
        for (old, new) in char_changes(&original, &text) {
            let old = start + old.start..start + old.end;
            result.push((
                rope.offset_to_point_utf16(old.start)..rope.offset_to_point_utf16(old.end),
                text[new].to_string(),
            ));
        }
    }
    result
}

fn normalize_line_breaks(text: &str) -> String {
    if text.contains('\r') {
        text.replace("\r\n", "\n").replace('\r', "\n")
    } else {
        text.to_string()
    }
}

/// The changed byte ranges between `old` and `new` (Monaco `stringDiff`):
/// runs of deleted and inserted characters, as (old range, new range).
fn char_changes(old: &str, new: &str) -> Vec<(Range<usize>, Range<usize>)> {
    let old_chars: Vec<char> = old.chars().collect();
    let new_chars: Vec<char> = new.chars().collect();
    let offsets = |s: &str| -> Vec<usize> {
        s.char_indices()
            .map(|(i, _)| i)
            .chain(std::iter::once(s.len()))
            .collect()
    };
    let (old_offsets, new_offsets) = (offsets(old), offsets(new));
    let mut changes: Vec<(Range<usize>, Range<usize>)> = Vec::new();
    for op in similar::capture_diff_slices(similar::Algorithm::Myers, &old_chars, &new_chars) {
        let (old_range, new_range) = match op {
            similar::DiffOp::Equal { .. } => continue,
            similar::DiffOp::Delete {
                old_index,
                old_len,
                new_index,
            } => (old_index..old_index + old_len, new_index..new_index),
            similar::DiffOp::Insert {
                old_index,
                new_index,
                new_len,
            } => (old_index..old_index, new_index..new_index + new_len),
            similar::DiffOp::Replace {
                old_index,
                old_len,
                new_index,
                new_len,
            } => (
                old_index..old_index + old_len,
                new_index..new_index + new_len,
            ),
        };
        let old_range = old_offsets[old_range.start]..old_offsets[old_range.end];
        let new_range = new_offsets[new_range.start]..new_offsets[new_range.end];
        match changes.last_mut() {
            Some((last_old, last_new))
                if last_old.end == old_range.start && last_new.end == new_range.start =>
            {
                last_old.end = old_range.end;
                last_new.end = new_range.end;
            }
            _ => changes.push((old_range, new_range)),
        }
    }
    changes
}

/// Merges ranges that intersect or touch (Monaco formats each merged range
/// once).
fn merge_ranges(mut ranges: Vec<Range<PointUtf16>>) -> Vec<Range<PointUtf16>> {
    ranges.sort_by_key(|range| range.start);
    let mut merged: Vec<Range<PointUtf16>> = Vec::new();
    for range in ranges {
        match merged.last_mut() {
            Some(last) if range.start <= last.end => last.end = last.end.max(range.end),
            _ => merged.push(range),
        }
    }
    merged
}

fn edits_intersect(a: &[TextEdit], b: &[TextEdit]) -> bool {
    a.iter()
        .any(|(a, _)| b.iter().any(|(b, _)| a.start <= b.end && b.start <= a.end))
}

impl CodeEditor {
    /// The model's indentation, for formatters.
    pub fn formatting_options(&self, cx: &gpui::App) -> FormattingOptions {
        let options = self.model().read(cx).options();
        FormattingOptions {
            tab_size: options.tab_size,
            insert_spaces: options.insert_spaces,
        }
    }

    /// Formats the document with the first document formatter, or the
    /// first range formatter over the whole text (Monaco
    /// `editor.action.formatDocument`). Returns whether there was a
    /// formatter. Typing or moving the cursor before the result arrives
    /// cancels it.
    pub fn format_document(&mut self, cx: &mut Context<Self>) -> bool {
        if self.options.read_only {
            return false;
        }
        let model = self.model().read(cx);
        let language = model.language_id().map(|l| l.to_string());
        let context = model.provider_context();
        let options = self.formatting_options(cx);
        if let Some(provider) = document_formatting_providers(cx, language.as_deref())
            .into_iter()
            .next()
        {
            let task = provider.provide_document_formatting_edits(&context, options, cx);
            self.finish_format(vec![task], true, cx);
            return true;
        }
        let full = PointUtf16::zero()..self.snapshot.max_point_utf16();
        self.format_ranges(vec![full], true, cx)
    }

    /// Formats the selections with the first range formatter; an empty
    /// selection formats its line (Monaco `editor.action.formatSelection`).
    /// Returns whether there was a formatter.
    pub fn format_selection(&mut self, cx: &mut Context<Self>) -> bool {
        if self.options.read_only {
            return false;
        }
        let ranges = self
            .cursors
            .selections()
            .into_iter()
            .map(|selection| {
                let range = selection.range();
                if range.start == range.end {
                    let row = range.start.row;
                    PointUtf16::new(row, 0)
                        ..PointUtf16::new(row, self.snapshot.rope().line_len_utf16(row))
                } else {
                    range
                }
            })
            .collect();
        self.format_ranges(ranges, true, cx)
    }

    /// Monaco `formatDocumentRangesWithProvider`: formats each merged range,
    /// and re-formats ranges whose edits would collide as one.
    fn format_ranges(
        &mut self,
        ranges: Vec<Range<PointUtf16>>,
        explicit: bool,
        cx: &mut Context<Self>,
    ) -> bool {
        let model = self.model().read(cx);
        let language = model.language_id().map(|l| l.to_string());
        let Some(provider) = document_range_formatting_providers(cx, language.as_deref())
            .into_iter()
            .next()
        else {
            return false;
        };
        let context = model.provider_context();
        let options = self.formatting_options(cx);
        let mut ranges = merge_ranges(ranges);
        let requests: Vec<Task<anyhow::Result<Vec<TextEdit>>>> = ranges
            .iter()
            .map(|range| {
                provider.provide_document_range_formatting_edits(
                    &context,
                    range.clone(),
                    options,
                    cx,
                )
            })
            .collect();
        let state = self.format_state(cx);
        self.format_task = Some(cx.spawn(async move |this, cx| {
            let mut results = Vec::new();
            for request in requests {
                match request.await {
                    Ok(edits) => results.push(edits),
                    Err(err) => {
                        log::warn!("formatting failed: {err:#}");
                        return;
                    }
                }
            }
            // Ranges whose edits collide are formatted together.
            'merge: loop {
                for i in 0..results.len() {
                    for j in i + 1..results.len() {
                        if !edits_intersect(&results[i], &results[j]) {
                            continue;
                        }
                        let merged =
                            ranges[i].start.min(ranges[j].start)..ranges[i].end.max(ranges[j].end);
                        let Ok(request) = this.update(cx, |_, cx| {
                            provider.provide_document_range_formatting_edits(
                                &context,
                                merged.clone(),
                                options,
                                cx,
                            )
                        }) else {
                            return;
                        };
                        let Ok(edits) = request.await else {
                            return;
                        };
                        ranges.remove(j);
                        ranges.remove(i);
                        results.remove(j);
                        results.remove(i);
                        ranges.push(merged);
                        results.push(edits);
                        continue 'merge;
                    }
                }
                break;
            }
            this.update(cx, |this, cx| {
                if this.format_state(cx) == state {
                    this.apply_format_edits(results.concat(), explicit, cx);
                }
                this.format_task = None;
            })
            .ok();
        }));
        true
    }

    /// Applies the results of `tasks` unless the text or cursors changed
    /// meanwhile (Monaco's `EditorStateCancellationTokenSource` with `Value |
    /// Position`).
    fn finish_format(
        &mut self,
        tasks: Vec<Task<anyhow::Result<Vec<TextEdit>>>>,
        explicit: bool,
        cx: &mut Context<Self>,
    ) {
        let state = self.format_state(cx);
        self.format_task = Some(cx.spawn(async move |this, cx| {
            let mut edits = Vec::new();
            for task in tasks {
                match task.await {
                    Ok(result) => edits.extend(result),
                    Err(err) => {
                        log::warn!("formatting failed: {err:#}");
                        return;
                    }
                }
            }
            this.update(cx, |this, cx| {
                if this.format_state(cx) == state {
                    this.apply_format_edits(edits, explicit, cx);
                }
                this.format_task = None;
            })
            .ok();
        }));
    }

    fn format_state(&self, cx: &gpui::App) -> (text::Version, Vec<crate::cursor::Selection>) {
        (
            self.model().read(cx).buffer().version(),
            self.cursors.selections(),
        )
    }

    /// Monaco `FormattingEdit.execute`: one undo step, cursors following
    /// the edits, the primary cursor kept at the same height on screen.
    fn apply_format_edits(&mut self, edits: Vec<TextEdit>, explicit: bool, cx: &mut Context<Self>) {
        let edits = minimal_edits(self.snapshot.rope(), edits);
        if edits.is_empty() || self.options.read_only {
            return;
        }
        self.keep_cursor_on_screen();
        self.push_undo_stop(cx);
        self.execute_edits(edits, None, cx);
        self.push_undo_stop(cx);
        if explicit {
            let head = self.cursors.primary().position;
            self.reveal_range(head..head, RevealType::CenterIfOutsideViewport, cx);
        }
    }

    /// Monaco `StableEditorScrollState`: the next frame scrolls so the
    /// primary cursor's line stays where it is on screen.
    fn keep_cursor_on_screen(&mut self) {
        self.pending_cursor_offset = self.last_cursor_offset;
    }

    /// After typing `ch`: formats with the first on-type formatter whose
    /// trigger characters include it, with the `format_on_type` option
    /// (Monaco `FormatOnType`). Only with a single empty selection; edits
    /// at or above the cursor's line before the result cancel it.
    pub(crate) fn format_on_type(&mut self, ch: char, cx: &mut Context<Self>) {
        if !self.options.format_on_type || self.options.read_only {
            return;
        }
        let selections = self.cursors.selections();
        if selections.len() > 1 || !selections[0].is_empty() {
            return;
        }
        let model = self.model().read(cx);
        let language = model.language_id().map(|l| l.to_string());
        let Some(provider) = on_type_formatting_providers(cx, language.as_deref())
            .into_iter()
            .next()
            .filter(|p| p.auto_format_trigger_characters().contains(&ch))
        else {
            return;
        };
        let context = model.provider_context();
        let position = selections[0].head;
        let task = provider.provide_on_type_formatting_edits(
            &context,
            position,
            ch,
            self.formatting_options(cx),
            cx,
        );
        let requested = context.snapshot;
        self.format_on_type_task = Some(cx.spawn(async move |this, cx| {
            let edits = match task.await {
                Ok(edits) => edits,
                Err(err) => {
                    log::warn!("formatting failed: {err:#}");
                    return;
                }
            };
            this.update(cx, |this, cx| {
                this.format_on_type_task = None;
                let buffer = this.model().read(cx).buffer();
                let current = buffer.snapshot();
                let edits = if current.version() == requested.version() {
                    edits
                } else {
                    // Monaco cancels when a change ends at or above the
                    // cursor's line; edits below are mapped through.
                    let patch = buffer.edits_since(requested.version());
                    let changes =
                        crate::events::content_changes(requested.rope(), current.rope(), &patch);
                    if changes.iter().any(|c| c.range.end.row <= position.row) {
                        return;
                    }
                    let map = |p: PointUtf16, bias| {
                        let offset = requested.point_utf16_to_offset(p);
                        current.offset_to_point_utf16(patch.transform_offset(offset, bias))
                    };
                    edits
                        .into_iter()
                        .map(|(range, text)| {
                            (
                                map(range.start, Bias::Left)..map(range.end, Bias::Right),
                                text,
                            )
                        })
                        .collect()
                };
                this.apply_format_edits(edits, false, cx);
            })
            .ok();
        }));
    }

    /// After a paste of `range`: formats it with the first range formatter,
    /// with the `format_on_paste` option (Monaco `FormatOnPaste`).
    pub(crate) fn format_on_paste(&mut self, range: Range<PointUtf16>, cx: &mut Context<Self>) {
        if self.options.format_on_paste && !self.options.read_only {
            self.format_ranges(vec![range], false, cx);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(row: u32, column: u32) -> PointUtf16 {
        PointUtf16::new(row, column)
    }

    #[test]
    fn minimal_edits_keep_unchanged_text() {
        let rope = Rope::from("fn  main( ){\nlet x=1;\n}");
        // A whole-document replacement becomes the few characters that
        // actually change.
        let edits = minimal_edits(
            &rope,
            vec![(p(0, 0)..p(2, 1), "fn main() {\n    let x = 1;\n}".into())],
        );
        let mut text = rope.to_string();
        for (range, new) in edits.iter().rev() {
            let start = rope.point_utf16_to_offset(range.start);
            let end = rope.point_utf16_to_offset(range.end);
            text.replace_range(start..end, new);
        }
        assert_eq!(text, "fn main() {\n    let x = 1;\n}");
        assert!(edits.len() > 1);
        assert!(edits.iter().all(|(_, new)| new.len() <= 4), "{edits:?}");
        // The first line's `fn` is untouched.
        assert!(edits.iter().all(|(range, _)| range.start >= p(0, 2)));
    }

    #[test]
    fn minimal_edits_merge_and_skip_noops() {
        let rope = Rope::from("abc");
        // Touching edits merge; an identical replacement disappears.
        assert_eq!(
            minimal_edits(
                &rope,
                vec![
                    (p(0, 1)..p(0, 2), "x".into()),
                    (p(0, 0)..p(0, 1), "a".into())
                ]
            ),
            vec![(p(0, 1)..p(0, 2), "x".to_string())]
        );
        assert!(minimal_edits(&rope, vec![(p(0, 0)..p(0, 3), "abc".into())]).is_empty());
        assert!(minimal_edits(&rope, vec![(p(0, 1)..p(0, 1), String::new())]).is_empty());
        // CRLF in new text matches the rope's line breaks.
        let rope = Rope::from("a\nb");
        assert!(minimal_edits(&rope, vec![(p(0, 0)..p(1, 1), "a\r\nb".into())]).is_empty());
        // Multi-byte characters stay whole.
        let rope = Rope::from("é😀x");
        assert_eq!(
            minimal_edits(&rope, vec![(p(0, 0)..p(0, 4), "é😀y".into())]),
            vec![(p(0, 3)..p(0, 4), "y".to_string())]
        );
    }

    #[test]
    fn ranges_merge_when_touching() {
        assert_eq!(
            merge_ranges(vec![p(2, 0)..p(3, 0), p(0, 0)..p(1, 0), p(1, 0)..p(1, 5)]),
            vec![p(0, 0)..p(1, 5), p(2, 0)..p(3, 0)]
        );
    }
}
