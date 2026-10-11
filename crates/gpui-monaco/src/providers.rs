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
    /// Models any of these filters match (Monaco's `LanguageFilter`s).
    Filters(Vec<LanguageFilter>),
}

/// Selects models by language, uri scheme and path (Monaco
/// `LanguageFilter`). Every field that's set must match, and at least one
/// must be set.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LanguageFilter {
    /// A language id, or `*` for any language.
    pub language: Option<SharedString>,
    /// A uri scheme such as `file` or `untitled`, or `*` for any.
    pub scheme: Option<SharedString>,
    /// A glob for the uri's path, such as `**/*.test.ts`.
    pub pattern: Option<SharedString>,
}

impl LanguageFilter {
    /// Monaco `score` for one filter: 0 when it doesn't match.
    fn score(&self, target: &ProviderTarget) -> u32 {
        let mut score = 0;
        if let Some(scheme) = &self.scheme {
            let uri_scheme = target
                .uri
                .as_deref()
                .and_then(|uri| uri.split_once(':'))
                .map(|(s, _)| s);
            if uri_scheme == Some(scheme.as_ref()) {
                score = 10;
            } else if scheme == "*" {
                score = 5;
            } else {
                return 0;
            }
        }
        if let Some(language) = &self.language {
            if target.language_id.as_deref() == Some(language.as_ref()) {
                score = 10;
            } else if language == "*" {
                score = score.max(5);
            } else {
                return 0;
            }
        }
        if let Some(pattern) = &self.pattern {
            let path = target.uri.as_deref().and_then(crate::languages::uri_path);
            if path.is_some_and(|path| {
                path == pattern.as_ref() || syntax::glob_matches(pattern, &path)
            }) {
                score = 10;
            } else {
                return 0;
            }
        }
        score
    }
}

/// The model a provider lookup is for: its language id and uri (what
/// Monaco's selectors score).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ProviderTarget {
    /// The model's language id.
    pub language_id: Option<SharedString>,
    /// The model's uri.
    pub uri: Option<SharedString>,
}

impl From<Option<&str>> for ProviderTarget {
    fn from(language_id: Option<&str>) -> Self {
        Self {
            language_id: language_id.map(|id| SharedString::from(id.to_string())),
            uri: None,
        }
    }
}

impl From<&crate::TextModel> for ProviderTarget {
    fn from(model: &crate::TextModel) -> Self {
        Self {
            language_id: model.language_id().cloned(),
            uri: model.uri().cloned(),
        }
    }
}

impl LanguageSelector {
    /// A selector for a single language id.
    pub fn language(id: impl Into<SharedString>) -> Self {
        Self::Languages(vec![id.into()])
    }

    /// Whether a model with `language_id` is selected. Models without a language only
    /// match [`LanguageSelector::All`].
    pub fn matches(&self, language_id: Option<&str>) -> bool {
        self.matches_target(&language_id.into())
    }

    /// Whether the model `target` describes is selected (Monaco `score`
    /// above 0).
    pub fn matches_target(&self, target: &ProviderTarget) -> bool {
        match self {
            Self::All => true,
            Self::Languages(ids) => target
                .language_id
                .as_ref()
                .is_some_and(|id| ids.iter().any(|l| l == id)),
            Self::Filters(filters) => filters.iter().any(|filter| filter.score(target) > 0),
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
    /// A custom color (Monaco `Customcolor`).
    Customcolor,
    /// A folder.
    Folder,
    /// A type parameter.
    TypeParameter,
    /// A user (an account).
    User,
    /// An issue.
    Issue,
    /// A tool.
    Tool,
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

    /// A name for the provider in the suggest widget's explain mode
    /// (Monaco `_debugDisplayName`); the type's name by default.
    fn debug_display_name(&self) -> SharedString {
        std::any::type_name::<Self>().into()
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
    /// Monaco `canIncreaseVerbosity`: the hover shows a `+` to ask for more.
    pub can_increase_verbosity: bool,
    /// Monaco `canDecreaseVerbosity`: the hover shows a `−` to ask for less.
    pub can_decrease_verbosity: bool,
}

/// Monaco `HoverContext`: why the hover is asked for.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct HoverContext {
    /// Set when the user asked for more or less detail.
    pub verbosity_request: Option<HoverVerbosityRequest>,
}

/// Monaco `HoverVerbosityRequest`.
#[derive(Clone, Debug, PartialEq)]
pub struct HoverVerbosityRequest {
    /// How much more (positive) or less (negative) verbose to be.
    pub verbosity_delta: i32,
    /// The provider's previous hover at this position.
    pub previous_hover: Hover,
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

    /// Like [`HoverProvider::provide_hover`], with Monaco's `HoverContext`:
    /// asked again with a `verbosity_request` when the user clicks the
    /// hover's `+` or `−` (see [`Hover::can_increase_verbosity`]). Defaults
    /// to `provide_hover`.
    fn provide_hover_with_context(
        &self,
        context: &ProviderContext,
        position: PointUtf16,
        hover_context: &HoverContext,
        cx: &mut App,
    ) -> Task<anyhow::Result<Option<Hover>>> {
        let _ = hover_context;
        self.provide_hover(context, position, cx)
    }
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

/// How rename suggestions were asked for (Monaco
/// `NewSymbolNameTriggerKind`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NewSymbolNameTriggerKind {
    /// The rename box's suggestions button was clicked.
    Invoke,
    /// The rename box opened; only providers that
    /// [support it](NewSymbolNamesProvider::supports_automatic_new_symbol_names_trigger_kind)
    /// are asked.
    Automatic,
}

/// A tag on a rename suggestion (Monaco `NewSymbolNameTag`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NewSymbolNameTag {
    /// Generated by AI; shown with a sparkle.
    AIGenerated,
}

/// A suggested new name for a symbol (Monaco `NewSymbolName`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NewSymbolName {
    /// The name (Monaco `newSymbolName`).
    pub new_symbol_name: SharedString,
    /// Its tags.
    pub tags: Vec<NewSymbolNameTag>,
}

