//! [`FoldState`] through edits and updates.
//!
//! `sanitize_and_merge*` are ported from Monaco's
//! `contrib/folding/test/browser/foldingRanges.test.ts`, `update` and
//! `delete` from `foldingModel.test.ts` and `has_ranges` from
//! `hiddenRangeModel.test.ts` (MIT). Their line numbers are Monaco's
//! (1-based) where the helpers say so.

use text::{Buffer, Point};

use crate::view::fold_model::FoldingModel;
use crate::view::folding::{
    FoldSource, FoldState, FoldingRangeKind, MergeRange, compute_indent_ranges, sanitize_and_merge,
};

fn fold_range(start: u32, end: u32, collapsed: bool, source: FoldSource, kind: &str) -> MergeRange {
    MergeRange {
        start,
        end,
        kind: (!kind.is_empty()).then(|| FoldingRangeKind::Other(kind.to_string().into())),
        collapsed,
        source,
    }
}

use FoldSource::{Provider, Recovered, UserDefined};

#[test]
fn sanitize_and_merge1() {
    let r = fold_range;
    // Monaco's first range starts at line 0, which doesn't exist; rows
    // start at 0, so it's left out.
    let set1 = [
        r(1, 100, false, Provider, "A"),
        r(1, 100, false, Provider, "Z"),
        r(10, 10, false, Provider, ""),
        r(20, 80, false, Provider, "C1"),
        r(22, 80, true, Provider, "D1"),
        r(90, 101, false, Provider, ""),
    ];
    let set2 = [
        r(20, 80, true, Provider, ""),
        r(18, 80, true, Provider, ""),
        r(21, 81, true, Provider, "Z"),
        r(22, 80, true, Provider, "D2"),
    ];
    // Monaco's `maxLineNumber` 100 is a line count of 101 here, the
    // numbers being rows.
    let result = sanitize_and_merge(&set1, &set2, 101, None);
    assert_eq!(
        result,
        [
            r(1, 100, false, Provider, "A"),
            r(20, 80, true, Provider, "C1"),
            r(22, 80, true, Provider, "D1"),
        ]
    );
}

#[test]
fn sanitize_and_merge2() {
    let r = fold_range;
    let set1 = [
        r(1, 100, false, Provider, "a1"),
        r(2, 100, false, Provider, "a2"),
        r(3, 19, false, Provider, "a3"),
        r(20, 71, false, Provider, "a4"),
        r(21, 29, false, Provider, "a5"),
        r(81, 91, false, Provider, "a6"),
    ];
    let set2 = [
        r(30, 39, true, Provider, "b1"),
        r(40, 49, true, UserDefined, "b2"),
        r(50, 100, true, UserDefined, "b3"),
        r(80, 90, true, UserDefined, "b4"),
        r(92, 100, true, UserDefined, "b5"),
    ];
    let result = sanitize_and_merge(&set1, &set2, 101, None);
    assert_eq!(
        result,
        [
            r(1, 100, false, Provider, "a1"),
            r(2, 100, false, Provider, "a2"),
            r(3, 19, false, Provider, "a3"),
            r(21, 29, false, Provider, "a5"),
            r(30, 39, true, Recovered, "b1"),
            r(40, 49, true, UserDefined, "b2"),
            r(50, 100, true, UserDefined, "b3"),
            r(80, 90, true, UserDefined, "b4"),
            r(92, 100, true, UserDefined, "b5"),
        ]
    );
}

#[test]
fn sanitize_and_merge3() {
    let r = fold_range;
    let set1 = [
        r(1, 100, false, Provider, "a1"),
        r(10, 29, false, Provider, "a2"),
        r(35, 39, true, Recovered, "a3"),
    ];
    let set2 = [
        r(10, 29, true, Recovered, "b1"),
        r(20, 28, true, Provider, "b2"),
        r(30, 39, true, Recovered, "b3"),
    ];
    let result = sanitize_and_merge(&set1, &set2, 101, None);
    assert_eq!(
        result,
        [
            r(1, 100, false, Provider, "a1"),
            r(10, 29, true, Provider, "a2"),
            r(20, 28, true, Recovered, "b2"),
            r(30, 39, true, Recovered, "b3"),
            r(35, 39, true, Recovered, "a3"),
        ]
    );
}

#[test]
fn sanitize_and_merge4() {
    let r = fold_range;
    let set1 = [r(1, 100, false, Provider, "a1")];
    let set2 = [
        r(20, 28, true, Provider, "b1"),
        r(30, 38, true, Provider, "b2"),
    ];
    let result = sanitize_and_merge(&set1, &set2, 101, None);
    assert_eq!(
        result,
        [
            r(1, 100, false, Provider, "a1"),
            r(20, 28, true, Recovered, "b1"),
            r(30, 38, true, Recovered, "b2"),
        ]
    );
}

