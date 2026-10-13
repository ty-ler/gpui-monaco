//! Themes: editor chrome colors plus token colors.
//!
//! Chrome colors are VS Code color ids (`editor.background`), resolved as
//! Monaco's `StandaloneTheme.getColor` does: a theme's own colors first,
//! then the color registry's defaults for its kind (dark, light, high
//! contrast dark or light), which may derive from other colors.
//! [`EditorColors`] holds the resolved values the editor draws with.
//!
//! Two kinds of token rules are supported and can be mixed:
//! - **Monaco rules** keyed by Monarch token names (`keyword`, `string.key.json`),
//!   as in Monaco's built-in themes and `monaco.editor.defineTheme`. The
//!   built-in [`Theme::vs`], [`Theme::vs_dark`], [`Theme::hc_black`] and
//!   [`Theme::hc_light`] are ported from Monaco's
//!   `standalone/common/themes.ts` (MIT), language-specific rules included.
//! - **TextMate rules** from VS Code theme JSON `tokenColors`, keyed by scope
//!   selectors (`entity.name.function`, `keyword.control`).
//!
//! Each vocabulary resolves like Monaco's `TokenTheme`: a rule inherits the
//! fields it leaves unset from its parent (`comment.doc` with only a font
//! style keeps `comment`'s color), and `''` rules set the default.
//!
//! Tree-sitter capture names (e.g. `function.method`) are mapped onto both
//! vocabularies by [`monaco_token_for_capture`] and [`textmate_scopes_for_capture`].
//! As in Monaco, a capture's Monaco token carries its language as a suffix
//! (`keyword.json`), so language-specific rules apply.
//! Monarch tokens (`keyword.rust`, from
//! [`set_monarch_tokens_provider`](crate::set_monarch_tokens_provider)) are
//! Monaco token names already: Monaco rules match them as Monaco's
//! `TokenTheme` does, then TextMate rules as if they were scopes, then the
//! TextMate scopes of the capture they'd be.

mod color_registry;
mod token_theme;

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use gpui::{Hsla, rgb, rgba};
use serde::Deserialize;
use syntax::HighlightId;

pub(crate) use color_registry::ColorScheme;
use color_registry::Resolver;
use token_theme::TokenTrie;

