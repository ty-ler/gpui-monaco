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

use crate::codicons::Codicon;

/// Which models a provider applies to (Monaco `LanguageSelector`).
///
/// Lookups rank the providers whose selector matches a model by
/// [`score`](LanguageSelector::score), best first, and among equal scores
/// the most recently registered first, as Monaco's
/// `LanguageFeatureRegistry.ordered` does.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LanguageSelector {
    /// Every model, including ones without a language; scores like `*`.
    All,
    /// Models whose language id is one of these; `*` matches any language
    /// (Monaco's string selectors).
    Languages(Vec<SharedString>),
    /// Models any of these filters match (Monaco's `LanguageFilter`s).
    Filters(Vec<LanguageFilter>),
}

/// Selects models by language, uri scheme and path (Monaco
/// `LanguageFilter`). Every field that's set must match, and at least one
/// must be set.
///
/// Monaco's `hasAccessToAllModels` and `notebookType` don't apply: every
/// model is available to providers, and there are no notebooks.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LanguageFilter {
    /// A language id, or `*` for any language.
    pub language: Option<SharedString>,
    /// A uri scheme such as `file` or `untitled`, or `*` for any.
    pub scheme: Option<SharedString>,
    /// A glob for the uri's path, such as `**/*.test.ts`.
    pub pattern: Option<SharedString>,
    /// When a provider whose filters are all exclusive matches a model,
    /// it's the only one of its kind used for it (Monaco `exclusive`).
    pub exclusive: bool,
}

impl LanguageFilter {
    /// Monaco `score` for one filter: 10 for an exact match, 5 for a `*`
    /// match, 0 when it doesn't match. Empty fields count as unset.
    pub fn score(&self, target: &ProviderTarget) -> u32 {
        let mut score = 0;
        if let Some(scheme) = self.scheme.as_ref().filter(|s| !s.is_empty()) {
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
        if let Some(language) = self.language.as_ref().filter(|l| !l.is_empty()) {
            if target.language_id.as_deref() == Some(language.as_ref()) {
                score = 10;
            } else if language == "*" {
                score = score.max(5);
            } else {
                return 0;
            }
        }
        if let Some(pattern) = self.pattern.as_ref().filter(|p| !p.is_empty()) {
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
/// Monaco's selectors score). From a [`TextModel`](crate::TextModel), it's
/// the model's [`language_id`](crate::TextModel::language_id) (`plaintext`
/// for a model without a language) and its
/// [`resource`](crate::TextModel::resource) (`inmemory://model/N` without a
/// uri), so `plaintext` selectors and `{ scheme: "inmemory" }` filters
/// apply as in Monaco.
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
            uri: Some(model.resource().clone()),
        }
    }
}

impl LanguageSelector {
    /// A selector for a single language id.
    pub fn language(id: impl Into<SharedString>) -> Self {
        Self::Languages(vec![id.into()])
    }

    /// Whether a model with `language_id` and no uri is selected. `None`
    /// only matches [`LanguageSelector::All`], `*`, and filters on the
    /// scheme or path; a model without a language is `plaintext` (see
    /// [`ProviderTarget`]).
    pub fn matches(&self, language_id: Option<&str>) -> bool {
        self.matches_target(&language_id.into())
    }

    /// Whether the model `target` describes is selected (Monaco `score`
    /// above 0).
    pub fn matches_target(&self, target: &ProviderTarget) -> bool {
        self.score(target) > 0
    }

    /// How well the selector matches the model `target` describes (Monaco
    /// `languages.score`): 10 for an exact language, scheme or path match,
    /// 5 for a `*` match, 0 for none. A list scores its best entry.
    pub fn score(&self, target: &ProviderTarget) -> u32 {
        match self {
            Self::All => 5,
            Self::Languages(ids) => ids
                .iter()
                .map(|id| {
                    if id == "*" {
                        5
                    } else if target.language_id.as_ref() == Some(id) {
                        10
                    } else {
                        0
                    }
                })
                .max()
                .unwrap_or(0),
            Self::Filters(filters) => filters
                .iter()
                .map(|filter| filter.score(target))
                .max()
                .unwrap_or(0),
        }
    }

    /// Monaco `isExclusive`: every filter is exclusive.
    fn is_exclusive(&self) -> bool {
        match self {
            Self::Filters(filters) => {
                !filters.is_empty() && filters.iter().all(|filter| filter.exclusive)
            }
            _ => false,
        }
    }
}

/// Monaco `languages.score`: how well `selector` matches the model
/// `target` describes; see [`LanguageSelector::score`].
pub fn score(selector: &LanguageSelector, target: &ProviderTarget) -> u32 {
    selector.score(target)
}

/// What a provider sees about the model it's asked about.
#[derive(Clone)]
pub struct ProviderContext {
    /// The model's uri, if the host gave it one, as given. Monaco's
    /// `model.uri` is [`ProviderContext::resource`], which a provider
    /// naming the model (in a `Location`, say) should use.
    pub uri: Option<SharedString>,
    /// The model's uri as Monaco has it (`model.uri.toString()`): the
    /// [`TextModel::resource`](crate::TextModel::resource), normalized, or
    /// `inmemory://model/N` for a model without a uri. LSP bridges identify
    /// the document by it.
    pub resource: SharedString,
    /// The model's [language id](crate::TextModel::language_id):
    /// `plaintext` for a model without one.
    pub language_id: Option<SharedString>,
    /// The model's text at the time of the request.
    pub snapshot: BufferSnapshot,
}

/// Markdown text and how to render it (Monaco `IMarkdownString`).
///
/// `"text".into()` makes an untrusted string without theme icons or HTML,
/// like Monaco's `new MarkdownString(text)`.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct MarkdownString {
    /// The markdown source (Monaco `value`).
    pub value: SharedString,
    /// Whether `command:` links run commands ([`crate::execute_command`])
    /// when clicked (Monaco `isTrusted`). Untrusted strings open only web
    /// and mail links.
    pub is_trusted: bool,
    /// Whether `$(name)` renders the codicon `name` (Monaco
    /// `supportThemeIcons`); see [`crate::codicons::parse_label_with_icons`].
    pub support_theme_icons: bool,
    /// Whether raw HTML renders, limited to a sanitized subset like
    /// Monaco's (Monaco `supportHtml`); otherwise HTML tags are dropped.
    pub support_html: bool,
}

impl MarkdownString {
    /// An untrusted markdown string (Monaco `new MarkdownString(value)`).
    pub fn new(value: impl Into<SharedString>) -> Self {
        Self {
            value: value.into(),
            ..Self::default()
        }
    }

    /// Plain text escaped so it renders as written (Monaco
    /// `new MarkdownString().appendText(text)`, newlines as paragraph
    /// breaks).
    pub fn from_plain_text(text: &str) -> Self {
        Self::new(escape_markdown_syntax_tokens(text))
    }

    /// Sets [`Self::is_trusted`].
    pub fn trusted(mut self, is_trusted: bool) -> Self {
        self.is_trusted = is_trusted;
        self
    }

    /// Sets [`Self::support_theme_icons`].
    pub fn with_theme_icons(mut self, support_theme_icons: bool) -> Self {
        self.support_theme_icons = support_theme_icons;
        self
    }

    /// Sets [`Self::support_html`].
    pub fn with_html(mut self, support_html: bool) -> Self {
        self.support_html = support_html;
        self
    }

    /// The markdown source.
    pub fn as_str(&self) -> &str {
        &self.value
    }
}

impl<T: Into<SharedString>> From<T> for MarkdownString {
    fn from(value: T) -> Self {
        Self::new(value)
    }
}

/// Monaco `MarkdownString.appendText`'s escaping
/// (`escapeMarkdownSyntaxTokens`): markdown punctuation is backslash
/// escaped, leading spaces and tabs become non-breaking spaces and each
/// newline starts a new paragraph. (Monaco makes every space
/// non-breaking, which keeps the text from wrapping; only indentation
/// needs it.)
pub fn escape_markdown_syntax_tokens(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + text.len() / 8);
    let mut line_start = true;
    for ch in text.chars() {
        match ch {
            '\\' | '`' | '*' | '_' | '{' | '}' | '[' | ']' | '(' | ')' | '#' | '+' | '-' | '!'
            | '~' | '>' | '|' | '<' | '.' | '$' => {
                out.push('\\');
                out.push(ch);
            }
            ' ' | '\t' if line_start => out.push('\u{a0}'),
            '\n' => out.push_str("\n\n"),
            _ => out.push(ch),
        }
        line_start = ch == '\n' || (line_start && (ch == ' ' || ch == '\t'));
    }
    out
}

/// Documentation that is either plain text or markdown (Monaco's
/// `string | IMarkdownString` for completion, signature and parameter
/// documentation and inlay hint tooltips). Plain text is shown as written,
/// keeping its line breaks and indentation (`white-space: pre-wrap`).
///
/// Strings convert to [`Documentation::PlainText`], as in Monaco; convert a
/// [`MarkdownString`] for markdown.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Documentation {
    /// Plain text (a Monaco `string`).
    PlainText(SharedString),
    /// Markdown (a Monaco `IMarkdownString`).
    Markdown(MarkdownString),
}

impl Documentation {
    /// The text: plain text, or the markdown source.
    pub fn as_str(&self) -> &str {
        match self {
            Documentation::PlainText(text) => text,
            Documentation::Markdown(markdown) => &markdown.value,
        }
    }

    /// As markdown: plain text is escaped (Monaco `appendText`).
    pub fn to_markdown(&self) -> MarkdownString {
        match self {
            Documentation::PlainText(text) => MarkdownString::from_plain_text(text),
            Documentation::Markdown(markdown) => markdown.clone(),
        }
    }
}

impl From<MarkdownString> for Documentation {
    fn from(markdown: MarkdownString) -> Self {
        Documentation::Markdown(markdown)
    }
}

impl From<&'static str> for Documentation {
    fn from(text: &'static str) -> Self {
        Documentation::PlainText(text.into())
    }
}

impl From<String> for Documentation {
    fn from(text: String) -> Self {
        Documentation::PlainText(text.into())
    }
}

