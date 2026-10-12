//! Filtering, scoring and sorting of completion items for the suggest
//! widget (Monaco's `src/vs/base/common/filters.ts` fuzzy scorer and
//! `src/vs/editor/contrib/suggest/browser/completionModel.ts`). Free of
//! gpui so it can be tested on its own.

pub mod completion_model;
pub mod fuzzy;
pub mod memory;
pub(crate) mod word_distance;

#[cfg(test)]
mod tests;

pub use completion_model::{
    CompletionModel, CompletionModelOptions, LineContext, ScoredCompletion, SnippetSortOrder,
};
pub use fuzzy::{
    FuzzyScore, FuzzyScoreOptions, any_score, fuzzy_score, fuzzy_score_at, fuzzy_score_graceful,
    fuzzy_score_graceful_aggressive, fuzzy_score_graceful_aggressive_at, fuzzy_score_graceful_at,
};
