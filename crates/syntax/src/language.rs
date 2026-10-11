//! Languages, tree-sitter grammars with their queries, and the registry.

use std::path::Path;
use std::sync::{Arc, Mutex, OnceLock, RwLock};

use tree_sitter::Query;

use crate::config::LanguageConfiguration;
use crate::highlighter::Highlighter;

/// An interned highlight capture name such as `keyword` or `string.special`,
/// or a Monaco token name from a Monarch grammar such as `keyword.rust`.
///
/// Ids are process-wide, so spans from different languages share one
/// namespace and a theme resolves each name once.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct HighlightId(pub u32);

/// Interned names, with whether each is a Monaco token name.
type Names = Mutex<Vec<(Arc<str>, bool)>>;

fn interner() -> &'static Names {
    static NAMES: OnceLock<Names> = OnceLock::new();
    NAMES.get_or_init(Default::default)
}

impl HighlightId {
    fn intern_kind(name: &str, monaco_token: bool) -> Self {
        let mut names = interner().lock().unwrap_or_else(|e| e.into_inner());
        if let Some(ix) = names
            .iter()
            .position(|(existing, token)| &**existing == name && *token == monaco_token)
        {
            return HighlightId(ix as u32);
        }
        names.push((name.into(), monaco_token));
        HighlightId(names.len() as u32 - 1)
    }

    /// Interns the tree-sitter capture name `name`.
    pub fn intern(name: &str) -> Self {
        Self::intern_kind(name, false)
    }

    /// Interns `name` as a Monaco token name (e.g. `keyword.rust` from a
    /// Monarch grammar), which themes match directly against their Monaco
    /// rules rather than mapping it as a tree-sitter capture. Its id differs
    /// from the capture of the same name.
    pub fn intern_monaco_token(name: &str) -> Self {
        Self::intern_kind(name, true)
    }

    /// Whether this is a Monaco token name (see
    /// [`HighlightId::intern_monaco_token`]) rather than a capture.
    pub fn is_monaco_token(self) -> bool {
        let names = interner().lock().unwrap_or_else(|e| e.into_inner());
        names.get(self.0 as usize).is_some_and(|(_, token)| *token)
    }

    /// The capture or token name, e.g. `keyword.function`.
    pub fn name(self) -> Arc<str> {
        let names = interner().lock().unwrap_or_else(|e| e.into_inner());
        names
            .get(self.0 as usize)
            .map(|(name, _)| name.clone())
            .unwrap_or_else(|| "".into())
    }
}

/// A tree-sitter language that can move between threads.
///
/// tree-sitter doesn't implement `Send`/`Sync` for `Language` on wasm,
/// where it can also be a language loaded into a wasm store. Grammars here
/// are compiled-in C tables, which are immutable and safe to share, as on
/// every other target.
pub(crate) struct SharedLanguage(pub(crate) tree_sitter::Language);

#[cfg(target_family = "wasm")]
#[allow(unsafe_code)]
// SAFETY: compiled-in grammars are immutable static tables (see above).
unsafe impl Send for SharedLanguage {}
#[cfg(target_family = "wasm")]
#[allow(unsafe_code)]
// SAFETY: as above.
unsafe impl Sync for SharedLanguage {}