impl From<SharedString> for Documentation {
    fn from(text: SharedString) -> Self {
        Documentation::PlainText(text)
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

impl CompletionItemKind {
    /// The kind's codicon in the suggest widget (Monaco
    /// `CompletionItemKinds.toIcon`).
    pub fn icon(self) -> Codicon {
        use CompletionItemKind::*;
        match self {
            Method => Codicon::SymbolMethod,
            Function => Codicon::SymbolFunction,
            Constructor => Codicon::SymbolConstructor,
            Field => Codicon::SymbolField,
            Variable => Codicon::SymbolVariable,
            Class => Codicon::SymbolClass,
            Struct => Codicon::SymbolStruct,
            Interface => Codicon::SymbolInterface,
            Module => Codicon::SymbolModule,
            Property => Codicon::SymbolProperty,
            Event => Codicon::SymbolEvent,
            Operator => Codicon::SymbolOperator,
            Unit => Codicon::SymbolUnit,
            Value => Codicon::SymbolValue,
            Enum => Codicon::SymbolEnum,
            Constant => Codicon::SymbolConstant,
            EnumMember => Codicon::SymbolEnumMember,
            Keyword => Codicon::SymbolKeyword,
            Snippet => Codicon::SymbolSnippet,
            Text => Codicon::SymbolText,
            Color => Codicon::SymbolColor,
            File => Codicon::SymbolFile,
            Reference => Codicon::SymbolReference,
            Customcolor => Codicon::SymbolCustomColor,
            Folder => Codicon::SymbolFolder,
            TypeParameter => Codicon::SymbolTypeParameter,
            User => Codicon::Account,
            Issue => Codicon::Issues,
            Tool => Codicon::Tools,
        }
    }
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
    pub documentation: Option<Documentation>,
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
    /// The single-line range to replace, on the cursor's line; defaults to
    /// the word before the cursor. Items with other ranges are skipped
    /// (Monaco `CompletionItem.isInvalid`).
    pub range: Option<Range<PointUtf16>>,
    /// Prefer selecting this item when it ties for the best filter score.
    pub preselect: bool,
    /// Typing one of these accepts the item, then inserts the character.
    pub commit_characters: Vec<char>,
    /// Edits applied elsewhere (e.g. imports) when accepted.
    pub additional_text_edits: Vec<(Range<PointUtf16>, String)>,
    /// The range replaced in replace mode (Monaco `range.replace`, e.g.
    /// the whole word); `range` is then the insert-mode range. It must be
    /// single-line, on the cursor's line, and start where the insert range
    /// does (the word's start without a `range`), or the item is skipped
    /// (Monaco `CompletionItem.isInvalid`). See `suggest.insert_mode`.
    pub replace_range: Option<Range<PointUtf16>>,
    /// Tags, such as [`CompletionItemTag::Deprecated`] (shown struck out).
    pub tags: Vec<CompletionItemTag>,
    /// A command run after the item is inserted.
    pub command: Option<Command>,
    /// Another command to run on acceptance (Monaco `action`): the ghost
    /// text made from suggestions (`quick_suggestions` `Inline`) runs it
    /// when there's no `command`.
    pub action: Option<Command>,
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

/// A location in a model (Monaco `Location`), or with the selection ranges
/// set, a link to one (Monaco `LocationLink`). `uri: None` means the same
/// model.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Location {
    /// The model's uri, or `None` for the model that was queried.
    pub uri: Option<SharedString>,
    /// The target range (0-based rows, UTF-16 columns): the whole
    /// definition for a link, shown in peek and the definition hover.
    pub range: Range<PointUtf16>,
    /// The range of the symbol the link starts from, in the queried model
    /// (Monaco `LocationLink.originSelectionRange`): Cmd/Ctrl+hover
    /// underlines it instead of the word at the mouse.
    pub origin_selection_range: Option<Range<PointUtf16>>,
    /// The part of `range` to select and reveal when navigating, such as
    /// the definition's name (Monaco `LocationLink.targetSelectionRange`);
    /// `range` without one.
    pub target_selection_range: Option<Range<PointUtf16>>,
}

impl Location {
    /// A location of `range` in the model `uri` (`None`: the queried one).
    pub fn new(uri: Option<SharedString>, range: Range<PointUtf16>) -> Self {
        Self {
            uri,
            range,
            ..Default::default()
        }
    }

    /// The range navigation selects (Monaco `targetSelectionRange ?? range`).
    pub fn selection_range(&self) -> Range<PointUtf16> {
        self.target_selection_range
            .clone()
            .unwrap_or_else(|| self.range.clone())
    }
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

/// Edits to one or more documents (Monaco `WorkspaceEdit`).
///
/// Applying one (rename, code actions) sends the [file
/// operations](Self::file_edits) to the [file edit
/// handler](crate::set_workspace_file_edit_handler) first, then edits the
/// text of models found with [`model_for_uri`](crate::model_for_uri).
/// Nothing is applied when a model can't be found, a
/// [version](Self::versions) doesn't match, or there are file operations
/// but no handler, as in standalone Monaco's bulk edit service.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct WorkspaceEdit {
    /// Edits by document uri; `None` is the document the request was about.
    pub edits: Vec<(Option<SharedString>, Vec<TextEdit>)>,
    /// Edits inserted as snippets (Monaco `IWorkspaceTextEdit` with
    /// `insertAsSnippet`), applied with `edits`.
    pub snippet_edits: Vec<WorkspaceSnippetEdit>,
    /// Creating, renaming and deleting files (Monaco
    /// `IWorkspaceFileEdit`), in order.
    pub file_edits: Vec<WorkspaceFileEdit>,
    /// The model versions the edits were computed for (Monaco
    /// `IWorkspaceTextEdit.versionId`, against
    /// [`TextModel::version`](crate::TextModel::version)); a model that
    /// changed since makes applying fail.
    pub versions: Vec<(Option<SharedString>, usize)>,
    /// Describes the text edits (Monaco `WorkspaceEditMetadata`).
    pub metadata: Option<WorkspaceEditMetadata>,
}

impl WorkspaceEdit {
    /// Monaco's bulk edit service shows a preview before applying when an
    /// edit's metadata asks for confirmation (and a [preview
    /// handler](crate::set_workspace_edit_preview_handler) is set).
    pub fn needs_confirmation(&self) -> bool {
        self.metadata.as_ref().is_some_and(|m| m.needs_confirmation)
            || self
                .file_edits
                .iter()
                .any(|edit| edit.metadata.as_ref().is_some_and(|m| m.needs_confirmation))
    }

    /// Whether there's nothing to apply.
    pub fn is_empty(&self) -> bool {
        self.edits.iter().all(|(_, edits)| edits.is_empty())
            && self.snippet_edits.is_empty()
            && self.file_edits.is_empty()
    }
}

/// A text edit whose text is a snippet (Monaco `IWorkspaceTextEdit` with
/// `textEdit.insertAsSnippet`). In the editor applying it, a single one
/// starts a snippet session; otherwise the snippet's plain text is
/// inserted, as Monaco does for models without an editor.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct WorkspaceSnippetEdit {
    /// The document, `None` for the one the request was about.
    pub uri: Option<SharedString>,
    /// The range replaced.
    pub range: Range<PointUtf16>,
    /// The snippet (TextMate syntax).
    pub snippet: String,
    /// Insert the snippet's indentation as is (Monaco `keepWhitespace`).
    pub keep_whitespace: bool,
}

/// Monaco `WorkspaceEditMetadata`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct WorkspaceEditMetadata {
    /// Ask before applying: the edit goes through the [preview
    /// handler](crate::set_workspace_edit_preview_handler), if one is set.
    pub needs_confirmation: bool,
    /// A label for the change.
    pub label: SharedString,
    /// More about it.
    pub description: Option<SharedString>,
}

/// A file operation in a [`WorkspaceEdit`] (Monaco `IWorkspaceFileEdit`):
/// creating `new_uri`, deleting `old_uri`, or renaming (or copying)
/// `old_uri` to `new_uri`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct WorkspaceFileEdit {
    /// The file renamed or deleted (Monaco `oldResource`).
    pub old_uri: Option<SharedString>,
    /// The file created, or the rename's target (Monaco `newResource`).
    pub new_uri: Option<SharedString>,
    /// How to do it.
    pub options: WorkspaceFileEditOptions,
    /// Describes it.
    pub metadata: Option<WorkspaceEditMetadata>,
}

/// Monaco `WorkspaceFileEditOptions`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct WorkspaceFileEditOptions {
    /// Replace an existing target.
    pub overwrite: bool,
    /// Don't fail if the file to delete or rename doesn't exist.
    pub ignore_if_not_exists: bool,
    /// Don't fail (or overwrite) if the target exists.
    pub ignore_if_exists: bool,
    /// Delete folders with their contents.
    pub recursive: bool,
    /// Copy rather than rename.
    pub copy: bool,
    /// The uri is a folder.
    pub folder: bool,
    /// Delete for good rather than to the trash.
    pub skip_trash_bin: bool,
    /// Don't delete files bigger than this many bytes (Monaco `maxSize`).
    pub max_size: Option<u64>,
    /// The new file's contents (Monaco `contents`).
    pub contents: Option<String>,
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
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Command {
    /// The registered command id.
    pub id: SharedString,
    /// Its title.
    pub title: SharedString,
    /// Shown on hover, e.g. over a code lens (Monaco `tooltip`).
    pub tooltip: Option<SharedString>,
    /// Its arguments.
    pub arguments: Vec<serde_json::Value>,
}

impl Command {
    /// The command `id` titled `title`, without arguments.
    pub fn new(id: impl Into<SharedString>, title: impl Into<SharedString>) -> Self {
        Self {
            id: id.into(),
            title: title.into(),
            ..Default::default()
        }
    }
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
    /// Generated by AI (Monaco `isAI`).
    pub is_ai: bool,
    /// The ranges the action applies to (Monaco `ranges`).
    pub ranges: Vec<Range<PointUtf16>>,
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

    /// A name for the provider (Monaco `displayName`).
    fn display_name(&self) -> Option<SharedString> {
        None
    }

