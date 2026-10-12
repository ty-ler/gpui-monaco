//! Snippet sessions: inserting a snippet and moving through its tab stops.
//!
//! Ports `OneSnippet` and `SnippetSession` from Monaco's
//! `src/vs/editor/contrib/snippet/browser/snippetSession.ts` and
//! `SnippetController2` from `snippetController2.ts` (MIT). Each cursor
//! gets its own copy of the snippet; placeholders are tracked with
//! decorations whose stickiness grows with typing only while their tab
//! stop is active. Inserting a snippet while one is active nests it into
//! the active placeholder. Choices (`${1|a,b|}`) show their options in the
//! suggest widget.

use std::collections::{HashMap, HashSet, VecDeque};
use std::ops::Range;

use gpui::{App, Context, Global};
use text::{PointUtf16, TransactionId};

use crate::cursor::{EditOperation, EditOperationType, Selection};
use crate::decorations::{DecorationId, DecorationOptions, TrackedRangeStickiness};
use crate::editor::CodeEditor;
use crate::providers::{Command, CompletionItem, CompletionItemKind};
use crate::snippet::parser::{Marker, MarkerId, Placeholder, SnippetParser, Text, TextmateSnippet};
use crate::snippet::variables::{SnippetVariableContext, SnippetWorkspace};
use crate::view::reveal::RevealType;

/// The command choice suggestions run after inserting (Monaco
/// `jumpToNextSnippetPlaceholder`).
pub(crate) const NEXT_PLACEHOLDER_COMMAND: &str = "jumpToNextSnippetPlaceholder";

/// How [`CodeEditor::insert_snippet`] inserts (Monaco
/// `ISnippetInsertOptions`).
#[derive(Clone, Debug)]
pub struct SnippetInsertOptions {
    /// UTF-16 columns before each cursor to replace.
    pub overwrite_before: u32,
    /// UTF-16 columns after each cursor to replace.
    pub overwrite_after: u32,
    /// Re-indent the snippet's lines to the cursor's line.
    pub adjust_whitespace: bool,
    /// Start a new undo step before inserting.
    pub undo_stop_before: bool,
    /// Start a new undo step after inserting.
    pub undo_stop_after: bool,
    /// The text `$CLIPBOARD` inserts; read from the clipboard when unset and
    /// the snippet uses it.
    pub clipboard_text: Option<String>,
}

impl Default for SnippetInsertOptions {
    fn default() -> Self {
        Self {
            overwrite_before: 0,
            overwrite_after: 0,
            adjust_whitespace: true,
            undo_stop_before: true,
            undo_stop_after: true,
            clipboard_text: None,
        }
    }
}

/// The workspace snippet variables (`WORKSPACE_NAME`, `WORKSPACE_FOLDER`,
/// `RELATIVE_FILEPATH`) refer to.
#[derive(Default)]
struct Workspace(Option<SnippetWorkspace>);

impl Global for Workspace {}

/// Sets the workspace snippet variables refer to (Monaco's workspace
/// service), or none.
pub fn set_snippet_workspace(cx: &mut App, workspace: Option<SnippetWorkspace>) {
    cx.set_global(Workspace(workspace));
}

/// One cursor's snippet (Monaco `OneSnippet`).
struct OneSnippet {
    snippet: TextmateSnippet,
    /// The leading whitespace of the line it was inserted on.
    line_leading_whitespace: String,
    /// Byte offset where it was inserted.
    offset: usize,
    /// A decoration per placeholder, created on the first move.
    decorations: Option<HashMap<MarkerId, DecorationId>>,
    /// Placeholders grouped by index, in tab order.
    groups: Vec<Vec<MarkerId>>,
    group_ix: isize,
}

/// Monaco `groupBy(placeholders, Placeholder.compareByIndex)`.
fn placeholder_groups(snippet: &TextmateSnippet) -> Vec<Vec<MarkerId>> {
    let mut placeholders = snippet.placeholders();
    let placeholder = |id: MarkerId| snippet.placeholder(id).expect("a placeholder");
    placeholders.sort_by(|&a, &b| Placeholder::compare_by_index(placeholder(a), placeholder(b)));
    let mut groups: Vec<Vec<MarkerId>> = Vec::new();
    for id in placeholders {
        match groups.last_mut() {
            Some(group) if placeholder(group[0]).index == placeholder(id).index => group.push(id),
            _ => groups.push(vec![id]),
        }
    }
    groups
}

/// The byte offset of UTF-16 offset `utf16` in `text`.
fn utf16_to_byte(text: &str, utf16: usize) -> usize {
    let mut units = 0;
    for (ix, ch) in text.char_indices() {
        if units >= utf16 {
            return ix;
        }
        units += ch.len_utf16();
    }
    text.len()
}

/// Splits at `\r\n`, `\r` or `\n`.
fn split_lines(text: &str) -> Vec<String> {
    let normalized = text.replace("\r\n", "\n").replace('\r', "\n");
    normalized.split('\n').map(str::to_string).collect()
}

fn leading_whitespace(text: &str) -> &str {
    &text[..text.len() - text.trim_start_matches([' ', '\t']).len()]
}

