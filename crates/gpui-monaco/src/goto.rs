//! Symbol navigation: go to definition, declaration, type definition,
//! implementations and references, with several results shown in a peek
//! view or stepped through with F12, and the `editor.action.goToLocations`
//! and `editor.action.peekLocations` API commands for given locations.
//!
//! Ports Monaco's `src/vs/editor/contrib/gotoSymbol/browser/goToCommands.ts`,
//! `symbolNavigation.ts` and `referencesModel.ts` (MIT).

use std::time::Duration;

use anyhow::Context as _;
use gpui::{App, Context, Entity, Global, SharedString, Task};
use text::PointUtf16;

use crate::actions::{CommandLocation, CommandPosition, GoToLocations, PeekLocations};
use crate::cursor::Selection;
use crate::decorations::DecorationOptions;
use crate::editor::CodeEditor;
use crate::options::{GoToLocation, LocationKind, NavigationCommand};
use crate::providers::{
    Location, ReferenceContext, declaration_providers, definition_providers,
    implementation_providers, reference_providers, type_definition_providers,
};
use crate::view::reveal::{RevealType, ScrollType};

/// How a navigation command shows its results (Monaco
/// `SymbolNavigationActionConfig`, plus whether the alternative command may
/// run).
#[derive(Clone, Copy, Debug)]
pub(crate) struct Navigation {
    /// Always show the results in a peek view.
    pub open_in_peek: bool,
    /// Open the result in an editor beside this one.
    pub open_to_side: bool,
    /// No message when nothing is found.
    pub mute_message: bool,
    /// Run `gotoLocation`'s alternative command when the only result is
    /// at the cursor.
    pub allow_alternative: bool,
    /// Monaco `SymbolNavigationAction._activeAlternativeCommands`: the
    /// commands, as bits by [`NavigationCommand`] discriminant, whose
    /// alternative led here; none of them runs as an alternative again.
    pub running_alternatives: u16,
}

impl Navigation {
    /// The registered command this navigation of `kind` is.
    fn command(&self, kind: LocationKind) -> NavigationCommand {
        use NavigationCommand::*;
        match (kind, self.open_in_peek) {
            (LocationKind::Definition, _) if self.open_to_side => RevealDefinitionAside,
            (LocationKind::Definition, false) => RevealDefinition,
            (LocationKind::Definition, true) => PeekDefinition,
            (LocationKind::Declaration, false) => RevealDeclaration,
            (LocationKind::Declaration, true) => PeekDeclaration,
            (LocationKind::TypeDefinition, false) => GoToTypeDefinition,
            (LocationKind::TypeDefinition, true) => PeekTypeDefinition,
            (LocationKind::Implementation, false) => GoToImplementation,
            (LocationKind::Implementation, true) => PeekImplementation,
            (LocationKind::References, false) => GoToReferences,
            (LocationKind::References, true) => PeekReferences,
        }
    }

    /// Running `command` with its own configuration (Monaco runs the
    /// registered action) as the alternative of this navigation of `kind`.
    fn alternative(&self, kind: LocationKind, command: NavigationCommand) -> Self {
        Self {
            open_in_peek: command.open_in_peek(),
            open_to_side: command.open_to_side(),
            // No registered navigation command mutes its message.
            mute_message: false,
            allow_alternative: true,
            running_alternatives: self.running_alternatives | 1 << self.command(kind) as u16,
        }
    }

    /// Whether `command` may run as an alternative from here.
    fn may_run(&self, command: NavigationCommand) -> bool {
        self.allow_alternative && self.running_alternatives & (1 << command as u16) == 0
    }
}

impl Default for Navigation {
    fn default() -> Self {
        Self {
            open_in_peek: false,
            open_to_side: false,
            mute_message: false,
            allow_alternative: true,
            running_alternatives: 0,
        }
    }
}

/// Monaco highlights the target of a jump this long.
const HIGHLIGHT_DURATION: Duration = Duration::from_millis(350);

#[derive(Default)]
pub(crate) struct GotoState {
    pub(crate) task: Option<Task<()>>,
    highlight: Option<Task<()>>,
}

