//! Language feature providers (Monaco `monaco.languages.register*Provider`).
//!
//! The editor renders completion, hover, signature help and go-to-definition
//! UI; hosts supply the data by registering providers for a language. Each
//! request gets a [`ProviderContext`] (the text snapshot plus the model's
//! uri and language) and returns a GPUI [`Task`]; dropping the task cancels
//! the request, which the editor does when a newer request supersedes it.

use std::ops::Range;
use std::sync::Arc;

use gpui::{App, Global, SharedString, Task};
use text::{BufferSnapshot, PointUtf16};

/// Which models a provider applies to (Monaco `LanguageSelector`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LanguageSelector {
    /// Every model, including ones without a language.
    All,
    /// Models whose language id is one of these.
    Languages(Vec<SharedString>),
}

impl LanguageSelector {
    /// A selector for a single language id.
    pub fn language(id: impl Into<SharedString>) -> Self {
        Self::Languages(vec![id.into()])
    }

    /// Whether a model with `language_id` is selected. Models without a language only
    /// match [`LanguageSelector::All`].
    pub fn matches(&self, language_id: Option<&str>) -> bool {
        match self {
            Self::All => true,
            Self::Languages(ids) => language_id.is_some_and(|id| ids.iter().any(|l| l == id)),
        }
    }
}

/// What a provider sees about the model it's asked about.
#[derive(Clone)]
pub struct ProviderContext {
    /// The model's uri, if the host gave it one (Monaco models always have
    /// one); LSP bridges use it to identify the document.
    pub uri: Option<SharedString>,
    /// The model's language id, if any.
    pub language_id: Option<SharedString>,
    /// The model's text at the time of the request.
    pub snapshot: BufferSnapshot,
}

/// Markdown text (Monaco `IMarkdownString`).
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct MarkdownString(pub SharedString);

impl<T: Into<SharedString>> From<T> for MarkdownString {
    fn from(value: T) -> Self {
        Self(value.into())
    }
}

// ----- completion ---------------------------------------------------------------

/// Monaco `CompletionItemKind`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum CompletionItemKind {
    /// A method.
    Method,
    /// A function.
    Function,
    /// A constructor.
    Constructor,
    /// A field.
    Field,
    /// A variable.
    Variable,
    /// A class.
    Class,
    /// A struct.
    Struct,
    /// An interface.
    Interface,
    /// A module.
    Module,
    /// A property.
    Property,
    /// An event.
    Event,
    /// An operator.
    Operator,
    /// A unit.
    Unit,
    /// A value.
    Value,
    /// A constant.
    Constant,
    /// An enum.
    Enum,
    /// An enum member.
    EnumMember,
    /// A keyword.
    Keyword,
    /// Plain text (the default).
    #[default]
    Text,
    /// A color.
    Color,
    /// A file.
    File,
    /// A reference.
    Reference,
    /// A folder.
    Folder,
    /// A type parameter.
    TypeParameter,
    /// A snippet.
    Snippet,
}

/// Monaco `CompletionItem`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct CompletionItem {
    /// The text shown in the suggest widget.
    pub label: SharedString,
    /// Picks the icon shown next to the label; also used to order snippets.
    pub kind: CompletionItemKind,
    /// Shown next to the label (e.g. a type signature).
    pub detail: Option<SharedString>,
    /// Shown in the details pane while the item is selected.
    pub documentation: Option<MarkdownString>,
    /// Sorts items with equal filter scores; defaults to the label.
    pub sort_text: Option<SharedString>,
    /// Text matched against what was typed; defaults to the label.
    pub filter_text: Option<SharedString>,
    /// Inserted on accept; defaults to the label. Snippet syntax if
    /// [`insert_text_is_snippet`](Self::insert_text_is_snippet).
    pub insert_text: Option<SharedString>,
    /// `insert_text` is a snippet (Monaco's `InsertAsSnippet` rule): its
    /// tab stops, placeholders and variables are expanded.
    pub insert_text_is_snippet: bool,
    /// Insert the text's indentation as is (Monaco's `KeepWhitespace`
    /// rule) instead of indenting its lines like the cursor's line.
    pub keep_whitespace: bool,
    /// The single-line range to replace; defaults to the word before the
    /// cursor.
    pub range: Option<Range<PointUtf16>>,
    /// Prefer selecting this item when it ties for the best filter score.
    pub preselect: bool,
    /// Typing one of these accepts the item, then inserts the character.
    pub commit_characters: Vec<char>,
    /// Edits applied elsewhere (e.g. imports) when accepted.
    pub additional_text_edits: Vec<(Range<PointUtf16>, String)>,
    /// The range replaced in replace mode (Monaco `range.replace`, e.g.
    /// the whole word); `range` is then the insert-mode range, which must
    /// start at the same position. See `suggest.insert_mode`.
    pub replace_range: Option<Range<PointUtf16>>,
    /// Tags, such as [`CompletionItemTag::Deprecated`] (shown struck out).
    pub tags: Vec<CompletionItemTag>,
    /// A command run after the item is inserted.
    pub command: Option<Command>,
    /// Data for [`CompletionProvider::resolve_completion_item`].
    pub data: Option<serde_json::Value>,
    /// Shown right after the label, dimmed, e.g. a signature (Monaco
    /// `CompletionItemLabel.detail`).
    pub label_detail: Option<SharedString>,
    /// Shown at the right of every row, e.g. a module path (Monaco
    /// `CompletionItemLabel.description`); replaces the inline `detail`.
    pub label_description: Option<SharedString>,
}

impl CompletionItem {
    /// An item with `label` and `kind`; every other field takes its default.
    pub fn new(label: impl Into<SharedString>, kind: CompletionItemKind) -> Self {
        Self {
            label: label.into(),
            kind,
            ..Default::default()
        }
    }

    /// [`filter_text`](Self::filter_text), or the label when unset.
    pub fn filter_text(&self) -> &str {
        self.filter_text.as_deref().unwrap_or(&self.label)
    }

    /// [`sort_text`](Self::sort_text), or the label when unset.
    pub fn sort_text(&self) -> &str {
        self.sort_text.as_deref().unwrap_or(&self.label)
    }

    /// [`insert_text`](Self::insert_text), or the label when unset.
    pub fn insert_text(&self) -> &str {
        self.insert_text.as_deref().unwrap_or(&self.label)
    }
}

/// Monaco `CompletionItemTag`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum CompletionItemTag {
    /// Deprecated: shown struck out, and hidden with
    /// `suggest.show_deprecated` off.
    Deprecated,
}

/// Monaco `CompletionList`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct CompletionList {
    /// The suggestions. Lists from several providers are concatenated.
    pub items: Vec<CompletionItem>,
    /// More typing should re-query instead of filtering.
    pub incomplete: bool,
}

/// Monaco `CompletionTriggerKind`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CompletionTrigger {
    /// Typing a word character or Ctrl+Space.
    Invoke,
    /// Typing one of the provider's
    /// [`trigger_characters`](CompletionProvider::trigger_characters).
    TriggerCharacter(char),
    /// Re-query of an incomplete list.
    Incomplete,
}

/// Supplies completion items (Monaco `CompletionItemProvider`).
///
/// Register with [`register_completion_provider`].
pub trait CompletionProvider: 'static {
    /// Characters that open the suggest widget when typed (e.g. `.`).
    fn trigger_characters(&self) -> Vec<char> {
        Vec::new()
    }

    /// Returns the completions at `position` (0-based row, UTF-16 column).
    ///
    /// Called when the user presses Ctrl+Space, types a trigger character, types a
    /// word character outside strings and comments with
    /// [`EditorOptions::quick_suggestions`](crate::EditorOptions::quick_suggestions) on,
    /// or keeps typing after an [`incomplete`](CompletionList::incomplete) list.
    /// Errors are ignored.
    fn provide_completions(
        &self,
        context: &ProviderContext,
        position: PointUtf16,
        trigger: CompletionTrigger,
        cx: &mut App,
    ) -> Task<anyhow::Result<CompletionList>>;

    /// Fills in an item's documentation, detail or additional edits once it's
    /// selected (Monaco `resolveCompletionItem`); every field of the
    /// returned item replaces the original. `None` (the default) when
    /// there's nothing to resolve.
    fn resolve_completion_item(
        &self,
        _item: &CompletionItem,
        _cx: &mut App,
    ) -> Option<Task<anyhow::Result<CompletionItem>>> {
        None
    }
}

