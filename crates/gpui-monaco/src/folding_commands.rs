//! Folding commands (Monaco `src/vs/editor/contrib/folding/browser/folding.ts`,
//! MIT): fold and unfold (recursively), toggle, fold levels, block
//! comments, regions, all but the selection, fold navigation, manual
//! folding ranges and the gutter and `⋯` clicks.

use gpui::{App, Context};
use text::PointUtf16;

use crate::cursor::Selection;
use crate::editor::{CodeEditor, CodeEditorEvent, FoldingRegionState, FoldingViewState};
use crate::providers::FoldingRangeKind;
use crate::view::fold_model::{self as fm, FoldingModel, SelectedRows};
use crate::view::folding::{FoldSource, FoldState, FoldingLimit, MergeRange};

/// Monaco's `cursorChangedScheduler` delay before
/// [`CodeEditor::reveal_cursor_in_folds`].
const REVEAL_CURSOR_DELAY: std::time::Duration = std::time::Duration::from_millis(200);

impl CodeEditor {
    /// Monaco `FoldingAction.getSelectedLines`: each selection's first row.
    fn selected_rows(&self) -> Vec<u32> {
        self.cursors
            .selections()
            .iter()
            .map(|s| s.start().row)
            .collect()
    }

    /// Brings the folding regions up to date with the text, so commands
    /// right after an edit act on the lines the regions moved to (Monaco's
    /// folding decorations follow edits).
    pub(crate) fn sync_folds(&mut self, cx: &App) {
        let model = self.model().clone();
        self.folds.sync(model.read(cx).buffer());
    }

    /// Computes the ranges to toggle from the model, toggles them and moves
    /// cursors out of hidden lines.
    fn run_fold_command(
        &mut self,
        cx: &mut Context<Self>,
        command: impl FnOnce(&FoldingModel, &[u32]) -> Vec<usize>,
    ) {
        if !self.options.folding {
            return;
        }
        self.sync_folds(cx);
        let rows = self.selected_rows();
        let collapsed = self.folds.collapsed_flags();
        let ranges = self.folds.ranges().to_vec();
        let toggle = command(&FoldingModel::new(&ranges, collapsed), &rows);
        self.folds.toggle(toggle);
        self.after_fold_command(cx);
    }

    /// Collapses the innermost expanded region at each selection, or its
    /// parent (Monaco `editor.fold`).
    pub fn fold(&mut self, cx: &mut Context<Self>) {
        self.run_fold_command(cx, |m, rows| fm::set_collapse_state_up(m, true, rows));
    }

    /// Monaco `editor.fold` (`collapse`) or `editor.unfold` with their
    /// `FoldingArguments`: `levels` levels (1 by default), `"up"` around the
    /// lines or `"down"` inside them, at the 0-based `lines` (by default
    /// each selection's first line). Fold without `levels` or `direction`
    /// is [`CodeEditor::fold`].
    pub fn fold_with_args(
        &mut self,
        levels: Option<u32>,
        direction: Option<&str>,
        lines: Option<Vec<u32>>,
        collapse: bool,
        cx: &mut Context<Self>,
    ) {
        if !self.options.folding {
            return;
        }
        let rows = lines.unwrap_or_else(|| self.selected_rows());
        self.sync_folds(cx);
        let collapsed = self.folds.collapsed_flags();
        let ranges = self.folds.ranges().to_vec();
        let model = FoldingModel::new(&ranges, collapsed);
        let up = direction == Some("up");
        let toggle = if collapse && levels.is_none() && direction.is_none() {
            fm::set_collapse_state_up(&model, true, &rows)
        } else {
            let levels = levels.unwrap_or(1).max(1) as usize;
            if up {
                fm::set_collapse_state_levels_up(&model, collapse, levels, &rows)
            } else {
                fm::set_collapse_state_levels_down(&model, collapse, levels, Some(&rows))
            }
        };
        self.folds.toggle(toggle);
        self.after_fold_command(cx);
    }

    /// Expands the region at each selection (Monaco `editor.unfold`).
    pub fn unfold(&mut self, cx: &mut Context<Self>) {
        self.run_fold_command(cx, |m, rows| {
            fm::set_collapse_state_levels_down(m, false, 1, Some(rows))
        });
    }