/// A buffer with the regions computed from indentation.
fn state(lines: &[&str], off_side: bool) -> (Buffer, FoldState) {
    let buffer = Buffer::new(&lines.join("\n"));
    let mut state = FoldState::default();
    recompute(&buffer, &mut state, off_side, &[]);
    (buffer, state)
}

/// Monaco `FoldingModel.update` with ranges computed from indentation.
fn recompute(buffer: &Buffer, state: &mut FoldState, off_side: bool, selection: &[u32]) -> bool {
    let rope = buffer.rope().clone();
    state.set_ranges(
        compute_indent_ranges(&rope, 4, off_side, None),
        buffer.version(),
        &rope,
        buffer,
        selection,
    )
}

/// The regions as Monaco's `(startLineNumber, endLineNumber, isCollapsed)`.
fn regions(state: &FoldState) -> Vec<(u32, u32, bool)> {
    state
        .ranges()
        .iter()
        .zip(state.collapsed_flags())
        .map(|(range, collapsed)| (range.start_row + 1, range.end_row + 1, collapsed))
        .collect()
}

/// Toggles the innermost regions at Monaco's 1-based `lines`.
fn toggle_at_lines(state: &mut FoldState, lines: &[u32]) -> bool {
    let model = FoldingModel::new(state.ranges(), state.collapsed_flags());
    let indexes: Vec<usize> = lines
        .iter()
        .filter_map(|line| model.find_range(line - 1))
        .collect();
    state.toggle(indexes)
}

fn offset(buffer: &Buffer, row: u32, column: u32) -> usize {
    buffer.rope().point_to_offset(Point::new(row, column))
}

/// The hidden rows as Monaco's 1-based `(startLineNumber, endLineNumber)`.
fn hidden(state: &FoldState) -> Vec<(u32, u32)> {
    state
        .hidden_rows()
        .iter()
        .map(|rows| (rows.start + 1, rows.end))
        .collect()
}

#[test]
fn update() {
    let (mut buffer, mut state) = state(
        &[
            "/**",
            " * Comment",
            " */",
            "class A {",
            "  void foo() {",
            "    // comment {",
            "  }",
            "}",
        ],
        false,
    );
    assert_eq!(
        regions(&state),
        [(1, 3, false), (4, 7, false), (5, 6, false)]
    );
    toggle_at_lines(&mut state, &[2, 5]);
    let at = offset(&buffer, 3, 0);
    buffer.edit([(at..at, "//hello\n")]).unwrap();
    recompute(&buffer, &mut state, false, &[]);
    assert_eq!(regions(&state), [(1, 3, true), (5, 8, false), (6, 7, true)]);
}

#[test]
fn delete() {
    let (mut buffer, mut state) = state(
        &[
            "function foo() {",
            "  switch (x) {",
            "    case 1:",
            "      //hello1",
            "      break;",
            "    case 2:",
            "      //hello2",
            "      break;",
            "    case 3:",
            "      //hello3",
            "      break;",
            "  }",
            "}",
        ],
        false,
    );
    assert_eq!(
        regions(&state),
        [
            (1, 12, false),
            (2, 11, false),
            (3, 5, false),
            (6, 8, false),
            (9, 11, false)
        ]
    );
    toggle_at_lines(&mut state, &[6]);
    // Monaco's `Range(6, 11, 9, 0)`, validated to (9, 1).
    let range = offset(&buffer, 5, 10)..offset(&buffer, 8, 0);
    buffer.edit([(range, "")]).unwrap();
    recompute(&buffer, &mut state, true, &[6]);
    assert_eq!(
        regions(&state),
        [(1, 9, false), (2, 8, false), (3, 5, false), (6, 8, false)]
    );
}

#[test]
fn has_ranges() {
    let (_, mut state) = state(
        &[
            "/**",
            " * Comment",
            " */",
            "class A {",
            "  void foo() {",
            "    if (true) {",
            "      //hello",
            "    }",
            "  }",
            "}",
        ],
        false,
    );
    assert!(!state.has_hidden());
    toggle_at_lines(&mut state, &[1, 6]);
    assert_eq!(hidden(&state), [(2, 3), (7, 7)]);
    assert!(state.has_hidden());
    let hidden_lines = |state: &FoldState| -> Vec<u32> {
        (1..=10).filter(|line| state.is_hidden(line - 1)).collect()
    };
    assert_eq!(hidden_lines(&state), [2, 3, 7]);
    toggle_at_lines(&mut state, &[4]);
    assert_eq!(hidden(&state), [(2, 3), (5, 9)]);
    assert_eq!(hidden_lines(&state), [2, 3, 5, 6, 7, 8, 9]);
    toggle_at_lines(&mut state, &[1, 6, 4]);
    assert!(hidden(&state).is_empty());
    assert!(!state.has_hidden());
    assert!(hidden_lines(&state).is_empty());
}

