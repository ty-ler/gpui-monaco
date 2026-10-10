//! Ghost text: the model of an inline completion rendered as faded text
//! inserted into the current line, and the diff-based computation that
//! derives it from a single text replacement.
//!
//! Ported from Monaco / VS Code (MIT License, Copyright (c) Microsoft
//! Corporation):
//! - `vs/editor/contrib/inlineCompletions/browser/model/ghostText.ts`
//!   ([`GhostText`], [`GhostTextPart`], [`GhostTextReplacement`]),
//! - `.../browser/model/computeGhostText.ts` ([`compute_ghost_text`],
//!   `cachingDiff`, `deletedCharacters`, `smartDiff`),
//! - `.../browser/model/singleTextEditHelpers.ts`
//!   ([`single_text_remove_common_prefix`], [`single_text_edit_augments`]),
//! - `vs/base/common/diff/diff.ts` (`LcsDiff`, the non-"pretty" path only),
//! - tests from `.../test/browser/computeGhostText.test.ts`.
//!
//! # Coordinates
//!
//! Monaco uses 1-based line numbers and columns; this port uses 0-based rows
//! and 0-based UTF-16 columns ([`PointUtf16`]). All string lengths and
//! offsets that Monaco measures with JS `string.length` are measured here in
//! UTF-16 code units too, so the arithmetic carries over unchanged.
//!
//! # Deviations from Monaco
//!
//! - **Char diff:** `LcsDiff` is ported faithfully (forward/reverse Myers
//!   with divide-and-conquer and the same history limits), so change
//!   alignment matches Monaco exactly. The `similar` crate is *not* used:
//!   its Myers variant breaks ties differently, which would change which
//!   ghost text is shown (e.g. the bracket-matching tests). Only the
//!   `pretty = false` path that `smartDiff` uses is ported, without a
//!   continue-processing predicate (Monaco passes none), so the "quit early"
//!   branches are unreachable and omitted.
//! - **Inline decorations:** Monaco's `GhostTextPart` carries per-line
//!   `LineDecoration`s (used by the suggest-widget preview for syntax
//!   colouring). They are omitted; [`GhostTextPart::lines`] holds plain
//!   strings.
//! - **Equality:** the structs derive `PartialEq`, which is structural.
//!   Monaco's `GhostTextPart.equals` ignores `preview`; ours does not.
//! - **Text access:** instead of an `ITextModel`, callers pass a line
//!   accessor (`&dyn Fn(u32) -> String`, line content without EOL). Columns
//!   past the end of a line are clamped when reading (as Monaco's
//!   `getValueInRange` validates its range).
//! - **Lone surrogates:** the diff runs on UTF-16 code units, as in Monaco.
//!   If a change boundary splits a surrogate pair, the resulting part text is
//!   decoded lossily (U+FFFD) since Rust strings must be valid UTF-8.
//! - **`singleTextRemoveCommonPrefix`'s `validModelRange`** parameter is not
//!   ported (only the inline-edit code path uses it).
//! - **The `cachingDiff` cache** is thread-local rather than module-global.

// Some of Monaco's API (replacements, rendering helpers) is kept for
// parity and partly exercised by the tests only.
#![allow(dead_code)]

use std::cell::RefCell;
use std::ops::Range;

use text::PointUtf16;

// ---------------------------------------------------------------------------
// Ghost text model (ghostText.ts)
// ---------------------------------------------------------------------------

/// Ghost text on a single line: one or more insertions into that line's
/// existing text. Port of Monaco's `GhostText`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct GhostText {
    /// 0-based row of the line the ghost text is inserted into.
    pub line: u32,
    /// The insertions, sorted by [`GhostTextPart::column`] (ties keep their
    /// order: a non-preview part precedes the preview part at the same
    /// column).
    pub parts: Vec<GhostTextPart>,
}

/// One insertion of a [`GhostText`]. Port of Monaco's `GhostTextPart`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct GhostTextPart {
    /// 0-based UTF-16 column in the ghost text's line where the text is
    /// inserted.
    pub column: u32,
    /// The inserted text split into lines (on `\r\n`, `\r` or `\n`, like
    /// Monaco's `splitLines`). The first line continues the existing line at
    /// [`column`](Self::column); each following line is a new line. Never
    /// empty for parts built by [`GhostTextPart::new`] (empty text yields one
    /// empty line).
    pub lines: Vec<String>,
    /// Whether this part is a preview of an inline suggestion while a
    /// suggestion (suggest widget item) is previewed — Monaco renders these
    /// in italics.
    pub preview: bool,
}

impl GhostTextPart {
    /// Creates a part inserting `text` at `column`, splitting it into lines.
    pub fn new(column: u32, text: &str, preview: bool) -> Self {
        Self {
            column,
            lines: split_lines(text),
            preview,
        }
    }

    /// The inserted text, lines joined with `\n`.
    pub fn text(&self) -> String {
        self.lines.join("\n")
    }
}

impl GhostText {
    /// Creates ghost text on `line` (0-based) with the given parts.
    ///
    /// Debug-asserts that parts are sorted by column, as Monaco asserts.
    pub fn new(line: u32, parts: Vec<GhostTextPart>) -> Self {
        debug_assert!(parts.windows(2).all(|w| w[0].column <= w[1].column));
        Self { line, parts }
    }

    /// Applies the ghost text to `document_text` (the whole document, lines
    /// separated by `\n`) and returns the result. With `debug`, each part is
    /// wrapped in `[` `]`. Monaco notes this is only used for testing and
    /// debugging.
    pub fn render(&self, document_text: &str, debug: bool) -> String {
        let line_start = line_start_offset(document_text, self.line);
        let line_end = document_text[line_start..]
            .find('\n')
            .map_or(document_text.len(), |i| line_start + i);
        let line = &document_text[line_start..line_end];
        let insertions = self.parts.iter().map(|p| {
            let text = p.lines.join("\n");
            let text = if debug { format!("[{text}]") } else { text };
            (line_start + utf16_col_to_byte(line, p.column), text)
        });
        apply_insertions(document_text, insertions)
    }

    /// Text for screen readers: `line_text` (the ghost text's line) capped at
    /// the last part's column with all parts inserted, starting at the first
    /// part's column. Empty when there are no parts.
    pub fn render_for_screen_reader(&self, line_text: &str) -> String {
        let (Some(first), Some(last)) = (self.parts.first(), self.parts.last()) else {
            return String::new();
        };
        let capped = &line_text[..utf16_col_to_byte(line_text, last.column)];
        let text = apply_insertions(
            capped,
            self.parts
                .iter()
                .map(|p| (utf16_col_to_byte(capped, p.column), p.lines.join("\n"))),
        );
        // The prefix before the first part is unchanged by the insertions.
        text[utf16_col_to_byte(capped, first.column)..].to_string()
    }

    /// Whether no part has any lines. Faithful to Monaco: since parts always
    /// have at least one (possibly empty) line, this is true only when there
    /// are no parts.
    pub fn is_empty(&self) -> bool {
        self.parts.iter().all(|p| p.lines.is_empty())
    }

    /// Number of lines the line occupies with the ghost text applied: 1 plus
    /// the number of line breaks inserted by all parts.
    pub fn line_count(&self) -> u32 {
        1 + self
            .parts
            .iter()
            .map(|p| p.lines.len().saturating_sub(1) as u32)
            .sum::<u32>()
    }