    /// Who provides it (Monaco `extensionId`).
    fn extension_id(&self) -> Option<SharedString> {
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
/// Register with [`register_selection_range_provider`]. Word ranges are
/// always added; when none is registered, so are the syntax tree's ranges
/// (or brackets' without a grammar).
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

/// What a folding range request is for (Monaco `FoldingContext`). Empty, as
/// in Monaco; it's there so information can be added without changing
/// [`FoldingRangeProvider`] again.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct FoldingContext {}

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
        folding_context: &FoldingContext,
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
    pub tooltip: Option<Documentation>,
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
    pub tooltip: Option<Documentation>,
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

    /// A name for the provider (Monaco `displayName`).
    fn display_name(&self) -> Option<SharedString> {
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
    /// The kind's name for screen readers (Monaco `symbolKindNames`).
    pub(crate) fn aria_name(self) -> &'static str {
        use SymbolKind::*;
        match self {
            Array => "array",
            Boolean => "boolean",
            Class => "class",
            Constant => "constant",
            Constructor => "constructor",
            Enum => "enumeration",
            EnumMember => "enumeration member",
            Event => "event",
            Field => "field",
            File => "file",
            Function => "function",
            Interface => "interface",
            Key => "key",
            Method => "method",
            Module => "module",
            Namespace => "namespace",
            Null => "null",
            Number => "number",
            Object => "object",
            Operator => "operator",
            Package => "package",
            Property => "property",
            String => "string",
            Struct => "struct",
            TypeParameter => "type parameter",
            Variable => "variable",
        }
    }

    /// The kind's codicon (Monaco `SymbolKinds.toIcon`).
    pub fn icon(self) -> Codicon {
        use SymbolKind::*;
        match self {
            File => Codicon::SymbolFile,
            Module => Codicon::SymbolModule,
            Namespace => Codicon::SymbolNamespace,
            Package => Codicon::SymbolPackage,
            Class => Codicon::SymbolClass,
            Method => Codicon::SymbolMethod,
            Property => Codicon::SymbolProperty,
            Field => Codicon::SymbolField,
            Constructor => Codicon::SymbolConstructor,
            Enum => Codicon::SymbolEnum,
            Interface => Codicon::SymbolInterface,
            Function => Codicon::SymbolFunction,
            Variable => Codicon::SymbolVariable,
            Constant => Codicon::SymbolConstant,
            String => Codicon::SymbolString,
            Number => Codicon::SymbolNumber,
            Boolean => Codicon::SymbolBoolean,
            Array => Codicon::SymbolArray,
            Object => Codicon::SymbolObject,
            Key => Codicon::SymbolKey,
            Null => Codicon::SymbolNull,
            EnumMember => Codicon::SymbolEnumMember,
            Struct => Codicon::SymbolStruct,
            Event => Codicon::SymbolEvent,
            Operator => Codicon::SymbolOperator,
            TypeParameter => Codicon::SymbolTypeParameter,
        }
    }

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
    /// The name of the symbol containing it (Monaco `containerName`).
    pub container_name: Option<SharedString>,
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

    /// A name for the provider (Monaco `displayName`).
    fn display_name(&self) -> Option<SharedString> {
        None
    }
}

// ----- workspace symbols ----------------------------------------------------------

/// A symbol anywhere in the workspace (VS Code `SymbolInformation`), for
/// "Go to Symbol in Workspace" (quick access with `#`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct WorkspaceSymbol {
    /// Its name.
    pub name: SharedString,
    /// Its kind.
    pub kind: SymbolKind,
    /// Tagged deprecated.
    pub deprecated: bool,
    /// The name of the symbol containing it, such as its class.
    pub container_name: Option<SharedString>,
    /// Its model's uri, in [`normalize_uri`](crate::normalize_uri)'s form.
    pub uri: SharedString,
    /// Where it is in `uri`, or `None` until
    /// [`WorkspaceSymbolProvider::resolve_workspace_symbol`] finds it.
    pub range: Option<Range<PointUtf16>>,
    /// Anything the provider needs to resolve it.
    pub data: Option<serde_json::Value>,
}

/// Finds symbols across the workspace (VS Code `WorkspaceSymbolProvider`).
/// Monaco has no such provider; quick access lists their symbols with
/// `#`. Unlike other providers, it's not tied to a language.
///
/// Register with [`register_workspace_symbol_provider`].
pub trait WorkspaceSymbolProvider: 'static {
    /// The symbols matching `query`. The quick access list scores them
    /// against the query too, so a provider may return more than match.
    fn provide_workspace_symbols(
        &self,
        query: &str,
        cx: &mut App,
    ) -> Task<anyhow::Result<Vec<WorkspaceSymbol>>>;

    /// Fills in a chosen symbol, such as its missing `range` (VS Code
    /// `resolveWorkspaceSymbol`); `None` if there's nothing to resolve.
    fn resolve_workspace_symbol(
        &self,
        _symbol: &WorkspaceSymbol,
        _cx: &mut App,
    ) -> Option<Task<anyhow::Result<WorkspaceSymbol>>> {
        None
    }
}

// ----- call and type hierarchy -------------------------------------------------------

/// A function, type or other symbol in a call or type hierarchy (VS Code
/// `CallHierarchyItem`, `TypeHierarchyItem`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct HierarchyItem {
    /// Its name.
    pub name: SharedString,
    /// Its kind.
    pub kind: SymbolKind,
    /// Tagged deprecated.
    pub deprecated: bool,
    /// More detail, such as a signature or module.
    pub detail: Option<SharedString>,
    /// Its model's uri, or `None` for the model that was queried.
    pub uri: Option<SharedString>,
    /// The whole definition.
    pub range: Range<PointUtf16>,
    /// The part to select and reveal, usually its name.
    pub selection_range: Range<PointUtf16>,
    /// Anything the provider needs to find its calls or types, such as a
    /// language server's item.
    pub data: Option<serde_json::Value>,
}

/// A call to an item (VS Code `CallHierarchyIncomingCall`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct IncomingCall {
    /// The caller.
    pub from: HierarchyItem,
    /// Where it makes the calls, in `from`'s model.
    pub from_ranges: Vec<Range<PointUtf16>>,
}

/// A call an item makes (VS Code `CallHierarchyOutgoingCall`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct OutgoingCall {
    /// What it calls.
    pub to: HierarchyItem,
    /// Where the item makes the calls, in the item's own model.
    pub from_ranges: Vec<Range<PointUtf16>>,
}

/// Finds who calls a function and what it calls (VS Code
/// `CallHierarchyProvider`; Monaco has none), for Peek Call Hierarchy.
///
/// Register with [`register_call_hierarchy_provider`].
pub trait CallHierarchyProvider: 'static {
    /// The items a hierarchy starts from at `position`, usually the
    /// function there.
    fn prepare_call_hierarchy(
        &self,
        context: &ProviderContext,
        position: PointUtf16,
        cx: &mut App,
    ) -> Task<anyhow::Result<Vec<HierarchyItem>>>;

    /// The calls to `item`.
    fn provide_incoming_calls(
        &self,
        item: &HierarchyItem,
        cx: &mut App,
    ) -> Task<anyhow::Result<Vec<IncomingCall>>>;

    /// The calls `item` makes.
    fn provide_outgoing_calls(
        &self,
        item: &HierarchyItem,
        cx: &mut App,
    ) -> Task<anyhow::Result<Vec<OutgoingCall>>>;
}

/// Finds a type's supertypes and subtypes (VS Code
/// `TypeHierarchyProvider`; Monaco has none), for Peek Type Hierarchy.
///
/// Register with [`register_type_hierarchy_provider`].
pub trait TypeHierarchyProvider: 'static {
    /// The items a hierarchy starts from at `position`, usually the type
    /// there.
    fn prepare_type_hierarchy(
        &self,
        context: &ProviderContext,
        position: PointUtf16,
        cx: &mut App,
    ) -> Task<anyhow::Result<Vec<HierarchyItem>>>;

    /// The types `item` extends or implements.
    fn provide_supertypes(
        &self,
        item: &HierarchyItem,
        cx: &mut App,
    ) -> Task<anyhow::Result<Vec<HierarchyItem>>>;

    /// The types that extend or implement `item`.
    fn provide_subtypes(
        &self,
        item: &HierarchyItem,
        cx: &mut App,
    ) -> Task<anyhow::Result<Vec<HierarchyItem>>>;
}

// ----- code lens -----------------------------------------------------------------

/// A command shown above a line (Monaco `CodeLens`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct CodeLens {
    /// The range it's about; it shows above its first line.
    pub range: Range<PointUtf16>,
    /// What it shows and runs; `None` until resolved.
    pub command: Option<Command>,
    /// Identifies the lens to its provider (Monaco `id`).
    pub id: Option<SharedString>,
    /// Data for [`CodeLensProvider::resolve_code_lens`], e.g. the language
    /// server's lens (`monaco-languageclient`'s `ProtocolCodeLens.data`).
    pub data: Option<serde_json::Value>,
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
    /// Identifies this request, a random UUID (Monaco `requestUuid`).
    pub request_uuid: SharedString,
    /// When the editor asked (Monaco `requestIssuedDateTime`).
    pub request_issued_date_time: web_time::SystemTime,
    /// The earliest the results show: the request time plus
    /// `inline_suggest.min_show_delay`, unless explicit or right after
    /// accepting (Monaco `earliestShownDateTime`).
    pub earliest_shown_date_time: web_time::SystemTime,
    /// The hint passed to [`refresh_inline_completions`] when that asked
    /// for this request (Monaco `changeHint`).
    pub change_hint: Option<InlineCompletionChangeHint>,
}

impl Default for InlineCompletionContext {
    /// An automatic request for both kinds, issued at the Unix epoch.
    fn default() -> Self {
        Self {
            trigger_kind: InlineCompletionTriggerKind::Automatic,
            selected_suggestion_info: None,
            include_inline_edits: true,
            include_inline_completions: true,
            request_uuid: SharedString::default(),
            request_issued_date_time: web_time::SystemTime::UNIX_EPOCH,
            earliest_shown_date_time: web_time::SystemTime::UNIX_EPOCH,
            change_hint: None,
        }
    }
}

/// What a provider passes when it asks the editor to request again
/// (Monaco `IInlineCompletionChangeHint`), handed back in the next
/// request's [`InlineCompletionContext::change_hint`].
#[derive(Clone, Debug, Default, PartialEq)]
pub struct InlineCompletionChangeHint {
    /// Whatever identifies what changed (Monaco `data`, JSON serializable).
    pub data: Option<serde_json::Value>,
}