    /// Collapses the region at each selection and everything inside it
    /// (Monaco `editor.foldRecursively`).
    pub fn fold_recursively(&mut self, cx: &mut Context<Self>) {
        self.run_fold_command(cx, |m, rows| {
            fm::set_collapse_state_levels_down(m, true, usize::MAX, Some(rows))
        });
    }

    /// Expands the region at each selection and everything inside it
    /// (Monaco `editor.unfoldRecursively`).
    pub fn unfold_recursively(&mut self, cx: &mut Context<Self>) {
        self.run_fold_command(cx, |m, rows| {
            fm::set_collapse_state_levels_down(m, false, usize::MAX, Some(rows))
        });
    }

    /// Toggles the region at each selection (Monaco `editor.toggleFold`).
    pub fn toggle_fold(&mut self, cx: &mut Context<Self>) {
        self.run_fold_command(cx, |m, rows| fm::toggle_collapse_state(m, 1, rows));
    }

    /// Toggles the region at each selection and everything inside it
    /// (Monaco `editor.toggleFoldRecursively`).
    pub fn toggle_fold_recursively(&mut self, cx: &mut Context<Self>) {
        self.run_fold_command(cx, |m, rows| fm::toggle_collapse_state(m, usize::MAX, rows));
    }

    /// Collapses every region (Monaco `editor.foldAll`).
    pub fn fold_all(&mut self, cx: &mut Context<Self>) {
        self.run_fold_command(cx, |m, _| {
            fm::set_collapse_state_levels_down(m, true, usize::MAX, None)
        });
    }

    /// Expands every region (Monaco `editor.unfoldAll`).
    pub fn unfold_all(&mut self, cx: &mut Context<Self>) {
        self.run_fold_command(cx, |m, _| {
            fm::set_collapse_state_levels_down(m, false, usize::MAX, None)
        });
    }

    /// Collapses every region at nesting `level` (1 is the top) except those
    /// around a selection (Monaco `editor.foldLevel1`…`7`).
    pub fn fold_level(&mut self, level: usize, cx: &mut Context<Self>) {
        self.run_fold_command(cx, |m, rows| {
            fm::set_collapse_state_at_level(m, level, true, rows)
        });
    }

    /// Collapses (or expands) every region except those around a selection
    /// (Monaco `editor.foldAllExcept` / `editor.unfoldAllExcept`).
    pub fn fold_all_except(&mut self, collapse: bool, cx: &mut Context<Self>) {
        self.run_fold_command(cx, |m, rows| {
            fm::set_collapse_state_for_rest(m, collapse, rows)
        });
    }

    /// Collapses every block comment (Monaco `editor.foldAllBlockComments`):
    /// comment ranges from providers, else ranges starting with the
    /// language's block comment.
    pub fn fold_all_block_comments(&mut self, cx: &mut Context<Self>) {
        if self.folds.has_types() {
            self.run_fold_command(cx, |m, _| {
                fm::set_collapse_state_for_type(m, &FoldingRangeKind::Comment, true)
            });
            return;
        }
        let model = self.model().read(cx);
        let Some(start) = model
            .language()
            .and_then(|language| language.config.comments().block_comment.clone())
            .map(|(start, _)| start)
        else {
            return;
        };
        let lines: Vec<String> = (0..model.line_count())
            .map(|row| model.line_content(row))
            .collect();
        self.run_fold_command(cx, |m, _| {
            fm::set_collapse_state_for_matching_lines(
                m,
                |row| {
                    lines
                        .get(row as usize)
                        .is_some_and(|line| line.trim_start().starts_with(start.as_str()))
                },
                true,
            )
        });
    }

    /// Collapses (or expands) every marker region (Monaco
    /// `editor.foldAllMarkerRegions` / `editor.unfoldAllMarkerRegions`):
    /// region ranges from providers, else ranges starting with the
    /// language's region start marker.
    pub fn fold_all_marker_regions(&mut self, collapse: bool, cx: &mut Context<Self>) {
        if self.folds.has_types() {
            self.run_fold_command(cx, |m, _| {
                fm::set_collapse_state_for_type(m, &FoldingRangeKind::Region, collapse)
            });
            return;
        }
        let model = self.model().read(cx);
        let Some(start) = model
            .language()
            .and_then(|language| language.config.folding.clone())
            .and_then(|folding| folding.markers)
            .map(|markers| markers.start)
        else {
            return;
        };
        let lines: Vec<String> = (0..model.line_count())
            .map(|row| model.line_content(row))
            .collect();
        self.run_fold_command(cx, |m, _| {
            fm::set_collapse_state_for_matching_lines(
                m,
                |row| {
                    lines
                        .get(row as usize)
                        .is_some_and(|line| start.is_match(line))
                },
                collapse,
            )
        });
    }

