//! Folding commands (Monaco `src/vs/editor/contrib/folding/browser/folding.ts`,
//! MIT): fold and unfold (recursively), toggle, fold levels, block
//! comments, regions, all but the selection, fold navigation, manual
//! folding ranges and the gutter and `⋯` clicks.

use gpui::{App, Context};
use text::PointUtf16;

use crate::cursor::Selection;
use crate::editor::{CodeEditor, FoldingRegionState, FoldingViewState};
use crate::providers::FoldingRangeKind;
use crate::view::fold_model::{self as fm, FoldingModel, SelectedRows};
use crate::view::reveal::RevealType;

impl CodeEditor {
    /// Monaco `FoldingAction.getSelectedLines`: each selection's first row.
    fn selected_rows(&self) -> Vec<u32> {
        self.cursors
            .selections()
            .iter()
            .map(|s| s.start().row)
            .collect()
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
        let buffer = self.model().read(cx).buffer().clone();
        let rows = self.selected_rows();
        let collapsed = self.folds.collapsed_flags(&buffer);
        let ranges = self.folds.ranges().to_vec();
        let toggle = command(&FoldingModel::new(&ranges, collapsed), &rows);
        self.folds.toggle(&buffer, toggle);
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
        let buffer = self.model().read(cx).buffer().clone();
        let collapsed = self.folds.collapsed_flags(&buffer);
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
        self.folds.toggle(&buffer, toggle);
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

    /// Toggles the import ranges (Monaco `editor.toggleImportFold`).
    pub fn toggle_import_fold(&mut self, cx: &mut Context<Self>) {
        self.run_fold_command(cx, |m, _| {
            (0..m.ranges.len())
                .rev()
                .filter(|&ix| m.ranges[ix].kind == Some(FoldingRangeKind::Imports))
                .collect()
        });
    }

    fn go_to_fold_line(
        &mut self,
        cx: &mut Context<Self>,
        find: impl FnOnce(u32, &FoldingModel) -> Option<u32>,
    ) {
        let buffer = self.model().read(cx).buffer().clone();
        let collapsed = self.folds.collapsed_flags(&buffer);
        let ranges = self.folds.ranges().to_vec();
        let row = self.cursors.selections()[0].start().row;
        if let Some(target) = find(row, &FoldingModel::new(&ranges, collapsed)) {
            let start = PointUtf16::new(target, 0);
            self.set_selections(&[Selection::collapsed(start)], cx);
            self.reveal_range(start..start, RevealType::Simple, cx);
        }
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
        let buffer = self.model().read(cx).buffer().clone();
        let mut cursor = None;
        for selection in self.cursors.selections() {
            let (start, end) = (selection.start(), selection.end());
            let last = if end.column == 0 {
                end.row.saturating_sub(1)
            } else {
                end.row
            };
            if last > start.row {
                self.folds.add_manual(&buffer, start.row, last);
                cursor = Some(PointUtf16::new(start.row, 0));
            }
        }
        if let Some(cursor) = cursor {
            self.set_selections(&[Selection::collapsed(cursor)], cx);
        }
        self.after_fold_command(cx);
    }

    /// Removes manual folding ranges around the cursors or touching the
    /// selections (Monaco `editor.removeManualFoldingRanges`).
    pub fn remove_manual_folding_ranges(&mut self, cx: &mut Context<Self>) {
        let buffer = self.model().read(cx).buffer().clone();
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
        self.folds.remove_manual(&buffer, &selections);
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
        let buffer = self.model().read(cx).buffer().clone();
        let collapsed = self.folds.collapsed_flags(&buffer);
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
        self.folds.toggle(&buffer, toggle);
        self.after_fold_change(cx);
        // Monaco `FoldingController.onEditorMouseUp`: the clicked line shows
        // (`revealPositionInCenterIfOutsideViewport`), not the cursor.
        self.reveal_position_in_center_if_outside_viewport(PointUtf16::new(row, 0), cx);
    }

    /// Moves cursors out of hidden rows to the end of their fold's start row.
    /// Like Monaco's line mapping change (`Cursor.onLineMappingChanged`), it
    /// doesn't scroll.
    pub(crate) fn after_fold_change(&mut self, cx: &mut Context<Self>) {
        self.move_cursors_out_of_folds(cx);
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

    fn move_cursors_out_of_folds(&mut self, cx: &mut Context<Self>) {
        let buffer = self.model().read(cx).buffer().clone();
        let collapsed = self.folds.collapsed_rows(&buffer);
        let fix = |p: PointUtf16| match collapsed
            .iter()
            .find(|(start, end)| *start < p.row && p.row <= *end)
        {
            Some((start, _)) => PointUtf16::new(*start, self.snapshot.line_len_utf16(*start)),
            None => p,
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
    /// the collapsed regions and the manual ranges, with a checksum of
    /// their lines.
    pub(crate) fn save_folding_state(&self, cx: &App) -> FoldingViewState {
        let model = self.model().read(cx);
        if !self.options.folding || model.is_too_large_for_tokenization() {
            return FoldingViewState::default();
        }
        let buffer = model.buffer();
        let line_count = buffer.rope().line_count();
        let manual = self.folds.manual_rows(buffer);
        let collapsed = self.folds.collapsed_rows(buffer);
        let mut regions: Vec<FoldingRegionState> = collapsed
            .iter()
            .map(|&(start_row, end_row)| FoldingRegionState {
                start_row,
                end_row,
                is_collapsed: true,
                manual: manual.contains(&(start_row, end_row)),
                checksum: None,
            })
            .collect();
        regions.extend(
            manual
                .iter()
                .filter(|range| !collapsed.contains(range))
                .map(|&(start_row, end_row)| FoldingRegionState {
                    start_row,
                    end_row,
                    is_collapsed: false,
                    manual: true,
                    checksum: None,
                }),
        );
        regions.retain(|r| r.start_row < r.end_row && r.end_row < line_count);
        regions.sort_by_key(|r| (r.start_row, std::cmp::Reverse(r.end_row)));
        for region in &mut regions {
            region.checksum = Some(lines_checksum(
                buffer.rope(),
                region.start_row,
                region.end_row,
            ));
        }
        FoldingViewState {
            regions,
            line_count,
        }
    }

    /// Monaco `FoldingController.restoreViewState`
    /// (`FoldingModel.applyMemento`): the regions whose lines still match
    /// their checksum fold again, without revealing the cursor.
    pub(crate) fn restore_folding_state(
        &mut self,
        state: &FoldingViewState,
        cx: &mut Context<Self>,
    ) {
        let model = self.model().read(cx);
        if !self.options.folding
            || model.is_too_large_for_tokenization()
            || state.regions.is_empty()
        {
            return;
        }
        let buffer = model.buffer().clone();
        let line_count = buffer.rope().line_count();
        for region in &state.regions {
            let (start, end) = (region.start_row, region.end_row);
            if start >= end || end >= line_count {
                continue;
            }
            if region
                .checksum
                .is_some_and(|checksum| checksum != lines_checksum(buffer.rope(), start, end))
            {
                continue;
            }
            if region.manual {
                if !self.folds.manual_rows(&buffer).contains(&(start, end)) {
                    self.folds.add_manual(&buffer, start, end);
                }
                if !region.is_collapsed {
                    self.folds.expand(&buffer, start);
                }
            } else if region.is_collapsed {
                self.folds.collapse(&buffer, start, end);
            }
        }
        self.move_cursors_out_of_folds(cx);
        cx.notify();
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
