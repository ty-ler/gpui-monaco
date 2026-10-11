//! The shape of a selection's per-line rectangles (Monaco's
//! `SelectionsOverlay` in `browser/viewParts/selections/selections.ts`):
//! which corners are rounded, which get a reverse-rounded ("concave") piece
//! joining them to the line above or below, and the 1px insets between
//! several selections.

/// How one corner of a selection rectangle is drawn (Monaco `CornerStyle`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CornerStyle {
    /// An outer corner: rounded (`roundedSelection`).
    Extern,
    /// An inner corner, where the line above or below sticks out past this
    /// one: a reverse-rounded piece outside the rectangle joins them.
    Intern,
    /// Flush with the line above or below: square.
    Flat,
}

/// The corners of one line's selection rectangle (Monaco's `startStyle`
/// and `endStyle`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SelectionCorners {
    /// Top-left (in left-to-right text).
    pub start_top: CornerStyle,
    /// Bottom-left.
    pub start_bottom: CornerStyle,
    /// Top-right.
    pub end_top: CornerStyle,
    /// Bottom-right.
    pub end_bottom: CornerStyle,
}

/// Monaco `ROUNDED_PIECE_WIDTH`: the width of a reverse-rounded corner piece.
pub const ROUNDED_PIECE_WIDTH: f32 = 10.;

/// Monaco `_enrichVisibleRangesWithStyle`: the corner styles of a
/// selection's rectangles on consecutive view lines, given each as `(left,
/// right)`. Edges closer than a quarter of `typical_halfwidth_character_width`
/// count as flush.
pub fn corner_styles(
    rows: &[(f32, f32)],
    typical_halfwidth_character_width: f32,
) -> Vec<SelectionCorners> {
    let epsilon = typical_halfwidth_character_width / 4.;
    rows.iter()
        .enumerate()
        .map(|(i, &(left, right))| {
            let mut corners = SelectionCorners {
                start_top: CornerStyle::Extern,
                start_bottom: CornerStyle::Extern,
                end_top: CornerStyle::Extern,
                end_bottom: CornerStyle::Extern,
            };
            if let Some(&(prev_left, prev_right)) = i.checked_sub(1).and_then(|i| rows.get(i)) {
                if (left - prev_left).abs() < epsilon {
                    corners.start_top = CornerStyle::Flat;
                } else if left > prev_left {
                    corners.start_top = CornerStyle::Intern;
                }
                if (right - prev_right).abs() < epsilon {
                    corners.end_top = CornerStyle::Flat;
                } else if prev_left < right && right < prev_right {
                    corners.end_top = CornerStyle::Intern;
                }
            }
            if let Some(&(next_left, next_right)) = rows.get(i + 1) {
                if (left - next_left).abs() < epsilon {
                    corners.start_bottom = CornerStyle::Flat;
                } else if next_left < left && left < next_right {
                    corners.start_bottom = CornerStyle::Intern;
                }
                if (right - next_right).abs() < epsilon {
                    corners.end_bottom = CornerStyle::Flat;
                } else if right < next_right {
                    corners.end_bottom = CornerStyle::Intern;
                }
            }
            corners
        })
        .collect()
}

/// Monaco `_actualRenderOneSelection`: with several selections, each one's
/// first line is inset 1px from the top and its last line (if another) 1px
/// from the bottom. Returns `(top, bottom)` insets for line `index` of
/// `count`.
pub fn insets(index: usize, count: usize, multiple_selections: bool) -> (f32, f32) {
    if !multiple_selections {
        return (0., 0.);
    }
    let first = index == 0;
    let last = index + 1 == count;
    (
        if first { 1. } else { 0. },
        if !first && last { 1. } else { 0. },
    )
}

#[cfg(test)]
mod tests {
    use super::CornerStyle::{Extern, Flat, Intern};
    use super::*;

    fn corners(s: [CornerStyle; 4]) -> SelectionCorners {
        SelectionCorners {
            start_top: s[0],
            start_bottom: s[1],
            end_top: s[2],
            end_bottom: s[3],
        }
    }

    #[test]
    fn single_line_is_round() {
        assert_eq!(corner_styles(&[(10., 50.)], 8.), vec![corners([Extern; 4])]);
    }

    #[test]
    fn three_lines() {
        // A selection from mid-line through a longer line to mid-line:
        //     |------|
        // |--------------|
        // |-----|
        let styles = corner_styles(&[(40., 80.), (0., 120.), (0., 30.)], 8.);
        assert_eq!(
            styles,
            vec![
                // Below sticks out on both sides: the bottom-left is an
                // inner corner, the bottom right too.
                corners([Extern, Intern, Extern, Intern]),
                // The top-left sticks out past the line above (outer), and
                // the left edge is flush with the line below.
                corners([Extern, Flat, Extern, Extern]),
                corners([Flat, Extern, Intern, Extern]),
            ]
        );
    }

    #[test]
    fn near_edges_are_flush() {
        // Within a quarter of a character: flush.
        let styles = corner_styles(&[(0., 100.), (1., 101.)], 8.);
        assert_eq!(styles[0], corners([Extern, Flat, Extern, Flat]));
        assert_eq!(styles[1], corners([Flat, Extern, Flat, Extern]));
    }

    #[test]
    fn an_empty_last_line_squares_the_corner_above() {
        // Whole lines selected up to column 1 of the next line, which has
        // an empty rectangle at the left edge.
        let styles = corner_styles(&[(0., 60.), (0., 0.)], 8.);
        assert_eq!(styles[0].start_bottom, Flat);
        assert_eq!(styles[0].end_bottom, Extern);
    }

    #[test]
    fn multi_selection_insets() {
        assert_eq!(insets(0, 3, false), (0., 0.));
        assert_eq!(insets(0, 3, true), (1., 0.));
        assert_eq!(insets(1, 3, true), (0., 0.));
        assert_eq!(insets(2, 3, true), (0., 1.));
        // A one-line selection gets just the top inset, as in Monaco.
        assert_eq!(insets(0, 1, true), (1., 0.));
    }
}
