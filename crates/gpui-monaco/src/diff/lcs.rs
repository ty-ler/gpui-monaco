//! The classic Myers O(ND) longest-common-subsequence diff with Monaco's
//! "pretty diff" shifting heuristics (port of `LcsDiff` and `DiffChange`
//! from Monaco's `vs/base/common/diff/diff.ts` and `diffChange.ts`), used by
//! the [legacy lines diff computer](super::legacy).
//!
//! Indices are `isize` like the JavaScript numbers they port: the algorithm
//! starts "just outside" the input ranges (index -1).

/// Upper bound on the number of differences whose search history is kept
/// (Monaco's `LocalConstants.MaxDifferencesHistory`): about 16 MB worst case.
const MAX_DIFFERENCES_HISTORY: isize = 1447;

/// Monaco's `Constants.MAX_SAFE_SMALL_INTEGER`.
const MAX_SAFE_SMALL_INTEGER: isize = 1 << 30;

/// Monaco's `Constants.MIN_SAFE_SMALL_INTEGER`.
const MIN_SAFE_SMALL_INTEGER: isize = -(1 << 30);

/// A difference between two sequences (Monaco's `DiffChange`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct DiffChange {
    /// The first affected element of the original sequence.
    pub(crate) original_start: isize,
    /// The number of affected original elements.
    pub(crate) original_length: isize,
    /// The first affected element of the modified sequence.
    pub(crate) modified_start: isize,
    /// The number of affected modified elements.
    pub(crate) modified_length: isize,
}

impl DiffChange {
    pub(crate) fn new(
        original_start: isize,
        original_length: isize,
        modified_start: isize,
        modified_length: isize,
    ) -> Self {
        Self {
            original_start,
            original_length,
            modified_start,
            modified_length,
        }
    }

    fn original_end(&self) -> isize {
        self.original_start + self.original_length
    }

    fn modified_end(&self) -> isize {
        self.modified_start + self.modified_length
    }
}

/// A sequence to diff (Monaco's `ISequence`, already resolved to what
/// `LcsDiff` derives from it).
pub(crate) struct LcsSequence {
    /// The elements, compared by equality: character codes, or ids of
    /// interned strings (equal ids for equal strings).
    pub(crate) elements: Vec<u32>,
    /// For string sequences, per element: its UTF-16 length and whether it
    /// matches JavaScript's `/^\s*$/`.
    pub(crate) strings: Option<Vec<StringElement>>,
    /// Ids of the elements returned by `getStrictElement`, if the sequence
    /// provides them (interned across both sequences).
    pub(crate) strict_elements: Option<Vec<u32>>,
}

/// What `LcsDiff` looks at in a string element.
#[derive(Clone, Copy, Debug)]
pub(crate) struct StringElement {
    /// The UTF-16 length.
    pub(crate) len: usize,
    /// Whether the string is empty or JavaScript whitespace only.
    pub(crate) is_whitespace: bool,
}

/// The result of [`LcsDiff::compute_diff`] (Monaco's `IDiffResult`).
pub(crate) struct DiffResult {
    /// Whether the continue predicate stopped the computation early.
    pub(crate) quit_early: bool,
    /// The differences, sorted.
    pub(crate) changes: Vec<DiffChange>,
}

/// Collects changes while walking a trace (Monaco's `DiffChangeHelper`).
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

    /// Marks the beginning of the next change.
    fn mark_next_change(&mut self) {
        if self.original_count > 0 || self.modified_count > 0 {
            self.changes.push(DiffChange::new(
                self.original_start,
                self.original_count,
                self.modified_start,
                self.modified_count,
            ));
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

    fn changes(mut self) -> Vec<DiffChange> {
        if self.original_count > 0 || self.modified_count > 0 {
            self.mark_next_change();
        }
        self.changes
    }

    fn reverse_changes(self) -> Vec<DiffChange> {
        let mut changes = self.changes();
        changes.reverse();
        changes
    }
}

/// The diagonal bounds and offsets of one recursion point search, as
/// [`LcsDiff::walk_trace`] needs them.
struct TraceState<'p> {
    diagonal_forward_base: isize,
    diagonal_forward_start: isize,
    diagonal_forward_end: isize,
    diagonal_forward_offset: isize,
    diagonal_reverse_base: isize,
    diagonal_reverse_start: isize,
    diagonal_reverse_end: isize,
    diagonal_reverse_offset: isize,
    forward_points: &'p [isize],
    reverse_points: &'p [isize],
    original_end: isize,
    modified_end: isize,
    mid_original: isize,
    mid_modified: isize,
    delta_is_even: bool,
    quit_early: bool,
}