    /// Toggles the import ranges (Monaco `editor.toggleImportFold`), then
    /// computes the ranges again.
    pub fn toggle_import_fold(&mut self, cx: &mut Context<Self>) {
        if !self.options.folding {
            return;
        }
        self.run_fold_command(cx, |m, _| {
            (0..m.ranges.len())
                .rev()
                .filter(|&ix| m.ranges[ix].kind == Some(FoldingRangeKind::Imports))
                .collect()
        });
        self.refresh_folding(cx);
    }

    /// Moves to the line `find` gives, then reveals the selection's start
    /// centered if it's off screen, as every folding action does (Monaco
    /// `FoldingAction.runEditorCommand`).
    fn go_to_fold_line(
        &mut self,
        cx: &mut Context<Self>,
        find: impl FnOnce(u32, &FoldingModel) -> Option<u32>,
    ) {
        if !self.options.folding {
            return;
        }
        self.sync_folds(cx);
        let collapsed = self.folds.collapsed_flags();
        let ranges = self.folds.ranges().to_vec();
        let row = self.cursors.selections()[0].start().row;
        if let Some(target) = find(row, &FoldingModel::new(&ranges, collapsed)) {
            let start = PointUtf16::new(target, 0);
            self.set_selections(&[Selection::collapsed(start)], cx);
        }
        let start = self.cursors.primary().selection().start();
        self.reveal_position_in_center_if_outside_viewport(start, cx);
    }

    /// Moves to the start of the enclosing fold (Monaco
    /// `editor.gotoParentFold`).
    pub fn go_to_parent_fold(&mut self, cx: &mut Context<Self>) {
        self.go_to_fold_line(cx, fm::parent_fold_line);
    }

    /// Moves to the previous fold at this level (Monaco
    /// `editor.gotoPreviousFold`).
    pub fn go_to_previous_fold(&mut self, cx: &mut Context<Self>) {
        self.go_to_fold_line(cx, fm::previous_fold_line);
    }

    /// Moves to the next fold at this level (Monaco `editor.gotoNextFold`).
    pub fn go_to_next_fold(&mut self, cx: &mut Context<Self>) {
        self.go_to_fold_line(cx, fm::next_fold_line);
    }

    /// Folds each multi-line selection as a manual range (Monaco
    /// `editor.createFoldingRangeFromSelection`, Cmd/Ctrl+K Cmd/Ctrl+,).
    pub fn create_fold_from_selection(&mut self, cx: &mut Context<Self>) {
        if !self.options.folding {
            return;
        }
        let mut rows = Vec::new();
        let mut cursor = None;
        for selection in self.cursors.selections() {
            let (start, end) = (selection.start(), selection.end());
            let last = if end.column == 0 {
                end.row.saturating_sub(1)
            } else {
                end.row
            };
            if last > start.row {
                rows.push((start.row, last));
                cursor = Some(PointUtf16::new(start.row, 0));
            }
        }
        if let Some(cursor) = cursor {
            self.set_selections(&[Selection::collapsed(cursor)], cx);
        }
        let model = self.model().clone();
        self.folds.add_manual(model.read(cx).buffer(), &rows);
        self.after_fold_command(cx);
    }

    /// Removes manual folding ranges around the cursors or touching the
    /// selections (Monaco `editor.removeManualFoldingRanges`).
    pub fn remove_manual_folding_ranges(&mut self, cx: &mut Context<Self>) {
        if !self.options.folding {
            return;
        }
        let selections: Vec<SelectedRows> = self
            .cursors
            .selections()
            .iter()
            .map(|s| SelectedRows {
                start: s.start().row,
                end: s.end().row,
                empty: s.is_empty(),
            })
            .collect();
        let model = self.model().clone();
        self.folds
            .remove_manual(model.read(cx).buffer(), &selections);
        self.after_fold_command(cx);
    }