/// How a partial accept was made (Monaco `PartialAcceptTriggerKind`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PartialAcceptTriggerKind {
    /// Accept Word (Cmd/Ctrl+Right).
    Word,
    /// Accept Line.
    Line,
    /// A suggestion was accepted from the suggest widget, and the inline
    /// completion extending it stays (Monaco `handleSuggestAccepted`).
    Suggest,
}

/// A partial accept (Monaco `PartialAcceptInfo`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PartialAcceptInfo {
    /// How it was made.
    pub kind: PartialAcceptTriggerKind,
    /// The UTF-16 length of the item's text accepted so far, from the start
    /// of its range.
    pub accepted_length: usize,
}

/// A warning shown with an inline completion (Monaco
/// `InlineCompletionWarning`): its ghost text gets a warning squiggle and
/// the toolbar shows the message.
#[derive(Clone, Debug, PartialEq)]
pub struct InlineCompletionWarning {
    /// The message, plain text or markdown.
    pub message: Documentation,
    /// Its icon; `None` is the `warning` codicon.
    pub icon: Option<Codicon>,
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
    /// Decides whether it still shows as the user types: the typed text is
    /// matched against this instead of `insert_text` (Monaco `filterText`).
    /// `None` matches against `insert_text`.
    pub filter_text: Option<String>,
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
    /// Closes the brackets `insert_text` opens and drops those it closes
    /// without opening, by the language's bracket pairs, ignoring brackets
    /// in strings and comments; the range grows or shrinks at its end by
    /// as much (Monaco `completeBracketPairs`, `closeBrackets`).
    pub complete_bracket_pairs: bool,
    /// A warning shown with the ghost text (Monaco `warning`).
    pub warning: Option<InlineCompletionWarning>,
    /// Run when it first shows (Monaco's deprecated `shownCommand`).
    pub shown_command: Option<Command>,
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

/// A command an [`InlineCompletionList`] offers (Monaco
/// `InlineCompletionCommand`).
#[derive(Clone, Debug, PartialEq)]
pub struct InlineCompletionCommand {
    /// The command, run from the menus.
    pub command: Command,
    /// Its icon in the inline edit gutter menu; `symbol-event` when `None`
    /// (Monaco `c.icon ?? Codicon.symbolEvent`). The toolbar's menu shows
    /// no icons.
    pub icon: Option<Codicon>,
}

impl InlineCompletionCommand {
    /// `command`, shown with `icon`.
    pub fn new(command: Command, icon: Option<Codicon>) -> Self {
        Self { command, icon }
    }
}

impl From<Command> for InlineCompletionCommand {
    fn from(command: Command) -> Self {
        Self::new(command, None)
    }
}

/// What an inline completions provider returns (Monaco
/// `InlineCompletions`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct InlineCompletionList {
    /// The completions, best first.
    pub items: Vec<InlineCompletion>,
    /// Keep the suggest widget from opening by itself while one shows.
    pub suppress_suggestions: Option<bool>,
    /// Extra commands for the inline suggestion toolbar's menu and the
    /// inline edit gutter menu (Monaco `InlineCompletions.commands`). A
    /// [`Command`] converts into one without an icon.
    pub commands: Vec<InlineCompletionCommand>,
    /// While the user types what a completion from this list suggests, it
    /// stays first even if a newer request returns others (Monaco
    /// `enableForwardStability`: the shown item is preserved through the
    /// requests that typing makes).
    pub enable_forward_stability: bool,
}

/// Why a provider's list is no longer used (Monaco
/// `InlineCompletionsDisposeReason`), passed to
/// [`InlineCompletionsProvider::dispose_inline_completions`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InlineCompletionsDisposeReason {
    /// Monaco stops waiting for providers once one returned a visible
    /// completion for an automatic request, and disposes the lists that
    /// come later with this reason. This editor waits for every provider,
    /// so it doesn't use it.
    LostRace,
    /// The request was superseded (typing, moving the cursor, stopping)
    /// after the list came back and before it was used.
    TokenCancellation,
    /// The list's items were shown or kept, and the last of them is gone
    /// (accepted, rejected, replaced or hidden).
    Other,
    /// The list had no items.
    Empty,
    /// None of the list's items were kept (not requested, changing nothing,
    /// or another provider's completion was preferred).
    NotTaken,
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

    /// `item` was shown for the first time, with `updated_insert_text` (its
    /// text after [`complete_bracket_pairs`](InlineCompletion::complete_bracket_pairs);
    /// empty for a jump) (Monaco `handleItemDidShow`).
    fn handle_item_did_show(&self, _item: &InlineCompletion, _updated_insert_text: &str) {}

    /// Part of `item` was accepted: word by word, line by line, or as the
    /// suggestion it extends (Monaco `handlePartialAccept`).
    fn handle_partial_accept(&self, _item: &InlineCompletion, _info: PartialAcceptInfo) {}

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

    /// `list`, which this provider returned, is no longer used: no item of
    /// it shows or will show, so what the provider keeps for it can go
    /// (Monaco `disposeInlineCompletions`). Called once per list, after
    /// its items' [`handle_end_of_lifetime`](Self::handle_end_of_lifetime).
    fn dispose_inline_completions(
        &self,
        _list: &InlineCompletionList,
        _reason: InlineCompletionsDisposeReason,
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

    /// Groups this provider excludes: while it's asked, providers in them
    /// aren't (Monaco `excludesGroupIds`, applied as Monaco's
    /// `getAvailableProviders` does).
    fn excludes_group_ids(&self) -> Vec<SharedString> {
        Vec::new()
    }

    /// The name heading the inline edit gutter menu; `None` shows "Inline
    /// Suggestion" (Monaco `displayName`).
    fn display_name(&self) -> Option<SharedString> {
        None
    }

    /// The provider's id (Monaco `providerId`, as
    /// `ProviderId.toStringWithoutVersion`: `"extension:provider"`, or just
    /// `"extension"`), for `editor.action.inlineSuggest.trigger`'s
    /// `providerId`, which names it or `"extension:*"`.
    fn provider_id(&self) -> Option<SharedString> {
        None
    }
}

/// Whether `id` (`editor.action.inlineSuggest.trigger`'s `providerId`) names
/// `provider` (Monaco `inlineCompletionProviderGetMatcher`): its
/// [`provider_id`](InlineCompletionsProvider::provider_id), or its
/// extension with `:*`.
pub(crate) fn inline_completions_provider_matches(
    provider: &dyn InlineCompletionsProvider,
    id: &str,
) -> bool {
    provider.provider_id().is_some_and(|provider_id| {
        let extension = provider_id.split(':').next().unwrap_or_default();
        provider_id.as_ref() == id || id.strip_suffix(":*") == Some(extension)
    })
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

/// Calls `f` when providers are registered or removed (Monaco's registry
/// `onDidChange`).
pub(crate) fn observe_provider_registry<V: 'static>(
    cx: &mut gpui::Context<V>,
    f: impl Fn(&mut V, &mut gpui::Context<V>) + 'static,
) -> gpui::Subscription {
    cx.observe_global::<LanguageFeatures>(f)
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
    /// Data for [`LinkProvider::resolve_link`], e.g. the language server's
    /// link (`monaco-languageclient`'s `ProtocolDocumentLink.data`).
    pub data: Option<serde_json::Value>,
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

/// Bumped when a code action provider is registered or removed; editors
/// ask for the lightbulb's actions again (Monaco's `CodeActionModel`
/// listening to `registry.onDidChange`).
#[derive(Default)]
pub(crate) struct CodeActionProvidersEpoch(pub u64);

impl Global for CodeActionProvidersEpoch {}

fn code_action_providers_changed(cx: &mut App) {
    cx.default_global::<CodeActionProvidersEpoch>().0 += 1;
}

/// Bumped when a completion provider is registered or removed; open
/// suggest sessions ask again or close (Monaco's `SuggestModel` listening
/// to `completionProvider.onDidChange`).
#[derive(Default)]
pub(crate) struct CompletionProvidersEpoch(pub u64);

impl Global for CompletionProvidersEpoch {}

fn completion_providers_changed(cx: &mut App) {
    cx.default_global::<CompletionProvidersEpoch>().0 += 1;
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
    pub documentation: Option<Documentation>,
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
    pub documentation: Option<Documentation>,
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
    /// Characters that open signature help (e.g. `(`, `,`). None by
    /// default, as Monaco's `signatureHelpTriggerCharacters` is optional: a
    /// provider without any answers only explicit requests (Cmd/Ctrl+Shift+
    /// Space) and re-queries while parameter hints show.
    fn trigger_characters(&self) -> Vec<char> {
        Vec::new()
    }

    /// Characters that re-query while it's open (e.g. `)`).
    fn retrigger_characters(&self) -> Vec<char> {
        Vec::new()
    }

    /// Returns the signatures for the call around `position` (0-based row, UTF-16
    /// column), or `None` to close parameter hints.
    ///
    /// Called on trigger characters, when explicitly requested, and on every edit
    /// while parameter hints are shown. The first provider (best first, see
    /// [`LanguageSelector`]) returning a non-empty result wins; errors are ignored.
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

/// What a formatter returns (Monaco's `TextEdit[]` from a formatting
/// provider, whose edits may also carry an `eol`).
///
/// A plain list of edits converts with `.into()`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FormattingEdits {
    /// The edits, as for [`TextEdit`].
    pub edits: Vec<TextEdit>,
    /// The line ending to switch the model to (Monaco `TextEdit.eol`, the
    /// last one given). It's set in the same undo step as the edits, before
    /// them (Monaco `pushEOL`).
    pub eol: Option<text::LineEnding>,
}

impl From<Vec<TextEdit>> for FormattingEdits {
    fn from(edits: Vec<TextEdit>) -> Self {
        Self { edits, eol: None }
    }
}

impl FormattingEdits {
    /// Appends `other`'s edits; its line ending, if it has one, wins.
    pub fn extend(&mut self, other: FormattingEdits) {
        self.edits.extend(other.edits);
        if other.eol.is_some() {
            self.eol = other.eol;
        }
    }
}

/// How the model is indented, for formatters (Monaco `FormattingOptions`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FormattingOptions {
    /// Columns per indentation level: the model's resolved
    /// [`indent_size`](crate::ModelOptions::indent_size), as Monaco passes
    /// `indentSize` here (the name follows Monaco's `tabSize` field).
    pub tab_size: u32,
    /// Indent with spaces rather than tabs.
    pub insert_spaces: bool,
}