// ----- hover ---------------------------------------------------------------------

/// Monaco `Hover`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Hover {
    /// Markdown sections, shown in order after any decoration hover messages.
    pub contents: Vec<MarkdownString>,
    /// The hovered range, used to keep the hover open while the mouse stays
    /// in it; defaults to the word under the mouse.
    pub range: Option<Range<PointUtf16>>,
}

/// Supplies hover cards (Monaco `HoverProvider`).
///
/// Register with [`register_hover_provider`].
pub trait HoverProvider: 'static {
    /// Returns the hover for `position` (0-based row, UTF-16 column), or `None`.
    ///
    /// Called after the mouse rests over text for [`HOVER_DELAY`](crate::HOVER_DELAY),
    /// or right away for the show-hover command at the cursor. Contents from every
    /// provider are shown together; errors are ignored.
    fn provide_hover(
        &self,
        context: &ProviderContext,
        position: PointUtf16,
        cx: &mut App,
    ) -> Task<anyhow::Result<Option<Hover>>>;
}

// ----- definition ----------------------------------------------------------------

/// A location in a model (Monaco `Location`). `uri: None` means the same
/// model.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Location {
    /// The model's uri, or `None` for the model that was queried.
    pub uri: Option<SharedString>,
    /// The target range (0-based rows, UTF-16 columns).
    pub range: Range<PointUtf16>,
}

/// Resolves go-to-definition targets (Monaco `DefinitionProvider`).
///
/// Register with [`register_definition_provider`].
pub trait DefinitionProvider: 'static {
    /// Returns where the symbol at `position` (0-based row, UTF-16 column) is
    /// defined; empty when there is no definition.
    ///
    /// Called shortly after the mouse moves over a word with Cmd/Ctrl held (to show
    /// a link) and on Cmd/Ctrl+click or F12. Locations from every provider are
    /// combined; errors are ignored.
    fn provide_definition(
        &self,
        context: &ProviderContext,
        position: PointUtf16,
        cx: &mut App,
    ) -> Task<anyhow::Result<Vec<Location>>>;
}

/// Finds where the symbol at a position is declared (Monaco
/// `DeclarationProvider`).
///
/// Register with [`register_declaration_provider`].
pub trait DeclarationProvider: 'static {
    /// The declarations of the symbol at `position`.
    fn provide_declaration(
        &self,
        context: &ProviderContext,
        position: PointUtf16,
        cx: &mut App,
    ) -> Task<anyhow::Result<Vec<Location>>>;
}

/// Finds the type of the symbol at a position (Monaco
/// `TypeDefinitionProvider`).
///
/// Register with [`register_type_definition_provider`].
pub trait TypeDefinitionProvider: 'static {
    /// Where the type of the symbol at `position` is defined.
    fn provide_type_definition(
        &self,
        context: &ProviderContext,
        position: PointUtf16,
        cx: &mut App,
    ) -> Task<anyhow::Result<Vec<Location>>>;
}

/// Finds implementations of the symbol at a position (Monaco
/// `ImplementationProvider`).
///
/// Register with [`register_implementation_provider`].
pub trait ImplementationProvider: 'static {
    /// The implementations of the interface or abstract member at
    /// `position`.
    fn provide_implementation(
        &self,
        context: &ProviderContext,
        position: PointUtf16,
        cx: &mut App,
    ) -> Task<anyhow::Result<Vec<Location>>>;
}

/// Options for a references request (Monaco `ReferenceContext`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ReferenceContext {
    /// Include the declaration of the symbol.
    pub include_declaration: bool,
}

/// Finds references to the symbol at a position (Monaco
/// `ReferenceProvider`).
///
/// Register with [`register_reference_provider`].
pub trait ReferenceProvider: 'static {
    /// The references to the symbol at `position`.
    fn provide_references(
        &self,
        context: &ProviderContext,
        position: PointUtf16,
        reference_context: ReferenceContext,
        cx: &mut App,
    ) -> Task<anyhow::Result<Vec<Location>>>;
}

/// Edits to one or more documents (Monaco `WorkspaceEdit` with text edits).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct WorkspaceEdit {
    /// Edits by document uri; `None` is the document the request was about.
    pub edits: Vec<(Option<SharedString>, Vec<TextEdit>)>,
}

/// What a rename replaces (Monaco `RenameLocation`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RenameLocation {
    /// The range of the symbol.
    pub range: Range<PointUtf16>,
    /// Its current name, shown in the rename box.
    pub text: SharedString,
}

/// Renames symbols (Monaco `RenameProvider`).
///
/// Register with [`register_rename_provider`].
pub trait RenameProvider: 'static {
    /// The symbol at `position` and its name (Monaco
    /// `resolveRenameLocation`). `None` (the default) means the provider
    /// doesn't resolve locations, and the word at the cursor is renamed.
    /// The task returns `Ok(None)` to pass to the next provider, or an error
    /// whose message says why the symbol can't be renamed.
    fn resolve_rename_location(
        &self,
        _context: &ProviderContext,
        _position: PointUtf16,
        _cx: &mut App,
    ) -> Option<Task<anyhow::Result<Option<RenameLocation>>>> {
        None
    }

    /// The edits that rename the symbol at `position` to `new_name`.
    /// `Ok(None)` or an error (its message is the reason) passes to the next
    /// provider.
    fn provide_rename_edits(
        &self,
        context: &ProviderContext,
        position: PointUtf16,
        new_name: &str,
        cx: &mut App,
    ) -> Task<anyhow::Result<Option<WorkspaceEdit>>>;
}

// ----- code actions --------------------------------------------------------------

/// A hierarchical, dot-separated code action kind (Monaco
/// `CodeActionKind`), e.g. `quickfix` or `refactor.extract`.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct CodeActionKind(pub SharedString);

impl CodeActionKind {
    /// Fixes for diagnostics (`quickfix`).
    pub const QUICK_FIX: &str = "quickfix";
    /// Refactorings (`refactor`).
    pub const REFACTOR: &str = "refactor";
    /// Extracting a method, variable, … (`refactor.extract`).
    pub const REFACTOR_EXTRACT: &str = "refactor.extract";
    /// Inlining (`refactor.inline`).
    pub const REFACTOR_INLINE: &str = "refactor.inline";
    /// Moving code (`refactor.move`).
    pub const REFACTOR_MOVE: &str = "refactor.move";
    /// Rewriting code (`refactor.rewrite`).
    pub const REFACTOR_REWRITE: &str = "refactor.rewrite";
    /// Whole-file actions (`source`).
    pub const SOURCE: &str = "source";
    /// Organize imports (`source.organizeImports`).
    pub const SOURCE_ORGANIZE_IMPORTS: &str = "source.organizeImports";
    /// Fix all auto-fixable problems (`source.fixAll`).
    pub const SOURCE_FIX_ALL: &str = "source.fixAll";
    /// Surround with (`surround`).
    pub const SURROUND_WITH: &str = "surround";

    /// A kind from its dotted name.
    pub fn new(kind: impl Into<SharedString>) -> Self {
        Self(kind.into())
    }

    /// Whether `other` is this kind or a sub-kind of it (Monaco
    /// `HierarchicalKind.contains`); the empty kind contains everything.
    pub fn contains(&self, other: &CodeActionKind) -> bool {
        self.0.is_empty()
            || self.0 == other.0
            || other
                .0
                .strip_prefix(self.0.as_ref())
                .is_some_and(|rest| rest.starts_with('.'))
    }