/// A tree-sitter grammar plus the queries gpui-monaco uses.
pub struct Grammar {
    pub(crate) ts_language: SharedLanguage,
    pub(crate) highlights: Option<Query>,
    /// Highlight id for each capture of `highlights`; `None` for captures that
    /// are not highlights (names starting with `_`).
    pub(crate) highlight_ids: Vec<Option<HighlightId>>,
    pub(crate) injections: Option<InjectionQuery>,
    pub(crate) brackets: Option<Query>,
    pub(crate) folds: Option<Query>,
    pub(crate) indents: Option<Query>,
    pub(crate) tags: Option<Query>,
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
            ts_language: SharedLanguage(ts_language),
            highlights: None,
            highlight_ids: Vec::new(),
            injections: None,
            brackets: None,
            folds: None,
            indents: None,
            tags: None,
        }
    }

    /// The tree-sitter language.
    pub fn ts_language(&self) -> &tree_sitter::Language {
        &self.ts_language.0
    }

    /// `highlights.scm`: captures are interned as [`HighlightId`]s, except names
    /// starting with `_`, which are helpers and produce no highlight.
    pub fn with_highlights_query(mut self, source: &str) -> Result<Self, tree_sitter::QueryError> {
        let query = Query::new(&self.ts_language.0, source)?;
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
        let query = Query::new(&self.ts_language.0, source)?;
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
        self.brackets = Some(Query::new(&self.ts_language.0, source)?);
        Ok(self)
    }

    /// `folds.scm`: foldable nodes captured as `@fold`.
    pub fn with_folds_query(mut self, source: &str) -> Result<Self, tree_sitter::QueryError> {
        self.folds = Some(Query::new(&self.ts_language.0, source)?);
        Ok(self)
    }

    /// `tags.scm` (tree-sitter's code navigation query): definitions
    /// captured as `@definition.<kind>` with their `@name`, used for
    /// document symbols.
    pub fn with_tags_query(mut self, source: &str) -> Result<Self, tree_sitter::QueryError> {
        self.tags = Some(Query::new(&self.ts_language.0, source)?);
        Ok(self)
    }

    /// `indents.scm`: `@indent` / `@outdent` style captures.
    pub fn with_indents_query(mut self, source: &str) -> Result<Self, tree_sitter::QueryError> {
        self.indents = Some(Query::new(&self.ts_language.0, source)?);
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

    /// The tags query, if set.
    pub fn tags_query(&self) -> Option<&Query> {
        self.tags.as_ref()
    }
}

/// A language: identity, file associations, editing configuration and an
/// optional grammar.
#[derive(Clone)]
pub struct Language {
    /// Stable identifier, like Monaco's language id (e.g. `typescriptreact`).
    pub id: Arc<str>,
    /// Display name.
    pub name: Arc<str>,
    /// Other names, matched case-insensitively (e.g. code fence info strings).
    pub aliases: Vec<String>,
    /// File extensions without the dot, matched case-insensitively.
    pub extensions: Vec<String>,
    /// Exact file names, matched case-insensitively as in Monaco (e.g.
    /// `Cargo.lock`).
    pub file_names: Vec<String>,
    /// Glob patterns for file names, or whole paths when they contain a
    /// `/` (Monaco `filenamePatterns`), e.g. `*.config.js`.
    pub filename_patterns: Vec<String>,
    /// A regex for the first line, used when no file name matches (Monaco
    /// `firstLine`), e.g. `^#!.*\bpython`.
    pub first_line: Option<String>,
    /// MIME types (Monaco `mimetypes`), e.g. `text/x-rust`.
    pub mimetypes: Vec<String>,
    /// Editing rules from `language-configuration.json`.
    pub config: LanguageConfiguration,
    /// The tree-sitter grammar; `None` means no syntax highlighting.
    pub grammar: Option<Arc<Grammar>>,
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
            filename_patterns: Vec::new(),
            first_line: None,
            mimetypes: Vec::new(),
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

    /// Sets [`Language::filename_patterns`].
    pub fn with_filename_patterns(mut self, patterns: &[&str]) -> Self {
        self.filename_patterns = patterns.iter().map(|p| p.to_string()).collect();
        self
    }

    /// Sets [`Language::first_line`].
    pub fn with_first_line(mut self, pattern: &str) -> Self {
        self.first_line = Some(pattern.to_string());
        self
    }

    /// Sets [`Language::mimetypes`].
    pub fn with_mimetypes(mut self, mimetypes: &[&str]) -> Self {
        self.mimetypes = mimetypes.iter().map(|m| m.to_string()).collect();
        self
    }

    /// Sets [`Language::config`].
    pub fn with_config(mut self, config: LanguageConfiguration) -> Self {
        self.config = config;
        self
    }

