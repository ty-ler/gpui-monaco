//! Editor (view) options, mirroring the subset of Monaco's `IEditorOptions`
//! implemented so far.

pub use crate::suggest::memory::SuggestSelection;
use gpui::{Pixels, SharedString, px};

/// Monaco `lineNumbers`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum LineNumbers {
    /// Absolute line numbers (the default).
    #[default]
    On,
    /// No line numbers; the gutter has no number column.
    Off,
    /// Distance from the primary cursor's line; that line shows its absolute number.
    Relative,
    /// Every 10th line, plus the current line.
    Interval,
}

/// Monaco `renderLineHighlight`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum RenderLineHighlight {
    /// No current-line highlight.
    None,
    /// Highlight only the gutter of the current line.
    Gutter,
    /// Highlight the text area of the current line (the default).
    #[default]
    Line,
    /// Highlight both the gutter and the text area of the current line.
    All,
}

/// Monaco `wordWrap`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum WordWrap {
    /// No wrapping (the default).
    #[default]
    Off,
    /// Wrap at the viewport width.
    On,
    /// Wrap at `word_wrap_column`.
    WordWrapColumn,
    /// Wrap at the smaller of the viewport width and `word_wrap_column`.
    Bounded,
}

/// Monaco `showFoldingControls`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ShowFoldingControls {
    /// Always show the fold chevrons.
    Always,
    /// Show the fold chevrons while the mouse is over the gutter (the default).
    #[default]
    Mouseover,
    /// Never show the fold chevrons.
    Never,
}

/// Monaco `minimap` options.
#[derive(Clone, Debug, PartialEq)]
pub struct MinimapOptions {
    /// Whether the minimap is shown (Monaco `minimap.enabled`, default `true`). Never shown
    /// for [`EditorOptions::single_line`] editors.
    pub enabled: bool,
    /// Columns rendered (Monaco `minimap.maxColumn`).
    pub max_column: u32,
    /// 1 or 2: pixels per character column and half the line height.
    pub scale: f32,
    /// Monaco `minimap.showSlider`: `true` always, `false` on mouse over.
    pub always_show_slider: bool,
}

impl Default for MinimapOptions {
    fn default() -> Self {
        Self {
            enabled: true,
            max_column: 120,
            scale: 1.,
            always_show_slider: false,
        }
    }
}

/// Monaco `find` options.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FindEditorOptions {
    /// Jump to matches while typing the search string (Monaco
    /// `find.cursorMoveOnType`, default `true`).
    pub cursor_move_on_type: bool,
    /// Where opening find takes its search string from (Monaco
    /// `find.seedSearchStringFromSelection`, default always).
    pub seed_search_string_from_selection: SeedSearchString,
    /// When opening find turns on find in selection (Monaco
    /// `find.autoFindInSelection`, default never).
    pub auto_find_in_selection: AutoFindInSelection,
    /// Scroll the text down so the widget doesn't cover the first line
    /// (Monaco `find.addExtraSpaceOnTop`, default `true`).
    pub add_extra_space_on_top: bool,
    /// Wrap around at the first and last match (Monaco `find.loop`,
    /// default `true`).
    pub loop_: bool,
}

impl Default for FindEditorOptions {
    fn default() -> Self {
        Self {
            cursor_move_on_type: true,
            seed_search_string_from_selection: SeedSearchString::Always,
            auto_find_in_selection: AutoFindInSelection::Never,
            add_extra_space_on_top: true,
            loop_: true,
        }
    }
}

/// Monaco `find.seedSearchStringFromSelection`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SeedSearchString {
    /// Never.
    Never,
    /// From the selection, or the word at the cursor.
    #[default]
    Always,
    /// Only from a selection.
    Selection,
}

/// Monaco `find.autoFindInSelection`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum AutoFindInSelection {
    /// Never.
    #[default]
    Never,
    /// Whenever there's a selection.
    Always,
    /// When the selection spans lines.
    Multiline,
}

/// Monaco `stickyScroll` options.
#[derive(Clone, Debug, PartialEq)]
pub struct StickyScrollOptions {
    /// Whether sticky scroll is on (Monaco `stickyScroll.enabled`, default `true`).
    pub enabled: bool,
    /// Most header lines pinned at once (Monaco `stickyScroll.maxLineCount`, default 5).
    pub max_line_count: u32,
    /// Where the scopes come from (Monaco `stickyScroll.defaultModel`,
    /// default outline).
    pub default_model: StickyScrollModel,
}

impl Default for StickyScrollOptions {
    fn default() -> Self {
        Self {
            enabled: true,
            max_line_count: 5,
            default_model: StickyScrollModel::Outline,
        }
    }
}