impl NewSymbolName {
    /// A suggestion without tags.
    pub fn new(name: impl Into<SharedString>) -> Self {
        Self {
            new_symbol_name: name.into(),
            tags: Vec::new(),
        }
    }
}

/// Suggests new names in the rename box (Monaco `NewSymbolNamesProvider`).
///
/// Register with [`register_new_symbol_name_provider`]. With one
/// registered, the rename box gets a suggestions button and a list of
/// candidates (Up/Down to pick one).
pub trait NewSymbolNamesProvider: 'static {
    /// Whether to be asked as soon as the rename box opens (Monaco
    /// `supportsAutomaticNewSymbolNamesTriggerKind`), not only when its
    /// button is clicked. `false` by default.
    fn supports_automatic_new_symbol_names_trigger_kind(&self) -> bool {
        false
    }

    /// Names for the symbol at `range` (Monaco `provideNewSymbolNames`).
    /// Errors are ignored.
    fn provide_new_symbol_names(
        &self,
        context: &ProviderContext,
        range: Range<PointUtf16>,
        trigger_kind: NewSymbolNameTriggerKind,
        cx: &mut App,
    ) -> Task<anyhow::Result<Vec<NewSymbolName>>>;
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

    /// Documentation commands for kinds of actions (Monaco
    /// `CodeActionProviderMetadata.documentation`), shown in the code action
    /// menu's action bar: the entry whose kind best matches what was asked
    /// for (e.g. `refactor` for the refactor command), else the first whose
    /// kind contains a returned action's. Empty by default.
    fn documentation(&self) -> Vec<(CodeActionKind, Command)> {
        Vec::new()
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

/// A piece of an inlay hint's label (Monaco `InlayHintLabelPart`), with its
/// own hover, go-to target and command.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct InlayHintLabelPart {
    /// Its text.
    pub label: SharedString,
    /// Shown on hover, after the hint's tooltip.
    pub tooltip: Option<MarkdownString>,
    /// Where Cmd/Ctrl+click goes (opened like go to definition).
    pub location: Option<Location>,
    /// Run by Cmd/Ctrl+click when there's no location, and offered in the
    /// hover.
    pub command: Option<Command>,
}

/// An inlay hint's label (Monaco `string | InlayHintLabelPart[]`).
#[derive(Clone, Debug, PartialEq)]
pub enum InlayHintLabel {
    /// Plain text.
    Text(SharedString),
    /// Parts shown one after another.
    Parts(Vec<InlayHintLabelPart>),
}

impl Default for InlayHintLabel {
    fn default() -> Self {
        Self::Text(SharedString::default())
    }
}

impl InlayHintLabel {
    /// The label as parts (a text label is one part).
    pub fn parts(&self) -> Vec<InlayHintLabelPart> {
        match self {
            Self::Text(text) => vec![InlayHintLabelPart {
                label: text.clone(),
                ..Default::default()
            }],
            Self::Parts(parts) => parts.clone(),
        }
    }

    /// The whole label's text.
    pub fn text(&self) -> String {
        match self {
            Self::Text(text) => text.to_string(),
            Self::Parts(parts) => parts.iter().map(|part| part.label.as_ref()).collect(),
        }
    }
}

impl From<&str> for InlayHintLabel {
    fn from(text: &str) -> Self {
        Self::Text(text.to_string().into())
    }
}

impl From<String> for InlayHintLabel {
    fn from(text: String) -> Self {
        Self::Text(text.into())
    }
}

impl From<SharedString> for InlayHintLabel {
    fn from(text: SharedString) -> Self {
        Self::Text(text)
    }
}

impl From<Vec<InlayHintLabelPart>> for InlayHintLabel {
    fn from(parts: Vec<InlayHintLabelPart>) -> Self {
        Self::Parts(parts)
    }
}

/// Inline text the editor shows but that isn't part of the document
/// (Monaco `InlayHint`), such as inferred types or parameter names.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct InlayHint {
    /// Where it shows (0-based row, UTF-16 column).
    pub position: PointUtf16,
    /// Its text, or parts with their own tooltips, locations and commands.
    pub label: InlayHintLabel,
    /// Its kind, if any.
    pub kind: Option<InlayHintKind>,
    /// Shown on hover.
    pub tooltip: Option<MarkdownString>,
    /// Applied when the hint is double-clicked (e.g. to insert the type it
    /// shows).
    pub text_edits: Vec<TextEdit>,
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

    /// Fills in a hint before its hover shows or it's clicked (Monaco
    /// `resolveInlayHint`): the returned label replaces the hint's, and its
    /// tooltip and text edits do when it has them. `None` (the default)
    /// leaves the hint as it is.
    fn resolve_inlay_hint(
        &self,
        _hint: &InlayHint,
        _cx: &mut App,
    ) -> Option<Task<anyhow::Result<InlayHint>>> {
        None
    }
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
    /// Whether inline edits are wanted (Monaco `includeInlineEdits`, from
    /// `inline_suggest.edits.enabled`); the editor drops them otherwise.
    pub include_inline_edits: bool,
    /// Whether classic inline completions are wanted (Monaco
    /// `includeInlineCompletions`).
    pub include_inline_completions: bool,
}