/// Declares [`EditorColors`]: each field with its doc comment and how it
/// resolves from a theme's colors (`$r` being the [`Resolver`]).
macro_rules! editor_colors {
    ($r:ident; $( $(#[$doc:meta])* $field:ident: $ty:ty = $value:expr; )*) => {
        /// Editor chrome colors, resolved from VS Code color ids (named in
        /// each field's docs) and their Monaco defaults.
        ///
        /// Defaults to [`EditorColors::vs`], Monaco's default theme.
        #[derive(Clone, Debug, PartialEq)]
        pub struct EditorColors {
            $( $(#[$doc])* pub $field: $ty, )*
        }

        impl EditorColors {
            fn from_resolver($r: &mut Resolver) -> Self {
                Self { $( $field: $value, )* }
            }

            /// Takes the fields that differ between `before` and `after`.
            fn merge_changed(&mut self, before: &Self, after: &Self) {
                $( if before.$field != after.$field {
                    self.$field = after.$field.clone();
                } )*
            }
        }
    };
}

/// Monaco's `currentLineHighlight` theming: the border shows when the
/// background is unset or transparent, or when the theme sets it.
fn line_highlight_border(r: &mut Resolver) -> Option<Hsla> {
    let background = r.get("editor.lineHighlightBackground");
    if background.is_none_or(|color| color.a == 0.) || r.defines("editor.lineHighlightBorder") {
        r.visible("editor.lineHighlightBorder")
    } else {
        None
    }
}

/// The six numbered `{prefix}1` … `{prefix}6` colors, `None` when
/// transparent.
fn levels(r: &mut Resolver, prefix: &str) -> [Option<Hsla>; 6] {
    std::array::from_fn(|i| r.visible(&format!("{prefix}{}", i + 1)))
}

editor_colors! { r;
    /// Text area background (`editor.background`).
    background: Hsla = r.color("editor.background");
    /// Default text color (`editor.foreground`).
    foreground: Hsla = r.color("editor.foreground");
    /// Line numbers (`editorLineNumber.foreground`).
    line_number: Hsla = r.color("editorLineNumber.foreground");
    /// The cursor's line number (`editorLineNumber.activeForeground`,
    /// defaulting to the deprecated `editorActiveLineNumber.foreground`).
    active_line_number: Hsla = r.color("editorLineNumber.activeForeground");
    /// The final line's number when `renderFinalNewline` is `dimmed`
    /// (`editorLineNumber.dimmedForeground`; `None` uses the line number
    /// color).
    line_number_dimmed: Option<Hsla> = r.get("editorLineNumber.dimmedForeground");
    /// Current line fill (`editor.lineHighlightBackground`); `None` draws no fill.
    line_highlight_background: Option<Hsla> = r.get("editor.lineHighlightBackground");
    /// Current line fill when the editor isn't focused
    /// (`editor.inactiveLineHighlightBackground`, defaulting to the focused
    /// one).
    inactive_line_highlight_background: Option<Hsla> =
        r.get("editor.inactiveLineHighlightBackground");
    /// Current line outline (`editor.lineHighlightBorder`); `None` draws no
    /// outline. As in Monaco, it's dropped when the theme sets a visible
    /// `editor.lineHighlightBackground` but not the border.
    line_highlight_border: Option<Hsla> = line_highlight_border(r);
    /// Selections in the focused editor (`editor.selectionBackground`).
    selection_background: Hsla = r.color("editor.selectionBackground");
    /// Selected text's color (`editor.selectionForeground`; none keeps the
    /// token colors). High-contrast themes set it.
    selection_foreground: Option<Hsla> = r.get("editor.selectionForeground");
    /// Inserted text in the diff editor (`diffEditor.insertedTextBackground`).
    diff_inserted_text_background: Hsla = r.color("diffEditor.insertedTextBackground");
    /// Removed text (`diffEditor.removedTextBackground`).
    diff_removed_text_background: Hsla = r.color("diffEditor.removedTextBackground");
    /// Inserted text's outline (`diffEditor.insertedTextBorder`, high
    /// contrast).
    diff_inserted_text_border: Option<Hsla> = r.get("diffEditor.insertedTextBorder");
    /// Removed text's outline (`diffEditor.removedTextBorder`).
    diff_removed_text_border: Option<Hsla> = r.get("diffEditor.removedTextBorder");
    /// Inserted lines (`diffEditor.insertedLineBackground`).
    diff_inserted_line_background: Hsla = r.color("diffEditor.insertedLineBackground");
    /// Removed lines (`diffEditor.removedLineBackground`).
    diff_removed_line_background: Hsla = r.color("diffEditor.removedLineBackground");
    /// The gutter beside inserted lines
    /// (`diffEditorGutter.insertedLineBackground`, defaulting to the lines').
    diff_gutter_inserted_line_background: Hsla = r.color_first(&[
        "diffEditorGutter.insertedLineBackground",
        "diffEditor.insertedLineBackground",
    ]);
    /// The gutter beside removed lines
    /// (`diffEditorGutter.removedLineBackground`, defaulting to the lines').
    diff_gutter_removed_line_background: Hsla = r.color_first(&[
        "diffEditorGutter.removedLineBackground",
        "diffEditor.removedLineBackground",
    ]);
    /// Inserted lines in the diff overview ruler
    /// (`diffEditorOverview.insertedForeground`; `None` uses the line
    /// background).
    diff_overview_inserted: Option<Hsla> = r.get("diffEditorOverview.insertedForeground");
    /// Removed lines in the diff overview ruler
    /// (`diffEditorOverview.removedForeground`).
    diff_overview_removed: Option<Hsla> = r.get("diffEditorOverview.removedForeground");
    /// The border between the two sides (`diffEditor.border`, high contrast).
    diff_border: Option<Hsla> = r.visible("diffEditor.border");
    /// The stripes where the other side has lines (`diffEditor.diagonalFill`).
    diff_diagonal_fill: Hsla = r.color("diffEditor.diagonalFill");
    /// The "hidden lines" bars (`diffEditor.unchangedRegionBackground`, else
    /// `sideBySideEditor.background`, else the editor background).
    diff_unchanged_region_background: Hsla = r.color_first(&[
        "diffEditor.unchangedRegionBackground",
        "sideBySideEditor.background",
        "editor.background",
    ]);
    /// Their text (`diffEditor.unchangedRegionForeground`).
    diff_unchanged_region_foreground: Hsla = r.color("diffEditor.unchangedRegionForeground");
    /// Their shadow (`diffEditor.unchangedRegionShadow`).
    diff_unchanged_region_shadow: Hsla = r.color("diffEditor.unchangedRegionShadow");
    /// Unchanged lines that can be hidden
    /// (`diffEditor.unchangedCodeBackground`).
    diff_unchanged_code_background: Hsla = r.color("diffEditor.unchangedCodeBackground");
    /// Gutter toolbars, like the diff editor's revert buttons
    /// (`editorGutter.itemBackground`).
    gutter_item_background: Hsla = r.color("editorGutter.itemBackground");
    /// Their icons (`editorGutter.itemGlyphForeground`).
    gutter_item_glyph_foreground: Hsla = r.color("editorGutter.itemGlyphForeground");
    /// Moved blocks' borders and arrows (`diffEditor.move.border`).
    diff_move_border: Hsla = r.color("diffEditor.move.border");
    /// The active moved block's (`diffEditor.moveActive.border`).
    diff_move_active_border: Hsla = r.color("diffEditor.moveActive.border");
    /// A multi-diff editor's file headers
    /// (`multiDiffEditor.headerBackground`, else the editor background).
    multi_diff_header_background: Hsla =
        r.color_first(&["multiDiffEditor.headerBackground", "editor.background"]);
    /// The border above them and below each file's diff
    /// (`multiDiffEditor.border`).
    multi_diff_border: Hsla = r.color("multiDiffEditor.border");
    /// A multi-diff editor's background, around the files
    /// (`multiDiffEditor.background`, defaulting to the editor background).
    multi_diff_background: Hsla = r.color("multiDiffEditor.background");
    /// The file headers' text (the workbench `foreground`).
    multi_diff_header_foreground: Hsla = r.color("foreground");
    /// The border under an expanded file's header
    /// (`sideBarSectionHeader.border`, high contrast themes only by
    /// default).
    multi_diff_header_border: Option<Hsla> = r.visible("sideBarSectionHeader.border");
    /// The line between files of the compact multi-diff variants
    /// (`panel.border`).
    multi_diff_entry_border: Option<Hsla> = r.visible("panel.border");
    /// The card variant's headers (`sideBarSectionHeader.background`).
    multi_diff_card_header_background: Option<Hsla> =
        r.visible("sideBarSectionHeader.background");
    /// The card variant's card borders (`widget.border`, else
    /// `editorWidget.border`).
    multi_diff_card_border: Option<Hsla> = r
        .first(&["widget.border", "editorWidget.border"])
        .filter(|color| color.a > 0.);
    /// The `A` of a created file in the compact headers
    /// (`gitDecoration.addedResourceForeground`).
    multi_diff_added_foreground: Hsla = r.color("gitDecoration.addedResourceForeground");
    /// The `D` of a deleted file (`gitDecoration.deletedResourceForeground`).
    multi_diff_deleted_foreground: Hsla = r.color("gitDecoration.deletedResourceForeground");
    /// The `R` of a renamed file (`gitDecoration.modifiedResourceForeground`).
    multi_diff_renamed_foreground: Hsla = r.color("gitDecoration.modifiedResourceForeground");
    /// Selections in an unfocused editor (`editor.inactiveSelectionBackground`).
    inactive_selection_background: Hsla = r.color("editor.inactiveSelectionBackground");
    /// The cursor (`editorCursor.foreground`).
    cursor: Hsla = r.color("editorCursor.foreground");
    /// The character under a block cursor (`editorCursor.background`; `None`
    /// keeps its color).
    cursor_background: Option<Hsla> = r.get("editorCursor.background");
    /// The primary cursor when there are several
    /// (`editorMultiCursor.primary.foreground`).
    multi_cursor_primary_foreground: Hsla = r.color("editorMultiCursor.primary.foreground");
    /// `editorMultiCursor.primary.background`.
    multi_cursor_primary_background: Option<Hsla> = r.get("editorMultiCursor.primary.background");
    /// The other cursors (`editorMultiCursor.secondary.foreground`).
    multi_cursor_secondary_foreground: Hsla = r.color("editorMultiCursor.secondary.foreground");
    /// `editorMultiCursor.secondary.background`.
    multi_cursor_secondary_background: Option<Hsla> =
        r.get("editorMultiCursor.secondary.background");
    /// Gutter background (`editorGutter.background`, defaulting to the
    /// editor background).
    gutter_background: Hsla = r.color("editorGutter.background");
    /// The folding chevrons (`editorGutter.foldingControlForeground`).
    folding_control_foreground: Hsla = r.color("editorGutter.foldingControlForeground");
    /// Scrollbar thumb (`scrollbarSlider.background`).
    scrollbar_slider: Hsla = r.color("scrollbarSlider.background");
    /// Hovered scrollbar thumb (`scrollbarSlider.hoverBackground`).
    scrollbar_slider_hover: Hsla = r.color("scrollbarSlider.hoverBackground");
    /// Dragged scrollbar thumb (`scrollbarSlider.activeBackground`).
    scrollbar_slider_active: Hsla = r.color("scrollbarSlider.activeBackground");
    /// Shadow drawn when the content is scrolled (`scrollbar.shadow`).
    scrollbar_shadow: Hsla = r.color("scrollbar.shadow");
    /// The overview ruler's left border (`editorOverviewRuler.border`).
    overview_ruler_border: Hsla = r.color("editorOverviewRuler.border");
    /// The overview ruler's background (`editorOverviewRuler.background`;
    /// `None` draws none).
    overview_ruler_background: Option<Hsla> = r.visible("editorOverviewRuler.background");
    /// Underline of IME composition text (`editor.compositionBorder`).
    ime_underline: Hsla = r.color("editor.compositionBorder");
    /// Current find match (`editor.findMatchBackground`).
    find_match_background: Hsla = r.color("editor.findMatchBackground");
    /// `editor.findMatchForeground`: Monaco's theming gives it to the other
    /// matches' text (`.findMatchInline`) despite its description; `None`
    /// keeps the token colors.
    find_match_foreground: Option<Hsla> = r.get("editor.findMatchForeground");
    /// Its outline (`editor.findMatchBorder`, high contrast).
    find_match_border: Option<Hsla> = r.visible("editor.findMatchBorder");
    /// Other find matches (`editor.findMatchHighlightBackground`).
    find_match_highlight_background: Hsla = r.color("editor.findMatchHighlightBackground");
    /// `editor.findMatchHighlightForeground`: Monaco's theming gives it to
    /// the current match's text (`.currentFindMatchInline`).
    find_match_highlight_foreground: Option<Hsla> = r.get("editor.findMatchHighlightForeground");
    /// Their outline (`editor.findMatchHighlightBorder`).
    find_match_highlight_border: Option<Hsla> = r.visible("editor.findMatchHighlightBorder");
    /// Find matches in the overview ruler (`editorOverviewRuler.findMatchForeground`).
    find_match_overview: Hsla = r.color("editorOverviewRuler.findMatchForeground");
    /// The find in selection scope (`editor.findRangeHighlightBackground`).
    find_range_highlight_background: Hsla = r.color("editor.findRangeHighlightBackground");
    /// Its outline (`editor.findRangeHighlightBorder`).
    find_range_highlight_border: Option<Hsla> = r.visible("editor.findRangeHighlightBorder");
    /// Error text, e.g. the find widget's "No results" (`errorForeground`).
    error_foreground: Hsla = r.color("errorForeground");
    /// The target of go to definition and similar jumps, briefly
    /// (`editor.symbolHighlightBackground`; defaults to the find match
    /// highlight).
    symbol_highlight_background: Hsla = r.color("editor.symbolHighlightBackground");
    /// Its outline (`editor.symbolHighlightBorder`).
    symbol_highlight_border: Option<Hsla> = r.visible("editor.symbolHighlightBorder");
    /// A link while Cmd/Ctrl is held over it (`editorLink.activeForeground`).
    link_active_foreground: Hsla = r.color("editorLink.activeForeground");
    /// Quick access previews (`editor.rangeHighlightBackground`).
    range_highlight_background: Hsla = r.color("editor.rangeHighlightBackground");
    /// Their outline (`editor.rangeHighlightBorder`).
    range_highlight_border: Option<Hsla> = r.visible("editor.rangeHighlightBorder");
    /// Range highlights in the overview ruler
    /// (`editorOverviewRuler.rangeHighlightForeground`).
    range_highlight_overview: Hsla = r.color("editorOverviewRuler.rangeHighlightForeground");
    /// The border of color decorator swatches (Monaco's
    /// `.colorpicker-color-decoration` CSS, not a theme color).
    color_decoration_border: Hsla = if r.scheme().is_light() {
        c(rgb(0x000000))
    } else {
        c(rgb(0xeeeeee))
    };
    /// Menus, like the context menu (`menu.background`).
    menu_background: Hsla = r.color("menu.background");
    /// Menu items (`menu.foreground`).
    menu_foreground: Hsla = r.color("menu.foreground");
    /// The menu's border (`menu.border`, high contrast).
    menu_border: Option<Hsla> = r.visible("menu.border");
    /// The highlighted menu row (`menu.selectionBackground`).
    menu_selection_background: Hsla = r.color("menu.selectionBackground");
    /// The highlighted menu row's text (`menu.selectionForeground`, else the
    /// menu's).
    menu_selection_foreground: Hsla =
        r.color_first(&["menu.selectionForeground", "menu.foreground"]);
    /// The highlighted menu row's outline (`menu.selectionBorder`, high
    /// contrast).
    menu_selection_border: Option<Hsla> = r.visible("menu.selectionBorder");
    /// Separators in menus and toolbars (`menu.separatorBackground`).
    menu_separator_background: Hsla = r.color("menu.separatorBackground");
    /// Button background (`button.background`).
    button_background: Hsla = r.color("button.background");
    /// Button text (`button.foreground`).
    button_foreground: Hsla = r.color("button.foreground");
    /// A hovered button (`button.hoverBackground`).
    button_hover_background: Hsla = r.color("button.hoverBackground");
    /// Button outline (`button.border`, high contrast).
    button_border: Option<Hsla> = r.visible("button.border");
    /// The separator in split buttons (`button.separator`).
    button_separator: Hsla = r.color("button.separator");
    /// Secondary buttons (`button.secondaryBackground`).
    button_secondary_background: Option<Hsla> = r.get("button.secondaryBackground");
    /// Their text (`button.secondaryForeground`).
    button_secondary_foreground: Hsla = r.color("button.secondaryForeground");
    /// A hovered secondary button (`button.secondaryHoverBackground`).
    button_secondary_hover_background: Option<Hsla> = r.get("button.secondaryHoverBackground");
    /// Secondary button outline (`button.secondaryBorder`).
    button_secondary_border: Option<Hsla> = r.visible("button.secondaryBorder");
    /// Code lens text (`editorCodeLens.foreground`).
    code_lens_foreground: Hsla = r.color("editorCodeLens.foreground");
    /// Linked editing ranges (`editor.linkedEditingBackground`).
    linked_editing_background: Hsla = r.color("editor.linkedEditingBackground");
    /// Ghost text (`editorGhostText.foreground`; high contrast themes keep
    /// the text color and add a border).
    ghost_text_foreground: Hsla = r.color_first(&["editorGhostText.foreground", "editor.foreground"]);
    /// `editorGhostText.background` (transparent: none).
    ghost_text_background: Hsla = r.color("editorGhostText.background");
    /// `editorGhostText.border` (transparent: none).
    ghost_text_border: Hsla = r.color("editorGhostText.border");
    /// The `placeholder` text (`editor.placeholder.foreground`; Monaco
    /// defaults it to the ghost text color).
    placeholder_foreground: Hsla =
        r.color_first(&["editor.placeholder.foreground", "editor.foreground"]);
    /// Highlighted Unicode characters' border (`editorUnicodeHighlight.border`,
    /// defaulting to `editorWarning.foreground`).
    unicode_highlight_border: Hsla = r.color("editorUnicodeHighlight.border");
    /// Their background (`editorUnicodeHighlight.background`; none by
    /// default).
    unicode_highlight_background: Hsla = r.color("editorUnicodeHighlight.background");
    /// Inline messages such as "No definition found"
    /// (`inputValidation.infoBackground`).
    message_background: Hsla = r.color("inputValidation.infoBackground");
    /// Their text (`inputValidation.infoForeground`; `None` keeps the
    /// widget's).
    message_foreground: Option<Hsla> = r.get("inputValidation.infoForeground");
    /// Their border (`inputValidation.infoBorder`).
    message_border: Hsla = r.color("inputValidation.infoBorder");
    /// Input warnings (`inputValidation.warningBackground`).
    input_validation_warning_background: Hsla = r.color("inputValidation.warningBackground");
    /// `inputValidation.warningForeground`.
    input_validation_warning_foreground: Option<Hsla> = r.get("inputValidation.warningForeground");
    /// `inputValidation.warningBorder`.
    input_validation_warning_border: Hsla = r.color("inputValidation.warningBorder");
    /// Input errors, like an invalid find regex
    /// (`inputValidation.errorBackground`).
    input_validation_error_background: Hsla = r.color("inputValidation.errorBackground");
    /// `inputValidation.errorForeground`.
    input_validation_error_foreground: Option<Hsla> = r.get("inputValidation.errorForeground");
    /// `inputValidation.errorBorder`.
    input_validation_error_border: Hsla = r.color("inputValidation.errorBorder");
    /// Collapsed regions' first lines (`editor.foldBackground`).
    fold_background: Hsla = r.color("editor.foldBackground");
    /// The `…` after a collapsed region (`editor.foldPlaceholderForeground`;
    /// `None` keeps the text color).
    fold_placeholder_foreground: Option<Hsla> = r.get("editor.foldPlaceholderForeground");
    /// Inlay hint text (`editorInlayHint.foreground`).
    inlay_hint_foreground: Hsla = r.color("editorInlayHint.foreground");
    /// Inlay hint background (`editorInlayHint.background`).
    inlay_hint_background: Hsla = r.color("editorInlayHint.background");
    /// Type hints (`editorInlayHint.typeForeground`).
    inlay_hint_type_foreground: Hsla = r.color("editorInlayHint.typeForeground");
    /// `editorInlayHint.typeBackground`.
    inlay_hint_type_background: Hsla = r.color("editorInlayHint.typeBackground");
    /// Parameter hints (`editorInlayHint.parameterForeground`).
    inlay_hint_parameter_foreground: Hsla = r.color("editorInlayHint.parameterForeground");
    /// `editorInlayHint.parameterBackground`.
    inlay_hint_parameter_background: Hsla = r.color("editorInlayHint.parameterBackground");
    /// The code action lightbulb (`editorLightBulb.foreground`).
    lightbulb_foreground: Hsla = r.color("editorLightBulb.foreground");
    /// The lightbulb when a preferred quick fix exists
    /// (`editorLightBulbAutoFix.foreground`).
    lightbulb_auto_fix_foreground: Hsla = r.color("editorLightBulbAutoFix.foreground");
    /// The lightbulb for AI fixes (`editorLightBulbAi.foreground`).
    lightbulb_ai_foreground: Hsla = r.color("editorLightBulbAi.foreground");
    /// The peek view's frame and arrow (`peekView.border`, defaulting to
    /// `editorInfo.foreground`).
    peek_view_border: Hsla = r.color("peekView.border");
    /// Its title bar (`peekViewTitle.background`).
    peek_view_title_background: Hsla = r.color("peekViewTitle.background");
    /// The title (`peekViewTitleLabel.foreground`).
    peek_view_title_foreground: Hsla = r.color("peekViewTitleLabel.foreground");
    /// The title's description (the folder) (`peekViewTitleDescription.foreground`).
    peek_view_title_description_foreground: Hsla = r.color("peekViewTitleDescription.foreground");
    /// The results list (`peekViewResult.background`).
    peek_view_result_background: Hsla = r.color("peekViewResult.background");
    /// Result lines (`peekViewResult.lineForeground`).
    peek_view_result_line_foreground: Hsla = r.color("peekViewResult.lineForeground");
    /// Result files (`peekViewResult.fileForeground`).
    peek_view_result_file_foreground: Hsla = r.color("peekViewResult.fileForeground");
    /// The selected result (`peekViewResult.selectionBackground`).
    peek_view_result_selection_background: Hsla = r.color("peekViewResult.selectionBackground");
    /// The selected result's text (`peekViewResult.selectionForeground`).
    peek_view_result_selection_foreground: Hsla = r.color("peekViewResult.selectionForeground");
    /// The match in result lines (`peekViewResult.matchHighlightBackground`).
    peek_view_result_match_highlight_background: Hsla =
        r.color("peekViewResult.matchHighlightBackground");
    /// The preview editor (`peekViewEditor.background`).
    peek_view_editor_background: Hsla = r.color("peekViewEditor.background");
    /// The preview editor's gutter (`peekViewEditorGutter.background`).
    peek_view_editor_gutter_background: Hsla = r.color("peekViewEditorGutter.background");
    /// The preview editor's sticky scroll
    /// (`peekViewEditorStickyScroll.background`).
    peek_view_editor_sticky_scroll_background: Hsla =
        r.color("peekViewEditorStickyScroll.background");
    /// Its gutter (`peekViewEditorStickyScrollGutter.background`).
    peek_view_editor_sticky_scroll_gutter_background: Hsla =
        r.color("peekViewEditorStickyScrollGutter.background");
    /// Matches in the preview editor (`peekViewEditor.matchHighlightBackground`).
    peek_view_editor_match_highlight_background: Hsla =
        r.color("peekViewEditor.matchHighlightBackground");
    /// Their outline (`peekViewEditor.matchHighlightBorder`).
    peek_view_editor_match_highlight_border: Option<Hsla> =
        r.visible("peekViewEditor.matchHighlightBorder");
    /// Occurrences of the symbol at the cursor (`editor.wordHighlightBackground`).
    word_highlight_background: Hsla = r.color("editor.wordHighlightBackground");
    /// Write occurrences of the symbol at the cursor
    /// (`editor.wordHighlightStrongBackground`).
    word_highlight_strong_background: Hsla = r.color("editor.wordHighlightStrongBackground");
    /// Textual occurrences (`editor.wordHighlightTextBackground`).
    word_highlight_text_background: Hsla = r.color("editor.wordHighlightTextBackground");
    /// Occurrences' outline (`editor.wordHighlightBorder`).
    word_highlight_border: Option<Hsla> = r.visible("editor.wordHighlightBorder");
    /// Write occurrences' outline (`editor.wordHighlightStrongBorder`).
    word_highlight_strong_border: Option<Hsla> = r.visible("editor.wordHighlightStrongBorder");
    /// Textual occurrences' outline (`editor.wordHighlightTextBorder`).
    word_highlight_text_border: Option<Hsla> = r.visible("editor.wordHighlightTextBorder");
    /// Occurrences in the overview ruler
    /// (`editorOverviewRuler.wordHighlightForeground`).
    word_highlight_overview: Hsla = r.color("editorOverviewRuler.wordHighlightForeground");
    /// Write occurrences in the overview ruler
    /// (`editorOverviewRuler.wordHighlightStrongForeground`).
    word_highlight_strong_overview: Hsla =
        r.color("editorOverviewRuler.wordHighlightStrongForeground");
    /// Textual occurrences in the overview ruler
    /// (`editorOverviewRuler.wordHighlightTextForeground`).
    word_highlight_text_overview: Hsla = r.color("editorOverviewRuler.wordHighlightTextForeground");
    /// Bracket pair colors by nesting level, cycling
    /// (`editorBracketHighlight.foreground1`–`6`; `None` for transparent).
    bracket_highlights: [Option<Hsla>; 6] = levels(r, "editorBracketHighlight.foreground");
    /// Unmatched brackets (`editorBracketHighlight.unexpectedBracket.foreground`).
    unexpected_bracket: Hsla = r.color("editorBracketHighlight.unexpectedBracket.foreground");
    /// Bracket pair guides by nesting level
    /// (`editorBracketPairGuide.background1`–`6`; `None` for transparent,
    /// which uses the bracket color at 30%).
    bracket_pair_guides: [Option<Hsla>; 6] = levels(r, "editorBracketPairGuide.background");
    /// Active bracket pair guides (`editorBracketPairGuide.activeBackground1`–`6`;
    /// `None` uses the bracket color).
    bracket_pair_guides_active: [Option<Hsla>; 6] =
        levels(r, "editorBracketPairGuide.activeBackground");
    /// Rendered whitespace (`editorWhitespace.foreground`).
    whitespace: Hsla = r.color("editorWhitespace.foreground");
    /// The `wordWrapIndicator` glyph (`editorWordWrapIndicator.foreground`,
    /// defaulting to the whitespace color).
    word_wrap_indicator: Hsla = r.color("editorWordWrapIndicator.foreground");
    /// Indent guides by indentation level, cycling over the visible ones
    /// (`editorIndentGuide.background1`–`6`, the first defaulting to the
    /// deprecated `editorIndentGuide.background`; `None` for transparent).
    indent_guides: [Option<Hsla>; 6] = levels(r, "editorIndentGuide.background");
    /// The active indent guide (`editorIndentGuide.activeBackground1`–`6`).
    indent_guides_active: [Option<Hsla>; 6] = levels(r, "editorIndentGuide.activeBackground");
    /// Rulers (`editorRuler.foreground`).
    ruler: Hsla = r.color("editorRuler.foreground");
    /// `editorUnnecessaryCode.opacity`: its alpha is how opaque unused code
    /// is drawn (opaque in high contrast, which underlines instead).
    unnecessary_code_opacity: Hsla = r.get("editorUnnecessaryCode.opacity").unwrap_or(c(rgb(0x000000)));
    /// `editorUnnecessaryCode.border` (transparent: none): an underline for
    /// unused code instead of fading.
    unnecessary_code_border: Hsla = r.color("editorUnnecessaryCode.border");
    /// Other occurrences of the selected text (`editor.selectionHighlightBackground`).
    selection_highlight_background: Hsla = r.color("editor.selectionHighlightBackground");
    /// Their outline (`editor.selectionHighlightBorder`).
    selection_highlight_border: Option<Hsla> = r.visible("editor.selectionHighlightBorder");
    /// Selection occurrences in the overview ruler
    /// (`editorOverviewRuler.selectionHighlightForeground`).
    selection_highlight_overview: Hsla = r.color("editorOverviewRuler.selectionHighlightForeground");
    /// Widget chrome, like the find widget (`editorWidget.background`).
    widget_background: Hsla = r.color("editorWidget.background");
    /// `editorWidget.foreground`.
    widget_foreground: Hsla = r.color("editorWidget.foreground");
    /// `editorWidget.border`.
    widget_border: Hsla = r.color("editorWidget.border");
    /// Widgets' resize sash (`editorWidget.resizeBorder`).
    widget_resize_border: Option<Hsla> = r.visible("editorWidget.resizeBorder");
    /// Widgets' drop shadow (`widget.shadow`; none in high contrast).
    widget_shadow: Option<Hsla> = r.visible("widget.shadow");
    /// Widgets' outline (`widget.border`, high contrast).
    widget_outline: Option<Hsla> = r.visible("widget.border");
    /// Snippet tab stops (`editor.snippetTabstopHighlightBackground`).
    snippet_tabstop_background: Hsla = r.color("editor.snippetTabstopHighlightBackground");
    /// `editor.snippetTabstopHighlightBorder` (transparent: none).
    snippet_tabstop_border: Hsla = r.color("editor.snippetTabstopHighlightBorder");
    /// The final tab stop (`editor.snippetFinalTabstopHighlightBackground`;
    /// transparent: none).
    snippet_final_tabstop_background: Hsla = r.color("editor.snippetFinalTabstopHighlightBackground");
    /// `editor.snippetFinalTabstopHighlightBorder`.
    snippet_final_tabstop_border: Hsla = r.color("editor.snippetFinalTabstopHighlightBorder");
    /// The suggest widget (`editorSuggestWidget.background`, defaulting to
    /// the widget background).
    suggest_background: Hsla = r.color("editorSuggestWidget.background");
    /// `editorSuggestWidget.border` (defaults to the widget border).
    suggest_border: Hsla = r.color("editorSuggestWidget.border");
    /// `editorSuggestWidget.foreground` (defaults to the editor foreground).
    suggest_foreground: Hsla = r.color("editorSuggestWidget.foreground");
    /// `editorSuggestWidget.selectedForeground`: the focused row's text.
    suggest_selected_foreground: Hsla = r.color("editorSuggestWidget.selectedForeground");
    /// `editorSuggestWidget.selectedIconForeground`: the focused row's icon
    /// (`None` keeps the icon colors).
    suggest_selected_icon_foreground: Option<Hsla> = r.get("editorSuggestWidget.selectedIconForeground");
    /// `editorSuggestWidget.selectedBackground`: the focused row.
    suggest_selected_background: Hsla = r.color("editorSuggestWidget.selectedBackground");
    /// `editorSuggestWidget.focusOutline`: the focused row's outline (high
    /// contrast).
    suggest_focus_outline: Option<Hsla> = r.visible("editorSuggestWidget.focusOutline");
    /// `editorSuggestWidget.highlightForeground`: matched characters.
    suggest_highlight_foreground: Hsla = r.color("editorSuggestWidget.highlightForeground");
    /// `editorSuggestWidget.focusHighlightForeground`: matched characters on
    /// the focused row.
    suggest_focus_highlight_foreground: Hsla = r.color("editorSuggestWidget.focusHighlightForeground");
    /// The suggest widget's status bar (`editorSuggestWidgetStatus.foreground`).
    suggest_status_foreground: Hsla = r.color("editorSuggestWidgetStatus.foreground");
    /// Hovers and parameter hints (`editorHoverWidget.background`, defaulting
    /// to the widget background).
    hover_background: Hsla = r.color("editorHoverWidget.background");
    /// `editorHoverWidget.foreground`.
    hover_foreground: Hsla = r.color("editorHoverWidget.foreground");
    /// `editorHoverWidget.border`.
    hover_border: Hsla = r.color("editorHoverWidget.border");
    /// The hover's status bar, with its actions
    /// (`editorHoverWidget.statusBarBackground`).
    hover_status_bar_background: Hsla = r.color("editorHoverWidget.statusBarBackground");
    /// The active parameter in parameter hints
    /// (`editorHoverWidget.highlightForeground`).
    hover_highlight_foreground: Hsla = r.color("editorHoverWidget.highlightForeground");
    /// The hovered word's range (`editor.hoverHighlightBackground`).
    hover_highlight_background: Hsla = r.color("editor.hoverHighlightBackground");
    /// Links in markdown (`textLink.foreground`).
    text_link_foreground: Hsla = r.color("textLink.foreground");
    /// Hovered links (`textLink.activeForeground`).
    text_link_active_foreground: Hsla = r.color("textLink.activeForeground");
    /// Inline code in markdown (`textCodeBlock.background`).
    text_code_block_background: Hsla = r.color("textCodeBlock.background");
    /// Block quotes in markdown (`textBlockQuote.background`); `None` in
    /// dark high contrast.
    text_block_quote_background: Option<Hsla> = r.visible("textBlockQuote.background");
    /// Block quotes' left border (`textBlockQuote.border`).
    text_block_quote_border: Hsla = r.color("textBlockQuote.border");
    /// Separators in text (`textSeparator.foreground`): markdown tables.
    text_separator_foreground: Hsla = r.color("textSeparator.foreground");
    /// Input boxes in widgets (`input.background`).
    input_background: Hsla = r.color("input.background");
    /// Their text (`input.foreground`).
    input_foreground: Hsla = r.color("input.foreground");
    /// Their outline (`input.border`, high contrast).
    input_border: Option<Hsla> = r.visible("input.border");
    /// Their placeholder (`input.placeholderForeground`).
    input_placeholder_foreground: Hsla = r.color("input.placeholderForeground");
    /// `inputOption.activeBackground`: an enabled toggle (e.g. match case).
    input_option_active_background: Hsla = r.color("inputOption.activeBackground");
    /// `inputOption.activeBorder`; also used for links in widgets.
    input_option_active_border: Hsla = r.color("inputOption.activeBorder");
    /// An enabled toggle's icon (`inputOption.activeForeground`).
    input_option_active_foreground: Hsla = r.color("inputOption.activeForeground");
    /// A hovered toggle (`inputOption.hoverBackground`).
    input_option_hover_background: Hsla = r.color("inputOption.hoverBackground");
    /// Checkboxes (`checkbox.background`).
    checkbox_background: Hsla = r.color("checkbox.background");
    /// `checkbox.foreground`.
    checkbox_foreground: Hsla = r.color("checkbox.foreground");
    /// `checkbox.border`.
    checkbox_border: Hsla = r.color("checkbox.border");
    /// A checked checkbox (`checkbox.selectBackground`).
    checkbox_select_background: Hsla = r.color("checkbox.selectBackground");
    /// `checkbox.selectBorder`.
    checkbox_select_border: Hsla = r.color("checkbox.selectBorder");
    /// `editorBracketMatch.background`.
    bracket_match_background: Hsla = r.color("editorBracketMatch.background");
    /// `editorBracketMatch.border`.
    bracket_match_border: Hsla = r.color("editorBracketMatch.border");
    /// Matching brackets' text (`editorBracketMatch.foreground`; `None` keeps
    /// their color).
    bracket_match_foreground: Option<Hsla> = r.get("editorBracketMatch.foreground");
    /// The overview ruler mark for matching brackets
    /// (`editorOverviewRuler.bracketMatchForeground`).
    overview_ruler_bracket_match: Hsla = r.color("editorOverviewRuler.bracketMatchForeground");
    /// `minimap.background` (defaults to the editor background).
    minimap_background: Option<Hsla> = r.get("minimap.background");
    /// `minimapSlider.background`.
    minimap_slider: Hsla = r.color("minimapSlider.background");
    /// `minimapSlider.hoverBackground`.
    minimap_slider_hover: Hsla = r.color("minimapSlider.hoverBackground");
    /// `minimapSlider.activeBackground`: while dragging.
    minimap_slider_active: Hsla = r.color("minimapSlider.activeBackground");
    /// Find matches in the minimap (`minimap.findMatchHighlight`).
    minimap_find_match_highlight: Option<Hsla> = r.visible("minimap.findMatchHighlight");
    /// The selection in the minimap (`minimap.selectionHighlight`).
    minimap_selection_highlight: Option<Hsla> = r.visible("minimap.selectionHighlight");
    /// Selection occurrences in the minimap
    /// (`minimap.selectionOccurrenceHighlight`).
    minimap_selection_occurrence_highlight: Option<Hsla> =
        r.visible("minimap.selectionOccurrenceHighlight");
    /// Errors in the minimap (`minimap.errorHighlight`).
    minimap_error_highlight: Option<Hsla> = r.visible("minimap.errorHighlight");
    /// Warnings in the minimap (`minimap.warningHighlight`).
    minimap_warning_highlight: Option<Hsla> = r.visible("minimap.warningHighlight");
    /// Infos in the minimap (`minimap.infoHighlight`).
    minimap_info_highlight: Option<Hsla> = r.visible("minimap.infoHighlight");
    /// `minimap.foregroundOpacity`: its alpha is how opaque minimap text is.
    minimap_foreground_opacity: Hsla = r.color("minimap.foregroundOpacity");
    /// `editorStickyScroll.background` (defaults to the editor background).
    sticky_scroll_background: Option<Hsla> = r.get("editorStickyScroll.background");
    /// Sticky lines' gutter (`editorStickyScrollGutter.background`).
    sticky_scroll_gutter_background: Option<Hsla> = r.get("editorStickyScrollGutter.background");
    /// A hovered or focused sticky line (`editorStickyScrollHover.background`).
    sticky_scroll_hover_background: Hsla = r.color("editorStickyScrollHover.background");
    /// The line under the sticky lines (`editorStickyScroll.border`, high
    /// contrast).
    sticky_scroll_border: Option<Hsla> = r.visible("editorStickyScroll.border");
    /// The shadow under the sticky lines (`editorStickyScroll.shadow`,
    /// defaulting to `scrollbar.shadow`).
    sticky_scroll_shadow: Hsla = r.color("editorStickyScroll.shadow");
    /// Keyboard focus outlines (`focusBorder`).
    focus_border: Hsla = r.color("focusBorder");
    /// High contrast themes' outline around elements (`contrastBorder`).
    contrast_border: Option<Hsla> = r.visible("contrastBorder");
    /// High contrast themes' outline around active elements
    /// (`contrastActiveBorder`).
    contrast_active_border: Option<Hsla> = r.visible("contrastActiveBorder");
    /// Icons (`icon.foreground`).
    icon_foreground: Hsla = r.color("icon.foreground");
    /// Secondary text, like a list item's description
    /// (`descriptionForeground`).
    description_foreground: Hsla = r.color("descriptionForeground");
    /// Disabled text (`disabledForeground`).
    disabled_foreground: Hsla = r.color("disabledForeground");
    /// Inline edit colors (`inlineEdit.*`).
    inline_edit: InlineEditColors = InlineEditColors::from_resolver(r);
    /// Symbol icon colors (`symbolIcon.*Foreground`), in the suggest
    /// widget and the quick outline.
    symbol_icons: SymbolIconColors = SymbolIconColors::from_resolver(r);
    /// An editor banner, such as the unicode highlighter's "too many
    /// characters" one (`banner.background`, defaulting to
    /// `list.activeSelectionBackground`).
    banner_background: Hsla = r.color("banner.background");
    /// `banner.foreground`: a banner's text and links (defaulting to
    /// `list.activeSelectionForeground`, else the editor foreground).
    banner_foreground: Hsla = r.color_first(&["banner.foreground", "editor.foreground"]);
    /// `banner.iconForeground`: a banner's icon (defaulting to
    /// `editorInfo.foreground`).
    banner_icon_foreground: Hsla = r.color("banner.iconForeground");
    /// Error squiggles (`editorError.foreground`).
    editor_error_foreground: Hsla = r.color("editorError.foreground");
    /// The background of error ranges (`editorError.background`).
    editor_error_background: Option<Hsla> = r.visible("editorError.background");
    /// A double underline under errors (`editorError.border`, high
    /// contrast).
    editor_error_border: Option<Hsla> = r.visible("editorError.border");
    /// Warning squiggles (`editorWarning.foreground`).
    editor_warning_foreground: Hsla = r.color("editorWarning.foreground");
    /// `editorWarning.background`.
    editor_warning_background: Option<Hsla> = r.visible("editorWarning.background");
    /// `editorWarning.border`.
    editor_warning_border: Option<Hsla> = r.visible("editorWarning.border");
    /// Info squiggles (`editorInfo.foreground`).
    editor_info_foreground: Hsla = r.color("editorInfo.foreground");
    /// `editorInfo.background`.
    editor_info_background: Option<Hsla> = r.visible("editorInfo.background");
    /// `editorInfo.border`.
    editor_info_border: Option<Hsla> = r.visible("editorInfo.border");
    /// Hint dots (`editorHint.foreground`; `None` in high contrast, which
    /// uses the border).
    editor_hint_foreground: Option<Hsla> = r.visible("editorHint.foreground");
    /// `editorHint.border`.
    editor_hint_border: Option<Hsla> = r.visible("editorHint.border");
    /// Errors in the overview ruler (`editorOverviewRuler.errorForeground`).
    overview_ruler_error: Hsla = r.color("editorOverviewRuler.errorForeground");
    /// Warnings in the overview ruler (`editorOverviewRuler.warningForeground`).
    overview_ruler_warning: Hsla = r.color("editorOverviewRuler.warningForeground");
    /// Infos in the overview ruler (`editorOverviewRuler.infoForeground`).
    overview_ruler_info: Hsla = r.color("editorOverviewRuler.infoForeground");
    /// The marker navigation zone's frame for errors
    /// (`editorMarkerNavigationError.background`).
    marker_navigation_error_background: Hsla = r.color("editorMarkerNavigationError.background");
    /// Its header (`editorMarkerNavigationError.headerBackground`).
    marker_navigation_error_header_background: Hsla =
        r.color("editorMarkerNavigationError.headerBackground");
    /// The zone's frame for warnings (`editorMarkerNavigationWarning.background`).
    marker_navigation_warning_background: Hsla = r.color("editorMarkerNavigationWarning.background");
    /// Its header (`editorMarkerNavigationWarning.headerBackground`).
    marker_navigation_warning_header_background: Hsla =
        r.color("editorMarkerNavigationWarning.headerBackground");
    /// The zone's frame for infos (`editorMarkerNavigationInfo.background`).
    marker_navigation_info_background: Hsla = r.color("editorMarkerNavigationInfo.background");
    /// Its header (`editorMarkerNavigationInfo.headerBackground`).
    marker_navigation_info_header_background: Hsla =
        r.color("editorMarkerNavigationInfo.headerBackground");
    /// The zone's body (`editorMarkerNavigation.background`).
    marker_navigation_background: Hsla = r.color("editorMarkerNavigation.background");
    /// The selected list row (`list.activeSelectionBackground`; `None` in
    /// high contrast, which outlines it).
    list_active_selection_background: Option<Hsla> = r.visible("list.activeSelectionBackground");
    /// Its text (`list.activeSelectionForeground`).
    list_active_selection_foreground: Option<Hsla> = r.get("list.activeSelectionForeground");
    /// Its icon (`list.activeSelectionIconForeground`).
    list_active_selection_icon_foreground: Option<Hsla> =
        r.get("list.activeSelectionIconForeground");
    /// The selected row of an unfocused list
    /// (`list.inactiveSelectionBackground`).
    list_inactive_selection_background: Option<Hsla> = r.visible("list.inactiveSelectionBackground");
    /// Its text (`list.inactiveSelectionForeground`).
    list_inactive_selection_foreground: Option<Hsla> = r.get("list.inactiveSelectionForeground");
    /// Count badges, like the peek view's per-file counts
    /// (`badge.background`).
    badge_background: Hsla = r.color("badge.background");
    /// Their text (`badge.foreground`).
    badge_foreground: Hsla = r.color("badge.foreground");
    /// A hovered or dragged sash (`sash.hoverBorder`).
    sash_hover_border: Hsla = r.color("sash.hoverBorder");
    /// A hovered list row (`list.hoverBackground`).
    list_hover_background: Option<Hsla> = r.visible("list.hoverBackground");
    /// Its text (`list.hoverForeground`).
    list_hover_foreground: Option<Hsla> = r.get("list.hoverForeground");
    /// The focused list row (`list.focusBackground`).
    list_focus_background: Option<Hsla> = r.visible("list.focusBackground");
    /// Its text (`list.focusForeground`).
    list_focus_foreground: Option<Hsla> = r.get("list.focusForeground");
    /// Its outline (`list.focusOutline`).
    list_focus_outline: Option<Hsla> = r.visible("list.focusOutline");
    /// Matched characters in lists (`list.highlightForeground`).
    list_highlight_foreground: Hsla = r.color("list.highlightForeground");
    /// Matched characters on the focused row (`list.focusHighlightForeground`).
    list_focus_highlight_foreground: Hsla = r.color("list.focusHighlightForeground");
    /// Invalid list items, like a rename candidate that can't apply
    /// (`list.invalidItemForeground`).
    list_invalid_item_foreground: Hsla = r.color("list.invalidItemForeground");
    /// De-emphasized list items (`list.deemphasizedForeground`).
    list_deemphasized_foreground: Hsla = r.color("list.deemphasizedForeground");
    /// The code action list (`editorActionList.background`).
    action_list_background: Hsla = r.color("editorActionList.background");
    /// Its items (`editorActionList.foreground`).
    action_list_foreground: Hsla = r.color("editorActionList.foreground");
    /// Its focused item (`editorActionList.focusBackground`; `None` in high
    /// contrast).
    action_list_focus_background: Option<Hsla> = r.visible("editorActionList.focusBackground");
    /// Its focused item's text (`editorActionList.focusForeground`).
    action_list_focus_foreground: Option<Hsla> = r.get("editorActionList.focusForeground");
    /// Quick access (`quickInput.background`).
    quick_input_background: Hsla = r.color("quickInput.background");
    /// Its text (`quickInput.foreground`).
    quick_input_foreground: Hsla = r.color("quickInput.foreground");
    /// Its title bar (`quickInputTitle.background`).
    quick_input_title_background: Hsla = r.color("quickInputTitle.background");
    /// Its progress bar while a provider is busy
    /// (`progressBar.background`).
    progress_bar_background: Hsla = r.color("progressBar.background");
    /// Its focused row (`quickInputList.focusBackground`; `None` in high
    /// contrast, which outlines it with `list.focusOutline`).
    quick_input_list_focus_background: Option<Hsla> = r.visible("quickInputList.focusBackground");
    /// Its focused row's text (`quickInputList.focusForeground`).
    quick_input_list_focus_foreground: Option<Hsla> = r.get("quickInputList.focusForeground");
    /// Its focused row's icon (`quickInputList.focusIconForeground`).
    quick_input_list_focus_icon_foreground: Option<Hsla> = r.get("quickInputList.focusIconForeground");
    /// Matched characters on its focused row
    /// (`quickInputList.focusHighlightForeground`).
    quick_input_list_focus_highlight_foreground: Hsla =
        r.color("quickInputList.focusHighlightForeground");
    /// Matched characters in the quick input's rows: Monaco's
    /// `standaloneQuickInput.css`, which overrides the theme's
    /// `list.highlightForeground` by base theme (`#0097fb` in vs-dark,
    /// `#0066bf` in vs, `#f38518` in hc-black, `#0f4a85` in hc-light).
    quick_input_highlight_foreground: Hsla = match r.scheme() {
        ColorScheme::Dark => c(rgb(0x0097fb)),
        ColorScheme::Light => c(rgb(0x0066bf)),
        ColorScheme::HcDark => c(rgb(0xf38518)),
        ColorScheme::HcLight => c(rgb(0x0f4a85)),
    };
    /// Matched characters on the quick input's focused row: the same, but
    /// `#9dddff` in vs (`standaloneQuickInput.css`).
    quick_input_focus_highlight_foreground: Hsla = match r.scheme() {
        ColorScheme::Light => c(rgb(0x9dddff)),
        ColorScheme::Dark => c(rgb(0x0097fb)),
        ColorScheme::HcDark => c(rgb(0xf38518)),
        ColorScheme::HcLight => c(rgb(0x0f4a85)),
    };
    /// Quick access group labels (`pickerGroup.foreground`).
    picker_group_foreground: Hsla = r.color("pickerGroup.foreground");
    /// The line above a group (`pickerGroup.border`).
    picker_group_border: Hsla = r.color("pickerGroup.border");
    /// Keybinding labels (`keybindingLabel.background`).
    keybinding_label_background: Hsla = r.color("keybindingLabel.background");
    /// `keybindingLabel.foreground`.
    keybinding_label_foreground: Hsla = r.color("keybindingLabel.foreground");
    /// `keybindingLabel.border`.
    keybinding_label_border: Hsla = r.color("keybindingLabel.border");
    /// `keybindingLabel.bottomBorder`.
    keybinding_label_bottom_border: Hsla = r.color("keybindingLabel.bottomBorder");
    /// A hovered toolbar button (`toolbar.hoverBackground`; `None` in high
    /// contrast, which outlines it).
    toolbar_hover_background: Option<Hsla> = r.visible("toolbar.hoverBackground");
    /// A hovered toolbar button's outline (`toolbar.hoverOutline`).
    toolbar_hover_outline: Option<Hsla> = r.visible("toolbar.hoverOutline");
    /// A pressed toolbar button (`toolbar.activeBackground`).
    toolbar_active_background: Option<Hsla> = r.visible("toolbar.activeBackground");
}

/// The colors of symbol icons, `symbolIcon.*Foreground` (Monaco
/// `contrib/symbolIcons/browser/symbolIcons.ts`): the icons of the suggest
/// widget's completion kinds and of the quick outline's symbol kinds. Those
/// without a color of their own default to the theme's `foreground`.
#[derive(Clone, Debug, PartialEq)]
pub struct SymbolIconColors {
    /// `symbolIcon.arrayForeground`.
    pub array: Hsla,
    /// `symbolIcon.booleanForeground`.
    pub boolean: Hsla,
    /// `symbolIcon.classForeground`.
    pub class: Hsla,
    /// `symbolIcon.colorForeground`.
    pub color: Hsla,
    /// `symbolIcon.constantForeground`.
    pub constant: Hsla,
    /// `symbolIcon.constructorForeground`.
    pub constructor: Hsla,
    /// `symbolIcon.enumeratorForeground`: enums and values.
    pub enumerator: Hsla,
    /// `symbolIcon.enumeratorMemberForeground`.
    pub enumerator_member: Hsla,
    /// `symbolIcon.eventForeground`.
    pub event: Hsla,
    /// `symbolIcon.fieldForeground`.
    pub field: Hsla,
    /// `symbolIcon.fileForeground`.
    pub file: Hsla,
    /// `symbolIcon.folderForeground`.
    pub folder: Hsla,
    /// `symbolIcon.functionForeground`.
    pub function: Hsla,
    /// `symbolIcon.interfaceForeground`.
    pub interface: Hsla,
    /// `symbolIcon.keyForeground`.
    pub key: Hsla,
    /// `symbolIcon.keywordForeground`.
    pub keyword: Hsla,
    /// `symbolIcon.methodForeground`.
    pub method: Hsla,
    /// `symbolIcon.moduleForeground`.
    pub module: Hsla,
    /// `symbolIcon.namespaceForeground`.
    pub namespace: Hsla,
    /// `symbolIcon.nullForeground`.
    pub null: Hsla,
    /// `symbolIcon.numberForeground`.
    pub number: Hsla,
    /// `symbolIcon.objectForeground`.
    pub object: Hsla,
    /// `symbolIcon.operatorForeground`.
    pub operator: Hsla,
    /// `symbolIcon.packageForeground`.
    pub package: Hsla,
    /// `symbolIcon.propertyForeground`.
    pub property: Hsla,
    /// `symbolIcon.referenceForeground`.
    pub reference: Hsla,
    /// `symbolIcon.snippetForeground`.
    pub snippet: Hsla,
    /// `symbolIcon.stringForeground`.
    pub string: Hsla,
    /// `symbolIcon.structForeground`.
    pub structure: Hsla,
    /// `symbolIcon.textForeground`.
    pub text: Hsla,
    /// `symbolIcon.typeParameterForeground`.
    pub type_parameter: Hsla,
    /// `symbolIcon.unitForeground`.
    pub unit: Hsla,
    /// `symbolIcon.variableForeground`.
    pub variable: Hsla,
}

impl SymbolIconColors {
    /// Monaco's defaults: `foreground` (the workbench text color) for most
    /// kinds, and fixed colors for classes, functions, fields and the like,
    /// the light themes' ones with `light`.
    pub fn new(foreground: Hsla, light: bool) -> Self {
        let pick = |dark: u32, light_color: u32| c(rgb(if light { light_color } else { dark }));
        let orange = pick(0xee9d28, 0xd67e00);
        let purple = pick(0xb180d7, 0x652d90);
        let blue = pick(0x75beff, 0x007acc);
        let mut colors = Self {
            class: orange,
            constructor: purple,
            enumerator: orange,
            enumerator_member: blue,
            event: orange,
            field: blue,
            function: purple,
            interface: blue,
            method: purple,
            variable: blue,
            ..Self::all(foreground)
        };
        colors.set_foreground(foreground);
        colors
    }

    fn all(color: Hsla) -> Self {
        Self {
            array: color,
            boolean: color,
            class: color,
            color,
            constant: color,
            constructor: color,
            enumerator: color,
            enumerator_member: color,
            event: color,
            field: color,
            file: color,
            folder: color,
            function: color,
            interface: color,
            key: color,
            keyword: color,
            method: color,
            module: color,
            namespace: color,
            null: color,
            number: color,
            object: color,
            operator: color,
            package: color,
            property: color,
            reference: color,
            snippet: color,
            string: color,
            structure: color,
            text: color,
            type_parameter: color,
            unit: color,
            variable: color,
        }
    }

    /// Sets the colors that default to `foreground`.
    fn set_foreground(&mut self, foreground: Hsla) {
        for slot in [
            &mut self.array,
            &mut self.boolean,
            &mut self.color,
            &mut self.constant,
            &mut self.file,
            &mut self.folder,
            &mut self.key,
            &mut self.keyword,
            &mut self.module,
            &mut self.namespace,
            &mut self.null,
            &mut self.number,
            &mut self.object,
            &mut self.operator,
            &mut self.package,
            &mut self.property,
            &mut self.reference,
            &mut self.snippet,
            &mut self.string,
            &mut self.structure,
            &mut self.text,
            &mut self.type_parameter,
            &mut self.unit,
        ] {
            *slot = foreground;
        }
    }

    /// The color of a completion kind's icon (Monaco
    /// `CompletionItemKinds.toIcon` and `symbolIcons.css`). `None` for the
    /// kinds whose codicon has no symbol color: `Customcolor`, `User`,
    /// `Issue` and `Tool`.
    pub fn for_completion_kind(&self, kind: crate::providers::CompletionItemKind) -> Option<Hsla> {
        use crate::providers::CompletionItemKind as K;
        Some(match kind {
            K::Method => self.method,
            K::Function => self.function,
            K::Constructor => self.constructor,
            K::Field => self.field,
            K::Variable => self.variable,
            K::Class => self.class,
            K::Struct => self.structure,
            K::Interface => self.interface,
            K::Module => self.module,
            K::Property => self.property,
            K::Event => self.event,
            K::Operator => self.operator,
            K::Unit => self.unit,
            // `symbol-value` shares the enumerator color.
            K::Value | K::Enum => self.enumerator,
            K::Constant => self.constant,
            K::EnumMember => self.enumerator_member,
            K::Keyword => self.keyword,
            K::Snippet => self.snippet,
            K::Text => self.text,
            K::Color => self.color,
            K::File => self.file,
            K::Reference => self.reference,
            K::Folder => self.folder,
            K::TypeParameter => self.type_parameter,
            K::Customcolor | K::User | K::Issue | K::Tool => return None,
        })
    }

    /// The color of a symbol kind's icon (Monaco `SymbolKinds.toIcon` and
    /// `symbolIcons.css`).
    pub fn for_symbol_kind(&self, kind: crate::providers::SymbolKind) -> Hsla {
        use crate::providers::SymbolKind as K;
        match kind {
            K::File => self.file,
            K::Module => self.module,
            K::Namespace => self.namespace,
            K::Package => self.package,
            K::Class => self.class,
            K::Method => self.method,
            K::Property => self.property,
            K::Field => self.field,
            K::Constructor => self.constructor,
            K::Enum => self.enumerator,
            K::Interface => self.interface,
            K::Function => self.function,
            K::Variable => self.variable,
            K::Constant => self.constant,
            K::String => self.string,
            K::Number => self.number,
            K::Boolean => self.boolean,
            K::Array => self.array,
            K::Object => self.object,
            K::Key => self.key,
            K::Null => self.null,
            K::EnumMember => self.enumerator_member,
            K::Struct => self.structure,
            K::Event => self.event,
            K::Operator => self.operator,
            K::TypeParameter => self.type_parameter,
        }
    }

    /// The `symbolIcon.*Foreground` colors.
    fn from_resolver(r: &mut Resolver) -> Self {
        let mut get = |key: &str| r.color(&format!("symbolIcon.{key}Foreground"));
        Self {
            array: get("array"),
            boolean: get("boolean"),
            class: get("class"),
            color: get("color"),
            constant: get("constant"),
            constructor: get("constructor"),
            enumerator: get("enumerator"),
            enumerator_member: get("enumeratorMember"),
            event: get("event"),
            field: get("field"),
            file: get("file"),
            folder: get("folder"),
            function: get("function"),
            interface: get("interface"),
            key: get("key"),
            keyword: get("keyword"),
            method: get("method"),
            module: get("module"),
            namespace: get("namespace"),
            null: get("null"),
            number: get("number"),
            object: get("object"),
            operator: get("operator"),
            package: get("package"),
            property: get("property"),
            reference: get("reference"),
            snippet: get("snippet"),
            string: get("string"),
            structure: get("struct"),
            text: get("text"),
            type_parameter: get("typeParameter"),
            unit: get("unit"),
            variable: get("variable"),
        }
    }
}

/// Inline edit (next edit suggestion) colors, `inlineEdit.*` (Monaco
/// `inlineEdits/theme.ts`), defaulting to diff editor, button and hover
/// colors.
#[derive(Clone, Debug, PartialEq)]
pub struct InlineEditColors {
    /// `inlineEdit.originalBackground` (20% of
    /// `diffEditor.removedTextBackground`).
    pub original_background: Hsla,
    /// `inlineEdit.modifiedBackground` (30% of
    /// `diffEditor.insertedTextBackground`).
    pub modified_background: Hsla,
    /// `inlineEdit.originalChangedLineBackground` (80% of
    /// `diffEditor.removedTextBackground`).
    pub original_changed_line_background: Hsla,
    /// `inlineEdit.originalChangedTextBackground` (80% of
    /// `diffEditor.removedTextBackground`).
    pub original_changed_text_background: Hsla,
    /// `inlineEdit.modifiedChangedLineBackground` (70% of
    /// `diffEditor.insertedLineBackground`, all of it in high contrast).
    pub modified_changed_line_background: Hsla,
    /// `inlineEdit.modifiedChangedTextBackground` (70% of
    /// `diffEditor.insertedTextBackground`).
    pub modified_changed_text_background: Hsla,
    /// `inlineEdit.originalBorder` (`diffEditor.removedTextBackground`).
    pub original_border: Hsla,
    /// `inlineEdit.modifiedBorder` (`diffEditor.insertedTextBackground`,
    /// darkened in light themes).
    pub modified_border: Hsla,
    /// `inlineEdit.tabWillAcceptOriginalBorder`.
    pub tab_will_accept_original_border: Hsla,
    /// `inlineEdit.tabWillAcceptModifiedBorder`.
    pub tab_will_accept_modified_border: Hsla,
    /// `inlineEdit.gutterIndicator.background`: the gutter behind the
    /// indicator.
    pub gutter_indicator_background: Hsla,
    /// `inlineEdit.gutterIndicator.primaryForeground`, `primaryBorder` and
    /// `primaryBackground`: the indicator when Tab jumps (button colors, the
    /// background at 40%, 50% in light themes).
    pub gutter_indicator_primary: [Hsla; 3],
    /// `inlineEdit.gutterIndicator.secondaryForeground`, `secondaryBorder`
    /// and `secondaryBackground`: when Tab does neither (the hover widget's
    /// colors).
    pub gutter_indicator_secondary: [Hsla; 3],
    /// `inlineEdit.gutterIndicator.successfulForeground`,
    /// `successfulBorder` and `successfulBackground`: when Tab accepts
    /// (button colors).
    pub gutter_indicator_successful: [Hsla; 3],
}

impl InlineEditColors {
    fn from_resolver(r: &mut Resolver) -> Self {
        let mut indicator = |kind: &str| {
            ["Foreground", "Border", "Background"]
                .map(|part| r.color(&format!("inlineEdit.gutterIndicator.{kind}{part}")))
        };
        let gutter_indicator_primary = indicator("primary");
        let gutter_indicator_secondary = indicator("secondary");
        let gutter_indicator_successful = indicator("successful");
        Self {
            original_background: r.color("inlineEdit.originalBackground"),
            modified_background: r.color("inlineEdit.modifiedBackground"),
            original_changed_line_background: r.color("inlineEdit.originalChangedLineBackground"),
            original_changed_text_background: r.color("inlineEdit.originalChangedTextBackground"),
            modified_changed_line_background: r.color("inlineEdit.modifiedChangedLineBackground"),
            modified_changed_text_background: r.color("inlineEdit.modifiedChangedTextBackground"),
            original_border: r.color("inlineEdit.originalBorder"),
            modified_border: r.color("inlineEdit.modifiedBorder"),
            tab_will_accept_original_border: r.color("inlineEdit.tabWillAcceptOriginalBorder"),
            tab_will_accept_modified_border: r.color("inlineEdit.tabWillAcceptModifiedBorder"),
            gutter_indicator_background: r.color("inlineEdit.gutterIndicator.background"),
            gutter_indicator_primary,
            gutter_indicator_secondary,
            gutter_indicator_successful,
        }
    }
}

impl EditorColors {
    /// `inlineEdit.modifiedBackground`.
    pub fn inline_edit_modified_background(&self) -> Hsla {
        self.inline_edit.modified_background
    }

    /// `inlineEdit.originalChangedTextBackground`.
    pub fn inline_edit_original_changed_text_background(&self) -> Hsla {
        self.inline_edit.original_changed_text_background
    }

    /// `inlineEdit.modifiedChangedTextBackground`.
    pub fn inline_edit_modified_changed_text_background(&self) -> Hsla {
        self.inline_edit.modified_changed_text_background
    }

    /// `inlineEdit.originalBorder`.
    pub fn inline_edit_original_border(&self) -> Hsla {
        self.inline_edit.original_border
    }

    /// `inlineEdit.modifiedBorder`.
    pub fn inline_edit_modified_border(&self) -> Hsla {
        self.inline_edit.modified_border
    }

    /// The gutter indicator's foreground, border and background when Tab
    /// jumps.
    pub fn inline_edit_gutter_indicator_primary(&self) -> [Hsla; 3] {
        self.inline_edit.gutter_indicator_primary
    }

    /// The gutter indicator's foreground, border and background when Tab
    /// does neither.
    pub fn inline_edit_gutter_indicator_secondary(&self) -> [Hsla; 3] {
        self.inline_edit.gutter_indicator_secondary
    }

    /// The gutter indicator's foreground, border and background when Tab
    /// accepts.
    pub fn inline_edit_gutter_indicator_successful(&self) -> [Hsla; 3] {
        self.inline_edit.gutter_indicator_successful
    }
}

fn c(color: gpui::Rgba) -> Hsla {
    color.into()
}

/// The colors Monaco's built-in themes set (`standalone/common/themes.ts`);
/// the rest are registry defaults.
const VS_COLORS: &[(&str, u32)] = &[
    ("editor.background", 0xfffffeff),
    ("editor.foreground", 0x000000ff),
    ("editor.inactiveSelectionBackground", 0xe5ebf1ff),
    ("editorIndentGuide.background1", 0xd3d3d3ff),
    ("editorIndentGuide.activeBackground1", 0x939393ff),
    ("editor.selectionHighlightBackground", 0xadd6ff4d),
];
const VS_DARK_COLORS: &[(&str, u32)] = &[
    ("editor.background", 0x1e1e1eff),
    ("editor.foreground", 0xd4d4d4ff),
    ("editor.inactiveSelectionBackground", 0x3a3d41ff),
    ("editorIndentGuide.background1", 0x404040ff),
    ("editorIndentGuide.activeBackground1", 0x707070ff),
    ("editor.selectionHighlightBackground", 0xadd6ff26),
];
const HC_BLACK_COLORS: &[(&str, u32)] = &[
    ("editor.background", 0x000000ff),
    ("editor.foreground", 0xffffffff),
    ("editorIndentGuide.background1", 0xffffffff),
    ("editorIndentGuide.activeBackground1", 0xffffffff),
];
const HC_LIGHT_COLORS: &[(&str, u32)] = &[
    ("editor.background", 0xffffffff),
    ("editor.foreground", 0x292929ff),
    ("editorIndentGuide.background1", 0x292929ff),
    ("editorIndentGuide.activeBackground1", 0x292929ff),
];

fn color_map(colors: &[(&str, u32)]) -> HashMap<String, Hsla> {
    colors
        .iter()
        .map(|(id, color)| (id.to_string(), c(rgba(*color))))
        .collect()
}

/// VS Code color ids and values, parsed; a value that isn't a hex color is
/// red, as Monaco's `Color.fromHex` makes it.
fn parse_colors(colors: &HashMap<String, String>) -> HashMap<String, Hsla> {
    colors
        .iter()
        .map(|(id, value)| {
            let color = parse_hex_color(value).unwrap_or_else(|| rgba(0xff0000ff).into());
            (id.clone(), color)
        })
        .collect()
}

impl EditorColors {
    /// The colors of a theme of `scheme` that sets `colors` itself, the
    /// others from their Monaco defaults (Monaco `StandaloneTheme.getColor`).
    pub(crate) fn resolve(scheme: ColorScheme, colors: &HashMap<String, Hsla>) -> Self {
        Self::from_resolver(&mut Resolver::new(scheme, colors))
    }

    /// Monaco `vs-dark`.
    pub fn vs_dark() -> Self {
        Self::resolve(ColorScheme::Dark, &color_map(VS_DARK_COLORS))
    }

    /// Monaco `vs`.
    pub fn vs() -> Self {
        Self::resolve(ColorScheme::Light, &color_map(VS_COLORS))
    }

    /// Monaco `hc-black`: the theme's own colors plus the `hcDark` registry
    /// defaults. `contrastBorder` is `#6FC3DF` and `contrastActiveBorder`
    /// (`focusBorder`) `#F38518`; highlights are outlined with it rather
    /// than filled.
    pub fn hc_black() -> Self {
        Self::resolve(ColorScheme::HcDark, &color_map(HC_BLACK_COLORS))
    }

    /// Monaco `hc-light`: the theme's own colors plus the `hcLight` registry
    /// defaults. `contrastBorder` is `#0F4A85` and `contrastActiveBorder`
    /// (`focusBorder`) `#006BBD`.
    pub fn hc_light() -> Self {
        Self::resolve(ColorScheme::HcLight, &color_map(HC_LIGHT_COLORS))
    }

    /// The color for brackets at nesting `level` (Monaco cycles through the
    /// non-transparent `editorBracketHighlight.foreground*` colors).
    pub fn bracket_color(&self, level: u32) -> Option<Hsla> {
        let colors: Vec<Hsla> = self.bracket_highlights.iter().flatten().copied().collect();
        (!colors.is_empty()).then(|| colors[level as usize % colors.len()])
    }

    /// The bracket pair guide color at nesting `level`, and the active
    /// one (Monaco's `indentGuides.ts` theming): each level's
    /// `editorBracketPairGuide.background*` (else its bracket color at 30%)
    /// and `activeBackground*` (else its bracket color), cycling through the
    /// levels that have both.
    pub fn bracket_pair_guide_colors(&self, level: u32) -> Option<(Hsla, Hsla)> {
        let colors: Vec<(Hsla, Hsla)> = (0..6)
            .filter_map(|i| {
                let bracket = self.bracket_highlights[i];
                let guide = self.bracket_pair_guides[i]
                    .or_else(|| bracket.map(|color| transparent(color, 0.3)))
                    .filter(|color| color.a > 0.)?;
                let active = self.bracket_pair_guides_active[i]
                    .or(bracket)
                    .filter(|color| color.a > 0.)?;
                Some((guide, active))
            })
            .collect();
        (!colors.is_empty()).then(|| colors[level as usize % colors.len()])
    }

    /// A marker's squiggle and icon color: `editorError.foreground` and the
    /// like, the border for hints in high contrast (which have no
    /// foreground).
    pub fn marker_color(&self, severity: crate::MarkerSeverity) -> Hsla {
        use crate::MarkerSeverity as S;
        match severity {
            S::Error => self.editor_error_foreground,
            S::Warning => self.editor_warning_foreground,
            S::Info => self.editor_info_foreground,
            S::Hint => self
                .editor_hint_foreground
                .or(self.editor_hint_border)
                .unwrap_or_else(gpui::transparent_black),
        }
    }

    /// The background of a marker's range (`editorError.background` and
    /// the like; none by default).
    pub fn marker_background(&self, severity: crate::MarkerSeverity) -> Option<Hsla> {
        use crate::MarkerSeverity as S;
        match severity {
            S::Error => self.editor_error_background,
            S::Warning => self.editor_warning_background,
            S::Info => self.editor_info_background,
            S::Hint => None,
        }
    }

    /// A marker's double underline in high contrast (`editorError.border`
    /// and the like; Monaco `squiggly-error`'s `border-bottom`). Hints
    /// have their own dotted `editor_hint_border`.
    pub fn marker_border(&self, severity: crate::MarkerSeverity) -> Option<Hsla> {
        use crate::MarkerSeverity as S;
        match severity {
            S::Error => self.editor_error_border,
            S::Warning => self.editor_warning_border,
            S::Info => self.editor_info_border,
            S::Hint => None,
        }
    }

    /// A marker's overview ruler mark (`editorOverviewRuler.errorForeground`
    /// and the like; none for hints).
    pub fn marker_overview_color(&self, severity: crate::MarkerSeverity) -> Option<Hsla> {
        use crate::MarkerSeverity as S;
        match severity {
            S::Error => Some(self.overview_ruler_error),
            S::Warning => Some(self.overview_ruler_warning),
            S::Info => Some(self.overview_ruler_info),
            S::Hint => None,
        }
    }

    /// The marker navigation zone's frame and header colors for a marker
    /// (`editorMarkerNavigation{Error,Warning,Info}.background` and
    /// `.headerBackground`; hints use the info ones, as in Monaco).
    pub fn marker_navigation_colors(&self, severity: crate::MarkerSeverity) -> (Hsla, Hsla) {
        use crate::MarkerSeverity as S;
        match severity {
            S::Error => (
                self.marker_navigation_error_background,
                self.marker_navigation_error_header_background,
            ),
            S::Warning => (
                self.marker_navigation_warning_background,
                self.marker_navigation_warning_header_background,
            ),
            S::Info | S::Hint => (
                self.marker_navigation_info_background,
                self.marker_navigation_info_header_background,
            ),
        }
    }

    /// Styles a list's focused row with `background` and `foreground` (a
    /// list's `*.focusBackground` / `*.focusForeground` pair), outlined with
    /// `list.focusOutline` when there's no background (high contrast).
    pub(crate) fn focused_list_row<E: gpui::Styled>(
        &self,
        row: E,
        background: Option<Hsla>,
        foreground: Option<Hsla>,
    ) -> E {
        let row = match foreground {
            Some(foreground) => row.text_color(foreground),
            None => row,
        };
        match (background, self.list_focus_outline) {
            (Some(background), _) => row.bg(background),
            (None, Some(outline)) => row.border_1().border_color(outline),
            (None, None) => row,
        }
    }

    /// A hovered toolbar button's style (`toolbar.hoverBackground`, or the
    /// `toolbar.hoverOutline` in high contrast).
    pub fn toolbar_hover(&self, style: gpui::StyleRefinement) -> gpui::StyleRefinement {
        use gpui::Styled as _;
        let style = match self.toolbar_hover_background {
            Some(background) => style.bg(background),
            None => style,
        };
        match self.toolbar_hover_outline {
            Some(outline) => style.border_1().border_color(outline),
            None => style,
        }
    }

    /// The indent guide color at indentation `level` (0-based), and the
    /// active one, cycling through the levels whose
    /// `editorIndentGuide.background*` and `activeBackground*` are both
    /// visible (Monaco's `indentGuides.ts` theming).
    pub fn indent_guide_colors(&self, level: u32) -> Option<(Hsla, Hsla)> {
        let colors: Vec<(Hsla, Hsla)> = (0..6)
            .filter_map(|i| Some((self.indent_guides[i]?, self.indent_guides_active[i]?)))
            .collect();
        (!colors.is_empty()).then(|| colors[level as usize % colors.len()])
    }
}

/// Monaco's `transparent(color, factor)`.
fn transparent(color: Hsla, factor: f32) -> Hsla {
    Hsla {
        a: color.a * factor,
        ..color
    }
}

impl Default for EditorColors {
    fn default() -> Self {
        Self::vs()
    }
}

/// `#RGB`, `#RGBA`, `#RRGGBB` or `#RRGGBBAA` (Monaco `Color.fromHex`), as
/// theme colors are written.
pub fn parse_hex_color(value: &str) -> Option<Hsla> {
    let hex = value.trim().strip_prefix('#')?;
    if !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    let expand = |s: &str| -> String { s.chars().flat_map(|c| [c, c]).collect() };
    let full = match hex.len() {
        3 => format!("{}ff", expand(hex)),
        4 => expand(hex),
        6 => format!("{hex}ff"),
        8 => hex.to_string(),
        _ => return None,
    };
    let value = u32::from_str_radix(&full, 16).ok()?;
    Some(rgba(value).into())
}

/// A Monaco token rule's color: `RRGGBB` or `RRGGBBAA`, with an optional
/// `#`; the alpha is dropped (Monaco's `ColorMap.getId`).
pub fn parse_token_color(value: &str) -> Option<Hsla> {
    let hex = value.strip_prefix('#').unwrap_or(value);
    if !matches!(hex.len(), 6 | 8) || !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    Some(rgb(u32::from_str_radix(&hex[..6], 16).ok()?).into())
}

/// How a token is drawn.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct TokenStyle {
    /// Text color; `None` uses the editor foreground.
    pub foreground: Option<Hsla>,
    /// Bold text.
    pub bold: bool,
    /// Italic text.
    pub italic: bool,
    /// Underlined text.
    pub underline: bool,
    /// Struck-through text.
    pub strikethrough: bool,
    /// Opacity multiplied into the color (e.g. faded unused code).
    pub opacity: Option<f32>,
}

/// A token rule's `fontStyle` (Monaco `FontStyle`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TokenFontStyle {
    /// `bold`.
    pub bold: bool,
    /// `italic`.
    pub italic: bool,
    /// `underline`.
    pub underline: bool,
    /// `strikethrough`.
    pub strikethrough: bool,
}

impl TokenFontStyle {
    /// Parses a `fontStyle` such as `"bold italic"`: space-separated whole
    /// words, the unknown ones ignored, so `""` is no style (Monaco
    /// `parseTokenTheme`).
    pub fn parse(font_style: &str) -> Self {
        let mut style = Self::default();
        for segment in font_style.split(' ') {
            match segment {
                "bold" => style.bold = true,
                "italic" => style.italic = true,
                "underline" => style.underline = true,
                "strikethrough" => style.strikethrough = true,
                _ => {}
            }
        }
        style
    }
}

/// The vocabulary a [`TokenRule`] selector uses.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RuleKind {
    /// Selector is a Monarch token name.
    Monaco,
    /// Selector is a TextMate scope.
    TextMate,
}

/// A theme rule: tokens matching `selector` get its color and font style.
/// Fields left `None` are inherited from the rule for the selector's parent
/// (`keyword` for `keyword.flow`), as in Monaco's `TokenTheme`.
#[derive(Clone, Debug, PartialEq)]
pub struct TokenRule {
    /// Which vocabulary `selector` is in.
    pub kind: RuleKind,
    /// A dotted token name or scope; matches itself and names extending it
    /// with `.` (`keyword` matches `keyword.control`), the deepest match
    /// winning. `""` sets the default for every token.
    pub selector: String,
    /// Text color; `None` keeps the parent rule's.
    pub foreground: Option<Hsla>,
    /// Font style; `None` keeps the parent rule's, and an empty
    /// [`TokenFontStyle`] (Monaco's `fontStyle: ''`) clears it.
    pub font_style: Option<TokenFontStyle>,
}

impl TokenRule {
    /// A Monaco rule, as `monaco.editor.defineTheme` takes them (`{ token,
    /// foreground, fontStyle }`). Fails, like Monaco, for a color other than
    /// `RRGGBB` or `RRGGBBAA` (with an optional `#`; the alpha is ignored).
    pub fn monaco(
        token: impl Into<String>,
        foreground: Option<&str>,
        font_style: Option<&str>,
    ) -> anyhow::Result<Self> {
        let foreground = match foreground {
            Some(color) => Some(
                parse_token_color(color)
                    .ok_or_else(|| anyhow::anyhow!("Illegal value for token color: {color}"))?,
            ),
            None => None,
        };
        Ok(Self {
            kind: RuleKind::Monaco,
            selector: token.into(),
            foreground,
            font_style: font_style.map(TokenFontStyle::parse),
        })
    }
}

/// Whether a theme is light or dark (Monaco `base`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ThemeKind {
    /// A light theme.
    Light,
    /// A dark theme.
    Dark,
}