/// Monaco `stickyScroll.defaultModel`. Each falls back to the next when it
/// has no scopes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum StickyScrollModel {
    /// The document symbols (`outlineModel`), from providers or the
    /// language's tags query.
    #[default]
    Outline,
    /// The folding ranges (`foldingProviderModel`).
    FoldingProvider,
    /// Indentation (`indentationModel`).
    Indentation,
}

/// Which whitespace is drawn as `·` and `→` (Monaco `renderWhitespace`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum RenderWhitespace {
    /// None.
    None,
    /// All except single spaces between words.
    Boundary,
    /// Only whitespace inside selections (the default).
    #[default]
    Selection,
    /// Only trailing whitespace.
    Trailing,
    /// All whitespace.
    All,
}

/// How the last line's number shows when the file ends with a newline
/// (Monaco `renderFinalNewline`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum RenderFinalNewline {
    /// Numbered like any line (the default).
    #[default]
    On,
    /// No line number.
    Off,
    /// A dimmed line number.
    Dimmed,
}

/// A vertical ruler (Monaco `IRulerOption`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Ruler {
    /// The visible column it's drawn at.
    pub column: u32,
    /// Its color; `None` uses the theme's `editorRuler.foreground`.
    pub color: Option<gpui::Hsla>,
}

/// Monaco `guides`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GuideOptions {
    /// Draw indent guides (Monaco `guides.indentation`, default `true`).
    pub indentation: bool,
    /// Highlight the guide of the block with the cursor (Monaco
    /// `guides.highlightActiveIndentation`, default `true`). Lines with
    /// bracket pair guides don't highlight indent guides.
    pub highlight_active_indentation: bool,
    /// Vertical bracket pair guides (Monaco `guides.bracketPairs`, default
    /// off).
    pub bracket_pairs: BracketPairGuides,
    /// Horizontal bracket pair guides (Monaco
    /// `guides.bracketPairsHorizontal`, default for the active pair).
    pub bracket_pairs_horizontal: BracketPairGuides,
    /// Highlight the guides of the pair around the cursor (Monaco
    /// `guides.highlightActiveBracketPair`, default `true`).
    pub highlight_active_bracket_pair: bool,
}

impl Default for GuideOptions {
    fn default() -> Self {
        Self {
            indentation: true,
            highlight_active_indentation: true,
            bracket_pairs: BracketPairGuides::Off,
            bracket_pairs_horizontal: BracketPairGuides::Active,
            highlight_active_bracket_pair: true,
        }
    }
}

/// Which bracket pairs get guides (Monaco's `true`, `"active"` and
/// `false`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum BracketPairGuides {
    /// None.
    #[default]
    Off,
    /// Only the pair around the cursor.
    Active,
    /// Every pair.
    On,
}

/// Which modifier adds a cursor on click (Monaco `multiCursorModifier`); the
/// other one opens definitions.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum MultiCursorModifier {
    /// Alt+click adds a cursor; Cmd/Ctrl+click goes to the definition.
    #[default]
    Alt,
    /// Cmd/Ctrl+click adds a cursor; Alt+click goes to the definition.
    CtrlCmd,
}

/// Monaco `cursorStyle`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum CursorStyle {
    /// A vertical bar [`EditorOptions::cursor_width`] pixels wide (the default).
    #[default]
    Line,
    /// A box covering the character after the cursor.
    Block,
    /// A 2px bar under the character after the cursor.
    Underline,
    /// A 1px vertical bar.
    LineThin,
}

/// Monaco `acceptSuggestionOnEnter`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum AcceptSuggestionOnEnter {
    /// Enter accepts.
    #[default]
    On,
    /// Enter accepts unless the suggestion would change nothing (the word
    /// is already typed) in a session that opened by itself.
    Smart,
    /// Only Tab accepts.
    Off,
}

/// Monaco `suggest.insertMode`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SuggestInsertMode {
    /// Insert before the rest of the word (the item's `range`).
    #[default]
    Insert,
    /// Replace the rest of the word too (the item's `replace_range`).
    Replace,
}

/// Monaco `quickSuggestions`: where suggestions open while typing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct QuickSuggestions {
    /// In code.
    pub other: bool,
    /// In comments.
    pub comments: bool,
    /// In strings.
    pub strings: bool,
}

impl QuickSuggestions {
    /// Everywhere.
    pub const ON: Self = Self {
        other: true,
        comments: true,
        strings: true,
    };
    /// Nowhere.
    pub const OFF: Self = Self {
        other: false,
        comments: false,
        strings: false,
    };
}

impl Default for QuickSuggestions {
    /// Monaco's default: in code only.
    fn default() -> Self {
        Self {
            other: true,
            comments: false,
            strings: false,
        }
    }
}