    /// Whether either kind contains the other (Monaco
    /// `HierarchicalKind.intersects`).
    pub fn intersects(&self, other: &CodeActionKind) -> bool {
        self.contains(other) || other.contains(self)
    }
}

impl From<&str> for CodeActionKind {
    fn from(kind: &str) -> Self {
        Self(SharedString::from(kind.to_string()))
    }
}

/// A command to run (Monaco `Command`), executed with
/// [`execute_command`](crate::execute_command).
#[derive(Clone, Debug, PartialEq)]
pub struct Command {
    /// The registered command id.
    pub id: SharedString,
    /// Its title.
    pub title: SharedString,
    /// Its arguments.
    pub arguments: Vec<serde_json::Value>,
}

/// A change offered at a position (Monaco `CodeAction`): an edit, a
/// command, or both (the edit first).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct CodeAction {
    /// The menu text.
    pub title: SharedString,
    /// Its kind; it decides the menu group and which commands offer it.
    pub kind: Option<CodeActionKind>,
    /// The diagnostics it fixes.
    pub diagnostics: Vec<crate::Marker>,
    /// Edits to apply.
    pub edit: Option<WorkspaceEdit>,
    /// A command to run after the edit.
    pub command: Option<Command>,
    /// The best fix for its diagnostics; auto fix applies preferred fixes.
    pub is_preferred: bool,
    /// Why the action can't be applied now; it's shown greyed out.
    pub disabled: Option<SharedString>,
    /// Data for [`CodeActionProvider::resolve_code_action`].
    pub data: Option<serde_json::Value>,
}

/// How code actions were asked for (Monaco `CodeActionTriggerType`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CodeActionTrigger {
    /// A command or the lightbulb's menu.
    Invoke,
    /// Automatically, for the lightbulb, as the cursor moves.
    Auto,
}

/// What a code action request is about (Monaco `CodeActionContext`).
#[derive(Clone, Debug, PartialEq)]
pub struct CodeActionContext {
    /// Only actions of this kind are wanted, if set.
    pub only: Option<CodeActionKind>,
    /// How they were asked for.
    pub trigger: CodeActionTrigger,
    /// The markers (diagnostics) in the range.
    pub markers: Vec<crate::Marker>,
}

/// Offers code actions: quick fixes, refactorings and source actions
/// (Monaco `CodeActionProvider`).
///
/// Register with [`register_code_action_provider`].
pub trait CodeActionProvider: 'static {
    /// The kinds this provider can return, to skip it for others (Monaco
    /// `providedCodeActionKinds`). `None` means any.
    fn provided_kinds(&self) -> Option<Vec<CodeActionKind>> {
        None
    }

    /// The actions for `range`.
    fn provide_code_actions(
        &self,
        context: &ProviderContext,
        range: Range<PointUtf16>,
        action_context: &CodeActionContext,
        cx: &mut App,
    ) -> Task<anyhow::Result<Vec<CodeAction>>>;

    /// Fills in an action's edit before it's applied (Monaco
    /// `resolveCodeAction`); `None` (the default) when there's nothing to
    /// resolve.
    fn resolve_code_action(
        &self,
        _action: &CodeAction,
        _cx: &mut App,
    ) -> Option<Task<anyhow::Result<CodeAction>>> {
        None
    }
}

// ----- selection ranges ------------------------------------------------------------

/// Supplies the ranges smart select grows through (Monaco
/// `SelectionRangeProvider`).
///
/// Register with [`register_selection_range_provider`]. With none, the
/// syntax tree (or brackets) and words are used.
pub trait SelectionRangeProvider: 'static {
    /// For each of `positions`, ranges containing it (in any order).
    fn provide_selection_ranges(
        &self,
        context: &ProviderContext,
        positions: &[PointUtf16],
        cx: &mut App,
    ) -> Task<anyhow::Result<Vec<Vec<Range<PointUtf16>>>>>;
}

// ----- folding -------------------------------------------------------------------

/// What a folding range holds (Monaco `FoldingRangeKind`); commands such as
/// "fold all block comments" pick ranges by kind.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum FoldingRangeKind {
    /// A comment.
    Comment,
    /// Imports.
    Imports,
    /// A region from markers such as `#region`.
    Region,
    /// Another kind.
    Other(SharedString),
}

/// A foldable range of lines (Monaco `FoldingRange`): `start_row` stays
/// visible and the rows after it through `end_row` hide.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FoldingRange {
    /// The first row (0-based).
    pub start_row: u32,
    /// The last row folded (0-based, inclusive).
    pub end_row: u32,
    /// What the range holds.
    pub kind: Option<FoldingRangeKind>,
}

/// Computes folding ranges (Monaco `FoldingRangeProvider`), replacing the
/// syntax and indentation ranges with `folding_strategy: Auto`.
///
/// Register with [`register_folding_range_provider`]. Ranges from several
/// providers are merged.
pub trait FoldingRangeProvider: 'static {
    /// The ranges for the whole document, or `None` to leave folding to
    /// the next fallback (syntax, then indentation).
    fn provide_folding_ranges(
        &self,
        context: &ProviderContext,
        cx: &mut App,
    ) -> Task<anyhow::Result<Option<Vec<FoldingRange>>>>;
}

// ----- inlay hints ---------------------------------------------------------------

/// Monaco `InlayHintKind`: picks the hint's colors.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InlayHintKind {
    /// A type annotation.
    Type,
    /// A parameter name.
    Parameter,
}

/// Inline text the editor shows but that isn't part of the document
/// (Monaco `InlayHint`), such as inferred types or parameter names.
#[derive(Clone, Debug, PartialEq)]
pub struct InlayHint {
    /// Where it shows (0-based row, UTF-16 column).
    pub position: PointUtf16,
    /// Its text. (Monaco's label parts are joined; their locations and
    /// commands aren't supported yet.)
    pub label: SharedString,
    /// Its kind, if any.
    pub kind: Option<InlayHintKind>,
    /// Shown on hover.
    pub tooltip: Option<MarkdownString>,
    /// A little space before the hint.
    pub padding_left: bool,
    /// A little space after it.
    pub padding_right: bool,
}

/// Supplies inlay hints for a range (Monaco `InlayHintsProvider`).
///
/// Register with [`register_inlay_hints_provider`]. Call
/// [`refresh_inlay_hints`] when they change without an edit.
pub trait InlayHintsProvider: 'static {
    /// The hints in `range` (the visible lines and a screen's worth around
    /// them).
    fn provide_inlay_hints(
        &self,
        context: &ProviderContext,
        range: Range<PointUtf16>,
        cx: &mut App,
    ) -> Task<anyhow::Result<Vec<InlayHint>>>;
}

// ----- document symbols ----------------------------------------------------------

/// Monaco `SymbolKind`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[allow(missing_docs)]
pub enum SymbolKind {
    File,
    Module,
    Namespace,
    Package,
    Class,
    Method,
    Property,
    Field,
    Constructor,
    Enum,
    Interface,
    #[default]
    Function,
    Variable,
    Constant,
    String,
    Number,
    Boolean,
    Array,
    Object,
    Key,
    Null,
    EnumMember,
    Struct,
    Event,
    Operator,
    TypeParameter,
}

impl SymbolKind {
    /// Monaco's name for the kind, as shown when grouping symbols
    /// (`symbolKindNames`).
    pub fn name(self) -> &'static str {
        match self {
            SymbolKind::File => "files",
            SymbolKind::Module => "modules",
            SymbolKind::Namespace => "namespaces",
            SymbolKind::Package => "packages",
            SymbolKind::Class => "classes",
            SymbolKind::Method => "methods",
            SymbolKind::Property => "properties",
            SymbolKind::Field => "fields",
            SymbolKind::Constructor => "constructors",
            SymbolKind::Enum => "enumerations",
            SymbolKind::Interface => "interfaces",
            SymbolKind::Function => "functions",
            SymbolKind::Variable => "variables",
            SymbolKind::Constant => "constants",
            SymbolKind::String => "strings",
            SymbolKind::Number => "numbers",
            SymbolKind::Boolean => "booleans",
            SymbolKind::Array => "arrays",
            SymbolKind::Object => "objects",
            SymbolKind::Key => "keys",
            SymbolKind::Null => "null",
            SymbolKind::EnumMember => "enumeration members",
            SymbolKind::Struct => "structs",
            SymbolKind::Event => "events",
            SymbolKind::Operator => "operators",
            SymbolKind::TypeParameter => "type parameters",
        }
    }
}