/// Editor colors and token rules.
pub struct Theme {
    /// Display name.
    pub name: String,
    /// Light or dark.
    pub kind: ThemeKind,
    /// Editor chrome colors.
    pub colors: EditorColors,
    rules: Vec<TokenRule>,
    monaco_rules: TokenTrie,
    textmate_rules: TokenTrie,
    /// The colors the theme sets itself (Monaco `IStandaloneThemeData.colors`),
    /// which [`Theme::define`] builds on.
    own_colors: HashMap<String, Hsla>,
    /// Styles by highlight and language.
    cache: Mutex<HashMap<HighlightId, HashMap<String, TokenStyle>>>,
    /// Whether semantic tokens color the text when
    /// `semantic_highlighting` is `ConfiguredByTheme` (VS Code's theme
    /// `semanticHighlighting`; Monaco's standalone themes say no).
    pub semantic_highlighting: bool,
    /// Whether it's a high contrast theme (Monaco's `hc-black` and `hc-light`
    /// bases, or a VS Code theme of `type` `hc`, `hcDark` or `hcLight`).
    /// [`ThemeKind`] says whether it's light or dark.
    pub high_contrast: bool,
    /// `semanticTokenColors` rules.
    pub(crate) semantic_rules: Vec<crate::semantic_theme::SemanticTokenRule>,
    pub(crate) semantic_cache: Mutex<HashMap<String, Option<crate::semantic_theme::SemanticStyle>>>,
}