/// Monaco `wordBasedSuggestions`: completions from words in documents,
/// offered when no other provider has any.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum WordBasedSuggestions {
    /// None.
    Off,
    /// Monaco's default: on unless inline suggestions are available (there
    /// are none yet, so the same as `MatchingDocuments`).
    #[default]
    OffWithInlineSuggestions,
    /// Words of this document.
    CurrentDocument,
    /// Words of open documents in the same language.
    MatchingDocuments,
    /// Words of all open documents.
    AllDocuments,
}

/// How ghost text relates to the text at the cursor (Monaco
/// `inlineSuggest.mode`, also `suggest.previewMode`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum InlineSuggestMode {
    /// Only text appended at the cursor.
    Prefix,
    /// Insertions anywhere in the rest of the word.
    Subword,
    /// Insertions in the rest of the word, but none before the cursor.
    #[default]
    SubwordSmart,
}

/// Monaco `inlineSuggest`.
#[derive(Clone, Debug, PartialEq)]
pub struct InlineSuggestOptions {
    /// `enabled`: show inline completions (on).
    pub enabled: bool,
    /// `mode`.
    pub mode: InlineSuggestMode,
    /// `suppressSuggestions`: keep the suggest widget from opening by itself
    /// while ghost text shows (off).
    pub suppress_suggestions: bool,
    /// `keepOnBlur`: keep ghost text when the editor loses focus (off).
    pub keep_on_blur: bool,
    /// `minShowDelay`: show results no sooner than this after the request.
    pub min_show_delay: std::time::Duration,
    /// `suppressInSnippetMode`: no ghost text inside a snippet (on).
    pub suppress_in_snippet_mode: bool,
}

impl Default for InlineSuggestOptions {
    fn default() -> Self {
        Self {
            enabled: true,
            mode: InlineSuggestMode::SubwordSmart,
            suppress_suggestions: false,
            keep_on_blur: false,
            min_show_delay: std::time::Duration::ZERO,
            suppress_in_snippet_mode: true,
        }
    }
}

/// Monaco `semanticHighlighting.enabled`: whether semantic tokens color
/// the text.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SemanticHighlighting {
    /// Always.
    On,
    /// Never.
    Off,
    /// As the theme says (`Theme::semantic_highlighting`; off for Monaco's
    /// built-in themes, as in standalone Monaco).
    #[default]
    ConfiguredByTheme,
}

/// Monaco `renderValidationDecorations`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum RenderValidationDecorations {
    /// Only while the editor is editable (not read-only).
    #[default]
    Editable,
    /// Always.
    On,
    /// Never.
    Off,
}

/// Monaco `tabCompletion`: what Tab does without the suggest widget.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum TabCompletion {
    /// Tab indents.
    #[default]
    Off,
    /// At the end of a word, Tab inserts the best completion; Tab and
    /// Shift+Tab then cycle through the other suggestions.
    On,
    /// Only snippet prefixes complete (Monaco's `onlySnippets`). Until
    /// snippets land this behaves like `Off`.
    OnlySnippets,
}

/// Monaco `editor.suggest`.
#[derive(Clone, Debug, PartialEq)]
pub struct SuggestOptions {
    /// `insertMode`.
    pub insert_mode: SuggestInsertMode,
    /// `filterGraceful`: tolerate small typos when filtering.
    pub filter_graceful: bool,
    /// `matchOnWordStartOnly`: the first character must match a word start.
    pub match_on_word_start_only: bool,
    /// `showDeprecated`.
    pub show_deprecated: bool,
    /// Kinds hidden from the list (Monaco's `show*` options set to false,
    /// e.g. `showWords: false` hides `Text`).
    pub hidden_kinds: Vec<crate::providers::CompletionItemKind>,
    /// `localityBonus`: among equal matches, prefer words used in the
    /// innermost bracket block around the cursor.
    pub locality_bonus: bool,
    /// `fontSize`: the widget's font size; 0 uses the editor's.
    pub font_size: f32,
    /// `lineHeight`: the widget's row height; 0 uses the editor's.
    pub line_height: f32,
    /// `showIcons`: kind icons next to labels.
    pub show_icons: bool,
    /// `showStatusBar`: the insert/replace and details hints under the list.
    pub show_status_bar: bool,
    /// `showInlineDetails`: details at the right of the focused row.
    pub show_inline_details: bool,
    /// `preview`: show the selected suggestion as ghost text (off).
    pub preview: bool,
    /// `previewMode`: how the preview relates to the typed text.
    pub preview_mode: InlineSuggestMode,
}

impl Default for SuggestOptions {
    fn default() -> Self {
        Self {
            insert_mode: SuggestInsertMode::Insert,
            filter_graceful: true,
            match_on_word_start_only: true,
            show_deprecated: true,
            hidden_kinds: Vec::new(),
            locality_bonus: false,
            font_size: 0.,
            line_height: 0.,
            show_icons: true,
            show_status_bar: false,
            show_inline_details: true,
            preview: false,
            preview_mode: InlineSuggestMode::SubwordSmart,
        }
    }
}

