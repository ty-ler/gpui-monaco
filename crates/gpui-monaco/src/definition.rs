//! Go to definition (Monaco's `contrib/gotoSymbol`): Cmd/Ctrl+hover shows a
//! link on symbols the providers resolve, with the definition's code in a
//! hover, and Cmd/Ctrl+click and F12 jump. Locations in other models are
//! reported with [`CodeEditorEvent::OpenLocation`](crate::CodeEditorEvent)
//! for the host to open.
//!
//! The link and its hover port Monaco's `goToDefinitionAtPosition.ts`
//! (MIT).

use std::ops::Range;
use std::time::Duration;

use gpui::{Context, SharedString, Task};
use text::PointUtf16;

use crate::cursor::{self, Selection};
use crate::decorations::{DecorationId, DecorationOptions, DecorationUnderline};
use crate::editor::CodeEditor;
use crate::providers::{Location, definition_providers};
use crate::theme::TokenStyle;

/// Monaco `GotoDefinitionAtPositionEditorContribution.MAX_SOURCE_PREVIEW_LINES`.
const MAX_SOURCE_PREVIEW_LINES: u32 = 8;

#[derive(Default)]
pub(crate) struct DefinitionLink {
    task: Option<Task<()>>,
    /// The word under the mouse and where it leads.
    pub link: Option<(Range<PointUtf16>, Vec<Location>)>,
    decorations: Vec<DecorationId>,
    pending: Option<PointUtf16>,
    /// Shown from the keyboard (Monaco `startFindDefinitionFromCursor`):
    /// the next cursor move or key press removes it.
    from_keyboard: bool,
}

/// Monaco `getLineFirstNonWhitespaceColumn`: 1-based, 0 for a blank line.
fn first_non_whitespace_column(line: &str) -> usize {
    line.chars()
        .position(|c| !c.is_whitespace())
        .map_or(0, |ix| ix + 1)
}

/// The UTF-16 column `column` of `line` as a byte offset, clamped.
fn utf16_to_byte(line: &str, column: u32) -> usize {
    let mut units = 0;
    for (ix, c) in line.char_indices() {
        if units >= column {
            return ix;
        }
        units += c.len_utf16() as u32;
    }
    line.len()
}

/// Removes exactly `count` leading whitespace characters from `line` if it
/// has that many (Monaco's `^\s{n}` replacement).
fn strip_indentation(line: &str, count: usize) -> &str {
    let mut chars = line.char_indices();
    for _ in 0..count {
        match chars.next() {
            Some((_, c)) if c.is_whitespace() => {}
            _ => return line,
        }
    }
    chars.next().map_or("", |(ix, _)| &line[ix..])
}

/// The code shown in a definition's hover (Monaco
/// `GotoDefinitionAtPositionEditorContribution.getPreviewValue`): the
/// definition's range, or for ranges of 8 lines or more the lines up to the
/// next one indented like the first, with their common indentation
/// removed. `None` if `range` starts outside `lines`.
pub(crate) fn definition_preview(lines: &[&str], range: &Range<PointUtf16>) -> Option<String> {
    let line_count = lines.len() as u32;
    let start = range.start.row;
    if start >= line_count {
        return None;
    }
    let indent = |row: u32| first_non_whitespace_column(lines[row as usize]);
    let mut preview = range.clone();
    if range.end.row.saturating_sub(start) >= MAX_SOURCE_PREVIEW_LINES {
        // Monaco `getPreviewRangeBasedOnIndentation`, in 0-based rows.
        let start_indent = indent(start);
        let max_row = line_count.min(start + 1 + MAX_SOURCE_PREVIEW_LINES) - 1;
        let mut end = start + 1;
        while end < max_row && indent(end) != start_indent {
            end += 1;
        }
        preview = PointUtf16::new(start, 0)..PointUtf16::new(end + 1, 0);
    }
    // Monaco `validateRange`.
    let clamp = |point: PointUtf16| {
        if point.row >= line_count {
            let last = line_count - 1;
            PointUtf16::new(last, lines[last as usize].encode_utf16().count() as u32)
        } else {
            point
        }
    };
    let preview = clamp(preview.start)..clamp(preview.end);
    // Monaco `stripIndentationFromPreviewRange`: the smallest indentation
    // of the first line and the lines after it, but not the last.
    let mut min_indent = indent(start);
    for row in start + 1..preview.end.row {
        min_indent = min_indent.min(indent(row));
    }
    let strip = min_indent.saturating_sub(1);
    let mut text = Vec::new();
    for row in preview.start.row..=preview.end.row {
        let line = lines[row as usize];
        let from = if row == preview.start.row {
            utf16_to_byte(line, preview.start.column)
        } else {
            0
        };
        let to = if row == preview.end.row {
            utf16_to_byte(line, preview.end.column)
        } else {
            line.len()
        };
        text.push(strip_indentation(&line[from.min(to)..to], strip));
    }
    Some(text.join("\n").trim().to_string())
}

