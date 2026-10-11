//! Themes: editor chrome colors plus token colors.
//!
//! Two kinds of token rules are supported and can be mixed:
//! - **Monaco rules** keyed by Monarch token names (`keyword`, `string.key.json`),
//!   as in Monaco's built-in themes and `monaco.editor.defineTheme`. The
//!   built-in [`Theme::vs`], [`Theme::vs_dark`], [`Theme::hc_black`] and
//!   [`Theme::hc_light`] are ported from Monaco's
//!   `standalone/common/themes.ts` (MIT). Most language-specific rules
//!   (`*.pug`, `*.sql`, `*.php`, ...) are left out.
//! - **TextMate rules** from VS Code theme JSON `tokenColors`, keyed by scope
//!   selectors (`entity.name.function`, `keyword.control`).
//!
//! Tree-sitter capture names (e.g. `function.method`) are mapped onto both
//! vocabularies by [`monaco_token_for_capture`] and [`textmate_scopes_for_capture`].

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use gpui::{Hsla, Rgba, rgb, rgba};
use serde::Deserialize;
use syntax::HighlightId;

/// Editor chrome colors, named after the VS Code color keys they come from.
///
/// Defaults to [`EditorColors::vs_dark`].
#[derive(Clone, Debug, PartialEq)]
pub struct EditorColors {
    /// Text area background (`editor.background`).
    pub background: Hsla,
    /// Default text color (`editor.foreground`).
    pub foreground: Hsla,
    /// Line numbers (`editorLineNumber.foreground`).
    pub line_number: Hsla,
    /// The cursor's line number (`editorLineNumber.activeForeground`).
    pub active_line_number: Hsla,
    /// Current line fill (`editor.lineHighlightBackground`); `None` draws no fill.
    pub line_highlight_background: Option<Hsla>,
    /// Current line outline (`editor.lineHighlightBorder`); `None` draws no outline.
    pub line_highlight_border: Option<Hsla>,
    /// Selections in the focused editor (`editor.selectionBackground`).
    pub selection_background: Hsla,
    /// Selected text's color (`editor.selectionForeground`; none keeps the
    /// token colors). High-contrast themes set it.
    pub selection_foreground: Option<Hsla>,
    /// Inserted text in the diff editor (`diffEditor.insertedTextBackground`).
    pub diff_inserted_text_background: Hsla,
    /// Removed text (`diffEditor.removedTextBackground`).
    pub diff_removed_text_background: Hsla,
    /// Inserted text's outline (`diffEditor.insertedTextBorder`, high
    /// contrast).
    pub diff_inserted_text_border: Option<Hsla>,
    /// Removed text's outline (`diffEditor.removedTextBorder`).
    pub diff_removed_text_border: Option<Hsla>,
    /// Inserted lines (`diffEditor.insertedLineBackground`).
    pub diff_inserted_line_background: Hsla,
    /// Removed lines (`diffEditor.removedLineBackground`).
    pub diff_removed_line_background: Hsla,
    /// The gutter beside inserted lines
    /// (`diffEditorGutter.insertedLineBackground`).
    pub diff_gutter_inserted_line_background: Hsla,
    /// The gutter beside removed lines
    /// (`diffEditorGutter.removedLineBackground`).
    pub diff_gutter_removed_line_background: Hsla,
    /// The stripes where the other side has lines (`diffEditor.diagonalFill`).
    pub diff_diagonal_fill: Hsla,
    /// The "hidden lines" bars (`diffEditor.unchangedRegionBackground`).
    pub diff_unchanged_region_background: Hsla,
    /// Their text (`diffEditor.unchangedRegionForeground`).
    pub diff_unchanged_region_foreground: Hsla,
    /// Their shadow (`diffEditor.unchangedRegionShadow`).
    pub diff_unchanged_region_shadow: Hsla,
    /// Unchanged lines that can be hidden
    /// (`diffEditor.unchangedCodeBackground`).
    pub diff_unchanged_code_background: Hsla,
    /// Gutter toolbars, like the diff editor's revert buttons
    /// (`editorGutter.itemBackground`).
    pub gutter_item_background: Hsla,
    /// Their icons (`editorGutter.itemGlyphForeground`).
    pub gutter_item_glyph_foreground: Hsla,
    /// Separators in menus and toolbars (`menu.separatorBackground`).
    pub menu_separator_background: Hsla,
    /// Moved blocks' borders and arrows (`diffEditor.move.border`).
    pub diff_move_border: Hsla,
    /// The active moved block's (`diffEditor.moveActive.border`).
    pub diff_move_active_border: Hsla,
    /// A multi-diff editor's file headers
    /// (`multiDiffEditor.headerBackground`).
    pub multi_diff_header_background: Hsla,
    /// The border above them (`multiDiffEditor.border`).
    pub multi_diff_border: Hsla,
    /// Selections in an unfocused editor (`editor.inactiveSelectionBackground`).
    pub inactive_selection_background: Hsla,
    /// The cursor (`editorCursor.foreground`).
    pub cursor: Hsla,
    /// Gutter background (`editorGutter.background`, else the editor background).
    pub gutter_background: Hsla,
    /// Scrollbar thumb (`scrollbarSlider.background`).
    pub scrollbar_slider: Hsla,
    /// Hovered scrollbar thumb (`scrollbarSlider.hoverBackground`).
    pub scrollbar_slider_hover: Hsla,
    /// Dragged scrollbar thumb (`scrollbarSlider.activeBackground`).
    pub scrollbar_slider_active: Hsla,
    /// Shadow drawn when the content is scrolled (`scrollbar.shadow`).
    pub scrollbar_shadow: Hsla,
    /// The overview ruler's left border (`editorOverviewRuler.border`).
    pub overview_ruler_border: Hsla,
    /// Underline of IME composition text; follows the foreground when loading VS Code
    /// colors.
    pub ime_underline: Hsla,
    /// Current find match (`editor.findMatchBackground`).
    pub find_match_background: Hsla,
    /// Other find matches (`editor.findMatchHighlightBackground`).
    pub find_match_highlight_background: Hsla,
    /// Find matches in the overview ruler (`editorOverviewRuler.findMatchForeground`).
    pub find_match_overview: Hsla,
    /// The find in selection scope (`editor.findRangeHighlightBackground`).
    pub find_range_highlight_background: Hsla,
    /// Error text, e.g. the find widget's "No results" (`errorForeground`).
    pub error_foreground: Hsla,
    /// The target of go to definition and similar jumps, briefly
    /// (`editor.symbolHighlightBackground`; defaults to the find match
    /// highlight).
    pub symbol_highlight_background: Hsla,
    /// A link while Cmd/Ctrl is held over it (`editorLink.activeForeground`).
    pub link_active_foreground: Hsla,
    /// Quick access previews (`editor.rangeHighlightBackground`).
    pub range_highlight_background: Hsla,
    /// The border of color decorator swatches (Monaco's
    /// `.colorpicker-color-decoration` CSS, not a theme color).
    pub color_decoration_border: Hsla,
    /// The highlighted menu row (`menu.selectionBackground`).
    pub menu_selection_background: Hsla,
    /// The highlighted menu row's text (`menu.selectionForeground`).
    pub menu_selection_foreground: Hsla,
    /// Button background (`button.background`).
    pub button_background: Hsla,
    /// Button text (`button.foreground`).
    pub button_foreground: Hsla,
    /// Code lens text (`editorCodeLens.foreground`).
    pub code_lens_foreground: Hsla,
    /// Linked editing ranges (`editor.linkedEditingBackground`).
    pub linked_editing_background: Hsla,
    /// Ghost text (`editorGhostText.foreground`).
    pub ghost_text_foreground: Hsla,
    /// `editorGhostText.background` (transparent: none).
    pub ghost_text_background: Hsla,
    /// `editorGhostText.border` (transparent: none).
    pub ghost_text_border: Hsla,
    /// The `placeholder` text (`editor.placeholder.foreground`; Monaco
    /// defaults it to the ghost text color).
    pub placeholder_foreground: Hsla,
    /// Highlighted Unicode characters' border (`editorUnicodeHighlight.border`,
    /// defaulting to `editorWarning.foreground`).
    pub unicode_highlight_border: Hsla,
    /// Their background (`editorUnicodeHighlight.background`; none by
    /// default).
    pub unicode_highlight_background: Hsla,
    /// Inline messages such as "No definition found"
    /// (`inputValidation.infoBackground`).
    pub message_background: Hsla,
    /// Their border (`inputValidation.infoBorder`).
    pub message_border: Hsla,
    /// Collapsed regions' first lines (`editor.foldBackground`).
    pub fold_background: Hsla,
    /// Inlay hint text (`editorInlayHint.foreground`).
    pub inlay_hint_foreground: Hsla,
    /// Inlay hint background (`editorInlayHint.background`).
    pub inlay_hint_background: Hsla,
    /// Type hints (`editorInlayHint.typeForeground`).
    pub inlay_hint_type_foreground: Hsla,
    /// `editorInlayHint.typeBackground`.
    pub inlay_hint_type_background: Hsla,
    /// Parameter hints (`editorInlayHint.parameterForeground`).
    pub inlay_hint_parameter_foreground: Hsla,
    /// `editorInlayHint.parameterBackground`.
    pub inlay_hint_parameter_background: Hsla,
    /// The code action lightbulb (`editorLightBulb.foreground`).
    pub lightbulb_foreground: Hsla,
    /// The lightbulb when a preferred quick fix exists
    /// (`editorLightBulbAutoFix.foreground`).
    pub lightbulb_auto_fix_foreground: Hsla,
    /// The peek view's frame and arrow (`peekView.border`).
    pub peek_view_border: Hsla,
    /// Its title bar (`peekViewTitle.background`).
    pub peek_view_title_background: Hsla,
    /// The title (`peekViewTitleLabel.foreground`).
    pub peek_view_title_foreground: Hsla,
    /// The title's description (the folder) (`peekViewTitleDescription.foreground`).
    pub peek_view_title_description_foreground: Hsla,
    /// The results list (`peekViewResult.background`).
    pub peek_view_result_background: Hsla,
    /// Result lines (`peekViewResult.lineForeground`).
    pub peek_view_result_line_foreground: Hsla,
    /// Result files (`peekViewResult.fileForeground`).
    pub peek_view_result_file_foreground: Hsla,
    /// The selected result (`peekViewResult.selectionBackground`).
    pub peek_view_result_selection_background: Hsla,
    /// The selected result's text (`peekViewResult.selectionForeground`).
    pub peek_view_result_selection_foreground: Hsla,
    /// The match in result lines (`peekViewResult.matchHighlightBackground`).
    pub peek_view_result_match_highlight_background: Hsla,
    /// The preview editor (`peekViewEditor.background`).
    pub peek_view_editor_background: Hsla,
    /// Matches in the preview editor (`peekViewEditor.matchHighlightBackground`).
    pub peek_view_editor_match_highlight_background: Hsla,
    /// Occurrences of the symbol at the cursor (`editor.wordHighlightBackground`).
    pub word_highlight_background: Hsla,
    /// Write occurrences of the symbol at the cursor
    /// (`editor.wordHighlightStrongBackground`).
    pub word_highlight_strong_background: Hsla,
    /// Occurrences in the overview ruler
    /// (`editorOverviewRuler.wordHighlightForeground`).
    pub word_highlight_overview: Hsla,
    /// Write occurrences in the overview ruler
    /// (`editorOverviewRuler.wordHighlightStrongForeground`).
    pub word_highlight_strong_overview: Hsla,
    /// Bracket pair colors by nesting level, cycling
    /// (`editorBracketHighlight.foreground1`–`6`; `None` for transparent).
    pub bracket_highlights: [Option<Hsla>; 6],
    /// Unmatched brackets (`editorBracketHighlight.unexpectedBracket.foreground`).
    pub unexpected_bracket: Hsla,
    /// Rendered whitespace (`editorWhitespace.foreground`).
    pub whitespace: Hsla,
    /// The `wordWrapIndicator` glyph (`editorWordWrapIndicator.foreground`,
    /// defaulting to the whitespace color).
    pub word_wrap_indicator: Hsla,
    /// Indent guides (`editorIndentGuide.background1`).
    pub indent_guide: Hsla,
    /// The active indent guide (`editorIndentGuide.activeBackground1`).
    pub indent_guide_active: Hsla,
    /// Rulers (`editorRuler.foreground`).
    pub ruler: Hsla,
    /// `editorUnnecessaryCode.opacity`: its alpha is how opaque unused code
    /// is drawn.
    pub unnecessary_code_opacity: Hsla,
    /// `editorUnnecessaryCode.border` (transparent: none): an underline for
    /// unused code instead of fading.
    pub unnecessary_code_border: Hsla,
    /// Other occurrences of the selected text (`editor.selectionHighlightBackground`).
    pub selection_highlight_background: Hsla,
    /// Selection occurrences in the overview ruler
    /// (`editorOverviewRuler.selectionHighlightForeground`).
    pub selection_highlight_overview: Hsla,
    /// Widget chrome (`editorWidget.background` / `.foreground` / `.border`).
    pub widget_background: Hsla,
    /// `editorWidget.foreground`.
    pub widget_foreground: Hsla,
    /// `editorWidget.border`.
    pub widget_border: Hsla,
    /// Snippet tab stops (`editor.snippetTabstopHighlightBackground`).
    pub snippet_tabstop_background: Hsla,
    /// `editor.snippetTabstopHighlightBorder` (transparent: none).
    pub snippet_tabstop_border: Hsla,
    /// The final tab stop (`editor.snippetFinalTabstopHighlightBackground`;
    /// transparent: none).
    pub snippet_final_tabstop_background: Hsla,
    /// `editor.snippetFinalTabstopHighlightBorder`.
    pub snippet_final_tabstop_border: Hsla,
    /// The suggest widget (`editorSuggestWidget.background`, defaulting to
    /// the widget background).
    pub suggest_background: Hsla,
    /// `editorSuggestWidget.border` (defaults to the widget border).
    pub suggest_border: Hsla,
    /// `editorSuggestWidget.foreground` (defaults to the editor foreground).
    pub suggest_foreground: Hsla,
    /// `editorSuggestWidget.selectedForeground`: the focused row's text.
    pub suggest_selected_foreground: Hsla,
    /// `editorSuggestWidget.selectedBackground`: the focused row.
    pub suggest_selected_background: Hsla,
    /// `editorSuggestWidget.highlightForeground`: matched characters.
    pub suggest_highlight_foreground: Hsla,
    /// `editorSuggestWidget.focusHighlightForeground`: matched characters on
    /// the focused row.
    pub suggest_focus_highlight_foreground: Hsla,
    /// Input boxes in widgets (`input.background` / `inputOption.activeBackground`).
    pub input_background: Hsla,
    /// `inputOption.activeBackground`: an enabled toggle (e.g. match case).
    pub input_option_active_background: Hsla,
    /// `inputOption.activeBorder`; also used for links in widgets.
    pub input_option_active_border: Hsla,
    /// `editorBracketMatch.background` / `.border`.
    pub bracket_match_background: Hsla,
    /// `editorBracketMatch.border`.
    pub bracket_match_border: Hsla,
    /// The overview ruler mark for matching brackets
    /// (`editorOverviewRuler.bracketMatchForeground`).
    pub overview_ruler_bracket_match: Hsla,
    /// `minimap.background` (defaults to the editor background).
    pub minimap_background: Option<Hsla>,
    /// `minimapSlider.background` / `.hoverBackground`.
    pub minimap_slider: Hsla,
    /// `minimapSlider.hoverBackground`.
    pub minimap_slider_hover: Hsla,
    /// `editorStickyScroll.background` (defaults to the editor background).
    pub sticky_scroll_background: Option<Hsla>,
    /// A hovered or focused sticky line (`editorStickyScrollHover.background`).
    pub sticky_scroll_hover_background: Hsla,
    /// Keyboard focus outlines (`focusBorder`).
    pub focus_border: Hsla,
    /// Inline edit colors (`inlineEdit.*`).
    pub inline_edit: InlineEditColors,
    /// Symbol icon colors (`symbolIcon.*Foreground`), in the suggest
    /// widget and the quick outline.
    pub symbol_icons: SymbolIconColors,
    /// An editor banner, such as the unicode highlighter's "too many
    /// characters" one (`banner.background`).
    pub banner_background: Hsla,
    /// `banner.foreground`: a banner's text and links.
    pub banner_foreground: Hsla,
    /// `banner.iconForeground`: a banner's icon.
    pub banner_icon_foreground: Hsla,
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