impl OneSnippet {
    fn new(snippet: TextmateSnippet, line_leading_whitespace: String) -> Self {
        let groups = placeholder_groups(&snippet);
        Self {
            snippet,
            line_leading_whitespace,
            offset: 0,
            decorations: None,
            groups,
            group_ix: -1,
        }
    }

    fn has_placeholder(&self) -> bool {
        !self.snippet.placeholders().is_empty()
    }

    fn is_at_first_placeholder(&self) -> bool {
        self.group_ix <= 0 || self.groups.is_empty()
    }

    fn is_at_last_placeholder(&self) -> bool {
        self.group_ix == self.groups.len() as isize - 1
    }

    /// No placeholder, or only a final tab stop at the very end.
    fn is_trivial(&self) -> bool {
        let placeholders = self.snippet.placeholders();
        match placeholders.as_slice() {
            [] => true,
            [only] => {
                self.snippet
                    .placeholder(*only)
                    .is_some_and(Placeholder::is_final_tabstop)
                    && self.snippet.right_most_descendant(self.snippet.root()) == *only
            }
            _ => false,
        }
    }

    fn has_choice(&self) -> bool {
        let mut found = false;
        self.snippet.walk(|id| {
            found = matches!(self.snippet.marker(id), Marker::Choice(_));
            !found
        });
        found
    }

    fn active_group(&self) -> &[MarkerId] {
        if self.group_ix < 0 {
            return &[];
        }
        self.groups
            .get(self.group_ix as usize)
            .map_or(&[], Vec::as_slice)
    }

    fn active_placeholder_count(&self) -> usize {
        self.active_group().len()
    }
}

/// The active snippet (Monaco `SnippetSession` plus the controller's state).
pub(crate) struct SnippetSession {
    snippets: Vec<OneSnippet>,
    /// The undo step that inserted it: undoing it ends the session.
    inserted: Option<TransactionId>,
    /// The choice whose options were last suggested.
    current_choice: Option<MarkerId>,
}

/// Editor state for snippets.
#[derive(Default)]
pub(crate) struct SnippetState {
    pub(crate) session: Option<SnippetSession>,
    /// Set while the controller changes text or selections itself.
    updating: bool,
}

impl CodeEditor {
    /// Whether a snippet's tab stops are active (Monaco `inSnippetMode`).
    pub fn in_snippet_mode(&self) -> bool {
        self.snippet.session.is_some()
    }

    /// Whether the current tab stop offers a choice (Monaco
    /// `inSnippetChoice`).
    pub(crate) fn in_snippet_choice(&self) -> bool {
        self.snippet
            .session
            .as_ref()
            .is_some_and(|s| s.current_choice.is_some())
    }

    /// Whether Tab moves to another tab stop (Monaco `hasNextTabstop`).
    pub fn has_next_tabstop(&self) -> bool {
        self.snippet
            .session
            .as_ref()
            .is_some_and(|s| !s.snippets[0].is_at_last_placeholder())
    }

    /// Whether Shift+Tab moves to an earlier tab stop (Monaco
    /// `hasPrevTabstop`).
    pub fn has_prev_tabstop(&self) -> bool {
        self.snippet
            .session
            .as_ref()
            .is_some_and(|s| !s.snippets[0].is_at_first_placeholder())
    }

    /// Inserts `template` (TextMate snippet syntax) at every cursor and
    /// selects its first tab stop (Monaco `SnippetController2.insert`,
    /// `editor.action.insertSnippet`). Inside an active snippet it's nested
    /// into the current placeholder.
    pub fn insert_snippet(
        &mut self,
        template: &str,
        options: SnippetInsertOptions,
        cx: &mut Context<Self>,
    ) {
        if self.options.read_only {
            return;
        }
        self.snippet.updating = true;
        if options.undo_stop_before {
            self.model()
                .update(cx, |model, _| model.push_stack_element());
        }
        let mut options = options;
        if options.clipboard_text.is_none() && SnippetParser::guess_needs_clipboard(template) {
            options.clipboard_text = cx.read_from_clipboard().and_then(|item| item.text());
        }
        if self.snippet.session.is_none() {
            self.start_snippet_session(template, &options, cx);
        } else {
            self.merge_snippet(template, &options, cx);
        }
        if options.undo_stop_after {
            self.model()
                .update(cx, |model, _| model.push_stack_element());
        }
        self.snippet.updating = false;
        self.update_snippet_state(cx);
    }

    fn start_snippet_session(
        &mut self,
        template: &str,
        options: &SnippetInsertOptions,
        cx: &mut Context<Self>,
    ) {
        let (edits, mut snippets) = self.snippet_edits(template, options, false, cx);
        let ends = self.apply_snippet_edits(&edits, &mut snippets, cx);
        let inserted = self.model().read(cx).last_transaction_id();
        self.snippet.session = Some(SnippetSession {
            snippets,
            inserted,
            current_choice: None,
        });
        let selections =
            if self.snippet.session.as_ref().expect("set").snippets[0].has_placeholder() {
                self.snippet_move(Some(true), cx)
            } else {
                ends
            };
        if !selections.is_empty() {
            self.set_selections(&selections, cx);
        }
        let primary = self.cursors.primary().position;
        self.reveal_range(primary..primary, RevealType::Simple, cx);
    }

