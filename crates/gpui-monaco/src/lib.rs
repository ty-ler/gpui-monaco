//! A Monaco-like code editor component for GPUI.
//!
//! The public API is split like Monaco's: a [`TextModel`] entity holds the
//! text, language, undo history, decorations and markers, and any number of
//! [`CodeEditor`] entities render and edit it.
//!
//! Call [`init`] once at startup to bind Monaco's default keybindings.
//!
//! Monaco's global `monaco.editor.*` registries and events have free
//! functions taking the app: [`editors`], [`models`], [`on_did_create_model`],
//! [`model_markers`], [`set_theme`], [`add_editor_action`],
//! [`add_keybinding_rules`] and so on.
#![warn(missing_docs)]

mod accessibility;
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
pub mod colorize;
mod commands;
mod context_menu;
pub use context_menu::CONTEXT_MENU_KEY_CONTEXT;
mod completion;
pub mod cursor;
pub mod decorations;
mod definition;
pub mod diff;
pub mod diff_editor;
pub mod multi_diff_editor;
pub use diff_editor::{
    AccessibilitySignal, DiffAlgorithm, DiffEditor, DiffEditorEvent, DiffEditorOptions,
    DiffEditorViewState, HideUnchangedRegions,
};
pub use multi_diff_editor::{MultiDiffDocument, MultiDiffEditor, MultiDiffEditorEvent};
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
mod indentation_commands;
mod tooltip;
pub use font_zoom::{editor_zoom_level, set_editor_zoom_level};
mod format;
mod goto;
mod hover;
mod inlay_hints;
mod inline_completions;
mod languages;
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
mod registry;
mod rename;
pub mod replace_pattern;
pub mod search;
mod section_headers;
mod tab_focus;
pub use tab_focus::{set_tab_focus_mode, tab_focus_mode};
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

pub use api::{
    BracketPairInfo, CursorStateComputer, DecorationsCollection, FindMatch, WordAtPosition,
};
pub use commands::{add_command, execute_command, register_command, unregister_command};
pub use decorations::{
    BlockDecoration, DecorationId, DecorationOptions, DecorationUnderline, InjectedText,
    OverviewRulerLane,
};
pub use editor::{CodeEditor, CodeEditorEvent, ViewState};
pub use editor_actions::{
    ActionPrecondition, ActionRun, EditorActionDescriptor, EditorActionInfo, KeybindingRule,
    add_editor_action, add_keybinding_rule, add_keybinding_rules,
};
pub use element::EditorElement;
pub use events::{ContentChange, CursorChangeReason, CursorChangeSource, SelectionChange};
pub use find::{FindOptions, MATCHES_LIMIT};
pub use find_widget::FindWidget;
pub use goto::has_symbol_navigation;
pub use hover::{HOVER_DELAY, HoverWidget};
pub use languages::{on_language, set_language_configuration};
pub use markers::{
    Marker, MarkerFilter, MarkerSeverity, MarkerTag, MarkerZone, ModelMarker, RelatedInformation,
    model_markers, remove_all_markers,
};
pub use model::{
    EditKind, ModelOptions, SyntaxSnapshot, TextModel, TextModelEvent, language_registry,
    model_for_uri, register_model, set_language_registry, set_model_resolver,
};
pub use mouse_target::{EditorMouseEvent, MouseTarget, MouseTargetKind};
pub use options::EditorOptions;
pub use registry::{
    Disposable, diff_editors, editors, models, on_did_change_markers, on_did_change_model_language,
    on_did_create_diff_editor, on_did_create_editor, on_did_create_model, on_will_dispose_model,
};
pub use signature_help::SignatureHelpWidget;
pub use snippet_session::{SnippetInsertOptions, set_snippet_workspace};
pub use suggest_widget::SuggestWidget;
pub use syntax;
pub use text;
pub use theme::{
    EditorColors, Theme, ThemeKind, TokenStyle, define_theme, global_theme, set_theme, theme_named,
};
pub use view::reveal::RevealType;
pub use widgets::{
    ContentWidget, ContentWidgetPreference, GlyphMarginWidget, OverlayPosition, OverlayWidget,
    ViewZone, WidgetId, ZoneHeight,
};

/// Binds Monaco's default keybindings in the editor's key context, and
/// starts tracking models for [`models`] and [`on_did_create_model`].
pub fn init(cx: &mut gpui::App) {
    registry::ensure(cx);
    actions::bind_default_keys(cx);
    diff_editor::bind_keys(cx);
}