    /// Applies the `symbolIcon.*Foreground` keys, after `foreground` for
    /// the colors that default to it.
    fn apply_vscode_colors(&mut self, get: &dyn Fn(&str) -> Option<Hsla>) {
        if let Some(foreground) = get("foreground") {
            self.set_foreground(foreground);
        }
        for (slot, key) in [
            (&mut self.array, "arrayForeground"),
            (&mut self.boolean, "booleanForeground"),
            (&mut self.class, "classForeground"),
            (&mut self.color, "colorForeground"),
            (&mut self.constant, "constantForeground"),
            (&mut self.constructor, "constructorForeground"),
            (&mut self.enumerator, "enumeratorForeground"),
            (&mut self.enumerator_member, "enumeratorMemberForeground"),
            (&mut self.event, "eventForeground"),
            (&mut self.field, "fieldForeground"),
            (&mut self.file, "fileForeground"),
            (&mut self.folder, "folderForeground"),
            (&mut self.function, "functionForeground"),
            (&mut self.interface, "interfaceForeground"),
            (&mut self.key, "keyForeground"),
            (&mut self.keyword, "keywordForeground"),
            (&mut self.method, "methodForeground"),
            (&mut self.module, "moduleForeground"),
            (&mut self.namespace, "namespaceForeground"),
            (&mut self.null, "nullForeground"),
            (&mut self.number, "numberForeground"),
            (&mut self.object, "objectForeground"),
            (&mut self.operator, "operatorForeground"),
            (&mut self.package, "packageForeground"),
            (&mut self.property, "propertyForeground"),
            (&mut self.reference, "referenceForeground"),
            (&mut self.snippet, "snippetForeground"),
            (&mut self.string, "stringForeground"),
            (&mut self.structure, "structForeground"),
            (&mut self.text, "textForeground"),
            (&mut self.type_parameter, "typeParameterForeground"),
            (&mut self.unit, "unitForeground"),
            (&mut self.variable, "variableForeground"),
        ] {
            if let Some(color) = get(&format!("symbolIcon.{key}")) {
                *slot = color;
            }
        }
    }
}

/// Inline edit (next edit suggestion) colors, `inlineEdit.*`. Each is
/// `None` until a theme sets it, and then derives from other colors as in
/// Monaco; the [`EditorColors`] methods of the same names resolve them.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct InlineEditColors {
    /// `inlineEdit.originalBackground`.
    pub original_background: Option<Hsla>,
    /// `inlineEdit.modifiedBackground`.
    pub modified_background: Option<Hsla>,
    /// `inlineEdit.originalChangedLineBackground`.
    pub original_changed_line_background: Option<Hsla>,
    /// `inlineEdit.originalChangedTextBackground`.
    pub original_changed_text_background: Option<Hsla>,
    /// `inlineEdit.modifiedChangedLineBackground`.
    pub modified_changed_line_background: Option<Hsla>,
    /// `inlineEdit.modifiedChangedTextBackground`.
    pub modified_changed_text_background: Option<Hsla>,
    /// `inlineEdit.originalBorder`.
    pub original_border: Option<Hsla>,
    /// `inlineEdit.modifiedBorder`.
    pub modified_border: Option<Hsla>,
    /// `inlineEdit.gutterIndicator.primaryForeground`, `primaryBorder` and
    /// `primaryBackground`: the indicator when Tab jumps.
    pub gutter_indicator_primary: [Option<Hsla>; 3],
    /// `inlineEdit.gutterIndicator.secondaryForeground`, `secondaryBorder`
    /// and `secondaryBackground`: when Tab does neither.
    pub gutter_indicator_secondary: [Option<Hsla>; 3],
    /// `inlineEdit.gutterIndicator.successfulForeground`,
    /// `successfulBorder` and `successfulBackground`: when Tab accepts.
    pub gutter_indicator_successful: [Option<Hsla>; 3],
}

/// Monaco's `transparent(color, factor)`.
fn transparent(color: Hsla, factor: f32) -> Hsla {
    Hsla {
        a: color.a * factor,
        ..color
    }
}

impl EditorColors {
    /// `inlineEdit.originalBackground` (20% of
    /// `diffEditor.removedTextBackground`).
    pub fn inline_edit_original_background(&self) -> Hsla {
        (self.inline_edit.original_background)
            .unwrap_or_else(|| transparent(self.diff_removed_text_background, 0.2))
    }

    /// `inlineEdit.modifiedBackground` (30% of
    /// `diffEditor.insertedTextBackground`).
    pub fn inline_edit_modified_background(&self) -> Hsla {
        (self.inline_edit.modified_background)
            .unwrap_or_else(|| transparent(self.diff_inserted_text_background, 0.3))
    }

