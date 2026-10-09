//! A Monaco-like code editor component for GPUI.
//!
//! The public API is split like Monaco's: a [`TextModel`] entity holds the
//! text, language, undo history, decorations and markers, and any number of
//! [`CodeEditor`] entities render and edit it.
//!
//! Call [`init`] once at startup to bind Monaco's default keybindings.

pub mod actions;
pub mod cursor;
pub mod decorations;
mod editor;
mod element;
mod find;
mod find_widget;
mod model;
mod multi_cursor;
pub mod options;
pub mod search;
pub mod theme;
pub mod view;
pub mod widgets;

pub use decorations::{
    DecorationId, DecorationOptions, DecorationUnderline, InjectedText, OverviewRulerLane,
};
pub use editor::{CodeEditor, CodeEditorEvent, ViewState};
pub use element::EditorElement;
pub use find::{FindOptions, MATCHES_LIMIT};
pub use find_widget::FindWidget;
pub use model::{
    EditKind, ModelOptions, SyntaxSnapshot, TextModel, TextModelEvent, language_registry,
    set_language_registry,
};
pub use options::EditorOptions;
pub use syntax;
pub use text;
pub use theme::{EditorColors, Theme, ThemeKind, TokenStyle};
pub use widgets::{
    ContentWidget, ContentWidgetPreference, OverlayPosition, OverlayWidget, ViewZone, WidgetId,
    ZoneHeight,
};

/// Binds Monaco's default keybindings in the editor's key context.
pub fn init(cx: &mut gpui::App) {
    actions::bind_default_keys(cx);
}
