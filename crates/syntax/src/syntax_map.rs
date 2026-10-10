//! [`SyntaxSnapshot`]: incrementally parsed tree-sitter trees for a buffer,
//! including injected languages (e.g. code blocks in Markdown, `<script>` in
//! HTML) as nested layers.
//!
//! Parsing is a pure function of the text and the previous snapshot, so
//! callers can run it on a background thread and swap the result in.

use std::cell::RefCell;
use std::collections::HashMap;
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
    /// The language this layer was parsed as.
    pub language: Arc<Language>,
    /// The parsed tree, with byte offsets into the whole buffer text.
    pub tree: Tree,
    /// Injection nesting depth: 0 for the root.
    pub depth: usize,
    /// Byte ranges this layer was parsed from (the whole text for the root).
    pub ranges: Vec<Range<usize>>,
    /// This layer's injections, cached so a reparse that reuses the tree
    /// skips the injection query (Rust makes a layer per macro invocation).
    injections: Option<Arc<Injections>>,
}

#[derive(Clone)]
struct Injection {
    language: Arc<Language>,
    ranges: Vec<Range<usize>>,
    /// Everything the match captured (e.g. a fence's info string too): an
    /// edit here may change the injection even outside `ranges`.
    extent: Range<usize>,
}

type Injections = Vec<Injection>;

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
    /// Byte range in the buffer text.
    pub range: Range<usize>,
    /// The capture to style the range with.
    pub highlight: HighlightId,
}

