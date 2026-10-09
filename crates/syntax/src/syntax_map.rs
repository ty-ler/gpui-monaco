//! [`SyntaxSnapshot`]: incrementally parsed tree-sitter trees for a buffer,
//! including injected languages (e.g. code blocks in Markdown, `<script>` in
//! HTML) as nested layers.
//!
//! Parsing is a pure function of the text and the previous snapshot, so
//! callers can run it on a background thread and swap the result in.

use std::cell::RefCell;
use std::ops::Range;
use std::sync::Arc;

use streaming_iterator::StreamingIterator;
use text::{BufferSnapshot, Patch, Point, Rope};
use tree_sitter::{InputEdit, Node, Parser, QueryCursor, TextProvider, Tree};

use crate::language::{HighlightId, Language, LanguageRegistry};

/// Injections nested deeper than this are ignored.
const MAX_INJECTION_DEPTH: usize = 4;

thread_local! {
    static PARSER: RefCell<Parser> = RefCell::new(Parser::new());
}

/// One parsed tree: the root language at depth 0, injections deeper.
#[derive(Clone)]
pub struct SyntaxLayer {
    pub language: Arc<Language>,
    pub tree: Tree,
    pub depth: usize,
    /// Byte ranges this layer was parsed from (the whole text for the root).
    pub ranges: Vec<Range<usize>>,
}

impl SyntaxLayer {
    /// The byte range spanned by this layer.
    pub fn extent(&self) -> Range<usize> {
        let start = self.ranges.first().map_or(0, |r| r.start);
        let end = self.ranges.last().map_or(0, |r| r.end);
        start..end
    }
}

/// A highlighted span of text.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HighlightSpan {
    pub range: Range<usize>,
    pub highlight: HighlightId,
}

/// The syntax trees for one version of a buffer.
#[derive(Clone)]
pub struct SyntaxSnapshot {
    layers: Vec<SyntaxLayer>,
    version: text::Version,
}

impl std::fmt::Debug for SyntaxSnapshot {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SyntaxSnapshot")
            .field("version", &self.version)
            .field("layers", &self.layers.len())
            .finish()
    }
}

/// Feeds rope text to tree-sitter queries without copying it.
pub struct RopeTextProvider<'a>(pub &'a Rope);

pub struct ByteChunks<'a>(text::Chunks<'a>);

impl<'a> Iterator for ByteChunks<'a> {
    type Item = &'a [u8];

    fn next(&mut self) -> Option<&'a [u8]> {
        self.0.next().map(str::as_bytes)
    }
}

impl<'a> TextProvider<&'a [u8]> for RopeTextProvider<'a> {
    type I = ByteChunks<'a>;

    fn text(&mut self, node: Node) -> Self::I {
        ByteChunks(self.0.chunks_in_range(node.byte_range()))
    }
}

fn ts_point(point: Point) -> tree_sitter::Point {
    tree_sitter::Point::new(point.row as usize, point.column as usize)
}

fn parse(
    language: &Language,
    rope: &Rope,
    ranges: &[Range<usize>],
    old_tree: Option<&Tree>,
) -> Option<Tree> {
    let grammar = language.grammar.as_ref()?;
    PARSER.with(|parser| {
        let mut parser = parser.borrow_mut();
        parser.set_language(&grammar.ts_language).ok()?;
        let included: Vec<tree_sitter::Range> = ranges
            .iter()
            .map(|range| tree_sitter::Range {
                start_byte: range.start,
                end_byte: range.end,
                start_point: ts_point(rope.offset_to_point(range.start)),
                end_point: ts_point(rope.offset_to_point(range.end)),
            })
            .collect();
        if included.is_empty() {
            parser.set_included_ranges(&[]).ok()?;
        } else {
            parser.set_included_ranges(&included).ok()?;
        }
        let len = rope.len();
        let mut callback = |byte: usize, _: tree_sitter::Point| -> &[u8] {
            if byte >= len {
                return &[];
            }
            rope.chunks_in_range(byte..len)
                .next()
                .map_or(&[][..], str::as_bytes)
        };
        parser.parse_with_options(&mut callback, old_tree, None)
    })
}

