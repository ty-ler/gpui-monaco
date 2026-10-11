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
//!
//! # Cargo features
//!
//! - `word-segmenter` (default): ICU4X's word segmenter and its compiled
//!   data, for Monaco's `wordSegmenterLocales`
//!   ([`EditorOptions::word_segmenter_locales`]). Without it the option has
//!   no effect.
//! - `codicons` (off): Microsoft's codicon icon font from the
//!   `gpui-monaco-codicons` crate, licensed CC-BY-4.0 (apps that enable it
//!   must ship its attribution), so the widgets draw Monaco's icons. Without
//!   it they draw Unicode stand-ins; see [`codicons`].
#![warn(missing_docs)]

mod accessibility;
pub mod actions;
mod api;
pub mod api_commands;
mod bracket_colors;
mod bracket_matching;
mod bulk_edit;
pub mod code_action;
mod code_lens;
pub mod codicons;
pub mod color;
mod color_picker;
pub use color_picker::{ColorPickerWidget, STANDALONE_COLOR_PICKER_KEY_CONTEXT};
mod color_detector;
pub mod colorize;
mod commands;
pub mod context_keys;
mod context_menu;
pub use context_menu::CONTEXT_MENU_KEY_CONTEXT;
mod completion;
pub mod cursor;
pub mod decorations;
mod definition;
mod developer;
pub use developer::TokenInspection;
pub mod diff;
pub mod diff_editor;
pub mod multi_diff_editor;
pub use diff_editor::{
    AccessibilitySignal, CharChange, DiffAlgorithm, DiffEditor, DiffEditorEvent, DiffEditorModel,
    DiffEditorOptions, DiffEditorViewState, DiffHunkAction, DiffHunkContext, DiffHunkHandler,
    DiffHunkToolbar, HideUnchangedRegions, LineChange,
};
pub use multi_diff_editor::{
    MultiDiffDocument, MultiDiffEditor, MultiDiffEditorEvent, MultiDiffFileAction,
    MultiDiffFileHandler,
};
mod dnd;
mod drop_paste;
mod editor;
mod editor_actions;
mod editor_commands;
mod editor_state;
mod element;
pub mod events;
mod find;
mod find_widget;
mod folding_commands;
mod font_zoom;
mod indentation_commands;
mod tooltip;
pub use font_zoom::{editor_zoom_level, remeasure_fonts, set_editor_zoom_level};
mod format;
mod geometry;
mod goto;
mod high_contrast;
pub use high_contrast::{
    auto_detect_high_contrast, os_high_contrast, set_auto_detect_high_contrast,
    set_os_high_contrast,
};
mod hover;
mod inlay_hints;
mod inline_completions;
mod languages;
pub mod layout_info;
pub mod link_computer;
mod linked_editing;
mod links;
mod long_lines_helper;
pub mod markdown;
mod markers;
mod message;
mod minimap;
mod model;
pub mod mouse_target;
mod multi_cursor;
mod navigation_history;
mod occurrences;
mod openers;
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
mod suggest_inline_completions;
mod suggest_widget;
pub mod theme;
pub mod unicode_highlight;
mod unicode_highlighter;
mod unusual_line_terminators;
pub mod view;
pub mod widgets;

pub use api::{
    BracketPairInfo, CursorStateComputer, DecorationsCollection, EndCursorState,
    EndOfLinePreference, FindMatch, ModelDecoration, SingleEditOperation, WordAtPosition,
};
pub use bulk_edit::{
    has_workspace_edit_preview_handler, set_workspace_edit_preview_handler,
    set_workspace_file_edit_handler,
};
pub use commands::{add_command, execute_command, register_command, unregister_command};
pub use context_keys::{ContextKey, ContextKeyValue};
pub use decorations::{
    BlockDecoration, DecorationId, DecorationOptions, DecorationUnderline, GlyphMarginLane,
    GlyphMarginOptions, InjectedText, InjectedTextBox, LeftBorder, MinimapDecoration,
    MinimapPosition, MinimapSectionHeaderStyle, OverviewRulerLane,
};
#[doc(hidden)]
pub use editor::OverviewRulerForTests;
pub use editor::{CodeEditor, CodeEditorEvent, FoldingRegionState, FoldingViewState, ViewState};
pub use editor_actions::{
    ActionPrecondition, ActionRun, EditorActionDescriptor, EditorActionInfo, KeybindingRule,
    add_editor_action, add_keybinding_rule, add_keybinding_rules,
};
pub use editor_commands::{
    CursorStateComputerData, EditOperationBuilder, EditorCommand, TrackedSelection,
};
pub use element::EditorElement;
pub use events::{
    ClipboardCopyEvent, ClipboardPasteEvent, ConfigurationChange, ContentChange, ContentSizeChange,
    CursorChangeReason, CursorChangeSource, CursorPositionChange, DecorationsChange, ModelChange,
    ModelContentChange, ModelOptionsChange, MouseWheelEvent, PasteEvent, ScrollEvent,
    SelectionChange,
};
pub use find::{FindOptions, MATCHES_LIMIT};
pub use find_widget::FindWidget;
pub use goto::has_symbol_navigation;
pub use hover::{HOVER_DELAY, HoverWidget};
pub use inline_completions::inline_edits::InlineEditViewKind;
pub use languages::{
    LanguageTokenizer, on_language, on_language_encountered, register_tokens_provider_factory,
    set_language_configuration, set_monarch_tokens_provider, set_tokens_provider,
};
pub use layout_info::{LayoutInfo, MinimapLayoutInfo, OverviewRulerPosition, RenderMinimap};
pub use markers::{
    Marker, MarkerFilter, MarkerSeverity, MarkerTag, MarkerZone, ModelMarker, RelatedInformation,
    model_markers, remove_all_markers,
};
pub use model::{
    EditKind, ModelOptions, SyntaxSnapshot, TextModel, TextModelEvent, language_registry,
    model_for_uri, register_model, set_language_registry, set_model_resolver,
};
pub use mouse_target::{EditorMouseEvent, MouseTarget, MouseTargetDetail, MouseTargetKind};
pub use openers::{register_editor_opener, register_link_opener};
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
    EditorColors, InlineEditColors, SymbolIconColors, Theme, ThemeKind, TokenFontStyle, TokenStyle,
    define_theme, global_theme, set_theme, theme_named,
};
pub use view::reveal::{RevealType, ScrollType};
pub use widgets::{
    ContentWidget, ContentWidgetPosition, ContentWidgetPreference, GlyphMarginWidget,
    OverlayPosition, OverlayWidget, PositionAffinity, ViewZone, WidgetId, ZoneHeight,
};

/// Registers the codicon font with the `codicons` feature ([`codicons`]),
/// binds Monaco's default keybindings in the editor's key context, starts
/// tracking models for [`models`] and [`on_did_create_model`], and registers
/// Monaco's `editor.action.goToLocations` and `editor.action.peekLocations`
/// API commands for [`execute_command`].
pub fn init(cx: &mut gpui::App) {
    codicons::register_codicon_font(cx);
    registry::ensure(cx);
    goto::register_commands(cx);
    actions::bind_default_keys(cx);
    diff_editor::bind_keys(cx);
}