/// What opens the color picker from a swatch (Monaco
/// `colorDecoratorsActivatedOn`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ColorDecoratorsActivatedOn {
    /// Hovering or clicking.
    #[default]
    ClickAndHover,
    /// Hovering.
    Hover,
    /// Clicking.
    Click,
}

/// When the built-in color finder (hex, `rgb()`, `hsl()` and the like)
/// decorates colors (Monaco `defaultColorDecorators`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum DefaultColorDecorators {
    /// Only when no color provider found any.
    #[default]
    Auto,
    /// Always, alongside providers.
    Always,
    /// Never.
    Never,
}

/// Where folding ranges come from (Monaco `foldingStrategy`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum FoldingStrategy {
    /// Folding range providers, else the syntax tree (`folds.scm`), else
    /// indentation.
    #[default]
    Auto,
    /// Indentation (and region markers) only.
    Indentation,
}

/// When inlay hints show (Monaco `inlayHints.enabled`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum InlayHintsMode {
    /// Always.
    #[default]
    On,
    /// Hidden while Ctrl+Alt is held.
    OnUnlessPressed,
    /// Shown only while Ctrl+Alt is held.
    OffUnlessPressed,
    /// Never (nothing is requested).
    Off,
}

/// Monaco `inlayHints`.
#[derive(Clone, Debug, PartialEq)]
pub struct InlayHintsOptions {
    /// `enabled`.
    pub enabled: InlayHintsMode,
    /// `padding`: pad the hint's background.
    pub padding: bool,
    /// `maximumLength`: hint text per line, beyond which it's cut with
    /// `…`; 0 for no limit.
    pub maximum_length: usize,
}

impl Default for InlayHintsOptions {
    fn default() -> Self {
        Self {
            enabled: InlayHintsMode::On,
            padding: false,
            maximum_length: 43,
        }
    }
}

/// When the lightbulb asks for code actions (Monaco
/// `ShowLightbulbIconMode`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum LightbulbMode {
    /// Never.
    Off,
    /// When the cursor is on or next to code, or text is selected.
    #[default]
    OnCode,
    /// Always.
    On,
}

/// What symbol navigation does with several results (Monaco
/// `GoToLocationValues`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum GoToLocation {
    /// Show them in a peek view.
    #[default]
    Peek,
    /// Go to the first and show them all in a peek view.
    GotoAndPeek,
    /// Go to the first; F4 goes to the next.
    Goto,
}

/// A symbol navigation command.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum LocationKind {
    /// Go to definition (F12).
    Definition,
    /// Go to declaration.
    Declaration,
    /// Go to type definition.
    TypeDefinition,
    /// Go to implementations (Cmd/Ctrl+F12).
    Implementation,
    /// Go to references (Shift+F12).
    References,
}

/// Monaco `gotoLocation`: what each navigation command does with several
/// results, and what it runs instead when its only result is where the
/// cursor already is.
#[derive(Clone, Debug, PartialEq)]
pub struct GotoLocationOptions {
    /// `multipleDefinitions`.
    pub multiple_definitions: GoToLocation,
    /// `multipleTypeDefinitions`.
    pub multiple_type_definitions: GoToLocation,
    /// `multipleDeclarations`.
    pub multiple_declarations: GoToLocation,
    /// `multipleImplementations`.
    pub multiple_implementations: GoToLocation,
    /// `multipleReferences`.
    pub multiple_references: GoToLocation,
    /// `alternativeDefinitionCommand` (go to references by default).
    pub alternative_definition_command: Option<LocationKind>,
    /// `alternativeTypeDefinitionCommand` (go to references by default).
    pub alternative_type_definition_command: Option<LocationKind>,
    /// `alternativeDeclarationCommand` (go to references by default).
    pub alternative_declaration_command: Option<LocationKind>,
    /// `alternativeImplementationCommand` (none by default).
    pub alternative_implementation_command: Option<LocationKind>,
    /// `alternativeReferenceCommand` (none by default).
    pub alternative_reference_command: Option<LocationKind>,
}

impl Default for GotoLocationOptions {
    fn default() -> Self {
        Self {
            multiple_definitions: GoToLocation::Peek,
            multiple_type_definitions: GoToLocation::Peek,
            multiple_declarations: GoToLocation::Peek,
            multiple_implementations: GoToLocation::Peek,
            multiple_references: GoToLocation::Peek,
            alternative_definition_command: Some(LocationKind::References),
            alternative_type_definition_command: Some(LocationKind::References),
            alternative_declaration_command: Some(LocationKind::References),
            alternative_implementation_command: None,
            alternative_reference_command: None,
        }
    }
}