    /// Sets [`Language::grammar`].
    pub fn with_grammar(mut self, grammar: Grammar) -> Self {
        self.grammar = Some(Arc::new(grammar));
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

/// Appends the entries of `new` that `list` doesn't have yet.
fn merge_list(list: &mut Vec<String>, new: Vec<String>) {
    for entry in new {
        if !list.contains(&entry) {
            list.push(entry);
        }
    }
}

impl Language {
    /// Monaco `LanguagesRegistry._mergeLanguage`: `self` registered again
    /// as `new`. File associations, aliases and MIME types are added to
    /// the existing ones; the name, first-line pattern, grammar and
    /// highlighter are `new`'s when it has them; configuration fields `new`
    /// sets replace the existing ones.
    fn merged(&self, new: Language) -> Language {
        let mut merged = self.clone();
        // Monaco names a language after its first alias: a registration
        // without one keeps the name.
        if new.name != new.id || merged.name == merged.id {
            merged.name = new.name;
        }
        merge_list(&mut merged.aliases, new.aliases);
        merge_list(&mut merged.extensions, new.extensions);
        merge_list(&mut merged.file_names, new.file_names);
        merge_list(&mut merged.filename_patterns, new.filename_patterns);
        merge_list(&mut merged.mimetypes, new.mimetypes);
        merged.first_line = new.first_line.or(merged.first_line);
        merged.config = merged.config.combine(&new.config);
        merged.grammar = new.grammar.or(merged.grammar);
        merged.highlighter = new.highlighter.or(merged.highlighter);
        merged.hidden = merged.hidden && new.hidden;
        merged
    }
}

/// Identifies a configuration added with
/// [`LanguageRegistry::register_configuration`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ConfigurationRegistration(u64);

/// A language's configurations (Monaco `ComposedLanguageConfiguration`):
/// the one it was registered with, then the ones added at runtime, in
/// order.
#[derive(Default)]
struct ConfigurationStack {
    base: LanguageConfiguration,
    registrations: Vec<(ConfigurationRegistration, LanguageConfiguration)>,
}

impl ConfigurationStack {
    /// Monaco `combineLanguageConfigurations`: each field from the newest
    /// configuration that sets it.
    fn resolve(&self) -> LanguageConfiguration {
        self.registrations
            .iter()
            .fold(self.base.clone(), |config, (_, newer)| {
                config.combine(newer)
            })
    }
}

/// Languages met while tokenizing (see
/// [`LanguageRegistry::take_encountered_languages`]).
#[derive(Default)]
struct Encountered {
    seen: std::collections::HashSet<Arc<str>>,
    pending: Vec<Arc<str>>,
}

/// All registered languages. Cheap to share behind an `Arc`.
#[derive(Default)]
pub struct LanguageRegistry {
    languages: RwLock<Vec<Arc<Language>>>,
    /// Each language's configurations, by id.
    configurations: RwLock<std::collections::HashMap<Arc<str>, ConfigurationStack>>,
    next_configuration: std::sync::atomic::AtomicU64,
    /// Highlighters set before their language was registered (Monaco's
    /// tokenization registry doesn't need the language).
    pending_highlighters: RwLock<std::collections::HashMap<Arc<str>, HighlighterFactory>>,
    encountered: Mutex<Encountered>,
}

impl LanguageRegistry {
    /// An empty registry.
    pub fn new() -> Self {
        Self::default()
    }

    /// Registers `language` (Monaco `languages.register`). Registering an
    /// id again merges into the existing language, as Monaco's
    /// `LanguagesRegistry` does: its file associations, aliases and MIME
    /// types are added, its grammar, highlighter and name replace the
    /// existing ones only when it has them, and configuration fields it
    /// sets replace those it was registered with (configurations added
    /// with [`register_configuration`](Self::register_configuration) still
    /// apply on top). A highlighter set with
    /// [`set_highlighter`](Self::set_highlighter) before the language was
    /// registered applies now.
    pub fn register(&self, language: Language) -> Arc<Language> {
        let mut languages = self.languages.write().unwrap_or_else(|e| e.into_inner());
        let mut configurations = self
            .configurations
            .write()
            .unwrap_or_else(|e| e.into_inner());
        let mut language = match languages.iter().position(|l| l.id == language.id) {
            Some(ix) => {
                let existing = languages.remove(ix);
                let stack = configurations.entry(existing.id.clone()).or_default();
                stack.base = stack.base.combine(&language.config);
                existing.merged(language)
            }
            None => {
                configurations.insert(
                    language.id.clone(),
                    ConfigurationStack {
                        base: language.config.clone(),
                        registrations: Vec::new(),
                    },
                );
                language
            }
        };
        if let Some(stack) = configurations.get(&language.id) {
            language.config = stack.resolve();
        }
        if let Some(highlighter) = self
            .pending_highlighters
            .write()
            .unwrap_or_else(|e| e.into_inner())
            .remove(&language.id)
        {
            language.highlighter = Some(highlighter);
        }
        let language = Arc::new(language);
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

    /// Finds a visible language by file name, file name pattern or
    /// extension (Monaco's `getAssociationByPath`): an exact file name wins,
    /// then the longest matching pattern, then the longest matching
    /// extension; among equals, the language registered last. Matching
    /// ignores case.
    pub fn language_for_path(&self, path: &Path) -> Option<Arc<Language>> {
        let path = path.to_str()?.replace('\\', "/").to_lowercase();
        let file_name = path.rsplit('/').next().unwrap_or(&path).to_string();
        let languages = self.languages.read().unwrap_or_else(|e| e.into_inner());
        let mut pattern_match: Option<(usize, &Arc<Language>)> = None;
        let mut extension_match: Option<(usize, &Arc<Language>)> = None;
        for language in languages.iter().rev().filter(|l| !l.hidden) {
            if language
                .file_names
                .iter()
                .any(|name| name.to_lowercase() == file_name)
            {
                return Some(language.clone());
            }
            for pattern in &language.filename_patterns {
                if pattern_match.is_none_or(|(len, _)| pattern.len() > len) {
                    let target = if pattern.contains('/') {
                        &path
                    } else {
                        &file_name
                    };
                    if glob_matches(&pattern.to_lowercase(), target) {
                        pattern_match = Some((pattern.len(), language));
                    }
                }
            }
            for extension in &language.extensions {
                let extension = format!(".{}", extension.trim_start_matches('.').to_lowercase());
                if extension_match.is_none_or(|(len, _)| extension.len() > len)
                    && file_name.ends_with(&extension)
                {
                    extension_match = Some((extension.len(), language));
                }
            }
        }
        pattern_match
            .or(extension_match)
            .map(|(_, language)| language.clone())
    }

    /// Monaco `guessLanguageIdByFilepathOrFirstLine`: by `path` (see
    /// [`LanguageRegistry::language_for_path`]), else by the first line's
    /// [`Language::first_line`] regex (the language registered last wins).
    pub fn language_for_path_or_first_line(
        &self,
        path: Option<&Path>,
        first_line: Option<&str>,
    ) -> Option<Arc<Language>> {
        if let Some(language) = path.and_then(|path| self.language_for_path(path)) {
            return Some(language);
        }
        let first_line = first_line?;
        let first_line = first_line.strip_prefix('\u{feff}').unwrap_or(first_line);
        if first_line.is_empty() {
            return None;
        }
        let languages = self.languages.read().unwrap_or_else(|e| e.into_inner());
        languages
            .iter()
            .rev()
            .filter(|l| !l.hidden)
            .find(|language| {
                language.first_line.as_ref().is_some_and(|pattern| {
                    fancy_regex::Regex::new(pattern)
                        .is_ok_and(|regex| regex.is_match(first_line).unwrap_or(false))
                })
            })
            .cloned()
    }

    /// Replaces the [`Language::highlighter`] of the language `id` (e.g. a
    /// [`MonarchHighlighter`](crate::MonarchHighlighter) for Monaco
    /// `setMonarchTokensProvider`), `None` going back to its grammar.
    /// Returns whether it's registered; if it isn't, the highlighter is
    /// kept for when it is (as Monaco's tokenization registry doesn't need
    /// the language). Models pick it up when they next resolve their
    /// language.
    pub fn set_highlighter(&self, id: &str, highlighter: Option<HighlighterFactory>) -> bool {
        let mut languages = self.languages.write().unwrap_or_else(|e| e.into_inner());
        let Some(slot) = languages.iter_mut().find(|language| &*language.id == id) else {
            let mut pending = self
                .pending_highlighters
                .write()
                .unwrap_or_else(|e| e.into_inner());
            match highlighter {
                Some(highlighter) => pending.insert(id.into(), highlighter),
                None => pending.remove(id),
            };
            return false;
        };
        let mut language = (**slot).clone();
        language.highlighter = highlighter;
        *slot = Arc::new(language);
        true
    }

    /// The language with this MIME type (Monaco `getLanguageIdByMimeType`).
    pub fn language_for_mime_type(&self, mime_type: &str) -> Option<Arc<Language>> {
        let languages = self.languages.read().unwrap_or_else(|e| e.into_inner());
        languages
            .iter()
            .find(|language| language.mimetypes.iter().any(|m| m == mime_type))
            .cloned()
    }

    /// The highlighter set with [`set_highlighter`](Self::set_highlighter)
    /// for `id` before it was registered, if any.
    pub fn pending_highlighter(&self, id: &str) -> Option<HighlighterFactory> {
        self.pending_highlighters
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .get(id)
            .cloned()
    }

    /// Layers `config` over the configuration of the language `id` (Monaco
    /// `LanguageConfigurationRegistry.register`, as
    /// `setLanguageConfiguration` uses it): each field it sets replaces
    /// the current one, and fields it leaves `None` keep theirs. Later
    /// registrations win. Returns `None` if `id` isn't registered. Models
    /// pick it up when they next resolve their language.
    pub fn register_configuration(
        &self,
        id: &str,
        config: LanguageConfiguration,
    ) -> Option<ConfigurationRegistration> {
        use std::sync::atomic::Ordering;
        let registration =
            ConfigurationRegistration(self.next_configuration.fetch_add(1, Ordering::Relaxed) + 1);
        self.update_configuration(id, |stack| stack.registrations.push((registration, config)))
            .then_some(registration)
    }

    /// Removes a configuration added with
    /// [`register_configuration`](Self::register_configuration), restoring
    /// what it replaced unless a later one replaces it too. Returns whether
    /// it was there.
    pub fn unregister_configuration(
        &self,
        id: &str,
        registration: ConfigurationRegistration,
    ) -> bool {
        let mut found = false;
        self.update_configuration(id, |stack| {
            let before = stack.registrations.len();
            stack.registrations.retain(|(r, _)| *r != registration);
            found = stack.registrations.len() != before;
        });
        found
    }

    fn update_configuration(&self, id: &str, update: impl FnOnce(&mut ConfigurationStack)) -> bool {
        let mut languages = self.languages.write().unwrap_or_else(|e| e.into_inner());
        let Some(slot) = languages.iter_mut().find(|language| &*language.id == id) else {
            return false;
        };
        let mut configurations = self
            .configurations
            .write()
            .unwrap_or_else(|e| e.into_inner());
        let stack = configurations
            .entry(slot.id.clone())
            .or_insert_with(|| ConfigurationStack {
                base: slot.config.clone(),
                registrations: Vec::new(),
            });
        update(stack);
        let mut language = (**slot).clone();
        language.config = stack.resolve();
        *slot = Arc::new(language);
        true
    }

    /// Notes that the language `id` was met while tokenizing another, e.g.
    /// by a Monarch grammar's `nextEmbedded` (Monaco
    /// `requestBasicLanguageFeatures`). Each id is noted once.
    pub fn note_language_encountered(&self, id: &str) {
        let mut encountered = self.encountered.lock().unwrap_or_else(|e| e.into_inner());
        if !encountered.seen.contains(id) {
            let id: Arc<str> = id.into();
            encountered.seen.insert(id.clone());
            encountered.pending.push(id);
        }
    }

    /// The languages [noted](Self::note_language_encountered) since the
    /// last call.
    pub fn take_encountered_languages(&self) -> Vec<Arc<str>> {
        std::mem::take(
            &mut self
                .encountered
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .pending,
        )
    }
}

/// Monaco's glob matching for file associations: `*` and `?` within a path
/// segment, `**` across segments, `{a,b}` alternatives and `[...]` classes.
pub fn glob_matches(pattern: &str, target: &str) -> bool {
    glob_regex(pattern).is_some_and(|regex| regex.is_match(target))
}

fn glob_regex(pattern: &str) -> Option<regex::Regex> {
    let mut out = String::from("^");
    let chars: Vec<char> = pattern.chars().collect();
    let mut i = 0;
    let mut in_braces = false;
    while i < chars.len() {
        let c = chars[i];
        match c {
            '*' if chars.get(i + 1) == Some(&'*') => {
                // `**/` matches any number of segments, including none.
                if chars.get(i + 2) == Some(&'/') {
                    out.push_str("(?:[^/]*/)*");
                    i += 3;
                } else {
                    out.push_str(".*");
                    i += 2;
                }
                continue;
            }
            '*' => out.push_str("[^/]*"),
            '?' => out.push_str("[^/]"),
            '{' => {
                in_braces = true;
                out.push_str("(?:");
            }
            '}' if in_braces => {
                in_braces = false;
                out.push(')');
            }
            ',' if in_braces => out.push('|'),
            '[' => {
                let end = chars[i..].iter().position(|&c| c == ']')? + i;
                let class: String = chars[i + 1..end].iter().collect();
                let class = class
                    .strip_prefix('!')
                    .map_or(class.clone(), |rest| format!("^{rest}"));
                out.push('[');
                out.push_str(&class.replace('\\', "\\\\"));
                out.push(']');
                i = end + 1;
                continue;
            }
            _ => out.push_str(&regex::escape(&c.to_string())),
        }
        i += 1;
    }
    out.push('$');
    regex::Regex::new(&out).ok()
}