    /// `inlineEdit.originalChangedLineBackground` (80% of
    /// `diffEditor.removedTextBackground`).
    pub fn inline_edit_original_changed_line_background(&self) -> Hsla {
        (self.inline_edit.original_changed_line_background)
            .unwrap_or_else(|| transparent(self.diff_removed_text_background, 0.8))
    }

    /// `inlineEdit.originalChangedTextBackground` (80% of
    /// `diffEditor.removedTextBackground`).
    pub fn inline_edit_original_changed_text_background(&self) -> Hsla {
        (self.inline_edit.original_changed_text_background)
            .unwrap_or_else(|| transparent(self.diff_removed_text_background, 0.8))
    }

    /// `inlineEdit.modifiedChangedLineBackground` (70% of
    /// `diffEditor.insertedLineBackground`).
    pub fn inline_edit_modified_changed_line_background(&self) -> Hsla {
        (self.inline_edit.modified_changed_line_background)
            .unwrap_or_else(|| transparent(self.diff_inserted_line_background, 0.7))
    }

    /// `inlineEdit.modifiedChangedTextBackground` (70% of
    /// `diffEditor.insertedTextBackground`).
    pub fn inline_edit_modified_changed_text_background(&self) -> Hsla {
        (self.inline_edit.modified_changed_text_background)
            .unwrap_or_else(|| transparent(self.diff_inserted_text_background, 0.7))
    }

    /// `inlineEdit.originalBorder` (`diffEditor.removedTextBackground`).
    pub fn inline_edit_original_border(&self) -> Hsla {
        (self.inline_edit.original_border).unwrap_or(self.diff_removed_text_background)
    }

    /// `inlineEdit.modifiedBorder` (`diffEditor.insertedTextBackground`).
    pub fn inline_edit_modified_border(&self) -> Hsla {
        (self.inline_edit.modified_border).unwrap_or(self.diff_inserted_text_background)
    }

    /// The gutter indicator's foreground, border and background when Tab
    /// jumps (button colors, the background at 40%).
    pub fn inline_edit_gutter_indicator_primary(&self) -> [Hsla; 3] {
        let [fg, border, bg] = self.inline_edit.gutter_indicator_primary;
        let border = border.unwrap_or(self.button_background);
        [
            fg.unwrap_or(self.button_foreground),
            border,
            bg.unwrap_or_else(|| transparent(border, 0.4)),
        ]
    }

    /// The gutter indicator's foreground, border and background when Tab
    /// does neither (the hover widget's colors).
    pub fn inline_edit_gutter_indicator_secondary(&self) -> [Hsla; 3] {
        let [fg, border, bg] = self.inline_edit.gutter_indicator_secondary;
        [
            fg.unwrap_or(self.widget_foreground),
            border.unwrap_or(self.widget_border),
            bg.unwrap_or(self.widget_background),
        ]
    }

    /// The gutter indicator's foreground, border and background when Tab
    /// accepts (button colors).
    pub fn inline_edit_gutter_indicator_successful(&self) -> [Hsla; 3] {
        let [fg, border, bg] = self.inline_edit.gutter_indicator_successful;
        let border = border.unwrap_or(self.button_background);
        [
            fg.unwrap_or(self.button_foreground),
            border,
            bg.unwrap_or(border),
        ]
    }
}

fn c(color: Rgba) -> Hsla {
    color.into()
}

impl EditorColors {
    /// Monaco `vs-dark`.
    pub fn vs_dark() -> Self {
        Self {
            background: c(rgb(0x1e1e1e)),
            foreground: c(rgb(0xd4d4d4)),
            line_number: c(rgb(0x858585)),
            active_line_number: c(rgb(0xc6c6c6)),
            line_highlight_background: None,
            line_highlight_border: Some(c(rgb(0x282828))),
            selection_background: c(rgb(0x264f78)),
            selection_foreground: None,
            diff_inserted_text_background: c(rgba(0x9ccc2c33)),
            diff_removed_text_background: c(rgba(0xff000033)),
            diff_inserted_text_border: None,
            diff_removed_text_border: None,
            diff_inserted_line_background: c(rgba(0x9bb95533)),
            diff_removed_line_background: c(rgba(0xff000033)),
            diff_gutter_inserted_line_background: c(rgba(0x9bb95533)),
            diff_gutter_removed_line_background: c(rgba(0xff000033)),
            diff_diagonal_fill: c(rgba(0xcccccc33)),
            diff_unchanged_region_background: c(rgb(0x1e1e1e)),
            diff_unchanged_region_foreground: c(rgb(0xcccccc)),
            diff_unchanged_region_shadow: c(rgba(0x000000ff)),
            diff_unchanged_code_background: c(rgba(0x74747429)),
            gutter_item_background: c(rgb(0x37373d)),
            gutter_item_glyph_foreground: c(rgb(0xd4d4d4)),
            menu_separator_background: c(rgb(0x606060)),
            diff_move_border: c(rgba(0x8b8b8b9c)),
            diff_move_active_border: c(rgb(0xffa500)),
            multi_diff_header_background: c(rgb(0x262626)),
            multi_diff_border: c(rgba(0x00000000)),
            inactive_selection_background: c(rgb(0x3a3d41)),
            cursor: c(rgb(0xaeafad)),
            gutter_background: c(rgb(0x1e1e1e)),
            scrollbar_slider: c(rgba(0x79797966)),
            scrollbar_slider_hover: c(rgba(0x646464b3)),
            scrollbar_slider_active: c(rgba(0xbfbfbf66)),
            scrollbar_shadow: c(rgb(0x000000)),
            overview_ruler_border: c(rgba(0x7f7f7f4d)),
            ime_underline: c(rgb(0xd4d4d4)),
            find_match_background: c(rgb(0x515c6a)),
            find_match_highlight_background: c(rgba(0xea5c0055)),
            find_match_overview: c(rgba(0xd186167e)),
            find_range_highlight_background: c(rgba(0x3a3d4166)),
            error_foreground: c(rgb(0xf48771)),
            symbol_highlight_background: c(rgba(0xea5c0055)),
            link_active_foreground: c(rgb(0x4e94ce)),
            range_highlight_background: c(rgba(0xffffff0b)),
            color_decoration_border: c(rgba(0xeeeeeeff)),
            button_background: c(rgb(0x0e639c)),
            menu_selection_background: c(rgb(0x04395e)),
            menu_selection_foreground: c(rgb(0xffffff)),
            button_foreground: c(rgb(0xffffff)),
            code_lens_foreground: c(rgb(0x999999)),
            linked_editing_background: c(rgba(0xff00004d)),
            ghost_text_foreground: c(rgba(0xffffff56)),
            ghost_text_background: gpui::transparent_black(),
            ghost_text_border: gpui::transparent_black(),
            placeholder_foreground: c(rgba(0xffffff56)),
            unicode_highlight_border: c(rgb(0xcca700)),
            unicode_highlight_background: gpui::transparent_black(),
            message_background: c(rgb(0x063b49)),
            message_border: c(rgb(0x007acc)),
            fold_background: c(rgba(0x264f784d)),
            inlay_hint_foreground: c(rgb(0x969696)),
            inlay_hint_background: c(rgba(0x4d4d4d1a)),
            inlay_hint_type_foreground: c(rgb(0x969696)),
            inlay_hint_type_background: c(rgba(0x4d4d4d1a)),
            inlay_hint_parameter_foreground: c(rgb(0x969696)),
            inlay_hint_parameter_background: c(rgba(0x4d4d4d1a)),
            lightbulb_foreground: c(rgb(0xffcc00)),
            lightbulb_auto_fix_foreground: c(rgb(0x75beff)),
            peek_view_border: c(rgb(0x3794ff)),
            peek_view_title_background: c(rgb(0x252526)),
            peek_view_title_foreground: c(rgb(0xffffff)),
            peek_view_title_description_foreground: c(rgba(0xccccccb3)),
            peek_view_result_background: c(rgb(0x252526)),
            peek_view_result_line_foreground: c(rgb(0xbbbbbb)),
            peek_view_result_file_foreground: c(rgb(0xffffff)),
            peek_view_result_selection_background: c(rgba(0x3399ff33)),
            peek_view_result_selection_foreground: c(rgb(0xffffff)),
            peek_view_result_match_highlight_background: c(rgba(0xea5c004d)),
            peek_view_editor_background: c(rgb(0x001f33)),
            peek_view_editor_match_highlight_background: c(rgba(0xff8f0099)),
            word_highlight_background: c(rgba(0x575757b8)),
            word_highlight_strong_background: c(rgba(0x004972b8)),
            word_highlight_overview: c(rgba(0xa0a0a0cc)),
            word_highlight_strong_overview: c(rgba(0xc0a0c0cc)),
            selection_highlight_background: c(rgba(0xadd6ff26)),
            bracket_highlights: [
                Some(c(rgb(0xffd700))),
                Some(c(rgb(0xda70d6))),
                Some(c(rgb(0x179fff))),
                None,
                None,
                None,
            ],
            unexpected_bracket: c(rgba(0xff1212cc)),
            whitespace: c(rgba(0xe3e4e229)),
            word_wrap_indicator: c(rgba(0xe3e4e229)),
            indent_guide: c(rgb(0x404040)),
            indent_guide_active: c(rgb(0x707070)),
            ruler: c(rgb(0x5a5a5a)),
            unnecessary_code_opacity: c(rgba(0x000000aa)),
            unnecessary_code_border: gpui::transparent_black(),
            selection_highlight_overview: c(rgba(0xa0a0a0cc)),
            widget_background: c(rgb(0x252526)),
            widget_foreground: c(rgb(0xcccccc)),
            widget_border: c(rgb(0x454545)),
            snippet_tabstop_background: c(rgba(0x7c7c7c4d)),
            snippet_tabstop_border: gpui::transparent_black(),
            snippet_final_tabstop_background: gpui::transparent_black(),
            snippet_final_tabstop_border: c(rgb(0x525252)),
            suggest_background: c(rgb(0x252526)),
            suggest_border: c(rgb(0x454545)),
            suggest_foreground: c(rgb(0xd4d4d4)),
            suggest_selected_foreground: c(rgb(0xffffff)),
            suggest_selected_background: c(rgb(0x04395e)),
            suggest_highlight_foreground: c(rgb(0x2aaaff)),
            suggest_focus_highlight_foreground: c(rgb(0x2aaaff)),
            input_background: c(rgb(0x3c3c3c)),
            input_option_active_background: c(rgba(0x007fd466)),
            input_option_active_border: c(rgb(0x007acc)),
            bracket_match_background: c(rgba(0x0064001a)),
            bracket_match_border: c(rgb(0x888888)),
            overview_ruler_bracket_match: c(rgb(0xa0a0a0)),
            minimap_background: None,
            minimap_slider: c(rgba(0x79797933)),
            minimap_slider_hover: c(rgba(0x64646459)),
            sticky_scroll_background: None,
            sticky_scroll_hover_background: c(rgb(0x2a2d2e)),
            focus_border: c(rgb(0x007fd4)),
            inline_edit: InlineEditColors::default(),
            // Monaco's `foreground` is #CCCCCC.
            symbol_icons: SymbolIconColors::new(c(rgb(0xcccccc)), false),
            // `listActiveSelectionBackground`, `listActiveSelectionForeground`
            // and `editorInfo.foreground`.
            banner_background: c(rgb(0x04395e)),
            banner_foreground: c(rgb(0xffffff)),
            banner_icon_foreground: c(rgb(0x3794ff)),
        }
    }

