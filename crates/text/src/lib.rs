//! Text storage for gpui-monaco: a rope built on `sum_tree`, coordinate
//! conversions, anchors, edits and undo history.
//!
//! This crate intentionally has no gpui dependency so it can be tested fast
//! and in isolation.
#![warn(missing_docs)]

mod buffer;
mod indentation;
mod line_ending;
mod patch;
mod point;
mod rope;

pub use buffer::{
    Anchor, Buffer, BufferSnapshot, EditError, Stickiness, TransactionId, TransactionMetadata,
    UndoOutcome, Version,
};
pub use indentation::{GuessedIndentation, guess_indentation};
pub use line_ending::{LineEnding, normalize_line_endings};
pub use patch::{Edit, Patch};
pub use point::{Location, OffsetUtf16, Point, PointUtf16, Target};
pub use rope::{Chunks, Rope, TextSummary};
pub use sum_tree::Bias;
