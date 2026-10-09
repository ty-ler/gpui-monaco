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
    pub fn language(id: impl Into<SharedString>) -> Self {
        Self::Languages(vec![id.into()])
    }

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
    pub language_id: Option<SharedString>,
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
    Method,
    Function,
    Constructor,
    Field,
    Variable,
    Class,
    Struct,
    Interface,
    Module,
    Property,
    Event,
    Operator,
    Unit,
    Value,
    Constant,
    Enum,
    EnumMember,
    Keyword,
    #[default]
    Text,
    Color,
    File,
    Reference,
    Folder,
    TypeParameter,
    Snippet,
}

/// Monaco `CompletionItem`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct CompletionItem {
    pub label: SharedString,
    pub kind: CompletionItemKind,
    /// Shown next to the label (e.g. a type signature).
    pub detail: Option<SharedString>,
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
    pub preselect: bool,
    /// Typing one of these accepts the item, then inserts the character.
    pub commit_characters: Vec<char>,
    /// Edits applied elsewhere (e.g. imports) when accepted.
    pub additional_text_edits: Vec<(Range<PointUtf16>, String)>,
}

impl CompletionItem {
    pub fn new(label: impl Into<SharedString>, kind: CompletionItemKind) -> Self {
        Self {
            label: label.into(),
            kind,
            ..Default::default()
        }
    }

    pub fn filter_text(&self) -> &str {
        self.filter_text.as_deref().unwrap_or(&self.label)
    }

    pub fn sort_text(&self) -> &str {
        self.sort_text.as_deref().unwrap_or(&self.label)
    }

    pub fn insert_text(&self) -> &str {
        self.insert_text.as_deref().unwrap_or(&self.label)
    }
}

/// Monaco `CompletionList`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct CompletionList {
    pub items: Vec<CompletionItem>,
    /// More typing should re-query instead of filtering.
    pub incomplete: bool,
}

/// Monaco `CompletionTriggerKind`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CompletionTrigger {
    /// Typing a word character or Ctrl+Space.
    Invoke,
    TriggerCharacter(char),
    /// Re-query of an incomplete list.
    Incomplete,
}

pub trait CompletionProvider: 'static {
    /// Characters that open the suggest widget when typed (e.g. `.`).
    fn trigger_characters(&self) -> Vec<char> {
        Vec::new()
    }

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
    pub contents: Vec<MarkdownString>,
    /// The hovered range, used to keep the hover open while the mouse stays
    /// in it; defaults to the word under the mouse.
    pub range: Option<Range<PointUtf16>>,
}

pub trait HoverProvider: 'static {
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
    pub uri: Option<SharedString>,
    pub range: Range<PointUtf16>,
}

pub trait DefinitionProvider: 'static {
    fn provide_definition(
        &self,
        context: &ProviderContext,
        position: PointUtf16,
        cx: &mut App,
    ) -> Task<anyhow::Result<Vec<Location>>>;
}

// ----- signature help ------------------------------------------------------------

/// Monaco `ParameterInformation`; `label` is a substring of the signature
/// label or a UTF-16 range into it.
#[derive(Clone, Debug, PartialEq)]
pub struct ParameterInformation {
    pub label: ParameterLabel,
    pub documentation: Option<MarkdownString>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum ParameterLabel {
    Text(SharedString),
    Range(Range<usize>),
}

#[derive(Clone, Debug, PartialEq)]
pub struct SignatureInformation {
    pub label: SharedString,
    pub documentation: Option<MarkdownString>,
    pub parameters: Vec<ParameterInformation>,
    /// Overrides `SignatureHelp::active_parameter` for this signature.
    pub active_parameter: Option<usize>,
}

/// Monaco `SignatureHelp`.
#[derive(Clone, Debug, PartialEq)]
pub struct SignatureHelp {
    pub signatures: Vec<SignatureInformation>,
    pub active_signature: usize,
    pub active_parameter: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SignatureHelpTrigger {
    Invoke,
    TriggerCharacter(char),
    /// Typing or moving while signature help is shown.
    ContentChange,
}

pub trait SignatureHelpProvider: 'static {
    /// Characters that open signature help (e.g. `(`, `,`).
    fn trigger_characters(&self) -> Vec<char> {
        vec!['(', ',']
    }

    /// Characters that re-query while it's open (e.g. `)`).
    fn retrigger_characters(&self) -> Vec<char> {
        Vec::new()
    }

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
}

impl Global for LanguageFeatures {}

fn features(cx: &mut App) -> &mut LanguageFeatures {
    cx.default_global::<LanguageFeatures>()
}

macro_rules! register_fn {
    ($name:ident, $field:ident, $trait:ident) => {
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

register_fn!(register_completion_provider, completion, CompletionProvider);
register_fn!(register_hover_provider, hover, HoverProvider);
register_fn!(register_definition_provider, definition, DefinitionProvider);
register_fn!(
    register_signature_help_provider,
    signature_help,
    SignatureHelpProvider
);

/// Removes a provider registered with any `register_*_provider`.
pub fn unregister_provider(cx: &mut App, registration: ProviderRegistration) {
    let features = features(cx);
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

pub fn completion_providers(
    cx: &App,
    language_id: Option<&str>,
) -> Vec<Arc<dyn CompletionProvider>> {
    cx.try_global::<LanguageFeatures>()
        .map(|f| matching(&f.completion, language_id))
        .unwrap_or_default()
}

pub fn hover_providers(cx: &App, language_id: Option<&str>) -> Vec<Arc<dyn HoverProvider>> {
    cx.try_global::<LanguageFeatures>()
        .map(|f| matching(&f.hover, language_id))
        .unwrap_or_default()
}

pub fn definition_providers(
    cx: &App,
    language_id: Option<&str>,
) -> Vec<Arc<dyn DefinitionProvider>> {
    cx.try_global::<LanguageFeatures>()
        .map(|f| matching(&f.definition, language_id))
        .unwrap_or_default()
}

pub fn signature_help_providers(
    cx: &App,
    language_id: Option<&str>,
) -> Vec<Arc<dyn SignatureHelpProvider>> {
    cx.try_global::<LanguageFeatures>()
        .map(|f| matching(&f.signature_help, language_id))
        .unwrap_or_default()
}