/// A symbol in a document's outline (Monaco `DocumentSymbol`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct DocumentSymbol {
    /// Its name.
    pub name: SharedString,
    /// E.g. a signature.
    pub detail: Option<SharedString>,
    /// Its kind.
    pub kind: SymbolKind,
    /// Deprecated, by Monaco's `SymbolTag.Deprecated`.
    pub deprecated: bool,
    /// The whole definition.
    pub range: Range<PointUtf16>,
    /// The part to select and reveal, usually the name.
    pub selection_range: Range<PointUtf16>,
    /// Nested symbols.
    pub children: Vec<DocumentSymbol>,
}

/// Supplies a document's symbols (Monaco `DocumentSymbolProvider`), for go
/// to symbol, the outline and sticky scroll. Without one, the language's
/// `tags.scm` query is used.
///
/// Register with [`register_document_symbol_provider`].
pub trait DocumentSymbolProvider: 'static {
    /// The document's symbols, nested.
    fn provide_document_symbols(
        &self,
        context: &ProviderContext,
        cx: &mut App,
    ) -> Task<anyhow::Result<Vec<DocumentSymbol>>>;
}

// ----- code lens -----------------------------------------------------------------

/// A command shown above a line (Monaco `CodeLens`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct CodeLens {
    /// The range it's about; it shows above its first line.
    pub range: Range<PointUtf16>,
    /// What it shows and runs; `None` until resolved.
    pub command: Option<Command>,
}

/// Supplies code lenses (Monaco `CodeLensProvider`): clickable commands
/// above lines, such as "3 references".
///
/// Register with [`register_code_lens_provider`].
pub trait CodeLensProvider: 'static {
    /// The document's lenses.
    fn provide_code_lenses(
        &self,
        context: &ProviderContext,
        cx: &mut App,
    ) -> Task<anyhow::Result<Vec<CodeLens>>>;

    /// Fills in the command of a lens returned without one, when it
    /// scrolls into view.
    fn resolve_code_lens(
        &self,
        _context: &ProviderContext,
        _lens: &CodeLens,
        _cx: &mut App,
    ) -> Option<Task<anyhow::Result<CodeLens>>> {
        None
    }
}

// ----- linked editing ------------------------------------------------------------

/// Ranges edited together (Monaco `LinkedEditingRanges`).
#[derive(Clone, Debug, Default)]
pub struct LinkedEditingRanges {
    /// The ranges; the one containing the cursor is the reference.
    pub ranges: Vec<Range<PointUtf16>>,
    /// What the ranges may contain; typing anything else ends linked
    /// editing. Defaults to the language's word pattern.
    pub word_pattern: Option<syntax::Pattern>,
}

/// Supplies ranges that change together, such as an HTML element's start
/// and end tag names (Monaco `LinkedEditingRangeProvider`).
///
/// Register with [`register_linked_editing_range_provider`]. Without one,
/// the editor links HTML tag names from the syntax tree.
pub trait LinkedEditingRangeProvider: 'static {
    /// The ranges linked with the one at `position`, if any.
    fn provide_linked_editing_ranges(
        &self,
        context: &ProviderContext,
        position: PointUtf16,
        cx: &mut App,
    ) -> Task<anyhow::Result<Option<LinkedEditingRanges>>>;
}

// ----- inline completions --------------------------------------------------------

/// Why inline completions are requested (Monaco
/// `InlineCompletionTriggerKind`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InlineCompletionTriggerKind {
    /// While typing.
    Automatic,
    /// By `editor.action.inlineSuggest.trigger` or cycling.
    Explicit,
}

/// The suggest widget's selected item, for previews (Monaco
/// `SelectedSuggestionInfo`).
#[derive(Clone, Debug, PartialEq)]
pub struct SelectedSuggestionInfo {
    /// The range it replaces.
    pub range: Range<PointUtf16>,
    /// Its text.
    pub text: String,
    /// Its kind.
    pub completion_kind: CompletionItemKind,
    /// Whether `text` is a snippet.
    pub is_snippet_text: bool,
}

/// Monaco `InlineCompletionContext`.
#[derive(Clone, Debug, PartialEq)]
pub struct InlineCompletionContext {
    /// Why they're requested.
    pub trigger_kind: InlineCompletionTriggerKind,
    /// The suggest widget's selection, while it's open.
    pub selected_suggestion_info: Option<SelectedSuggestionInfo>,
}

/// One inline completion (Monaco `InlineCompletion`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct InlineCompletion {
    /// The text to insert.
    pub insert_text: String,
    /// `insert_text` is a snippet.
    pub is_snippet: bool,
    /// The single-line range it replaces; by default from the start of the
    /// word at the cursor to the end of the line.
    pub range: Option<Range<PointUtf16>>,
    /// Run after accepting.
    pub command: Option<Command>,
    /// Edits elsewhere applied on accept.
    pub additional_text_edits: Vec<(Range<PointUtf16>, String)>,
}

/// What an inline completions provider returns (Monaco
/// `InlineCompletions`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct InlineCompletionList {
    /// The completions, best first.
    pub items: Vec<InlineCompletion>,
    /// Keep the suggest widget from opening by itself while one shows.
    pub suppress_suggestions: Option<bool>,
}

/// Supplies ghost text at the cursor (Monaco `InlineCompletionsProvider`).
///
/// Register with [`register_inline_completions_provider`].
pub trait InlineCompletionsProvider: 'static {
    /// Completions at `position`.
    fn provide_inline_completions(
        &self,
        context: &ProviderContext,
        position: PointUtf16,
        completion_context: InlineCompletionContext,
        cx: &mut App,
    ) -> Task<anyhow::Result<InlineCompletionList>>;

    /// How long to wait after typing before asking (Monaco
    /// `debounceDelayMs`); `None` uses the editor's 50ms.
    fn debounce_delay(&self) -> Option<std::time::Duration> {
        None
    }

    /// `item` was shown for the first time.
    fn handle_item_did_show(&self, _item: &InlineCompletion) {}

    /// The first `accepted_length` UTF-16 units of `item` were accepted
    /// word by word or line by line.
    fn handle_partial_accept(&self, _item: &InlineCompletion, _accepted_length: usize) {}
}

// ----- semantic tokens -------------------------------------------------------------

/// The token types and modifiers a semantic tokens provider's data
/// indexes (Monaco `SemanticTokensLegend`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SemanticTokensLegend {
    /// Token types, e.g. `variable`, by index.
    pub token_types: Vec<SharedString>,
    /// Modifiers, e.g. `readonly`, by bit.
    pub token_modifiers: Vec<SharedString>,
}

/// Semantic tokens (Monaco `SemanticTokens`): five numbers per token,
/// relative to the previous one: line delta, start (relative on the same
/// line), length (UTF-16), type index and modifier bits.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SemanticTokens {
    /// Identifies this result for a later delta request.
    pub result_id: Option<SharedString>,
    /// The encoded tokens.
    pub data: Vec<u32>,
}

/// A change to a previous result's data (Monaco `SemanticTokensEdit`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SemanticTokensEdit {
    /// Index in the data array.
    pub start: usize,
    /// How many numbers to remove.
    pub delete_count: usize,
    /// What to insert.
    pub data: Vec<u32>,
}