/// Converts a patch into tree-sitter edits, applied in order.
fn input_edits(old: &Rope, new: &Rope, patch: &Patch) -> Vec<InputEdit> {
    patch
        .edits()
        .iter()
        .map(|edit| {
            let start = edit.new.start;
            let start_point = new.offset_to_point(start);
            let old_extent = old.text_summary_for_range(edit.old.clone()).lines();
            InputEdit {
                start_byte: start,
                old_end_byte: start + edit.old_len(),
                new_end_byte: edit.new.end,
                start_position: ts_point(start_point),
                old_end_position: ts_point(start_point + old_extent),
                new_end_position: ts_point(new.offset_to_point(edit.new.end)),
            }
        })
        .collect()
}

impl SyntaxSnapshot {
    /// Parses `text` from scratch.
    pub fn parse(
        language: Arc<Language>,
        text: &BufferSnapshot,
        registry: &LanguageRegistry,
    ) -> Self {
        let mut snapshot = Self {
            layers: Vec::new(),
            version: text.version(),
        };
        snapshot.build(language, text, &[], registry);
        snapshot
    }

    /// Re-parses after edits, reusing the previous trees. `old_text` must be
    /// the text this snapshot was parsed from and `patch` the edits from it to
    /// `text` (e.g. `buffer.edits_since(old_text.version())`).
    pub fn reparse(
        &self,
        old_text: &Rope,
        text: &BufferSnapshot,
        patch: &Patch,
        registry: &LanguageRegistry,
    ) -> Self {
        let Some(root) = self.layers.first() else {
            return self.clone();
        };
        let edits = input_edits(old_text, text.rope(), patch);
        let old_layers: Vec<SyntaxLayer> = self
            .layers
            .iter()
            .map(|layer| {
                let mut layer = layer.clone();
                for edit in &edits {
                    layer.tree.edit(edit);
                }
                layer.ranges = layer
                    .ranges
                    .iter()
                    .map(|r| {
                        patch.transform_offset(r.start, text::Bias::Right)
                            ..patch.transform_offset(r.end, text::Bias::Left)
                    })
                    .collect();
                layer
            })
            .collect();
        let mut snapshot = Self {
            layers: Vec::new(),
            version: text.version(),
        };
        snapshot.build(root.language.clone(), text, &old_layers, registry);
        snapshot
    }

    fn build(
        &mut self,
        root_language: Arc<Language>,
        text: &BufferSnapshot,
        old_layers: &[SyntaxLayer],
        registry: &LanguageRegistry,
    ) {
        let rope = text.rope();
        let root_old = old_layers.first().map(|layer| &layer.tree);
        let Some(root_tree) = parse(&root_language, rope, &[], root_old) else {
            return;
        };
        self.layers.push(SyntaxLayer {
            language: root_language,
            tree: root_tree,
            depth: 0,
            ranges: std::iter::once(0..rope.len()).collect(),
        });

        // Breadth-first over injections.
        let mut ix = 0;
        while ix < self.layers.len() {
            let parent = self.layers[ix].clone();
            ix += 1;
            if parent.depth >= MAX_INJECTION_DEPTH {
                continue;
            }
            for (language, ranges) in find_injections(&parent, rope, registry) {
                let start = ranges[0].start;
                let old = old_layers.iter().find(|old| {
                    old.depth == parent.depth + 1
                        && Arc::ptr_eq(&old.language, &language)
                        && old.ranges.first().map(|r| r.start) == Some(start)
                });
                if let Some(tree) = parse(&language, rope, &ranges, old.map(|l| &l.tree)) {
                    self.layers.push(SyntaxLayer {
                        language,
                        tree,
                        depth: parent.depth + 1,
                        ranges,
                    });
                }
            }
        }
    }

    pub fn version(&self) -> text::Version {
        self.version
    }

    pub fn layers(&self) -> &[SyntaxLayer] {
        &self.layers
    }

    pub fn root(&self) -> Option<&SyntaxLayer> {
        self.layers.first()
    }

    /// The innermost layer containing `offset`.
    pub fn layer_at(&self, offset: usize) -> Option<&SyntaxLayer> {
        self.layers
            .iter()
            .filter(|layer| {
                layer
                    .ranges
                    .iter()
                    .any(|r| r.start <= offset && offset <= r.end)
            })
            .max_by_key(|layer| layer.depth)
    }