    /// A click on the fold control at `row`, or (`icon: false`) on the `⋯`
    /// after a collapsed line (Monaco `FoldingController.onEditorMouseUp`):
    /// toggles the region; `recursive` (Shift or middle click) also its
    /// children, `surrounding` (Alt) the regions around it instead.
    pub fn click_fold(
        &mut self,
        row: u32,
        icon: bool,
        recursive: bool,
        surrounding: bool,
        cx: &mut Context<Self>,
    ) {
        if !self.options.folding {
            return;
        }
        self.sync_folds(cx);
        let collapsed = self.folds.collapsed_flags();
        let ranges = self.folds.ranges().to_vec();
        let model = FoldingModel::new(&ranges, collapsed);
        let Some(region) = model
            .find_range(row)
            .filter(|&r| ranges[r].start_row == row)
        else {
            return;
        };
        let is_collapsed = model.collapsed[region];
        if !icon && !is_collapsed {
            return;
        }
        let toggle = if surrounding {
            let outside = |r: usize| {
                let (a, b) = (&ranges[r], &ranges[region]);
                let inside = b.start_row <= a.start_row && a.end_row <= b.end_row;
                let around = a.start_row <= b.start_row && b.end_row <= a.end_row;
                !inside && !around
            };
            let candidates = model.regions_inside(None, |r, _| outside(r));
            // Unfold folded surroundings; if none are folded, fold them all.
            let folded: Vec<usize> = candidates
                .iter()
                .copied()
                .filter(|&r| model.collapsed[r])
                .collect();
            if folded.is_empty() {
                candidates
            } else {
                folded
            }
        } else {
            let mut toggle: Vec<usize> = if recursive {
                model.regions_inside(Some(region), |r, _| model.collapsed[r] == is_collapsed)
            } else {
                Vec::new()
            };
            // Recursive: first fold the children; with none left, the region.
            if is_collapsed || !recursive || toggle.is_empty() {
                toggle.push(region);
            }
            toggle
        };
        self.folds.toggle(toggle);
        self.after_fold_change(cx);
        // Monaco `FoldingController.onEditorMouseUp`: the clicked line shows
        // (`revealPositionInCenterIfOutsideViewport`), not the cursor.
        self.reveal_position_in_center_if_outside_viewport(PointUtf16::new(row, 0), cx);
    }