    /// Monaco `SnippetSession.merge`: nests `template` into the active
    /// placeholders.
    fn merge_snippet(
        &mut self,
        template: &str,
        options: &SnippetInsertOptions,
        cx: &mut Context<Self>,
    ) {
        let (edits, mut snippets) = self.snippet_edits(template, options, true, cx);
        let ends = self.apply_snippet_edits(&edits, &mut snippets, cx);
        let Some(mut session) = self.snippet.session.take() else {
            return;
        };
        let is_trivial = snippets[0].is_trivial();
        let active: usize = session
            .snippets
            .iter()
            .map(OneSnippet::active_placeholder_count)
            .sum();
        let can_merge = snippets.len() == active;
        let mut nested: VecDeque<OneSnippet> = snippets.into();
        if !is_trivial && can_merge {
            for one in &mut session.snippets {
                self.merge_one_snippet(one, &mut nested, cx);
            }
        }
        let has_placeholder = session.snippets[0].has_placeholder();
        self.snippet.session = Some(session);
        let selections = if has_placeholder && !is_trivial && can_merge {
            self.snippet_move(None, cx)
        } else {
            ends
        };
        if !selections.is_empty() {
            self.set_selections(&selections, cx);
        }
    }

    /// Applies the insertion edits and records where each snippet landed.
    /// Returns a cursor at the end of each insertion.
    fn apply_snippet_edits(
        &mut self,
        edits: &[(Range<PointUtf16>, String)],
        snippets: &mut [OneSnippet],
        cx: &mut Context<Self>,
    ) -> Vec<Selection> {
        // Each edit's range in the new text.
        let ranges = self.apply(
            EditOperation {
                edits: edits.to_vec(),
                selections_after: self.cursors.selections(),
                kind: EditOperationType::Other,
                push_stack_element_before: false,
                push_stack_element_after: false,
                auto_closed: Vec::new(),
                follow_up: None,
                auto_whitespace: Vec::new(),
            },
            cx,
        );
        for (one, range) in snippets.iter_mut().zip(&ranges) {
            one.offset = range.start;
        }
        ranges
            .iter()
            .map(|range| Selection::collapsed(self.snapshot.offset_to_point_utf16(range.end)))
            .collect()
    }

    /// Monaco `adjustSelection`: the selection grown by `before` and
    /// `after` columns around its head, on the head's line.
    fn adjust_snippet_selection(&self, selection: Selection, before: u32, after: u32) -> Selection {
        if before == 0 && after == 0 {
            return selection;
        }
        let head = selection.head;
        let line_len = self.snapshot.line_len_utf16(head.row);
        let start = PointUtf16::new(head.row, head.column.saturating_sub(before).min(line_len));
        let end = PointUtf16::new(head.row, (head.column + after).min(line_len));
        let reversed = selection.head < selection.anchor;
        Selection::from_range(start..end, reversed)
    }

    fn text_in(&self, range: Range<PointUtf16>) -> String {
        let start = self.snapshot.point_utf16_to_offset(range.start);
        let end = self.snapshot.point_utf16_to_offset(range.end);
        self.snapshot.rope().text_for_range(start..end)
    }