/// One inline completion (Monaco `InlineCompletion`).
///
/// A classic completion shows as ghost text while what's typed matches it.
/// An inline edit (`is_inline_edit`, a "next edit suggestion") can change
/// text anywhere: the editor diffs `range` against `insert_text`, shows the
/// changes where they are, and offers to jump there with Tab.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct InlineCompletion {
    /// The text to insert.
    pub insert_text: String,
    /// `insert_text` is a snippet.
    pub is_snippet: bool,
    /// The range it replaces; by default from the start of the word at the
    /// cursor to the end of the line. A completion's range is on one line;
    /// an inline edit's can span lines.
    pub range: Option<Range<PointUtf16>>,
    /// Run after accepting.
    pub command: Option<Command>,
    /// Edits elsewhere applied on accept.
    pub additional_text_edits: Vec<(Range<PointUtf16>, String)>,
    /// Show it as an inline edit, wherever it is, rather than as ghost text
    /// at the cursor (Monaco `isInlineEdit`).
    pub is_inline_edit: bool,
    /// Only show it while the cursor is in this range (Monaco `showRange`).
    /// It follows edits.
    pub show_range: Option<Range<PointUtf16>>,
    /// A link at the bottom of the inline edit's gutter menu, shown as
    /// "title..." (Monaco `gutterMenuLinkAction`).
    pub gutter_menu_link_action: Option<Command>,
    /// The document `range` and `additional_text_edits` refer to, when it
    /// isn't the one asked about (Monaco `uri`): a cross-file inline edit.
    /// The editor diffs it against that model ([`crate::model_for_uri`]),
    /// shows where it goes in the long-distance hint, and accepting (or
    /// jumping) moves it to an editor showing that model, after emitting
    /// [`CodeEditorEvent::OpenLocation`](crate::CodeEditorEvent::OpenLocation)
    /// so the host can open or focus one. An item with a `uri` is an inline
    /// edit.
    pub uri: Option<SharedString>,
    /// Suggests moving the cursor here instead of editing (Monaco
    /// `jumpToPosition`): Tab jumps there, which accepts it, and
    /// `insert_text` is ignored. Such an item is an inline edit.
    pub jump_to_position: Option<PointUtf16>,
    /// Shows the item as `content` at `range` rather than as a diff, until
    /// it's jumped to (Monaco `hint`, the "custom" view). Such an item is an
    /// inline edit; accepting it leaves the cursor where it is.
    pub hint: Option<InlineCompletionHint>,
    /// Shows the inline edit gutter indicator and its menu for this ghost
    /// text too (Monaco `showInlineEditMenu`). Such items count as inline
    /// edits for `include_inline_edits`, and sort after the others.
    pub show_inline_edit_menu: bool,
    /// A second way to accept the inline edit, run with Shift+Tab or from
    /// the word replacement view's label (Monaco's
    /// `InlineSuggestAlternativeAction`, which VS Code fills in itself for
    /// renames; here the provider supplies it). An inline edit with one
    /// shows as a word replacement.
    pub alternative_action: Option<InlineCompletionAlternativeAction>,
}

/// How an inline completion's [`hint`](InlineCompletion::hint) is drawn
/// (Monaco `InlineCompletionHintStyle`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum InlineCompletionHintStyle {
    /// Colored like code in the model's language.
    #[default]
    Code,
    /// Plain text.
    Label,
}

/// Where and what an inline completion shows instead of its diff (Monaco
/// `IInlineCompletionHint`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct InlineCompletionHint {
    /// The range it's shown at, in the model asked about; it follows edits.
    pub range: Range<PointUtf16>,
    /// How `content` is drawn.
    pub style: InlineCompletionHintStyle,
    /// The text shown.
    pub content: String,
}

/// A second way to accept an inline edit (Monaco
/// `InlineSuggestAlternativeAction`).
#[derive(Clone, Debug, PartialEq)]
pub struct InlineCompletionAlternativeAction {
    /// Shown beside the new word, e.g. "Rename".
    pub label: SharedString,
    /// Run instead of applying the edit.
    pub command: Command,
    /// How many places it changes, shown as "label (n)" when more than one
    /// (Monaco's `count`).
    pub count: Option<usize>,
}

/// What an inline completions provider returns (Monaco
/// `InlineCompletions`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct InlineCompletionList {
    /// The completions, best first.
    pub items: Vec<InlineCompletion>,
    /// Keep the suggest widget from opening by itself while one shows.
    pub suppress_suggestions: Option<bool>,
    /// Extra commands for the inline suggestion toolbar's menu (Monaco
    /// `InlineCompletions.commands`).
    pub commands: Vec<Command>,
}

/// Why an inline completion stopped showing for good (Monaco
/// `InlineCompletionEndOfLifeReason`).
#[derive(Clone, Debug, PartialEq)]
#[allow(clippy::large_enum_variant)] // Matches Monaco's shape; rarely built.
pub enum InlineCompletionEndOfLifeReason {
    /// The user accepted it (Monaco also passes `alternativeAction`).
    Accepted {
        /// Its [`alternative_action`](InlineCompletion::alternative_action)
        /// ran instead of its edit (Shift+Tab).
        alternative_action: bool,
    },
    /// The user dismissed it while it showed (Escape).
    Rejected,
    /// Neither: it was hidden, replaced or never shown.
    Ignored {
        /// The completion from a newer request that replaced it.
        superseded_by: Option<InlineCompletion>,
        /// The user typed text it didn't match.
        user_typing_disagreed: bool,
    },
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