    /// Sorted, non-overlapping highlight spans intersecting `range`. Deeper
    /// (injected) layers take precedence over their parents; within a layer
    /// the innermost node wins, and for the same node the **last** matching
    /// pattern in the query wins (tree-sitter-highlight semantics, which is
    /// why queries put generic patterns like `(identifier) @variable` first).
    pub fn highlights(&self, text: &Rope, range: Range<usize>) -> Vec<HighlightSpan> {
        let mut result: Vec<HighlightSpan> = Vec::new();
        let mut layers: Vec<&SyntaxLayer> = self.layers.iter().collect();
        layers.sort_by_key(|layer| layer.depth);
        for layer in layers {
            let extent = layer.extent();
            if extent.end < range.start || extent.start > range.end {
                continue;
            }
            let spans = layer_highlights(layer, text, range.clone());
            if !spans.is_empty() {
                result = overlay(&result, &spans);
            }
        }
        result
    }
}

fn find_injections(
    parent: &SyntaxLayer,
    rope: &Rope,
    registry: &LanguageRegistry,
) -> Vec<(Arc<Language>, Vec<Range<usize>>)> {
    let Some(injections) = parent
        .language
        .grammar
        .as_ref()
        .and_then(|g| g.injections.as_ref())
    else {
        return Vec::new();
    };
    let query = &injections.query;
    let mut cursor = QueryCursor::new();
    let mut combined: Vec<(Arc<Language>, Vec<Range<usize>>)> = Vec::new();
    let mut separate: Vec<(Arc<Language>, Vec<Range<usize>>)> = Vec::new();
    let mut matches = cursor.matches(query, parent.tree.root_node(), RopeTextProvider(rope));
    while let Some(m) = matches.next() {
        let mut language_name: Option<String> = None;
        let mut is_combined = false;
        let mut include_children = false;
        for property in query.property_settings(m.pattern_index) {
            match &*property.key {
                "injection.language" => language_name = property.value.as_deref().map(Into::into),
                "injection.combined" => is_combined = true,
                "injection.include-children" => include_children = true,
                _ => {}
            }
        }
        let mut content: Vec<Node> = Vec::new();
        for capture in m.captures() {
            if Some(capture.index) == injections.language_capture {
                let name = rope.text_for_range(capture.node.byte_range());
                language_name = Some(name);
            } else if Some(capture.index) == injections.content_capture {
                content.push(capture.node);
            }
        }
        let Some(language) = language_name
            .as_deref()
            .and_then(|name| registry.language_for_name(name))
        else {
            continue;
        };
        if language.grammar.is_none() || content.is_empty() {
            continue;
        }
        let mut ranges = Vec::new();
        for node in content {
            node_ranges(node, include_children, &mut ranges);
        }
        ranges.retain(|r| !r.is_empty());
        if ranges.is_empty() {
            continue;
        }
        if is_combined {
            match combined.iter_mut().find(|(l, _)| Arc::ptr_eq(l, &language)) {
                Some((_, existing)) => existing.extend(ranges),
                None => combined.push((language, ranges)),
            }
        } else {
            separate.push((language, ranges));
        }
    }
    for (_, ranges) in &mut combined {
        ranges.sort_by_key(|r| r.start);
    }
    separate.extend(combined);
    separate
}

/// The node's byte range, minus its named children unless
/// `include_children`. Anonymous children are kept: some grammars (e.g.
/// Markdown's `code_fence_content`) emit anonymous tokens inside content that
/// belongs to the injected language.
fn node_ranges(node: Node, include_children: bool, out: &mut Vec<Range<usize>>) {
    let range = node.byte_range();
    if include_children || node.named_child_count() == 0 {
        out.push(range);
        return;
    }
    let mut start = range.start;
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        let child_range = child.byte_range();
        if child_range.start > start {
            out.push(start..child_range.start);
        }
        start = start.max(child_range.end);
    }
    if start < range.end {
        out.push(start..range.end);
    }
}

