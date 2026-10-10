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
mod code_lens;
pub mod color;
mod color_picker;
pub use color_picker::{ColorPickerWidget, STANDALONE_COLOR_PICKER_KEY_CONTEXT};
mod color_detector;
mod commands;
mod context_menu;
pub use context_menu::CONTEXT_MENU_KEY_CONTEXT;
mod completion;
pub mod cursor;
pub mod decorations;
mod definition;
mod dnd;
mod drop_paste;
mod editor;
mod editor_actions;
mod element;
pub mod events;
mod find;
mod find_widget;
mod folding_commands;
mod font_zoom;
pub use font_zoom::{editor_zoom_level, set_editor_zoom_level};
mod format;
mod goto;
mod hover;
mod inlay_hints;
mod inline_completions;
pub mod link_computer;
mod linked_editing;
mod links;
pub mod markdown;
mod markers;
mod message;
mod model;
pub mod mouse_target;
mod multi_cursor;
mod navigation_history;
mod occurrences;
pub mod options;
pub mod outline;
mod peek;
pub mod providers;
pub mod quick_access;
mod rename;
pub mod replace_pattern;
pub mod search;
mod section_headers;
mod semantic_theme;
mod semantic_tokens;
mod signature_help;
mod smart_select;
pub mod snippet;
mod snippet_session;
mod sticky_model;
pub mod suggest;
mod suggest_widget;
pub mod theme;
pub mod unicode_highlight;
mod unicode_highlighter;
mod unusual_line_terminators;
pub mod view;
pub mod widgets;

pub use api::{DecorationsCollection, WordAtPosition};
pub use commands::{execute_command, register_command, unregister_command};
pub use decorations::{
    DecorationId, DecorationOptions, DecorationUnderline, InjectedText, OverviewRulerLane,
};
pub use editor::{CodeEditor, CodeEditorEvent, ViewState};
pub use editor_actions::{ActionPrecondition, ActionRun, EditorActionDescriptor, EditorActionInfo};
pub use element::EditorElement;
pub use events::{ContentChange, CursorChangeReason, CursorChangeSource, SelectionChange};
pub use find::{FindOptions, MATCHES_LIMIT};
pub use find_widget::FindWidget;
pub use goto::has_symbol_navigation;
pub use hover::{HOVER_DELAY, HoverWidget};
pub use markers::{Marker, MarkerSeverity, MarkerTag, MarkerZone, RelatedInformation};
pub use model::{
    EditKind, ModelOptions, SyntaxSnapshot, TextModel, TextModelEvent, language_registry,
    model_for_uri, register_model, set_language_registry, set_model_resolver,
};
pub use mouse_target::{EditorMouseEvent, MouseTarget, MouseTargetKind};
pub use options::EditorOptions;
pub use signature_help::SignatureHelpWidget;
pub use snippet_session::{SnippetInsertOptions, set_snippet_workspace};
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