    /// Monaco `createEditsAndSnippetsFromSelections`: one parsed, adjusted
    /// and resolved snippet per cursor, in cursor order.
    fn snippet_edits(
        &mut self,
        template: &str,
        options: &SnippetInsertOptions,
        enforce_final_tabstop: bool,
        cx: &mut Context<Self>,
    ) -> (Vec<(Range<PointUtf16>, String)>, Vec<OneSnippet>) {
        let selections = self.cursors.selections();
        let primary = selections[0];
        let range_of = |s: Selection| s.start()..s.end();
        let first_before = self.text_in(range_of(self.adjust_snippet_selection(
            primary,
            options.overwrite_before,
            0,
        )));
        let first_after = self.text_in(range_of(self.adjust_snippet_selection(
            primary,
            0,
            options.overwrite_after,
        )));
        let first_non_whitespace = |row: u32| {
            let line = self.snapshot.line(row);
            line.find(|c: char| c != ' ' && c != '\t')
        };
        let first_line_first_non_whitespace = first_non_whitespace(primary.head.row);
        let now = chrono::Local::now();
        let workspace = cx.try_global::<Workspace>().and_then(|w| w.0.clone());
        let (path, uri, line_comment, block_comment) = {
            let model = self.model().read(cx);
            let uri = model.uri().map(|uri| uri.to_string());
            let path = uri
                .as_deref()
                .and_then(|uri| uri.strip_prefix("file://"))
                .map(str::to_string);
            let config = model.language().map(|language| language.config.clone());
            (
                path.clone(),
                uri.filter(|_| path.is_none()),
                config
                    .as_ref()
                    .and_then(|c| c.comments().line_comment.clone()),
                config.and_then(|c| c.comments().block_comment.clone()),
            )
        };
        let config = self.cursor_config(cx);
        let mut indexed: Vec<(usize, Selection)> = selections.iter().copied().enumerate().collect();
        indexed.sort_by_key(|(_, s)| (s.start(), s.end()));
        let count = indexed.len();
        let mut edits = vec![None; count];
        let mut snippets: Vec<Option<OneSnippet>> = (0..count).map(|_| None).collect();
        for (ix, selection) in indexed {
            let mut before = self.adjust_snippet_selection(selection, options.overwrite_before, 0);
            let mut after = self.adjust_snippet_selection(selection, 0, options.overwrite_after);
            if self.text_in(range_of(before)) != first_before {
                before = selection;
            }
            if self.text_in(range_of(after)) != first_after {
                after = selection;
            }
            let range = before.start().min(selection.start())..after.end().max(selection.end());
            let mut snippet = SnippetParser::new().parse(template, true, enforce_final_tabstop);
            let adjust = options.adjust_whitespace
                || (ix > 0
                    && first_line_first_non_whitespace != first_non_whitespace(selection.head.row));
            let line_leading_whitespace =
                self.adjust_snippet_whitespace(range.start, adjust, &mut snippet, &config);
            let head = selection.head;
            let start_line = self.snapshot.line(selection.start().row);
            // Monaco `SelectionBasedVariableResolver`: without selected
            // text, a suggestion's `$SELECTION` is what typing last replaced
            // at this cursor (its `OvertypingCapturer`).
            let (selected_text, selection_is_multiline) = match (
                self.text_in(range_of(selection)),
                self.suggest
                    .snippet_overtyped
                    .as_ref()
                    .and_then(|overtyped| overtyped.get(ix)),
            ) {
                (text, Some((overtyped, multiline))) if text.is_empty() => {
                    (overtyped.clone(), *multiline)
                }
                (text, _) => (text, selection.start().row != selection.end().row),
            };
            let context = SnippetVariableContext {
                selected_text,
                selection_is_multiline,
                selection_line_leading_whitespace: leading_whitespace(
                    &start_line
                        .encode_utf16()
                        .take(selection.start().column as usize)
                        .map(|u| char::from_u32(u as u32).unwrap_or(' '))
                        .collect::<String>(),
                )
                .to_string(),
                current_line: self.snapshot.line(head.row),
                current_word: self.model().read(cx).word_at_position(head).map(|w| w.word),
                line_index: head.row,
                cursor_index: ix,
                cursor_count: count,
                path: path.clone(),
                uri: uri.clone(),
                workspace: workspace.clone(),
                clipboard: options.clipboard_text.clone(),
                clipboard_spread: self.options.multi_cursor_paste
                    == crate::cursor::MultiCursorPaste::Spread,
                line_comment: line_comment.clone(),
                block_comment: block_comment.clone(),
                now: Some(now),
            };
            resolve_snippet_variables(&mut snippet, &context, &line_leading_whitespace);
            edits[ix] = Some((range, snippet.to_string()));
            snippets[ix] = Some(OneSnippet::new(snippet, line_leading_whitespace));
        }
        (
            edits.into_iter().flatten().collect(),
            snippets.into_iter().flatten().collect(),
        )
    }

    /// Monaco `SnippetSession.adjustWhitespace`: indents the snippet's lines
    /// like the line at `position` and normalizes their indentation.
    /// Returns that line's leading whitespace.
    fn adjust_snippet_whitespace(
        &self,
        position: PointUtf16,
        adjust_indentation: bool,
        snippet: &mut TextmateSnippet,
        config: &crate::cursor::CursorConfig,
    ) -> String {
        let line: String = self
            .snapshot
            .line(position.row)
            .encode_utf16()
            .take(position.column as usize)
            .map(|u| char::from_u32(u as u32).unwrap_or(' '))
            .collect();
        let line_leading_whitespace = leading_whitespace(&line).to_string();
        let mut texts = Vec::new();
        snippet.walk(|id| {
            if let Marker::Text(_) = snippet.marker(id)
                && !snippet
                    .parent(id)
                    .is_some_and(|parent| matches!(snippet.marker(parent), Marker::Choice(_)))
            {
                texts.push(id);
            }
            true
        });
        for id in texts {
            let Marker::Text(Text { value, .. }) = snippet.marker(id) else {
                continue;
            };
            let mut lines = split_lines(value);
            if adjust_indentation {
                let offset = snippet.offset(id).unwrap_or(0);
                if offset == 0 {
                    lines[0] = config.normalize_indentation(&lines[0]);
                } else {
                    let text = snippet.to_string();
                    let previous = text.encode_utf16().nth(offset - 1);
                    if matches!(previous, Some(0x0a) | Some(0x0d)) {
                        lines[0] = config.normalize_indentation(&format!(
                            "{line_leading_whitespace}{}",
                            lines[0]
                        ));
                    }
                }
                for line in lines.iter_mut().skip(1) {
                    *line =
                        config.normalize_indentation(&format!("{line_leading_whitespace}{line}"));
                }
            }
            let new_value = lines.join("\n");
            if new_value != *value {
                let text = snippet.create(Marker::Text(Text::new(new_value)));
                snippet.replace(id, &[text]);
            }
        }
        line_leading_whitespace
    }