    /// Monaco `FoldingController.onCursorPositionChanged`: while lines are
    /// hidden, a cursor change (re)schedules
    /// [`Self::reveal_cursor_in_folds`] after Monaco's delay.
    pub(crate) fn folding_cursor_changed(&mut self, cx: &mut Context<Self>) {
        if !self.options.folding || !self.folds.has_hidden() {
            return;
        }
        self.fold_reveal_task = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(REVEAL_CURSOR_DELAY).await;
            this.update(cx, |this, cx| this.reveal_cursor_in_folds(cx))
                .ok();
        }));
    }

    /// Monaco `FoldingController.revealCursor`: expands every collapsed
    /// region hiding a selection's start (its anchor), wherever the
    /// selection came from (find, go to line or definition, jump to
    /// bracket, add selection to next find match, the API), and reveals
    /// the primary cursor centered if it's off screen.
    fn reveal_cursor_in_folds(&mut self, cx: &mut Context<Self>) {
        self.fold_reveal_task = None;
        if !self.options.folding {
            return;
        }
        self.sync_folds(cx);
        let rows: Vec<u32> = self
            .cursors
            .selections()
            .iter()
            .map(|s| s.anchor.row)
            .collect();
        if self.folds.reveal_rows(&rows) {
            let position = self.cursors.primary().position;
            self.reveal_position_in_center_if_outside_viewport(position, cx);
            cx.notify();
        }
    }

    /// Moves cursors out of hidden rows to the end of the line above them.
    /// Like Monaco's line mapping change (`Cursor.onLineMappingChanged`), it
    /// doesn't scroll.
    pub(crate) fn after_fold_change(&mut self, cx: &mut Context<Self>) {
        self.move_cursors_out_of_folds();
        self.cursors_changed(false, cx);
    }

    /// [`Self::after_fold_change`] for a folding command: then the primary
    /// selection's start shows, centered if it was off screen (Monaco
    /// `FoldingAction.runEditorCommand`, `FoldingController.reveal`).
    fn after_fold_command(&mut self, cx: &mut Context<Self>) {
        self.after_fold_change(cx);
        let start = self.cursors.primary().selection().start();
        self.reveal_position_in_center_if_outside_viewport(start, cx);
    }

    /// Monaco `HiddenRangeModel.adjustSelections`: a selection end in
    /// hidden lines moves to the end of the line above them.
    fn move_cursors_out_of_folds(&mut self) {
        let hidden = self.folds.hidden_rows();
        let fix = |p: PointUtf16| {
            let ix = hidden.partition_point(|rows| rows.start <= p.row);
            match ix.checked_sub(1).map(|ix| &hidden[ix]) {
                Some(rows) if rows.contains(&p.row) && rows.start > 0 => {
                    let row = rows.start - 1;
                    PointUtf16::new(row, self.snapshot.line_len_utf16(row))
                }
                _ => p,
            }
        };
        let selections: Vec<Selection> = self
            .cursors
            .selections()
            .into_iter()
            .map(|s| Selection {
                anchor: fix(s.anchor),
                head: fix(s.head),
            })
            .collect();
        self.cursors.set_selections(&selections);
    }

    /// Monaco `FoldingController.saveViewState` (`FoldingModel.getMemento`):
    /// the collapsed regions and those not from the provider, with a
    /// checksum of their lines.
    pub(crate) fn save_folding_state(&self, cx: &App) -> FoldingViewState {
        let model = self.model().read(cx);
        if !self.options.folding || model.is_too_large_for_tokenization() {
            return FoldingViewState::default();
        }
        let folded_imports = self.folded_imports;
        let rope = model.buffer().rope();
        let line_count = rope.line_count();
        let regions = self
            .folds
            .memento()
            .into_iter()
            .filter(|r| r.start < r.end && r.end < line_count)
            .map(|r| FoldingRegionState {
                start_row: r.start,
                end_row: r.end,
                is_collapsed: r.collapsed,
                manual: r.source == FoldSource::UserDefined,
                checksum: Some(lines_checksum(rope, r.start, r.end)),
            })
            .collect();
        FoldingViewState {
            regions,
            line_count,
            folded_imports,
        }
    }

    /// Monaco `FoldingController.restoreViewState`
    /// (`FoldingModel.applyMemento`): the regions whose lines still match
    /// their checksum are merged in. The cursors stay where they are, even
    /// in hidden lines (Monaco skips `adjustSelections` while restoring),
    /// and aren't revealed.
    pub(crate) fn restore_folding_state(
        &mut self,
        state: &FoldingViewState,
        cx: &mut Context<Self>,
    ) {
        let model = self.model().clone();
        let model = model.read(cx);
        if !self.options.folding || model.is_too_large_for_tokenization() {
            return;
        }
        self.folded_imports = state.folded_imports;
        if state.regions.is_empty() {
            return;
        }
        let buffer = model.buffer();
        let line_count = buffer.rope().line_count();
        let saved: Vec<MergeRange> = state
            .regions
            .iter()
            .filter(|region| {
                region.start_row < region.end_row
                    && region.end_row < line_count
                    && region.checksum.is_none_or(|checksum| {
                        checksum == lines_checksum(buffer.rope(), region.start_row, region.end_row)
                    })
            })
            .map(|region| MergeRange {
                start: region.start_row,
                end: region.end_row,
                kind: None,
                collapsed: region.is_collapsed,
                source: if region.manual {
                    FoldSource::UserDefined
                } else {
                    FoldSource::Provider
                },
            })
            .collect();
        self.folds.apply_memento(buffer, saved);
        cx.notify();
    }

    /// Forgets the folding regions and any computation (Monaco
    /// `FoldingController.onModelChanged`, for a new model or when
    /// `folding` or `foldingMaximumRegions` change).
    pub(crate) fn reset_folding(&mut self) {
        self.folds = FoldState::default();
        self.fold_task = None;
        self.fold_requested = None;
        self.fold_computing = false;
        self.fold_rerun = false;
        self.fold_stale = false;
        self.folded_imports = false;
        self.fold_mouse_down = None;
        self.fold_reveal_task = None;
    }

    /// Computes the ranges again for the current text on the next frame
    /// (Monaco `FoldingController.onFoldingStrategyChanged`, after the
    /// strategy, the language configuration or the providers changed).
    pub(crate) fn refresh_folding(&mut self, cx: &mut Context<Self>) {
        self.fold_stale = true;
        cx.notify();
    }

    /// Monaco `FoldingController`'s `onDidChangeConfiguration`: changing
    /// `folding` or `foldingMaximumRegions` starts over (turning folding off
    /// discards the folds), and changing `foldingStrategy` computes the
    /// ranges again. `old` holds the previous `(folding,
    /// folding_maximum_regions, folding_strategy)`.
    pub(crate) fn folding_options_changed(
        &mut self,
        old: &(bool, usize, crate::options::FoldingStrategy),
        cx: &mut Context<Self>,
    ) {
        let (folding, maximum_regions, strategy) = *old;
        if folding != self.options.folding
            || maximum_regions != self.options.folding_maximum_regions
        {
            self.reset_folding();
            cx.notify();
        } else if strategy != self.options.folding_strategy {
            self.refresh_folding(cx);
        }
    }

    /// Records what the last range computation found, emitting
    /// [`CodeEditorEvent::FoldingLimitChanged`] when it changed (Monaco
    /// `RangesLimitReporter.update`).
    pub(crate) fn set_folding_limit(&mut self, limit: FoldingLimit, cx: &mut Context<Self>) {
        if limit != self.folding_limit {
            self.folding_limit = limit;
            cx.emit(CodeEditorEvent::FoldingLimitChanged);
        }
    }

    /// How many folding ranges the last computation found, and the limit
    /// (`folding_maximum_regions`) they were cut to if there were more
    /// (Monaco `FoldingController.limitReporter`'s `computed` and
    /// `limited`). [`CodeEditorEvent::FoldingLimitChanged`] reports changes.
    pub fn folding_limit(&self) -> FoldingLimit {
        self.folding_limit
    }

    /// Monaco `FoldingController.onEditorMouseDown`: a left or middle press
    /// the folding contribution may act on when released on the same line
    /// (see [`Self::folding_mouse_up`]): on a folding control (`icon`), or
    /// at the end of a collapsed line's text (its `⋯`, or past it with
    /// `unfold_on_click_after_end_of_line`) while lines are hidden. `None`
    /// forgets any earlier press.
    pub(crate) fn folding_mouse_down(&mut self, row: Option<u32>, icon: bool) {
        self.fold_mouse_down = row
            .filter(|_| self.options.folding && (icon || self.folds.has_hidden()))
            .map(|row| (row, icon));
    }

    /// Monaco `FoldingController.onEditorMouseUp`: toggles the region
    /// starting at the line pressed if released on it, over the folding
    /// controls' column for a control press (`on_controls`) or at the end
    /// of the line's text otherwise (`at_line_end`). Returns whether it
    /// did.
    pub(crate) fn folding_mouse_up(
        &mut self,
        row: Option<u32>,
        on_controls: bool,
        at_line_end: bool,
        recursive: bool,
        surrounding: bool,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some((line, icon)) = self.fold_mouse_down.take() else {
            return false;
        };
        if row != Some(line) || (icon && !on_controls) || (!icon && !at_line_end) {
            return false;
        }
        self.click_fold(line, icon, recursive, surrounding, cx);
        true
    }
}