    /// Monaco `vs`.
    pub fn vs() -> Self {
        Self {
            background: c(rgb(0xfffffe)),
            foreground: c(rgb(0x000000)),
            line_number: c(rgb(0x237893)),
            active_line_number: c(rgb(0x0b216f)),
            line_highlight_background: None,
            line_highlight_border: Some(c(rgb(0xeeeeee))),
            selection_background: c(rgb(0xadd6ff)),
            selection_foreground: None,
            diff_inserted_text_background: c(rgba(0x9ccc2c40)),
            diff_removed_text_background: c(rgba(0xff000033)),
            diff_inserted_text_border: None,
            diff_removed_text_border: None,
            diff_inserted_line_background: c(rgba(0x9bb95533)),
            diff_removed_line_background: c(rgba(0xff000033)),
            diff_gutter_inserted_line_background: c(rgba(0x9bb95533)),
            diff_gutter_removed_line_background: c(rgba(0xff000033)),
            diff_diagonal_fill: c(rgba(0x22222233)),
            diff_unchanged_region_background: c(rgb(0xfffffe)),
            diff_unchanged_region_foreground: c(rgb(0x616161)),
            diff_unchanged_region_shadow: c(rgba(0x737373bf)),
            diff_unchanged_code_background: c(rgba(0xb8b8b829)),
            gutter_item_background: c(rgb(0xd4d7e9)),
            gutter_item_glyph_foreground: c(rgb(0x000000)),
            menu_separator_background: c(rgb(0xd4d4d4)),
            diff_move_border: c(rgba(0x8b8b8b9c)),
            diff_move_active_border: c(rgb(0xffa500)),
            multi_diff_header_background: c(rgb(0xececec)),
            multi_diff_border: c(rgba(0xccccccff)),
            inactive_selection_background: c(rgb(0xe5ebf1)),
            cursor: c(rgb(0x000000)),
            gutter_background: c(rgb(0xfffffe)),
            scrollbar_slider: c(rgba(0x64646466)),
            scrollbar_slider_hover: c(rgba(0x646464b3)),
            scrollbar_slider_active: c(rgba(0x00000099)),
            scrollbar_shadow: c(rgb(0xdddddd)),
            overview_ruler_border: c(rgba(0x7f7f7f4d)),
            ime_underline: c(rgb(0x000000)),
            find_match_background: c(rgb(0xa8ac94)),
            find_match_highlight_background: c(rgba(0xea5c0055)),
            find_match_overview: c(rgba(0xd186167e)),
            find_range_highlight_background: c(rgba(0xb4b4b44c)),
            error_foreground: c(rgb(0xa1260d)),
            symbol_highlight_background: c(rgba(0xea5c0055)),
            link_active_foreground: c(rgb(0x0000ff)),
            range_highlight_background: c(rgba(0xfdff0033)),
            color_decoration_border: c(rgba(0x000000ff)),
            button_background: c(rgb(0x007acc)),
            menu_selection_background: c(rgb(0x0060c0)),
            menu_selection_foreground: c(rgb(0xffffff)),
            button_foreground: c(rgb(0xffffff)),
            code_lens_foreground: c(rgb(0x999999)),
            linked_editing_background: c(rgba(0xff00004d)),
            ghost_text_foreground: c(rgba(0x00000077)),
            ghost_text_background: gpui::transparent_black(),
            ghost_text_border: gpui::transparent_black(),
            placeholder_foreground: c(rgba(0x00000077)),
            unicode_highlight_border: c(rgb(0xbf8803)),
            unicode_highlight_background: gpui::transparent_black(),
            message_background: c(rgb(0xd6ecf2)),
            message_border: c(rgb(0x007acc)),
            fold_background: c(rgba(0xadd6ff4d)),
            inlay_hint_foreground: c(rgb(0x969696)),
            inlay_hint_background: c(rgba(0xc4c4c41a)),
            inlay_hint_type_foreground: c(rgb(0x969696)),
            inlay_hint_type_background: c(rgba(0xc4c4c41a)),
            inlay_hint_parameter_foreground: c(rgb(0x969696)),
            inlay_hint_parameter_background: c(rgba(0xc4c4c41a)),
            lightbulb_foreground: c(rgb(0xddb100)),
            lightbulb_auto_fix_foreground: c(rgb(0x007acc)),
            peek_view_border: c(rgb(0x1a85ff)),
            peek_view_title_background: c(rgb(0xf3f3f3)),
            peek_view_title_foreground: c(rgb(0x000000)),
            peek_view_title_description_foreground: c(rgba(0x00616161)),
            peek_view_result_background: c(rgb(0xf3f3f3)),
            peek_view_result_line_foreground: c(rgb(0x646465)),
            peek_view_result_file_foreground: c(rgb(0x1e1e1e)),
            peek_view_result_selection_background: c(rgba(0x3399ff33)),
            peek_view_result_selection_foreground: c(rgb(0x6c6c6c)),
            peek_view_result_match_highlight_background: c(rgba(0xea5c004d)),
            peek_view_editor_background: c(rgb(0xf2f8fc)),
            peek_view_editor_match_highlight_background: c(rgba(0xf5d802de)),
            word_highlight_background: c(rgba(0x57575740)),
            word_highlight_strong_background: c(rgba(0x0e639c40)),
            word_highlight_overview: c(rgba(0xa0a0a0cc)),
            word_highlight_strong_overview: c(rgba(0xc0a0c0cc)),
            selection_highlight_background: c(rgba(0xadd6ff80)),
            bracket_highlights: [
                Some(c(rgb(0x0431fa))),
                Some(c(rgb(0x319331))),
                Some(c(rgb(0x7b3814))),
                None,
                None,
                None,
            ],
            unexpected_bracket: c(rgba(0xff1212cc)),
            whitespace: c(rgba(0x33333333)),
            word_wrap_indicator: c(rgba(0x33333333)),
            indent_guide: c(rgb(0xd3d3d3)),
            indent_guide_active: c(rgb(0x939393)),
            ruler: c(rgb(0xd3d3d3)),
            unnecessary_code_opacity: c(rgba(0x00000077)),
            unnecessary_code_border: gpui::transparent_black(),
            selection_highlight_overview: c(rgba(0xa0a0a0cc)),
            widget_background: c(rgb(0xf3f3f3)),
            widget_foreground: c(rgb(0x616161)),
            widget_border: c(rgb(0xc8c8c8)),
            snippet_tabstop_background: c(rgba(0x0a32641a)),
            snippet_tabstop_border: gpui::transparent_black(),
            snippet_final_tabstop_background: gpui::transparent_black(),
            snippet_final_tabstop_border: c(rgba(0x0a326480)),
            suggest_background: c(rgb(0xf3f3f3)),
            suggest_border: c(rgb(0xc8c8c8)),
            suggest_foreground: c(rgb(0x000000)),
            suggest_selected_foreground: c(rgb(0xffffff)),
            suggest_selected_background: c(rgb(0x0060c0)),
            suggest_highlight_foreground: c(rgb(0x0066bf)),
            suggest_focus_highlight_foreground: c(rgb(0xbbe7ff)),
            input_background: c(rgb(0xffffff)),
            input_option_active_background: c(rgba(0x007fd433)),
            input_option_active_border: c(rgb(0x007acc)),
            bracket_match_background: c(rgba(0x0064001a)),
            bracket_match_border: c(rgb(0xb9b9b9)),
            overview_ruler_bracket_match: c(rgb(0xa0a0a0)),
            minimap_background: None,
            minimap_slider: c(rgba(0x79797933)),
            minimap_slider_hover: c(rgba(0x64646459)),
            sticky_scroll_background: None,
            sticky_scroll_hover_background: c(rgb(0xf0f0f0)),
            focus_border: c(rgb(0x0090f1)),
            inline_edit: InlineEditColors::default(),
            // Monaco's `foreground` is #616161.
            symbol_icons: SymbolIconColors::new(c(rgb(0x616161)), true),
            // `listActiveSelectionBackground`, `listActiveSelectionForeground`
            // and `editorInfo.foreground`.
            banner_background: c(rgb(0x0060c0)),
            banner_foreground: c(rgb(0xffffff)),
            banner_icon_foreground: c(rgb(0x1a85ff)),
        }
    }

