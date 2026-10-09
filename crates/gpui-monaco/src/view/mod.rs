//! The view model: how model rows become rendered "view lines" once folding
//! and soft wrapping are applied (Monaco's `viewModel`).

pub mod folding;
pub mod line_breaks;
pub mod prefix_sum;
pub mod view_lines;