/// Monaco `FoldingModel._getLinesChecksum`: `hash` of the region's second
/// and last lines, modulo 1,000,000.
fn lines_checksum(rope: &text::Rope, start_row: u32, end_row: u32) -> i32 {
    let text = rope.line(start_row + 1) + &rope.line(end_row);
    string_hash(&text) % 1_000_000
}

/// Monaco `stringHash(s, 0)` (`base/common/hash.ts`): `hash * 31 + code
/// unit` over UTF-16, in 32-bit integers.
fn string_hash(s: &str) -> i32 {
    let step = |hash: i32, value: i32| hash.wrapping_shl(5).wrapping_sub(hash).wrapping_add(value);
    s.encode_utf16()
        .fold(step(0, 149_417), |hash, unit| step(hash, unit as i32))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn string_hash_matches_monaco() {
        // `stringHash('', 0)` and `hash('a')`, as Monaco computes them.
        assert_eq!(string_hash(""), 149_417);
        assert_eq!(string_hash("a"), 149_417 * 31 + 97);
        // Overflow wraps like JavaScript's `| 0`.
        let long = "x".repeat(100);
        let mut expected: i32 = 149_417;
        for _ in 0..100 {
            expected = expected.wrapping_mul(31).wrapping_add(120);
        }
        assert_eq!(string_hash(&long), expected);
    }
}