    /// Monaco `hc-black`: the theme's own colors plus the `hcDark` registry
    /// defaults. `contrastBorder` is `#6FC3DF` and `contrastActiveBorder`
    /// (`focusBorder`) `#F38518`.
    ///
    /// Monaco outlines find matches, word/selection/symbol highlights and
    /// range highlights with `contrastActiveBorder` instead of filling them
    /// (their `hcDark` backgrounds are `null`); with no border slots here,
    /// those are approximated by translucent `#F38518` fills.
    pub fn hc_black() -> Self {
        // An approximated outline-only highlight (see above).
        let outline_fill = c(rgba(0xf3851833));
        Self {
            background: c(rgb(0x000000)),
            foreground: c(rgb(0xffffff)),
            line_number: c(rgb(0xffffff)),
            active_line_number: c(rgb(0xf38518)),
            line_highlight_background: None,
            line_highlight_border: Some(c(rgb(0xf38518))),
            selection_background: c(rgb(0xf3f518)),
            selection_foreground: Some(c(rgb(0x000000))),
            diff_inserted_text_background: c(rgba(0x00000000)),
            diff_removed_text_background: c(rgba(0x00000000)),
            diff_inserted_text_border: Some(c(rgba(0x33ff2eff))),
            diff_removed_text_border: Some(c(rgba(0xff008fff))),
            diff_inserted_line_background: c(rgba(0x00000000)),
            diff_removed_line_background: c(rgba(0x00000000)),
            diff_gutter_inserted_line_background: c(rgba(0x00000000)),
            diff_gutter_removed_line_background: c(rgba(0x00000000)),
            diff_diagonal_fill: c(rgba(0xcccccc33)),
            diff_unchanged_region_background: c(rgb(0x000000)),
            diff_unchanged_region_foreground: c(rgb(0xffffff)),
            diff_unchanged_region_shadow: c(rgba(0x000000ff)),
            diff_unchanged_code_background: c(rgba(0x00000000)),
            gutter_item_background: c(rgb(0xffffff)),
            gutter_item_glyph_foreground: c(rgb(0x000000)),
            menu_separator_background: c(rgb(0x6fc3df)),
            diff_move_border: c(rgba(0x8b8b8b9c)),
            diff_move_active_border: c(rgb(0xffa500)),
            multi_diff_header_background: c(rgb(0x000000)),
            multi_diff_border: c(rgba(0x6fc3dfff)),
            inactive_selection_background: c(rgba(0xf3f518b3)),
            cursor: c(rgb(0xffffff)),
            gutter_background: c(rgb(0x000000)),
            scrollbar_slider: c(rgba(0x6fc3df99)),
            scrollbar_slider_hover: c(rgba(0x6fc3dfcc)),
            scrollbar_slider_active: c(rgb(0x6fc3df)),
            scrollbar_shadow: gpui::transparent_black(),
            overview_ruler_border: c(rgba(0x7f7f7f4d)),
            ime_underline: c(rgb(0xffffff)),
            find_match_background: c(rgba(0xf3851866)),
            find_match_highlight_background: outline_fill,
            find_match_overview: c(rgb(0xab5a00)),
            find_range_highlight_background: gpui::transparent_black(),
            error_foreground: c(rgb(0xf48771)),
            symbol_highlight_background: outline_fill,
            link_active_foreground: c(rgb(0x00ffff)),
            range_highlight_background: outline_fill,
            color_decoration_border: c(rgba(0xeeeeeeff)),
            // `menu.selectionBackground` is `null` (outlined); approximated.
            menu_selection_background: outline_fill,
            menu_selection_foreground: c(rgb(0xffffff)),
            // `button.background` is `null` (a bordered button); hc-light's
            // fill keeps it visible.
            button_background: c(rgb(0x0f4a85)),
            button_foreground: c(rgb(0xffffff)),
            code_lens_foreground: c(rgb(0x999999)),
            linked_editing_background: c(rgba(0xff00004d)),
            // `editorGhostText.foreground` is `null`: ghost text keeps the
            // text color and gets a border instead.
            ghost_text_foreground: c(rgb(0xffffff)),
            ghost_text_background: gpui::transparent_black(),
            ghost_text_border: c(rgba(0xffffffcc)),
            placeholder_foreground: c(rgb(0xffffff)),
            unicode_highlight_border: c(rgb(0xffd370)),
            unicode_highlight_background: gpui::transparent_black(),
            message_background: c(rgb(0x000000)),
            message_border: c(rgb(0x6fc3df)),
            fold_background: gpui::transparent_black(),
            inlay_hint_foreground: c(rgb(0xffffff)),
            inlay_hint_background: c(rgba(0xffffff1a)),
            inlay_hint_type_foreground: c(rgb(0xffffff)),
            inlay_hint_type_background: c(rgba(0xffffff1a)),
            inlay_hint_parameter_foreground: c(rgb(0xffffff)),
            inlay_hint_parameter_background: c(rgba(0xffffff1a)),
            lightbulb_foreground: c(rgb(0xffcc00)),
            lightbulb_auto_fix_foreground: c(rgb(0x75beff)),
            peek_view_border: c(rgb(0x6fc3df)),
            peek_view_title_background: c(rgb(0x000000)),
            peek_view_title_foreground: c(rgb(0xffffff)),
            peek_view_title_description_foreground: c(rgba(0xffffff99)),
            peek_view_result_background: c(rgb(0x000000)),
            peek_view_result_line_foreground: c(rgb(0xffffff)),
            peek_view_result_file_foreground: c(rgb(0xffffff)),
            peek_view_result_selection_background: outline_fill,
            peek_view_result_selection_foreground: c(rgb(0xffffff)),
            peek_view_result_match_highlight_background: outline_fill,
            peek_view_editor_background: c(rgb(0x000000)),
            peek_view_editor_match_highlight_background: outline_fill,
            word_highlight_background: outline_fill,
            word_highlight_strong_background: c(rgba(0xf385184d)),
            word_highlight_overview: c(rgba(0xa0a0a0cc)),
            word_highlight_strong_overview: c(rgba(0xc0a0c0cc)),
            selection_highlight_background: outline_fill,
            bracket_highlights: [
                Some(c(rgb(0xffd700))),
                Some(c(rgb(0xda70d6))),
                Some(c(rgb(0x87cefa))),
                None,
                None,
                None,
            ],
            unexpected_bracket: c(rgb(0xff3232)),
            whitespace: c(rgba(0xe3e4e229)),
            word_wrap_indicator: c(rgba(0xe3e4e229)),
            indent_guide: c(rgb(0xffffff)),
            indent_guide_active: c(rgb(0xffffff)),
            ruler: c(rgb(0xffffff)),
            // `editorUnnecessaryCode.opacity` is `null`: unused code is
            // underlined instead of faded.
            unnecessary_code_opacity: c(rgba(0x000000ff)),
            unnecessary_code_border: c(rgba(0xffffffcc)),
            selection_highlight_overview: c(rgba(0xa0a0a0cc)),
            widget_background: c(rgb(0x0c141f)),
            widget_foreground: c(rgb(0xffffff)),
            widget_border: c(rgb(0x6fc3df)),
            snippet_tabstop_background: c(rgba(0x7c7c7c4d)),
            snippet_tabstop_border: gpui::transparent_black(),
            snippet_final_tabstop_background: gpui::transparent_black(),
            snippet_final_tabstop_border: c(rgb(0x525252)),
            suggest_background: c(rgb(0x0c141f)),
            suggest_border: c(rgb(0x6fc3df)),
            suggest_foreground: c(rgb(0xffffff)),
            // The focused row is inverted.
            suggest_selected_foreground: c(rgb(0x0c141f)),
            suggest_selected_background: c(rgb(0xffffff)),
            suggest_highlight_foreground: c(rgb(0xf38518)),
            suggest_focus_highlight_foreground: c(rgb(0x0c141f)),
            input_background: c(rgb(0x000000)),
            input_option_active_background: gpui::transparent_black(),
            input_option_active_border: c(rgb(0x6fc3df)),
            bracket_match_background: c(rgba(0x0064001a)),
            bracket_match_border: c(rgb(0x6fc3df)),
            overview_ruler_bracket_match: c(rgb(0xa0a0a0)),
            minimap_background: None,
            minimap_slider: c(rgba(0x6fc3df4d)),
            minimap_slider_hover: c(rgba(0x6fc3df66)),
            sticky_scroll_background: None,
            // `editorStickyScrollHover.background` is `null`.
            sticky_scroll_hover_background: gpui::transparent_black(),
            focus_border: c(rgb(0xf38518)),
            inline_edit: InlineEditColors::default(),
            // Monaco's `foreground` is #FFFFFF.
            symbol_icons: SymbolIconColors::new(c(rgb(0xffffff)), false),
            // The list selection colors are `null` in high contrast.
            banner_background: gpui::transparent_black(),
            banner_foreground: c(rgb(0xffffff)),
            banner_icon_foreground: c(rgb(0x3794ff)),
        }
    }

