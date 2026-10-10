//! The view model: how model rows become rendered "view lines" once folding
//! and soft wrapping are applied (Monaco's `viewModel`).

pub mod bracket_pairs;
pub mod folding;
pub mod guides;
pub mod line_breaks;
pub mod prefix_sum;
pub mod reveal;
pub mod view_lines;