/// Results of a navigation command: every location has its model's uri
/// (`None` only for a queried model without one), sorted by uri and
/// position, without duplicates (Monaco `ReferencesModel`).
pub(crate) fn sorted_locations(
    locations: Vec<Location>,
    queried: Option<&SharedString>,
) -> Vec<Location> {
    let mut locations: Vec<Location> = locations
        .into_iter()
        .map(|location| Location {
            uri: location.uri.clone().or_else(|| queried.cloned()),
            ..location
        })
        .collect();
    locations.sort_by(|a, b| {
        a.uri
            .cmp(&b.uri)
            .then(a.range.start.cmp(&b.range.start))
            .then(a.range.end.cmp(&b.range.end))
    });
    locations.dedup_by(|a, b| a.uri == b.uri && a.range == b.range);
    locations
}

/// Monaco `ReferencesModel.ariaMessage`, which navigation commands alert:
/// how many `locations` (sorted, see [`sorted_locations`]) in how many
/// files, named by path when there's one.
fn aria_message(locations: &[Location]) -> String {
    let path = |location: &Location| {
        location.uri.as_deref().map_or_else(String::new, |uri| {
            crate::languages::uri_path(uri).unwrap_or_else(|| uri.to_string())
        })
    };
    let mut files = locations.iter().map(|l| &l.uri).collect::<Vec<_>>();
    files.dedup();
    match (locations, files.len()) {
        ([], _) => "No results found".to_string(),
        ([location], _) => format!("Found 1 symbol in {}", path(location)),
        ([first, ..], 1) => format!("Found {} symbols in {}", locations.len(), path(first)),
        _ => format!("Found {} symbols in {} files", locations.len(), files.len()),
    }
}

/// Monaco's message for a navigation command without results.
fn no_result_message(kind: LocationKind, word: Option<&str>) -> String {
    let what = match kind {
        LocationKind::Definition => "definition",
        LocationKind::Declaration => "declaration",
        LocationKind::TypeDefinition => "type definition",
        LocationKind::Implementation => "implementation",
        LocationKind::References => "references",
    };
    match word {
        Some(word) => format!("No {what} found for '{word}'"),
        None => format!("No {what} found"),
    }
}

/// The results F12 steps through after a `Goto` navigation (Monaco
/// `SymbolNavigationService`). Shared by all editors, like Monaco's.
#[derive(Default)]
struct SymbolNavigation {
    locations: Vec<Location>,
    index: usize,
}

impl Global for SymbolNavigation {}

/// Whether F12 has results to step through.
pub fn has_symbol_navigation(cx: &App) -> bool {
    cx.try_global::<SymbolNavigation>()
        .is_some_and(|nav| !nav.locations.is_empty())
}

impl CodeEditor {
    /// Asks `model`'s providers of `kind` about `position` in it. With
    /// `compact` (Monaco's Go to References), a reference provider that
    /// finds exactly two references is asked again without the declaration,
    /// and that answer is used if it has exactly one (Monaco
    /// `getReferencesAtPosition`).
    fn request_locations(
        model: &Entity<crate::TextModel>,
        kind: LocationKind,
        position: PointUtf16,
        compact: bool,
        cx: &mut Context<Self>,
    ) -> Vec<Task<anyhow::Result<Vec<Location>>>> {
        let context = model.read(cx).provider_context();
        let language: crate::providers::ProviderTarget = model.read(cx).into();
        match kind {
            LocationKind::Definition => definition_providers(cx, language)
                .into_iter()
                .map(|p| p.provide_definition(&context, position, cx))
                .collect(),
            LocationKind::Declaration => declaration_providers(cx, language)
                .into_iter()
                .map(|p| p.provide_declaration(&context, position, cx))
                .collect(),
            LocationKind::TypeDefinition => type_definition_providers(cx, language)
                .into_iter()
                .map(|p| p.provide_type_definition(&context, position, cx))
                .collect(),
            LocationKind::Implementation => implementation_providers(cx, language)
                .into_iter()
                .map(|p| p.provide_implementation(&context, position, cx))
                .collect(),
            LocationKind::References => reference_providers(cx, language)
                .into_iter()
                .map(|provider| {
                    let with_declaration = provider.provide_references(
                        &context,
                        position,
                        ReferenceContext {
                            include_declaration: true,
                        },
                        cx,
                    );
                    if !compact {
                        return with_declaration;
                    }
                    let context = context.clone();
                    cx.spawn(async move |_, cx| {
                        let references = with_declaration.await?;
                        if references.len() != 2 {
                            return Ok(references);
                        }
                        let without_declaration = cx
                            .update(|cx| {
                                provider.provide_references(
                                    &context,
                                    position,
                                    ReferenceContext {
                                        include_declaration: false,
                                    },
                                    cx,
                                )
                            })
                            .await?;
                        Ok(if without_declaration.len() == 1 {
                            without_declaration
                        } else {
                            references
                        })
                    })
                })
                .collect(),
        }
    }