/// A delta against the previous result (Monaco `SemanticTokensEdits`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SemanticTokensEdits {
    /// Identifies this result for the next delta request.
    pub result_id: Option<SharedString>,
    /// The changes, in order.
    pub edits: Vec<SemanticTokensEdit>,
}

/// What a document semantic tokens provider returns.
#[derive(Clone, Debug, PartialEq)]
pub enum SemanticTokensResult {
    /// All tokens.
    Tokens(SemanticTokens),
    /// Changes to the previous result.
    Edits(SemanticTokensEdits),
}

/// Supplies semantic tokens for a whole document (Monaco
/// `DocumentSemanticTokensProvider`). They color the text over the syntax
/// highlighting when `semantic_highlighting` is on.
///
/// Register with [`register_document_semantic_tokens_provider`]; call
/// [`refresh_semantic_tokens`] when they change without an edit.
pub trait DocumentSemanticTokensProvider: 'static {
    /// The types and modifiers the data refers to.
    fn legend(&self) -> SemanticTokensLegend;

    /// The document's tokens, or edits to the result `last_result_id`
    /// names. `None` for no tokens.
    fn provide_document_semantic_tokens(
        &self,
        context: &ProviderContext,
        last_result_id: Option<SharedString>,
        cx: &mut App,
    ) -> Task<anyhow::Result<Option<SemanticTokensResult>>>;

    /// The editor no longer needs result `result_id`.
    fn release_document_semantic_tokens(&self, _result_id: Option<SharedString>) {}
}

/// Supplies semantic tokens for a range (Monaco
/// `DocumentRangeSemanticTokensProvider`), used for the visible lines until
/// a document provider answers.
///
/// Register with [`register_document_range_semantic_tokens_provider`].
pub trait DocumentRangeSemanticTokensProvider: 'static {
    /// The types and modifiers the data refers to.
    fn legend(&self) -> SemanticTokensLegend;

    /// The tokens in `range` (lines relative to the document start).
    fn provide_document_range_semantic_tokens(
        &self,
        context: &ProviderContext,
        range: Range<PointUtf16>,
        cx: &mut App,
    ) -> Task<anyhow::Result<Option<SemanticTokens>>>;
}

/// Bumped by [`refresh_semantic_tokens`]; editors re-request when it
/// changes.
#[derive(Default)]
pub(crate) struct SemanticTokensEpoch(pub u64);

impl Global for SemanticTokensEpoch {}

/// Repaints the editor when providers are registered or removed, or a
/// refresh is requested, so it notices.
pub(crate) fn observe_language_features<V: 'static>(
    cx: &mut gpui::Context<V>,
) -> Vec<gpui::Subscription> {
    vec![
        cx.observe_global::<LanguageFeatures>(|_, cx| cx.notify()),
        cx.observe_global::<SemanticTokensEpoch>(|_, cx| cx.notify()),
        cx.observe_global::<InlayHintsEpoch>(|_, cx| cx.notify()),
    ]
}

/// Asks every editor for semantic tokens again (Monaco's provider
/// `onDidChange`), e.g. after `workspace/semanticTokens/refresh`.
pub fn refresh_semantic_tokens(cx: &mut App) {
    cx.default_global::<SemanticTokensEpoch>().0 += 1;
}

// ----- links ---------------------------------------------------------------------

/// A link in the text (Monaco `ILink`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct DocumentLink {
    /// Where it is.
    pub range: Range<PointUtf16>,
    /// Its target; `None` until [`LinkProvider::resolve_link`] fills it in.
    pub url: Option<SharedString>,
    /// The hover label instead of "Follow link".
    pub tooltip: Option<SharedString>,
}

/// Finds links in a document (Monaco `LinkProvider`). The editor also
/// detects URLs itself (Monaco's link computer); links from providers
/// replace the ones they overlap.
///
/// Register with [`register_link_provider`].
pub trait LinkProvider: 'static {
    /// The document's links.
    fn provide_links(
        &self,
        context: &ProviderContext,
        cx: &mut App,
    ) -> Task<anyhow::Result<Vec<DocumentLink>>>;

    /// Fills in the target of a link returned without one.
    fn resolve_link(
        &self,
        _link: &DocumentLink,
        _cx: &mut App,
    ) -> Option<Task<anyhow::Result<DocumentLink>>> {
        None
    }
}

/// Bumped by [`refresh_inlay_hints`]; editors re-request when it changes.
#[derive(Default)]
pub(crate) struct InlayHintsEpoch(pub u64);

impl Global for InlayHintsEpoch {}

/// Asks every editor to request inlay hints again (Monaco
/// `onDidChangeInlayHints`), e.g. after a language server's
/// `workspace/inlayHint/refresh`.
pub fn refresh_inlay_hints(cx: &mut App) {
    cx.default_global::<InlayHintsEpoch>().0 += 1;
}

// ----- document highlights -------------------------------------------------------

/// What an occurrence highlight marks (Monaco `DocumentHighlightKind`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum DocumentHighlightKind {
    /// A textual occurrence.
    #[default]
    Text,
    /// A read of a symbol.
    Read,
    /// A write to a symbol; drawn with the stronger color.
    Write,
}

/// One occurrence of the symbol at the cursor (Monaco `DocumentHighlight`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DocumentHighlight {
    /// The occurrence's range (0-based rows, UTF-16 columns).
    pub range: Range<PointUtf16>,
    /// Text, read or write.
    pub kind: DocumentHighlightKind,
}

/// Finds the occurrences of the symbol at a position, for occurrence
/// highlighting (Monaco `DocumentHighlightProvider`). Without one, the editor
/// highlights the word's textual occurrences.
///
/// Register with [`register_document_highlight_provider`].
pub trait DocumentHighlightProvider: 'static {
    /// The occurrences to highlight for the symbol at `position` (0-based row,
    /// UTF-16 column), or `None` for none. Called after the cursor rests on a
    /// word for [`EditorOptions::occurrences_highlight_delay`](crate::EditorOptions::occurrences_highlight_delay).
    fn provide_document_highlights(
        &self,
        context: &ProviderContext,
        position: PointUtf16,
        cx: &mut App,
    ) -> Task<anyhow::Result<Option<Vec<DocumentHighlight>>>>;
}

// ----- signature help ------------------------------------------------------------

/// Monaco `ParameterInformation`; `label` is a substring of the signature
/// label or a UTF-16 range into it.
#[derive(Clone, Debug, PartialEq)]
pub struct ParameterInformation {
    /// The parameter's position in the signature label.
    pub label: ParameterLabel,
    /// Shown below the signature while this parameter is active.
    pub documentation: Option<MarkdownString>,
}

/// Where a parameter appears in its signature label.
#[derive(Clone, Debug, PartialEq)]
pub enum ParameterLabel {
    /// The first occurrence of this text in the label.
    Text(SharedString),
    /// A UTF-16 offset range into the label.
    Range(Range<usize>),
}

/// One signature of a callable (Monaco `SignatureInformation`).
#[derive(Clone, Debug, PartialEq)]
pub struct SignatureInformation {
    /// The full signature text; the active parameter is highlighted in it.
    pub label: SharedString,
    /// Shown below the signature.
    pub documentation: Option<MarkdownString>,
    /// The parameters, in order.
    pub parameters: Vec<ParameterInformation>,
    /// Overrides `SignatureHelp::active_parameter` for this signature.
    pub active_parameter: Option<usize>,
}

/// Monaco `SignatureHelp`.
#[derive(Clone, Debug, PartialEq)]
pub struct SignatureHelp {
    /// The available signatures (e.g. overloads). Empty counts as no result.
    pub signatures: Vec<SignatureInformation>,
    /// Index into `signatures` of the one to show.
    pub active_signature: usize,
    /// Index of the highlighted parameter, unless the signature overrides it.
    pub active_parameter: usize,
}