    /// Number of new lines the ghost text adds below its line
    /// (`line_count() - 1`).
    pub fn additional_line_count(&self) -> u32 {
        self.line_count() - 1
    }
}

/// Ghost text that replaces a column range of a line instead of only
/// inserting. Port of Monaco's `GhostTextReplacement`. Not produced by
/// [`compute_ghost_text`]; Monaco's views accept either kind.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct GhostTextReplacement {
    /// 0-based row of the line.
    pub line: u32,
    /// Replaced 0-based UTF-16 column range (end exclusive).
    pub column_range: Range<u32>,
    /// The replacement text.
    pub text: String,
    /// Extra lines to reserve below the line in the view.
    pub additional_reserved_line_count: u32,
    /// A single non-preview part inserting [`text`](Self::text) at the end of
    /// the column range.
    pub parts: Vec<GhostTextPart>,
    /// [`text`](Self::text) split into lines.
    pub new_lines: Vec<String>,
}

impl GhostTextReplacement {
    /// Creates a replacement of `column_range` on `line` by `text`.
    pub fn new(
        line: u32,
        column_range: Range<u32>,
        text: String,
        additional_reserved_line_count: u32,
    ) -> Self {
        let parts = vec![GhostTextPart::new(column_range.end, &text, false)];
        let new_lines = split_lines(&text);
        Self {
            line,
            column_range,
            text,
            additional_reserved_line_count,
            parts,
            new_lines,
        }
    }

    /// Text for screen readers: the new lines joined with `\n`.
    pub fn render_for_screen_reader(&self, _line_text: &str) -> String {
        self.new_lines.join("\n")
    }

    /// Applies the replacement to `document_text`. With `debug`, the replaced
    /// range is wrapped in `(` `)` and kept, followed by `[new text]`.
    pub fn render(&self, document_text: &str, debug: bool) -> String {
        let line_start = line_start_offset(document_text, self.line);
        let line_end = document_text[line_start..]
            .find('\n')
            .map_or(document_text.len(), |i| line_start + i);
        let line = &document_text[line_start..line_end];
        let start = line_start + utf16_col_to_byte(line, self.column_range.start);
        let end = line_start + utf16_col_to_byte(line, self.column_range.end);
        let new_text = self.new_lines.join("\n");
        if debug {
            apply_insertions(
                document_text,
                [(start, "(".to_string()), (end, format!(")[{new_text}]"))],
            )
        } else {
            format!(
                "{}{}{}",
                &document_text[..start],
                new_text,
                &document_text[end..]
            )
        }
    }

    /// Number of lines of the replacement text.
    pub fn line_count(&self) -> u32 {
        self.new_lines.len() as u32
    }

    /// See [`GhostText::is_empty`].
    pub fn is_empty(&self) -> bool {
        self.parts.iter().all(|p| p.lines.is_empty())
    }
}

/// Either kind of ghost text. Port of Monaco's `GhostTextOrReplacement`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum GhostTextOrReplacement {
    /// Insert-only ghost text.
    GhostText(GhostText),
    /// Replacing ghost text.
    Replacement(GhostTextReplacement),
}

// ---------------------------------------------------------------------------
// Single text edit helpers (singleTextEditHelpers.ts)
// ---------------------------------------------------------------------------

/// A replacement of `range` (in the current text's coordinates) by `text`.
/// Port of Monaco's `TextReplacement` as far as ghost text needs it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct TextReplacement {
    /// Replaced range: 0-based rows, 0-based UTF-16 columns.
    pub range: Range<PointUtf16>,
    /// Inserted text.
    pub text: String,
}

/// Removes the common prefix of the replaced text and `edit.text`, moving the
/// range start past it. `edit.text` is normalized from `\r\n` to `\n` first.
/// `line_text(row)` returns a line's content without its EOL.
pub(crate) fn single_text_remove_common_prefix(
    edit: &TextReplacement,
    line_text: &dyn Fn(u32) -> String,
) -> TextReplacement {
    let normalized: Vec<u16> = edit.text.replace("\r\n", "\n").encode_utf16().collect();
    let value_to_replace = value_in_range(line_text, &edit.range);
    let common = common_prefix_length(&value_to_replace, &normalized);
    let start = add_text_length(edit.range.start, &value_to_replace[..common]);
    TextReplacement {
        range: start..edit.range.end,
        text: String::from_utf16_lossy(&normalized[common..]),
    }
}

/// Whether `edit` augments `base`: its text extends `base`'s text and its
/// range starts where `base`'s does and ends at or after it.
pub(crate) fn single_text_edit_augments(edit: &TextReplacement, base: &TextReplacement) -> bool {
    edit.text.starts_with(&base.text)
        && base.range.start == edit.range.start
        && base.range.end <= edit.range.end
}

// ---------------------------------------------------------------------------
// computeGhostText (computeGhostText.ts)
// ---------------------------------------------------------------------------

/// How the ghost text may relate to the existing text. Mirrors Monaco's
/// `'prefix' | 'subword' | 'subwordSmart'` (the `inlineSuggest.mode` /
/// `suggest.previewMode` settings).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum GhostTextMode {
    /// The suggestion must extend the replaced text at its end only (a single
    /// insertion at the end).
    Prefix,
    /// The replaced text must be a subsequence of the suggestion; multiple
    /// insertions are allowed.
    Subword,
    /// Like [`Subword`](Self::Subword), but no insertion may lie before the
    /// cursor on the same line.
    SubwordSmart,
}