impl GotoLocationOptions {
    /// What `kind` does with several results.
    pub fn multiple(&self, kind: LocationKind) -> GoToLocation {
        match kind {
            LocationKind::Definition => self.multiple_definitions,
            LocationKind::Declaration => self.multiple_declarations,
            LocationKind::TypeDefinition => self.multiple_type_definitions,
            LocationKind::Implementation => self.multiple_implementations,
            LocationKind::References => self.multiple_references,
        }
    }

    /// What `kind` runs instead when its only result is at the cursor.
    pub fn alternative(&self, kind: LocationKind) -> Option<LocationKind> {
        match kind {
            LocationKind::Definition => self.alternative_definition_command,
            LocationKind::Declaration => self.alternative_declaration_command,
            LocationKind::TypeDefinition => self.alternative_type_definition_command,
            LocationKind::Implementation => self.alternative_implementation_command,
            LocationKind::References => self.alternative_reference_command,
        }
    }
}

/// Options for one [`CodeEditor`](crate::CodeEditor) view (Monaco `IEditorOptions`).
///
/// The model-level options (tab size, insert spaces, …) live in
/// [`ModelOptions`](crate::ModelOptions).
#[derive(Clone, Debug, PartialEq)]
pub struct EditorOptions {
    /// Monaco `fontFamily`. Defaults to Menlo on macOS, Consolas on Windows and
    /// Droid Sans Mono elsewhere.
    pub font_family: SharedString,
    /// Monaco `fontSize`. Defaults to 12px on macOS and 14px elsewhere.
    pub font_size: Pixels,
    /// `None` computes it from the font size like Monaco
    /// (`GOLDEN_LINE_HEIGHT_RATIO`, 1.5 on macOS and 1.35 elsewhere).
    pub line_height: Option<Pixels>,
    /// Monaco `lineNumbers`.
    pub line_numbers: LineNumbers,
    /// Monaco `lineNumbersMinChars`: the number column is at least this many digits
    /// wide (default 5).
    pub line_numbers_min_chars: u32,
    /// Monaco `lineDecorationsWidth`: space between line numbers and text.
    pub line_decorations_width: Pixels,
    /// Monaco `glyphMargin`: show the gutter column for glyph decorations (default
    /// `true`).
    pub glyph_margin: bool,
    /// Monaco `renderLineHighlight`.
    pub render_line_highlight: RenderLineHighlight,
    /// Monaco `cursorStyle`.
    pub cursor_style: CursorStyle,
    /// Monaco `cursorWidth`: width of the [`CursorStyle::Line`] cursor (default 2px).
    pub cursor_width: Pixels,
    /// Whether the cursor blinks (default `true`). Monaco's `cursorBlinking` has more
    /// styles; `false` is `solid`.
    pub cursor_blinking: bool,
    /// Monaco `scrollBeyondLastLine`: allow scrolling the last line to the top of the
    /// viewport (default `true`).
    pub scroll_beyond_last_line: bool,
    /// Monaco `readOnly`: ignore edits from typing, IME and editing commands (default
    /// `false`).
    pub read_only: bool,
    /// Monaco `wordSeparators`: characters that end a word for word navigation,
    /// double-click selection and whole-word search.
    pub word_separators: SharedString,
    /// Monaco `scrollbar.verticalScrollbarSize` (default 14px; 0 hides it).
    pub vertical_scrollbar_size: Pixels,
    /// Monaco `scrollbar.horizontalScrollbarSize` (default 12px; 0 hides it).
    pub horizontal_scrollbar_size: Pixels,
    /// Pixels scrolled per wheel "line" step is the line height; this scales it
    /// (Monaco `mouseWheelScrollSensitivity`).
    pub mouse_wheel_scroll_sensitivity: f32,
    /// Monaco `emptySelectionClipboard`: copy and cut with an empty selection act on
    /// the whole line (default `true`).
    pub empty_selection_clipboard: bool,
    /// Monaco `stickyTabStops`: move the cursor over indentation spaces as if they were
    /// tabs (default `false`).
    pub sticky_tab_stops: bool,
    /// Monaco `wordWrap`.
    pub word_wrap: WordWrap,
    /// Monaco `wordWrapColumn`, in characters (default 80); used by
    /// [`WordWrap::WordWrapColumn`] and [`WordWrap::Bounded`].
    pub word_wrap_column: u32,
    /// Monaco `wrappingIndent`: indentation of wrapped continuation lines (default
    /// same as the first line).
    pub wrapping_indent: crate::view::line_breaks::WrappingIndent,
    /// Monaco `folding`: enable code folding and its gutter column (default `true`).
    pub folding: bool,
    /// Monaco `foldingStrategy`.
    pub folding_strategy: FoldingStrategy,
    /// Monaco `foldingMaximumRegions`: provider and syntax ranges beyond
    /// this many are dropped, the most deeply nested first.
    pub folding_maximum_regions: usize,
    /// Monaco `foldingHighlight`: shade collapsed regions' first lines.
    pub folding_highlight: bool,
    /// Monaco `unfoldOnClickAfterEndOfLine`: a click after the end of a
    /// collapsed line unfolds it (a click on its `⋯` always does).
    pub unfold_on_click_after_end_of_line: bool,
    /// Monaco `foldingImportsByDefault`: collapse import ranges when a
    /// model is first shown.
    pub folding_imports_by_default: bool,
    /// Monaco `showFoldingControls`.
    pub show_folding_controls: ShowFoldingControls,
    /// Monaco `autoClosingBrackets` / `autoClosingQuotes` /
    /// `autoClosingComments`.
    pub auto_closing_brackets: crate::cursor::AutoClosingStrategy,
    /// Monaco `autoClosingQuotes`.
    pub auto_closing_quotes: crate::cursor::AutoClosingStrategy,
    /// Monaco `autoClosingComments`.
    pub auto_closing_comments: crate::cursor::AutoClosingStrategy,
    /// Monaco `autoClosingOvertype` / `autoClosingDelete`.
    pub auto_closing_overtype: crate::cursor::AutoClosingEditStrategy,
    /// Monaco `autoClosingDelete`.
    pub auto_closing_delete: crate::cursor::AutoClosingEditStrategy,
    /// Monaco `autoSurround`.
    pub auto_surround: crate::cursor::AutoSurroundStrategy,
    /// Monaco `autoIndent`.
    pub auto_indent: crate::cursor::AutoIndent,
    /// Monaco `matchBrackets`: highlight the bracket pair at the cursor.
    pub match_brackets: bool,
    /// Monaco `quickSuggestions`: where suggestions open while typing.
    pub quick_suggestions: QuickSuggestions,
    /// Monaco `quickSuggestionsDelay`: how long after typing they open.
    pub quick_suggestions_delay: std::time::Duration,
    /// Monaco `suggestOnTriggerCharacters`.
    pub suggest_on_trigger_characters: bool,
    /// Monaco `acceptSuggestionOnEnter`.
    pub accept_suggestion_on_enter: AcceptSuggestionOnEnter,
    /// `tabCompletion`.
    pub tab_completion: TabCompletion,
    /// Monaco `acceptSuggestionOnCommitCharacter`.
    pub accept_suggestion_on_commit_character: bool,
    /// Monaco `suggestSelection`.
    pub suggest_selection: SuggestSelection,
    /// Monaco `wordBasedSuggestions`.
    pub word_based_suggestions: WordBasedSuggestions,
    /// Monaco `snippetSuggestions` (`None` hides snippets).
    pub snippet_suggestions: Option<crate::suggest::SnippetSortOrder>,
    /// Monaco `suggest`.
    pub suggest: SuggestOptions,
    /// Monaco `gotoLocation`.
    pub goto_location: GotoLocationOptions,
    /// Monaco `lightbulb.enabled`.
    pub lightbulb: LightbulbMode,
    /// Monaco `inlayHints`.
    pub inlay_hints: InlayHintsOptions,
    /// Monaco `smartSelect`.
    pub smart_select: crate::cursor::smart_select::SmartSelectOptions,
    /// Monaco `formatOnType`: format with the on-type formatter when one of
    /// its trigger characters is typed.
    pub format_on_type: bool,
    /// Monaco `formatOnPaste`: format pasted text with the range formatter.
    pub format_on_paste: bool,
    /// Monaco `minimap`.
    pub minimap: MinimapOptions,
    /// Monaco `stickyScroll`.
    pub sticky_scroll: StickyScrollOptions,
    /// Monaco `find`.
    pub find: FindEditorOptions,
    /// Monaco `bracketPairColorization.enabled`: color brackets by nesting
    /// level (default `true`).
    pub bracket_pair_colorization: bool,
    /// Monaco `renderWhitespace`.
    pub render_whitespace: RenderWhitespace,
    /// Monaco `renderControlCharacters`: show control characters as visible
    /// symbols such as `␇` (default `true`).
    pub render_control_characters: bool,
    /// Monaco `renderFinalNewline`.
    pub render_final_newline: RenderFinalNewline,
    /// Monaco `guides`.
    pub guides: GuideOptions,
    /// Monaco `rulers`: vertical lines at visible columns.
    pub rulers: Vec<Ruler>,
    /// Monaco `occurrencesHighlight`: highlight the occurrences of the symbol
    /// at the cursor (default `true`).
    pub occurrences_highlight: bool,
    /// Monaco `occurrencesHighlightDelay`: how long the cursor rests before
    /// occurrences are highlighted (default 250ms).
    pub occurrences_highlight_delay: std::time::Duration,
    /// Monaco `selectionHighlight`: highlight other occurrences of the
    /// selected text (default `true`).
    pub selection_highlight: bool,
    /// Monaco `selectionHighlightMaxLength`: longer selections aren't
    /// highlighted (default 200 UTF-16 units; 0 for no limit).
    pub selection_highlight_max_length: u32,
    /// Monaco `selectionHighlightMultiline`: also highlight multi-line
    /// selections (default `false`).
    pub selection_highlight_multiline: bool,
    /// Monaco `multiCursorModifier`.
    pub multi_cursor_modifier: MultiCursorModifier,
    /// Monaco `multiCursorPaste`: how pasting with several cursors splits
    /// the text.
    pub multi_cursor_paste: crate::cursor::MultiCursorPaste,
    /// Monaco `inlineSuggest`.
    pub inline_suggest: InlineSuggestOptions,
    /// Monaco `codeLens`: show code lenses (on).
    pub code_lens: bool,
    /// Monaco `codeLensFontFamily`; `None` uses the editor's.
    pub code_lens_font_family: Option<SharedString>,
    /// Monaco `codeLensFontSize`; 0 (or under 5) uses 90% of the editor's.
    pub code_lens_font_size: f32,
    /// Monaco `linkedEditing`: edit linked ranges (like HTML tag pairs)
    /// together (off).
    pub linked_editing: bool,
    /// Monaco `semanticHighlighting.enabled`.
    pub semantic_highlighting: SemanticHighlighting,
    /// Monaco `links`: detect links and follow them with Cmd/Ctrl+click.
    pub links: bool,
    /// Monaco `contextmenu`: right-click and Shift+F10 show the editor's
    /// context menu.
    pub context_menu: bool,
    /// Monaco `colorDecorators`: a swatch before each color in the text,
    /// which opens the color picker.
    pub color_decorators: bool,
    /// Monaco `colorDecoratorsLimit`: at most this many swatches (500).
    pub color_decorators_limit: usize,
    /// Monaco `colorDecoratorsActivatedOn`: what opens the picker from a
    /// swatch.
    pub color_decorators_activated_on: ColorDecoratorsActivatedOn,
    /// Monaco `defaultColorDecorators`: when the built-in CSS color finder
    /// runs.
    pub default_color_decorators: DefaultColorDecorators,
    /// Monaco `showUnused`: fade code diagnostics tag unnecessary.
    pub show_unused: bool,
    /// Monaco `showDeprecated`: strike through code diagnostics tag
    /// deprecated.
    pub show_deprecated: bool,
    /// Monaco `renderValidationDecorations`: when diagnostics are drawn.
    pub render_validation_decorations: RenderValidationDecorations,
    /// Monaco `multiCursorLimit`: the most cursors at once (default 10000).
    pub multi_cursor_limit: usize,
    /// Monaco `multiCursorMergeOverlapping`: merge overlapping selections
    /// (default `true`).
    pub multi_cursor_merge_overlapping: bool,
    /// Monaco `columnSelection`: mouse drags and Shift+arrows make column
    /// (box) selections.
    pub column_selection: bool,
    /// Monaco `overtypeCursorStyle`: the cursor shape in overtype mode.
    pub overtype_cursor_style: CursorStyle,
    /// Monaco `overtypeOnPaste`: in overtype mode, pasting overtypes too.
    pub overtype_on_paste: bool,
    /// Monaco `comments`: how comment toggling inserts and removes tokens.
    pub comments: crate::cursor::CommentOptions,
    /// A one-line input (like Monaco's find input): Enter, Tab and Escape
    /// propagate to the parent and pasted line breaks become spaces.
    pub single_line: bool,
}