    /// The decoration of a placeholder (Monaco's `snippet-placeholder` and
    /// `finish-snippet-placeholder`): active ones grow when typing at their
    /// edges.
    fn placeholder_decoration(&self, active: bool, is_final: bool) -> DecorationOptions {
        let colors = &self.theme.colors;
        let (background, border) = if is_final {
            (
                colors.snippet_final_tabstop_background,
                colors.snippet_final_tabstop_border,
            )
        } else {
            (
                colors.snippet_tabstop_background,
                colors.snippet_tabstop_border,
            )
        };
        let visible = |color: gpui::Hsla| (color.a > 0.).then_some(color);
        DecorationOptions {
            background: visible(background),
            border: visible(border),
            stickiness: if active && !is_final {
                TrackedRangeStickiness::AlwaysGrowsWhenTypingAtEdges
            } else {
                TrackedRangeStickiness::NeverGrowsWhenTypingAtEdges
            },
            ..Default::default()
        }
    }

    fn placeholder_range(&self, id: DecorationId, cx: &App) -> Option<Range<usize>> {
        self.model()
            .read(cx)
            .decoration(id)
            .map(|decoration| decoration.range.clone())
    }

    /// Monaco `OneSnippet._initDecorations`.
    fn init_snippet_decorations(&mut self, one: &mut OneSnippet, cx: &mut Context<Self>) {
        if one.decorations.is_some() {
            return;
        }
        let text = one.snippet.to_string();
        let placeholders = one.snippet.placeholders();
        let decorations: Vec<_> = placeholders
            .iter()
            .map(|&id| {
                let offset = one.snippet.offset(id).unwrap_or(0);
                let len = one.snippet.full_len(id);
                let start = one.offset + utf16_to_byte(&text, offset);
                let end = one.offset + utf16_to_byte(&text, offset + len);
                let is_final = one
                    .snippet
                    .placeholder(id)
                    .is_some_and(Placeholder::is_final_tabstop);
                (start..end, self.placeholder_decoration(false, is_final))
            })
            .collect();
        let owner = Some(cx.entity_id());
        let ids = self.model().update(cx, |model, cx| {
            model.delta_decorations(owner, &[], decorations, cx)
        });
        one.decorations = Some(placeholders.into_iter().zip(ids).collect());
    }

    fn set_placeholder_active(
        &mut self,
        one: &OneSnippet,
        placeholder: MarkerId,
        active: bool,
        cx: &mut Context<Self>,
    ) {
        let Some(&id) = one.decorations.as_ref().and_then(|d| d.get(&placeholder)) else {
            return;
        };
        let is_final = one
            .snippet
            .placeholder(placeholder)
            .is_some_and(Placeholder::is_final_tabstop);
        let options = self.placeholder_decoration(active, is_final);
        self.model().update(cx, |model, cx| {
            model.change_decoration_options(id, options, cx)
        });
    }

    /// Monaco `_hasPlaceholderBeenCollapsed`: it, or a placeholder around
    /// it, was authored with text but its range is now empty.
    fn placeholder_collapsed(&self, one: &OneSnippet, placeholder: MarkerId, cx: &App) -> bool {
        let decorations = one.decorations.as_ref();
        let mut marker = Some(placeholder);
        while let Some(id) = marker {
            if one.snippet.placeholder(id).is_some() {
                let range = decorations
                    .and_then(|d| d.get(&id))
                    .and_then(|&d| self.placeholder_range(d, cx));
                if range.is_none_or(|r| r.is_empty())
                    && !one.snippet.marker_to_string(id).is_empty()
                {
                    return true;
                }
            }
            marker = one.snippet.parent(id);
        }
        false
    }

    /// Monaco `OneSnippet.move`: applies the current tab stop's transforms,
    /// moves to the next (`Some(true)`), previous (`Some(false)`) or same
    /// (`None`) one, and returns its selections.
    fn move_one_snippet(
        &mut self,
        one: &mut OneSnippet,
        forward: Option<bool>,
        cx: &mut Context<Self>,
    ) -> Vec<Selection> {
        self.init_snippet_decorations(one, cx);
        // Transform the text of the tab stop being left.
        let mut edits = Vec::new();
        for &placeholder in one.active_group() {
            let Some(transform) = one
                .snippet
                .placeholder(placeholder)
                .and_then(|p| p.transform.clone())
            else {
                continue;
            };
            let Some(range) = one
                .decorations
                .as_ref()
                .and_then(|d| d.get(&placeholder))
                .and_then(|&id| self.placeholder_range(id, cx))
            else {
                continue;
            };
            let rope = self.model().read(cx).buffer().rope().clone();
            let current = rope.text_for_range(range.clone());
            let config = self.cursor_config(cx);
            let mut lines = split_lines(&transform.resolve(&current));
            for line in lines.iter_mut().skip(1) {
                *line =
                    config.normalize_indentation(&format!("{}{line}", one.line_leading_whitespace));
            }
            edits.push((
                rope.offset_to_point_utf16(range.start)..rope.offset_to_point_utf16(range.end),
                lines.join("\n"),
            ));
        }
        if !edits.is_empty() {
            self.snapshot = self.model().read(cx).snapshot();
            self.apply(
                EditOperation {
                    edits,
                    selections_after: self.cursors.selections(),
                    kind: EditOperationType::Other,
                    push_stack_element_before: false,
                    push_stack_element_after: false,
                    auto_closed: Vec::new(),
                    follow_up: None,
                    auto_whitespace: Vec::new(),
                },
                cx,
            );
        }

        let mut could_skip = false;
        let last = one.groups.len() as isize - 1;
        match forward {
            Some(true) if one.group_ix < last => {
                one.group_ix += 1;
                could_skip = true;
            }
            Some(false) if one.group_ix > 0 => {
                one.group_ix -= 1;
                could_skip = true;
            }
            // The current tab stop's selection may be stale: restore it.
            _ => {}
        }

        let mut active = HashSet::new();
        let mut selections = Vec::new();
        let group = one.active_group().to_vec();
        for placeholder in group {
            could_skip = could_skip && self.placeholder_collapsed(one, placeholder, cx);
            let Some(range) = one
                .decorations
                .as_ref()
                .and_then(|d| d.get(&placeholder))
                .and_then(|&id| self.placeholder_range(id, cx))
            else {
                continue;
            };
            let rope = self.model().read(cx).buffer().rope().clone();
            selections.push(Selection::new(
                rope.offset_to_point_utf16(range.start),
                rope.offset_to_point_utf16(range.end),
            ));
            self.set_placeholder_active(one, placeholder, true, cx);
            active.insert(placeholder);
            for enclosing in one.snippet.enclosing_placeholders(placeholder) {
                if one
                    .decorations
                    .as_ref()
                    .is_some_and(|d| d.contains_key(&enclosing))
                {
                    self.set_placeholder_active(one, enclosing, true, cx);
                    active.insert(enclosing);
                }
            }
        }
        let inactive: Vec<MarkerId> = one
            .decorations
            .as_ref()
            .map(|d| d.keys().filter(|p| !active.contains(p)).copied().collect())
            .unwrap_or_default();
        for placeholder in inactive {
            self.set_placeholder_active(one, placeholder, false, cx);
        }
        if could_skip {
            self.move_one_snippet(one, forward, cx)
        } else {
            selections
        }
    }