/// Myers' O(ND) difference algorithm (Monaco's `LcsDiff`).
pub(crate) struct LcsDiff<'a> {
    continue_processing: &'a dyn Fn() -> bool,
    original: LcsSequence,
    modified: LcsSequence,
    has_strings: bool,
    forward_history: Vec<Vec<isize>>,
    reverse_history: Vec<Vec<isize>>,
}

impl<'a> LcsDiff<'a> {
    /// `continue_processing` is polled once per edit distance step; when it
    /// returns false the diff is approximated.
    pub(crate) fn new(
        original: LcsSequence,
        modified: LcsSequence,
        continue_processing: &'a dyn Fn() -> bool,
    ) -> Self {
        // Like Monaco's `_isStringArray`: an empty array is not a string array.
        let is_string = |s: &LcsSequence| s.strings.is_some() && !s.elements.is_empty();
        let has_strings = is_string(&original) && is_string(&modified);
        Self {
            continue_processing,
            original,
            modified,
            has_strings,
            forward_history: Vec::new(),
            reverse_history: Vec::new(),
        }
    }

    fn elements_are_equal(&self, original_index: isize, new_index: isize) -> bool {
        self.original.elements[original_index as usize]
            == self.modified.elements[new_index as usize]
    }

    fn elements_are_strict_equal(&self, original_index: isize, new_index: isize) -> bool {
        if !self.elements_are_equal(original_index, new_index) {
            return false;
        }
        match (
            &self.original.strict_elements,
            &self.modified.strict_elements,
        ) {
            (Some(original), Some(modified)) => {
                original[original_index as usize] == modified[new_index as usize]
            }
            (None, None) => true,
            _ => false,
        }
    }

    fn original_elements_are_equal(&self, index1: isize, index2: isize) -> bool {
        self.original.elements[index1 as usize] == self.original.elements[index2 as usize]
    }

    fn modified_elements_are_equal(&self, index1: isize, index2: isize) -> bool {
        self.modified.elements[index1 as usize] == self.modified.elements[index2 as usize]
    }

    /// Computes the differences between the two sequences; `pretty` shifts
    /// them to be more intuitive (Monaco's `ComputeDiff`).
    pub(crate) fn compute_diff(mut self, pretty: bool) -> DiffResult {
        let original_end = self.original.elements.len() as isize - 1;
        let modified_end = self.modified.elements.len() as isize - 1;
        let mut quit_early = false;
        let mut changes =
            self.compute_diff_recursive(0, original_end, 0, modified_end, &mut quit_early);
        if pretty {
            // The diff can only be cleaned up once all of it is computed.
            changes = self.prettify_changes(changes);
        }
        DiffResult {
            quit_early,
            changes,
        }
    }