    /// Monaco `hc-light`: the theme's own colors plus the `hcLight` registry
    /// defaults. `contrastBorder` is `#0F4A85` and `contrastActiveBorder`
    /// (`focusBorder`) `#006BBD`.
    ///
    /// As in [`EditorColors::hc_black`], highlights Monaco only outlines are
    /// approximated by translucent `#006BBD` fills.
    pub fn hc_light() -> Self {
        // An approximated outline-only highlight (see `hc_black`).
        let outline_fill = c(rgba(0x006bbd33));
        Self {
            background: c(rgb(0xffffff)),
            foreground: c(rgb(0x292929)),
            line_number: c(rgb(0x292929)),
            active_line_number: c(rgb(0x006bbd)),
            line_highlight_background: None,
            line_highlight_border: Some(c(rgb(0x0f4a85))),
            selection_background: c(rgb(0x0f4a85)),
            selection_foreground: Some(c(rgb(0xffffff))),
            diff_inserted_text_background: c(rgba(0x00000000)),
            diff_removed_text_background: c(rgba(0x00000000)),
            diff_inserted_text_border: Some(c(rgba(0x374e06ff))),
            diff_removed_text_border: Some(c(rgba(0xad0707ff))),
            diff_inserted_line_background: c(rgba(0x00000000)),
            diff_removed_line_background: c(rgba(0x00000000)),
            diff_gutter_inserted_line_background: c(rgba(0x00000000)),
            diff_gutter_removed_line_background: c(rgba(0x00000000)),
            diff_diagonal_fill: c(rgba(0x22222233)),
            diff_unchanged_region_background: c(rgb(0xffffff)),
            diff_unchanged_region_foreground: c(rgb(0x292929)),
            diff_unchanged_region_shadow: c(rgba(0x737373bf)),
            diff_unchanged_code_background: c(rgba(0x00000000)),
            gutter_item_background: c(rgb(0x000000)),
            gutter_item_glyph_foreground: c(rgb(0xffffff)),
            menu_separator_background: c(rgb(0x0f4a85)),
            diff_move_border: c(rgba(0x8b8b8b9c)),
            diff_move_active_border: c(rgb(0xffa500)),
            multi_diff_header_background: c(rgb(0xffffff)),
            multi_diff_border: c(rgba(0x0f4a85ff)),
            inactive_selection_background: c(rgba(0x0f4a8580)),
            cursor: c(rgb(0x0f4a85)),
            gutter_background: c(rgb(0xffffff)),
            scrollbar_slider: c(rgba(0x0f4a8566)),
            scrollbar_slider_hover: c(rgba(0x0f4a85cc)),
            scrollbar_slider_active: c(rgb(0x0f4a85)),
            scrollbar_shadow: gpui::transparent_black(),
            overview_ruler_border: c(rgb(0x666666)),
            ime_underline: c(rgb(0x292929)),
            find_match_background: c(rgba(0x006bbd66)),
            find_match_highlight_background: outline_fill,
            find_match_overview: c(rgb(0xab5a00)),
            find_range_highlight_background: gpui::transparent_black(),
            error_foreground: c(rgb(0xb5200d)),
            symbol_highlight_background: outline_fill,
            link_active_foreground: c(rgb(0x292929)),
            range_highlight_background: outline_fill,
            color_decoration_border: c(rgba(0x000000ff)),
            menu_selection_background: c(rgba(0x0f4a851a)),
            menu_selection_foreground: c(rgb(0x292929)),
            button_background: c(rgb(0x0f4a85)),
            button_foreground: c(rgb(0xffffff)),
            code_lens_foreground: c(rgb(0x292929)),
            linked_editing_background: c(rgb(0xffffff)),
            // `editorGhostText.foreground` is `null`: ghost text keeps the
            // text color and gets a border instead.
            ghost_text_foreground: c(rgb(0x292929)),
            ghost_text_background: gpui::transparent_black(),
            ghost_text_border: c(rgba(0x292929cc)),
            placeholder_foreground: c(rgb(0x292929)),
            unicode_highlight_border: c(rgb(0x895503)),
            unicode_highlight_background: gpui::transparent_black(),
            message_background: c(rgb(0xffffff)),
            message_border: c(rgb(0x0f4a85)),
            fold_background: gpui::transparent_black(),
            inlay_hint_foreground: c(rgb(0x000000)),
            inlay_hint_background: c(rgba(0x0f4a851a)),
            inlay_hint_type_foreground: c(rgb(0x000000)),
            inlay_hint_type_background: c(rgba(0x0f4a851a)),
            inlay_hint_parameter_foreground: c(rgb(0x000000)),
            inlay_hint_parameter_background: c(rgba(0x0f4a851a)),
            lightbulb_foreground: c(rgb(0x007acc)),
            lightbulb_auto_fix_foreground: c(rgb(0x007acc)),
            peek_view_border: c(rgb(0x0f4a85)),
            peek_view_title_background: c(rgb(0xffffff)),
            peek_view_title_foreground: c(rgb(0x292929)),
            peek_view_title_description_foreground: c(rgb(0x292929)),
            peek_view_result_background: c(rgb(0xffffff)),
            peek_view_result_line_foreground: c(rgb(0x292929)),
            peek_view_result_file_foreground: c(rgb(0x292929)),
            peek_view_result_selection_background: outline_fill,
            peek_view_result_selection_foreground: c(rgb(0x292929)),
            peek_view_result_match_highlight_background: outline_fill,
            peek_view_editor_background: c(rgb(0xffffff)),
            peek_view_editor_match_highlight_background: outline_fill,
            word_highlight_background: outline_fill,
            word_highlight_strong_background: c(rgba(0x006bbd4d)),
            word_highlight_overview: c(rgba(0xa0a0a0cc)),
            word_highlight_strong_overview: c(rgba(0xc0a0c0cc)),
            selection_highlight_background: outline_fill,
            bracket_highlights: [
                Some(c(rgb(0x0431fa))),
                Some(c(rgb(0x319331))),
                Some(c(rgb(0x7b3814))),
                None,
                None,
                None,
            ],
            unexpected_bracket: c(rgb(0xb5200d)),
            whitespace: c(rgb(0xcccccc)),
            word_wrap_indicator: c(rgb(0xcccccc)),
            indent_guide: c(rgb(0x292929)),
            indent_guide_active: c(rgb(0x292929)),
            ruler: c(rgb(0x292929)),
            // `editorUnnecessaryCode.opacity` is `null`: unused code is
            // underlined instead of faded.
            unnecessary_code_opacity: c(rgba(0x000000ff)),
            unnecessary_code_border: c(rgb(0x0f4a85)),
            selection_highlight_overview: c(rgba(0xa0a0a0cc)),
            widget_background: c(rgb(0xffffff)),
            widget_foreground: c(rgb(0x292929)),
            widget_border: c(rgb(0x0f4a85)),
            snippet_tabstop_background: c(rgba(0x0a326433)),
            snippet_tabstop_border: gpui::transparent_black(),
            snippet_final_tabstop_background: gpui::transparent_black(),
            snippet_final_tabstop_border: c(rgb(0x292929)),
            suggest_background: c(rgb(0xffffff)),
            suggest_border: c(rgb(0x0f4a85)),
            suggest_foreground: c(rgb(0x292929)),
            // The focused row is inverted.
            suggest_selected_foreground: c(rgb(0xffffff)),
            suggest_selected_background: c(rgb(0x292929)),
            suggest_highlight_foreground: c(rgb(0x006bbd)),
            suggest_focus_highlight_foreground: c(rgb(0xffffff)),
            input_background: c(rgb(0xffffff)),
            input_option_active_background: gpui::transparent_black(),
            input_option_active_border: c(rgb(0x0f4a85)),
            bracket_match_background: gpui::transparent_black(),
            bracket_match_border: c(rgb(0x0f4a85)),
            overview_ruler_bracket_match: c(rgb(0xa0a0a0)),
            minimap_background: None,
            minimap_slider: c(rgba(0x0f4a8533)),
            minimap_slider_hover: c(rgba(0x0f4a8566)),
            sticky_scroll_background: None,
            sticky_scroll_hover_background: c(rgba(0x0f4a851a)),
            focus_border: c(rgb(0x006bbd)),
            inline_edit: InlineEditColors::default(),
            // Monaco's `foreground` is #292929.
            symbol_icons: SymbolIconColors::new(c(rgb(0x292929)), true),
            // The list selection colors are `null` in high contrast.
            banner_background: gpui::transparent_black(),
            banner_foreground: c(rgb(0x292929)),
            banner_icon_foreground: c(rgb(0x1a85ff)),
        }
    }