const BLOCKS: [&str; 7] = ["a {", "  b {", "    c", "  }", "}", "d {", "  e"];

#[test]
fn regions_move_with_edits_before_recomputing() {
    let (mut buffer, mut state) = state(&BLOCKS, false);
    assert_eq!(
        regions(&state),
        [(1, 4, false), (2, 3, false), (6, 7, false)]
    );
    toggle_at_lines(&mut state, &[2]);
    // Two lines above everything: the regions and hidden rows follow at
    // once, with no new ranges computed.
    buffer.edit([(0..0, "x\ny\n")]).unwrap();
    assert!(state.sync(&buffer));
    assert_eq!(
        regions(&state),
        [(3, 6, false), (4, 5, true), (8, 9, false)]
    );
    assert_eq!(hidden(&state), [(5, 5)]);
    assert!(state.range_starting_at(2).is_some());
    assert!(state.range_starting_at(0).is_none());
    // Typing within a line moves nothing.
    let at = offset(&buffer, 4, 2);
    buffer.edit([(at..at, "zz")]).unwrap();
    assert!(!state.sync(&buffer));
    // Deleting the lines above moves them back.
    buffer.edit([(0..4, "")]).unwrap();
    state.sync(&buffer);
    assert_eq!(
        regions(&state),
        [(1, 4, false), (2, 3, true), (6, 7, false)]
    );
}

#[test]
fn lines_typed_into_a_collapsed_region_stay_hidden() {
    let (mut buffer, mut state) = state(&BLOCKS, false);
    toggle_at_lines(&mut state, &[1]);
    assert_eq!(hidden(&state), [(2, 4)]);
    // A new line in the middle of the hidden ones, as Replace All or Format
    // Document would add: nothing unfolds.
    let at = offset(&buffer, 2, 5);
    buffer.edit([(at..at, "\n    c2")]).unwrap();
    state.sync(&buffer);
    assert_eq!(regions(&state)[0], (1, 5, true));
    assert_eq!(hidden(&state), [(2, 5)]);
    // Recomputing keeps it collapsed: no cursor is in it.
    recompute(&buffer, &mut state, false, &[]);
    assert_eq!(regions(&state)[0], (1, 5, true));
    assert_eq!(hidden(&state), [(2, 5)]);
}

#[test]
fn a_line_typed_after_a_collapsed_header_shows_until_the_update() {
    let (mut buffer, mut state) = state(&BLOCKS, false);
    toggle_at_lines(&mut state, &[1]);
    // Enter at the end of the collapsed header: the new line shows (Monaco's
    // view model keeps the header's visibility for it) and the region
    // grows, as its decoration does.
    let at = offset(&buffer, 0, 3);
    buffer.edit([(at..at, "\n  n")]).unwrap();
    state.sync(&buffer);
    assert_eq!(hidden(&state), [(3, 5)]);
    assert_eq!(regions(&state)[0], (1, 5, true));
    // The update derives the hidden rows from the region again; with the
    // same end it stays collapsed, so the line hides.
    assert!(recompute(&buffer, &mut state, false, &[]));
    assert_eq!(regions(&state)[0], (1, 5, true));
    assert_eq!(hidden(&state), [(2, 5)]);
}

#[test]
fn a_cursor_in_a_changed_collapsed_region_expands_it_on_update() {
    let (mut buffer, mut state) = state(&BLOCKS, false);
    toggle_at_lines(&mut state, &[1]);
    // The last line of the block, indented into it: the computed range now
    // ends a line lower (Monaco keeps the collapsed state when the start
    // matches, unless a selection starts in it or right after it).
    let at = offset(&buffer, 4, 0);
    buffer.edit([(at..at, "  ")]).unwrap();
    state.sync(&buffer);
    let mut cursor_inside = state.clone();
    recompute(&buffer, &mut state, false, &[]);
    assert_eq!(regions(&state)[0], (1, 5, true));
    assert_eq!(hidden(&state), [(2, 5)]);
    recompute(&buffer, &mut cursor_inside, false, &[4]);
    assert_eq!(regions(&cursor_inside)[0], (1, 5, false));
    assert!(hidden(&cursor_inside).is_empty());
}