    /// Computes the differences on the inclusive bounded ranges recursively.
    fn compute_diff_recursive(
        &mut self,
        mut original_start: isize,
        mut original_end: isize,
        mut modified_start: isize,
        mut modified_end: isize,
        quit_early: &mut bool,
    ) -> Vec<DiffChange> {
        *quit_early = false;

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

        // All insertions, all deletions or identical sequences.
        if original_start > original_end || modified_start > modified_end {
            if modified_start <= modified_end {
                debug_assert_eq!(original_start, original_end + 1);
                return vec![DiffChange::new(
                    original_start,
                    0,
                    modified_start,
                    modified_end - modified_start + 1,
                )];
            } else if original_start <= original_end {
                debug_assert_eq!(modified_start, modified_end + 1);
                return vec![DiffChange::new(
                    original_start,
                    original_end - original_start + 1,
                    modified_start,
                    0,
                )];
            }
            return Vec::new();
        }

        // Divide and conquer.
        let mut mid_original = 0;
        let mut mid_modified = 0;
        let result = self.compute_recursion_point(
            original_start,
            original_end,
            modified_start,
            modified_end,
            &mut mid_original,
            &mut mid_modified,
            quit_early,
        );

        if let Some(result) = result {
            // The full trace was in memory.
            return result;
        } else if !*quit_early {
            // First half: (original_start, modified_start) to (mid_original,
            // mid_modified); second half from the next point to the ends.
            let left_changes = self.compute_diff_recursive(
                original_start,
                mid_original,
                modified_start,
                mid_modified,
                quit_early,
            );
            let right_changes = if !*quit_early {
                self.compute_diff_recursive(
                    mid_original + 1,
                    original_end,
                    mid_modified + 1,
                    modified_end,
                    quit_early,
                )
            } else {
                // No time for this half: consider all of it different.
                vec![DiffChange::new(
                    mid_original + 1,
                    original_end - (mid_original + 1) + 1,
                    mid_modified + 1,
                    modified_end - (mid_modified + 1) + 1,
                )]
            };
            return concatenate_changes(left_changes, right_changes);
        }

        // We quit early and have nothing meaningful.
        vec![DiffChange::new(
            original_start,
            original_end - original_start + 1,
            modified_start,
            modified_end - modified_start + 1,
        )]
    }