    /// Applies VS Code workbench color keys (`editor.background`, ...).
    fn apply_vscode_colors(&mut self, colors: &HashMap<String, String>) {
        let get = |key: &str| colors.get(key).and_then(|value| parse_hex_color(value));
        let set = |slot: &mut Hsla, key: &str| {
            if let Some(color) = get(key) {
                *slot = color;
            }
        };
        let previous_background = self.background;
        set(&mut self.background, "editor.background");
        set(&mut self.foreground, "editor.foreground");
        set(&mut self.line_number, "editorLineNumber.foreground");
        set(
            &mut self.active_line_number,
            "editorLineNumber.activeForeground",
        );
        set(&mut self.selection_background, "editor.selectionBackground");
        set(
            &mut self.diff_inserted_text_background,
            "diffEditor.insertedTextBackground",
        );
        set(
            &mut self.diff_removed_text_background,
            "diffEditor.removedTextBackground",
        );
        if let Some(color) = get("diffEditor.insertedTextBorder") {
            self.diff_inserted_text_border = Some(color);
        }
        if let Some(color) = get("diffEditor.removedTextBorder") {
            self.diff_removed_text_border = Some(color);
        }
        set(
            &mut self.diff_inserted_line_background,
            "diffEditor.insertedLineBackground",
        );
        set(
            &mut self.diff_removed_line_background,
            "diffEditor.removedLineBackground",
        );
        // The gutter defaults to the line backgrounds.
        self.diff_gutter_inserted_line_background = get("diffEditorGutter.insertedLineBackground")
            .unwrap_or(self.diff_inserted_line_background);
        self.diff_gutter_removed_line_background = get("diffEditorGutter.removedLineBackground")
            .unwrap_or(self.diff_removed_line_background);
        set(&mut self.diff_diagonal_fill, "diffEditor.diagonalFill");
        // The bars default to the editor's background and foreground.
        self.diff_unchanged_region_background = get("diffEditor.unchangedRegionBackground")
            .or_else(|| get("sideBySideEditor.background"))
            .or_else(|| get("editor.background"))
            .unwrap_or(self.diff_unchanged_region_background);
        self.diff_unchanged_region_foreground = get("diffEditor.unchangedRegionForeground")
            .or_else(|| get("foreground"))
            .unwrap_or(self.diff_unchanged_region_foreground);
        set(
            &mut self.diff_unchanged_region_shadow,
            "diffEditor.unchangedRegionShadow",
        );
        set(
            &mut self.diff_unchanged_code_background,
            "diffEditor.unchangedCodeBackground",
        );
        set(
            &mut self.gutter_item_background,
            "editorGutter.itemBackground",
        );
        set(
            &mut self.gutter_item_glyph_foreground,
            "editorGutter.itemGlyphForeground",
        );
        set(
            &mut self.menu_separator_background,
            "menu.separatorBackground",
        );
        if let Some(color) = get("editor.selectionForeground") {
            self.selection_foreground = Some(color);
        }
        set(
            &mut self.inactive_selection_background,
            "editor.inactiveSelectionBackground",
        );
        set(&mut self.cursor, "editorCursor.foreground");
        set(
            &mut self.ghost_text_foreground,
            "editorGhostText.foreground",
        );
        set(
            &mut self.ghost_text_background,
            "editorGhostText.background",
        );
        set(&mut self.ghost_text_border, "editorGhostText.border");
        let inline_edit = &mut self.inline_edit;
        for (slot, key) in [
            (
                &mut inline_edit.original_background,
                "inlineEdit.originalBackground",
            ),
            (
                &mut inline_edit.modified_background,
                "inlineEdit.modifiedBackground",
            ),
            (
                &mut inline_edit.original_changed_line_background,
                "inlineEdit.originalChangedLineBackground",
            ),
            (
                &mut inline_edit.original_changed_text_background,
                "inlineEdit.originalChangedTextBackground",
            ),
            (
                &mut inline_edit.modified_changed_line_background,
                "inlineEdit.modifiedChangedLineBackground",
            ),
            (
                &mut inline_edit.modified_changed_text_background,
                "inlineEdit.modifiedChangedTextBackground",
            ),
            (
                &mut inline_edit.original_border,
                "inlineEdit.originalBorder",
            ),
            (
                &mut inline_edit.modified_border,
                "inlineEdit.modifiedBorder",
            ),
        ] {
            if let Some(color) = get(key) {
                *slot = Some(color);
            }
        }
        for (slots, kind) in [
            (&mut inline_edit.gutter_indicator_primary, "primary"),
            (&mut inline_edit.gutter_indicator_secondary, "secondary"),
            (&mut inline_edit.gutter_indicator_successful, "successful"),
        ] {
            for (slot, part) in slots.iter_mut().zip(["Foreground", "Border", "Background"]) {
                if let Some(color) = get(&format!("inlineEdit.gutterIndicator.{kind}{part}")) {
                    *slot = Some(color);
                }
            }
        }
        self.symbol_icons.apply_vscode_colors(&get);
        set(&mut self.banner_background, "banner.background");
        set(&mut self.banner_foreground, "banner.foreground");
        set(&mut self.banner_icon_foreground, "banner.iconForeground");
        self.unicode_highlight_border = get("editorUnicodeHighlight.border")
            .or_else(|| get("editorWarning.foreground"))
            .unwrap_or(self.unicode_highlight_border);
        self.unicode_highlight_background = get("editorUnicodeHighlight.background")
            .or_else(|| get("editorWarning.background"))
            .unwrap_or(self.unicode_highlight_background);
        self.placeholder_foreground = get("editor.placeholder.foreground")
            .or_else(|| get("editorGhostText.foreground"))
            .unwrap_or(self.placeholder_foreground);
        set(&mut self.scrollbar_slider, "scrollbarSlider.background");
        set(
            &mut self.scrollbar_slider_hover,
            "scrollbarSlider.hoverBackground",
        );
        set(
            &mut self.scrollbar_slider_active,
            "scrollbarSlider.activeBackground",
        );
        set(&mut self.scrollbar_shadow, "scrollbar.shadow");
        set(
            &mut self.overview_ruler_border,
            "editorOverviewRuler.border",
        );
        set(
            &mut self.find_match_background,
            "editor.findMatchBackground",
        );
        set(
            &mut self.find_match_highlight_background,
            "editor.findMatchHighlightBackground",
        );
        set(
            &mut self.find_range_highlight_background,
            "editor.findRangeHighlightBackground",
        );
        set(&mut self.error_foreground, "errorForeground");
        set(
            &mut self.sticky_scroll_hover_background,
            "editorStickyScrollHover.background",
        );
        set(&mut self.focus_border, "focusBorder");
        set(
            &mut self.find_match_overview,
            "editorOverviewRuler.findMatchForeground",
        );
        self.symbol_highlight_background = self.find_match_highlight_background;
        set(
            &mut self.symbol_highlight_background,
            "editor.symbolHighlightBackground",
        );
        set(
            &mut self.message_background,
            "inputValidation.infoBackground",
        );
        set(&mut self.message_border, "inputValidation.infoBorder");
        set(&mut self.fold_background, "editor.foldBackground");
        set(
            &mut self.link_active_foreground,
            "editorLink.activeForeground",
        );
        set(
            &mut self.inlay_hint_foreground,
            "editorInlayHint.foreground",
        );
        set(
            &mut self.inlay_hint_background,
            "editorInlayHint.background",
        );
        // The kind colors default to the general ones.
        self.inlay_hint_type_foreground = self.inlay_hint_foreground;
        self.inlay_hint_type_background = self.inlay_hint_background;
        self.inlay_hint_parameter_foreground = self.inlay_hint_foreground;
        self.inlay_hint_parameter_background = self.inlay_hint_background;
        set(
            &mut self.inlay_hint_type_foreground,
            "editorInlayHint.typeForeground",
        );
        set(
            &mut self.inlay_hint_type_background,
            "editorInlayHint.typeBackground",
        );
        set(
            &mut self.inlay_hint_parameter_foreground,
            "editorInlayHint.parameterForeground",
        );
        set(
            &mut self.inlay_hint_parameter_background,
            "editorInlayHint.parameterBackground",
        );
        set(&mut self.lightbulb_foreground, "editorLightBulb.foreground");
        set(
            &mut self.lightbulb_auto_fix_foreground,
            "editorLightBulbAutoFix.foreground",
        );
        set(&mut self.peek_view_border, "peekView.border");
        set(
            &mut self.peek_view_title_background,
            "peekViewTitle.background",
        );
        set(
            &mut self.peek_view_title_foreground,
            "peekViewTitleLabel.foreground",
        );
        set(
            &mut self.peek_view_title_description_foreground,
            "peekViewTitleDescription.foreground",
        );
        set(
            &mut self.peek_view_result_background,
            "peekViewResult.background",
        );
        set(
            &mut self.peek_view_result_line_foreground,
            "peekViewResult.lineForeground",
        );
        set(
            &mut self.peek_view_result_file_foreground,
            "peekViewResult.fileForeground",
        );
        set(
            &mut self.peek_view_result_selection_background,
            "peekViewResult.selectionBackground",
        );
        set(
            &mut self.peek_view_result_selection_foreground,
            "peekViewResult.selectionForeground",
        );
        set(
            &mut self.peek_view_result_match_highlight_background,
            "peekViewResult.matchHighlightBackground",
        );
        set(
            &mut self.peek_view_editor_background,
            "peekViewEditor.background",
        );
        set(
            &mut self.peek_view_editor_match_highlight_background,
            "peekViewEditor.matchHighlightBackground",
        );
        set(
            &mut self.word_highlight_background,
            "editor.wordHighlightBackground",
        );
        set(
            &mut self.word_highlight_strong_background,
            "editor.wordHighlightStrongBackground",
        );
        set(
            &mut self.word_highlight_overview,
            "editorOverviewRuler.wordHighlightForeground",
        );
        set(
            &mut self.word_highlight_strong_overview,
            "editorOverviewRuler.wordHighlightStrongForeground",
        );
        set(
            &mut self.selection_highlight_background,
            "editor.selectionHighlightBackground",
        );
        set(
            &mut self.selection_highlight_overview,
            "editorOverviewRuler.selectionHighlightForeground",
        );
        for (i, slot) in self.bracket_highlights.iter_mut().enumerate() {
            if let Some(color) = get(&format!("editorBracketHighlight.foreground{}", i + 1)) {
                *slot = (color.a > 0.).then_some(color);
            }
        }
        set(
            &mut self.unexpected_bracket,
            "editorBracketHighlight.unexpectedBracket.foreground",
        );
        set(&mut self.whitespace, "editorWhitespace.foreground");
        self.word_wrap_indicator = get("editorWordWrapIndicator.foreground")
            .or_else(|| get("editorWhitespace.foreground"))
            .unwrap_or(self.word_wrap_indicator);
        set(&mut self.indent_guide, "editorIndentGuide.background");
        set(&mut self.indent_guide, "editorIndentGuide.background1");
        set(
            &mut self.indent_guide_active,
            "editorIndentGuide.activeBackground",
        );
        set(
            &mut self.indent_guide_active,
            "editorIndentGuide.activeBackground1",
        );
        set(&mut self.ruler, "editorRuler.foreground");
        set(
            &mut self.unnecessary_code_opacity,
            "editorUnnecessaryCode.opacity",
        );
        set(
            &mut self.unnecessary_code_border,
            "editorUnnecessaryCode.border",
        );
        set(&mut self.widget_background, "editorWidget.background");
        set(&mut self.widget_foreground, "editorWidget.foreground");
        set(&mut self.widget_border, "editorWidget.border");
        set(
            &mut self.snippet_tabstop_background,
            "editor.snippetTabstopHighlightBackground",
        );
        set(
            &mut self.snippet_tabstop_border,
            "editor.snippetTabstopHighlightBorder",
        );
        set(
            &mut self.snippet_final_tabstop_background,
            "editor.snippetFinalTabstopHighlightBackground",
        );
        set(
            &mut self.snippet_final_tabstop_border,
            "editor.snippetFinalTabstopHighlightBorder",
        );
        self.suggest_background = self.widget_background;
        self.suggest_border = self.widget_border;
        self.suggest_foreground = self.foreground;
        set(
            &mut self.suggest_background,
            "editorSuggestWidget.background",
        );
        set(&mut self.suggest_border, "editorSuggestWidget.border");
        set(
            &mut self.suggest_foreground,
            "editorSuggestWidget.foreground",
        );
        set(
            &mut self.suggest_selected_foreground,
            "quickInputList.focusForeground",
        );
        set(
            &mut self.suggest_selected_foreground,
            "editorSuggestWidget.selectedForeground",
        );
        set(
            &mut self.suggest_selected_background,
            "quickInputList.focusBackground",
        );
        set(
            &mut self.suggest_selected_background,
            "editorSuggestWidget.selectedBackground",
        );
        set(
            &mut self.suggest_highlight_foreground,
            "list.highlightForeground",
        );
        set(
            &mut self.suggest_highlight_foreground,
            "editorSuggestWidget.highlightForeground",
        );
        set(
            &mut self.suggest_focus_highlight_foreground,
            "list.focusHighlightForeground",
        );
        set(
            &mut self.suggest_focus_highlight_foreground,
            "editorSuggestWidget.focusHighlightForeground",
        );
        set(&mut self.input_background, "input.background");
        set(&mut self.button_background, "button.background");
        set(
            &mut self.menu_selection_background,
            "menu.selectionBackground",
        );
        set(
            &mut self.menu_selection_foreground,
            "menu.selectionForeground",
        );
        set(&mut self.button_foreground, "button.foreground");
        set(
            &mut self.input_option_active_background,
            "inputOption.activeBackground",
        );
        set(
            &mut self.input_option_active_border,
            "inputOption.activeBorder",
        );
        set(
            &mut self.bracket_match_background,
            "editorBracketMatch.background",
        );
        set(&mut self.bracket_match_border, "editorBracketMatch.border");
        set(
            &mut self.overview_ruler_bracket_match,
            "editorOverviewRuler.bracketMatchForeground",
        );
        set(&mut self.minimap_slider, "minimapSlider.background");
        set(
            &mut self.minimap_slider_hover,
            "minimapSlider.hoverBackground",
        );
        if let Some(color) = get("minimap.background") {
            self.minimap_background = Some(color);
        }
        if let Some(color) = get("editorStickyScroll.background") {
            self.sticky_scroll_background = Some(color);
        }
        // The gutter defaults to the editor background (and keeps following
        // it unless a base theme gave it its own color).
        match get("editorGutter.background") {
            Some(color) => self.gutter_background = color,
            None if self.gutter_background == previous_background => {
                self.gutter_background = self.background
            }
            None => {}
        }
        if let Some(color) = get("editor.lineHighlightBackground") {
            self.line_highlight_background = Some(color);
        }
        if let Some(color) = get("editor.lineHighlightBorder") {
            self.line_highlight_border = Some(color);
        } else if self.line_highlight_background.is_some() {
            self.line_highlight_border = None;
        }
        self.ime_underline = self.foreground;
    }
}

impl EditorColors {
    /// The color for brackets at nesting `level` (Monaco cycles through the
    /// non-transparent `editorBracketHighlight.foreground*` colors).
    pub fn bracket_color(&self, level: u32) -> Option<Hsla> {
        let colors: Vec<Hsla> = self.bracket_highlights.iter().flatten().copied().collect();
        (!colors.is_empty()).then(|| colors[level as usize % colors.len()])
    }
}

impl Default for EditorColors {
    fn default() -> Self {
        Self::vs_dark()
    }
}