/// Computes the ghost text that shows `edit` as pure insertions into the
/// current text, or `None` if it cannot be shown as ghost text (it deletes or
/// changes existing characters, spans lines after its common prefix,
/// violates `mode`, or is too large to diff).
///
/// - `edit`: the suggested replacement, in the current text's coordinates.
/// - `line_text`: returns the content (without EOL) of a 0-based row of the
///   current text. Called only for rows within `edit.range`.
/// - `mode`: see [`GhostTextMode`].
/// - `cursor`: the cursor position; only used by
///   [`GhostTextMode::SubwordSmart`].
/// - `preview_suffix_length`: UTF-16 length of the suffix of the (prefix-
///   trimmed) suggestion text that is a suggest-widget *preview*; parts in it
///   get `preview: true`. If the text is `hello` and the suffix length is 2,
///   the non-preview part is `hel` and the preview part is `lo`.
///
/// Returns ghost text whose `parts` may be empty when the edit inserts
/// nothing.
pub(crate) fn compute_ghost_text(
    edit: &TextReplacement,
    line_text: &dyn Fn(u32) -> String,
    mode: GhostTextMode,
    cursor: Option<PointUtf16>,
    preview_suffix_length: usize,
) -> Option<GhostText> {
    let e = single_text_remove_common_prefix(edit, line_text);
    let mut range = e.range;
    let mut text: Vec<u16> = e.text.encode_utf16().collect();

    if range.end.row != range.start.row {
        // This edit might span multiple lines, but the first lines must be a
        // common prefix.
        return None;
    }

    let source_line: Vec<u16> = line_text(range.start.row).encode_utf16().collect();
    let source_indentation_length = leading_whitespace_len(&source_line);

    let suggestion_touches_indentation = range.start.column as usize <= source_indentation_length;
    if suggestion_touches_indentation {
        // source:      ··········[······abc]
        //                         ^^^^^^^^^ inlineCompletion.range
        //              ^^^^^^^^^^ ^^^^^^ sourceIndentationLength
        //                         ^^^^^^ replacedIndentation.length
        //                               ^^^ rangeThatDoesNotReplaceIndentation
        // inlineCompletion.text: '··foo'
        //                         ^^ suggestionAddedIndentationLength
        let suggestion_added_indentation_length = leading_whitespace_len(&text);

        let replaced_indentation = js_substring(
            &source_line,
            range.start.column as usize,
            source_indentation_length,
        );
        let replaced_len = replaced_indentation.len() as u32;

        let new_start = if range.start.column + replaced_len <= range.end.column {
            PointUtf16::new(range.start.row, range.start.column + replaced_len)
        } else {
            range.end
        };
        range = new_start..range.end;

        text = if text.starts_with(replaced_indentation) {
            // Adds more indentation without changing existing indentation: we
            // can add ghost text for this.
            text[replaced_indentation.len()..].to_vec()
        } else {
            // Changes or removes existing indentation. Only add ghost text for
            // the non-indentation part.
            text[suggestion_added_indentation_length..].to_vec()
        };
    }

    // This is a single line string.
    let value_to_be_replaced = value_in_range(line_text, &range);

    // No ghost text in case the diff would be too slow to compute.
    let changes = caching_diff(&value_to_be_replaced, &text)?;

    let line = range.start.row;
    let mut parts = Vec::new();

    if mode == GhostTextMode::Prefix {
        let mut insertions = changes.iter().filter(|c| c.original_length == 0);
        let first = insertions.next();
        let more = insertions.next().is_some();
        if more || first.is_some_and(|c| c.original_start != value_to_be_replaced.len() as isize) {
            // Prefixes only have a single change.
            return None;
        }
    }

    let preview_start_in_completion_text = text.len() as isize - preview_suffix_length as isize;

    for c in &changes {
        let insert_column = range.start.column as isize + c.original_start + c.original_length;

        if mode == GhostTextMode::SubwordSmart
            && let Some(cursor) = cursor
            && cursor.row == range.start.row
            && insert_column < cursor.column as isize
        {
            // No ghost text before cursor.
            return None;
        }

        if c.original_length > 0 {
            return None;
        }

        if c.modified_length == 0 {
            continue;
        }

        let modified_end = c.modified_start + c.modified_length;
        let non_preview_text_end = c
            .modified_start
            .max(modified_end.min(preview_start_in_completion_text));
        let non_preview_text = &text[c.modified_start as usize..non_preview_text_end as usize];
        let italic_text = &text[non_preview_text_end as usize..modified_end as usize];

        if !non_preview_text.is_empty() {
            parts.push(GhostTextPart::new(
                insert_column as u32,
                &String::from_utf16_lossy(non_preview_text),
                false,
            ));
        }
        if !italic_text.is_empty() {
            parts.push(GhostTextPart::new(
                insert_column as u32,
                &String::from_utf16_lossy(italic_text),
                true,
            ));
        }
    }

    Some(GhostText::new(line, parts))
}

/// A difference between two sequences: `original[original_start..][..
/// original_length]` was replaced by `modified[modified_start..][..
/// modified_length]`. Port of Monaco's `DiffChange` / `IDiffChange`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct DiffChange {
    original_start: isize,
    original_length: isize,
    modified_start: isize,
    modified_length: isize,
}

impl DiffChange {
    fn original_end(&self) -> isize {
        self.original_start + self.original_length
    }

    fn modified_end(&self) -> isize {
        self.modified_start + self.modified_length
    }
}

/// The last `caching_diff` request: original, new, result.
type DiffCacheEntry = (Vec<u16>, Vec<u16>, Option<Vec<DiffChange>>);

thread_local! {
    static LAST_DIFF_REQUEST: RefCell<Option<DiffCacheEntry>> = const { RefCell::new(None) };
}

/// [`smart_diff`] with bracket matching, falling back to the plain diff when
/// that deletes fewer characters; the last request is cached.
fn caching_diff(original: &[u16], new: &[u16]) -> Option<Vec<DiffChange>> {
    if let Some(changes) = LAST_DIFF_REQUEST.with_borrow(|last| {
        last.as_ref()
            .filter(|(o, n, _)| o == original && n == new)
            .map(|(_, _, changes)| changes.clone())
    }) {
        return changes;
    }
    let mut changes = smart_diff(original, new, true);
    if let Some(smart) = &changes {
        let deleted_chars = deleted_characters(smart);
        if deleted_chars > 0 {
            // For performance reasons, don't compute diff if there is nothing
            // to improve.
            let new_changes = smart_diff(original, new, false);
            if let Some(new_changes) = new_changes
                && deleted_characters(&new_changes) < deleted_chars
            {
                // Disabling smartness seems to be better here.
                changes = Some(new_changes);
            }
        }
    }
    LAST_DIFF_REQUEST.set(Some((original.to_vec(), new.to_vec(), changes.clone())));
    changes
}

/// Total number of original elements removed by `changes`.
fn deleted_characters(changes: &[DiffChange]) -> isize {
    changes.iter().map(|c| c.original_length).sum()
}

/// Character diff of two UTF-16 strings, `None` if either exceeds 5000 code
/// units.
///
/// When matching `if ()` with `if (f() = 1) { g(); }`, align it like this:
/// `if (       )`, not like this: `if (  )`, also not like this:
/// `if (             )`. With `smart_bracket_matching`, parentheses are
/// preprocessed into unique symbols per (group, nesting level) so they match
/// correctly.
fn smart_diff(
    original: &[u16],
    new: &[u16],
    smart_bracket_matching: bool,
) -> Option<Vec<DiffChange>> {
    if original.len() > 5000 || new.len() > 5000 {
        // We don't want to work on strings that are too big.
        return None;
    }

    let max_char_code = original
        .iter()
        .chain(new)
        .copied()
        .max()
        .map_or(0, i32::from);
    let unique_char_code = |id: i32| max_char_code + id + 1;

    let elements = |source: &[u16]| -> Vec<i32> {
        let mut level = 0;
        let mut group = 0;
        source
            .iter()
            .map(|&ch| {
                // TODO (Monaco): support more brackets.
                if smart_bracket_matching && ch == u16::from(b'(') {
                    let id = group * 100 + level;
                    level += 1;
                    unique_char_code(2 * id)
                } else if smart_bracket_matching && ch == u16::from(b')') {
                    level = (level - 1).max(0);
                    let id = group * 100 + level;
                    if level == 0 {
                        group += 1;
                    }
                    unique_char_code(2 * id + 1)
                } else {
                    i32::from(ch)
                }
            })
            .collect()
    };

    Some(LcsDiff::new(elements(original), elements(new)).compute_diff())
}

// ---------------------------------------------------------------------------
// LcsDiff (vs/base/common/diff/diff.ts), pretty = false, no predicate
// ---------------------------------------------------------------------------

const MAX_SAFE_SMALL_INTEGER: isize = 1 << 30;
const MIN_SAFE_SMALL_INTEGER: isize = -(1 << 30);

/// Total memory for history is (worst case)
/// `2 * [(MAX_DIFFERENCES_HISTORY + 1)^2 - 1] * sizeof(int)` ≈ 16MB.
const MAX_DIFFERENCES_HISTORY: isize = 1447;

