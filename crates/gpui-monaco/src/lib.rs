//! A Monaco-like code editor component for GPUI.
//!
//! The public API is split like Monaco's: a `TextModel` entity holds the text,
//! language, undo history, decorations and markers, and any number of
//! `CodeEditor` entities render and edit it.

pub use syntax;
pub use text;

pub mod cursor;