fn layer_highlights(layer: &SyntaxLayer, text: &Rope, range: Range<usize>) -> Vec<HighlightSpan> {
    let Some(grammar) = layer.language.grammar.as_ref() else {
        return Vec::new();
    };
    let Some(query) = grammar.highlights.as_ref() else {
        return Vec::new();
    };
    let mut cursor = QueryCursor::new();
    cursor.set_byte_range(range.clone());
    // (start, end, pattern, highlight)
    let mut captures: Vec<(usize, usize, usize, HighlightId)> = Vec::new();
    let mut matches = cursor.captures(query, layer.tree.root_node(), RopeTextProvider(text));
    while let Some((m, capture_ix)) = matches.next() {
        let capture = m.captures()[*capture_ix];
        let Some(Some(highlight)) = grammar.highlight_ids.get(capture.index as usize) else {
            continue;
        };
        let node_range = capture.node.byte_range();
        if node_range.is_empty() {
            continue;
        }
        captures.push((
            node_range.start,
            node_range.end,
            m.pattern_index,
            *highlight,
        ));
    }
    // Outer nodes first; for the same node, the latest pattern first.
    captures.sort_by(|a, b| a.0.cmp(&b.0).then(b.1.cmp(&a.1)).then(b.2.cmp(&a.2)));

    let mut spans: Vec<HighlightSpan> = Vec::new();
    let mut push = |start: usize, end: usize, highlight: HighlightId| {
        let start = start.max(range.start);
        let end = end.min(range.end);
        if start >= end {
            return;
        }
        if let Some(last) = spans.last_mut()
            && last.range.end == start
            && last.highlight == highlight
        {
            last.range.end = end;
            return;
        }
        spans.push(HighlightSpan {
            range: start..end,
            highlight,
        });
    };
    // Stack of (start, end, highlight) for enclosing captures.
    let mut stack: Vec<(usize, usize, HighlightId)> = Vec::new();
    let mut pos = 0usize;
    for (start, end, _, highlight) in captures {
        while let Some(&(_, top_end, top_hl)) = stack.last() {
            if top_end > start {
                break;
            }
            push(pos, top_end, top_hl);
            pos = pos.max(top_end);
            stack.pop();
        }
        if let Some(&(top_start, top_end, top_hl)) = stack.last() {
            if top_start == start && top_end == end {
                continue; // same node: the later pattern already won
            }
            push(pos, start, top_hl);
        }
        pos = pos.max(start);
        stack.push((start, end, highlight));
    }
    while let Some((_, end, highlight)) = stack.pop() {
        push(pos, end, highlight);
        pos = pos.max(end);
    }
    spans
}

/// Spans in `top` replace whatever `base` had in their ranges.
fn overlay(base: &[HighlightSpan], top: &[HighlightSpan]) -> Vec<HighlightSpan> {
    let mut result = Vec::with_capacity(base.len() + top.len());
    let mut top_iter = top.iter().peekable();
    let mut base_iter = base.iter().cloned().peekable();
    let mut pending: Option<HighlightSpan> = base_iter.next();
    loop {
        match (pending.take(), top_iter.peek()) {
            (None, None) => break,
            (Some(span), None) => {
                result.push(span);
                pending = base_iter.next();
            }
            (None, Some(_)) => {
                result.push(top_iter.next().cloned().unwrap_or_else(|| unreachable!()));
            }
            (Some(span), Some(t)) => {
                if span.range.end <= t.range.start {
                    result.push(span);
                    pending = base_iter.next();
                } else if t.range.end <= span.range.start {
                    result.push((*t).clone());
                    top_iter.next();
                    pending = Some(span);
                } else {
                    // Overlap: keep the part of `span` before `t`, defer the rest.
                    if span.range.start < t.range.start {
                        result.push(HighlightSpan {
                            range: span.range.start..t.range.start,
                            highlight: span.highlight,
                        });
                    }
                    let t = top_iter.next().cloned().unwrap_or_else(|| unreachable!());
                    pending = (span.range.end > t.range.end).then_some(HighlightSpan {
                        range: t.range.end..span.range.end,
                        highlight: span.highlight,
                    });
                    if pending.is_none() {
                        pending = base_iter.next();
                        // Drop base spans entirely covered by `t`.
                        while let Some(p) = pending.as_ref() {
                            if p.range.end <= t.range.end {
                                pending = base_iter.next();
                            } else {
                                if p.range.start < t.range.end {
                                    pending = Some(HighlightSpan {
                                        range: t.range.end..p.range.end,
                                        highlight: p.highlight,
                                    });
                                }
                                break;
                            }
                        }
                    }
                    result.push(t);
                }
            }
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    fn span(range: Range<usize>, id: u32) -> HighlightSpan {
        HighlightSpan {
            range,
            highlight: HighlightId(id),
        }
    }

    #[test]
    fn overlay_replaces_ranges() {
        let base = vec![span(0..10, 1), span(12..20, 2)];
        let top = vec![span(3..5, 9), span(8..14, 8)];
        assert_eq!(
            overlay(&base, &top),
            vec![
                span(0..3, 1),
                span(3..5, 9),
                span(5..8, 1),
                span(8..14, 8),
                span(14..20, 2)
            ]
        );
        assert_eq!(overlay(&[], &top), top);
        assert_eq!(overlay(&base, &[]), base);
    }
}