/// The syntax trees for one version of a buffer.
#[derive(Clone)]
pub struct SyntaxSnapshot {
    layers: Vec<SyntaxLayer>,
    version: text::Version,
    /// Injected layers' (extent, index) sorted by start, so highlighting a
    /// range skips the thousands of layers elsewhere (Rust has one per macro
    /// invocation).
    by_start: Arc<[(Range<usize>, usize)]>,
    /// The longest injected extent, bounding how far before a range an
    /// intersecting layer can start.
    max_extent: usize,
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
            by_start: Arc::new([]),
            max_extent: 0,
        };
        snapshot.build(language, text, &[], &[], registry);
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
            // Nothing parsed (the root parse failed): stay empty, but at the
            // new version.
            let mut snapshot = self.clone();
            snapshot.version = text.version();
            return snapshot;
        };
        let edits = input_edits(old_text, text.rope(), patch);
        let edited: Vec<Range<usize>> = patch.edits().iter().map(|e| e.new.clone()).collect();
        let old_layers: Vec<SyntaxLayer> = self
            .layers
            .iter()
            .map(|layer| {
                let mut layer = layer.clone();
                for edit in &edits {
                    layer.tree.edit(edit);
                }
                let transform = |ranges: &[Range<usize>]| -> Vec<Range<usize>> {
                    ranges
                        .iter()
                        .map(|r| {
                            patch.transform_offset(r.start, text::Bias::Right)
                                ..patch.transform_offset(r.end, text::Bias::Left)
                        })
                        .collect()
                };
                layer.ranges = transform(&layer.ranges);
                // Only an untouched layer is reused, and its injections then
                // just shift with the edits around it.
                // The root's are updated piecewise by `update_root_injections`.
                if layer.depth == 0 || !touched(&layer.ranges, &edited) {
                    layer.injections = layer.injections.as_ref().map(|injections| {
                        Arc::new(
                            injections
                                .iter()
                                .map(|injection| Injection {
                                    language: injection.language.clone(),
                                    ranges: transform(&injection.ranges),
                                    extent: transform(std::slice::from_ref(&injection.extent))
                                        .remove(0),
                                })
                                .collect(),
                        )
                    });
                } else {
                    layer.injections = None;
                }
                layer
            })
            .collect();
        let mut snapshot = Self {
            layers: Vec::new(),
            version: text.version(),
            by_start: Arc::new([]),
            max_extent: 0,
        };
        snapshot.build(root.language.clone(), text, &old_layers, &edited, registry);
        snapshot
    }

    fn build(
        &mut self,
        root_language: Arc<Language>,
        text: &BufferSnapshot,
        old_layers: &[SyntaxLayer],
        edited: &[Range<usize>],
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
            injections: None,
        });

        // Old injection layers by (depth, language, ranges), since there can
        // be thousands of them.
        let old_by_key: HashMap<(usize, usize, &[Range<usize>]), &SyntaxLayer> = old_layers
            .iter()
            .skip(1)
            .map(|old| {
                let key = (
                    old.depth,
                    Arc::as_ptr(&old.language) as usize,
                    &old.ranges[..],
                );
                (key, old)
            })
            .collect();

        // Breadth-first over injections.
        let mut ix = 0;
        while ix < self.layers.len() {
            let parent_ix = ix;
            ix += 1;
            let depth = self.layers[parent_ix].depth + 1;
            if depth > MAX_INJECTION_DEPTH {
                continue;
            }
            let injections = match self.layers[parent_ix].injections.clone() {
                Some(cached) => cached,
                None => {
                    let layer = &self.layers[parent_ix];
                    let found = Arc::new(match old_layers.first() {
                        Some(old_root) if parent_ix == 0 => {
                            update_root_injections(layer, old_root, rope, edited, registry)
                        }
                        _ => find_injections(layer, rope, registry, None),
                    });
                    self.layers[parent_ix].injections = Some(found.clone());
                    found
                }
            };
            for Injection {
                language, ranges, ..
            } in injections.iter().cloned()
            {
                // Injected layers are small, so instead of incremental
                // reparsing (which some grammars' external scanners get wrong,
                // e.g. tree-sitter-md's inline grammar) an untouched layer
                // keeps its tree and a touched one is parsed from scratch.
                let key = (depth, Arc::as_ptr(&language) as usize, &ranges[..]);
                let untouched = old_by_key.get(&key).filter(|_| !touched(&ranges, edited));
                let (tree, injections) = match untouched {
                    Some(old) => (Some(old.tree.clone()), old.injections.clone()),
                    None => (parse(&language, rope, &ranges, None), None),
                };
                if let Some(tree) = tree {
                    self.layers.push(SyntaxLayer {
                        language,
                        tree,
                        depth,
                        ranges,
                        injections,
                    });
                }
            }
        }
        let mut by_start: Vec<(Range<usize>, usize)> = self
            .layers
            .iter()
            .enumerate()
            .skip(1)
            .map(|(ix, layer)| (layer.extent(), ix))
            .collect();
        by_start.sort_unstable_by_key(|(extent, _)| extent.start);
        self.max_extent = by_start.iter().map(|(e, _)| e.len()).max().unwrap_or(0);
        self.by_start = by_start.into();
    }

    /// The buffer version these trees were parsed from.
    pub fn version(&self) -> text::Version {
        self.version
    }

    /// All layers, the root first and then injections breadth-first, so in
    /// increasing depth.
    pub fn layers(&self) -> &[SyntaxLayer] {
        &self.layers
    }

    /// The root layer, or `None` if the language has no grammar or parsing failed.
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

    /// Byte ranges of every node containing `offset`, in every layer around
    /// it (smart select's syntax ranges), innermost first per layer.
    pub fn selection_ranges(&self, offset: usize) -> Vec<Range<usize>> {
        let mut ranges = Vec::new();
        for layer in &self.layers {
            let extent = layer.extent();
            if layer.depth > 0 && !(extent.start <= offset && offset <= extent.end) {
                continue;
            }
            let mut node = layer
                .tree
                .root_node()
                .descendant_for_byte_range(offset, offset);
            while let Some(current) = node {
                let range = current.byte_range();
                if !range.is_empty() {
                    ranges.push(range);
                }
                node = current.parent();
            }
        }
        ranges
    }

    /// Foldable regions from the layers' `folds.scm` queries: each `@fold`
    /// node from its first row to the row before its closing line (a line
    /// starting with a closing bracket or tag, or a node ending at a line
    /// start). `None` if no layer has a folds query. Injected layers in the
    /// root's language are skipped.
    pub fn fold_ranges(&self, text: &Rope) -> Option<Vec<(u32, u32)>> {
        let mut any_query = false;
        let mut ranges = Vec::new();
        let root = self.layers.first().map(|layer| layer.language.id.clone());
        for layer in &self.layers {
            // Injections in the root's own language (Rust macro bodies) are
            // covered by the root's ranges and would cost a query each.
            if layer.depth > 0 && Some(&layer.language.id) == root.as_ref() {
                continue;
            }
            let Some(query) = layer
                .language
                .grammar
                .as_ref()
                .and_then(|grammar| grammar.folds.as_ref())
            else {
                continue;
            };
            any_query = true;
            let mut cursor = QueryCursor::new();
            for range in &layer.ranges {
                cursor.set_byte_range(range.clone());
                let mut matches =
                    cursor.captures(query, layer.tree.root_node(), RopeTextProvider(text));
                while let Some((m, capture_ix)) = matches.next() {
                    let node = m.captures()[*capture_ix].node;
                    if let Some(rows) = fold_rows(node, text) {
                        ranges.push(rows);
                    }
                }
            }
        }
        ranges.sort_unstable();
        ranges.dedup();
        any_query.then_some(ranges)
    }

    /// Sorted, non-overlapping highlight spans intersecting `range`. Deeper
    /// (injected) layers take precedence over their parents; within a layer
    /// the innermost node wins, and for the same node the **last** matching
    /// pattern in the query wins (tree-sitter-highlight semantics, which is
    /// why queries put generic patterns like `(identifier) @variable` first).
    pub fn highlights(&self, text: &Rope, range: Range<usize>) -> Vec<HighlightSpan> {
        let mut result: Vec<HighlightSpan> = Vec::new();
        let intersects =
            |extent: &Range<usize>| extent.end >= range.start && extent.start <= range.end;
        let first = self
            .by_start
            .partition_point(|(extent, _)| extent.start + self.max_extent < range.start);
        let last = self
            .by_start
            .partition_point(|(extent, _)| extent.start <= range.end);
        let mut layers: Vec<usize> = self.by_start[first..last.max(first)]
            .iter()
            .filter(|(extent, _)| intersects(extent))
            .map(|(_, ix)| *ix)
            .collect();
        // `build` adds layers breadth-first, so index order is depth order.
        debug_assert!(self.layers.is_sorted_by_key(|layer| layer.depth));
        layers.sort_unstable();
        if self
            .layers
            .first()
            .is_some_and(|root| intersects(&root.extent()))
        {
            layers.insert(0, 0);
        }
        // Merge one depth at a time: layers at the same depth mostly have
        // disjoint extents, so their spans concatenate into one sorted list
        // and each depth costs one linear merge (not one per layer, which
        // is quadratic with Rust's thousands of macro layers).
        let mut depth_spans: Vec<HighlightSpan> = Vec::new();
        let mut depth = None;
        for ix in layers {
            let layer = &self.layers[ix];
            if depth != Some(layer.depth) {
                if !depth_spans.is_empty() {
                    result = overlay(&result, &std::mem::take(&mut depth_spans));
                }
                depth = Some(layer.depth);
            }
            let spans = layer_highlights(layer, text, range.clone());
            if spans.is_empty() {
                continue;
            }
            let disjoint = depth_spans
                .last()
                .is_none_or(|last| last.range.end <= spans[0].range.start);
            if disjoint {
                depth_spans.extend(spans);
            } else {
                // Overlapping layers at one depth: the later one wins.
                depth_spans = overlay(&depth_spans, &spans);
            }
        }
        if !depth_spans.is_empty() {
            result = overlay(&result, &depth_spans);
        }
        result
    }
}