    /// Converts the forward and reverse search traces into changes (Monaco's
    /// `WALKTRACE`).
    fn walk_trace(&self, state: TraceState<'_>) -> Vec<DiffChange> {
        // Walk backward through the forward diagonals history.
        let mut change_helper = DiffChangeHelper::new();
        let mut diagonal_forward_base = state.diagonal_forward_base;
        let mut diagonal_min = state.diagonal_forward_start;
        let mut diagonal_max = state.diagonal_forward_end;
        let mut diagonal_relative =
            (state.mid_original - state.mid_modified) - state.diagonal_forward_offset;
        let mut last_original_index = MIN_SAFE_SMALL_INTEGER;
        let mut history_index = self.forward_history.len() as isize - 1;
        let mut points = state.forward_points;

        loop {
            let diagonal = diagonal_relative + diagonal_forward_base;
            let d = diagonal as usize;
            if diagonal == diagonal_min
                || (diagonal < diagonal_max && points[d - 1] < points[d + 1])
            {
                // Vertical line (an insertion).
                let original_index = points[d + 1];
                let modified_index =
                    original_index - diagonal_relative - state.diagonal_forward_offset;
                if original_index < last_original_index {
                    change_helper.mark_next_change();
                }
                last_original_index = original_index;
                change_helper.add_modified_element(original_index + 1, modified_index);
                diagonal_relative = (diagonal + 1) - diagonal_forward_base;
            } else {
                // Horizontal line (a deletion).
                let original_index = points[d - 1] + 1;
                let modified_index =
                    original_index - diagonal_relative - state.diagonal_forward_offset;
                if original_index < last_original_index {
                    change_helper.mark_next_change();
                }
                last_original_index = original_index - 1;
                change_helper.add_original_element(original_index, modified_index + 1);
                diagonal_relative = (diagonal - 1) - diagonal_forward_base;
            }

            if history_index >= 0 {
                points = &self.forward_history[history_index as usize];
                // The base is stored in the first slot.
                diagonal_forward_base = points[0];
                diagonal_min = 1;
                diagonal_max = points.len() as isize - 1;
            }
            history_index -= 1;
            if history_index < -1 {
                break;
            }
        }

        // The forward changes were added backwards.
        let forward_changes = change_helper.reverse_changes();

        let reverse_changes = if state.quit_early {
            // Assume everything after the mid point is a diff.
            let mut original_start_point = state.mid_original + 1;
            let mut modified_start_point = state.mid_modified + 1;
            if let Some(last_forward_change) = forward_changes.last() {
                original_start_point = original_start_point.max(last_forward_change.original_end());
                modified_start_point = modified_start_point.max(last_forward_change.modified_end());
            }
            vec![DiffChange::new(
                original_start_point,
                state.original_end - original_start_point + 1,
                modified_start_point,
                state.modified_end - modified_start_point + 1,
            )]
        } else {
            // Walk backward through the reverse diagonals history.
            let mut change_helper = DiffChangeHelper::new();
            let mut diagonal_reverse_base = state.diagonal_reverse_base;
            let mut diagonal_min = state.diagonal_reverse_start;
            let mut diagonal_max = state.diagonal_reverse_end;
            let mut diagonal_relative =
                (state.mid_original - state.mid_modified) - state.diagonal_reverse_offset;
            let mut last_original_index = MAX_SAFE_SMALL_INTEGER;
            let mut history_index = if state.delta_is_even {
                self.reverse_history.len() as isize - 1
            } else {
                self.reverse_history.len() as isize - 2
            };
            let mut points = state.reverse_points;

            loop {
                let diagonal = diagonal_relative + diagonal_reverse_base;
                let d = diagonal as usize;
                if diagonal == diagonal_min
                    || (diagonal < diagonal_max && points[d - 1] >= points[d + 1])
                {
                    // Horizontal line (a deletion).
                    let original_index = points[d + 1] - 1;
                    let modified_index =
                        original_index - diagonal_relative - state.diagonal_reverse_offset;
                    if original_index > last_original_index {
                        change_helper.mark_next_change();
                    }
                    last_original_index = original_index + 1;
                    change_helper.add_original_element(original_index + 1, modified_index + 1);
                    diagonal_relative = (diagonal + 1) - diagonal_reverse_base;
                } else {
                    // Vertical line (an insertion).
                    let original_index = points[d - 1];
                    let modified_index =
                        original_index - diagonal_relative - state.diagonal_reverse_offset;
                    if original_index > last_original_index {
                        change_helper.mark_next_change();
                    }
                    last_original_index = original_index;
                    change_helper.add_modified_element(original_index + 1, modified_index + 1);
                    diagonal_relative = (diagonal - 1) - diagonal_reverse_base;
                }

                if history_index >= 0 {
                    points = &self.reverse_history[history_index as usize];
                    diagonal_reverse_base = points[0];
                    diagonal_min = 1;
                    diagonal_max = points.len() as isize - 1;
                }
                history_index -= 1;
                if history_index < -1 {
                    break;
                }
            }

            change_helper.changes()
        };

        concatenate_changes(forward_changes, reverse_changes)
    }