    /// Monaco `SnippetSession._move` over every cursor's snippet.
    fn snippet_move(&mut self, forward: Option<bool>, cx: &mut Context<Self>) -> Vec<Selection> {
        let Some(mut session) = self.snippet.session.take() else {
            return Vec::new();
        };
        let updating = std::mem::replace(&mut self.snippet.updating, true);
        let mut selections = Vec::new();
        for one in &mut session.snippets {
            selections.extend(self.move_one_snippet(one, forward, cx));
        }
        self.snippet.updating = updating;
        self.snippet.session = Some(session);
        selections
    }

    /// Monaco `OneSnippet.merge`: replaces each active placeholder with the
    /// next nested snippet, ordering the nested tab stops right after it.
    fn merge_one_snippet(
        &mut self,
        one: &mut OneSnippet,
        nested: &mut VecDeque<OneSnippet>,
        cx: &mut Context<Self>,
    ) {
        // Monaco sorts with fractional indices: the nested snippet's tab
        // stops come right after the placeholder they replace, its final
        // tab stop last among them. Here: sort keys, then small integers.
        const NESTING: f64 = 10.;
        let mut keys: HashMap<MarkerId, f64> = one
            .snippet
            .placeholders()
            .into_iter()
            .map(|id| {
                (
                    id,
                    one.snippet.placeholder(id).expect("placeholder").index as f64,
                )
            })
            .collect();
        let owner = Some(cx.entity_id());
        for placeholder in one.active_group().to_vec() {
            let Some(other) = nested.pop_front() else {
                break;
            };
            let base = one
                .snippet
                .placeholder(placeholder)
                .expect("placeholder")
                .index as f64;
            let last_index = other
                .snippet
                .placeholder_info()
                .last
                .and_then(|id| other.snippet.placeholder(id))
                .map_or(0, |p| p.index) as f64;
            // Where each nested placeholder is, in the inserted text.
            let other_text = other.snippet.to_string();
            let nested_info: Vec<(f64, Range<usize>)> = other
                .snippet
                .placeholders()
                .into_iter()
                .map(|id| {
                    let p = other.snippet.placeholder(id).expect("placeholder");
                    let key = if p.is_final_tabstop() {
                        base + (last_index + 1.) / NESTING
                    } else {
                        base + p.index as f64 / NESTING
                    };
                    let offset = other.snippet.offset(id).unwrap_or(0);
                    let len = other.snippet.full_len(id);
                    let start = other.offset + utf16_to_byte(&other_text, offset);
                    let end = other.offset + utf16_to_byte(&other_text, offset + len);
                    (key, start..end)
                })
                .collect();
            let before: HashSet<MarkerId> = one.snippet.placeholders().into_iter().collect();
            one.snippet
                .replace_with_snippet(placeholder, &other.snippet);
            keys.remove(&placeholder);
            if let Some(id) = one
                .decorations
                .as_mut()
                .and_then(|d| d.remove(&placeholder))
            {
                self.model().update(cx, |model, cx| {
                    model.delta_decorations(owner, &[id], Vec::new(), cx)
                });
            }
            // The imported placeholders, in the same order as the nested
            // snippet's.
            let imported: Vec<MarkerId> = one
                .snippet
                .placeholders()
                .into_iter()
                .filter(|id| !before.contains(id))
                .collect();
            let options = self.placeholder_decoration(false, false);
            let ranges: Vec<_> = nested_info
                .iter()
                .map(|(_, range)| (range.clone(), options.clone()))
                .collect();
            let ids = self.model().update(cx, |model, cx| {
                model.delta_decorations(owner, &[], ranges, cx)
            });
            for ((id, decoration), (key, _)) in imported.iter().zip(ids).zip(&nested_info) {
                keys.insert(*id, *key);
                if let Some(decorations) = one.decorations.as_mut() {
                    decorations.insert(*id, decoration);
                }
            }
        }
        // Monaco `_renormalizePlaceholderIndices`: non-final tab stops become
        // 1, 2, 3… in key order.
        let mut unique: Vec<f64> = keys.values().copied().filter(|k| *k != 0.).collect();
        unique.sort_by(f64::total_cmp);
        unique.dedup();
        for (id, key) in &keys {
            if *key == 0. {
                continue;
            }
            let rank = unique.iter().position(|k| k == key).expect("present") + 1;
            if let Marker::Placeholder(p) = one.snippet.marker_mut(*id) {
                p.index = rank;
            }
        }
        one.groups = placeholder_groups(&one.snippet);
    }

