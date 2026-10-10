//! A Monaco-like code editor component for GPUI.
//!
//! The public API is split like Monaco's: a [`TextModel`] entity holds the
//! text, language, undo history, decorations and markers, and any number of
//! [`CodeEditor`] entities render and edit it.
//!
//! Call [`init`] once at startup to bind Monaco's default keybindings.
#![warn(missing_docs)]

pub mod actions;
mod api;
mod bracket_colors;
mod bracket_matching;
pub mod code_action;
mod commands;
mod completion;
pub mod cursor;
pub mod decorations;
mod definition;
mod editor;
mod element;
pub mod events;
mod find;
mod find_widget;
mod format;
mod goto;
mod hover;
mod inlay_hints;
pub mod markdown;
mod markers;
mod message;
mod model;
pub mod mouse_target;
mod multi_cursor;
mod navigation_history;
mod occurrences;
pub mod options;
mod peek;
pub mod providers;
mod rename;
pub mod search;
mod signature_help;
pub mod suggest;
mod suggest_widget;
pub mod theme;
pub mod view;
pub mod widgets;

pub use api::{DecorationsCollection, WordAtPosition};
pub use commands::{execute_command, register_command, unregister_command};
pub use decorations::{
    DecorationId, DecorationOptions, DecorationUnderline, InjectedText, OverviewRulerLane,
};
pub use editor::{CodeEditor, CodeEditorEvent, ViewState};
pub use element::EditorElement;
pub use events::{ContentChange, CursorChangeReason, CursorChangeSource, SelectionChange};
pub use find::{FindOptions, MATCHES_LIMIT};
pub use find_widget::FindWidget;
pub use goto::has_symbol_navigation;
pub use hover::{HOVER_DELAY, HoverWidget};
pub use markers::{Marker, MarkerSeverity, MarkerZone};
pub use model::{
    EditKind, ModelOptions, SyntaxSnapshot, TextModel, TextModelEvent, language_registry,
    model_for_uri, register_model, set_language_registry, set_model_resolver,
};
pub use mouse_target::{EditorMouseEvent, MouseTarget, MouseTargetKind};
pub use options::EditorOptions;
pub use signature_help::SignatureHelpWidget;
pub use suggest_widget::SuggestWidget;
pub use syntax;
pub use text;
pub use theme::{EditorColors, Theme, ThemeKind, TokenStyle};
pub use view::reveal::RevealType;
pub use widgets::{
    ContentWidget, ContentWidgetPreference, OverlayPosition, OverlayWidget, ViewZone, WidgetId,
    ZoneHeight,
};

/// Binds Monaco's default keybindings in the editor's key context.
pub fn init(cx: &mut gpui::App) {
    actions::bind_default_keys(cx);
}