/// Monaco `MarkdownString.appendCodeblock`: `code` in a fence longer than
/// any it contains.
pub(crate) fn markdown_code_block(language: &str, code: &str) -> String {
    let longest = code
        .lines()
        .map(|line| line.chars().take_while(|c| *c == '`').count())
        .max()
        .unwrap_or(0);
    let fence = "`".repeat(if longest >= 3 { longest + 1 } else { 3 });
    format!("\n{fence}{language}\n{code}\n{fence}\n")
}

/// Monaco `startFindDefinition`'s link range: the first result's
/// `originSelectionRange`, else the word, grown to cover every result's
/// origin range when there are several.
fn link_range(word: Range<PointUtf16>, locations: &[Location]) -> Range<PointUtf16> {
    let mut range = locations
        .first()
        .and_then(|location| location.origin_selection_range.clone())
        .unwrap_or(word);
    if locations.len() > 1 {
        for origin in locations
            .iter()
            .filter_map(|location| location.origin_selection_range.as_ref())
        {
            range.start = range.start.min(origin.start);
            range.end = range.end.max(origin.end);
        }
    }
    range
}

impl CodeEditor {
    fn request_definitions(
        &mut self,
        position: PointUtf16,
        cx: &mut Context<Self>,
    ) -> Vec<Task<anyhow::Result<Vec<Location>>>> {
        let model = self.model().read(cx);
        let context = model.provider_context();
        definition_providers(cx, self.provider_target(cx))
            .into_iter()
            .map(|provider| provider.provide_definition(&context, position, cx))
            .collect()
    }

    /// Mouse moved with the platform modifier held over `position` (or
    /// released / left text when `None`).
    pub(crate) fn definition_mouse_move(
        &mut self,
        position: Option<PointUtf16>,
        cx: &mut Context<Self>,
    ) {
        let Some(position) = position else {
            self.clear_definition_link(cx);
            return;
        };
        if let Some((range, _)) = &self.definition.link
            && range.start <= position
            && position < range.end
        {
            return;
        }
        if self.definition.pending == Some(position) {
            return;
        }
        self.start_find_definition(position, false, cx);
    }

    /// Monaco `startFindDefinition`: asks the providers for the definition
    /// of the word at `position` and links it, with the definition's code
    /// as its hover. From the keyboard, the hover is shown at the cursor
    /// afterwards.
    fn start_find_definition(
        &mut self,
        position: PointUtf16,
        from_keyboard: bool,
        cx: &mut Context<Self>,
    ) {
        self.clear_definition_link(cx);
        self.definition.pending = Some(position);
        let config = self.cursor_config(cx);
        let word = cursor::word_at(&config, self.snapshot.rope(), position);
        if word.is_none() && !from_keyboard {
            return;
        }
        self.definition.task = Some(cx.spawn(async move |this, cx| {
            let mut locations = Vec::new();
            if word.is_some() {
                cx.background_executor()
                    .timer(Duration::from_millis(50))
                    .await;
                let Ok(tasks) = this.update(cx, |this, cx| this.request_definitions(position, cx))
                else {
                    return;
                };
                for task in tasks {
                    if let Ok(found) = task.await {
                        locations.extend(found);
                    }
                }
            }
            this.update(cx, |this, cx| {
                if this.definition.pending != Some(position) {
                    return;
                }
                if let Some(word) = word
                    && !locations.is_empty()
                {
                    let range = link_range(word, &locations);
                    this.set_definition_link(range, locations, from_keyboard, cx);
                }
                // Monaco `ShowDefinitionPreviewHoverAction`: then the hover.
                if from_keyboard {
                    this.show_hover_at_cursor(cx);
                }
            })
            .ok();
        }));
    }