    /// Finds the point `(mid_original, mid_modified)` in the middle of the
    /// LCS at which the problem can be split recursively. Returns the changes
    /// directly if the full trace is in memory (Monaco's
    /// `ComputeRecursionPoint`).
    #[allow(clippy::too_many_arguments)]
    fn compute_recursion_point(
        &mut self,
        mut original_start: isize,
        original_end: isize,
        mut modified_start: isize,
        modified_end: isize,
        mid_original: &mut isize,
        mid_modified: &mut isize,
        quit_early: &mut bool,
    ) -> Option<Vec<DiffChange>> {
        let mut diagonal_forward_start: isize = 0;
        let mut diagonal_forward_end: isize = 0;
        let mut diagonal_reverse_start: isize = 0;
        let mut diagonal_reverse_end: isize = 0;

        // The traversal starts just outside the given boundary.
        original_start -= 1;
        modified_start -= 1;

        *mid_original = 0;
        *mid_modified = 0;

        self.forward_history.clear();
        self.reverse_history.clear();

        // Each cell corresponds to a diagonal of the edit graph and holds the
        // original index of the furthest reaching point on that diagonal.
        let max_differences = (original_end - original_start) + (modified_end - modified_start);
        let num_diagonals = max_differences + 1;
        let mut forward_points = vec![0isize; num_diagonals as usize];
        let mut reverse_points = vec![0isize; num_diagonals as usize];
        // The diagonals through (original_start, modified_start) and
        // (original_end, modified_end).
        let diagonal_forward_base = modified_end - modified_start;
        let diagonal_reverse_base = original_end - original_start;
        // Offsets to compute the modified index from the original index and
        // the (relative) diagonal number.
        let diagonal_forward_offset = original_start - modified_start;
        let diagonal_reverse_offset = original_end - modified_end;

        // Its parity decides when to check for overlap.
        let delta = diagonal_reverse_base - diagonal_forward_base;
        let delta_is_even = delta % 2 == 0;

        forward_points[diagonal_forward_base as usize] = original_start;
        reverse_points[diagonal_reverse_base as usize] = original_end;

        *quit_early = false;

        macro_rules! trace_state {
            () => {
                TraceState {
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
                    original_end,
                    modified_end,
                    mid_original: *mid_original,
                    mid_modified: *mid_modified,
                    delta_is_even,
                    quit_early: *quit_early,
                }
            };
        }

        // Iterate on the number of differences; `numDifferences <=
        // maxDifferences / 2 + 1` with JavaScript's float division.
        let mut num_differences: isize = 1;
        while 2 * num_differences <= max_differences + 2 {
            let mut furthest_original_index: isize = 0;
            let mut furthest_modified_index: isize = 0;

            // Forward direction.
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
                // STEP 1: extend from the neighbor diagonal that is further.
                let mut original_index = if diagonal == diagonal_forward_start
                    || (diagonal < diagonal_forward_end
                        && forward_points[d - 1] < forward_points[d + 1])
                {
                    forward_points[d + 1]
                } else {
                    forward_points[d - 1] + 1
                };
                let mut modified_index =
                    original_index - (diagonal - diagonal_forward_base) - diagonal_forward_offset;

                // To test for false overlap in step 3.
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

                if original_index + modified_index
                    > furthest_original_index + furthest_modified_index
                {
                    furthest_original_index = original_index;
                    furthest_modified_index = modified_index;
                }

                // STEP 3: with odd delta, overlap first happens going
                // forward; check against the previous reverse iteration.
                if !delta_is_even
                    // `<= num_differences - 1` in Monaco.
                    && (diagonal - diagonal_reverse_base).abs() < num_differences
                    && original_index >= reverse_points[d]
                {
                    *mid_original = original_index;
                    *mid_modified = modified_index;

                    if temp_original_index <= reverse_points[d]
                        && num_differences <= MAX_DIFFERENCES_HISTORY + 1
                    {
                        // Overlap with the full trace in memory.
                        return Some(self.walk_trace(trace_state!()));
                    }
                    // False overlap, or not enough history: just return the
                    // recursion point.
                    return None;
                }
                diagonal += 2;
            }

            // Check whether to quit early before the next iteration.
            let match_length_of_longest_times_two = (furthest_original_index - original_start)
                + (furthest_modified_index - modified_start)
                - num_differences;

            if !(self.continue_processing)() {
                *quit_early = true;

                // Use the furthest forward point.
                *mid_original = furthest_original_index;
                *mid_modified = furthest_modified_index;

                if match_length_of_longest_times_two > 0
                    && num_differences <= MAX_DIFFERENCES_HISTORY + 1
                {
                    // Enough history to walk it backwards.
                    return Some(self.walk_trace(trace_state!()));
                }
                // Not enough history: shift the starts back into bounds.
                original_start += 1;
                modified_start += 1;
                return Some(vec![DiffChange::new(
                    original_start,
                    original_end - original_start + 1,
                    modified_start,
                    modified_end - modified_start + 1,
                )]);
            }

            // Reverse direction.
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
                // STEP 1: extend from the neighbor diagonal that is further.
                let mut original_index = if diagonal == diagonal_reverse_start
                    || (diagonal < diagonal_reverse_end
                        && reverse_points[d - 1] >= reverse_points[d + 1])
                {
                    reverse_points[d + 1] - 1
                } else {
                    reverse_points[d - 1]
                };
                let mut modified_index =
                    original_index - (diagonal - diagonal_reverse_base) - diagonal_reverse_offset;

                // To test for false overlap.
                let temp_original_index = original_index;

                // STEP 2: follow the snake.
                while original_index > original_start
                    && modified_index > modified_start
                    && self.elements_are_equal(original_index, modified_index)
                {
                    original_index -= 1;
                    modified_index -= 1;
                }
                reverse_points[d] = original_index;

                // STEP 4: with even delta, overlap first happens going
                // backward.
                if delta_is_even
                    && (diagonal - diagonal_forward_base).abs() <= num_differences
                    && original_index <= forward_points[d]
                {
                    *mid_original = original_index;
                    *mid_modified = modified_index;

                    if temp_original_index >= forward_points[d]
                        && num_differences <= MAX_DIFFERENCES_HISTORY + 1
                    {
                        // Overlap with the full trace in memory.
                        return Some(self.walk_trace(trace_state!()));
                    }
                    return None;
                }
                diagonal += 2;
            }