/// Accumulates original/modified elements into [`DiffChange`]s;
/// [`mark_next_change`](Self::mark_next_change) separates changes.
struct DiffChangeHelper {
    changes: Vec<DiffChange>,
    original_start: isize,
    modified_start: isize,
    original_count: isize,
    modified_count: isize,
}

impl DiffChangeHelper {
    fn new() -> Self {
        Self {
            changes: Vec::new(),
            original_start: MAX_SAFE_SMALL_INTEGER,
            modified_start: MAX_SAFE_SMALL_INTEGER,
            original_count: 0,
            modified_count: 0,
        }
    }

    fn mark_next_change(&mut self) {
        if self.original_count > 0 || self.modified_count > 0 {
            self.changes.push(DiffChange {
                original_start: self.original_start,
                original_length: self.original_count,
                modified_start: self.modified_start,
                modified_length: self.modified_count,
            });
        }
        self.original_count = 0;
        self.modified_count = 0;
        self.original_start = MAX_SAFE_SMALL_INTEGER;
        self.modified_start = MAX_SAFE_SMALL_INTEGER;
    }

    fn add_original_element(&mut self, original_index: isize, modified_index: isize) {
        self.original_start = self.original_start.min(original_index);
        self.modified_start = self.modified_start.min(modified_index);
        self.original_count += 1;
    }

    fn add_modified_element(&mut self, original_index: isize, modified_index: isize) {
        self.original_start = self.original_start.min(original_index);
        self.modified_start = self.modified_start.min(modified_index);
        self.modified_count += 1;
    }

    fn into_changes(mut self) -> Vec<DiffChange> {
        if self.original_count > 0 || self.modified_count > 0 {
            self.mark_next_change();
        }
        self.changes
    }

    fn into_reverse_changes(self) -> Vec<DiffChange> {
        let mut changes = self.into_changes();
        changes.reverse();
        changes
    }
}

/// Myers' O(ND) difference algorithm ("An O(ND) Difference Algorithm and its
/// variations"), as implemented by Monaco's `LcsDiff` (itself ported from
/// C#). Elements are compared by value.
struct LcsDiff {
    original: Vec<i32>,
    modified: Vec<i32>,
    forward_history: Vec<Vec<isize>>,
    reverse_history: Vec<Vec<isize>>,
}

/// Arguments of `WALKTRACE` that describe the diagonal state at the overlap.
struct WalkTraceState<'a> {
    diagonal_forward_base: isize,
    diagonal_forward_start: isize,
    diagonal_forward_end: isize,
    diagonal_forward_offset: isize,
    diagonal_reverse_base: isize,
    diagonal_reverse_start: isize,
    diagonal_reverse_end: isize,
    diagonal_reverse_offset: isize,
    forward_points: &'a [isize],
    reverse_points: &'a [isize],
    mid_original: isize,
    mid_modified: isize,
    delta_is_even: bool,
}

impl LcsDiff {
    fn new(original: Vec<i32>, modified: Vec<i32>) -> Self {
        Self {
            original,
            modified,
            forward_history: Vec::new(),
            reverse_history: Vec::new(),
        }
    }

    fn elements_are_equal(&self, original_index: isize, modified_index: isize) -> bool {
        self.original[original_index as usize] == self.modified[modified_index as usize]
    }

    fn compute_diff(&mut self) -> Vec<DiffChange> {
        let original_end = self.original.len() as isize - 1;
        let modified_end = self.modified.len() as isize - 1;
        self.compute_diff_recursive(0, original_end, 0, modified_end)
    }

    /// Computes the differences on the inclusive bounded range recursively.
    fn compute_diff_recursive(
        &mut self,
        mut original_start: isize,
        mut original_end: isize,
        mut modified_start: isize,
        mut modified_end: isize,
    ) -> Vec<DiffChange> {
        // Find the start of the differences.
        while original_start <= original_end
            && modified_start <= modified_end
            && self.elements_are_equal(original_start, modified_start)
        {
            original_start += 1;
            modified_start += 1;
        }

        // Find the end of the differences.
        while original_end >= original_start
            && modified_end >= modified_start
            && self.elements_are_equal(original_end, modified_end)
        {
            original_end -= 1;
            modified_end -= 1;
        }

        // All insertions, all deletions, or identical sequences.
        if original_start > original_end || modified_start > modified_end {
            if modified_start <= modified_end {
                debug_assert_eq!(original_start, original_end + 1);
                return vec![DiffChange {
                    original_start,
                    original_length: 0,
                    modified_start,
                    modified_length: modified_end - modified_start + 1,
                }];
            } else if original_start <= original_end {
                debug_assert_eq!(modified_start, modified_end + 1);
                return vec![DiffChange {
                    original_start,
                    original_length: original_end - original_start + 1,
                    modified_start,
                    modified_length: 0,
                }];
            } else {
                return Vec::new();
            }
        }

        // Divide and conquer.
        match self.compute_recursion_point(
            original_start,
            original_end,
            modified_start,
            modified_end,
        ) {
            Ok(changes) => changes,
            Err((mid_original, mid_modified)) => {
                // First half: (original_start, modified_start) to
                // (mid_original, mid_modified). Second half: the next point
                // to (original_end, modified_end). Ranges are inclusive.
                let left = self.compute_diff_recursive(
                    original_start,
                    mid_original,
                    modified_start,
                    mid_modified,
                );
                let right = self.compute_diff_recursive(
                    mid_original + 1,
                    original_end,
                    mid_modified + 1,
                    modified_end,
                );
                concatenate_changes(left, right)
            }
        }
    }

