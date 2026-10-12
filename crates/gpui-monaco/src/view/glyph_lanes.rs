//! The glyph margin's lanes (Monaco `GlyphMarginLanesModel`,
//! `src/vs/editor/common/viewModel/glyphLanesModel.ts`, MIT): which lanes
//! each line's glyphs use, and how many lanes the margin needs for the line
//! that uses the most.

use std::ops::RangeInclusive;

use crate::decorations::GlyphMarginLane;

const LANES: [GlyphMarginLane; 3] = [
    GlyphMarginLane::Left,
    GlyphMarginLane::Center,
    GlyphMarginLane::Right,
];

fn bit(lane: GlyphMarginLane) -> u8 {
    1 << (lane as u8 - 1)
}

/// Monaco `GlyphMarginLanesModel`, by 0-based row: a bit per lane in use.
#[derive(Clone, Debug)]
pub(crate) struct GlyphMarginLanesModel {
    lanes: Vec<u8>,
    /// Lanes in use on every line (Monaco `persistLane`).
    persist: u8,
    required_lanes: usize,
}

impl Default for GlyphMarginLanesModel {
    fn default() -> Self {
        Self::new(0)
    }
}

impl GlyphMarginLanesModel {
    /// A model for rows up to `max_row`.
    pub fn new(max_row: u32) -> Self {
        Self {
            lanes: vec![0; max_row as usize + 1],
            persist: 0,
            // Always at least one lane.
            required_lanes: 1,
        }
    }

    /// Monaco `reset`: empty, for rows up to `max_row`.
    pub fn reset(&mut self, max_row: u32) {
        self.lanes.clear();
        self.lanes.resize(max_row as usize + 1, 0);
        self.required_lanes = 1;
    }

    /// The lanes the margin needs (Monaco `requiredLanes`).
    pub fn required_lanes(&self) -> usize {
        self.required_lanes
    }

    /// Monaco `push`: `lane` is in use on `rows`, or on every row with
    /// `persist`.
    pub fn push(&mut self, lane: GlyphMarginLane, rows: RangeInclusive<u32>, persist: bool) {
        if persist {
            self.persist |= bit(lane);
        }
        for row in rows {
            let row = row as usize;
            if row >= self.lanes.len() {
                self.lanes.resize(row + 1, 0);
            }
            self.lanes[row] |= bit(lane);
            self.required_lanes = self.required_lanes.max(self.count_at(row as u32));
        }
    }

    fn mask_at(&self, row: u32) -> u8 {
        self.persist | self.lanes.get(row as usize).copied().unwrap_or(0)
    }

    fn count_at(&self, row: u32) -> usize {
        self.mask_at(row).count_ones() as usize
    }

    /// Monaco `getLanesAtLine`: the lanes in use on `row`, left to right;
    /// the center one when none is.
    pub fn lanes_at(&self, row: u32) -> Vec<GlyphMarginLane> {
        let mask = self.mask_at(row);
        if mask == 0 {
            return vec![GlyphMarginLane::Center];
        }
        LANES
            .into_iter()
            .filter(|lane| mask & bit(*lane) != 0)
            .collect()
    }

    /// Where `lane` is drawn on `row`: its index among the lanes in use
    /// there (Monaco's `getLanesAtLine(…).indexOf(lane)`).
    pub fn lane_index(&self, row: u32, lane: GlyphMarginLane) -> usize {
        let mask = self.mask_at(row);
        if mask == 0 {
            return 0;
        }
        (mask & (bit(lane) - 1)).count_ones() as usize
    }
}

#[cfg(test)]
mod tests {
    //! Ports `glyphLanesModel.test.ts`, with 0-based rows.
    use super::*;
    use GlyphMarginLane::{Center, Left, Right};

    fn assert_lines(model: &GlyphMarginLanesModel, from: u32, expected: &[&[GlyphMarginLane]]) {
        let actual: Vec<Vec<GlyphMarginLane>> = (0..expected.len() as u32)
            .map(|i| model.lanes_at(from + i))
            .collect();
        let expected: Vec<Vec<GlyphMarginLane>> = expected.iter().map(|l| l.to_vec()).collect();
        assert_eq!(actual, expected, "from row {from}");
    }

    #[test]
    fn handles_empty() {
        let model = GlyphMarginLanesModel::new(9);
        assert_eq!(model.required_lanes(), 1);
        assert_lines(&model, 0, &[&[Center]]);
    }

    #[test]
    fn works_with_a_single_line_range() {
        let mut model = GlyphMarginLanesModel::new(9);
        model.push(Left, 1..=2, false);
        assert_eq!(model.required_lanes(), 1);
        assert_lines(
            &model,
            0,
            &[&[Center], &[Left], &[Left], &[Center], &[Center]],
        );
    }

    #[test]
    fn persists_ranges() {
        let mut model = GlyphMarginLanesModel::new(9);
        model.push(Left, 1..=2, true);
        assert_eq!(model.required_lanes(), 1);
        assert_lines(&model, 0, &[&[Left], &[Left], &[Left], &[Left], &[Left]]);
    }

    #[test]
    fn handles_overlaps() {
        let mut model = GlyphMarginLanesModel::new(9);
        model.push(Left, 5..=8, false);
        model.push(Right, 4..=6, false);
        model.push(Center, 6..=7, false);
        assert_eq!(model.required_lanes(), 3);
        assert_lines(
            &model,
            4,
            &[
                &[Right],
                &[Left, Right],
                &[Left, Center, Right],
                &[Left, Center],
                &[Left],
                &[Center],
            ],
        );
        assert_eq!(model.lane_index(6, Right), 2);
        assert_eq!(model.lane_index(5, Right), 1);
        assert_eq!(model.lane_index(9, Center), 0);
    }
}