impl Clone for Theme {
    fn clone(&self) -> Self {
        let mut theme = Self::new(
            self.name.clone(),
            self.kind,
            self.colors.clone(),
            self.rules.clone(),
        );
        theme.own_colors = self.own_colors.clone();
        theme.semantic_highlighting = self.semantic_highlighting;
        theme.high_contrast = self.high_contrast;
        theme.semantic_rules = self.semantic_rules.clone();
        theme
    }
}

impl std::fmt::Debug for Theme {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Theme").field("name", &self.name).finish()
    }
}

macro_rules! monaco_rules {
    ($($token:literal => $color:literal $(, $style:literal)?);* $(;)?) => {
        vec![$(TokenRule {
            kind: RuleKind::Monaco,
            selector: $token.to_string(),
            foreground: if $color.is_empty() { None } else { parse_token_color($color) },
            font_style: None $(.or(Some(TokenFontStyle::parse($style))))?,
        }),*]
    };
}

fn trie(rules: &[TokenRule], kind: RuleKind) -> TokenTrie {
    TokenTrie::new(
        rules
            .iter()
            .filter(|rule| rule.kind == kind)
            .map(|rule| (rule.selector.as_str(), rule.foreground, rule.font_style)),
    )
}

/// `token` with the `.language` suffix Monaco's tokens carry, unless it
/// already ends with it.
fn with_language(token: &str, language: &str) -> String {
    if language.is_empty()
        || token
            .strip_suffix(language)
            .is_some_and(|rest| rest.ends_with('.'))
    {
        token.to_string()
    } else {
        format!("{token}.{language}")
    }
}

