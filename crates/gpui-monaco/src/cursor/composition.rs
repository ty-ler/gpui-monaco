//! What an IME composition typed, and the typing behavior that runs when it
//! ends.
//!
//! Ports `CompositionState` and `CompositionOutcome` from Monaco's
//! `src/vs/editor/common/cursor/cursor.ts` and
//! `src/vs/editor/common/cursor/cursorTypeOperations.ts`,
//! `TypeOperations.compositionEndWithInterceptors`,
//! `AutoClosingOvertypeWithInterceptorsOperation` and
//! `CompositionEndOvertypeOperation` from
//! `src/vs/editor/common/cursor/cursorTypeEditOperations.ts`,
//! `CompositionSurroundSelectionCommand` from
//! `src/vs/editor/common/commands/surroundSelectionCommand.ts` and
//! `ReplaceOvertypeCommandOnCompositionEnd` from
//! `src/vs/editor/common/commands/replaceCommand.ts` (MIT).
//!
//! While a composition is in progress (see
//! [`CursorCollection::start_composition`]), typing skips the language's
//! interceptors, except typing over an auto-inserted closing character.
//! When it ends, [`composition_end_with_language`] looks at what it typed:
//! a single character gets the interceptors after the fact (a dead-key `'`
//! auto-closes, overtypes or surrounds the selection it replaced), and in
//! overtype mode the composed text types over as many characters after it.

use std::ops::Range;

use super::edit::{
    Command, CursorStateHelper, EditBuilder, EditOperation, EditOperationType, ReplaceCommand,
    execute_commands,
};
use super::language::{LanguageContext, LanguageSupport, is_quote};
use super::language_typing::{
    auto_closing_open_char_type, auto_closing_pair, is_auto_closing_overtype, should_surround_char,
};
use super::{CursorCollection, CursorConfig, CursorModel, Position, Selection, strings};

/// A cursor's line when a composition started or ended (Monaco
/// `CompositionLineState`), in UTF-16 units.
#[derive(Clone, Debug, PartialEq, Eq)]
struct CompositionLineState {
    text: Vec<u16>,
    row: u32,
    start_selection_offset: u32,
    end_selection_offset: u32,
}

/// The cursors' lines when an IME composition started (Monaco
/// `CompositionState`), to tell what the composition typed when it ends.
/// [`CursorCollection::start_composition`] records one.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CompositionState {
    /// `None` when a selection spanned lines: the outcome can't be told.
    original: Option<Vec<CompositionLineState>>,
}

/// What a composition did at one cursor (Monaco `CompositionOutcome`): the
/// text it replaced and the text it left, between the longest common prefix
/// and suffix of the cursor's line before and after.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CompositionOutcome {
    /// The text the composition removed.
    pub deleted_text: String,
    /// Where the selection started in `deleted_text` when the composition
    /// started (UTF-16 units).
    pub deleted_selection_start: u32,
    /// Where the selection ended in `deleted_text` when the composition
    /// started (UTF-16 units).
    pub deleted_selection_end: u32,
    /// The text the composition left.
    pub inserted_text: String,
    /// Where `inserted_text` is in the text now.
    pub inserted_text_range: Range<Position>,
}

fn capture(model: &dyn CursorModel, selections: &[Selection]) -> Option<Vec<CompositionLineState>> {
    selections
        .iter()
        .map(|selection| {
            let (start, end) = (selection.start(), selection.end());
            (start.row == end.row).then(|| CompositionLineState {
                text: strings::utf16(&model.line_content(start.row)),
                row: start.row,
                start_selection_offset: start.column,
                end_selection_offset: end.column,
            })
        })
        .collect()
}

fn common_prefix_length(a: &[u16], b: &[u16]) -> usize {
    a.iter().zip(b).take_while(|(a, b)| a == b).count()
}

fn common_suffix_length(a: &[u16], b: &[u16]) -> usize {
    a.iter()
        .rev()
        .zip(b.iter().rev())
        .take_while(|(a, b)| a == b)
        .count()
}

impl CompositionState {
    /// Records the cursors' lines as a composition starts (Monaco
    /// `new CompositionState(model, selections)`).
    pub fn new(model: &impl CursorModel, selections: &[Selection]) -> Self {
        Self {
            original: capture(model, selections),
        }
    }