impl Default for EditorOptions {
    fn default() -> Self {
        Self {
            font_family: if cfg!(target_os = "macos") {
                "Menlo".into()
            } else if cfg!(target_os = "windows") {
                "Consolas".into()
            } else {
                "Droid Sans Mono".into()
            },
            font_size: if cfg!(target_os = "macos") {
                px(12.)
            } else {
                px(14.)
            },
            line_height: None,
            line_numbers: LineNumbers::On,
            line_numbers_min_chars: 5,
            line_decorations_width: px(10.),
            glyph_margin: true,
            render_line_highlight: RenderLineHighlight::Line,
            cursor_style: CursorStyle::Line,
            cursor_width: px(2.),
            cursor_blinking: true,
            scroll_beyond_last_line: true,
            read_only: false,
            word_separators: "`~!@#$%^&*()-=+[{]}\\|;:'\",.<>/?".into(),
            vertical_scrollbar_size: px(14.),
            horizontal_scrollbar_size: px(12.),
            mouse_wheel_scroll_sensitivity: 1.0,
            empty_selection_clipboard: true,
            sticky_tab_stops: false,
            word_wrap: WordWrap::Off,
            word_wrap_column: 80,
            wrapping_indent: crate::view::line_breaks::WrappingIndent::Same,
            folding: true,
            folding_strategy: FoldingStrategy::Auto,
            folding_maximum_regions: 5000,
            folding_highlight: true,
            unfold_on_click_after_end_of_line: false,
            folding_imports_by_default: false,
            show_folding_controls: ShowFoldingControls::Mouseover,
            single_line: false,
            auto_closing_brackets: Default::default(),
            auto_closing_quotes: Default::default(),
            auto_closing_comments: Default::default(),
            auto_closing_overtype: Default::default(),
            auto_closing_delete: Default::default(),
            auto_surround: Default::default(),
            auto_indent: Default::default(),
            match_brackets: true,
            quick_suggestions: QuickSuggestions::default(),
            quick_suggestions_delay: std::time::Duration::from_millis(10),
            suggest_on_trigger_characters: true,
            accept_suggestion_on_enter: AcceptSuggestionOnEnter::On,
            tab_completion: TabCompletion::Off,
            accept_suggestion_on_commit_character: true,
            suggest_selection: Default::default(),
            word_based_suggestions: WordBasedSuggestions::OffWithInlineSuggestions,
            snippet_suggestions: Some(crate::suggest::SnippetSortOrder::Inline),
            suggest: SuggestOptions::default(),
            goto_location: GotoLocationOptions::default(),
            lightbulb: LightbulbMode::OnCode,
            inlay_hints: InlayHintsOptions::default(),
            smart_select: Default::default(),
            format_on_type: false,
            format_on_paste: false,
            minimap: MinimapOptions::default(),
            sticky_scroll: StickyScrollOptions::default(),
            find: FindEditorOptions::default(),
            comments: Default::default(),
            overtype_cursor_style: CursorStyle::Block,
            bracket_pair_colorization: true,
            render_whitespace: RenderWhitespace::Selection,
            render_control_characters: true,
            render_final_newline: RenderFinalNewline::On,
            guides: GuideOptions::default(),
            rulers: Vec::new(),
            occurrences_highlight: true,
            occurrences_highlight_delay: std::time::Duration::from_millis(250),
            selection_highlight: true,
            selection_highlight_max_length: 200,
            selection_highlight_multiline: false,
            multi_cursor_modifier: MultiCursorModifier::Alt,
            multi_cursor_paste: Default::default(),
            inline_suggest: InlineSuggestOptions::default(),
            code_lens: true,
            code_lens_font_family: None,
            code_lens_font_size: 0.,
            linked_editing: false,
            semantic_highlighting: SemanticHighlighting::ConfiguredByTheme,
            links: true,
            context_menu: true,
            color_decorators: true,
            color_decorators_limit: 500,
            color_decorators_activated_on: ColorDecoratorsActivatedOn::ClickAndHover,
            default_color_decorators: DefaultColorDecorators::Auto,
            show_unused: true,
            show_deprecated: true,
            render_validation_decorations: RenderValidationDecorations::Editable,
            multi_cursor_limit: 10_000,
            multi_cursor_merge_overlapping: true,
            column_selection: false,
            overtype_on_paste: true,
        }
    }
}

impl EditorOptions {
    /// Options for a one-line input box: no gutter, scrollbars or line
    /// highlight.
    pub fn single_line_input() -> Self {
        Self {
            line_numbers: LineNumbers::Off,
            glyph_margin: false,
            folding: false,
            render_line_highlight: RenderLineHighlight::None,
            scroll_beyond_last_line: false,
            vertical_scrollbar_size: px(0.),
            horizontal_scrollbar_size: px(0.),
            line_decorations_width: px(2.),
            single_line: true,
            minimap: MinimapOptions {
                enabled: false,
                ..MinimapOptions::default()
            },
            sticky_scroll: StickyScrollOptions {
                enabled: false,
                ..StickyScrollOptions::default()
            },
            color_decorators: false,
            context_menu: false,
            ..Self::default()
        }
    }
}

impl EditorOptions {
    /// The line height in pixels: [`line_height`](Self::line_height), or the font size
    /// times 1.5 on macOS and 1.35 elsewhere, rounded.
    pub fn resolved_line_height(&self) -> Pixels {
        self.line_height.unwrap_or_else(|| {
            let ratio = if cfg!(target_os = "macos") { 1.5 } else { 1.35 };
            px((f32::from(self.font_size) * ratio).round())
        })
    }
}