    /// `item` was shown and then dismissed (Monaco's deprecated
    /// `handleRejection`; also see `handle_end_of_lifetime`).
    fn handle_rejection(&self, _item: &InlineCompletion) {}

    /// `item` won't show again: it was accepted, rejected or ignored.
    /// Called once per item (Monaco `handleEndOfLifetime`).
    fn handle_end_of_lifetime(
        &self,
        _item: &InlineCompletion,
        _reason: &InlineCompletionEndOfLifeReason,
    ) {
    }

    /// The group this provider belongs to, for other providers'
    /// [`yields_to_group_ids`](Self::yields_to_group_ids) (Monaco
    /// `groupId`).
    fn group_id(&self) -> Option<SharedString> {
        None
    }

    /// Groups this provider yields to: it's asked only after them, and not
    /// at all if one of them has a visible completion (Monaco
    /// `yieldsToGroupIds`).
    fn yields_to_group_ids(&self) -> Vec<SharedString> {
        Vec::new()
    }

    /// The name heading the inline edit gutter menu; `None` shows "Inline
    /// Suggestion" (Monaco `displayName`).
    fn display_name(&self) -> Option<SharedString> {
        None
    }
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
        cx.observe_global::<FoldingRangesEpoch>(|_, cx| cx.notify()),
        cx.observe_global::<CodeLensEpoch>(|_, cx| cx.notify()),
    ]
}

/// Asks every editor for semantic tokens again (Monaco's provider
/// `onDidChange`), e.g. after `workspace/semanticTokens/refresh`.
pub fn refresh_semantic_tokens(cx: &mut App) {
    cx.default_global::<SemanticTokensEpoch>().0 += 1;
}

// ----- colors --------------------------------------------------------------------

/// A color in the text (Monaco `IColorInformation`).
#[derive(Clone, Debug, PartialEq)]
pub struct ColorInformation {
    /// Where it is.
    pub range: Range<PointUtf16>,
    /// The color, components in `0..=1`.
    pub color: crate::color::ColorF,
}

/// A way to write a color (Monaco `IColorPresentation`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ColorPresentation {
    /// Shown in the color picker's header, and inserted without a
    /// `text_edit`.
    pub label: SharedString,
    /// The edit that writes it.
    pub text_edit: Option<TextEdit>,
    /// Other edits made with it.
    pub additional_text_edits: Vec<TextEdit>,
}

/// Finds colors in a document and writes them (Monaco
/// `DocumentColorProvider`).
///
/// Register with [`register_color_provider`]. Without providers, the
/// editor finds hex, `rgb()`, `rgba()`, `hsl()` and `hsla()` colors itself
/// (see `default_color_decorators`).
pub trait DocumentColorProvider: 'static {
    /// The document's colors.
    fn provide_document_colors(
        &self,
        context: &ProviderContext,
        cx: &mut App,
    ) -> Task<anyhow::Result<Vec<ColorInformation>>>;

    /// The ways to write `color` at its range, the first being the default.
    fn provide_color_presentations(
        &self,
        context: &ProviderContext,
        color: &ColorInformation,
        cx: &mut App,
    ) -> Task<anyhow::Result<Vec<ColorPresentation>>>;
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

/// Bumped by [`refresh_folding_ranges`]; editors recompute when it changes.
#[derive(Default)]
pub(crate) struct FoldingRangesEpoch(pub u64);

impl Global for FoldingRangesEpoch {}

/// Asks every editor to request folding ranges again (Monaco's
/// `FoldingRangeProvider.onDidChange`), e.g. after a language server's
/// `workspace/foldingRange/refresh`.
pub fn refresh_folding_ranges(cx: &mut App) {
    cx.default_global::<FoldingRangesEpoch>().0 += 1;
}

/// Bumped by [`refresh_code_lenses`]; editors request again when it changes.
#[derive(Default)]
pub(crate) struct CodeLensEpoch(pub u64);

impl Global for CodeLensEpoch {}

/// Asks every editor to request code lenses again (Monaco's
/// `CodeLensProvider.onDidChange`), e.g. after a language server's
/// `workspace/codeLens/refresh`.
pub fn refresh_code_lenses(cx: &mut App) {
    cx.default_global::<CodeLensEpoch>().0 += 1;
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
    /// UTF-16 column). `None` leaves it to the next provider, and finally to
    /// the textual occurrences; an empty list highlights nothing. Called
    /// after the cursor rests on a word for
    /// [`EditorOptions::occurrences_highlight_delay`](crate::EditorOptions::occurrences_highlight_delay).
    /// With [`EditorOptions::occurrences_highlight_from_all_providers`](crate::EditorOptions::occurrences_highlight_from_all_providers)
    /// every matching provider is asked and the answers are merged.
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

    /// Like [`provide_signature_help`](Self::provide_signature_help), with
    /// Monaco's whole `SignatureHelpContext`: whether hints are already
    /// showing or requested, and what they show. Defaults to
    /// `provide_signature_help`.
    fn provide_signature_help_with_context(
        &self,
        context: &ProviderContext,
        position: PointUtf16,
        help_context: &SignatureHelpContext,
        cx: &mut App,
    ) -> Task<anyhow::Result<Option<SignatureHelp>>> {
        self.provide_signature_help(context, position, help_context.trigger.clone(), cx)
    }
}