    /// Moves to the next tab stop (Monaco `jumpToNextSnippetPlaceholder`,
    /// Tab).
    pub fn next_snippet_placeholder(&mut self, cx: &mut Context<Self>) {
        self.move_snippet(true, cx);
    }

    /// Moves to the previous tab stop (Monaco
    /// `jumpToPrevSnippetPlaceholder`, Shift+Tab).
    pub fn prev_snippet_placeholder(&mut self, cx: &mut Context<Self>) {
        self.move_snippet(false, cx);
    }

    fn move_snippet(&mut self, forward: bool, cx: &mut Context<Self>) {
        if self.snippet.session.is_none() {
            return;
        }
        let selections = self.snippet_move(Some(forward), cx);
        if !selections.is_empty() {
            self.snippet.updating = true;
            self.set_selections(&selections, cx);
            self.snippet.updating = false;
            let position = selections[0].head;
            self.reveal_range(position..position, RevealType::CenterIfOutsideViewport, cx);
        }
        self.update_snippet_state(cx);
    }

    /// Moves through every remaining tab stop (Monaco `acceptSnippet`).
    pub fn finish_snippet(&mut self, cx: &mut Context<Self>) {
        while self.in_snippet_mode() {
            self.next_snippet_placeholder(cx);
        }
    }

    /// Leaves snippet mode (Monaco `leaveSnippet`, Escape); with
    /// `reset_selection`, keeps only the primary cursor.
    pub fn cancel_snippet(&mut self, reset_selection: bool, cx: &mut Context<Self>) {
        let Some(session) = self.snippet.session.take() else {
            return;
        };
        let ids: Vec<DecorationId> = session
            .snippets
            .iter()
            .filter_map(|one| one.decorations.as_ref())
            .flat_map(|d| d.values().copied())
            .collect();
        if !ids.is_empty() {
            let owner = Some(cx.entity_id());
            self.model().update(cx, |model, cx| {
                model.delta_decorations(owner, &ids, Vec::new(), cx)
            });
        }
        if reset_selection {
            let primary = self.cursors.selections()[0];
            self.set_selections(&[primary], cx);
        }
    }

    /// Called when the selections change.
    pub(crate) fn snippet_selections_changed(&mut self, cx: &mut Context<Self>) {
        if !self.snippet.updating {
            self.update_snippet_state(cx);
        }
    }

    /// Monaco `SnippetController2._updateState`: leaves snippet mode once
    /// the insertion was undone, at the last tab stop, or when the
    /// selections left the placeholders.
    fn update_snippet_state(&mut self, cx: &mut Context<Self>) {
        let Some(session) = &self.snippet.session else {
            return;
        };
        let last = self.model().read(cx).last_transaction_id();
        if session.inserted.is_some() && last < session.inserted {
            self.cancel_snippet(false, cx);
            return;
        }
        if !session.snippets[0].has_placeholder() {
            self.cancel_snippet(false, cx);
            return;
        }
        if session.snippets[0].is_at_last_placeholder() || !self.selections_within_placeholders(cx)
        {
            self.model()
                .update(cx, |model, _| model.push_stack_element());
            self.cancel_snippet(false, cx);
            return;
        }
        self.handle_snippet_choice(cx);
    }

    /// Monaco `computePossibleSelections`: the ranges of each tab stop
    /// index (except the final one), or none for an index whose
    /// placeholder lost its range.
    fn possible_selections(
        &self,
        one: &OneSnippet,
        cx: &App,
    ) -> HashMap<usize, Vec<Range<PointUtf16>>> {
        let rope = self.model().read(cx).buffer().rope().clone();
        let mut result = HashMap::new();
        for group in &one.groups {
            let mut ranges = Vec::new();
            let mut index = None;
            for &placeholder in group {
                let p = one.snippet.placeholder(placeholder).expect("placeholder");
                if p.is_final_tabstop() {
                    break;
                }
                index = Some(p.index);
                match one
                    .decorations
                    .as_ref()
                    .and_then(|d| d.get(&placeholder))
                    .and_then(|&id| self.placeholder_range(id, cx))
                {
                    Some(range) => ranges.push(
                        rope.offset_to_point_utf16(range.start)
                            ..rope.offset_to_point_utf16(range.end),
                    ),
                    None => {
                        index = None;
                        break;
                    }
                }
            }
            if let Some(index) = index {
                result.insert(index, ranges);
            }
        }
        result
    }