    /// Runs a navigation command at the cursor (Monaco's `revealDefinition`,
    /// `revealDeclaration`, `goToTypeDefinition`, `goToImplementation` and
    /// `goToReferences`): one result is selected; several are handled as
    /// `options.goto_location` says (a peek view by default).
    pub fn go_to(&mut self, kind: LocationKind, cx: &mut Context<Self>) {
        self.navigate(kind, Navigation::default(), cx);
    }

    /// Go to definition in an editor beside this one (Monaco
    /// `editor.action.revealDefinitionAside`, Cmd/Ctrl+K F12): the result
    /// goes to the host's [editor openers](crate::register_editor_opener),
    /// else as
    /// [`CodeEditorEvent::OpenLocationToSide`](crate::CodeEditorEvent::OpenLocationToSide),
    /// even in this editor's model. (Standalone Monaco has no editor groups and
    /// opens it in place; a host without one can do the same with
    /// [`CodeEditor::go_to_location`].)
    pub fn reveal_definition_aside(&mut self, cx: &mut Context<Self>) {
        self.navigate(
            LocationKind::Definition,
            Navigation {
                open_to_side: true,
                ..Navigation::default()
            },
            cx,
        );
    }

    /// Runs a navigation command and shows the results in a peek view, even
    /// a single one (Monaco's `peekDefinition`, `peekDeclaration`,
    /// `peekTypeDefinition`, `peekImplementation` and the references peek).
    pub fn peek(&mut self, kind: LocationKind, cx: &mut Context<Self>) {
        self.navigate(
            kind,
            Navigation {
                open_in_peek: true,
                ..Navigation::default()
            },
            cx,
        );
    }

    /// Moves the cursor to `position` and goes to `locations` like a
    /// navigation command (Monaco's `editor.action.goToLocations` API
    /// command and its `GenericGoToLocationAction`): one is selected, and
    /// several are handled as `multiple` says, or as
    /// `goto_location.multiple_references` without it. The peek view's
    /// title is "Locations". Without locations, `no_results_message` shows
    /// at the cursor, if given. Locations without a uri are in this model.
    pub fn go_to_locations(
        &mut self,
        position: PointUtf16,
        locations: Vec<Location>,
        multiple: Option<GoToLocation>,
        no_results_message: Option<SharedString>,
        cx: &mut Context<Self>,
    ) {
        self.run_go_to_locations(position, locations, multiple, no_results_message, false, cx);
    }

    /// [`CodeEditor::go_to_locations`] showing the locations in a peek
    /// view, even a single one (Monaco's `editor.action.peekLocations` API
    /// command, also `editor.action.showReferences`).
    pub fn peek_locations(
        &mut self,
        position: PointUtf16,
        locations: Vec<Location>,
        multiple: Option<GoToLocation>,
        cx: &mut Context<Self>,
    ) {
        self.run_go_to_locations(position, locations, multiple, None, true, cx);
    }

    /// Monaco's `goToLocations` handler: `setPosition`, then the generic
    /// action's `run` (which has no alternative command).
    pub(crate) fn run_go_to_locations(
        &mut self,
        position: PointUtf16,
        locations: Vec<Location>,
        multiple: Option<GoToLocation>,
        no_results_message: Option<SharedString>,
        open_in_peek: bool,
        cx: &mut Context<Self>,
    ) {
        let position = self.model().read(cx).validate_position(position);
        self.set_selections(&[Selection::collapsed(position)], cx);
        self.reveal_position_in_center_if_outside_viewport(position, cx);
        let uri = self.model().read(cx).uri().cloned();
        let locations = sorted_locations(locations, uri.as_ref());
        self.announce(aria_message(&locations), cx);
        if locations.is_empty() {
            // Monaco mutes the message unless one was given.
            if let Some(message) = no_results_message {
                self.show_message(message, position, cx);
            }
            return;
        }
        let preference = multiple.unwrap_or(self.options.goto_location.multiple_references);
        self.on_locations(None, preference, locations, open_in_peek, false, cx);
    }