/// Why and in what state signature help is asked for (Monaco
/// `SignatureHelpContext`).
#[derive(Clone, Debug, PartialEq)]
pub struct SignatureHelpContext {
    /// What asked (Monaco `triggerKind` and `triggerCharacter`).
    pub trigger: SignatureHelpTrigger,
    /// Hints are already showing or being requested (Monaco `isRetrigger`).
    pub is_retrigger: bool,
    /// The hints showing, with the signature the user chose (Monaco
    /// `activeSignatureHelp`).
    pub active_signature_help: Option<SignatureHelp>,
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

    /// The edits that format all of `ranges` at once (Monaco
    /// `provideDocumentRangesFormattingEdits`), for several selections.
    /// `None`, the default, formats them one at a time.
    fn provide_document_ranges_formatting_edits(
        &self,
        _context: &ProviderContext,
        _ranges: &[Range<PointUtf16>],
        _options: FormattingOptions,
        _cx: &mut App,
    ) -> Option<Task<anyhow::Result<Vec<TextEdit>>>> {
        None
    }
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

// ----- drop and paste ------------------------------------------------------------

/// Data being dropped or pasted, of one MIME type (Monaco
/// `IDataTransferItem`).
#[derive(Clone, Debug, PartialEq)]
pub enum DataTransferItem {
    /// Text, such as `text/plain` or a `text/uri-list`.
    Text(String),
    /// Files (Monaco's `files` entries).
    Files(Vec<std::path::PathBuf>),
}

/// The data of a drop or paste by MIME type (Monaco `VSDataTransfer`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct DataTransfer {
    items: Vec<(SharedString, DataTransferItem)>,
}

impl DataTransfer {
    /// Sets the data of `mime_type` (lowercased, as Monaco normalizes it).
    pub fn insert(&mut self, mime_type: impl Into<SharedString>, item: DataTransferItem) {
        let mime_type: SharedString = mime_type.into().to_lowercase().into();
        self.items.retain(|(mime, _)| *mime != mime_type);
        self.items.push((mime_type, item));
    }

    /// The data of `mime_type`.
    pub fn get(&self, mime_type: &str) -> Option<&DataTransferItem> {
        let mime_type = mime_type.to_lowercase();
        self.items
            .iter()
            .find(|(mime, _)| **mime == mime_type)
            .map(|(_, item)| item)
    }

    /// The text of `mime_type`.
    pub fn text(&self, mime_type: &str) -> Option<&str> {
        match self.get(mime_type)? {
            DataTransferItem::Text(text) => Some(text),
            DataTransferItem::Files(_) => None,
        }
    }

    /// Whether there's data of `mime_type`.
    pub fn has(&self, mime_type: &str) -> bool {
        self.get(mime_type).is_some()
    }

    /// Removes the data of `mime_type`.
    pub fn remove(&mut self, mime_type: &str) {
        let mime_type = mime_type.to_lowercase();
        self.items.retain(|(mime, _)| *mime != mime_type);
    }

    /// The MIME types with data.
    pub fn mime_types(&self) -> impl Iterator<Item = &SharedString> {
        self.items.iter().map(|(mime, _)| mime)
    }

    /// Whether any data matches `pattern` (Monaco `matchesMimeType`):
    /// `*/*`, `type/*` or an exact type. Files match `files`.
    pub fn matches(&self, pattern: &str) -> bool {
        matches_mime_type(pattern, self.mime_types().map(|m| m.as_ref()))
    }
}

/// Monaco `matchesMimeType`: whether `pattern` (`*/*`, `type/*` or a type)
/// matches any of `mime_types`, ignoring case.
pub fn matches_mime_type<'a>(pattern: &str, mime_types: impl IntoIterator<Item = &'a str>) -> bool {
    let pattern = pattern.to_lowercase();
    let Some((pattern_type, pattern_subtype)) = pattern.split_once('/') else {
        return mime_types
            .into_iter()
            .any(|m| m.eq_ignore_ascii_case(&pattern));
    };
    mime_types.into_iter().any(|mime| {
        let mime = mime.to_lowercase();
        if pattern == "*/*" {
            return true;
        }
        match mime.split_once('/') {
            Some((ty, subtype)) => {
                ty == pattern_type && (pattern_subtype == "*" || subtype == pattern_subtype)
            }
            None => false,
        }
    })
}

/// Monaco `HierarchicalKind.contains`: `parent` is `kind` or a dotted
/// prefix of it (`text` contains `text.plain`; empty contains all).
pub fn edit_kind_contains(parent: &str, kind: &str) -> bool {
    parent.is_empty()
        || kind == parent
        || kind
            .strip_prefix(parent)
            .is_some_and(|rest| rest.starts_with('.'))
}

/// What a drop or paste edit should rank below (Monaco `DropYieldTo`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EditYieldTo {
    /// Edits whose kind this contains.
    Kind(SharedString),
    /// Edits handling this MIME type.
    MimeType(SharedString),
}

/// The text a drop or paste edit inserts.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EditInsertText {
    /// Plain text.
    Text(String),
    /// A snippet (TextMate syntax).
    Snippet(String),
}

impl Default for EditInsertText {
    fn default() -> Self {
        Self::Text(String::new())
    }
}

