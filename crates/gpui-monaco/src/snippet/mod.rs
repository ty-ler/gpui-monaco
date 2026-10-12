//! Snippets (Monaco's `contrib/snippet`): the TextMate snippet syntax.

pub mod parser;
#[cfg(test)]
mod parser_tests;
pub mod variables;
#[cfg(test)]
mod variables_tests;