/// The rows a fold of `node` covers.
fn fold_rows(node: Node, text: &Rope) -> Option<(u32, u32)> {
    let start = node.start_position();
    let end = node.end_position();
    let mut last = end.row as u32;
    if end.column == 0 {
        // The node includes its line break.
        last = last.checked_sub(1)?;
    } else {
        // Keep a closing line visible (`}`, `)`, `]`, `</tag>`).
        let line_start = text.point_to_offset(Point::new(end.row as u32, 0));
        let head = text.text_for_range(line_start..node.end_byte());
        if is_closing_line(head.trim_start()) {
            last = last.checked_sub(1)?;
        }
    }
    // A fenced block (ending with its line break) keeps its closing fence.
    let is_fence = |row: u32| {
        let line_start = text.point_to_offset(Point::new(row, 0));
        let line_end = text.point_to_offset(Point::new(row + 1, 0)).max(line_start);
        let line = text.text_for_range(line_start..line_end);
        let line = line.trim();
        line.starts_with("```") || line.starts_with("~~~")
    };
    if end.column == 0 && last > start.row as u32 && is_fence(start.row as u32) && is_fence(last) {
        last -= 1;
    }
    (last > start.row as u32).then_some((start.row as u32, last))
}

fn is_closing_line(line: &str) -> bool {
    line.starts_with(['}', ')', ']']) || line.starts_with("</")
}