    /// Walks the forward and reverse histories back from the overlap point to
    /// produce the change list (Monaco's `WALKTRACE`).
    fn walk_trace(&self, s: WalkTraceState<'_>) -> Vec<DiffChange> {
        // First, walk backward through the forward diagonals history.
        let mut forward_points = s.forward_points;
        let mut diagonal_forward_base = s.diagonal_forward_base;
        let mut change_helper = DiffChangeHelper::new();
        let mut diagonal_min = s.diagonal_forward_start;
        let mut diagonal_max = s.diagonal_forward_end;
        let mut diagonal_relative = (s.mid_original - s.mid_modified) - s.diagonal_forward_offset;
        let mut last_original_index = MIN_SAFE_SMALL_INTEGER;
        let mut history_index = self.forward_history.len() as isize - 1;

        loop {
            let diagonal = diagonal_relative + diagonal_forward_base;
            let d = diagonal as usize;

            if diagonal == diagonal_min
                || (diagonal < diagonal_max && forward_points[d - 1] < forward_points[d + 1])
            {
                // Vertical line (the element is an insert).
                let original_index = forward_points[d + 1];
                let modified_index = original_index - diagonal_relative - s.diagonal_forward_offset;
                if original_index < last_original_index {
                    change_helper.mark_next_change();
                }
                last_original_index = original_index;
                change_helper.add_modified_element(original_index + 1, modified_index);
                diagonal_relative = (diagonal + 1) - diagonal_forward_base;
            } else {
                // Horizontal line (the element is a deletion).
                let original_index = forward_points[d - 1] + 1;
                let modified_index = original_index - diagonal_relative - s.diagonal_forward_offset;
                if original_index < last_original_index {
                    change_helper.mark_next_change();
                }
                last_original_index = original_index - 1;
                change_helper.add_original_element(original_index, modified_index + 1);
                diagonal_relative = (diagonal - 1) - diagonal_forward_base;
            }

            if history_index >= 0 {
                forward_points = &self.forward_history[history_index as usize];
                diagonal_forward_base = forward_points[0];
                diagonal_min = 1;
                diagonal_max = forward_points.len() as isize - 1;
            }
            history_index -= 1;
            if history_index < -1 {
                break;
            }
        }

        // We added the forward changes backwards.
        let forward_changes = change_helper.into_reverse_changes();

        // Now walk backward through the reverse diagonals history.
        let mut reverse_points = s.reverse_points;
        let mut diagonal_reverse_base = s.diagonal_reverse_base;
        let mut change_helper = DiffChangeHelper::new();
        diagonal_min = s.diagonal_reverse_start;
        diagonal_max = s.diagonal_reverse_end;
        diagonal_relative = (s.mid_original - s.mid_modified) - s.diagonal_reverse_offset;
        last_original_index = MAX_SAFE_SMALL_INTEGER;
        history_index = if s.delta_is_even {
            self.reverse_history.len() as isize - 1
        } else {
            self.reverse_history.len() as isize - 2
        };

        loop {
            let diagonal = diagonal_relative + diagonal_reverse_base;
            let d = diagonal as usize;

            if diagonal == diagonal_min
                || (diagonal < diagonal_max && reverse_points[d - 1] >= reverse_points[d + 1])
            {
                // Horizontal line (the element is a deletion).
                let original_index = reverse_points[d + 1] - 1;
                let modified_index = original_index - diagonal_relative - s.diagonal_reverse_offset;
                if original_index > last_original_index {
                    change_helper.mark_next_change();
                }
                last_original_index = original_index + 1;
                change_helper.add_original_element(original_index + 1, modified_index + 1);
                diagonal_relative = (diagonal + 1) - diagonal_reverse_base;
            } else {
                // Vertical line (the element is an insertion).
                let original_index = reverse_points[d - 1];
                let modified_index = original_index - diagonal_relative - s.diagonal_reverse_offset;
                if original_index > last_original_index {
                    change_helper.mark_next_change();
                }
                last_original_index = original_index;
                change_helper.add_modified_element(original_index + 1, modified_index + 1);
                diagonal_relative = (diagonal - 1) - diagonal_reverse_base;
            }

            if history_index >= 0 {
                reverse_points = &self.reverse_history[history_index as usize];
                diagonal_reverse_base = reverse_points[0];
                diagonal_min = 1;
                diagonal_max = reverse_points.len() as isize - 1;
            }
            history_index -= 1;
            if history_index < -1 {
                break;
            }
        }

        let reverse_changes = change_helper.into_changes();
        concatenate_changes(forward_changes, reverse_changes)
    }

    /// Finds the point `(mid_original, mid_modified)` in the middle of the
    /// LCS where the problem can be split. If the full trace is in memory
    /// when it is found, returns the change list (`Ok`); otherwise returns
    /// the recursion point (`Err`).
    fn compute_recursion_point(
        &mut self,
        mut original_start: isize,
        original_end: isize,
        mut modified_start: isize,
        modified_end: isize,
    ) -> Result<Vec<DiffChange>, (isize, isize)> {
        let mut original_index: isize = 0;
        let mut modified_index: isize = 0;
        let mut diagonal_forward_start: isize = 0;
        let mut diagonal_forward_end: isize = 0;
        let mut diagonal_reverse_start: isize = 0;
        let mut diagonal_reverse_end: isize = 0;

        // To traverse the edit graph and produce the proper LCS, our actual
        // start position is just outside the given boundary.
        original_start -= 1;
        modified_start -= 1;

        self.forward_history.clear();
        self.reverse_history.clear();

        // Each cell corresponds to a diagonal in the edit graph and holds the
        // original index of the furthest reaching point on that diagonal.
        let max_differences = (original_end - original_start) + (modified_end - modified_start);
        let num_diagonals = max_differences + 1;
        let mut forward_points = vec![0isize; num_diagonals as usize];
        let mut reverse_points = vec![0isize; num_diagonals as usize];
        // Index of the diagonal through (original_start, modified_start) /
        // (original_end, modified_end).
        let diagonal_forward_base = modified_end - modified_start;
        let diagonal_reverse_base = original_end - original_start;
        // Offsets to compute modified_index from original_index and diagonal.
        let diagonal_forward_offset = original_start - modified_start;
        let diagonal_reverse_offset = original_end - modified_end;

        let delta = diagonal_reverse_base - diagonal_forward_base;
        let delta_is_even = delta % 2 == 0;

        forward_points[diagonal_forward_base as usize] = original_start;
        reverse_points[diagonal_reverse_base as usize] = original_end;

        macro_rules! walk_trace {
            ($mid_original:expr, $mid_modified:expr) => {
                self.walk_trace(WalkTraceState {
                    diagonal_forward_base,
                    diagonal_forward_start,
                    diagonal_forward_end,
                    diagonal_forward_offset,
                    diagonal_reverse_base,
                    diagonal_reverse_start,
                    diagonal_reverse_end,
                    diagonal_reverse_offset,
                    forward_points: &forward_points,
                    reverse_points: &reverse_points,
                    mid_original: $mid_original,
                    mid_modified: $mid_modified,
                    delta_is_even,
                })
            };
        }

        let mut num_differences: isize = 1;
        while num_differences <= max_differences / 2 + 1 {
            // Run the algorithm in the forward direction.
            diagonal_forward_start = clip_diagonal_bound(
                diagonal_forward_base - num_differences,
                num_differences,
                diagonal_forward_base,
                num_diagonals,
            );
            diagonal_forward_end = clip_diagonal_bound(
                diagonal_forward_base + num_differences,
                num_differences,
                diagonal_forward_base,
                num_diagonals,
            );
            let mut diagonal = diagonal_forward_start;
            while diagonal <= diagonal_forward_end {
                let d = diagonal as usize;
                // STEP 1: extend from the neighbouring diagonal that reaches
                // further.
                if diagonal == diagonal_forward_start
                    || (diagonal < diagonal_forward_end
                        && forward_points[d - 1] < forward_points[d + 1])
                {
                    original_index = forward_points[d + 1];
                } else {
                    original_index = forward_points[d - 1] + 1;
                }
                modified_index =
                    original_index - (diagonal - diagonal_forward_base) - diagonal_forward_offset;

                let temp_original_index = original_index;

                // STEP 2: follow the snake.
                while original_index < original_end
                    && modified_index < modified_end
                    && self.elements_are_equal(original_index + 1, modified_index + 1)
                {
                    original_index += 1;
                    modified_index += 1;
                }
                forward_points[d] = original_index;

                // STEP 3: if delta is odd, check for overlap with the reverse
                // diagonals of the previous iteration.
                if !delta_is_even
                    && (diagonal - diagonal_reverse_base).abs() < num_differences
                    && original_index >= reverse_points[d]
                {
                    if temp_original_index <= reverse_points[d]
                        && MAX_DIFFERENCES_HISTORY > 0
                        && num_differences <= MAX_DIFFERENCES_HISTORY + 1
                    {
                        // We overlapped, and we have the full trace in memory.
                        return Ok(walk_trace!(original_index, modified_index));
                    } else {
                        // False overlap, or not enough memory for the full
                        // trace: just return the recursion point.
                        return Err((original_index, modified_index));
                    }
                }
                diagonal += 2;
            }

            // (Monaco checks its continue-processing predicate here; none is
            // used for ghost text.)

            // Run the algorithm in the reverse direction.
            diagonal_reverse_start = clip_diagonal_bound(
                diagonal_reverse_base - num_differences,
                num_differences,
                diagonal_reverse_base,
                num_diagonals,
            );
            diagonal_reverse_end = clip_diagonal_bound(
                diagonal_reverse_base + num_differences,
                num_differences,
                diagonal_reverse_base,
                num_diagonals,
            );
            let mut diagonal = diagonal_reverse_start;
            while diagonal <= diagonal_reverse_end {
                let d = diagonal as usize;
                // STEP 1.
                if diagonal == diagonal_reverse_start
                    || (diagonal < diagonal_reverse_end
                        && reverse_points[d - 1] >= reverse_points[d + 1])
                {
                    original_index = reverse_points[d + 1] - 1;
                } else {
                    original_index = reverse_points[d - 1];
                }
                modified_index =
                    original_index - (diagonal - diagonal_reverse_base) - diagonal_reverse_offset;

                let temp_original_index = original_index;

                // STEP 2.
                while original_index > original_start
                    && modified_index > modified_start
                    && self.elements_are_equal(original_index, modified_index)
                {
                    original_index -= 1;
                    modified_index -= 1;
                }
                reverse_points[d] = original_index;

                // STEP 4: if delta is even, check for overlap with the forward
                // diagonals of this iteration.
                if delta_is_even
                    && (diagonal - diagonal_forward_base).abs() <= num_differences
                    && original_index <= forward_points[d]
                {
                    if temp_original_index >= forward_points[d]
                        && MAX_DIFFERENCES_HISTORY > 0
                        && num_differences <= MAX_DIFFERENCES_HISTORY + 1
                    {
                        return Ok(walk_trace!(original_index, modified_index));
                    } else {
                        return Err((original_index, modified_index));
                    }
                }
                diagonal += 2;
            }

            // Save current vectors to history before the next iteration; the
            // first slot holds the diagonal base index.
            if num_differences <= MAX_DIFFERENCES_HISTORY {
                let start = diagonal_forward_start as usize;
                let end = diagonal_forward_end as usize;
                let mut temp = Vec::with_capacity(end - start + 2);
                temp.push(diagonal_forward_base - diagonal_forward_start + 1);
                temp.extend_from_slice(&forward_points[start..=end]);
                self.forward_history.push(temp);

                let start = diagonal_reverse_start as usize;
                let end = diagonal_reverse_end as usize;
                let mut temp = Vec::with_capacity(end - start + 2);
                temp.push(diagonal_reverse_base - diagonal_reverse_start + 1);
                temp.extend_from_slice(&reverse_points[start..=end]);
                self.reverse_history.push(temp);
            }

            num_differences += 1;
        }

        // We have the full trace in history; convert it to a change list.
        Ok(walk_trace!(original_index, modified_index))
    }
}