    /// What the composition did at each cursor, given the text and cursors
    /// now (Monaco `deduceOutcome`). `None` when it can't be told: a
    /// selection spans lines, or the cursor count changed.
    pub fn deduce_outcome(
        &self,
        model: &impl CursorModel,
        selections: &[Selection],
    ) -> Option<Vec<CompositionOutcome>> {
        let original = self.original.as_ref()?;
        let current = capture(model, selections)?;
        if original.len() != current.len() {
            return None;
        }
        Some(
            original
                .iter()
                .zip(&current)
                .map(|(original, current)| Self::outcome(original, current))
                .collect(),
        )
    }

    /// Monaco `CompositionState._deduceOutcome`.
    fn outcome(
        original: &CompositionLineState,
        current: &CompositionLineState,
    ) -> CompositionOutcome {
        let common_prefix = (original.start_selection_offset as usize)
            .min(current.start_selection_offset as usize)
            .min(common_prefix_length(&original.text, &current.text));
        let common_suffix = (original.text.len() - original.end_selection_offset as usize)
            .min(current.text.len() - current.end_selection_offset as usize)
            .min(common_suffix_length(&original.text, &current.text));
        let deleted_end = (original.text.len() - common_suffix).max(common_prefix);
        let inserted_end = (current.text.len() - common_suffix).max(common_prefix);
        let prefix = common_prefix as u32;
        CompositionOutcome {
            deleted_text: String::from_utf16_lossy(&original.text[common_prefix..deleted_end]),
            deleted_selection_start: original.start_selection_offset - prefix,
            deleted_selection_end: original.end_selection_offset - prefix,
            inserted_text: String::from_utf16_lossy(&current.text[common_prefix..inserted_end]),
            inserted_text_range: Position::new(current.row, prefix)
                ..Position::new(current.row, inserted_end as u32),
        }
    }
}

/// Runs the typing behavior Monaco runs when an IME composition ends from
/// the keyboard (Monaco `TypeOperations.compositionEndWithInterceptors`,
/// "very similar with typing, but the character is already in the text
/// buffer"). `composition` is the state recorded when it started (see
/// [`CursorCollection::start_composition`]); `cursors` are the cursors now.
///
/// When every cursor's composition left the same single character, with a
/// language:
///
/// - if it replaced a selection, the selection is surrounded instead (a
///   dead-key `'` over `test` gives `'test'`);
/// - if it typed a closing character before an auto-inserted one, the
///   auto-inserted one goes;
/// - if it typed an auto-closing pair's opening character, the closing one
///   follows.
///
/// Otherwise, in overtype mode, the composed text types over as many
/// characters after it. Returns `None` when there is nothing to do. With
/// `lang == None` only the overtyping applies.
pub fn composition_end_with_language<M: CursorModel>(
    config: &CursorConfig,
    model: &M,
    cursors: &CursorCollection,
    composition: &CompositionState,
    lang: Option<&LanguageContext>,
) -> Option<EditOperation> {
    let dm: &dyn CursorModel = model;
    let selections = cursors.selections();
    // Could not deduce what the composition did.
    let compositions = composition.deduce_outcome(model, &selections)?;
    let inserted_text = &compositions.first()?.inserted_text;
    if compositions
        .iter()
        .any(|c| &c.inserted_text != inserted_text)
    {
        // Not all selections agree on what was typed.
        return None;
    }
    let lang = lang.map(LanguageSupport::new);
    let ch = inserted_text.as_str();
    let (Some(lang), 1) = (lang.as_ref(), strings::utf16_len(ch)) else {
        // Only a single inserted character gets the interceptors.
        return composition_end_overtype(config, dm, &selections, &compositions);
    };

    if compositions.iter().any(|c| !c.deleted_text.is_empty()) {
        // Could this have been a surround selection?
        let close = lang.surrounding_close(ch)?;
        if !should_surround_char(config, ch) {
            return None;
        }
        for composition in &compositions {
            if composition.deleted_selection_start != 0
                || composition.deleted_selection_end as usize
                    != strings::utf16_len(&composition.deleted_text) as usize
            {
                // More text was deleted than was selected.
                return None;
            }
            if composition
                .deleted_text
                .chars()
                .all(|c| c == ' ' || c == '\t')
            {
                // Only whitespace was deleted.
                return None;
            }
            if is_quote(ch) && is_quote(&composition.deleted_text) {
                // A quote was typed over a quote.
                return None;
            }
        }
        if selections.iter().any(|s| !s.is_empty()) || selections.len() != compositions.len() {
            return None;
        }
        let commands = selections
            .iter()
            .zip(&compositions)
            .map(|(selection, composition)| {
                Some(Box::new(CompositionSurroundSelectionCommand {
                    position: selection.head,
                    text: composition.deleted_text.clone(),
                    char_after: close.to_string(),
                }) as Box<dyn Command>)
            })
            .collect();
        return Some(execute_commands(
            dm,
            &selections,
            commands,
            EditOperationType::TypingOther,
            true,
            false,
        ));
    }

    // Monaco `AutoClosingOvertypeWithInterceptorsOperation`.
    let auto_closed = cursors.auto_closed_ranges().closing_ranges();
    if is_auto_closing_overtype(config, lang, dm, &selections, &auto_closed, ch) {
        // The closing character is "doubled" now: delete the one after.
        let commands = selections
            .iter()
            .map(|s| {
                let p = s.head;
                ReplaceCommand::boxed(p..Position::new(p.row, p.column + 1), "")
            })
            .collect();
        return Some(execute_commands(
            dm,
            &selections,
            commands,
            EditOperationType::TypingOther,
            true,
            false,
        ));
    }

    if let Some(pair) = auto_closing_pair(config, lang, dm, &selections, ch, true) {
        return Some(auto_closing_open_char_type(
            dm,
            &selections,
            ch,
            true,
            &pair,
        ));
    }

    composition_end_overtype(config, dm, &selections, &compositions)
}