/// Formats a whole document (Monaco `DocumentFormattingEditProvider`).
///
/// Register with [`register_document_formatting_provider`]. When several
/// formatters match a model, [`set_formatter_selector`] picks one.
pub trait DocumentFormattingProvider: 'static {
    /// The edits that format the document. Big replacements are fine: the
    /// editor reduces them to the characters that actually change.
    fn provide_document_formatting_edits(
        &self,
        context: &ProviderContext,
        options: FormattingOptions,
        cx: &mut App,
    ) -> Task<anyhow::Result<FormattingEdits>>;

    /// The name to show when choosing between formatters (Monaco
    /// `displayName`).
    fn display_name(&self) -> Option<SharedString> {
        None
    }

    /// Who provides it (Monaco's internal `extensionId`): a range formatter
    /// with the same id isn't offered again as a document formatter.
    fn extension_id(&self) -> Option<SharedString> {
        None
    }
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
    ) -> Task<anyhow::Result<FormattingEdits>>;

    /// The edits that format all of `ranges` at once (Monaco
    /// `provideDocumentRangesFormattingEdits`), for several selections.
    /// `None`, the default, formats them one at a time.
    fn provide_document_ranges_formatting_edits(
        &self,
        _context: &ProviderContext,
        _ranges: &[Range<PointUtf16>],
        _options: FormattingOptions,
        _cx: &mut App,
    ) -> Option<Task<anyhow::Result<FormattingEdits>>> {
        None
    }

    /// The name to show when choosing between formatters (Monaco
    /// `displayName`).
    fn display_name(&self) -> Option<SharedString> {
        None
    }

    /// Who provides it (Monaco's internal `extensionId`); see
    /// [`DocumentFormattingProvider::extension_id`].
    fn extension_id(&self) -> Option<SharedString> {
        None
    }
}

/// What is being formatted (Monaco `FormattingKind`), for
/// [`set_formatter_selector`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FormattingKind {
    /// The whole document (`editor.action.formatDocument`).
    File,
    /// Selections or pasted text (`editor.action.formatSelection`,
    /// `format_on_paste`).
    Selection,
}

/// Why it's formatted (Monaco `FormattingMode`), for
/// [`set_formatter_selector`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FormattingMode {
    /// The user asked (a format command).
    Explicit,
    /// Automatically (`format_on_paste`): don't interrupt the user.
    Silent,
}

/// A formatter offered to a [`set_formatter_selector`] selector.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FormatterInfo {
    /// Its [`display_name`](DocumentFormattingProvider::display_name).
    pub display_name: Option<SharedString>,
    /// Its [`extension_id`](DocumentFormattingProvider::extension_id).
    pub extension_id: Option<SharedString>,
}

/// The formatters a [`set_formatter_selector`] selector chooses from
/// (Monaco `IFormattingEditProviderSelector`'s arguments).
#[derive(Clone)]
pub struct FormatterSelection {
    /// The matching formatters, in the order they're tried (best first, see
    /// [`LanguageSelector`]). For [`FormattingKind::File`], the document formatters, then
    /// the range formatters (formatting the whole text) whose extension id
    /// no document formatter has.
    pub formatters: Vec<FormatterInfo>,
    /// The model being formatted (Monaco `document`).
    pub context: ProviderContext,
    /// Why.
    pub mode: FormattingMode,
    /// What.
    pub kind: FormattingKind,
}

type FormatterSelector = dyn Fn(&FormatterSelection, &mut App) -> Task<Option<usize>>;

/// The selectors from [`set_formatter_selector`], newest first.
#[derive(Default)]
struct FormatterSelectors {
    next_id: u64,
    selectors: Vec<(u64, std::rc::Rc<FormatterSelector>)>,
}

impl Global for FormatterSelectors {}

/// Chooses which formatter runs when formatting (Monaco
/// `FormattingConflicts.setFormatterSelector`): `selector` gets the
/// matching formatters and answers the index of the one to use, or `None`
/// to not format, e.g. after asking the user. The newest selector set is
/// used; [`Disposable::dispose`](crate::Disposable::dispose) removes it.
///
/// Without one, the first formatter is used, as in the standalone editor
/// (`editor.api.ts`). With one, it's asked whenever a formatter matches,
/// even only one, as in Monaco; on-type formatting doesn't ask.
pub fn set_formatter_selector(
    cx: &mut App,
    selector: impl Fn(&FormatterSelection, &mut App) -> Task<Option<usize>> + 'static,
) -> crate::Disposable {
    let selectors = cx.default_global::<FormatterSelectors>();
    selectors.next_id += 1;
    let id = selectors.next_id;
    selectors
        .selectors
        .insert(0, (id, std::rc::Rc::new(selector)));
    crate::Disposable::new(move |cx| {
        cx.default_global::<FormatterSelectors>()
            .selectors
            .retain(|(other, _)| *other != id);
    })
}

/// Monaco `FormattingConflicts.select`: the newest selector's choice, or
/// `None` without a selector (the caller then takes the first formatter).
pub(crate) fn select_formatter(
    selection: &FormatterSelection,
    cx: &mut App,
) -> Option<Task<Option<usize>>> {
    let selector = cx
        .try_global::<FormatterSelectors>()?
        .selectors
        .first()?
        .1
        .clone();
    Some(selector(selection, cx))
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
    ) -> Task<anyhow::Result<FormattingEdits>>;
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

    /// Each MIME type with its data, in the order they were added.
    pub fn iter(&self) -> impl Iterator<Item = (&SharedString, &DataTransferItem)> {
        self.items.iter().map(|(mime, item)| (mime, item))
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
    /// Other edits in this document, applied with it.
    pub additional_edits: Vec<TextEdit>,
    /// Edits applied after it, possibly in other documents (Monaco
    /// `additionalEdit`), as [`CodeEditor::apply_workspace_edit`](crate::CodeEditor::apply_workspace_edit)
    /// does.
    pub additional_edit: Option<WorkspaceEdit>,
    /// Edits it should rank below.
    pub yield_to: Vec<EditYieldTo>,
}

/// Offers ways to insert dropped data (Monaco `DocumentDropEditProvider`).
///
/// Register with [`register_document_drop_edit_provider`].
pub trait DocumentDropEditProvider: 'static {
    /// Identifies the provider (Monaco `id`).
    fn id(&self) -> Option<SharedString> {
        None
    }

    /// The MIME types it handles (`None`: any).
    fn drop_mime_types(&self) -> Option<Vec<SharedString>> {
        None
    }

    /// The kinds of edits it returns (Monaco `providedDropEditKinds`).
    fn provided_drop_edit_kinds(&self) -> Vec<SharedString> {
        Vec::new()
    }

    /// The edits for dropping `data` at `position`.
    fn provide_document_drop_edits(
        &self,
        context: &ProviderContext,
        position: PointUtf16,
        data: &DataTransfer,
        cx: &mut App,
    ) -> Task<anyhow::Result<Vec<DocumentDropOrPasteEdit>>>;

    /// Fills in an edit before it's applied (Monaco
    /// `resolveDocumentDropEdit`), e.g. computing its `additional_edit`
    /// only for the edit that's used. `None` (the default) applies it as
    /// it is.
    fn resolve_document_drop_edit(
        &self,
        _edit: &DocumentDropOrPasteEdit,
        _cx: &mut App,
    ) -> Option<Task<anyhow::Result<DocumentDropOrPasteEdit>>> {
        None
    }
}

/// What a paste edit request is for (Monaco `DocumentPasteContext`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DocumentPasteContext {
    /// Why it's asked for.
    pub trigger_kind: DocumentPasteTriggerKind,
    /// Only edits of this kind are wanted ("Paste As..." with a kind), if
    /// set (Monaco `only`).
    pub only: Option<SharedString>,
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
    /// Identifies the provider (Monaco `id`).
    fn id(&self) -> Option<SharedString> {
        None
    }

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

    /// Like [`provide_document_paste_edits`](Self::provide_document_paste_edits),
    /// with Monaco's whole `DocumentPasteContext`, including the kind
    /// "Paste As..." asks for. Defaults to `provide_document_paste_edits`.
    fn provide_document_paste_edits_with_context(
        &self,
        context: &ProviderContext,
        ranges: &[Range<PointUtf16>],
        data: &DataTransfer,
        paste_context: &DocumentPasteContext,
        cx: &mut App,
    ) -> Task<anyhow::Result<Vec<DocumentDropOrPasteEdit>>> {
        self.provide_document_paste_edits(context, ranges, data, paste_context.trigger_kind, cx)
    }

    /// Fills in an edit before it's applied (Monaco
    /// `resolveDocumentPasteEdit`). `None` (the default) applies it as it
    /// is.
    fn resolve_document_paste_edit(
        &self,
        _edit: &DocumentDropOrPasteEdit,
        _cx: &mut App,
    ) -> Option<Task<anyhow::Result<DocumentDropOrPasteEdit>>> {
        None
    }

    /// The MIME types [`prepare_document_paste`](Self::prepare_document_paste)
    /// adds at copy time (Monaco `copyMimeTypes`), so that pasting the text
    /// asks the providers that handle them.
    fn copy_mime_types(&self) -> Vec<SharedString> {
        Vec::new()
    }

    /// Runs when text is copied or cut in an editor with this provider
    /// (Monaco `prepareDocumentPaste`): returns data to add to the paste's
    /// [`DataTransfer`] when that copy is pasted, such as what the copied
    /// code imports. `ranges` are the copied ranges (whole lines for empty
    /// selections). Data of higher priority providers wins. `None` (the
    /// default) if it doesn't prepare pastes.
    fn prepare_document_paste(
        &self,
        _context: &ProviderContext,
        _ranges: &[Range<PointUtf16>],
        _cx: &mut App,
    ) -> Option<Task<anyhow::Result<DataTransfer>>> {
        None
    }

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
    workspace_symbols: Vec<Entry<dyn WorkspaceSymbolProvider>>,
    call_hierarchy: Vec<Entry<dyn CallHierarchyProvider>>,
    type_hierarchy: Vec<Entry<dyn TypeHierarchyProvider>>,
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