/// Concatenates two change lists, fusing the boundary changes if they
/// overlap (recursion may split a change in two).
fn concatenate_changes(mut left: Vec<DiffChange>, right: Vec<DiffChange>) -> Vec<DiffChange> {
    if left.is_empty() || right.is_empty() {
        return if right.is_empty() { left } else { right };
    }
    let mut right = right.into_iter();
    let first_right = right.next().expect("non-empty");
    let last_left = left.last_mut().expect("non-empty");
    if let Some(merged) = changes_overlap(last_left, &first_right) {
        *last_left = merged;
    } else {
        left.push(first_right);
    }
    left.extend(right);
    left
}

/// The merge of two changes if they overlap or touch.
fn changes_overlap(left: &DiffChange, right: &DiffChange) -> Option<DiffChange> {
    debug_assert!(left.original_start <= right.original_start);
    debug_assert!(left.modified_start <= right.modified_start);

    let original_touches = left.original_end() >= right.original_start;
    let modified_touches = left.modified_end() >= right.modified_start;
    if !(original_touches || modified_touches) {
        return None;
    }
    Some(DiffChange {
        original_start: left.original_start,
        original_length: if original_touches {
            right.original_end() - left.original_start
        } else {
            left.original_length
        },
        modified_start: left.modified_start,
        modified_length: if modified_touches {
            right.modified_end() - left.modified_start
        } else {
            left.modified_length
        },
    })
}

/// Clips a diagonal index to the valid range, to the boundary or one inside
/// it depending on the parity of the boundary and `num_differences`.
fn clip_diagonal_bound(
    diagonal: isize,
    num_differences: isize,
    diagonal_base_index: isize,
    num_diagonals: isize,
) -> isize {
    if diagonal >= 0 && diagonal < num_diagonals {
        return diagonal;
    }
    let diagonals_below = diagonal_base_index;
    let diagonals_above = num_diagonals - diagonal_base_index - 1;
    let diff_even = num_differences % 2 == 0;
    if diagonal < 0 {
        let lower_bound_even = diagonals_below % 2 == 0;
        if diff_even == lower_bound_even { 0 } else { 1 }
    } else {
        let upper_bound_even = diagonals_above % 2 == 0;
        if diff_even == upper_bound_even {
            num_diagonals - 1
        } else {
            num_diagonals - 2
        }
    }
}

// ---------------------------------------------------------------------------
// String / position helpers
// ---------------------------------------------------------------------------

/// Monaco's `splitLines`: splits on `\r\n`, `\r` or `\n`.
fn split_lines(text: &str) -> Vec<String> {
    let mut lines = Vec::new();
    let mut rest = text;
    while let Some(i) = rest.find(['\r', '\n']) {
        lines.push(rest[..i].to_string());
        let skip = if rest[i..].starts_with("\r\n") { 2 } else { 1 };
        rest = &rest[i + skip..];
    }
    lines.push(rest.to_string());
    lines
}

/// Length of the leading run of spaces and tabs (Monaco's
/// `getLeadingWhitespace(...).length`).
fn leading_whitespace_len(s: &[u16]) -> usize {
    s.iter()
        .take_while(|&&c| c == u16::from(b' ') || c == u16::from(b'\t'))
        .count()
}

/// Number of equal leading code units.
fn common_prefix_length(a: &[u16], b: &[u16]) -> usize {
    a.iter().zip(b).take_while(|(x, y)| x == y).count()
}

/// JS `String.prototype.substring`: clamps both indices and swaps them if
/// reversed.
fn js_substring(s: &[u16], start: usize, end: usize) -> &[u16] {
    let (start, end) = (start.min(s.len()), end.min(s.len()));
    let (start, end) = if start <= end {
        (start, end)
    } else {
        (end, start)
    };
    &s[start..end]
}