/// `#RGB`, `#RGBA`, `#RRGGBB` or `#RRGGBBAA`.
pub fn parse_hex_color(value: &str) -> Option<Hsla> {
    let hex = value.trim().strip_prefix('#')?;
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

impl TokenStyle {
    fn from_font_style(foreground: Option<Hsla>, font_style: Option<&str>) -> Self {
        let font_style = font_style.unwrap_or_default();
        Self {
            foreground,
            bold: font_style.contains("bold"),
            italic: font_style.contains("italic"),
            underline: font_style.contains("underline"),
            strikethrough: font_style.contains("strikethrough"),
            opacity: None,
        }
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

/// A theme rule: tokens matching `selector` get `style`.
#[derive(Clone, Debug, PartialEq)]
pub struct TokenRule {
    /// Which vocabulary `selector` is in.
    pub kind: RuleKind,
    /// A dotted token name or scope; matches itself and names extending it with `.`
    /// (`keyword` matches `keyword.control`). The longest match wins, later rules on ties.
    pub selector: String,
    /// The style for matching tokens.
    pub style: TokenStyle,
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
    cache: Mutex<HashMap<HighlightId, TokenStyle>>,
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
            style: TokenStyle::from_font_style(
                if $color.is_empty() { None } else { parse_hex_color(concat!("#", $color)) },
                None $(.or(Some($style)))?,
            ),
        }),*]
    };
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
            rules,
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

    /// Monaco's `vs-dark`.
    pub fn vs_dark() -> Self {
        let rules = monaco_rules! {
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
            "meta.tag" => "CE9178";
            "metatag" => "DD6A6F";
            "key" => "9CDCFE";
            "string.key.json" => "9CDCFE";
            "string.value.json" => "CE9178";
            "attribute.name" => "9CDCFE";
            "attribute.value" => "CE9178";
            "string" => "CE9178";
            "keyword" => "569CD6";
            "keyword.flow" => "C586C0";
            "keyword.json" => "CE9178";
        };
        Self::new("vs-dark", ThemeKind::Dark, EditorColors::vs_dark(), rules)
    }

    /// Monaco's `vs`.
    pub fn vs() -> Self {
        let rules = monaco_rules! {
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
            "metatag" => "e00000";
            "key" => "863B00";
            "string.key.json" => "A31515";
            "string.value.json" => "0451A5";
            "attribute.name" => "FF0000";
            "attribute.value" => "0451A5";
            "string" => "A31515";
            "keyword" => "0000FF";
            "keyword.json" => "0451A5";
            "keyword.flow" => "AF00DB";
        };
        Self::new("vs", ThemeKind::Light, EditorColors::vs(), rules)
    }

    /// Monaco's `hc-black` (high contrast dark). [`ThemeKind`] has no
    /// high-contrast variant, so its kind is [`ThemeKind::Dark`].
    pub fn hc_black() -> Self {
        let rules = monaco_rules! {
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
            "meta" => "D4D4D4";
            "meta.tag" => "CE9178";
            "metatag" => "569CD6";
            "key" => "9CDCFE";
            "string.key" => "9CDCFE";
            "string.value" => "CE9178";
            "attribute.name" => "569CD6";
            "attribute.value" => "3FF23F";
            "string" => "CE9178";
            "keyword" => "569CD6";
            "keyword.flow" => "C586C0";
        };
        let mut theme = Self::new("hc-black", ThemeKind::Dark, EditorColors::hc_black(), rules);
        theme.high_contrast = true;
        theme
    }

    /// Monaco's `hc-light` (high contrast light). [`ThemeKind`] has no
    /// high-contrast variant, so its kind is [`ThemeKind::Light`].
    pub fn hc_light() -> Self {
        let rules = monaco_rules! {
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
            "metatag" => "e00000";
            "key" => "863B00";
            "string.key.json" => "A31515";
            "string.value.json" => "0451A5";
            "attribute.name" => "264F78";
            "attribute.value" => "0451A5";
            "string" => "A31515";
            "keyword" => "0000FF";
            "keyword.flow" => "AF00DB";
        };
        let mut theme = Self::new(
            "hc-light",
            ThemeKind::Light,
            EditorColors::hc_light(),
            rules,
        );
        theme.high_contrast = true;
        theme
    }

    /// Monaco `defineTheme`: `base` plus extra Monaco rules and VS Code color
    /// keys. With `inherit: false` the base's token rules are dropped.
    pub fn define(
        name: impl Into<String>,
        base: &Theme,
        inherit: bool,
        rules: Vec<TokenRule>,
        colors: &HashMap<String, String>,
    ) -> Self {
        let mut all_rules = if inherit {
            base.rules.clone()
        } else {
            Vec::new()
        };
        all_rules.extend(rules);
        let mut editor_colors = base.colors.clone();
        editor_colors.apply_vscode_colors(colors);
        let mut theme = Self::new(name, base.kind, editor_colors, all_rules);
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
    pub fn from_vscode_json_with_includes(
        json: &str,
        resolve: &dyn Fn(&str) -> Option<String>,
    ) -> Result<Self, serde_json::Error> {
        let mut merged = VsCodeTheme::default();
        load_vscode_theme(json, resolve, 0, &mut merged)?;
        // VS Code's bundled themes leave `type` to the extension manifest
        // (`uiTheme`), so fall back to the background's lightness.
        let kind = match merged.kind.as_deref() {
            Some("light") | Some("hcLight") => ThemeKind::Light,
            Some(_) => ThemeKind::Dark,
            None => match merged
                .colors
                .get("editor.background")
                .and_then(|c| parse_hex_color(c))
            {
                Some(background) if background.l > 0.5 => ThemeKind::Light,
                _ => ThemeKind::Dark,
            },
        };
        let mut colors = match kind {
            ThemeKind::Light => EditorColors::vs(),
            ThemeKind::Dark => EditorColors::vs_dark(),
        };
        colors.apply_vscode_colors(&merged.colors);
        let mut rules = Vec::new();
        for token_color in merged.token_colors {
            let style = TokenStyle::from_font_style(
                token_color
                    .settings
                    .foreground
                    .as_deref()
                    .and_then(parse_hex_color),
                token_color.settings.font_style.as_deref(),
            );
            let scopes = match token_color.scope {
                None => continue,
                Some(Scope::One(scopes)) => scopes
                    .split(',')
                    .map(|s| s.trim().to_string())
                    .collect::<Vec<_>>(),
                Some(Scope::Many(scopes)) => scopes,
            };
            for scope in scopes {
                // Descendant selectors ("meta.embedded string") are matched on
                // their last segment only.
                let Some(selector) = scope.split_whitespace().last() else {
                    continue;
                };
                rules.push(TokenRule {
                    kind: RuleKind::TextMate,
                    selector: selector.to_string(),
                    style,
                });
            }
        }
        let mut theme = Self::new(
            merged.name.unwrap_or_else(|| "VS Code theme".into()),
            kind,
            colors,
            rules,
        );
        theme.semantic_highlighting = merged.semantic_highlighting.unwrap_or(false);
        theme.high_contrast = matches!(
            merged.kind.as_deref(),
            Some("hc" | "hcDark" | "hcLight" | "hc-black" | "hc-light")
        );
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

    /// The style for a highlight capture.
    pub fn style_for(&self, highlight: HighlightId) -> TokenStyle {
        let mut cache = self.cache.lock().unwrap_or_else(|e| e.into_inner());
        *cache
            .entry(highlight)
            .or_insert_with(|| self.resolve(&highlight.name()))
    }

    fn resolve(&self, capture: &str) -> TokenStyle {
        // TextMate rules: the first candidate scope with a matching rule wins;
        // among rules matching it, the longest (most specific) selector, and
        // later rules on ties.
        for scope in textmate_scopes_for_capture(capture) {
            if let Some(style) = best_match(&self.rules, RuleKind::TextMate, scope) {
                return style;
            }
        }
        if let Some(token) = monaco_token_for_capture(capture)
            && let Some(style) = best_match(&self.rules, RuleKind::Monaco, token)
        {
            return style;
        }
        TokenStyle::default()
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

/// `selector` matches `name` if equal or a dot-segment prefix of it.
fn selector_matches(selector: &str, name: &str) -> bool {
    name == selector
        || (name.starts_with(selector) && name.as_bytes().get(selector.len()) == Some(&b'.'))
}

pub(crate) fn best_match(rules: &[TokenRule], kind: RuleKind, name: &str) -> Option<TokenStyle> {
    let mut best: Option<&TokenRule> = None;
    for rule in rules.iter().filter(|rule| rule.kind == kind) {
        if selector_matches(&rule.selector, name)
            && best.is_none_or(|b| rule.selector.len() >= b.selector.len())
        {
            best = Some(rule);
        }
    }
    best.map(|rule| rule.style)
}

/// The Monarch token a tree-sitter capture corresponds to in Monaco themes.
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

    #[test]
    fn hex_colors() {
        assert_eq!(parse_hex_color("#fff"), Some(rgba(0xffffffff).into()));
        assert_eq!(parse_hex_color("#11223344"), Some(rgba(0x11223344).into()));
        assert_eq!(parse_hex_color("nope"), None);
    }

    #[test]
    fn monaco_rules_resolve_captures() {
        let theme = Theme::vs_dark();
        let keyword = theme.style_for(HighlightId::intern("keyword"));
        assert_eq!(keyword.foreground, parse_hex_color("#569CD6"));
        let key = theme.style_for(HighlightId::intern("string.special.key"));
        assert_eq!(key.foreground, parse_hex_color("#9CDCFE"));
        let function = theme.style_for(HighlightId::intern("function"));
        assert_eq!(
            function.foreground, None,
            "Monaco's vs-dark doesn't color functions"
        );
    }

    #[test]
    fn hc_black_theme() {
        let theme = Theme::hc_black();
        assert_eq!(theme.name, "hc-black");
        assert_eq!(theme.kind, ThemeKind::Dark);
        let colors = &theme.colors;
        assert_eq!(Some(colors.background), parse_hex_color("#000000"));
        assert_eq!(Some(colors.foreground), parse_hex_color("#FFFFFF"));
        assert_eq!(Some(colors.indent_guide), parse_hex_color("#FFFFFF"));
        assert_eq!(Some(colors.widget_border), parse_hex_color("#6FC3DF"));
        assert_eq!(Some(colors.peek_view_border), parse_hex_color("#6FC3DF"));
        assert_eq!(Some(colors.focus_border), parse_hex_color("#F38518"));
        assert_eq!(colors.line_highlight_border, parse_hex_color("#F38518"));
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
        let keyword = theme.style_for(HighlightId::intern("keyword"));
        assert_eq!(keyword.foreground, parse_hex_color("#0000FF"));
        let invalid = best_match(theme.rules(), RuleKind::Monaco, "invalid");
        assert_eq!(invalid.unwrap().foreground, parse_hex_color("#B5200D"));
        let string = theme.style_for(HighlightId::intern("string"));
        assert_eq!(string.foreground, parse_hex_color("#A31515"));
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
}