/// A way to drop or paste data (Monaco `DocumentDropEdit` and
/// `DocumentPasteEdit`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct DocumentDropOrPasteEdit {
    /// Its label in the drop and paste pickers.
    pub title: SharedString,
    /// Its kind, such as `text.plain` or `uri.path.absolute` (Monaco
    /// `HierarchicalKind`).
    pub kind: Option<SharedString>,
    /// The MIME type it handles, for [`EditYieldTo::MimeType`].
    pub handled_mime_type: Option<SharedString>,
    /// What it inserts at each selection (or the drop position).
    pub insert_text: EditInsertText,
    /// Other edits, applied with it.
    pub additional_edits: Vec<TextEdit>,
    /// Edits it should rank below.
    pub yield_to: Vec<EditYieldTo>,
}

/// Offers ways to insert dropped data (Monaco `DocumentDropEditProvider`).
///
/// Register with [`register_document_drop_edit_provider`].
pub trait DocumentDropEditProvider: 'static {
    /// The MIME types it handles (`None`: any).
    fn drop_mime_types(&self) -> Option<Vec<SharedString>> {
        None
    }

    /// The edits for dropping `data` at `position`.
    fn provide_document_drop_edits(
        &self,
        context: &ProviderContext,
        position: PointUtf16,
        data: &DataTransfer,
        cx: &mut App,
    ) -> Task<anyhow::Result<Vec<DocumentDropOrPasteEdit>>>;
}

/// Why paste edits are asked for (Monaco `DocumentPasteTriggerKind`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DocumentPasteTriggerKind {
    /// A normal paste.
    Automatic,
    /// "Paste As...".
    PasteAs,
}

/// Offers ways to insert pasted data (Monaco `DocumentPasteEditProvider`).
///
/// Register with [`register_document_paste_edit_provider`].
pub trait DocumentPasteEditProvider: 'static {
    /// The MIME types it handles.
    fn paste_mime_types(&self) -> Vec<SharedString>;

    /// The kinds of edits it returns, for "Paste As..." preferences.
    fn provided_paste_edit_kinds(&self) -> Vec<SharedString> {
        Vec::new()
    }

    /// The edits for pasting `data` over `ranges`.
    fn provide_document_paste_edits(
        &self,
        context: &ProviderContext,
        ranges: &[Range<PointUtf16>],
        data: &DataTransfer,
        trigger: DocumentPasteTriggerKind,
        cx: &mut App,
    ) -> Task<anyhow::Result<Vec<DocumentDropOrPasteEdit>>>;

    /// Whether it's the built-in plain text provider, which leaves the
    /// paste to the editor when it's the only one.
    #[doc(hidden)]
    fn is_default_text_provider(&self) -> bool {
        false
    }
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
    new_symbol_names: Vec<Entry<dyn NewSymbolNamesProvider>>,
    code_action: Vec<Entry<dyn CodeActionProvider>>,
    inlay_hints: Vec<Entry<dyn InlayHintsProvider>>,
    links: Vec<Entry<dyn LinkProvider>>,
    colors: Vec<Entry<dyn DocumentColorProvider>>,
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
    drop_edits: Vec<Entry<dyn DocumentDropEditProvider>>,
    paste_edits: Vec<Entry<dyn DocumentPasteEditProvider>>,
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
    /// Registers a color provider for the models `selector` matches (Monaco
    /// `registerColorProvider`). Results from all matching providers are
    /// merged.
    register_color_provider,
    colors,
    DocumentColorProvider
);
register_fn!(
    /// Registers a drop edit provider for the models `selector` matches
    /// (Monaco `registerDocumentDropEditProvider`). The editor has built-in
    /// ones for plain text and paths.
    register_document_drop_edit_provider,
    drop_edits,
    DocumentDropEditProvider
);
register_fn!(
    /// Registers a paste edit provider for the models `selector` matches
    /// (Monaco `registerDocumentPasteEditProvider`). The editor has built-in
    /// ones for plain text, paths and HTML.
    register_document_paste_edit_provider,
    paste_edits,
    DocumentPasteEditProvider
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
    /// Registers a rename suggestions provider for the models `selector`
    /// matches (Monaco `registerNewSymbolNameProvider`). Suggestions from all
    /// matching providers are merged.
    register_new_symbol_name_provider,
    new_symbol_names,
    NewSymbolNamesProvider
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
    features.colors.retain(|e| e.id != registration);
    features.definition.retain(|e| e.id != registration);
    features.signature_help.retain(|e| e.id != registration);
    features
        .document_formatting
        .retain(|e| e.id != registration);
    features.range_formatting.retain(|e| e.id != registration);
    features.declaration.retain(|e| e.id != registration);
    features.rename.retain(|e| e.id != registration);
    features.new_symbol_names.retain(|e| e.id != registration);
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
    features.drop_edits.retain(|e| e.id != registration);
    features.paste_edits.retain(|e| e.id != registration);
}

/// The registered drop edit providers whose selector matches
/// `language_id`, newest first, then the built-in ones (Monaco's order).
pub fn drop_edit_providers(
    cx: &App,
    target: impl Into<ProviderTarget>,
) -> Vec<Arc<dyn DocumentDropEditProvider>> {
    let target = target.into();
    let mut providers = cx
        .try_global::<LanguageFeatures>()
        .map(|f| matching(&f.drop_edits, &target))
        .unwrap_or_default();
    providers.reverse();
    providers.extend(crate::drop_paste::default_drop_providers());
    providers
}