    /// Shows the definition preview hover for the symbol at the cursor
    /// (Monaco `editor.action.showDefinitionPreviewHover`): the link and
    /// hover Cmd/Ctrl+hover shows, until the cursor moves or a key is
    /// pressed.
    pub fn show_definition_preview_hover(&mut self, cx: &mut Context<Self>) {
        let position = self.cursors.primary().position;
        self.start_find_definition(position, true, cx);
    }

    /// The cursor moved or a key was pressed: a link shown from the
    /// keyboard goes away (Monaco `startFindDefinitionFromCursor`'s
    /// listeners).
    pub(crate) fn dismiss_keyboard_definition_link(&mut self, cx: &mut Context<Self>) {
        if self.definition.from_keyboard && self.definition.link.is_some() {
            self.clear_definition_link(cx);
        }
    }

    /// The link's hover (Monaco `startFindDefinition`): a count for several
    /// results, the definition's code for one in a model that can be
    /// found.
    fn definition_hover_message(
        &self,
        locations: &[Location],
        cx: &mut Context<Self>,
    ) -> Option<SharedString> {
        if locations.len() > 1 {
            return Some(format!("Click to show {} definitions.", locations.len()).into());
        }
        let location = locations.first()?;
        let model = match &location.uri {
            Some(uri) if Some(uri) != self.model().read(cx).uri() => {
                crate::model::model_for_uri(cx, uri)?
            }
            _ => self.model().clone(),
        };
        let model = model.read(cx);
        let text = model.text();
        let lines: Vec<&str> = text
            .split('\n')
            .map(|line| line.trim_end_matches('\r'))
            .collect();
        let preview = definition_preview(&lines, &location.range)?;
        let language = model
            .language_id()
            .map(|id| id.to_string())
            .unwrap_or_default();
        Some(markdown_code_block(&language, &preview).into())
    }

    fn set_definition_link(
        &mut self,
        range: Range<PointUtf16>,
        locations: Vec<Location>,
        from_keyboard: bool,
        cx: &mut Context<Self>,
    ) {
        let hover_message = self.definition_hover_message(&locations, cx);
        let rope = self.snapshot.rope();
        let bytes = rope.point_utf16_to_offset(range.start)..rope.point_utf16_to_offset(range.end);
        let link = self.theme.colors.input_option_active_border;
        let old = std::mem::take(&mut self.definition.decorations);
        let owner = Some(cx.entity_id());
        self.definition.decorations = self.model().update(cx, |model, cx| {
            model.delta_decorations(
                owner,
                &old,
                vec![(
                    bytes,
                    DecorationOptions {
                        underline: Some(DecorationUnderline {
                            color: link,
                            wavy: false,
                        }),
                        text: Some(TokenStyle {
                            foreground: Some(link),
                            ..Default::default()
                        }),
                        hover_message,
                        ..Default::default()
                    },
                )],
                cx,
            )
        });
        self.definition.link = Some((range, locations));
        self.definition.from_keyboard = from_keyboard;
        cx.notify();
    }

    pub(crate) fn clear_definition_link(&mut self, cx: &mut Context<Self>) {
        self.definition.task = None;
        self.definition.pending = None;
        self.definition.link = None;
        self.definition.from_keyboard = false;
        let old = std::mem::take(&mut self.definition.decorations);
        if !old.is_empty() {
            self.model().update(cx, |model, cx| {
                model.delta_decorations(None, &old, Vec::new(), cx)
            });
            cx.notify();
        }
    }

