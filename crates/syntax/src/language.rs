//! Languages, tree-sitter grammars with their queries, and the registry.

use std::path::Path;
use std::sync::{Arc, Mutex, OnceLock, RwLock};

use tree_sitter::Query;

use crate::config::LanguageConfiguration;
use crate::highlighter::Highlighter;

/// An interned highlight capture name such as `keyword` or `string.special`.
///
/// Ids are process-wide, so spans from different languages share one
/// namespace and a theme resolves each name once.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct HighlightId(pub u32);

fn interner() -> &'static Mutex<Vec<Arc<str>>> {
    static NAMES: OnceLock<Mutex<Vec<Arc<str>>>> = OnceLock::new();
    NAMES.get_or_init(Default::default)
}

impl HighlightId {
    /// Interns `name`.
    pub fn intern(name: &str) -> Self {
        let mut names = interner().lock().unwrap_or_else(|e| e.into_inner());
        if let Some(ix) = names.iter().position(|existing| &**existing == name) {
            return HighlightId(ix as u32);
        }
        names.push(name.into());
        HighlightId(names.len() as u32 - 1)
    }

    /// The capture name, e.g. `keyword.function`.
    pub fn name(self) -> Arc<str> {
        let names = interner().lock().unwrap_or_else(|e| e.into_inner());
        names
            .get(self.0 as usize)
            .cloned()
            .unwrap_or_else(|| "".into())
    }
}

/// A tree-sitter grammar plus the queries gpui-monaco uses.
pub struct Grammar {
    pub(crate) ts_language: tree_sitter::Language,
    pub(crate) highlights: Option<Query>,
    /// Highlight id for each capture of `highlights`; `None` for captures that
    /// are not highlights (names starting with `_`).
    pub(crate) highlight_ids: Vec<Option<HighlightId>>,
    pub(crate) injections: Option<InjectionQuery>,
    pub(crate) brackets: Option<Query>,
    pub(crate) folds: Option<Query>,
    pub(crate) indents: Option<Query>,
}

pub(crate) struct InjectionQuery {
    pub query: Query,
    pub content_capture: Option<u32>,
    pub language_capture: Option<u32>,
    /// Whether any pattern sets `injection.combined` (such injections can't
    /// be updated piecewise).
    pub has_combined: bool,
}

impl Grammar {
    /// A grammar with no queries; add them with the `with_*_query` methods.
    pub fn new(ts_language: tree_sitter::Language) -> Self {
        Self {
            ts_language,
            highlights: None,
            highlight_ids: Vec::new(),
            injections: None,
            brackets: None,
            folds: None,
            indents: None,
        }
    }

    /// The tree-sitter language.
    pub fn ts_language(&self) -> &tree_sitter::Language {
        &self.ts_language
    }

    /// `highlights.scm`: captures are interned as [`HighlightId`]s, except names
    /// starting with `_`, which are helpers and produce no highlight.
    pub fn with_highlights_query(mut self, source: &str) -> Result<Self, tree_sitter::QueryError> {
        let query = Query::new(&self.ts_language, source)?;
        self.highlight_ids = query
            .capture_names()
            .iter()
            .map(|name| (!name.starts_with('_')).then(|| HighlightId::intern(name)))
            .collect();
        self.highlights = Some(query);
        Ok(self)
    }

    /// `injections.scm`: `@injection.content` captures are parsed as the language
    /// named by `@injection.language` or the `injection.language` property.
    pub fn with_injections_query(mut self, source: &str) -> Result<Self, tree_sitter::QueryError> {
        let query = Query::new(&self.ts_language, source)?;
        let content_capture = query.capture_index_for_name("injection.content");
        let language_capture = query.capture_index_for_name("injection.language");
        let has_combined = (0..query.pattern_count()).any(|ix| {
            query
                .property_settings(ix)
                .iter()
                .any(|p| &*p.key == "injection.combined")
        });
        self.injections = Some(InjectionQuery {
            query,
            content_capture,
            language_capture,
            has_combined,
        });
        Ok(self)
    }