/// The registered paste edit providers whose selector matches
/// `language_id`, newest first, then the built-in ones (Monaco's order).
pub fn paste_edit_providers(
    cx: &App,
    target: impl Into<ProviderTarget>,
) -> Vec<Arc<dyn DocumentPasteEditProvider>> {
    let target = target.into();
    let mut providers = cx
        .try_global::<LanguageFeatures>()
        .map(|f| matching(&f.paste_edits, &target))
        .unwrap_or_default();
    providers.reverse();
    providers.extend(crate::drop_paste::default_paste_providers());
    providers
}

fn matching<T: ?Sized>(entries: &[Entry<T>], target: &ProviderTarget) -> Vec<Arc<T>> {
    entries
        .iter()
        .filter(|e| e.selector.matches_target(target))
        .map(|e| e.provider.clone())
        .collect()
}

/// The registered completion providers whose selector matches `language_id`, in
/// registration order.
pub fn completion_providers(
    cx: &App,
    target: impl Into<ProviderTarget>,
) -> Vec<Arc<dyn CompletionProvider>> {
    cx.try_global::<LanguageFeatures>()
        .map(|f| matching(&f.completion, &target.into()))
        .unwrap_or_default()
}

/// The registered hover providers whose selector matches `language_id`, in
/// registration order.
pub fn hover_providers(cx: &App, target: impl Into<ProviderTarget>) -> Vec<Arc<dyn HoverProvider>> {
    cx.try_global::<LanguageFeatures>()
        .map(|f| matching(&f.hover, &target.into()))
        .unwrap_or_default()
}

/// The registered definition providers whose selector matches `language_id`, in
/// registration order.
pub fn definition_providers(
    cx: &App,
    target: impl Into<ProviderTarget>,
) -> Vec<Arc<dyn DefinitionProvider>> {
    cx.try_global::<LanguageFeatures>()
        .map(|f| matching(&f.definition, &target.into()))
        .unwrap_or_default()
}

/// The registered signature help providers whose selector matches `language_id`,
/// in registration order.
pub fn signature_help_providers(
    cx: &App,
    target: impl Into<ProviderTarget>,
) -> Vec<Arc<dyn SignatureHelpProvider>> {
    cx.try_global::<LanguageFeatures>()
        .map(|f| matching(&f.signature_help, &target.into()))
        .unwrap_or_default()
}

/// The registered document highlight providers whose selector matches
/// `language_id`, in registration order.
pub fn document_highlight_providers(
    cx: &App,
    target: impl Into<ProviderTarget>,
) -> Vec<Arc<dyn DocumentHighlightProvider>> {
    cx.try_global::<LanguageFeatures>()
        .map(|f| matching(&f.document_highlight, &target.into()))
        .unwrap_or_default()
}

/// The registered document formatters whose selector matches `language_id`,
/// in registration order.
pub fn document_formatting_providers(
    cx: &App,
    target: impl Into<ProviderTarget>,
) -> Vec<Arc<dyn DocumentFormattingProvider>> {
    cx.try_global::<LanguageFeatures>()
        .map(|f| matching(&f.document_formatting, &target.into()))
        .unwrap_or_default()
}

/// The registered range formatters whose selector matches `language_id`, in
/// registration order.
pub fn document_range_formatting_providers(
    cx: &App,
    target: impl Into<ProviderTarget>,
) -> Vec<Arc<dyn DocumentRangeFormattingProvider>> {
    cx.try_global::<LanguageFeatures>()
        .map(|f| matching(&f.range_formatting, &target.into()))
        .unwrap_or_default()
}

/// The registered on-type formatters whose selector matches `language_id`,
/// in registration order.
pub fn on_type_formatting_providers(
    cx: &App,
    target: impl Into<ProviderTarget>,
) -> Vec<Arc<dyn OnTypeFormattingProvider>> {
    cx.try_global::<LanguageFeatures>()
        .map(|f| matching(&f.on_type_formatting, &target.into()))
        .unwrap_or_default()
}

/// The registered declaration providers whose selector matches `language_id`, in
/// registration order.
pub fn declaration_providers(
    cx: &App,
    target: impl Into<ProviderTarget>,
) -> Vec<Arc<dyn DeclarationProvider>> {
    cx.try_global::<LanguageFeatures>()
        .map(|f| matching(&f.declaration, &target.into()))
        .unwrap_or_default()
}

/// The registered type definition providers whose selector matches `language_id`, in
/// registration order.
pub fn type_definition_providers(
    cx: &App,
    target: impl Into<ProviderTarget>,
) -> Vec<Arc<dyn TypeDefinitionProvider>> {
    cx.try_global::<LanguageFeatures>()
        .map(|f| matching(&f.type_definition, &target.into()))
        .unwrap_or_default()
}

/// The registered implementation providers whose selector matches `language_id`, in
/// registration order.
pub fn implementation_providers(
    cx: &App,
    target: impl Into<ProviderTarget>,
) -> Vec<Arc<dyn ImplementationProvider>> {
    cx.try_global::<LanguageFeatures>()
        .map(|f| matching(&f.implementation, &target.into()))
        .unwrap_or_default()
}

/// The registered reference providers whose selector matches `language_id`, in
/// registration order.
pub fn reference_providers(
    cx: &App,
    target: impl Into<ProviderTarget>,
) -> Vec<Arc<dyn ReferenceProvider>> {
    cx.try_global::<LanguageFeatures>()
        .map(|f| matching(&f.references, &target.into()))
        .unwrap_or_default()
}