    /// Whether a Cmd/Ctrl+hover link is showing (for the pointer cursor).
    pub(crate) fn has_definition_link(&self) -> bool {
        self.definition.link.is_some()
    }

    /// Follows the link under the mouse (Cmd/Ctrl+click; with Alt, or Cmd/Ctrl
    /// when Alt adds cursors, `open_to_side`). Returns whether there was
    /// one.
    pub(crate) fn follow_definition_link(
        &mut self,
        position: PointUtf16,
        open_to_side: bool,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some((range, _)) = self.definition.link.clone() else {
            return false;
        };
        if !(range.start <= position && position <= range.end) {
            return false;
        }
        self.clear_definition_link(cx);
        // Monaco moves the cursor there and runs go to definition, without
        // a message when nothing is found.
        self.set_selections(&[Selection::collapsed(position)], cx);
        // Monaco `definitionLinkOpensInPeek`, except in a peek view.
        let open_in_peek =
            !open_to_side && self.options.definition_link_opens_in_peek && !self.in_peek;
        self.navigate(
            crate::options::LocationKind::Definition,
            crate::goto::Navigation {
                open_in_peek,
                open_to_side,
                mute_message: true,
                allow_alternative: true,
            },
            cx,
        );
        true
    }

    /// Selects `location` if it's in this model, else asks the host to open
    /// it.
    pub fn go_to_location(&mut self, location: Location, cx: &mut Context<Self>) {
        let same_model =
            location.uri.is_none() || location.uri.as_ref() == self.model().read(cx).uri();
        if same_model {
            self.set_selections(
                &[Selection {
                    anchor: location.range.start,
                    head: location.range.start,
                }],
                cx,
            );
        } else {
            self.open_location_with_openers(location, false, cx);
        }
    }

    /// F12: go to the definition of the symbol at the cursor (Monaco
    /// `editor.action.revealDefinition`).
    pub fn go_to_definition(&mut self, cx: &mut Context<Self>) {
        self.go_to(crate::options::LocationKind::Definition, cx);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn range(start: (u32, u32), end: (u32, u32)) -> Range<PointUtf16> {
        PointUtf16::new(start.0, start.1)..PointUtf16::new(end.0, end.1)
    }

    #[test]
    fn short_ranges_are_shown_whole_without_common_indentation() {
        let lines = [
            "impl A {",
            "    fn add(a: i32) {",
            "        a",
            "    }",
            "}",
        ];
        assert_eq!(
            definition_preview(&lines, &range((1, 4), (3, 5))).as_deref(),
            Some("fn add(a: i32) {\n    a\n}")
        );
        // Just the name, as a provider giving the identifier's range.
        assert_eq!(
            definition_preview(&lines, &range((1, 7), (1, 10))).as_deref(),
            Some("add")
        );
        // Outside the model: nothing.
        assert_eq!(definition_preview(&lines, &range((9, 0), (9, 1))), None);
    }

    #[test]
    fn long_ranges_stop_at_the_next_line_indented_alike() {
        let mut lines = vec!["fn long() {"];
        lines.extend(std::iter::repeat_n("    x;", 12));
        lines.push("}");
        // Ten lines or more: up to 8 lines after the start, here all
        // indented differently from the first.
        let preview = definition_preview(&lines, &range((0, 0), (13, 1))).unwrap();
        assert_eq!(preview.lines().count(), 9);
        let lines = [
            "fn a() {", "    1", "}", "fn b() {", "}", "", "", "", "", "",
        ];
        let preview = definition_preview(&lines, &range((0, 0), (9, 0))).unwrap();
        assert_eq!(preview, "fn a() {\n    1\n}");
    }

    #[test]
    fn code_blocks_use_a_longer_fence_than_the_code() {
        assert_eq!(markdown_code_block("rust", "a"), "\n```rust\na\n```\n");
        assert_eq!(
            markdown_code_block("", "````\nx"),
            "\n`````\n````\nx\n`````\n"
        );
    }
}