    /// `brackets.scm`: pairs captured as `@open` and `@close`.
    pub fn with_brackets_query(mut self, source: &str) -> Result<Self, tree_sitter::QueryError> {
        self.brackets = Some(Query::new(&self.ts_language, source)?);
        Ok(self)
    }

    /// `folds.scm`: foldable nodes captured as `@fold`.
    pub fn with_folds_query(mut self, source: &str) -> Result<Self, tree_sitter::QueryError> {
        self.folds = Some(Query::new(&self.ts_language, source)?);
        Ok(self)
    }

    /// `indents.scm`: `@indent` / `@outdent` style captures.
    pub fn with_indents_query(mut self, source: &str) -> Result<Self, tree_sitter::QueryError> {
        self.indents = Some(Query::new(&self.ts_language, source)?);
        Ok(self)
    }

    /// The highlights query, if set.
    pub fn highlights_query(&self) -> Option<&Query> {
        self.highlights.as_ref()
    }

    /// The brackets query, if set.
    pub fn brackets_query(&self) -> Option<&Query> {
        self.brackets.as_ref()
    }

    /// The folds query, if set.
    pub fn folds_query(&self) -> Option<&Query> {
        self.folds.as_ref()
    }

    /// The indents query, if set.
    pub fn indents_query(&self) -> Option<&Query> {
        self.indents.as_ref()
    }
}

/// A language: identity, file associations, editing configuration and an
/// optional grammar.
pub struct Language {
    /// Stable identifier, like Monaco's language id (e.g. `typescriptreact`).
    pub id: Arc<str>,
    /// Display name.
    pub name: Arc<str>,
    /// Other names, matched case-insensitively (e.g. code fence info strings).
    pub aliases: Vec<String>,
    /// File extensions without the dot, matched case-insensitively.
    pub extensions: Vec<String>,
    /// Exact file names, matched case-sensitively (e.g. `Cargo.lock`).
    pub file_names: Vec<String>,
    /// Editing rules from `language-configuration.json`.
    pub config: LanguageConfiguration,
    /// The tree-sitter grammar; `None` means no syntax highlighting.
    pub grammar: Option<Grammar>,
    /// Hidden languages (e.g. `markdown_inline`) are only used as injections.
    pub hidden: bool,
    /// Builds a custom [`Highlighter`] for models of this language instead of
    /// the tree-sitter one; see [`Language::with_highlighter`].
    pub highlighter: Option<HighlighterFactory>,
}

/// Creates a [`Highlighter`] for a language (with the registry, for
/// injections).
pub type HighlighterFactory =
    Arc<dyn Fn(&Arc<Language>, &Arc<LanguageRegistry>) -> Arc<dyn Highlighter> + Send + Sync>;