/// Registers a completion provider for the models `selector` matches (Monaco
/// `registerCompletionItemProvider`). Providers are asked in groups of equal
/// [score](LanguageSelector::score), best first, and the first group with items
/// wins; the lists of that group's providers are merged. Open suggest sessions
/// ask again, as they do when one is unregistered (Monaco's `SuggestModel` on
/// `completionProvider.onDidChange`).
pub fn register_completion_provider(
    cx: &mut App,
    selector: LanguageSelector,
    provider: impl CompletionProvider,
) -> ProviderRegistration {
    let features = features(cx);
    features.next_id += 1;
    let id = ProviderRegistration(features.next_id);
    features.completion.push(Entry {
        id,
        selector,
        provider: Arc::new(provider),
    });
    completion_providers_changed(cx);
    id
}
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
    /// (Monaco `registerDocumentHighlightProvider`). The first non-empty result, best
    /// first (see [`LanguageSelector`]), is shown; with none, the word's textual
    /// occurrences are.
    register_document_highlight_provider,
    document_highlight,
    DocumentHighlightProvider
);

register_fn!(
    /// Registers a signature help provider for the models `selector` matches (Monaco
    /// `registerSignatureHelpProvider`). The first non-empty result, best first (see
    /// [`LanguageSelector`]), is shown.
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
/// Registers a folding range provider for the models `selector` matches (Monaco
/// `registerFoldingRangeProvider`). Results from all matching providers are merged.
/// Editors compute their ranges again, as they do when one is unregistered (Monaco
/// `foldingRangeProvider.onDidChange`).
pub fn register_folding_range_provider(
    cx: &mut App,
    selector: LanguageSelector,
    provider: impl FoldingRangeProvider,
) -> ProviderRegistration {
    let features = features(cx);
    features.next_id += 1;
    let id = ProviderRegistration(features.next_id);
    features.folding.push(Entry {
        id,
        selector,
        provider: Arc::new(provider),
    });
    refresh_folding_ranges(cx);
    id
}
register_fn!(
    /// Registers a document symbol provider for the models `selector`
    /// matches (Monaco `registerDocumentSymbolProvider`).
    register_document_symbol_provider,
    document_symbols,
    DocumentSymbolProvider
);
register_fn!(
    /// Registers a call hierarchy provider for the models `selector`
    /// matches (VS Code `registerCallHierarchyProvider`). Peek Call
    /// Hierarchy uses the best matching one with items.
    register_call_hierarchy_provider,
    call_hierarchy,
    CallHierarchyProvider
);
register_fn!(
    /// Registers a type hierarchy provider for the models `selector`
    /// matches (VS Code `registerTypeHierarchyProvider`). Peek Type
    /// Hierarchy uses the best matching one with items.
    register_type_hierarchy_provider,
    type_hierarchy,
    TypeHierarchyProvider
);

/// The registered call hierarchy providers whose selector matches
/// `target`, best first (see [`LanguageSelector`]).
pub fn call_hierarchy_providers(
    cx: &App,
    target: impl Into<ProviderTarget>,
) -> Vec<Arc<dyn CallHierarchyProvider>> {
    cx.try_global::<LanguageFeatures>()
        .map(|f| matching(&f.call_hierarchy, &target.into()))
        .unwrap_or_default()
}

/// The registered type hierarchy providers whose selector matches
/// `target`, best first (see [`LanguageSelector`]).
pub fn type_hierarchy_providers(
    cx: &App,
    target: impl Into<ProviderTarget>,
) -> Vec<Arc<dyn TypeHierarchyProvider>> {
    cx.try_global::<LanguageFeatures>()
        .map(|f| matching(&f.type_hierarchy, &target.into()))
        .unwrap_or_default()
}

/// Registers a workspace symbol provider (VS Code
/// `registerWorkspaceSymbolProvider`): quick access lists the symbols of
/// every registered one with `#`.
pub fn register_workspace_symbol_provider(
    cx: &mut App,
    provider: impl WorkspaceSymbolProvider,
) -> ProviderRegistration {
    let features = features(cx);
    features.next_id += 1;
    let id = ProviderRegistration(features.next_id);
    features.workspace_symbols.push(Entry {
        id,
        selector: LanguageSelector::All,
        provider: Arc::new(provider),
    });
    id
}

/// The registered workspace symbol providers, in registration order.
pub fn workspace_symbol_providers(cx: &App) -> Vec<Arc<dyn WorkspaceSymbolProvider>> {
    cx.try_global::<LanguageFeatures>()
        .map(|f| {
            f.workspace_symbols
                .iter()
                .map(|e| e.provider.clone())
                .collect()
        })
        .unwrap_or_default()
}

/// Registers a code lens provider for the models `selector` matches
/// (Monaco `registerCodeLensProvider`). Lenses from all are shown.
/// Editors ask for lenses again, as they do when one is unregistered
/// (Monaco `codeLensProvider.onDidChange`).
pub fn register_code_lens_provider(
    cx: &mut App,
    selector: LanguageSelector,
    provider: impl CodeLensProvider,
) -> ProviderRegistration {
    let features = features(cx);
    features.next_id += 1;
    let id = ProviderRegistration(features.next_id);
    features.code_lens.push(Entry {
        id,
        selector,
        provider: Arc::new(provider),
    });
    refresh_code_lenses(cx);
    id
}
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
    /// The best-ranked one (see [`LanguageSelector`]) with tokens is used.
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
    /// `registerLinkProvider`). Better-ranked providers (see [`LanguageSelector`]) win where links overlap.
    register_link_provider,
    links,
    LinkProvider
);
/// Registers an inlay hints provider for the models `selector` matches (Monaco
/// `registerInlayHintsProvider`). Results from all matching providers are merged.
/// Editors ask for hints again, as they do when one is unregistered
/// (Monaco `inlayHintsProvider.onDidChange`).
pub fn register_inlay_hints_provider(
    cx: &mut App,
    selector: LanguageSelector,
    provider: impl InlayHintsProvider,
) -> ProviderRegistration {
    let features = features(cx);
    features.next_id += 1;
    let id = ProviderRegistration(features.next_id);
    features.inlay_hints.push(Entry {
        id,
        selector,
        provider: Arc::new(provider),
    });
    refresh_inlay_hints(cx);
    id
}
/// Registers a code action provider for the models `selector` matches (Monaco
/// `registerCodeActionProvider`). Results from all matching providers are merged.
/// Editors ask for the lightbulb's actions again, as they do when one is
/// unregistered (Monaco's `CodeActionModel` on `registry.onDidChange`).
pub fn register_code_action_provider(
    cx: &mut App,
    selector: LanguageSelector,
    provider: impl CodeActionProvider,
) -> ProviderRegistration {
    let features = features(cx);
    features.next_id += 1;
    let id = ProviderRegistration(features.next_id);
    features.code_action.push(Entry {
        id,
        selector,
        provider: Arc::new(provider),
    });
    code_action_providers_changed(cx);
    id
}
register_fn!(
    /// Registers a rename provider for the models `selector` matches (Monaco
    /// `registerRenameProvider`). Providers are tried best first (see [`LanguageSelector`]).
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
    /// `registerDocumentFormattingEditProvider`). The best-ranked one is used.
    register_document_formatting_provider,
    document_formatting,
    DocumentFormattingProvider
);
register_fn!(
    /// Registers a range formatter for the models `selector` matches (Monaco
    /// `registerDocumentRangeFormattingEditProvider`). The best-ranked one is used.
    register_document_range_formatting_provider,
    range_formatting,
    DocumentRangeFormattingProvider
);
register_fn!(
    /// Registers an on-type formatter for the models `selector` matches (Monaco
    /// `registerOnTypeFormattingEditProvider`). The best-ranked one is used.
    register_on_type_formatting_provider,
    on_type_formatting,
    OnTypeFormattingProvider
);

/// Removes a provider registered with any `register_*_provider`.
pub fn unregister_provider(cx: &mut App, registration: ProviderRegistration) {
    let features = features(cx);
    features.document_highlight.retain(|e| e.id != registration);
    let completion_changed = features.completion.len();
    features.completion.retain(|e| e.id != registration);
    let completion_changed = features.completion.len() != completion_changed;
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
    let code_action_changed = features.code_action.len();
    features.code_action.retain(|e| e.id != registration);
    let code_action_changed = features.code_action.len() != code_action_changed;
    let inlay_hints_changed = features.inlay_hints.len();
    features.inlay_hints.retain(|e| e.id != registration);
    let inlay_hints_changed = features.inlay_hints.len() != inlay_hints_changed;
    features.links.retain(|e| e.id != registration);
    features.semantic_tokens.retain(|e| e.id != registration);
    features.inline_completions.retain(|e| e.id != registration);
    features.linked_editing.retain(|e| e.id != registration);
    let code_lens_changed = features.code_lens.len();
    features.code_lens.retain(|e| e.id != registration);
    let code_lens_changed = features.code_lens.len() != code_lens_changed;
    features.document_symbols.retain(|e| e.id != registration);
    features.workspace_symbols.retain(|e| e.id != registration);
    features.call_hierarchy.retain(|e| e.id != registration);
    features.type_hierarchy.retain(|e| e.id != registration);
    features
        .range_semantic_tokens
        .retain(|e| e.id != registration);
    let folding_providers = features.folding.len();
    features.folding.retain(|e| e.id != registration);
    let folding_changed = features.folding.len() != folding_providers;
    features.selection_range.retain(|e| e.id != registration);
    features.type_definition.retain(|e| e.id != registration);
    features.implementation.retain(|e| e.id != registration);
    features.references.retain(|e| e.id != registration);
    features.on_type_formatting.retain(|e| e.id != registration);
    features.drop_edits.retain(|e| e.id != registration);
    features.paste_edits.retain(|e| e.id != registration);
    if folding_changed {
        refresh_folding_ranges(cx);
    }
    // Monaco's per-feature registries fire `onDidChange` on removal too.
    if inlay_hints_changed {
        refresh_inlay_hints(cx);
    }
    if code_lens_changed {
        refresh_code_lenses(cx);
    }
    if code_action_changed {
        code_action_providers_changed(cx);
    }
    if completion_changed {
        completion_providers_changed(cx);
    }
}

