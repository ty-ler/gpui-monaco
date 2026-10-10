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
    /// Inserted on accept; defaults to the label. (Snippet syntax isn't
    /// expanded yet.)
    pub insert_text: Option<SharedString>,
    /// The single-line range to replace; defaults to the word before the
    /// cursor.
    pub range: Option<Range<PointUtf16>>,
    /// Prefer selecting this item when it ties for the best filter score.
    pub preselect: bool,
    /// Typing one of these accepts the item, then inserts the character.
    pub commit_characters: Vec<char>,
    /// Edits applied elsewhere (e.g. imports) when accepted.
    pub additional_text_edits: Vec<(Range<PointUtf16>, String)>,
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

/// Removes a provider registered with any `register_*_provider`.
pub fn unregister_provider(cx: &mut App, registration: ProviderRegistration) {
    let features = features(cx);
    features.document_highlight.retain(|e| e.id != registration);
    features.completion.retain(|e| e.id != registration);
    features.hover.retain(|e| e.id != registration);
    features.definition.retain(|e| e.id != registration);
    features.signature_help.retain(|e| e.id != registration);
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