/// The text in `range` as UTF-16, lines joined with `\n` (Monaco's
/// `getValueInRange(range, EndOfLinePreference.LF)`). Columns are clamped to
/// the line lengths.
fn value_in_range(line_text: &dyn Fn(u32) -> String, range: &Range<PointUtf16>) -> Vec<u16> {
    let mut result = Vec::new();
    for row in range.start.row..=range.end.row {
        let line: Vec<u16> = line_text(row).encode_utf16().collect();
        let start = if row == range.start.row {
            (range.start.column as usize).min(line.len())
        } else {
            0
        };
        let end = if row == range.end.row {
            (range.end.column as usize).min(line.len())
        } else {
            line.len()
        };
        if row != range.start.row {
            result.push(u16::from(b'\n'));
        }
        result.extend_from_slice(&line[start..end.max(start)]);
    }
    result
}

/// `TextLength.ofText(text).addToPosition(position)`: the position after
/// `text` (UTF-16, `\n`-separated) when it starts at `position`.
fn add_text_length(position: PointUtf16, text: &[u16]) -> PointUtf16 {
    match text.iter().rposition(|&c| c == u16::from(b'\n')) {
        None => PointUtf16::new(position.row, position.column + text.len() as u32),
        Some(last_newline) => {
            let line_breaks = text.iter().filter(|&&c| c == u16::from(b'\n')).count() as u32;
            PointUtf16::new(
                position.row + line_breaks,
                (text.len() - last_newline - 1) as u32,
            )
        }
    }
}

/// Byte offset of the start of 0-based `row` in `text` (lines split on
/// `\n`); the end of the text if there are fewer rows.
fn line_start_offset(text: &str, row: u32) -> usize {
    let mut offset = 0;
    for _ in 0..row {
        match text[offset..].find('\n') {
            Some(i) => offset += i + 1,
            None => return text.len(),
        }
    }
    offset
}

/// Byte offset of UTF-16 `column` in `line`, clamped to the line's length
/// (and rounded up past a split surrogate pair).
fn utf16_col_to_byte(line: &str, column: u32) -> usize {
    let mut utf16 = 0;
    for (byte, ch) in line.char_indices() {
        if utf16 >= column as usize {
            return byte;
        }
        utf16 += ch.len_utf16();
    }
    line.len()
}

/// Inserts each `(byte offset, text)` into `base`. Offsets must be
/// non-decreasing; insertions at the same offset keep their order.
fn apply_insertions(base: &str, insertions: impl IntoIterator<Item = (usize, String)>) -> String {
    let mut result = String::with_capacity(base.len());
    let mut last = 0;
    for (offset, text) in insertions {
        debug_assert!(offset >= last);
        result.push_str(&base[last..offset]);
        result.push_str(&text);
        last = offset;
    }
    result.push_str(&base[last..]);
    result
}

#[cfg(test)]
mod tests {
    //! Ported from `computeGhostText.test.ts`.

    use super::*;

    /// Result of [`get_output`]: one value when `prefix` and `subword` modes
    /// agree, both otherwise (Monaco's `getOutput` returns either the shared
    /// value or a `{ prefix, subword }` object).
    #[derive(Debug, PartialEq)]
    enum Output {
        Same(Option<String>),
        Modes {
            prefix: Option<String>,
            subword: Option<String>,
        },
    }

    fn same(rendered: Option<&str>) -> Output {
        Output::Same(rendered.map(str::to_string))
    }

    fn modes(prefix: Option<&str>, subword: Option<&str>) -> Output {
        Output::Modes {
            prefix: prefix.map(str::to_string),
            subword: subword.map(str::to_string),
        }
    }

    /// Converts a UTF-16 offset into `text` to a position (Monaco's
    /// `getPositionAt`, `\n` line breaks).
    fn position_at(text: &str, offset: usize) -> PointUtf16 {
        let units: Vec<u16> = text.encode_utf16().collect();
        add_text_length(PointUtf16::new(0, 0), &units[..offset.min(units.len())])
    }

    /// `text` marks the replaced range with `[` and `]`; `suggestion` is the
    /// replacement text. Returns the debug rendering of the ghost text for
    /// the `prefix` and `subword` modes.
    fn get_output(text: &str, suggestion: &str) -> Output {
        let units: Vec<u16> = text.encode_utf16().collect();
        let range_start_offset = units.iter().position(|&c| c == u16::from(b'[')).unwrap();
        let range_end_offset = units.iter().position(|&c| c == u16::from(b']')).unwrap() - 1;
        let cleaned = text.replacen('[', "", 1).replacen(']', "", 1);
        let range =
            position_at(&cleaned, range_start_offset)..position_at(&cleaned, range_end_offset);
        let lines: Vec<String> = cleaned.split('\n').map(str::to_string).collect();
        let line_text = |row: u32| lines.get(row as usize).cloned().unwrap_or_default();
        let edit = TextReplacement {
            range,
            text: suggestion.to_string(),
        };
        let mut results = [GhostTextMode::Prefix, GhostTextMode::Subword].map(|mode| {
            compute_ghost_text(&edit, &line_text, mode, None, 0).map(|g| g.render(&cleaned, true))
        });
        if results[0] == results[1] {
            Output::Same(results[0].take())
        } else {
            let [prefix, subword] = results;
            Output::Modes { prefix, subword }
        }
    }

    #[test]
    fn basic() {
        assert_eq!(get_output("[foo]baz", "foobar"), same(Some("foo[bar]baz")));
        assert_eq!(get_output("[aaa]aaa", "aaaaaa"), same(Some("aaa[aaa]aaa")));
        assert_eq!(get_output("[foo]baz", "boobar"), same(None));
        assert_eq!(get_output("[foo]foo", "foofoo"), same(Some("foo[foo]foo")));
        assert_eq!(
            get_output("foo[]", "bar\nhello"),
            same(Some("foo[bar\nhello]"))
        );
    }

    #[test]
    fn empty_ghost_text() {
        assert_eq!(get_output("[foo]", "foo"), same(Some("foo")));
    }

    #[test]
    fn whitespace_indentation() {
        assert_eq!(get_output("[ foo]", "foobar"), same(Some(" foo[bar]")));
        assert_eq!(get_output("[\tfoo]", "foobar"), same(Some("\tfoo[bar]")));
        assert_eq!(
            get_output("[\t foo]", "\tfoobar"),
            same(Some("\t foo[bar]"))
        );
        assert_eq!(
            get_output("[\tfoo]", "\t\tfoobar"),
            modes(None, Some("\t[\t]foo[bar]"))
        );
        assert_eq!(get_output("[\t]", "\t\tfoobar"), same(Some("\t[\tfoobar]")));
        assert_eq!(get_output("\t[]", "\t"), same(Some("\t[\t]")));
        assert_eq!(get_output("\t[\t]", ""), same(Some("\t\t")));

        assert_eq!(get_output("[ ]", "return 1"), same(Some(" [return 1]")));
    }

    #[test]
    fn whitespace_outside_of_indentation() {
        assert_eq!(get_output("bar[ foo]", "foobar"), same(None));
        assert_eq!(get_output("bar[\tfoo]", "foobar"), same(None));
    }

    #[test]
    fn unsupported_case() {
        assert_eq!(get_output("fo[o\n]", "x\nbar"), same(None));
    }

    #[test]
    fn new_line() {
        assert_eq!(get_output("fo[o\n]", "o\nbar"), same(Some("foo\n[bar]")));
    }