    /// The model `location` is in: this editor's for no uri or its own,
    /// else an open model with that uri (Monaco's `ITextModelService`,
    /// which in the standalone editor resolves only existing models).
    pub(crate) fn model_for_location(
        &self,
        location: &Location,
        cx: &App,
    ) -> Option<Entity<crate::TextModel>> {
        match &location.uri {
            None => Some(self.model().clone()),
            Some(_) if self.model().read(cx).is_at(location.uri.as_deref()) => {
                Some(self.model().clone())
            }
            Some(_) => crate::registry::models(cx)
                .into_iter()
                .find(|model| model.read(cx).is_at(location.uri.as_deref())),
        }
    }

    /// Monaco `SymbolNavigationAction.run`.
    pub(crate) fn navigate(
        &mut self,
        kind: LocationKind,
        navigation: Navigation,
        cx: &mut Context<Self>,
    ) {
        let position = self.cursors.primary().position;
        let model = self.model().clone();
        self.navigate_from(kind, &model, position, navigation, cx);
    }

    /// Runs a navigation command from `position` in `model`, which may be
    /// another model than this editor's (Monaco's `runEditorCommand` with
    /// a `SymbolNavigationAnchor`, as the inlay hint location menu runs
    /// them), showing the results in this editor.
    pub(crate) fn navigate_from(
        &mut self,
        kind: LocationKind,
        model: &Entity<crate::TextModel>,
        position: PointUtf16,
        navigation: Navigation,
        cx: &mut Context<Self>,
    ) {
        let Navigation {
            open_in_peek,
            open_to_side,
            mute_message,
            ..
        } = navigation;
        // Go to References is compact, Peek References isn't.
        let compact = kind == LocationKind::References && !open_in_peek;
        let tasks = Self::request_locations(model, kind, position, compact, cx);
        let model = model.clone();
        let state = self.navigation_state(cx);
        let uri = model.read(cx).uri().cloned();
        self.start_cancellable(crate::editor_state::CancellableOperation::Navigation);
        self.goto.task = Some(cx.spawn(async move |this, cx| {
            let mut locations = Vec::new();
            for task in tasks {
                match task.await {
                    Ok(found) => locations.extend(found),
                    Err(err) => log::warn!("navigation failed: {err:#}"),
                }
            }
            let locations = sorted_locations(locations, uri.as_ref());
            this.update(cx, |this, cx| {
                this.goto.task = None;
                // Monaco cancels when the text or the cursor changed.
                if this.navigation_state(cx) != state {
                    return;
                }
                // Monaco alerts the results' `ariaMessage` first.
                this.announce(aria_message(&locations), cx);
                // Monaco `referenceAt`, on `OneReference.range`: a link's
                // target selection range.
                let at_cursor = locations.iter().any(|location| {
                    let range = location.selection_range();
                    location.uri == uri && range.start <= position && position <= range.end
                });
                let alternative = this
                    .options
                    .goto_location
                    .alternative(kind)
                    .filter(|&alternative| navigation.may_run(alternative));
                match (locations.len(), alternative) {
                    (0, _) => {
                        if !mute_message {
                            let word = this
                                .model()
                                .read(cx)
                                .word_at_position(position)
                                .map(|word| word.word);
                            let message = no_result_message(kind, word.as_deref());
                            this.show_message(message, position, cx);
                        }
                    }
                    // Already at the only result: run the alternative.
                    (1, Some(alternative)) if at_cursor => {
                        this.navigate_from(
                            alternative.kind(),
                            &model,
                            position,
                            navigation.alternative(kind, alternative),
                            cx,
                        );
                    }
                    _ => {
                        let preference = this.options.goto_location.multiple(kind);
                        this.on_locations(
                            Some(kind),
                            preference,
                            locations,
                            open_in_peek,
                            open_to_side,
                            cx,
                        )
                    }
                }
            })
            .ok();
        }));
    }