impl Theme {
    /// A theme from its parts. Token styles are resolved lazily and cached.
    pub fn new(
        name: impl Into<String>,
        kind: ThemeKind,
        colors: EditorColors,
        rules: Vec<TokenRule>,
    ) -> Self {
        Self {
            name: name.into(),
            kind,
            colors,
            monaco_rules: trie(&rules, RuleKind::Monaco),
            textmate_rules: trie(&rules, RuleKind::TextMate),
            rules,
            own_colors: HashMap::new(),
            cache: Mutex::default(),
            semantic_highlighting: false,
            high_contrast: false,
            semantic_rules: Vec::new(),
            semantic_cache: Mutex::default(),
        }
    }

    /// The token rules, in order.
    pub fn rules(&self) -> &[TokenRule] {
        &self.rules
    }

    pub(crate) fn scheme(&self) -> ColorScheme {
        ColorScheme::new(self.kind == ThemeKind::Light, self.high_contrast)
    }

    /// The theme's color `id` (such as `editorOverviewRuler.findMatchForeground`),
    /// else Monaco's default for it in this kind of theme (Monaco
    /// `IColorTheme.getColor` with `useDefault`); `None` for an unknown id
    /// or a color that's null by default.
    pub fn color(&self, id: &str) -> Option<Hsla> {
        Resolver::new(self.scheme(), &self.own_colors).get(id)
    }

    fn builtin(
        name: &str,
        scheme: ColorScheme,
        colors: &[(&str, u32)],
        rules: Vec<TokenRule>,
    ) -> Self {
        let own_colors = color_map(colors);
        let kind = if scheme.is_light() {
            ThemeKind::Light
        } else {
            ThemeKind::Dark
        };
        let mut theme = Self::new(
            name,
            kind,
            EditorColors::resolve(scheme, &own_colors),
            rules,
        );
        theme.own_colors = own_colors;
        theme.high_contrast = scheme.is_high_contrast();
        theme
    }

    /// Monaco's `vs-dark`.
    pub fn vs_dark() -> Self {
        let rules = monaco_rules! {
            "" => "D4D4D4";
            "invalid" => "f44747";
            "emphasis" => "", "italic";
            "strong" => "", "bold";
            "variable" => "74B0DF";
            "variable.predefined" => "4864AA";
            "variable.parameter" => "9CDCFE";
            "constant" => "569CD6";
            "comment" => "608B4E";
            "number" => "B5CEA8";
            "number.hex" => "5BB498";
            "regexp" => "B46695";
            "annotation" => "cc6666";
            "type" => "3DC9B0";
            "delimiter" => "DCDCDC";
            "delimiter.html" => "808080";
            "delimiter.xml" => "808080";
            "tag" => "569CD6";
            "tag.id.pug" => "4F76AC";
            "tag.class.pug" => "4F76AC";
            "meta.scss" => "A79873";
            "meta.tag" => "CE9178";
            "metatag" => "DD6A6F";
            "metatag.content.html" => "9CDCFE";
            "metatag.html" => "569CD6";
            "metatag.xml" => "569CD6";
            "metatag.php" => "", "bold";
            "key" => "9CDCFE";
            "string.key.json" => "9CDCFE";
            "string.value.json" => "CE9178";
            "attribute.name" => "9CDCFE";
            "attribute.value" => "CE9178";
            "attribute.value.number.css" => "B5CEA8";
            "attribute.value.unit.css" => "B5CEA8";
            "attribute.value.hex.css" => "D4D4D4";
            "string" => "CE9178";
            "string.sql" => "FF0000";
            "keyword" => "569CD6";
            "keyword.flow" => "C586C0";
            "keyword.json" => "CE9178";
            "keyword.flow.scss" => "569CD6";
            "operator.scss" => "909090";
            "operator.sql" => "778899";
            "operator.swift" => "909090";
            "predefined.sql" => "FF00FF";
        };
        Self::builtin("vs-dark", ColorScheme::Dark, VS_DARK_COLORS, rules)
    }

    /// Monaco's `vs`.
    pub fn vs() -> Self {
        let rules = monaco_rules! {
            "" => "000000";
            "invalid" => "cd3131";
            "emphasis" => "", "italic";
            "strong" => "", "bold";
            "variable" => "001188";
            "variable.predefined" => "4864AA";
            "constant" => "dd0000";
            "comment" => "008000";
            "number" => "098658";
            "number.hex" => "3030c0";
            "regexp" => "800000";
            "annotation" => "808080";
            "type" => "008080";
            "delimiter" => "000000";
            "delimiter.html" => "383838";
            "delimiter.xml" => "0000FF";
            "tag" => "800000";
            "tag.id.pug" => "4F76AC";
            "tag.class.pug" => "4F76AC";
            "meta.scss" => "800000";
            "metatag" => "e00000";
            "metatag.content.html" => "FF0000";
            "metatag.html" => "808080";
            "metatag.xml" => "808080";
            "metatag.php" => "", "bold";
            "key" => "863B00";
            "string.key.json" => "A31515";
            "string.value.json" => "0451A5";
            "attribute.name" => "FF0000";
            "attribute.value" => "0451A5";
            "attribute.value.number" => "098658";
            "attribute.value.unit" => "098658";
            "attribute.value.html" => "0000FF";
            "attribute.value.xml" => "0000FF";
            "string" => "A31515";
            "string.html" => "0000FF";
            "string.sql" => "FF0000";
            "string.yaml" => "0451A5";
            "keyword" => "0000FF";
            "keyword.json" => "0451A5";
            "keyword.flow" => "AF00DB";
            "keyword.flow.scss" => "0000FF";
            "operator.scss" => "666666";
            "operator.sql" => "778899";
            "operator.swift" => "666666";
            "predefined.sql" => "C700C7";
        };
        Self::builtin("vs", ColorScheme::Light, VS_COLORS, rules)
    }

    /// Monaco's `hc-black` (high contrast dark). [`ThemeKind`] has no
    /// high-contrast variant, so its kind is [`ThemeKind::Dark`].
    pub fn hc_black() -> Self {
        let rules = monaco_rules! {
            "" => "FFFFFF";
            "invalid" => "f44747";
            "emphasis" => "", "italic";
            "strong" => "", "bold";
            "variable" => "1AEBFF";
            "variable.parameter" => "9CDCFE";
            "constant" => "569CD6";
            "comment" => "608B4E";
            "number" => "FFFFFF";
            "regexp" => "C0C0C0";
            "annotation" => "569CD6";
            "type" => "3DC9B0";
            "delimiter" => "FFFF00";
            "delimiter.html" => "FFFF00";
            "tag" => "569CD6";
            "tag.id.pug" => "4F76AC";
            "tag.class.pug" => "4F76AC";
            "meta" => "D4D4D4";
            "meta.tag" => "CE9178";
            "metatag" => "569CD6";
            "metatag.content.html" => "1AEBFF";
            "metatag.html" => "569CD6";
            "metatag.xml" => "569CD6";
            "metatag.php" => "", "bold";
            "key" => "9CDCFE";
            "string.key" => "9CDCFE";
            "string.value" => "CE9178";
            "attribute.name" => "569CD6";
            "attribute.value" => "3FF23F";
            "string" => "CE9178";
            "string.sql" => "FF0000";
            "keyword" => "569CD6";
            "keyword.flow" => "C586C0";
            "operator.sql" => "778899";
            "operator.swift" => "909090";
            "predefined.sql" => "FF00FF";
        };
        Self::builtin("hc-black", ColorScheme::HcDark, HC_BLACK_COLORS, rules)
    }