    #[test]
    fn multi_part_diffing() {
        assert_eq!(
            get_output("foo[()]", "(x);"),
            modes(None, Some("foo([x])[;]"))
        );
        assert_eq!(
            get_output("[\tfoo]", "\t\tfoobar"),
            modes(None, Some("\t[\t]foo[bar]"))
        );
        assert_eq!(
            get_output("[(y ===)]", "(y === 1) { f(); }"),
            modes(None, Some("(y ===[ 1])[ { f(); }]"))
        );
        assert_eq!(
            get_output("[(y ==)]", "(y === 1) { f(); }"),
            modes(None, Some("(y ==[= 1])[ { f(); }]"))
        );

        assert_eq!(
            get_output("[(y ==)]", "(y === 1) { f(); }"),
            modes(None, Some("(y ==[= 1])[ { f(); }]"))
        );
    }

    #[test]
    fn multi_part_diffing_1() {
        assert_eq!(
            get_output("[if () ()]", "if (1 == f()) ()"),
            modes(None, Some("if ([1 == f()]) ()"))
        );
    }

    #[test]
    fn multi_part_diffing_2() {
        assert_eq!(get_output("[)]", "())"), modes(None, Some("[(])[)]")));
        assert_eq!(get_output("[))]", "(())"), modes(None, Some("[((]))")));
    }

    #[test]
    fn parenthesis_matching() {
        assert_eq!(
            get_output("[console.log()]", "console.log({ label: \"(\" })"),
            modes(None, Some("console.log([{ label: \"(\" }])"))
        );
    }

    // --- Additional tests (not in Monaco's suite) for the Rust API. ---

    #[test]
    fn preview_suffix_splits_parts() {
        let lines = ["hel".to_string()];
        let line_text = |row: u32| lines[row as usize].clone();
        let edit = TextReplacement {
            range: PointUtf16::new(0, 0)..PointUtf16::new(0, 3),
            text: "hello world".into(),
        };
        let ghost = compute_ghost_text(&edit, &line_text, GhostTextMode::Prefix, None, 6).unwrap();
        assert_eq!(
            ghost.parts,
            vec![
                GhostTextPart::new(3, "lo", false),
                GhostTextPart::new(3, " world", true),
            ]
        );
        assert_eq!(ghost.render("hel", true), "hel[lo][ world]");
        assert_eq!(ghost.render_for_screen_reader("hel"), "lo world");
    }

    #[test]
    fn subword_smart_rejects_ghost_text_before_cursor() {
        let lines = ["fo".to_string()];
        let line_text = |row: u32| lines[row as usize].clone();
        let edit = TextReplacement {
            range: PointUtf16::new(0, 0)..PointUtf16::new(0, 2),
            text: "f(o)".into(),
        };
        let cursor = Some(PointUtf16::new(0, 2));
        assert!(
            compute_ghost_text(&edit, &line_text, GhostTextMode::SubwordSmart, cursor, 0).is_none()
        );
        assert!(compute_ghost_text(&edit, &line_text, GhostTextMode::Subword, cursor, 0).is_some());
    }

    #[test]
    fn line_counts() {
        let ghost = GhostText::new(
            0,
            vec![
                GhostTextPart::new(1, "a\nb", false),
                GhostTextPart::new(2, "c\r\nd\re", false),
            ],
        );
        assert_eq!(ghost.line_count(), 4);
        assert_eq!(ghost.additional_line_count(), 3);
        assert!(!ghost.is_empty());
        assert!(GhostText::new(0, Vec::new()).is_empty());
    }

    #[test]
    fn replacement_render() {
        let r = GhostTextReplacement::new(1, 1..3, "XY\nZ".into(), 0);
        assert_eq!(r.render("a\nbcde", true), "a\nb(cd)[XY\nZ]e");
        assert_eq!(r.render("a\nbcde", false), "a\nbXY\nZe");
        assert_eq!(r.line_count(), 2);
        assert_eq!(r.parts, vec![GhostTextPart::new(3, "XY\nZ", false)]);
    }

    #[test]
    fn utf16_columns() {
        // "é" is one UTF-16 unit, "😀" two.
        let lines = ["😀é".to_string()];
        let line_text = |row: u32| lines[row as usize].clone();
        let edit = TextReplacement {
            range: PointUtf16::new(0, 0)..PointUtf16::new(0, 3),
            text: "😀éx".into(),
        };
        let ghost = compute_ghost_text(&edit, &line_text, GhostTextMode::Prefix, None, 0).unwrap();
        assert_eq!(ghost.parts, vec![GhostTextPart::new(3, "x", false)]);
        assert_eq!(ghost.render("😀é", true), "😀é[x]");
    }

    /// Checks that the LcsDiff port yields ordered, non-overlapping changes
    /// that transform the original into the modified sequence and keep a
    /// common subsequence of the expected (LCS) length on small inputs.
    #[test]
    fn lcs_diff_changes_are_valid() {
        let mut seed: u64 = 0x2545_f491_4f6c_dd1d;
        let mut next = move |n: u64| {
            seed = seed
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            (seed >> 33) % n
        };
        let lcs_len = |a: &[i32], b: &[i32]| {
            let mut dp = vec![vec![0usize; b.len() + 1]; a.len() + 1];
            for i in 0..a.len() {
                for j in 0..b.len() {
                    dp[i + 1][j + 1] = if a[i] == b[j] {
                        dp[i][j] + 1
                    } else {
                        dp[i][j + 1].max(dp[i + 1][j])
                    };
                }
            }
            dp[a.len()][b.len()]
        };
        // Small inputs (checked for minimality) plus long ones that exceed
        // MAX_DIFFERENCES_HISTORY and take the recursion-point path.
        let cases = (0..500)
            .map(|_| (next(12) as usize, next(12) as usize, 4))
            .chain((0..4).map(|_| (4000, 4000, 26)))
            .collect::<Vec<_>>();
        for (len_a, len_b, alphabet) in cases {
            let a: Vec<i32> = (0..len_a).map(|_| next(alphabet) as i32).collect();
            let b: Vec<i32> = (0..len_b).map(|_| next(alphabet) as i32).collect();
            let changes = LcsDiff::new(a.clone(), b.clone()).compute_diff();
            let mut rebuilt = Vec::new();
            let mut pos = 0isize;
            let mut deleted = 0;
            for c in &changes {
                assert!(c.original_start >= pos, "{changes:?}");
                assert!(c.original_length > 0 || c.modified_length > 0);
                rebuilt.extend_from_slice(&a[pos as usize..c.original_start as usize]);
                rebuilt.extend_from_slice(&b[c.modified_start as usize..c.modified_end() as usize]);
                pos = c.original_end();
                deleted += c.original_length as usize;
            }
            rebuilt.extend_from_slice(&a[pos as usize..]);
            assert_eq!(rebuilt, b, "{a:?} {b:?} {changes:?}");
            if len_a <= 12 {
                assert_eq!(len_a - deleted, lcs_len(&a, &b), "{a:?} {b:?} {changes:?}");
            }
        }
    }

    #[test]
    fn augments() {
        let base = TextReplacement {
            range: PointUtf16::new(0, 1)..PointUtf16::new(0, 2),
            text: "ab".into(),
        };
        let edit = TextReplacement {
            range: PointUtf16::new(0, 1)..PointUtf16::new(0, 4),
            text: "abc".into(),
        };
        assert!(single_text_edit_augments(&edit, &base));
        assert!(!single_text_edit_augments(&base, &edit));
    }
}