    fn navigation_state(&self, cx: &App) -> (text::Version, Vec<Selection>) {
        (
            self.model().read(cx).buffer().version(),
            self.cursors.selections(),
        )
    }

    /// Monaco `SymbolNavigationAction._onResult`: `preference` is what to
    /// do with several results; `kind` names them in the peek view's title
    /// ("Locations" without one).
    fn on_locations(
        &mut self,
        kind: Option<LocationKind>,
        preference: GoToLocation,
        locations: Vec<Location>,
        open_in_peek: bool,
        open_to_side: bool,
        cx: &mut Context<Self>,
    ) {
        if !self.in_peek
            && (open_in_peek || (preference == GoToLocation::Peek && locations.len() > 1))
        {
            self.open_peek(kind, locations, None, cx);
            return;
        }
        let first = locations[0].clone();
        let peek = locations.len() > 1 && preference == GoToLocation::GotoAndPeek;
        let opened_here = if open_to_side {
            // The peek view would belong to the other editor.
            self.open_location_with_openers(first.clone(), true, cx);
            false
        } else {
            self.open_location(&first, !peek, cx)
        };
        if peek && opened_here {
            self.open_peek(kind, locations.clone(), Some(0), cx);
        }
        if preference == GoToLocation::Goto {
            self.put_symbol_navigation(locations, 0, cx);
        }
    }

    /// Selects the start of `location` and centers it at once if it's in
    /// this editor's model, briefly highlighting it; otherwise offers it
    /// to the host's editor openers, else emits
    /// [`CodeEditorEvent::OpenLocation`](crate::CodeEditorEvent::OpenLocation).
    /// Returns whether it was in this model.
    pub(crate) fn open_location(
        &mut self,
        location: &Location,
        highlight: bool,
        cx: &mut Context<Self>,
    ) -> bool {
        let here = self.model().read(cx).is_at(location.uri.as_deref());
        if !here {
            self.open_location_with_openers(location.clone(), false, cx);
            return false;
        }
        // Monaco `_openReference`: a link's `targetSelectionRange`, else its
        // range.
        let range = location.selection_range();
        let start = range.start;
        self.set_selections(&[Selection::collapsed(start)], cx);
        // Standalone Monaco's `openCodeEditor` ignores the requested
        // `NearTopIfOutsideViewport` and centers the selection at once
        // (`StandaloneCodeEditorService.doOpenEditor`).
        self.reveal_range_with_scroll_type(
            start..start,
            RevealType::Center,
            ScrollType::Immediate,
            cx,
        );
        if highlight {
            self.highlight_symbol(range, cx);
        }
        true
    }