/// The registered drop edit providers whose selector matches
/// `target`, best first (see [`LanguageSelector`]), then the built-in ones.
pub fn drop_edit_providers(
    cx: &App,
    target: impl Into<ProviderTarget>,
) -> Vec<Arc<dyn DocumentDropEditProvider>> {
    let target = target.into();
    let mut providers = cx
        .try_global::<LanguageFeatures>()
        .map(|f| matching(&f.drop_edits, &target))
        .unwrap_or_default();
    providers.extend(crate::drop_paste::default_drop_providers());
    providers
}

/// The registered paste edit providers whose selector matches
/// `target`, best first (see [`LanguageSelector`]), then the built-in ones.
pub fn paste_edit_providers(
    cx: &App,
    target: impl Into<ProviderTarget>,
) -> Vec<Arc<dyn DocumentPasteEditProvider>> {
    let target = target.into();
    let mut providers = cx
        .try_global::<LanguageFeatures>()
        .map(|f| matching(&f.paste_edits, &target))
        .unwrap_or_default();
    providers.extend(crate::drop_paste::default_paste_providers());
    providers
}

/// Monaco `LanguageFeatureRegistry._orderedForEach`: the entries matching
/// `target` with their scores, by score (best first), then newest first
/// (`_compareByScoreAndTime`). A matching exclusive selector leaves only
/// its provider.
fn ordered<'a, T: ?Sized>(
    entries: &'a [Entry<T>],
    target: &ProviderTarget,
) -> Vec<(u32, &'a Entry<T>)> {
    let mut scored: Vec<(u32, &Entry<T>)> = entries
        .iter()
        .map(|entry| (entry.selector.score(target), entry))
        .filter(|(score, _)| *score > 0)
        .collect();
    // Monaco gives the first exclusive match the whole registry; entries
    // are kept newest first here, so that's the newest one.
    if let Some(exclusive) = scored
        .iter()
        .rev()
        .find(|(_, entry)| entry.selector.is_exclusive())
        .map(|(_, entry)| *entry)
    {
        return vec![(1000, exclusive)];
    }
    scored.sort_by(|(a_score, a), (b_score, b)| b_score.cmp(a_score).then(b.id.0.cmp(&a.id.0)));
    scored
}

/// The providers of `entries` matching `target`, best first (Monaco
/// `LanguageFeatureRegistry.ordered`).
fn matching<T: ?Sized>(entries: &[Entry<T>], target: &ProviderTarget) -> Vec<Arc<T>> {
    ordered(entries, target)
        .into_iter()
        .map(|(_, entry)| entry.provider.clone())
        .collect()
}

/// The providers of `entries` matching `target` in groups of equal score,
/// best first (Monaco `LanguageFeatureRegistry.orderedGroups`).
fn matching_groups<T: ?Sized>(entries: &[Entry<T>], target: &ProviderTarget) -> Vec<Vec<Arc<T>>> {
    let mut groups: Vec<(u32, Vec<Arc<T>>)> = Vec::new();
    for (score, entry) in ordered(entries, target) {
        match groups.last_mut() {
            Some((last, group)) if *last == score => group.push(entry.provider.clone()),
            _ => groups.push((score, vec![entry.provider.clone()])),
        }
    }
    groups.into_iter().map(|(_, group)| group).collect()
}

/// The registered completion providers whose selector matches `target`,
/// best first: by [score](LanguageSelector::score), then newest first.
pub fn completion_providers(
    cx: &App,
    target: impl Into<ProviderTarget>,
) -> Vec<Arc<dyn CompletionProvider>> {
    cx.try_global::<LanguageFeatures>()
        .map(|f| matching(&f.completion, &target.into()))
        .unwrap_or_default()
}

/// [`completion_providers`] in groups of equal score, best first (Monaco
/// `orderedGroups`). The suggest widget asks one group at a time and stops
/// at the first with items.
pub fn completion_provider_groups(
    cx: &App,
    target: impl Into<ProviderTarget>,
) -> Vec<Vec<Arc<dyn CompletionProvider>>> {
    cx.try_global::<LanguageFeatures>()
        .map(|f| matching_groups(&f.completion, &target.into()))
        .unwrap_or_default()
}

/// The registered hover providers whose selector matches `target`,
/// best first (see [`LanguageSelector`]).
pub fn hover_providers(cx: &App, target: impl Into<ProviderTarget>) -> Vec<Arc<dyn HoverProvider>> {
    cx.try_global::<LanguageFeatures>()
        .map(|f| matching(&f.hover, &target.into()))
        .unwrap_or_default()
}

/// The registered definition providers whose selector matches `target`,
/// best first (see [`LanguageSelector`]).
pub fn definition_providers(
    cx: &App,
    target: impl Into<ProviderTarget>,
) -> Vec<Arc<dyn DefinitionProvider>> {
    cx.try_global::<LanguageFeatures>()
        .map(|f| matching(&f.definition, &target.into()))
        .unwrap_or_default()
}

/// The registered signature help providers whose selector matches `target`,
/// best first (see [`LanguageSelector`]).
pub fn signature_help_providers(
    cx: &App,
    target: impl Into<ProviderTarget>,
) -> Vec<Arc<dyn SignatureHelpProvider>> {
    cx.try_global::<LanguageFeatures>()
        .map(|f| matching(&f.signature_help, &target.into()))
        .unwrap_or_default()
}

/// The registered document highlight providers whose selector matches
/// `target`, best first (see [`LanguageSelector`]).
pub fn document_highlight_providers(
    cx: &App,
    target: impl Into<ProviderTarget>,
) -> Vec<Arc<dyn DocumentHighlightProvider>> {
    cx.try_global::<LanguageFeatures>()
        .map(|f| matching(&f.document_highlight, &target.into()))
        .unwrap_or_default()
}

/// The registered document formatters whose selector matches `target`,
/// best first (see [`LanguageSelector`]).
pub fn document_formatting_providers(
    cx: &App,
    target: impl Into<ProviderTarget>,
) -> Vec<Arc<dyn DocumentFormattingProvider>> {
    cx.try_global::<LanguageFeatures>()
        .map(|f| matching(&f.document_formatting, &target.into()))
        .unwrap_or_default()
}

/// The registered range formatters whose selector matches `target`,
/// best first (see [`LanguageSelector`]).
pub fn document_range_formatting_providers(
    cx: &App,
    target: impl Into<ProviderTarget>,
) -> Vec<Arc<dyn DocumentRangeFormattingProvider>> {
    cx.try_global::<LanguageFeatures>()
        .map(|f| matching(&f.range_formatting, &target.into()))
        .unwrap_or_default()
}

/// The registered on-type formatters whose selector matches `target`,
/// best first (see [`LanguageSelector`]).
pub fn on_type_formatting_providers(
    cx: &App,
    target: impl Into<ProviderTarget>,
) -> Vec<Arc<dyn OnTypeFormattingProvider>> {
    cx.try_global::<LanguageFeatures>()
        .map(|f| matching(&f.on_type_formatting, &target.into()))
        .unwrap_or_default()
}

/// The registered declaration providers whose selector matches `target`,
/// best first (see [`LanguageSelector`]).
pub fn declaration_providers(
    cx: &App,
    target: impl Into<ProviderTarget>,
) -> Vec<Arc<dyn DeclarationProvider>> {
    cx.try_global::<LanguageFeatures>()
        .map(|f| matching(&f.declaration, &target.into()))
        .unwrap_or_default()
}

/// The registered type definition providers whose selector matches `target`,
/// best first (see [`LanguageSelector`]).
pub fn type_definition_providers(
    cx: &App,
    target: impl Into<ProviderTarget>,
) -> Vec<Arc<dyn TypeDefinitionProvider>> {
    cx.try_global::<LanguageFeatures>()
        .map(|f| matching(&f.type_definition, &target.into()))
        .unwrap_or_default()
}

/// The registered implementation providers whose selector matches `target`,
/// best first (see [`LanguageSelector`]).
pub fn implementation_providers(
    cx: &App,
    target: impl Into<ProviderTarget>,
) -> Vec<Arc<dyn ImplementationProvider>> {
    cx.try_global::<LanguageFeatures>()
        .map(|f| matching(&f.implementation, &target.into()))
        .unwrap_or_default()
}

/// The registered reference providers whose selector matches `target`,
/// best first (see [`LanguageSelector`]).
pub fn reference_providers(
    cx: &App,
    target: impl Into<ProviderTarget>,
) -> Vec<Arc<dyn ReferenceProvider>> {
    cx.try_global::<LanguageFeatures>()
        .map(|f| matching(&f.references, &target.into()))
        .unwrap_or_default()
}

/// The registered rename providers whose selector matches `target`,
/// best first (see [`LanguageSelector`]).
pub fn rename_providers(
    cx: &App,
    target: impl Into<ProviderTarget>,
) -> Vec<Arc<dyn RenameProvider>> {
    cx.try_global::<LanguageFeatures>()
        .map(|f| matching(&f.rename, &target.into()))
        .unwrap_or_default()
}

/// The registered rename suggestions providers whose selector matches
/// `target`, best first (see [`LanguageSelector`]).
pub fn new_symbol_names_providers(
    cx: &App,
    target: impl Into<ProviderTarget>,
) -> Vec<Arc<dyn NewSymbolNamesProvider>> {
    cx.try_global::<LanguageFeatures>()
        .map(|f| matching(&f.new_symbol_names, &target.into()))
        .unwrap_or_default()
}

/// The registered code action providers whose selector matches
/// `target`, best first (see [`LanguageSelector`]).
pub fn code_action_providers(
    cx: &App,
    target: impl Into<ProviderTarget>,
) -> Vec<Arc<dyn CodeActionProvider>> {
    cx.try_global::<LanguageFeatures>()
        .map(|f| matching(&f.code_action, &target.into()))
        .unwrap_or_default()
}

/// The registered document symbol providers whose selector matches
/// `target`, best first (see [`LanguageSelector`]).
pub fn document_symbol_providers(
    cx: &App,
    target: impl Into<ProviderTarget>,
) -> Vec<Arc<dyn DocumentSymbolProvider>> {
    cx.try_global::<LanguageFeatures>()
        .map(|f| matching(&f.document_symbols, &target.into()))
        .unwrap_or_default()
}

/// The registered code lens providers whose selector matches
/// `target`, best first (see [`LanguageSelector`]).
pub fn code_lens_providers(
    cx: &App,
    target: impl Into<ProviderTarget>,
) -> Vec<Arc<dyn CodeLensProvider>> {
    cx.try_global::<LanguageFeatures>()
        .map(|f| matching(&f.code_lens, &target.into()))
        .unwrap_or_default()
}