/// Why signature help was requested (Monaco `SignatureHelpTriggerKind`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SignatureHelpTrigger {
    /// Explicitly requested (Cmd/Ctrl+Shift+Space).
    Invoke,
    /// Typing a trigger character, or a retrigger character while it's shown.
    TriggerCharacter(char),
    /// The text or cursor changed while signature help is shown (clicking
    /// elsewhere closes it instead).
    ContentChange,
}

/// Supplies parameter hints (Monaco `SignatureHelpProvider`).
///
/// Register with [`register_signature_help_provider`].
pub trait SignatureHelpProvider: 'static {
    /// Characters that open signature help (e.g. `(`, `,`).
    fn trigger_characters(&self) -> Vec<char> {
        vec!['(', ',']
    }

    /// Characters that re-query while it's open (e.g. `)`).
    fn retrigger_characters(&self) -> Vec<char> {
        Vec::new()
    }

    /// Returns the signatures for the call around `position` (0-based row, UTF-16
    /// column), or `None` to close parameter hints.
    ///
    /// Called on trigger characters, when explicitly requested, and on every edit
    /// while parameter hints are shown. The first provider (in registration order)
    /// returning a non-empty result wins; errors are ignored.
    fn provide_signature_help(
        &self,
        context: &ProviderContext,
        position: PointUtf16,
        trigger: SignatureHelpTrigger,
        cx: &mut App,
    ) -> Task<anyhow::Result<Option<SignatureHelp>>>;
}

// ----- formatting ---------------------------------------------------------------

/// A replacement of `range` (0-based rows, UTF-16 columns) with the text
/// (Monaco `TextEdit`). A list of edits refers to the text before any of
/// them, and they must not overlap.
pub type TextEdit = (Range<PointUtf16>, String);

/// How the model is indented, for formatters (Monaco `FormattingOptions`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FormattingOptions {
    /// Spaces per indentation level (and per tab).
    pub tab_size: u32,
    /// Indent with spaces rather than tabs.
    pub insert_spaces: bool,
}

/// Formats a whole document (Monaco `DocumentFormattingEditProvider`).
///
/// Register with [`register_document_formatting_provider`].
pub trait DocumentFormattingProvider: 'static {
    /// The edits that format the document. Big replacements are fine: the
    /// editor reduces them to the characters that actually change.
    fn provide_document_formatting_edits(
        &self,
        context: &ProviderContext,
        options: FormattingOptions,
        cx: &mut App,
    ) -> Task<anyhow::Result<Vec<TextEdit>>>;
}

/// Formats part of a document (Monaco
/// `DocumentRangeFormattingEditProvider`). Also formats whole documents
/// when no [`DocumentFormattingProvider`] is registered, and pasted text
/// with `format_on_paste`.
///
/// Register with [`register_document_range_formatting_provider`].
pub trait DocumentRangeFormattingProvider: 'static {
    /// The edits that format `range`.
    fn provide_document_range_formatting_edits(
        &self,
        context: &ProviderContext,
        range: Range<PointUtf16>,
        options: FormattingOptions,
        cx: &mut App,
    ) -> Task<anyhow::Result<Vec<TextEdit>>>;
}

/// Formats as you type (Monaco `OnTypeFormattingEditProvider`), with the
/// `format_on_type` option.
///
/// Register with [`register_on_type_formatting_provider`].
pub trait OnTypeFormattingProvider: 'static {
    /// Characters that trigger formatting when typed (e.g. `;` or `}`).
    fn auto_format_trigger_characters(&self) -> Vec<char>;

    /// The edits after `ch` was typed, leaving the cursor at `position`.
    fn provide_on_type_formatting_edits(
        &self,
        context: &ProviderContext,
        position: PointUtf16,
        ch: char,
        options: FormattingOptions,
        cx: &mut App,
    ) -> Task<anyhow::Result<Vec<TextEdit>>>;
}

// ----- registry ------------------------------------------------------------------

/// Identifies a registration, for [`unregister_provider`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ProviderRegistration(u64);

struct Entry<T: ?Sized> {
    id: ProviderRegistration,
    selector: LanguageSelector,
    provider: Arc<T>,
}

impl<T: ?Sized> Clone for Entry<T> {
    fn clone(&self) -> Self {
        Self {
            id: self.id,
            selector: self.selector.clone(),
            provider: self.provider.clone(),
        }
    }
}

#[derive(Default)]
struct LanguageFeatures {
    next_id: u64,
    completion: Vec<Entry<dyn CompletionProvider>>,
    hover: Vec<Entry<dyn HoverProvider>>,
    definition: Vec<Entry<dyn DefinitionProvider>>,
    signature_help: Vec<Entry<dyn SignatureHelpProvider>>,
    document_highlight: Vec<Entry<dyn DocumentHighlightProvider>>,
    rename: Vec<Entry<dyn RenameProvider>>,
    code_action: Vec<Entry<dyn CodeActionProvider>>,
    inlay_hints: Vec<Entry<dyn InlayHintsProvider>>,
    links: Vec<Entry<dyn LinkProvider>>,
    semantic_tokens: Vec<Entry<dyn DocumentSemanticTokensProvider>>,
    inline_completions: Vec<Entry<dyn InlineCompletionsProvider>>,
    linked_editing: Vec<Entry<dyn LinkedEditingRangeProvider>>,
    code_lens: Vec<Entry<dyn CodeLensProvider>>,
    document_symbols: Vec<Entry<dyn DocumentSymbolProvider>>,
    range_semantic_tokens: Vec<Entry<dyn DocumentRangeSemanticTokensProvider>>,
    folding: Vec<Entry<dyn FoldingRangeProvider>>,
    selection_range: Vec<Entry<dyn SelectionRangeProvider>>,
    declaration: Vec<Entry<dyn DeclarationProvider>>,
    type_definition: Vec<Entry<dyn TypeDefinitionProvider>>,
    implementation: Vec<Entry<dyn ImplementationProvider>>,
    references: Vec<Entry<dyn ReferenceProvider>>,
    document_formatting: Vec<Entry<dyn DocumentFormattingProvider>>,
    range_formatting: Vec<Entry<dyn DocumentRangeFormattingProvider>>,
    on_type_formatting: Vec<Entry<dyn OnTypeFormattingProvider>>,
}

impl Global for LanguageFeatures {}

fn features(cx: &mut App) -> &mut LanguageFeatures {
    cx.default_global::<LanguageFeatures>()
}