/// Whether any edited range (in new offsets) touches `ranges`.
fn touched(ranges: &[Range<usize>], edited: &[Range<usize>]) -> bool {
    edited.iter().any(|edit| {
        ranges
            .iter()
            .any(|range| edit.start <= range.end && edit.end >= range.start)
    })
}

/// The root layer's injections after a reparse: the old ones away from the
/// changes, shifted, plus a query over just the changed ranges. Falls back to
/// a full query for queries with combined injections.
fn update_root_injections(
    root: &SyntaxLayer,
    old_root: &SyntaxLayer,
    rope: &Rope,
    edited: &[Range<usize>],
    registry: &LanguageRegistry,
) -> Injections {
    let (Some(old), Some(query)) = (
        old_root.injections.as_ref(),
        root.language
            .grammar
            .as_ref()
            .and_then(|g| g.injections.as_ref()),
    ) else {
        return find_injections(root, rope, registry, None);
    };
    if query.has_combined {
        return find_injections(root, rope, registry, None);
    }
    let mut changed: Vec<Range<usize>> = edited.to_vec();
    changed.extend(
        old_root
            .tree
            .changed_ranges(&root.tree)
            .map(|r| r.start_byte..r.end_byte),
    );
    let (dropped, mut result): (Injections, Injections) = old
        .iter()
        .cloned()
        .partition(|injection| touched(std::slice::from_ref(&injection.extent), &changed));
    // Re-query the changes and wherever a dropped injection was, padded so
    // empty edits and nodes merely adjacent to an edit are matched too.
    changed.extend(dropped.iter().map(|injection| injection.extent.clone()));
    changed.sort_by_key(|r| r.start);
    let mut merged: Vec<Range<usize>> = Vec::new();
    for range in changed {
        let range = range.start.saturating_sub(1)..(range.end + 1).min(rope.len());
        match merged.last_mut() {
            Some(last) if range.start <= last.end => last.end = last.end.max(range.end),
            _ => merged.push(range),
        }
    }
    let mut seen: std::collections::HashSet<Vec<Range<usize>>> = result
        .iter()
        .map(|injection| injection.ranges.clone())
        .collect();
    for range in &merged {
        for injection in find_injections(root, rope, registry, Some(range.clone())) {
            if seen.insert(injection.ranges.clone()) {
                result.push(injection);
            }
        }
    }
    result.sort_by_key(|injection| injection.extent.start);
    result
}

fn find_injections(
    parent: &SyntaxLayer,
    rope: &Rope,
    registry: &LanguageRegistry,
    within: Option<Range<usize>>,
) -> Injections {
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
    if let Some(within) = within {
        cursor.set_byte_range(within);
    }
    let mut combined: Vec<(Arc<Language>, Vec<Range<usize>>)> = Vec::new();
    let mut separate: Injections = Vec::new();
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
        let extent = m
            .captures()
            .iter()
            .map(|capture| capture.node.start_byte())
            .min()
            .unwrap_or(0)
            ..m.captures()
                .iter()
                .map(|capture| capture.node.end_byte())
                .max()
                .unwrap_or(0);
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
            separate.push(Injection {
                language,
                ranges,
                extent,
            });
        }
    }
    for (language, mut ranges) in combined {
        ranges.sort_by_key(|r| r.start);
        let extent = ranges.first().map_or(0, |r| r.start)..ranges.last().map_or(0, |r| r.end);
        separate.push(Injection {
            language,
            ranges,
            extent,
        });
    }
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