            // Save the vectors to the history; the first slot holds the index
            // of the base diagonal.
            if num_differences <= MAX_DIFFERENCES_HISTORY {
                let mut temp = Vec::with_capacity(
                    (diagonal_forward_end - diagonal_forward_start + 2) as usize,
                );
                temp.push(diagonal_forward_base - diagonal_forward_start + 1);
                temp.extend_from_slice(
                    &forward_points
                        [diagonal_forward_start as usize..=diagonal_forward_end as usize],
                );
                self.forward_history.push(temp);

                let mut temp = Vec::with_capacity(
                    (diagonal_reverse_end - diagonal_reverse_start + 2) as usize,
                );
                temp.push(diagonal_reverse_base - diagonal_reverse_start + 1);
                temp.extend_from_slice(
                    &reverse_points
                        [diagonal_reverse_start as usize..=diagonal_reverse_end as usize],
                );
                self.reverse_history.push(temp);
            }

            num_differences += 1;
        }

        // The full trace is in the history; convert it to changes.
        Some(self.walk_trace(trace_state!()))
    }

    /// Shifts the changes to be more intuitive (Monaco's `PrettifyChanges`).
    fn prettify_changes(&self, mut changes: Vec<DiffChange>) -> Vec<DiffChange> {
        let original_len = self.original.elements.len() as isize;
        let modified_len = self.modified.elements.len() as isize;

        // Shift all the changes down first.
        let mut i = 0;
        while i < changes.len() {
            let (original_stop, modified_stop) = match changes.get(i + 1) {
                Some(next) => (next.original_start, next.modified_start),
                None => (original_len, modified_len),
            };
            let change = &mut changes[i];
            let check_original = change.original_length > 0;
            let check_modified = change.modified_length > 0;

            while change.original_start + change.original_length < original_stop
                && change.modified_start + change.modified_length < modified_stop
                && (!check_original
                    || self.original_elements_are_equal(
                        change.original_start,
                        change.original_start + change.original_length,
                    ))
                && (!check_modified
                    || self.modified_elements_are_equal(
                        change.modified_start,
                        change.modified_start + change.modified_length,
                    ))
            {
                let start_strict_equal =
                    self.elements_are_strict_equal(change.original_start, change.modified_start);
                let end_strict_equal = self.elements_are_strict_equal(
                    change.original_start + change.original_length,
                    change.modified_start + change.modified_length,
                );
                if end_strict_equal && !start_strict_equal {
                    // Moving down would create an equal change of elements
                    // that are not strictly equal.
                    break;
                }
                change.original_start += 1;
                change.modified_start += 1;
            }

            if i + 1 < changes.len()
                && let Some(merged) = changes_overlap(&changes[i], &changes[i + 1])
            {
                changes[i] = merged;
                changes.remove(i + 1);
                continue;
            }
            i += 1;
        }

        // Shift changes back up until we hit empty or whitespace-only lines.
        let mut i = changes.len() as isize - 1;
        while i >= 0 {
            let iu = i as usize;
            if iu >= changes.len() {
                // Monaco would read `undefined` here and throw.
                i -= 1;
                continue;
            }
            let (original_stop, modified_stop) = if iu > 0 {
                let prev = &changes[iu - 1];
                (prev.original_end(), prev.modified_end())
            } else {
                (0, 0)
            };

            let change = changes[iu];
            let check_original = change.original_length > 0;
            let check_modified = change.modified_length > 0;

            let mut best_delta = 0;
            let mut best_score = self.boundary_score(
                change.original_start,
                change.original_length,
                change.modified_start,
                change.modified_length,
            );

            let mut delta = 1;
            loop {
                let original_start = change.original_start - delta;
                let modified_start = change.modified_start - delta;

                if original_start < original_stop || modified_start < modified_stop {
                    break;
                }
                if check_original
                    && !self.original_elements_are_equal(
                        original_start,
                        original_start + change.original_length,
                    )
                {
                    break;
                }
                if check_modified
                    && !self.modified_elements_are_equal(
                        modified_start,
                        modified_start + change.modified_length,
                    )
                {
                    break;
                }

                let touching_previous_change =
                    original_start == original_stop && modified_start == modified_stop;
                let score = if touching_previous_change { 5 } else { 0 }
                    + self.boundary_score(
                        original_start,
                        change.original_length,
                        modified_start,
                        change.modified_length,
                    );

                if score > best_score {
                    best_score = score;
                    best_delta = delta;
                }
                delta += 1;
            }

            changes[iu].original_start -= best_delta;
            changes[iu].modified_start -= best_delta;

            if iu > 0
                && let Some(merged) = changes_overlap(&changes[iu - 1], &changes[iu])
            {
                changes[iu - 1] = merged;
                changes.remove(iu);
                // Like Monaco (`i++; continue;` in a decrementing loop): look
                // at the same index again.
                continue;
            }
            i -= 1;
        }

        // There can be several longest common subsequences; prefer the ones
        // containing longer lines.
        if self.has_strings {
            for i in 1..changes.len() {
                let a_change = changes[i - 1];
                let b_change = changes[i];
                let matched_length = b_change.original_start - a_change.original_end();
                let a_original_start = a_change.original_start;
                let b_original_end = b_change.original_end();
                let ab_original_length = b_original_end - a_original_start;
                let a_modified_start = a_change.modified_start;
                let b_modified_end = b_change.modified_end();
                let ab_modified_length = b_modified_end - a_modified_start;
                // Avoid wasting a lot of time with these searches.
                if matched_length < 5 && ab_original_length < 20 && ab_modified_length < 20 {
                    let better = self.find_better_contiguous_sequence(
                        a_original_start,
                        ab_original_length,
                        a_modified_start,
                        ab_modified_length,
                        matched_length,
                    );
                    if let Some((original_match_start, modified_match_start)) = better
                        && (original_match_start != a_change.original_end()
                            || modified_match_start != a_change.modified_end())
                    {
                        // Switch to the sequence with the better score.
                        let a = &mut changes[i - 1];
                        a.original_length = original_match_start - a.original_start;
                        a.modified_length = modified_match_start - a.modified_start;
                        let b = &mut changes[i];
                        b.original_start = original_match_start + matched_length;
                        b.modified_start = modified_match_start + matched_length;
                        b.original_length = b_original_end - b.original_start;
                        b.modified_length = b_modified_end - b.modified_start;
                    }
                }
            }
        }

        changes
    }

    /// Monaco's `_findBetterContiguousSequence`.
    fn find_better_contiguous_sequence(
        &self,
        original_start: isize,
        original_length: isize,
        modified_start: isize,
        modified_length: isize,
        desired_length: isize,
    ) -> Option<(isize, isize)> {
        if original_length < desired_length || modified_length < desired_length {
            return None;
        }
        let original_max = original_start + original_length - desired_length + 1;
        let modified_max = modified_start + modified_length - desired_length + 1;
        let mut best_score = 0;
        let mut best_original_start = 0;
        let mut best_modified_start = 0;
        for i in original_start..original_max {
            for j in modified_start..modified_max {
                let score = self.contiguous_sequence_score(i, j, desired_length);
                if score > 0 && score > best_score {
                    best_score = score;
                    best_original_start = i;
                    best_modified_start = j;
                }
            }
        }
        (best_score > 0).then_some((best_original_start, best_modified_start))
    }

    /// Monaco's `_contiguousSequenceScore`: the total length of the matching
    /// original strings, or 0 if they do not all match.
    fn contiguous_sequence_score(
        &self,
        original_start: isize,
        modified_start: isize,
        length: isize,
    ) -> usize {
        let strings = self.original.strings.as_deref().unwrap_or_default();
        let mut score = 0;
        for l in 0..length {
            if !self.elements_are_equal(original_start + l, modified_start + l) {
                return 0;
            }
            score += strings[(original_start + l) as usize].len;
        }
        score
    }

    fn is_boundary(&self, sequence: &LcsSequence, index: isize) -> bool {
        if index <= 0 || index >= sequence.elements.len() as isize - 1 {
            return true;
        }
        self.has_strings
            && sequence
                .strings
                .as_ref()
                .is_some_and(|strings| strings[index as usize].is_whitespace)
    }

    fn region_is_boundary(&self, sequence: &LcsSequence, start: isize, length: isize) -> bool {
        if self.is_boundary(sequence, start) || self.is_boundary(sequence, start - 1) {
            return true;
        }
        if length > 0 {
            let end = start + length;
            if self.is_boundary(sequence, end - 1) || self.is_boundary(sequence, end) {
                return true;
            }
        }
        false
    }

    /// Monaco's `_boundaryScore`.
    fn boundary_score(
        &self,
        original_start: isize,
        original_length: isize,
        modified_start: isize,
        modified_length: isize,
    ) -> isize {
        let original_score =
            self.region_is_boundary(&self.original, original_start, original_length) as isize;
        let modified_score =
            self.region_is_boundary(&self.modified, modified_start, modified_length) as isize;
        original_score + modified_score
    }
}