/// Monaco `CompositionEndOvertypeOperation`.
fn composition_end_overtype(
    config: &CursorConfig,
    model: &dyn CursorModel,
    selections: &[Selection],
    compositions: &[CompositionOutcome],
) -> Option<EditOperation> {
    if !config.overtype {
        return None;
    }
    let commands = compositions
        .iter()
        .map(|c| {
            Some(Box::new(ReplaceOvertypeCommandOnCompositionEnd {
                range: c.inserted_text_range.clone(),
            }) as Box<dyn Command>)
        })
        .collect();
    Some(execute_commands(
        model,
        selections,
        commands,
        EditOperationType::TypingOther,
        true,
        false,
    ))
}

/// Monaco `CompositionSurroundSelectionCommand`: after a composition typed
/// the opening character over a selection, puts the selection's text and
/// the closing character back after it, and selects the text again.
struct CompositionSurroundSelectionCommand {
    position: Position,
    text: String,
    char_after: String,
}

impl Command for CompositionSurroundSelectionCommand {
    fn edit_operations(&mut self, _: &dyn CursorModel, builder: &mut EditBuilder) {
        builder.add_edit(
            self.position..self.position,
            &format!("{}{}", self.text, self.char_after),
        );
    }

    fn compute_cursor_state(&self, helper: &CursorStateHelper) -> Selection {
        let range = &helper.inverse_edit_ranges()[0];
        Selection::new(
            Position::new(range.end.row, range.start.column),
            Position::new(
                range.end.row,
                range
                    .end
                    .column
                    .saturating_sub(strings::utf16_len(&self.char_after)),
            ),
        )
    }
}

/// Monaco `ReplaceOvertypeCommandOnCompositionEnd`: deletes as many
/// characters after the composed text (`range`) as it has, on its line.
struct ReplaceOvertypeCommandOnCompositionEnd {
    range: Range<Position>,
}

impl Command for ReplaceOvertypeCommandOnCompositionEnd {
    fn edit_operations(&mut self, model: &dyn CursorModel, builder: &mut EditBuilder) {
        let length = self
            .range
            .end
            .column
            .saturating_sub(self.range.start.column);
        let end = self.range.end;
        // Monaco `addPositiveOffsetToModelPosition`, clamped to the line.
        let column = (end.column + length).min(model.line_max_column(end.row));
        builder.add_edit(end..Position::new(end.row, column), "");
    }

    fn compute_cursor_state(&self, helper: &CursorStateHelper) -> Selection {
        Selection::collapsed(helper.inverse_edit_ranges()[0].end)
    }
}