impl Language {
    /// A language with no file associations, default configuration and no grammar.
    pub fn new(id: impl Into<Arc<str>>, name: impl Into<Arc<str>>) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
            aliases: Vec::new(),
            extensions: Vec::new(),
            file_names: Vec::new(),
            config: LanguageConfiguration::default(),
            grammar: None,
            hidden: false,
            highlighter: None,
        }
    }

    /// Sets [`Language::aliases`].
    pub fn with_aliases(mut self, aliases: &[&str]) -> Self {
        self.aliases = aliases.iter().map(|a| a.to_string()).collect();
        self
    }

    /// Sets [`Language::extensions`].
    pub fn with_extensions(mut self, extensions: &[&str]) -> Self {
        self.extensions = extensions.iter().map(|e| e.to_string()).collect();
        self
    }

    /// Sets [`Language::file_names`].
    pub fn with_file_names(mut self, names: &[&str]) -> Self {
        self.file_names = names.iter().map(|e| e.to_string()).collect();
        self
    }

    /// Sets [`Language::config`].
    pub fn with_config(mut self, config: LanguageConfiguration) -> Self {
        self.config = config;
        self
    }

    /// Sets [`Language::grammar`].
    pub fn with_grammar(mut self, grammar: Grammar) -> Self {
        self.grammar = Some(grammar);
        self
    }

    /// Highlights this language's models with a custom [`Highlighter`] (e.g.
    /// TextMate grammars or LSP semantic tokens) instead of
    /// [`TreeSitterHighlighter`](crate::TreeSitterHighlighter). Its
    /// [`update`](Highlighter::update) runs on a background thread after
    /// edits, like tree-sitter parsing.
    pub fn with_highlighter(
        mut self,
        factory: impl Fn(&Arc<Language>, &Arc<LanguageRegistry>) -> Arc<dyn Highlighter>
        + Send
        + Sync
        + 'static,
    ) -> Self {
        self.highlighter = Some(Arc::new(factory));
        self
    }

    /// Whether models of this language get syntax highlighting.
    pub fn has_highlighter(&self) -> bool {
        self.grammar.is_some() || self.highlighter.is_some()
    }

    /// Marks the language [`hidden`](Language::hidden).
    pub fn hidden(mut self) -> Self {
        self.hidden = true;
        self
    }

    fn matches_name(&self, name: &str) -> bool {
        self.id.eq_ignore_ascii_case(name)
            || self.name.eq_ignore_ascii_case(name)
            || self.aliases.iter().any(|a| a.eq_ignore_ascii_case(name))
            || self.extensions.iter().any(|e| e.eq_ignore_ascii_case(name))
    }
}

impl std::fmt::Debug for Language {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Language").field("id", &self.id).finish()
    }
}

/// All registered languages. Cheap to share behind an `Arc`.
#[derive(Default)]
pub struct LanguageRegistry {
    languages: RwLock<Vec<Arc<Language>>>,
}

impl LanguageRegistry {
    /// An empty registry.
    pub fn new() -> Self {
        Self::default()
    }

    /// Registers `language`, replacing any language with the same id.
    pub fn register(&self, language: Language) -> Arc<Language> {
        let language = Arc::new(language);
        let mut languages = self.languages.write().unwrap_or_else(|e| e.into_inner());
        languages.retain(|existing| existing.id != language.id);
        languages.push(language.clone());
        language
    }

    /// Every registered language, in registration order.
    pub fn languages(&self) -> Vec<Arc<Language>> {
        self.languages
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    /// The language with exactly this id.
    pub fn language_for_id(&self, id: &str) -> Option<Arc<Language>> {
        self.languages
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .iter()
            .find(|language| &*language.id == id)
            .cloned()
    }

    /// Finds a language by id, name, alias or extension, case-insensitively.
    /// Used for injections such as Markdown code fence info strings.
    pub fn language_for_name(&self, name: &str) -> Option<Arc<Language>> {
        let name = name.trim();
        let languages = self.languages.read().unwrap_or_else(|e| e.into_inner());
        languages
            .iter()
            .find(|language| &*language.id == name)
            .or_else(|| {
                languages
                    .iter()
                    .find(|language| language.matches_name(name))
            })
            .cloned()
    }

    /// Finds a visible language by file name or extension.
    pub fn language_for_path(&self, path: &Path) -> Option<Arc<Language>> {
        let file_name = path.file_name()?.to_str()?;
        let extension = path.extension().and_then(|e| e.to_str());
        let languages = self.languages.read().unwrap_or_else(|e| e.into_inner());
        languages
            .iter()
            .filter(|language| !language.hidden)
            .find(|language| {
                language.file_names.iter().any(|n| n == file_name)
                    || extension.is_some_and(|ext| {
                        language
                            .extensions
                            .iter()
                            .any(|e| e.eq_ignore_ascii_case(ext))
                    })
            })
            .cloned()
    }
}