/// Concatenates two change lists, fusing the touching changes at the seam
/// (Monaco's `ConcatenateChanges`).
fn concatenate_changes(mut left: Vec<DiffChange>, right: Vec<DiffChange>) -> Vec<DiffChange> {
    let (Some(last), Some(first)) = (left.last_mut(), right.first()) else {
        return if right.is_empty() { left } else { right };
    };
    if let Some(merged) = changes_overlap(last, first) {
        // Recursion may split a change in two; fuse it back together.
        *last = merged;
        left.extend_from_slice(&right[1..]);
    } else {
        left.extend(right);
    }
    left
}

/// The merged change if `left` and `right` overlap or touch (Monaco's
/// `ChangesOverlap`).
fn changes_overlap(left: &DiffChange, right: &DiffChange) -> Option<DiffChange> {
    debug_assert!(left.original_start <= right.original_start);
    debug_assert!(left.modified_start <= right.modified_start);

    if left.original_end() >= right.original_start || left.modified_end() >= right.modified_start {
        let mut original_length = left.original_length;
        let mut modified_length = left.modified_length;
        if left.original_end() >= right.original_start {
            original_length = right.original_end() - left.original_start;
        }
        if left.modified_end() >= right.modified_start {
            modified_length = right.modified_end() - left.modified_start;
        }
        Some(DiffChange::new(
            left.original_start,
            original_length,
            left.modified_start,
            modified_length,
        ))
    } else {
        None
    }
}

/// Clips a diagonal index to the valid diagonals, to the boundary or one
/// inside depending on parity (Monaco's `ClipDiagonalBound`).
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