/// The registered linked editing range providers whose selector matches
/// `target`, best first (see [`LanguageSelector`]).
pub fn linked_editing_range_providers(
    cx: &App,
    target: impl Into<ProviderTarget>,
) -> Vec<Arc<dyn LinkedEditingRangeProvider>> {
    cx.try_global::<LanguageFeatures>()
        .map(|f| matching(&f.linked_editing, &target.into()))
        .unwrap_or_default()
}

/// The registered inline completions providers whose selector matches
/// `target`, best first (see [`LanguageSelector`]).
pub fn inline_completions_providers(
    cx: &App,
    target: impl Into<ProviderTarget>,
) -> Vec<Arc<dyn InlineCompletionsProvider>> {
    cx.try_global::<LanguageFeatures>()
        .map(|f| matching(&f.inline_completions, &target.into()))
        .unwrap_or_default()
}

/// The inline completions provider registered as `registration`.
pub(crate) fn inline_completions_provider(
    cx: &App,
    registration: ProviderRegistration,
) -> Option<Arc<dyn InlineCompletionsProvider>> {
    cx.try_global::<LanguageFeatures>()?
        .inline_completions
        .iter()
        .find(|entry| entry.id == registration)
        .map(|entry| entry.provider.clone())
}

/// The inline completions provider registered as `provider` has new
/// results (Monaco's provider `onDidChangeInlineCompletions`): the focused
/// editor it serves asks it again, passing `change_hint` in the request's
/// [`InlineCompletionContext::change_hint`]. An editor showing another
/// provider's suggestion ignores it; with
/// `inline_suggest.trigger_command_on_provider_change` every provider is
/// asked for inline edits instead.
pub fn refresh_inline_completions(
    cx: &mut App,
    provider: ProviderRegistration,
    change_hint: Option<InlineCompletionChangeHint>,
) {
    // Deferred, so a provider can call it while the editor asks it.
    cx.defer(move |cx| {
        let Some(provider) = inline_completions_provider(cx, provider) else {
            return;
        };
        for editor in crate::registry::editors(cx) {
            let provider = provider.clone();
            let change_hint = change_hint.clone();
            editor.update(cx, |editor, cx| {
                editor.inline_provider_changed(provider, change_hint, cx)
            });
        }
    });
}

/// The registered document semantic tokens providers whose selector
/// matches `target`, best first (see [`LanguageSelector`]).
pub fn document_semantic_tokens_providers(
    cx: &App,
    target: impl Into<ProviderTarget>,
) -> Vec<Arc<dyn DocumentSemanticTokensProvider>> {
    cx.try_global::<LanguageFeatures>()
        .map(|f| matching(&f.semantic_tokens, &target.into()))
        .unwrap_or_default()
}

/// [`document_semantic_tokens_providers`] in groups of equal score, best
/// first (Monaco `orderedGroups`). The editor asks only the first group,
/// as Monaco's `getDocumentSemanticTokens` does.
pub fn document_semantic_tokens_provider_groups(
    cx: &App,
    target: impl Into<ProviderTarget>,
) -> Vec<Vec<Arc<dyn DocumentSemanticTokensProvider>>> {
    cx.try_global::<LanguageFeatures>()
        .map(|f| matching_groups(&f.semantic_tokens, &target.into()))
        .unwrap_or_default()
}

/// [`document_range_semantic_tokens_providers`] in groups of equal score,
/// best first (Monaco `orderedGroups`). The editor asks every provider of
/// the first group, as Monaco's `getDocumentRangeSemanticTokens` does.
pub fn document_range_semantic_tokens_provider_groups(
    cx: &App,
    target: impl Into<ProviderTarget>,
) -> Vec<Vec<Arc<dyn DocumentRangeSemanticTokensProvider>>> {
    cx.try_global::<LanguageFeatures>()
        .map(|f| matching_groups(&f.range_semantic_tokens, &target.into()))
        .unwrap_or_default()
}

/// The registered range semantic tokens providers whose selector matches
/// `target`, best first (see [`LanguageSelector`]).
pub fn document_range_semantic_tokens_providers(
    cx: &App,
    target: impl Into<ProviderTarget>,
) -> Vec<Arc<dyn DocumentRangeSemanticTokensProvider>> {
    cx.try_global::<LanguageFeatures>()
        .map(|f| matching(&f.range_semantic_tokens, &target.into()))
        .unwrap_or_default()
}

/// The registered color providers whose selector matches `target`,
/// best first (see [`LanguageSelector`]).
pub fn color_providers(
    cx: &App,
    target: impl Into<ProviderTarget>,
) -> Vec<Arc<dyn DocumentColorProvider>> {
    cx.try_global::<LanguageFeatures>()
        .map(|f| matching(&f.colors, &target.into()))
        .unwrap_or_default()
}

/// The registered link providers whose selector matches `target`,
/// best first (see [`LanguageSelector`]).
pub fn link_providers(cx: &App, target: impl Into<ProviderTarget>) -> Vec<Arc<dyn LinkProvider>> {
    cx.try_global::<LanguageFeatures>()
        .map(|f| matching(&f.links, &target.into()))
        .unwrap_or_default()
}

/// The registered inlay hints providers whose selector matches
/// `target`, best first (see [`LanguageSelector`]).
pub fn inlay_hints_providers(
    cx: &App,
    target: impl Into<ProviderTarget>,
) -> Vec<Arc<dyn InlayHintsProvider>> {
    cx.try_global::<LanguageFeatures>()
        .map(|f| matching(&f.inlay_hints, &target.into()))
        .unwrap_or_default()
}

/// The registered folding range providers whose selector matches
/// `target`, best first (see [`LanguageSelector`]).
pub fn folding_range_providers(
    cx: &App,
    target: impl Into<ProviderTarget>,
) -> Vec<Arc<dyn FoldingRangeProvider>> {
    cx.try_global::<LanguageFeatures>()
        .map(|f| matching(&f.folding, &target.into()))
        .unwrap_or_default()
}

/// The registered selection range providers whose selector matches
/// `target`, best first (see [`LanguageSelector`]).
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

    fn target(language: &str, uri: &str) -> ProviderTarget {
        ProviderTarget {
            language_id: Some(language.to_string().into()),
            uri: Some(uri.to_string().into()),
        }
    }

    fn languages(ids: &[&str]) -> LanguageSelector {
        LanguageSelector::Languages(ids.iter().map(|id| id.to_string().into()).collect())
    }

    fn filter(
        language: Option<&str>,
        scheme: Option<&str>,
        pattern: Option<&str>,
    ) -> LanguageFilter {
        LanguageFilter {
            language: language.map(|s| s.to_string().into()),
            scheme: scheme.map(|s| s.to_string().into()),
            pattern: pattern.map(|s| s.to_string().into()),
            exclusive: false,
        }
    }

    fn filters(filters: Vec<LanguageFilter>) -> LanguageSelector {
        LanguageSelector::Filters(filters)
    }

    /// Monaco `languageSelector.test.ts`.
    #[test]
    fn selector_scores_like_monaco() {
        let model = target("farboo", "file:///testbed/file.fb");
        // score, invalid selector
        assert_eq!(filters(vec![filter(None, None, None)]).score(&model), 0);
        assert_eq!(languages(&[""]).score(&model), 0);
        // score, any language
        assert_eq!(
            filters(vec![filter(Some("*"), None, None)]).score(&model),
            5
        );
        assert_eq!(languages(&["*"]).score(&model), 5);
        assert_eq!(languages(&["*"]).score(&target("farboo", "foo:bar")), 5);
        assert_eq!(
            languages(&["farboo"]).score(&target("farboo", "foo:bar")),
            10
        );
        // score, default schemes
        let git = target("farboo", "git:foo/file.txt");
        assert_eq!(
            filters(vec![filter(Some("farboo"), Some(""), None)]).score(&git),
            10
        );
        assert_eq!(
            filters(vec![filter(Some("farboo"), Some("git"), None)]).score(&git),
            10
        );
        assert_eq!(
            filters(vec![filter(Some("farboo"), Some("*"), None)]).score(&git),
            10
        );
        assert_eq!(filters(vec![filter(Some("*"), None, None)]).score(&git), 5);
        assert_eq!(filters(vec![filter(None, Some("*"), None)]).score(&git), 5);
        assert_eq!(
            filters(vec![filter(None, Some("git"), None)]).score(&git),
            10
        );
        // score, filter
        assert_eq!(
            filters(vec![filter(Some("farboo"), Some("http"), None)]).score(&model),
            0
        );
        assert_eq!(
            filters(vec![filter(None, None, Some("**/*.fb"))]).score(&model),
            10
        );
        assert_eq!(
            filters(vec![filter(None, Some("file"), Some("**/*.fb"))]).score(&model),
            10
        );
        assert_eq!(
            filters(vec![filter(None, None, Some("**/*.fb"))]).score(&target("farboo", "foo:bar")),
            0
        );
        let js = target("javascript", "git:/my/file.js");
        assert_eq!(languages(&["javascript"]).score(&js), 10);
        assert_eq!(languages(&["fooLang"]).score(&js), 0);
        assert_eq!(languages(&["fooLang", "*"]).score(&js), 5);
        // score, max(filters)
        let matching = filter(Some("farboo"), Some("file"), None);
        let failing = filter(Some("farboo"), Some("http"), None);
        assert_eq!(
            filters(vec![matching.clone(), failing.clone()]).score(&model),
            10
        );
        assert_eq!(filters(vec![failing.clone(), failing]).score(&model), 0);
        assert_eq!(languages(&["farboo", "*"]).score(&model), 10);
        assert_eq!(languages(&["*", "farboo"]).score(&model), 10);
        // Document selector match - unexpected result value #60232
        assert_eq!(
            filters(vec![filter(
                Some("json"),
                Some("file"),
                Some("**/*.interface.json")
            )])
            .score(&target(
                "json",
                "file:///C:/Users/zlhe/Desktop/test.interface.json"
            )),
            10
        );
        // `All` is `*`, and `score` is Monaco's `languages.score`.
        assert_eq!(score(&LanguageSelector::All, &model), 5);
    }
}