    /// Monaco's `symbolHighlight` decoration on a jump target.
    fn highlight_symbol(&mut self, range: std::ops::Range<PointUtf16>, cx: &mut Context<Self>) {
        let collection = self.create_decorations_collection(
            vec![(
                range,
                DecorationOptions {
                    background: Some(self.theme.colors.symbol_highlight_background),
                    border: self.theme.colors.symbol_highlight_border,
                    ..Default::default()
                },
            )],
            cx,
        );
        self.goto.highlight = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(HIGHLIGHT_DURATION).await;
            let mut collection = collection;
            this.update(cx, |_, cx| collection.clear(cx)).ok();
        }));
    }

    fn put_symbol_navigation(
        &mut self,
        locations: Vec<Location>,
        index: usize,
        cx: &mut Context<Self>,
    ) {
        let nav = cx.default_global::<SymbolNavigation>();
        if locations.len() > 1 {
            *nav = SymbolNavigation { locations, index };
        } else {
            *nav = SymbolNavigation::default();
        }
    }

    /// F12 after a navigation command with several results in `Goto` mode:
    /// goes to the next one (Monaco `editor.gotoNextSymbolFromResult`).
    /// Returns whether there were results.
    pub fn go_to_next_symbol_from_result(&mut self, cx: &mut Context<Self>) -> bool {
        let Some(nav) = cx.try_global::<SymbolNavigation>() else {
            return false;
        };
        if nav.locations.is_empty() {
            return false;
        }
        let index = (nav.index + 1) % nav.locations.len();
        let location = nav.locations[index].clone();
        cx.global_mut::<SymbolNavigation>().index = index;
        // Monaco `SymbolNavigationService.revealNext`: no highlight.
        self.open_location(&location, false, cx);
        true
    }

    /// Escape: stops F12 navigation (Monaco
    /// `editor.gotoNextSymbolFromResult.cancel`). Returns whether it was on.
    pub(crate) fn cancel_symbol_navigation(&mut self, cx: &mut Context<Self>) -> bool {
        if !has_symbol_navigation(cx) {
            return false;
        }
        cx.set_global(SymbolNavigation::default());
        true
    }

    /// Stops F12 navigation once the cursor leaves the results (Monaco resets
    /// when the active editor's position isn't in one of them).
    pub(crate) fn check_symbol_navigation(&mut self, cx: &mut Context<Self>) {
        let Some(nav) = cx.try_global::<SymbolNavigation>() else {
            return;
        };
        if nav.locations.is_empty() {
            return;
        }
        let uri = self.model().read(cx).uri().cloned();
        let position = self.cursors.primary().position;
        // Monaco tests `OneReference.range`: a link's target selection range.
        let inside = nav.locations.iter().any(|location| {
            let range = location.selection_range();
            location.uri == uri && range.start <= position && position <= range.end
        });
        if !inside {
            cx.set_global(SymbolNavigation::default());
        }
    }

    /// Runs [`GoToLocations`] or [`PeekLocations`] in this editor: nothing
    /// when `uri` names another model (standalone Monaco's
    /// `openCodeEditor` only finds the focused editor's model).
    pub(crate) fn go_to_locations_action(&mut self, args: GoToLocations, cx: &mut Context<Self>) {
        let uri = self.model().read(cx).uri().cloned();
        if args.uri.is_some() && args.uri.as_deref() != uri.as_deref() {
            return;
        }
        let position = args
            .position
            .map_or(self.cursors.primary().position, CommandPosition::to_point);
        let locations = args
            .locations
            .iter()
            .map(CommandLocation::to_location)
            .collect();
        self.run_go_to_locations(
            position,
            locations,
            args.multiple.as_deref().and_then(parse_go_to_location),
            args.no_results_message.map(Into::into),
            args.open_in_peek,
            cx,
        );
    }
}

impl From<PeekLocations> for GoToLocations {
    /// Monaco's `peekLocations` handler runs `goToLocations` with
    /// `openInPeek`.
    fn from(peek: PeekLocations) -> Self {
        Self {
            uri: peek.uri,
            position: peek.position,
            locations: peek.locations,
            multiple: peek.multiple,
            no_results_message: None,
            open_in_peek: true,
        }
    }
}

/// Monaco `GoToLocationValues`.
pub(crate) fn parse_go_to_location(value: &str) -> Option<GoToLocation> {
    match value {
        "peek" => Some(GoToLocation::Peek),
        "gotoAndPeek" => Some(GoToLocation::GotoAndPeek),
        "goto" => Some(GoToLocation::Goto),
        _ => None,
    }
}

/// `editor.action.goToLocations` arguments, positional as Monaco's
/// `executeCommand` passes them: uri, position, locations, multiple,
/// noResultsMessage, openInPeek.
fn go_to_locations_args(args: &[serde_json::Value]) -> anyhow::Result<GoToLocations> {
    let arg = |ix: usize| args.get(ix).filter(|value| !value.is_null()).cloned();
    let uri: String = serde_json::from_value(arg(0).context("missing uri")?)?;
    let position: CommandPosition = serde_json::from_value(arg(1).context("missing position")?)?;
    let locations: Vec<CommandLocation> =
        serde_json::from_value(arg(2).context("missing locations")?)?;
    let multiple: Option<String> = arg(3).map(serde_json::from_value).transpose()?;
    let no_results_message: Option<String> = arg(4).map(serde_json::from_value).transpose()?;
    let open_in_peek: Option<bool> = arg(5).map(serde_json::from_value).transpose()?;
    Ok(GoToLocations {
        uri: Some(uri),
        position: Some(position),
        locations,
        multiple,
        no_results_message,
        open_in_peek: open_in_peek.unwrap_or(false),
    })
}