    /// Monaco's `hc-light` (high contrast light). [`ThemeKind`] has no
    /// high-contrast variant, so its kind is [`ThemeKind::Light`].
    pub fn hc_light() -> Self {
        let rules = monaco_rules! {
            "" => "292929";
            "invalid" => "B5200D";
            "emphasis" => "", "italic";
            "strong" => "", "bold";
            "variable" => "264F70";
            "variable.predefined" => "4864AA";
            "constant" => "dd0000";
            "comment" => "008000";
            "number" => "098658";
            "number.hex" => "3030c0";
            "regexp" => "800000";
            "annotation" => "808080";
            "type" => "008080";
            "delimiter" => "000000";
            "delimiter.html" => "383838";
            "tag" => "800000";
            "tag.id.pug" => "4F76AC";
            "tag.class.pug" => "4F76AC";
            "meta.scss" => "800000";
            "metatag" => "e00000";
            "metatag.content.html" => "B5200D";
            "metatag.html" => "808080";
            "metatag.xml" => "808080";
            "metatag.php" => "", "bold";
            "key" => "863B00";
            "string.key.json" => "A31515";
            "string.value.json" => "0451A5";
            "attribute.name" => "264F78";
            "attribute.value" => "0451A5";
            "string" => "A31515";
            "string.sql" => "B5200D";
            "keyword" => "0000FF";
            "keyword.flow" => "AF00DB";
            "operator.sql" => "778899";
            "operator.swift" => "666666";
            "predefined.sql" => "C700C7";
        };
        Self::builtin("hc-light", ColorScheme::HcLight, HC_LIGHT_COLORS, rules)
    }

    /// Monaco `defineTheme`: `base` plus extra Monaco rules and VS Code color
    /// keys.
    ///
    /// With `inherit`, the base's rules come first and its colors stay
    /// unless `colors` changes them, directly or through a color derived
    /// from one (`editor.selectionBackground` also moves the inactive
    /// selection). Without it, only `colors` and the registry defaults for
    /// the base's kind apply, and only `rules`. As in Monaco,
    /// `editor.foreground` and `editor.background` in `colors` also become a
    /// `''` rule, the default token style. A color that isn't hex (`#rgb`,
    /// `#rgba`, `#rrggbb` or `#rrggbbaa`) is red, as Monaco makes it.
    ///
    /// Unlike Monaco, which only accepts `vs`, `vs-dark`, `hc-black` and
    /// `hc-light`, any theme can be the base.
    pub fn define(
        name: impl Into<String>,
        base: &Theme,
        inherit: bool,
        rules: Vec<TokenRule>,
        colors: &HashMap<String, String>,
    ) -> Self {
        let scheme = base.scheme();
        let own = parse_colors(colors);
        let mut all_rules = if inherit {
            base.rules.clone()
        } else {
            Vec::new()
        };
        if let Some(foreground) = own.get("editor.foreground") {
            all_rules.push(TokenRule {
                kind: RuleKind::Monaco,
                selector: String::new(),
                foreground: Some(Hsla {
                    a: 1.,
                    ..*foreground
                }),
                font_style: None,
            });
        }
        all_rules.extend(rules);
        let (editor_colors, own_colors) = if inherit {
            let mut merged = base.own_colors.clone();
            merged.extend(own);
            let before = EditorColors::resolve(scheme, &base.own_colors);
            let after = EditorColors::resolve(scheme, &merged);
            let mut colors = base.colors.clone();
            colors.merge_changed(&before, &after);
            (colors, merged)
        } else {
            (EditorColors::resolve(scheme, &own), own)
        };
        let mut theme = Self::new(name, base.kind, editor_colors, all_rules);
        theme.own_colors = own_colors;
        theme.high_contrast = base.high_contrast;
        theme
    }

    /// Loads a VS Code color theme (`colors` + `tokenColors`). An `include`
    /// is ignored; use [`Theme::from_vscode_json_with_includes`] or
    /// [`Theme::from_vscode_file`] to follow it.
    pub fn from_vscode_json(json: &str) -> Result<Self, serde_json::Error> {
        Self::from_vscode_json_with_includes(json, &|_| None)
    }

    /// Like [`Theme::from_vscode_json`], resolving `include` paths (as written
    /// in the JSON, e.g. `./dark_vs.json`) with `resolve`. Included themes are
    /// applied first, so the including theme's colors and rules win.
    ///
    /// Colors the theme doesn't set get the registry defaults for its
    /// `type`: `light`, `dark`, `hcDark` (or `hc`) or `hcLight`.
    pub fn from_vscode_json_with_includes(
        json: &str,
        resolve: &dyn Fn(&str) -> Option<String>,
    ) -> Result<Self, serde_json::Error> {
        let mut merged = VsCodeTheme::default();
        load_vscode_theme(json, resolve, 0, &mut merged)?;
        // VS Code's bundled themes leave `type` to the extension manifest
        // (`uiTheme`), so fall back to the background's lightness.
        let scheme = match merged.kind.as_deref() {
            Some("light" | "vs") => ColorScheme::Light,
            Some("hcLight" | "hc-light") => ColorScheme::HcLight,
            Some("hc" | "hcDark" | "hc-black") => ColorScheme::HcDark,
            Some(_) => ColorScheme::Dark,
            None => match merged
                .colors
                .get("editor.background")
                .and_then(|c| parse_hex_color(c))
            {
                Some(background) if background.l > 0.5 => ColorScheme::Light,
                _ => ColorScheme::Dark,
            },
        };
        let own_colors = parse_colors(&merged.colors);
        let colors = EditorColors::resolve(scheme, &own_colors);
        let mut rules = Vec::new();
        for token_color in merged.token_colors {
            let foreground = token_color
                .settings
                .foreground
                .as_deref()
                .and_then(parse_hex_color);
            let font_style = token_color
                .settings
                .font_style
                .as_deref()
                .map(TokenFontStyle::parse);
            let scopes = match token_color.scope {
                // A rule without a scope sets the defaults.
                None => vec![String::new()],
                Some(Scope::One(scopes)) => scopes
                    .split(',')
                    .map(|s| s.trim().to_string())
                    .collect::<Vec<_>>(),
                Some(Scope::Many(scopes)) => scopes,
            };
            for scope in scopes {
                // Descendant selectors ("meta.embedded string") are matched on
                // their last segment only.
                let selector = scope.split_whitespace().last().unwrap_or_default();
                rules.push(TokenRule {
                    kind: RuleKind::TextMate,
                    selector: selector.to_string(),
                    foreground,
                    font_style,
                });
            }
        }
        let kind = if scheme.is_light() {
            ThemeKind::Light
        } else {
            ThemeKind::Dark
        };
        let mut theme = Self::new(
            merged.name.unwrap_or_else(|| "VS Code theme".into()),
            kind,
            colors,
            rules,
        );
        theme.own_colors = own_colors;
        theme.semantic_highlighting = merged.semantic_highlighting.unwrap_or(false);
        theme.high_contrast = scheme.is_high_contrast();
        theme.semantic_rules = merged
            .semantic_token_colors
            .iter()
            .filter_map(|(selector, value)| {
                crate::semantic_theme::SemanticTokenRule::parse(selector, value)
            })
            .collect();
        Ok(theme)
    }

    /// Loads a VS Code theme file, following `include`s relative to it.
    pub fn from_vscode_file(path: &std::path::Path) -> std::io::Result<Self> {
        let json = std::fs::read_to_string(path)?;
        let dir = path
            .parent()
            .map(std::path::Path::to_path_buf)
            .unwrap_or_default();
        Self::from_vscode_json_with_includes(&json, &|include| {
            std::fs::read_to_string(dir.join(include)).ok()
        })
        .map_err(std::io::Error::other)
    }

    /// The style for a highlight capture or Monarch token, without a
    /// language: language-specific rules (`keyword.json`) don't apply to
    /// captures. See [`Theme::style_for_language`].
    pub fn style_for(&self, highlight: HighlightId) -> TokenStyle {
        self.style_for_language(highlight, None)
    }

    /// The style for a highlight capture in `language` (a language id such
    /// as `json`), or a Monarch token. A capture's Monaco token gets the
    /// language as a suffix, as Monaco's tokens have, so `keyword.json`
    /// rules apply to JSON's `true`.
    pub fn style_for_language(&self, highlight: HighlightId, language: Option<&str>) -> TokenStyle {
        let language = language.unwrap_or_default();
        let mut cache = self.cache.lock().unwrap_or_else(|e| e.into_inner());
        let by_language = cache.entry(highlight).or_default();
        if let Some(style) = by_language.get(language) {
            return *style;
        }
        let style = if highlight.is_monaco_token() {
            self.resolve_monaco_token(&highlight.name())
        } else {
            self.resolve(&highlight.name(), language)
        };
        by_language.insert(language.to_string(), style);
        style
    }

    /// The style of tokens no rule matches: the `''` rules', Monaco's first.
    fn default_style(&self) -> TokenStyle {
        self.monaco_rules
            .root()
            .or_else(|| self.textmate_rules.root())
            .unwrap_or_default()
    }

    /// The most specific TextMate rule's style for `scope`, if one matches.
    pub(crate) fn textmate_style(&self, scope: &str) -> Option<TokenStyle> {
        self.textmate_rules.specific(scope)
    }

    /// Monaco `TokenTheme._match` over the Monaco rules: the style for
    /// `token`, the default one if no rule matches. `None` if the theme has
    /// no Monaco rules (it isn't a standalone theme).
    pub(crate) fn monaco_style(&self, token: &str) -> Option<TokenStyle> {
        self.rules
            .iter()
            .any(|rule| rule.kind == RuleKind::Monaco)
            .then(|| self.monaco_rules.match_token(token).0.style())
    }

    /// The style for a Monarch token such as `keyword.rust`: the Monaco
    /// rules matching it (`keyword.rust`, then `keyword`), as Monaco's
    /// `TokenTheme.match` picks, else the TextMate rules matching the token
    /// as a scope, else the TextMate scopes of the capture its first segment
    /// names (`type` gives `entity.name.type`, ...).
    fn resolve_monaco_token(&self, token: &str) -> TokenStyle {
        if let Some(style) = self
            .monaco_rules
            .specific(token)
            .or_else(|| self.textmate_rules.specific(token))
        {
            return style;
        }
        for scope in textmate_scopes_for_capture(token) {
            if let Some(style) = self.textmate_rules.specific(scope) {
                return style;
            }
        }
        self.default_style()
    }

    fn resolve(&self, capture: &str, language: &str) -> TokenStyle {
        // TextMate rules: the first candidate scope with a matching rule wins.
        for scope in textmate_scopes_for_capture(capture) {
            if let Some(style) = self.textmate_rules.specific(scope) {
                return style;
            }
        }
        if let Some(token) = monaco_token_for_capture_in(capture, language)
            && let Some(style) = self.monaco_rules.specific(&with_language(token, language))
        {
            return style;
        }
        self.default_style()
    }
}

#[derive(Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct VsCodeTheme {
    #[serde(default)]
    name: Option<String>,
    #[serde(default, rename = "type")]
    kind: Option<String>,
    #[serde(default)]
    include: Option<String>,
    #[serde(default)]
    colors: HashMap<String, String>,
    #[serde(default)]
    token_colors: Vec<TokenColor>,
    #[serde(default)]
    semantic_highlighting: Option<bool>,
    #[serde(default)]
    semantic_token_colors: HashMap<String, serde_json::Value>,
}

#[derive(Deserialize)]
struct TokenColor {
    #[serde(default)]
    scope: Option<Scope>,
    settings: Settings,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum Scope {
    One(String),
    Many(Vec<String>),
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Settings {
    #[serde(default)]
    foreground: Option<String>,
    #[serde(default)]
    font_style: Option<String>,
}

/// Merges `json` (and, first, whatever it includes) into `merged`.
fn load_vscode_theme(
    json: &str,
    resolve: &dyn Fn(&str) -> Option<String>,
    depth: usize,
    merged: &mut VsCodeTheme,
) -> Result<(), serde_json::Error> {
    let theme: VsCodeTheme = serde_json::from_str(&syntax::strip_jsonc(json))?;
    if depth < 8
        && let Some(include) = theme.include.as_deref()
        && let Some(included) = resolve(include)
    {
        load_vscode_theme(&included, resolve, depth + 1, merged)?;
    }
    if theme.name.is_some() {
        merged.name = theme.name;
    }
    if theme.kind.is_some() {
        merged.kind = theme.kind;
    }
    merged.colors.extend(theme.colors);
    merged.token_colors.extend(theme.token_colors);
    if theme.semantic_highlighting.is_some() {
        merged.semantic_highlighting = theme.semantic_highlighting;
    }
    merged
        .semantic_token_colors
        .extend(theme.semantic_token_colors);
    Ok(())
}

/// The Monarch token a tree-sitter capture corresponds to in Monaco themes,
/// without a language (see [`monaco_token_for_capture_in`]).
pub fn monaco_token_for_capture(capture: &str) -> Option<&'static str> {
    let first = capture.split('.').next().unwrap_or(capture);
    Some(match capture {
        "string.special.key" => "string.key.json",
        "string.regex" | "string.regexp" => "regexp",
        "variable.builtin" => "variable.predefined",
        "variable.parameter" => "variable.parameter",
        "constant.builtin" | "boolean" => "keyword",
        "number.hex" => "number.hex",
        "text.title" | "markup.heading" => "keyword",
        "text.emphasis" | "markup.italic" => "emphasis",
        "text.strong" | "markup.bold" => "strong",
        "text.literal" | "markup.raw" => "variable",
        "text.uri" | "text.reference" | "markup.link" => "string",
        _ => match first {
            "comment" => "comment",
            "string" | "character" => "string",
            "number" | "float" => "number",
            "keyword" | "conditional" | "repeat" | "include" | "exception" => "keyword",
            "type" | "constructor" => "type",
            "tag" => "tag",
            "attribute" => "attribute.name",
            "property" => "key",
            "constant" => "constant",
            "operator" | "punctuation" => "delimiter",
            "escape" => "string",
            "label" | "annotation" => "annotation",
            _ => return None,
        },
    })
}

/// The Monarch token a tree-sitter capture in `language` corresponds to,
/// before the language suffix: what Monaco's own tokenizer for the language
/// emits where it differs from [`monaco_token_for_capture`] (JSON string
/// values are `string.value`, HTML attribute values `attribute.value`, YAML
/// keys `type`, CSS properties `attribute.name`, ...).
pub fn monaco_token_for_capture_in(capture: &str, language: &str) -> Option<&'static str> {
    let specific = match (language, capture) {
        ("json" | "jsonc", "string") => Some("string.value"),
        ("json" | "jsonc", "string.special.key") => Some("string.key"),
        ("html", "string") => Some("attribute.value"),
        ("html", "constant") => Some("metatag"),
        ("yaml", "property") => Some("type"),
        ("css" | "scss" | "less", "property") => Some("attribute.name"),
        ("css" | "scss" | "less", "number") => Some("attribute.value.number"),
        ("css" | "scss" | "less", "type") => Some("attribute.value.unit"),
        ("css" | "scss" | "less", "string.special") => Some("attribute.value.hex"),
        _ => None,
    };
    specific.or_else(|| monaco_token_for_capture(capture))
}

/// The Monaco token type Monaco's `tokenize` reports for a highlight: a
/// Monarch token as it is (`keyword.rust`), or a capture's Monaco token
/// with the language as a suffix.
pub(crate) fn monaco_token_type(highlight: HighlightId, language: &str) -> String {
    let name = highlight.name();
    if highlight.is_monaco_token() {
        return name.to_string();
    }
    let token = monaco_token_for_capture_in(&name, language).unwrap_or(&name);
    with_language(token, language)
}

/// Candidate TextMate scopes for a tree-sitter capture, most specific first.
pub fn textmate_scopes_for_capture(capture: &str) -> &'static [&'static str] {
    let first = capture.split('.').next().unwrap_or(capture);
    match capture {
        "comment.documentation" | "comment.doc" => &["comment.block.documentation", "comment"],
        "string.special.key" => &[
            "support.type.property-name.json",
            "support.type.property-name",
            "string",
        ],
        "string.escape" | "escape" => &["constant.character.escape"],
        "string.regex" | "string.regexp" => &["string.regexp"],
        "string.special" | "string.special.symbol" => &["constant.other.symbol", "string"],
        "constant.builtin" | "boolean" => &["constant.language"],
        "keyword.operator" | "operator" => &["keyword.operator"],
        "keyword.control"
        | "keyword.return"
        | "keyword.conditional"
        | "keyword.repeat"
        | "conditional"
        | "repeat"
        | "keyword.exception"
        | "exception" => &["keyword.control"],
        "keyword.function" | "keyword.storage" | "keyword.type" => &["storage.type", "keyword"],
        "function.macro" => &["entity.name.function.macro", "entity.name.function"],
        "function.builtin" => &["support.function", "entity.name.function"],
        "function.method" | "function.method.call" | "function.call" => &["entity.name.function"],
        "type.builtin" => &["support.type.primitive", "support.type", "entity.name.type"],
        "variable.builtin" => &["variable.language"],
        "variable.parameter" => &["variable.parameter"],
        "variable.member" | "property" => &[
            "variable.other.property",
            "support.type.property-name",
            "variable",
        ],
        "text.title" | "markup.heading" => &["markup.heading", "entity.name.section"],
        "text.emphasis" | "markup.italic" => &["markup.italic"],
        "text.strong" | "markup.bold" => &["markup.bold"],
        "text.literal" | "markup.raw" => &["markup.inline.raw", "markup.raw"],
        "text.uri" | "markup.link.url" => &["markup.underline.link"],
        "text.reference" | "markup.link" => &["string.other.link", "markup.underline.link"],
        _ => match first {
            "comment" => &["comment"],
            "string" | "character" => &["string"],
            "number" | "float" => &["constant.numeric"],
            "keyword" => &["keyword", "storage.type"],
            "function" => &["entity.name.function", "support.function"],
            "type" => &["entity.name.type", "support.type", "storage.type"],
            "constructor" => &[
                "entity.name.type.class",
                "entity.name.type",
                "entity.name.function",
            ],
            "variable" => &["variable.other.readwrite", "variable"],
            "constant" => &["variable.other.constant", "constant"],
            "attribute" => &["entity.other.attribute-name"],
            "tag" => &["entity.name.tag"],
            "label" => &["entity.name.label"],
            "namespace" | "module" => &["entity.name.namespace", "entity.name.type.module"],
            "punctuation" => &["punctuation"],
            "embedded" => &["meta.embedded"],
            _ => &[],
        },
    }
}