    /// Monaco `isSelectionWithinPlaceholders`.
    fn selections_within_placeholders(&self, cx: &App) -> bool {
        let Some(session) = &self.snippet.session else {
            return false;
        };
        if !session.snippets[0].has_placeholder() {
            return false;
        }
        let mut selections: Vec<Range<PointUtf16>> = self
            .cursors
            .selections()
            .iter()
            .map(|s| s.start()..s.end())
            .collect();
        if selections.len() < session.snippets.len() {
            return false;
        }
        let contains = |outer: &Range<PointUtf16>, inner: &Range<PointUtf16>| {
            outer.start <= inner.start && inner.end <= outer.end
        };
        let mut all: HashMap<usize, Vec<Range<PointUtf16>>> = HashMap::new();
        for one in &session.snippets {
            let possible = self.possible_selections(one, cx);
            if all.is_empty() {
                for (index, ranges) in &possible {
                    let mut ranges = ranges.clone();
                    ranges.sort_by_key(|r| (r.start, r.end));
                    if ranges
                        .first()
                        .is_some_and(|first| selections.iter().any(|s| contains(first, s)))
                    {
                        all.insert(*index, Vec::new());
                    }
                }
            }
            if all.is_empty() {
                return false;
            }
            for (index, ranges) in all.iter_mut() {
                ranges.extend(possible.get(index).cloned().unwrap_or_default());
            }
        }
        selections.sort_by_key(|r| (r.start, r.end));
        all.retain(|_, ranges| {
            if ranges.len() != selections.len() {
                return false;
            }
            ranges.sort_by_key(|r| (r.start, r.end));
            ranges.iter().zip(&selections).all(|(r, s)| contains(r, s))
        });
        !all.is_empty()
    }

    /// Monaco `_handleChoice`: entering a choice tab stop suggests its
    /// options.
    fn handle_snippet_choice(&mut self, cx: &mut Context<Self>) {
        let Some(session) = &self.snippet.session else {
            return;
        };
        let one = &session.snippets[0];
        if !one.has_choice() {
            return;
        }
        let Some(&placeholder) = one.active_group().first() else {
            return;
        };
        let Some(choice) = one.snippet.choice(placeholder) else {
            self.snippet
                .session
                .as_mut()
                .expect("checked")
                .current_choice = None;
            return;
        };
        if session.current_choice == Some(placeholder) || choice.options.is_empty() {
            return;
        }
        let Some(range) = one
            .decorations
            .as_ref()
            .and_then(|d| d.get(&placeholder))
            .and_then(|&id| self.placeholder_range(id, cx))
        else {
            return;
        };
        let options: Vec<String> = choice.options.iter().map(|o| o.value.clone()).collect();
        self.snippet
            .session
            .as_mut()
            .expect("checked")
            .current_choice = Some(placeholder);
        let rope = self.model().read(cx).buffer().rope().clone();
        let range = rope.offset_to_point_utf16(range.start)..rope.offset_to_point_utf16(range.end);
        let word = self.text_in(range.clone());
        let is_any_option = options.contains(&word);
        let items = options
            .iter()
            .enumerate()
            .map(|(ix, option)| CompletionItem {
                insert_text: Some(option.clone().into()),
                sort_text: Some("a".repeat(ix + 1).into()),
                range: Some(range.clone()),
                filter_text: is_any_option.then(|| format!("{word}_{option}").into()),
                command: Some(Command {
                    id: NEXT_PLACEHOLDER_COMMAND.into(),
                    title: "Go to next placeholder...".into(),
                    arguments: Vec::new(),
                    ..Default::default()
                }),
                ..CompletionItem::new(option.clone(), CompletionItemKind::Value)
            })
            .collect();
        // Monaco shows them on the next microtask.
        cx.spawn(async move |this, cx| {
            this.update(cx, |this, cx| {
                this.show_simple_suggestions(items, range, cx)
            })
            .ok();
        })
        .detach();
    }
}

/// Resolves the snippet's variables, giving each the indentation of its
/// line within the snippet (for multi-line selections).
fn resolve_snippet_variables(
    snippet: &mut TextmateSnippet,
    context: &SnippetVariableContext,
    line_leading_whitespace: &str,
) {
    let mut variables = Vec::new();
    snippet.walk(|id| {
        if snippet.variable(id).is_some() {
            variables.push(id);
        }
        true
    });
    for variable in variables {
        // Monaco: the leading whitespace of the last line of the text before
        // the variable.
        let mut leading = line_leading_whitespace.to_string();
        snippet.walk(|id| {
            if id == variable {
                return false;
            }
            if let Marker::Text(text) = snippet.marker(id) {
                leading = leading_whitespace(split_lines(&text.value).last().map_or("", |l| l))
                    .to_string();
            }
            true
        });
        let resolver =
            |v: &crate::snippet::parser::Variable| context.resolve(&v.name, Some(&leading));
        snippet.resolve_variable(variable, &resolver);
    }
}