/// The editor to run a command for `uri` in: the focused one if it shows
/// `uri` (as Monaco's `openCodeEditor` from the focused editor), else the
/// first one showing it. Peek view previews don't count.
fn editor_for_uri(uri: &str, cx: &App) -> Option<Entity<CodeEditor>> {
    let editors: Vec<_> = crate::registry::editors(cx)
        .into_iter()
        .filter(|editor| {
            let editor = editor.read(cx);
            !editor.in_peek
                && editor.model().read(cx).uri().is_some()
                && editor.model().read(cx).is_at(Some(uri))
        })
        .collect();
    editors
        .iter()
        .find(|editor| editor.read(cx).has_focus)
        .or(editors.first())
        .cloned()
}

fn run_go_to_locations_command(args: anyhow::Result<GoToLocations>, cx: &mut App) {
    let args = match args {
        Ok(args) => args,
        Err(err) => {
            log::warn!("editor.action.goToLocations: {err:#}");
            return;
        }
    };
    // Commands often run inside an editor's update.
    cx.defer(move |cx| {
        let Some(editor) = args.uri.as_deref().and_then(|uri| editor_for_uri(uri, cx)) else {
            return;
        };
        editor.update(cx, |editor, cx| editor.go_to_locations_action(args, cx));
    });
}

/// Registers Monaco's `editor.action.goToLocations` and
/// `editor.action.peekLocations` API commands (and the
/// `editor.action.showReferences` alias), for
/// [`execute_command`](crate::execute_command) and command ids in code
/// lenses, code actions and completions.
pub(crate) fn register_commands(cx: &mut App) {
    // Built in for good: the registrations are never disposed.
    let _ = crate::commands::register_command(cx, "editor.action.goToLocations", |args, cx| {
        run_go_to_locations_command(go_to_locations_args(args), cx);
    });
    for id in [
        "editor.action.peekLocations",
        "editor.action.showReferences",
    ] {
        let _ = crate::commands::register_command(cx, id, |args, cx| {
            // Monaco passes on uri, position, locations and multiple only.
            let args = go_to_locations_args(&args[..args.len().min(4)]).map(|args| GoToLocations {
                open_in_peek: true,
                ..args
            });
            run_go_to_locations_command(args, cx);
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn location(uri: Option<&str>, row: u32) -> Location {
        Location {
            uri: uri.map(SharedString::from),
            range: PointUtf16::new(row, 0)..PointUtf16::new(row, 3),
            ..Default::default()
        }
    }

    #[test]
    fn locations_are_sorted_and_deduplicated() {
        let here = SharedString::from("file:///b.rs");
        let sorted = sorted_locations(
            vec![
                location(None, 5),
                location(Some("file:///a.rs"), 2),
                location(Some("file:///b.rs"), 5),
                location(None, 1),
            ],
            Some(&here),
        );
        assert_eq!(
            sorted,
            vec![
                location(Some("file:///a.rs"), 2),
                location(Some("file:///b.rs"), 1),
                location(Some("file:///b.rs"), 5),
            ]
        );
    }

    #[test]
    fn messages() {
        assert_eq!(
            no_result_message(LocationKind::Definition, Some("foo")),
            "No definition found for 'foo'"
        );
        assert_eq!(
            no_result_message(LocationKind::References, None),
            "No references found"
        );
    }

    /// Monaco `ReferencesModel.ariaMessage`.
    #[test]
    fn aria_messages() {
        assert_eq!(aria_message(&[]), "No results found");
        let a = |row| location(Some("file:///src/a.rs"), row);
        let b = |row| location(Some("file:///src/b.rs"), row);
        assert_eq!(aria_message(&[a(1)]), "Found 1 symbol in /src/a.rs");
        assert_eq!(aria_message(&[a(1), a(4)]), "Found 2 symbols in /src/a.rs");
        assert_eq!(
            aria_message(&[a(1), a(4), b(2)]),
            "Found 3 symbols in 2 files"
        );
    }
}
