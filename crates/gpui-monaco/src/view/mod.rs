//! The view model: how model rows become rendered "view lines" once folding
//! and soft wrapping are applied (Monaco's `viewModel`).

pub mod animation;
pub mod bracket_pairs;
pub mod fold_model;
#[cfg(test)]
mod fold_model_tests;
pub mod folding;
pub(crate) mod glyph_lanes;
pub mod guides;
pub mod line_breaks;
pub(crate) mod minimap;
pub(crate) mod overview_ruler;
pub mod prefix_sum;
pub mod reveal;
pub mod selections;
pub mod view_lines;