/// The registered rename providers whose selector matches `language_id`, in
/// registration order.
pub fn rename_providers(
    cx: &App,
    target: impl Into<ProviderTarget>,
) -> Vec<Arc<dyn RenameProvider>> {
    cx.try_global::<LanguageFeatures>()
        .map(|f| matching(&f.rename, &target.into()))
        .unwrap_or_default()
}

/// The registered rename suggestions providers whose selector matches
/// `language_id`, in registration order.
pub fn new_symbol_names_providers(
    cx: &App,
    target: impl Into<ProviderTarget>,
) -> Vec<Arc<dyn NewSymbolNamesProvider>> {
    cx.try_global::<LanguageFeatures>()
        .map(|f| matching(&f.new_symbol_names, &target.into()))
        .unwrap_or_default()
}

/// The registered code action providers whose selector matches
/// `language_id`, in registration order.
pub fn code_action_providers(
    cx: &App,
    target: impl Into<ProviderTarget>,
) -> Vec<Arc<dyn CodeActionProvider>> {
    cx.try_global::<LanguageFeatures>()
        .map(|f| matching(&f.code_action, &target.into()))
        .unwrap_or_default()
}

/// The registered document symbol providers whose selector matches
/// `language_id`, in registration order.
pub fn document_symbol_providers(
    cx: &App,
    target: impl Into<ProviderTarget>,
) -> Vec<Arc<dyn DocumentSymbolProvider>> {
    cx.try_global::<LanguageFeatures>()
        .map(|f| matching(&f.document_symbols, &target.into()))
        .unwrap_or_default()
}

/// The registered code lens providers whose selector matches
/// `language_id`, in registration order.
pub fn code_lens_providers(
    cx: &App,
    target: impl Into<ProviderTarget>,
) -> Vec<Arc<dyn CodeLensProvider>> {
    cx.try_global::<LanguageFeatures>()
        .map(|f| matching(&f.code_lens, &target.into()))
        .unwrap_or_default()
}

/// The registered linked editing range providers whose selector matches
/// `language_id`, in registration order.
pub fn linked_editing_range_providers(
    cx: &App,
    target: impl Into<ProviderTarget>,
) -> Vec<Arc<dyn LinkedEditingRangeProvider>> {
    cx.try_global::<LanguageFeatures>()
        .map(|f| matching(&f.linked_editing, &target.into()))
        .unwrap_or_default()
}

/// The registered inline completions providers whose selector matches
/// `language_id`, in registration order.
pub fn inline_completions_providers(
    cx: &App,
    target: impl Into<ProviderTarget>,
) -> Vec<Arc<dyn InlineCompletionsProvider>> {
    cx.try_global::<LanguageFeatures>()
        .map(|f| matching(&f.inline_completions, &target.into()))
        .unwrap_or_default()
}

/// The registered document semantic tokens providers whose selector
/// matches `language_id`, in registration order.
pub fn document_semantic_tokens_providers(
    cx: &App,
    target: impl Into<ProviderTarget>,
) -> Vec<Arc<dyn DocumentSemanticTokensProvider>> {
    cx.try_global::<LanguageFeatures>()
        .map(|f| matching(&f.semantic_tokens, &target.into()))
        .unwrap_or_default()
}

/// The registered range semantic tokens providers whose selector matches
/// `language_id`, in registration order.
pub fn document_range_semantic_tokens_providers(
    cx: &App,
    target: impl Into<ProviderTarget>,
) -> Vec<Arc<dyn DocumentRangeSemanticTokensProvider>> {
    cx.try_global::<LanguageFeatures>()
        .map(|f| matching(&f.range_semantic_tokens, &target.into()))
        .unwrap_or_default()
}

/// The registered color providers whose selector matches `language_id`, in
/// registration order.
pub fn color_providers(
    cx: &App,
    target: impl Into<ProviderTarget>,
) -> Vec<Arc<dyn DocumentColorProvider>> {
    cx.try_global::<LanguageFeatures>()
        .map(|f| matching(&f.colors, &target.into()))
        .unwrap_or_default()
}

/// The registered link providers whose selector matches `language_id`, in
/// registration order.
pub fn link_providers(cx: &App, target: impl Into<ProviderTarget>) -> Vec<Arc<dyn LinkProvider>> {
    cx.try_global::<LanguageFeatures>()
        .map(|f| matching(&f.links, &target.into()))
        .unwrap_or_default()
}

/// The registered inlay hints providers whose selector matches
/// `language_id`, in registration order.
pub fn inlay_hints_providers(
    cx: &App,
    target: impl Into<ProviderTarget>,
) -> Vec<Arc<dyn InlayHintsProvider>> {
    cx.try_global::<LanguageFeatures>()
        .map(|f| matching(&f.inlay_hints, &target.into()))
        .unwrap_or_default()
}

/// The registered folding range providers whose selector matches
/// `language_id`, in registration order.
pub fn folding_range_providers(
    cx: &App,
    target: impl Into<ProviderTarget>,
) -> Vec<Arc<dyn FoldingRangeProvider>> {
    cx.try_global::<LanguageFeatures>()
        .map(|f| matching(&f.folding, &target.into()))
        .unwrap_or_default()
}

/// The registered selection range providers whose selector matches
/// `language_id`, in registration order.
pub fn selection_range_providers(
    cx: &App,
    target: impl Into<ProviderTarget>,
) -> Vec<Arc<dyn SelectionRangeProvider>> {
    cx.try_global::<LanguageFeatures>()
        .map(|f| matching(&f.selection_range, &target.into()))
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