/// A shared theme handle.
pub type ThemeRef = Arc<Theme>;

/// Themes by name (Monaco `StandaloneThemeService._knownThemes`), starting
/// with the built-in ones, and the one [`set_theme`] chose.
struct Themes {
    known: HashMap<String, Arc<Theme>>,
    current: Option<Arc<Theme>>,
}

impl Default for Themes {
    fn default() -> Self {
        let known = [
            Theme::vs(),
            Theme::vs_dark(),
            Theme::hc_black(),
            Theme::hc_light(),
        ]
        .into_iter()
        .map(|theme| (theme.name.clone(), Arc::new(theme)))
        .collect();
        Self {
            known,
            current: None,
        }
    }
}

impl gpui::Global for Themes {}

/// Defines or replaces the theme named `theme.name` (Monaco
/// `editor.defineTheme`), e.g. one made with [`Theme::define`] or
/// [`Theme::from_vscode_json`], for [`set_theme`]. If it's the current
/// theme, every editor switches to the new version. Fails, like Monaco, for
/// names with characters other than ASCII letters, digits and `-`.
pub fn define_theme(cx: &mut gpui::App, theme: Theme) -> anyhow::Result<()> {
    let valid = !theme.name.is_empty()
        && theme
            .name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-');
    if !valid {
        anyhow::bail!("Illegal theme name!");
    }
    let name = theme.name.clone();
    let themes = cx.default_global::<Themes>();
    themes.known.insert(name.clone(), Arc::new(theme));
    // Monaco refreshes the current theme.
    if themes.current.as_ref().is_some_and(|t| t.name == name) {
        set_theme(cx, &name);
    }
    Ok(())
}

/// Switches every editor (including those in diff editors), and those
/// created later, to the theme named `name` (Monaco `editor.setTheme`);
/// an unknown name gets `vs`, as in Monaco. Don't call it from inside an
/// editor's update.
pub fn set_theme(cx: &mut gpui::App, name: &str) {
    let themes = cx.default_global::<Themes>();
    let Some(theme) = themes
        .known
        .get(name)
        .or_else(|| themes.known.get("vs"))
        .cloned()
    else {
        return;
    };
    set_theme_arc(cx, theme);
}

/// [`set_theme`] with the theme itself, which needn't be defined (the
/// theme `toggleHighContrast` switches back to may be an editor's own).
pub(crate) fn set_theme_arc(cx: &mut gpui::App, theme: Arc<Theme>) {
    let themes = cx.default_global::<Themes>();
    if themes
        .current
        .as_ref()
        .is_some_and(|current| Arc::ptr_eq(current, &theme))
    {
        return;
    }
    themes.current = Some(theme.clone());
    crate::registry::update_editors(cx, |editor, cx| editor.set_theme(theme.clone(), cx));
    // Diff editors draw some chrome in their editors' colors.
    for diff_editor in crate::registry::diff_editors(cx) {
        diff_editor.update(cx, |_, cx| cx.notify());
    }
}

/// The theme defined (or built in) with `name`: `vs`, `vs-dark`,
/// `hc-black`, `hc-light`, or one from [`define_theme`].
pub fn theme_named(cx: &gpui::App, name: &str) -> Option<Arc<Theme>> {
    match cx.try_global::<Themes>() {
        Some(themes) => themes.known.get(name).cloned(),
        None => Themes::default().known.get(name).cloned(),
    }
}