macro_rules! register_fn {
    ($(#[$meta:meta])* $name:ident, $field:ident, $trait:ident) => {
        $(#[$meta])*
        pub fn $name(
            cx: &mut App,
            selector: LanguageSelector,
            provider: impl $trait,
        ) -> ProviderRegistration {
            let features = features(cx);
            features.next_id += 1;
            let id = ProviderRegistration(features.next_id);
            features.$field.push(Entry {
                id,
                selector,
                provider: Arc::new(provider),
            });
            id
        }
    };
}

register_fn!(
    /// Registers a completion provider for the models `selector` matches (Monaco
    /// `registerCompletionItemProvider`). Results from all matching providers are merged.
    register_completion_provider,
    completion,
    CompletionProvider
);
register_fn!(
    /// Registers a hover provider for the models `selector` matches (Monaco
    /// `registerHoverProvider`). Results from all matching providers are shown together.
    register_hover_provider,
    hover,
    HoverProvider
);
register_fn!(
    /// Registers a definition provider for the models `selector` matches (Monaco
    /// `registerDefinitionProvider`). Results from all matching providers are combined.
    register_definition_provider,
    definition,
    DefinitionProvider
);
register_fn!(
    /// Registers a document highlight provider for the models `selector` matches
    /// (Monaco `registerDocumentHighlightProvider`). The first non-empty result, in
    /// registration order, is shown; with none, the word's textual occurrences are.
    register_document_highlight_provider,
    document_highlight,
    DocumentHighlightProvider
);

register_fn!(
    /// Registers a signature help provider for the models `selector` matches (Monaco
    /// `registerSignatureHelpProvider`). The first non-empty result, in registration order,
    /// is shown.
    register_signature_help_provider,
    signature_help,
    SignatureHelpProvider
);

register_fn!(
    /// Registers a selection range provider for the models `selector` matches (Monaco
    /// `registerSelectionRangeProvider`). Results from all matching providers are merged.
    register_selection_range_provider,
    selection_range,
    SelectionRangeProvider
);
register_fn!(
    /// Registers a folding range provider for the models `selector` matches (Monaco
    /// `registerFoldingRangeProvider`). Results from all matching providers are merged.
    register_folding_range_provider,
    folding,
    FoldingRangeProvider
);
register_fn!(
    /// Registers a document symbol provider for the models `selector`
    /// matches (Monaco `registerDocumentSymbolProvider`).
    register_document_symbol_provider,
    document_symbols,
    DocumentSymbolProvider
);
register_fn!(
    /// Registers a code lens provider for the models `selector` matches
    /// (Monaco `registerCodeLensProvider`). Lenses from all are shown.
    register_code_lens_provider,
    code_lens,
    CodeLensProvider
);
register_fn!(
    /// Registers a linked editing range provider for the models `selector`
    /// matches (Monaco `registerLinkedEditingRangeProvider`). The first with
    /// ranges wins.
    register_linked_editing_range_provider,
    linked_editing,
    LinkedEditingRangeProvider
);
register_fn!(
    /// Registers an inline completions provider for the models `selector`
    /// matches (Monaco `registerInlineCompletionsProvider`).
    register_inline_completions_provider,
    inline_completions,
    InlineCompletionsProvider
);
register_fn!(
    /// Registers a document semantic tokens provider for the models
    /// `selector` matches (Monaco `registerDocumentSemanticTokensProvider`).
    /// The most recently registered one with tokens is used.
    register_document_semantic_tokens_provider,
    semantic_tokens,
    DocumentSemanticTokensProvider
);
register_fn!(
    /// Registers a range semantic tokens provider for the models `selector`
    /// matches (Monaco `registerDocumentRangeSemanticTokensProvider`).
    register_document_range_semantic_tokens_provider,
    range_semantic_tokens,
    DocumentRangeSemanticTokensProvider
);
register_fn!(
    /// Registers a link provider for the models `selector` matches (Monaco
    /// `registerLinkProvider`). Later registrations win where links overlap.
    register_link_provider,
    links,
    LinkProvider
);
register_fn!(
    /// Registers an inlay hints provider for the models `selector` matches (Monaco
    /// `registerInlayHintsProvider`). Results from all matching providers are merged.
    register_inlay_hints_provider,
    inlay_hints,
    InlayHintsProvider
);
register_fn!(
    /// Registers a code action provider for the models `selector` matches (Monaco
    /// `registerCodeActionProvider`). Results from all matching providers are merged.
    register_code_action_provider,
    code_action,
    CodeActionProvider
);
register_fn!(
    /// Registers a rename provider for the models `selector` matches (Monaco
    /// `registerRenameProvider`). Providers are tried in registration order.
    register_rename_provider,
    rename,
    RenameProvider
);
register_fn!(
    /// Registers a declaration provider for the models `selector` matches (Monaco
    /// `registerDeclarationProvider`). Results from all matching providers are combined.
    register_declaration_provider,
    declaration,
    DeclarationProvider
);
register_fn!(
    /// Registers a type definition provider for the models `selector` matches (Monaco
    /// `registerTypeDefinitionProvider`). Results from all matching providers are combined.
    register_type_definition_provider,
    type_definition,
    TypeDefinitionProvider
);
register_fn!(
    /// Registers an implementation provider for the models `selector` matches (Monaco
    /// `registerImplementationProvider`). Results from all matching providers are combined.
    register_implementation_provider,
    implementation,
    ImplementationProvider
);
register_fn!(
    /// Registers a reference provider for the models `selector` matches (Monaco
    /// `registerReferenceProvider`). Results from all matching providers are combined.
    register_reference_provider,
    references,
    ReferenceProvider
);
register_fn!(
    /// Registers a document formatter for the models `selector` matches (Monaco
    /// `registerDocumentFormattingEditProvider`). The first one registered is used.
    register_document_formatting_provider,
    document_formatting,
    DocumentFormattingProvider
);
register_fn!(
    /// Registers a range formatter for the models `selector` matches (Monaco
    /// `registerDocumentRangeFormattingEditProvider`). The first one registered is used.
    register_document_range_formatting_provider,
    range_formatting,
    DocumentRangeFormattingProvider
);
register_fn!(
    /// Registers an on-type formatter for the models `selector` matches (Monaco
    /// `registerOnTypeFormattingEditProvider`). The first one registered is used.
    register_on_type_formatting_provider,
    on_type_formatting,
    OnTypeFormattingProvider
);

/// Removes a provider registered with any `register_*_provider`.
pub fn unregister_provider(cx: &mut App, registration: ProviderRegistration) {
    let features = features(cx);
    features.document_highlight.retain(|e| e.id != registration);
    features.completion.retain(|e| e.id != registration);
    features.hover.retain(|e| e.id != registration);
    features.definition.retain(|e| e.id != registration);
    features.signature_help.retain(|e| e.id != registration);
    features
        .document_formatting
        .retain(|e| e.id != registration);
    features.range_formatting.retain(|e| e.id != registration);
    features.declaration.retain(|e| e.id != registration);
    features.rename.retain(|e| e.id != registration);
    features.code_action.retain(|e| e.id != registration);
    features.inlay_hints.retain(|e| e.id != registration);
    features.links.retain(|e| e.id != registration);
    features.semantic_tokens.retain(|e| e.id != registration);
    features.inline_completions.retain(|e| e.id != registration);
    features.linked_editing.retain(|e| e.id != registration);
    features.code_lens.retain(|e| e.id != registration);
    features.document_symbols.retain(|e| e.id != registration);
    features
        .range_semantic_tokens
        .retain(|e| e.id != registration);
    features.folding.retain(|e| e.id != registration);
    features.selection_range.retain(|e| e.id != registration);
    features.type_definition.retain(|e| e.id != registration);
    features.implementation.retain(|e| e.id != registration);
    features.references.retain(|e| e.id != registration);
    features.on_type_formatting.retain(|e| e.id != registration);
}

fn matching<T: ?Sized>(entries: &[Entry<T>], language_id: Option<&str>) -> Vec<Arc<T>> {
    entries
        .iter()
        .filter(|e| e.selector.matches(language_id))
        .map(|e| e.provider.clone())
        .collect()
}

/// The registered completion providers whose selector matches `language_id`, in
/// registration order.
pub fn completion_providers(
    cx: &App,
    language_id: Option<&str>,
) -> Vec<Arc<dyn CompletionProvider>> {
    cx.try_global::<LanguageFeatures>()
        .map(|f| matching(&f.completion, language_id))
        .unwrap_or_default()
}

/// The registered hover providers whose selector matches `language_id`, in
/// registration order.
pub fn hover_providers(cx: &App, language_id: Option<&str>) -> Vec<Arc<dyn HoverProvider>> {
    cx.try_global::<LanguageFeatures>()
        .map(|f| matching(&f.hover, language_id))
        .unwrap_or_default()
}

/// The registered definition providers whose selector matches `language_id`, in
/// registration order.
pub fn definition_providers(
    cx: &App,
    language_id: Option<&str>,
) -> Vec<Arc<dyn DefinitionProvider>> {
    cx.try_global::<LanguageFeatures>()
        .map(|f| matching(&f.definition, language_id))
        .unwrap_or_default()
}

/// The registered signature help providers whose selector matches `language_id`,
/// in registration order.
pub fn signature_help_providers(
    cx: &App,
    language_id: Option<&str>,
) -> Vec<Arc<dyn SignatureHelpProvider>> {
    cx.try_global::<LanguageFeatures>()
        .map(|f| matching(&f.signature_help, language_id))
        .unwrap_or_default()
}

/// The registered document highlight providers whose selector matches
/// `language_id`, in registration order.
pub fn document_highlight_providers(
    cx: &App,
    language_id: Option<&str>,
) -> Vec<Arc<dyn DocumentHighlightProvider>> {
    cx.try_global::<LanguageFeatures>()
        .map(|f| matching(&f.document_highlight, language_id))
        .unwrap_or_default()
}

/// The registered document formatters whose selector matches `language_id`,
/// in registration order.
pub fn document_formatting_providers(
    cx: &App,
    language_id: Option<&str>,
) -> Vec<Arc<dyn DocumentFormattingProvider>> {
    cx.try_global::<LanguageFeatures>()
        .map(|f| matching(&f.document_formatting, language_id))
        .unwrap_or_default()
}

/// The registered range formatters whose selector matches `language_id`, in
/// registration order.
pub fn document_range_formatting_providers(
    cx: &App,
    language_id: Option<&str>,
) -> Vec<Arc<dyn DocumentRangeFormattingProvider>> {
    cx.try_global::<LanguageFeatures>()
        .map(|f| matching(&f.range_formatting, language_id))
        .unwrap_or_default()
}

/// The registered on-type formatters whose selector matches `language_id`,
/// in registration order.
pub fn on_type_formatting_providers(
    cx: &App,
    language_id: Option<&str>,
) -> Vec<Arc<dyn OnTypeFormattingProvider>> {
    cx.try_global::<LanguageFeatures>()
        .map(|f| matching(&f.on_type_formatting, language_id))
        .unwrap_or_default()
}

/// The registered declaration providers whose selector matches `language_id`, in
/// registration order.
pub fn declaration_providers(
    cx: &App,
    language_id: Option<&str>,
) -> Vec<Arc<dyn DeclarationProvider>> {
    cx.try_global::<LanguageFeatures>()
        .map(|f| matching(&f.declaration, language_id))
        .unwrap_or_default()
}

/// The registered type definition providers whose selector matches `language_id`, in
/// registration order.
pub fn type_definition_providers(
    cx: &App,
    language_id: Option<&str>,
) -> Vec<Arc<dyn TypeDefinitionProvider>> {
    cx.try_global::<LanguageFeatures>()
        .map(|f| matching(&f.type_definition, language_id))
        .unwrap_or_default()
}

/// The registered implementation providers whose selector matches `language_id`, in
/// registration order.
pub fn implementation_providers(
    cx: &App,
    language_id: Option<&str>,
) -> Vec<Arc<dyn ImplementationProvider>> {
    cx.try_global::<LanguageFeatures>()
        .map(|f| matching(&f.implementation, language_id))
        .unwrap_or_default()
}

/// The registered reference providers whose selector matches `language_id`, in
/// registration order.
pub fn reference_providers(cx: &App, language_id: Option<&str>) -> Vec<Arc<dyn ReferenceProvider>> {
    cx.try_global::<LanguageFeatures>()
        .map(|f| matching(&f.references, language_id))
        .unwrap_or_default()
}

/// The registered rename providers whose selector matches `language_id`, in
/// registration order.
pub fn rename_providers(cx: &App, language_id: Option<&str>) -> Vec<Arc<dyn RenameProvider>> {
    cx.try_global::<LanguageFeatures>()
        .map(|f| matching(&f.rename, language_id))
        .unwrap_or_default()
}

/// The registered code action providers whose selector matches
/// `language_id`, in registration order.
pub fn code_action_providers(
    cx: &App,
    language_id: Option<&str>,
) -> Vec<Arc<dyn CodeActionProvider>> {
    cx.try_global::<LanguageFeatures>()
        .map(|f| matching(&f.code_action, language_id))
        .unwrap_or_default()
}

/// The registered document symbol providers whose selector matches
/// `language_id`, in registration order.
pub fn document_symbol_providers(
    cx: &App,
    language_id: Option<&str>,
) -> Vec<Arc<dyn DocumentSymbolProvider>> {
    cx.try_global::<LanguageFeatures>()
        .map(|f| matching(&f.document_symbols, language_id))
        .unwrap_or_default()
}

/// The registered code lens providers whose selector matches
/// `language_id`, in registration order.
pub fn code_lens_providers(cx: &App, language_id: Option<&str>) -> Vec<Arc<dyn CodeLensProvider>> {
    cx.try_global::<LanguageFeatures>()
        .map(|f| matching(&f.code_lens, language_id))
        .unwrap_or_default()
}

/// The registered linked editing range providers whose selector matches
/// `language_id`, in registration order.
pub fn linked_editing_range_providers(
    cx: &App,
    language_id: Option<&str>,
) -> Vec<Arc<dyn LinkedEditingRangeProvider>> {
    cx.try_global::<LanguageFeatures>()
        .map(|f| matching(&f.linked_editing, language_id))
        .unwrap_or_default()
}

/// The registered inline completions providers whose selector matches
/// `language_id`, in registration order.
pub fn inline_completions_providers(
    cx: &App,
    language_id: Option<&str>,
) -> Vec<Arc<dyn InlineCompletionsProvider>> {
    cx.try_global::<LanguageFeatures>()
        .map(|f| matching(&f.inline_completions, language_id))
        .unwrap_or_default()
}

/// The registered document semantic tokens providers whose selector
/// matches `language_id`, in registration order.
pub fn document_semantic_tokens_providers(
    cx: &App,
    language_id: Option<&str>,
) -> Vec<Arc<dyn DocumentSemanticTokensProvider>> {
    cx.try_global::<LanguageFeatures>()
        .map(|f| matching(&f.semantic_tokens, language_id))
        .unwrap_or_default()
}

/// The registered range semantic tokens providers whose selector matches
/// `language_id`, in registration order.
pub fn document_range_semantic_tokens_providers(
    cx: &App,
    language_id: Option<&str>,
) -> Vec<Arc<dyn DocumentRangeSemanticTokensProvider>> {
    cx.try_global::<LanguageFeatures>()
        .map(|f| matching(&f.range_semantic_tokens, language_id))
        .unwrap_or_default()
}

/// The registered link providers whose selector matches `language_id`, in
/// registration order.
pub fn link_providers(cx: &App, language_id: Option<&str>) -> Vec<Arc<dyn LinkProvider>> {
    cx.try_global::<LanguageFeatures>()
        .map(|f| matching(&f.links, language_id))
        .unwrap_or_default()
}

/// The registered inlay hints providers whose selector matches
/// `language_id`, in registration order.
pub fn inlay_hints_providers(
    cx: &App,
    language_id: Option<&str>,
) -> Vec<Arc<dyn InlayHintsProvider>> {
    cx.try_global::<LanguageFeatures>()
        .map(|f| matching(&f.inlay_hints, language_id))
        .unwrap_or_default()
}

/// The registered folding range providers whose selector matches
/// `language_id`, in registration order.
pub fn folding_range_providers(
    cx: &App,
    language_id: Option<&str>,
) -> Vec<Arc<dyn FoldingRangeProvider>> {
    cx.try_global::<LanguageFeatures>()
        .map(|f| matching(&f.folding, language_id))
        .unwrap_or_default()
}

/// The registered selection range providers whose selector matches
/// `language_id`, in registration order.
pub fn selection_range_providers(
    cx: &App,
    language_id: Option<&str>,
) -> Vec<Arc<dyn SelectionRangeProvider>> {
    cx.try_global::<LanguageFeatures>()
        .map(|f| matching(&f.selection_range, language_id))
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kinds_are_hierarchical() {
        let refactor = CodeActionKind::from(CodeActionKind::REFACTOR);
        let extract = CodeActionKind::from(CodeActionKind::REFACTOR_EXTRACT);
        assert!(refactor.contains(&extract));
        assert!(!extract.contains(&refactor));
        assert!(extract.intersects(&refactor));
        assert!(!refactor.contains(&CodeActionKind::from("refactoring")));
        assert!(CodeActionKind::from("").contains(&refactor));
    }
}