#[test]
fn a_collapsed_region_follows_its_recomputed_range() {
    let (mut buffer, mut state) = state(&["a", "  b", "  c", "d", "e"], false);
    assert_eq!(regions(&state), [(1, 3, false)]);
    toggle_at_lines(&mut state, &[1]);
    // Indent the line after the block into it.
    let at = offset(&buffer, 3, 0);
    buffer.edit([(at..at, "  ")]).unwrap();
    state.sync(&buffer);
    assert_eq!(hidden(&state), [(2, 3)]);
    recompute(&buffer, &mut state, false, &[]);
    assert_eq!(regions(&state), [(1, 4, true)]);
    assert_eq!(hidden(&state), [(2, 4)]);
}

#[test]
fn crossing_ranges_are_dropped() {
    let buffer = Buffer::new(&["a"; 10].join("\n"));
    let mut state = FoldState::default();
    let range = |start_row, end_row| crate::view::folding::FoldRange {
        start_row,
        end_row,
        ..Default::default()
    };
    let rope = buffer.rope().clone();
    state.set_ranges(
        vec![range(0, 5), range(2, 7), range(3, 4)],
        buffer.version(),
        &rope,
        &buffer,
        &[],
    );
    let rows: Vec<(u32, u32)> = state
        .ranges()
        .iter()
        .map(|r| (r.start_row, r.end_row))
        .collect();
    assert_eq!(rows, [(0, 5), (3, 4)]);
    assert_eq!(state.ranges()[1].parent, Some(0));
}

#[test]
fn ranges_for_an_older_version_move_through_later_edits() {
    let mut buffer = Buffer::new(&BLOCKS.join("\n"));
    let old = buffer.rope().clone();
    let version = buffer.version();
    let ranges = compute_indent_ranges(&old, 4, false, None);
    buffer.edit([(0..0, "x\n")]).unwrap();
    let mut state = FoldState::default();
    state.set_ranges(ranges, version, &old, &buffer, &[]);
    assert_eq!(
        regions(&state),
        [(2, 5, false), (3, 4, false), (7, 8, false)]
    );
}

#[test]
fn hidden_cursors_reveal_their_regions() {
    let (_, mut state) = state(&BLOCKS, false);
    toggle_at_lines(&mut state, &[1, 2]);
    assert_eq!(hidden(&state), [(2, 4)]);
    // Row 2 (line 3) is inside both: both expand.
    assert!(state.reveal_rows(&[2]));
    assert_eq!(
        regions(&state),
        [(1, 4, false), (2, 3, false), (6, 7, false)]
    );
    assert!(!state.reveal_rows(&[2]));
}

#[test]
fn manual_ranges_merge_and_go() {
    let (buffer, mut state) = state(&BLOCKS, false);
    assert!(state.add_manual(&buffer, &[(5, 6)]));
    assert_eq!(state.manual_rows(), [(5, 6)]);
    assert_eq!(regions(&state)[2], (6, 7, true));
    assert_eq!(state.ranges()[2].source(), UserDefined);
    // A recompute keeps the user-defined range over the computed one.
    recompute(&buffer, &mut state, false, &[]);
    assert_eq!(state.ranges()[2].source(), UserDefined);
    let selections = [crate::view::fold_model::SelectedRows {
        start: 5,
        end: 5,
        empty: true,
    }];
    assert!(state.remove_manual(&buffer, &selections));
    assert!(state.manual_rows().is_empty());
    recompute(&buffer, &mut state, false, &[]);
    assert_eq!(regions(&state)[2], (6, 7, false));
}

#[test]
fn replacing_the_whole_text_drops_the_regions() {
    let (buffer, mut state) = state(&BLOCKS, false);
    toggle_at_lines(&mut state, &[1]);
    state.clear(&buffer);
    assert!(state.ranges().is_empty());
    assert!(!state.has_hidden());
}

#[test]
fn typing_within_lines_moves_the_positions_on_them() {
    let (mut buffer, mut state) = state(&BLOCKS, false);
    // Type before the inner block's header end and at the end of the outer
    // block's last line: the positions there move with the text.
    let start_of_header = offset(&buffer, 1, 0);
    let end_of_last = offset(&buffer, 3, 3);
    buffer
        .edit([
            (start_of_header..start_of_header, "zz"),
            (end_of_last..end_of_last, "x"),
        ])
        .unwrap();
    assert!(!state.sync(&buffer));
    // Enter after the typed `x` grows the outer region (its end sticks to
    // the end of the line); Enter before the header's `{` moves the inner
    // region's start down with it.
    let at = offset(&buffer, 3, 4);
    buffer.edit([(at..at, "\n")]).unwrap();
    let at = offset(&buffer, 1, 6);
    buffer.edit([(at..at, "\n")]).unwrap();
    state.sync(&buffer);
    assert_eq!(
        regions(&state),
        [(1, 6, false), (3, 4, false), (8, 9, false)]
    );
}