/// The theme chosen with [`set_theme`], which new editors start with.
pub fn global_theme(cx: &gpui::App) -> Option<Arc<Theme>> {
    cx.try_global::<Themes>()?.current.clone()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `#rrggbbaa`, as Monaco prints colors.
    fn hex(color: Hsla) -> String {
        let rgba = gpui::Rgba::from(color);
        let channel = |c: f32| (c * 255.).round() as u8;
        format!(
            "#{:02x}{:02x}{:02x}{:02x}",
            channel(rgba.r),
            channel(rgba.g),
            channel(rgba.b),
            channel(rgba.a)
        )
    }

    fn opt_hex(color: Option<Hsla>) -> Option<String> {
        color.map(hex)
    }

    fn colors(entries: &[(&str, &str)]) -> HashMap<String, String> {
        entries
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    fn json_style(theme: &Theme, capture: &str, language: &str) -> Option<String> {
        opt_hex(
            theme
                .style_for_language(HighlightId::intern(capture), Some(language))
                .foreground,
        )
    }

    #[test]
    fn hex_colors() {
        assert_eq!(parse_hex_color("#fff"), Some(rgba(0xffffffff).into()));
        assert_eq!(parse_hex_color("#11223344"), Some(rgba(0x11223344).into()));
        assert_eq!(parse_hex_color("nope"), None);
        assert_eq!(parse_hex_color("#ggg"), None);
    }

    /// Monaco `Color.fromHex`: a value `parseHex` rejects is `Color.red`,
    /// in `defineTheme` colors and VS Code themes alike.
    #[test]
    fn invalid_theme_colors_are_red() {
        let red = "#ff0000ff";
        let theme = Theme::define(
            "invalid",
            &Theme::vs_dark(),
            true,
            Vec::new(),
            &colors(&[
                ("editor.background", "#12345"),
                ("editorCursor.foreground", "blue"),
                ("editor.lineHighlightBackground", "#123456"),
            ]),
        );
        assert_eq!(hex(theme.colors.background), red);
        assert_eq!(hex(theme.colors.cursor), red);
        assert_eq!(
            opt_hex(theme.colors.line_highlight_background).as_deref(),
            Some("#123456ff")
        );
        let theme =
            Theme::from_vscode_json(r#"{"type": "dark", "colors": {"editor.background": ""}}"#)
                .unwrap();
        assert_eq!(hex(theme.colors.background), red);
    }

    #[test]
    fn token_colors_are_parsed_like_monaco() {
        // Monaco's `ColorMap.getId`: 6 or 8 digits, `#` optional, alpha dropped.
        assert_eq!(parse_token_color("FF0000"), Some(rgb(0xff0000).into()));
        assert_eq!(parse_token_color("#ff000080"), Some(rgb(0xff0000).into()));
        assert_eq!(parse_token_color("#fff"), None);
        assert_eq!(parse_token_color("red"), None);
        assert!(TokenRule::monaco("keyword", Some("#fff"), None).is_err());
        let rule = TokenRule::monaco("keyword", None, Some("italic bolder")).unwrap();
        assert_eq!(rule.foreground, None);
        assert_eq!(
            rule.font_style,
            Some(TokenFontStyle {
                italic: true,
                ..Default::default()
            })
        );
    }

    #[test]
    fn monaco_rules_resolve_captures() {
        let theme = Theme::vs_dark();
        let keyword = theme.style_for(HighlightId::intern("keyword"));
        assert_eq!(keyword.foreground, parse_hex_color("#569CD6"));
        let key = theme.style_for(HighlightId::intern("string.special.key"));
        assert_eq!(key.foreground, parse_hex_color("#9CDCFE"));
        // Monaco's vs-dark doesn't color functions: they get the `''` rule.
        let function = theme.style_for(HighlightId::intern("function"));
        assert_eq!(function.foreground, parse_hex_color("#D4D4D4"));
    }

    #[test]
    fn language_specific_rules_apply_to_captures() {
        let (dark, light) = (Theme::vs_dark(), Theme::vs());
        // JSON `true`, `false` and `null` are `keyword.json`.
        assert_eq!(
            json_style(&dark, "constant.builtin", "json").as_deref(),
            Some("#ce9178ff")
        );
        assert_eq!(
            json_style(&light, "constant.builtin", "json").as_deref(),
            Some("#0451a5ff")
        );
        // Without the language, the plain `keyword` rule.
        assert_eq!(
            opt_hex(
                light
                    .style_for(HighlightId::intern("constant.builtin"))
                    .foreground
            )
            .as_deref(),
            Some("#0000ffff")
        );
        // JSON string values and keys.
        assert_eq!(
            json_style(&light, "string", "json").as_deref(),
            Some("#0451a5ff")
        );
        assert_eq!(
            json_style(&light, "string.special.key", "json").as_deref(),
            Some("#a31515ff")
        );
        // HTML punctuation and attribute values.
        assert_eq!(
            json_style(&light, "punctuation.bracket", "html").as_deref(),
            Some("#383838ff")
        );
        assert_eq!(
            json_style(&light, "string", "html").as_deref(),
            Some("#0000ffff")
        );
        // YAML strings.
        assert_eq!(
            json_style(&light, "string", "yaml").as_deref(),
            Some("#0451a5ff")
        );
        // Other languages fall back to the language-less rules.
        assert_eq!(
            json_style(&light, "string", "rust").as_deref(),
            Some("#a31515ff")
        );
        assert_eq!(
            monaco_token_type(HighlightId::intern("string.special.key"), "json"),
            "string.key.json"
        );
        assert_eq!(
            monaco_token_type(HighlightId::intern("constant.builtin"), "json"),
            "keyword.json"
        );
    }

    #[test]
    fn builtin_themes_have_monacos_language_rules() {
        let theme = Theme::vs_dark();
        let monarch = |token: &str| theme.style_for(HighlightId::intern_monaco_token(token));
        assert_eq!(
            opt_hex(monarch("operator.sql").foreground).as_deref(),
            Some("#778899ff")
        );
        assert_eq!(
            opt_hex(monarch("attribute.value.hex.css").foreground).as_deref(),
            Some("#d4d4d4ff")
        );
        // `metatag.php` only sets bold, keeping `metatag`'s color.
        let php = monarch("metatag.php");
        assert!(php.bold);
        assert_eq!(opt_hex(php.foreground).as_deref(), Some("#dd6a6fff"));
        assert_eq!(
            opt_hex(
                Theme::vs()
                    .style_for(HighlightId::intern_monaco_token("string.yaml"))
                    .foreground
            )
            .as_deref(),
            Some("#0451a5ff")
        );
    }

    #[test]
    fn token_rules_inherit_from_parents_and_the_default_rule() {
        let rules = vec![
            TokenRule::monaco("comment.doc", None, Some("italic")).unwrap(),
            TokenRule::monaco("keyword.custom", Some("123456"), None).unwrap(),
            TokenRule::monaco("keyword", None, Some("bold")).unwrap(),
            TokenRule::monaco("", Some("abcdef"), None).unwrap(),
        ];
        let theme = Theme::define(
            "inherit-test",
            &Theme::vs_dark(),
            true,
            rules,
            &HashMap::new(),
        );
        let monarch = |token: &str| theme.style_for(HighlightId::intern_monaco_token(token));
        // A font-style-only rule keeps the parent's color.
        let doc = monarch("comment.doc.rust");
        assert!(doc.italic);
        assert_eq!(opt_hex(doc.foreground).as_deref(), Some("#608b4eff"));
        // A color-only rule keeps the parent's font style, and a later
        // `keyword` rule merges into vs-dark's.
        let custom = monarch("keyword.custom");
        assert!(custom.bold);
        assert_eq!(opt_hex(custom.foreground).as_deref(), Some("#123456ff"));
        assert_eq!(
            opt_hex(monarch("keyword").foreground).as_deref(),
            Some("#569cd6ff")
        );
        // The `''` rule is the default.
        assert_eq!(
            opt_hex(monarch("unknown").foreground).as_deref(),
            Some("#abcdefff")
        );
        // `editor.foreground` becomes a `''` rule, as in Monaco.
        let theme = Theme::define(
            "fg-test",
            &Theme::vs_dark(),
            true,
            Vec::new(),
            &colors(&[("editor.foreground", "#112233")]),
        );
        let unknown = theme.style_for(HighlightId::intern_monaco_token("unknown"));
        assert_eq!(opt_hex(unknown.foreground).as_deref(), Some("#112233ff"));
        // A `fontStyle: ''` clears an inherited style.
        let theme = Theme::define(
            "reset-test",
            &Theme::vs_dark(),
            true,
            vec![
                TokenRule::monaco("a", None, Some("bold")).unwrap(),
                TokenRule::monaco("a.b", None, Some("")).unwrap(),
            ],
            &HashMap::new(),
        );
        assert!(
            !theme
                .style_for(HighlightId::intern_monaco_token("a.b"))
                .bold
        );
    }

    #[test]
    fn textmate_rules_inherit_too() {
        let theme = Theme::from_vscode_json(
            r##"{
                "tokenColors": [
                    { "settings": { "foreground": "#010101" } },
                    { "scope": "comment", "settings": { "foreground": "#6A9955", "fontStyle": "italic" } },
                    { "scope": "comment.block.documentation", "settings": { "foreground": "#777777" } }
                ]
            }"##,
        )
        .unwrap();
        let doc = theme.style_for(HighlightId::intern("comment.doc"));
        assert!(doc.italic);
        assert_eq!(opt_hex(doc.foreground).as_deref(), Some("#777777ff"));
        // A rule without a scope is the default.
        let other = theme.style_for(HighlightId::intern("nothing.matches"));
        assert_eq!(opt_hex(other.foreground).as_deref(), Some("#010101ff"));
    }

    #[test]
    fn hc_black_theme() {
        let theme = Theme::hc_black();
        assert_eq!(theme.name, "hc-black");
        assert_eq!(theme.kind, ThemeKind::Dark);
        let colors = &theme.colors;
        assert_eq!(Some(colors.background), parse_hex_color("#000000"));
        assert_eq!(Some(colors.foreground), parse_hex_color("#FFFFFF"));
        assert_eq!(colors.indent_guides[0], parse_hex_color("#FFFFFF"));
        assert_eq!(Some(colors.widget_border), parse_hex_color("#6FC3DF"));
        assert_eq!(Some(colors.peek_view_border), parse_hex_color("#6FC3DF"));
        assert_eq!(Some(colors.focus_border), parse_hex_color("#F38518"));
        assert_eq!(colors.line_highlight_border, parse_hex_color("#F38518"));
        // Highlights are outlined, not filled.
        assert_eq!(colors.find_match_highlight_background.a, 0.);
        assert_eq!(
            colors.find_match_highlight_border,
            parse_hex_color("#F38518")
        );
        assert_eq!(
            colors.selection_highlight_border,
            parse_hex_color("#F38518")
        );
        assert_eq!(
            colors.word_highlight_strong_border,
            parse_hex_color("#F38518")
        );
        assert_eq!(
            opt_hex(colors.find_range_highlight_border).as_deref(),
            Some("#f3851866")
        );
        assert_eq!(colors.input_border, parse_hex_color("#6FC3DF"));
        assert_eq!(colors.button_border, parse_hex_color("#6FC3DF"));
        assert_eq!(colors.widget_shadow, None);
        let keyword = theme.style_for(HighlightId::intern("keyword"));
        assert_eq!(keyword.foreground, parse_hex_color("#569CD6"));
        let delimiter = theme.style_for(HighlightId::intern("punctuation.bracket"));
        assert_eq!(delimiter.foreground, parse_hex_color("#FFFF00"));
        let variable = theme.style_for(HighlightId::intern("text.literal"));
        assert_eq!(variable.foreground, parse_hex_color("#1AEBFF"));
    }

    #[test]
    fn hc_light_theme() {
        let theme = Theme::hc_light();
        assert_eq!(theme.name, "hc-light");
        assert_eq!(theme.kind, ThemeKind::Light);
        let colors = &theme.colors;
        assert_eq!(Some(colors.background), parse_hex_color("#FFFFFF"));
        assert_eq!(Some(colors.foreground), parse_hex_color("#292929"));
        assert_eq!(Some(colors.cursor), parse_hex_color("#0F4A85"));
        assert_eq!(Some(colors.widget_border), parse_hex_color("#0F4A85"));
        assert_eq!(Some(colors.focus_border), parse_hex_color("#006BBD"));
        assert_eq!(Some(colors.active_line_number), parse_hex_color("#006BBD"));
        assert_eq!(Some(colors.multi_diff_border), parse_hex_color("#CCCCCC"));
        let keyword = theme.style_for(HighlightId::intern("keyword"));
        assert_eq!(keyword.foreground, parse_hex_color("#0000FF"));
        let invalid = theme.style_for(HighlightId::intern_monaco_token("invalid"));
        assert_eq!(invalid.foreground, parse_hex_color("#B5200D"));
        let string = theme.style_for(HighlightId::intern("string"));
        assert_eq!(string.foreground, parse_hex_color("#A31515"));
    }

    #[test]
    fn builtin_defaults_match_monaco() {
        let (dark, light) = (EditorColors::vs_dark(), EditorColors::vs());
        let (hc_dark, hc_light) = (EditorColors::hc_black(), EditorColors::hc_light());
        let all = [&dark, &light, &hc_dark, &hc_light];
        let each = |get: &dyn Fn(&EditorColors) -> Hsla| all.map(|colors| hex(get(colors)));
        // Monaco's themes.ts.
        assert_eq!(hex(light.selection_highlight_background), "#add6ff4d");
        // `transparent(editorWidget.foreground, 0.2)`, contrastBorder in hc.
        assert_eq!(
            each(&|c| c.widget_border),
            ["#cccccc33", "#61616133", "#6fc3dfff", "#0f4a85ff"]
        );
        assert_eq!(
            each(&|c| c.menu_separator_background),
            ["#cccccc33", "#61616133", "#6fc3dfff", "#0f4a85ff"]
        );
        // `editorInfo.foreground`, which `peekView.border` and the banner icon
        // follow outside high contrast.
        assert_eq!(
            each(&|c| c.editor_info_foreground),
            ["#59a4f9ff", "#0063d3ff", "#59a4f9ff", "#0063d3ff"]
        );
        assert_eq!(hex(dark.peek_view_border), "#59a4f9ff");
        assert_eq!(hex(light.peek_view_border), "#0063d3ff");
        assert_eq!(hex(light.banner_icon_foreground), "#0063d3ff");
        assert_eq!(
            hex(light.peek_view_title_description_foreground),
            "#616161ff"
        );
        assert_eq!(hex(light.code_lens_foreground), "#919191ff");
        assert_eq!(hex(light.snippet_tabstop_background), "#0a326433");
        assert_eq!(hex(light.input_option_active_background), "#0090f133");
        assert_eq!(hex(light.minimap_slider), "#64646433");
        assert_eq!(
            light.minimap_slider_hover,
            transparent(light.scrollbar_slider_hover, 0.5)
        );
        assert_eq!(
            dark.minimap_slider_active,
            transparent(dark.scrollbar_slider_active, 0.5)
        );
        assert_eq!(hex(light.find_range_highlight_background), "#b4b4b44d");
        assert_eq!(
            each(&|c| c.ime_underline),
            ["#ffffffff", "#000000ff", "#ffffffff", "#000000ff"]
        );
        // Inline edits.
        assert_eq!(
            hex(light.inline_edit.gutter_indicator_primary[2]),
            "#007acc80"
        );
        assert_eq!(
            hex(dark.inline_edit.gutter_indicator_primary[2]),
            "#0e639c66"
        );
        assert!(light.inline_edit.modified_border.l < light.diff_inserted_text_background.l);
        assert_eq!(
            hc_dark.inline_edit.modified_changed_line_background,
            hc_dark.diff_inserted_line_background
        );
        // Menus are dropdowns.
        assert_eq!(
            each(&|c| c.menu_background),
            ["#3c3c3cff", "#ffffffff", "#000000ff", "#ffffffff"]
        );
        assert_eq!(
            each(&|c| c.menu_foreground),
            ["#f0f0f0ff", "#616161ff", "#ffffffff", "#292929ff"]
        );
        assert_eq!(opt_hex(hc_dark.menu_border).as_deref(), Some("#6fc3dfff"));
        assert_eq!(
            opt_hex(hc_dark.menu_selection_border).as_deref(),
            Some("#f38518ff")
        );
        assert_eq!(dark.menu_border, None);
        // Lists and quick input.
        assert_eq!(
            opt_hex(dark.list_active_selection_background).as_deref(),
            Some("#04395eff")
        );
        assert_eq!(
            opt_hex(light.action_list_focus_background).as_deref(),
            Some("#0060c0ff")
        );
        assert_eq!(
            opt_hex(dark.quick_input_list_focus_background).as_deref(),
            Some("#04395eff")
        );
        assert_eq!(
            hex(dark.quick_input_list_focus_highlight_foreground),
            "#2aaaffff"
        );
        assert_eq!(hex(light.list_highlight_foreground), "#0066bfff");
        // `standaloneQuickInput.css`.
        assert_eq!(hex(dark.quick_input_highlight_foreground), "#0097fbff");
        assert_eq!(
            hex(dark.quick_input_focus_highlight_foreground),
            "#0097fbff"
        );
        assert_eq!(hex(light.quick_input_highlight_foreground), "#0066bfff");
        assert_eq!(
            hex(light.quick_input_focus_highlight_foreground),
            "#9dddffff"
        );
        assert_eq!(
            hex(hc_dark.quick_input_focus_highlight_foreground),
            "#f38518ff"
        );
        assert_eq!(hex(hc_dark.list_highlight_foreground), "#f38518ff");
        assert_eq!(hex(dark.keybinding_label_foreground), "#ccccccff");
        assert_eq!(
            opt_hex(dark.toolbar_hover_background).as_deref(),
            Some("#5a5d5e50")
        );
        assert_eq!(hex(dark.input_option_hover_background), "#5a5d5e80");
        // Hovers.
        assert_eq!(dark.hover_background, dark.widget_background);
        assert_eq!(hex(dark.hover_highlight_background), "#264f7840");
        // Editor surfaces.
        assert_eq!(dark.multi_cursor_secondary_foreground, dark.cursor);
        assert_eq!(
            dark.peek_view_editor_gutter_background,
            dark.peek_view_editor_background
        );
        assert_eq!(
            opt_hex(dark.fold_placeholder_foreground).as_deref(),
            Some("#808080ff")
        );
        assert_eq!(
            each(&|c| c.folding_control_foreground),
            ["#c5c5c5ff", "#424242ff", "#ffffffff", "#292929ff"]
        );
        assert_eq!(dark.sticky_scroll_shadow, dark.scrollbar_shadow);
        assert_eq!(
            dark.indent_guide_colors(0).map(|(a, b)| (hex(a), hex(b))),
            Some(("#404040ff".into(), "#707070ff".into()))
        );
        // Levels 2-6 are transparent, so every level uses the first.
        assert_eq!(dark.indent_guide_colors(3), dark.indent_guide_colors(0));
        let (guide, active) = dark.bracket_pair_guide_colors(1).unwrap();
        assert_eq!(
            (hex(guide), hex(active)),
            ("#da70d64d".into(), "#da70d6ff".into())
        );
    }

    #[test]
    fn marker_colors_follow_the_theme() {
        let all = [
            EditorColors::vs_dark(),
            EditorColors::vs(),
            EditorColors::hc_black(),
            EditorColors::hc_light(),
        ];
        let each =
            |get: &dyn Fn(&EditorColors) -> Hsla| all.clone().map(|colors| hex(get(&colors)));
        assert_eq!(
            each(&|c| c.editor_error_foreground),
            ["#f14c4cff", "#e51400ff", "#f48771ff", "#b5200dff"]
        );
        assert_eq!(
            each(&|c| c.editor_warning_foreground),
            ["#cca700ff", "#bf8803ff", "#ffd370ff", "#895503ff"]
        );
        assert_eq!(
            opt_hex(all[0].editor_hint_foreground).as_deref(),
            Some("#eeeeeeb3")
        );
        assert_eq!(
            opt_hex(all[1].editor_hint_foreground).as_deref(),
            Some("#6c6c6cff")
        );
        assert_eq!(all[2].editor_hint_foreground, None);
        assert_eq!(
            opt_hex(all[2].editor_error_border).as_deref(),
            Some("#e47777cc")
        );
        assert_eq!(
            each(&|c| c.overview_ruler_error),
            ["#ff1212b3", "#ff1212b3", "#ff3232ff", "#b5200dff"]
        );
        assert_eq!(hex(all[0].overview_ruler_warning), "#cca700ff");
        assert_eq!(hex(all[2].overview_ruler_warning), "#ffcc00cc");
        assert_eq!(hex(all[0].marker_navigation_error_background), "#f14c4cff");
        assert_eq!(
            hex(all[0].marker_navigation_error_header_background),
            "#f14c4c1a"
        );
        assert_eq!(hex(all[2].marker_navigation_error_background), "#6fc3dfff");
        assert_eq!(
            hex(all[2].marker_navigation_warning_header_background),
            "#0c141fff"
        );
        assert_eq!(all[0].marker_navigation_background, all[0].background);
    }

    #[test]
    fn vscode_theme_json() {
        let json = r##"{
            // Dark+ excerpt
            "name": "Test Dark+",
            "type": "dark",
            "colors": { "editor.background": "#1E1E1E", "editorLineNumber.activeForeground": "#C6C6C6" },
            "tokenColors": [
                { "scope": "keyword", "settings": { "foreground": "#569CD6" } },
                { "scope": ["keyword.control", "keyword.control.flow"], "settings": { "foreground": "#C586C0" } },
                { "scope": "entity.name.function, support.function", "settings": { "foreground": "#DCDCAA" } },
                { "scope": "comment", "settings": { "foreground": "#6A9955", "fontStyle": "italic" } },
            ]
        }"##;
        let theme = Theme::from_vscode_json(json).unwrap();
        assert_eq!(theme.name, "Test Dark+");
        let function = theme.style_for(HighlightId::intern("function.method"));
        assert_eq!(function.foreground, parse_hex_color("#DCDCAA"));
        let flow = theme.style_for(HighlightId::intern("keyword.return"));
        assert_eq!(flow.foreground, parse_hex_color("#C586C0"));
        let comment = theme.style_for(HighlightId::intern("comment"));
        assert!(comment.italic);
    }

    #[test]
    fn vscode_colors_map_to_fields() {
        let theme = Theme::from_vscode_json(
            r##"{
                "type": "dark",
                "colors": {
                    "editorCodeLens.foreground": "#010101",
                    "editor.linkedEditingBackground": "#020202",
                    "editor.rangeHighlightBackground": "#030303",
                    "diffEditor.move.border": "#040404",
                    "diffEditor.moveActive.border": "#050505",
                    "multiDiffEditor.headerBackground": "#060606",
                    "multiDiffEditor.border": "#070707",
                    "editorActiveLineNumber.foreground": "#080808",
                    "menu.background": "#090909",
                    "editorHoverWidget.background": "#0a0a0a",
                    "editorError.foreground": "#0b0b0b",
                    "minimapSlider.activeBackground": "#0c0c0c",
                    "keybindingLabel.foreground": "#0d0d0d",
                    "toolbar.hoverBackground": "#0e0e0e",
                    "editorBracketPairGuide.background1": "#0f0f0f",
                    "editorIndentGuide.background2": "#101010",
                    "editorStickyScroll.shadow": "#111111",
                    "editorOverviewRuler.background": "#121212"
                }
            }"##,
        )
        .unwrap();
        let c = &theme.colors;
        assert_eq!(hex(c.code_lens_foreground), "#010101ff");
        assert_eq!(hex(c.linked_editing_background), "#020202ff");
        assert_eq!(hex(c.range_highlight_background), "#030303ff");
        assert_eq!(hex(c.diff_move_border), "#040404ff");
        assert_eq!(hex(c.diff_move_active_border), "#050505ff");
        assert_eq!(hex(c.multi_diff_header_background), "#060606ff");
        assert_eq!(hex(c.multi_diff_border), "#070707ff");
        // Monaco's default: the editor background.
        assert_eq!(c.multi_diff_background, c.background);
        // The deprecated id is `editorLineNumber.activeForeground`'s default.
        assert_eq!(hex(c.active_line_number), "#080808ff");
        assert_eq!(hex(c.menu_background), "#090909ff");
        assert_eq!(hex(c.hover_background), "#0a0a0aff");
        assert_eq!(hex(c.editor_error_foreground), "#0b0b0bff");
        assert_eq!(hex(c.marker_navigation_error_background), "#0b0b0bff");
        assert_eq!(hex(c.minimap_slider_active), "#0c0c0cff");
        assert_eq!(hex(c.keybinding_label_foreground), "#0d0d0dff");
        assert_eq!(
            opt_hex(c.toolbar_hover_background).as_deref(),
            Some("#0e0e0eff")
        );
        assert_eq!(
            opt_hex(c.bracket_pair_guides[0]).as_deref(),
            Some("#0f0f0fff")
        );
        assert_eq!(opt_hex(c.indent_guides[1]).as_deref(), Some("#101010ff"));
        assert_eq!(hex(c.sticky_scroll_shadow), "#111111ff");
        assert_eq!(
            opt_hex(c.overview_ruler_background).as_deref(),
            Some("#121212ff")
        );
    }

    #[test]
    fn vscode_high_contrast_themes_get_high_contrast_defaults() {
        let theme = Theme::from_vscode_json(r##"{ "type": "hcDark", "colors": {} }"##).unwrap();
        assert!(theme.high_contrast);
        assert_eq!(theme.kind, ThemeKind::Dark);
        assert_eq!(hex(theme.colors.widget_border), "#6fc3dfff");
        assert_eq!(
            opt_hex(theme.colors.find_match_border).as_deref(),
            Some("#f38518ff")
        );
        let theme = Theme::from_vscode_json(r##"{ "type": "hcLight" }"##).unwrap();
        assert!(theme.high_contrast);
        assert_eq!(theme.kind, ThemeKind::Light);
        assert_eq!(hex(theme.colors.widget_border), "#0f4a85ff");
        // A plain dark theme gets the dark registry defaults.
        let theme = Theme::from_vscode_json(r##"{ "type": "dark" }"##).unwrap();
        assert_eq!(hex(theme.colors.background), "#1e1e1eff");
        assert_eq!(hex(theme.colors.foreground), "#bbbbbbff");
    }

    #[test]
    fn derived_defaults_follow_their_source() {
        let define = |base: &Theme, inherit: bool, entries: &[(&str, &str)]| {
            Theme::define("derived", base, inherit, Vec::new(), &colors(entries)).colors
        };
        let dark = Theme::vs_dark();
        let c = define(&dark, true, &[("editor.selectionBackground", "#ff0000")]);
        assert_eq!(hex(c.fold_background), "#ff00004d");
        // vs-dark sets the inactive selection itself.
        assert_eq!(hex(c.inactive_selection_background), "#3a3d41ff");
        let c = define(&dark, true, &[("foreground", "#ff0000")]);
        assert_eq!(hex(c.widget_foreground), "#ff0000ff");
        assert_eq!(hex(c.widget_border), "#ff000033");
        assert_eq!(hex(c.menu_separator_background), "#ff000033");
        assert_eq!(hex(c.symbol_icons.array), "#ff0000ff");
        let c = define(&dark, true, &[("focusBorder", "#ff0000")]);
        assert_eq!(hex(c.input_option_active_background), "#ff000066");
        let c = define(&dark, true, &[("scrollbarSlider.background", "#ff0000")]);
        assert_eq!(hex(c.minimap_slider), "#ff000080");
        let c = define(&dark, true, &[("editorInfo.foreground", "#ff0000")]);
        assert_eq!(hex(c.peek_view_border), "#ff0000ff");
        assert_eq!(hex(c.banner_icon_foreground), "#ff0000ff");
        let c = define(&dark, true, &[("editorWidget.border", "#ff0000")]);
        assert_eq!(hex(c.hover_border), "#ff0000ff");
        assert_eq!(hex(c.suggest_border), "#ff0000ff");
        let c = define(&dark, true, &[("editorCursor.foreground", "#ff0000")]);
        assert_eq!(hex(c.multi_cursor_primary_foreground), "#ff0000ff");
        let c = define(&dark, true, &[("badge.background", "#ff0000")]);
        assert_eq!(hex(c.inlay_hint_background), "#ff00001a");
        let c = define(&dark, true, &[("peekViewEditor.background", "#ff0000")]);
        assert_eq!(hex(c.peek_view_editor_gutter_background), "#ff0000ff");
        let c = define(
            &dark,
            true,
            &[("list.activeSelectionBackground", "#ff0000")],
        );
        assert_eq!(hex(c.menu_selection_background), "#ff0000ff");
        assert_eq!(
            opt_hex(c.quick_input_list_focus_background).as_deref(),
            Some("#ff0000ff")
        );
        assert_eq!(hex(c.banner_background), "#ff0000ff");
        let c = define(&dark, true, &[("editor.background", "#ff0000")]);
        assert_eq!(hex(c.gutter_background), "#ff0000ff");
        assert_eq!(hex(c.marker_navigation_background), "#ff0000ff");
        let c = define(&dark, true, &[("editorWhitespace.foreground", "#ff0000")]);
        assert_eq!(hex(c.word_wrap_indicator), "#ff0000ff");
        // A line highlight background drops the default border...
        let c = define(
            &dark,
            true,
            &[("editor.lineHighlightBackground", "#ff0000")],
        );
        assert_eq!(c.line_highlight_border, None);
        // ...unless it's transparent or the theme sets the border too.
        let c = define(
            &dark,
            true,
            &[("editor.lineHighlightBackground", "#ff000000")],
        );
        assert_eq!(
            opt_hex(c.line_highlight_border).as_deref(),
            Some("#282828ff")
        );
        let c = define(
            &dark,
            true,
            &[
                ("editor.lineHighlightBackground", "#ff0000"),
                ("editor.lineHighlightBorder", "#00ff00"),
            ],
        );
        assert_eq!(
            opt_hex(c.line_highlight_border).as_deref(),
            Some("#00ff00ff")
        );
    }

    #[test]
    fn define_without_inherit_uses_only_registry_defaults() {
        let theme = Theme::define(
            "bare",
            &Theme::vs(),
            false,
            Vec::new(),
            &colors(&[("editor.background", "#ffffff")]),
        );
        let c = &theme.colors;
        // `transparent(editor.selectionBackground, 0.5)`, not themes.ts' #E5EBF1.
        assert_eq!(hex(c.inactive_selection_background), "#add6ff80");
        // `editorIndentGuide.background1` → `editorWhitespace.foreground`.
        assert_eq!(opt_hex(c.indent_guides[0]).as_deref(), Some("#33333333"));
        // `lessProminent(selection, background, 0.3, 0.6)`.
        assert_ne!(hex(c.selection_highlight_background), "#add6ff4d");
        assert_eq!(theme.rules().len(), 0);
        // With inherit, themes.ts' colors stay.
        let theme = Theme::define("full", &Theme::vs(), true, Vec::new(), &HashMap::new());
        assert_eq!(hex(theme.colors.inactive_selection_background), "#e5ebf1ff");
        assert_eq!(theme.colors, EditorColors::vs());
    }

    #[test]
    fn define_keeps_a_custom_bases_own_colors() {
        let mut colors = EditorColors::vs_dark();
        colors.cursor = rgb(0xff0000).into();
        let base = Theme::new("custom", ThemeKind::Dark, colors, Vec::new());
        let theme = Theme::define(
            "derived",
            &base,
            true,
            Vec::new(),
            &colors_map(&[("editor.background", "#000000")]),
        );
        assert_eq!(hex(theme.colors.cursor), "#ff0000ff");
        assert_eq!(hex(theme.colors.background), "#000000ff");
        assert_eq!(hex(theme.colors.gutter_background), "#000000ff");
    }

    fn colors_map(entries: &[(&str, &str)]) -> HashMap<String, String> {
        colors(entries)
    }
}
