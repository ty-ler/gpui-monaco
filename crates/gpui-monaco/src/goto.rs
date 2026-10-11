//! Symbol navigation: go to definition, declaration, type definition,
//! implementations and references, with several results shown in a peek
//! view or stepped through with F4, and the `editor.action.goToLocations`
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
use crate::editor::{CodeEditor, CodeEditorEvent};
use crate::options::{GoToLocation, LocationKind};
use crate::providers::{
    Location, ReferenceContext, declaration_providers, definition_providers,
    implementation_providers, reference_providers, type_definition_providers,
};
use crate::view::reveal::RevealType;

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
}

impl Default for Navigation {
    fn default() -> Self {
        Self {
            open_in_peek: false,
            open_to_side: false,
            mute_message: false,
            allow_alternative: true,
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
            uri: location.uri.or_else(|| queried.cloned()),
            range: location.range,
        })
        .collect();
    locations.sort_by(|a, b| {
        a.uri
            .cmp(&b.uri)
            .then(a.range.start.cmp(&b.range.start))
            .then(a.range.end.cmp(&b.range.end))
    });
    locations.dedup();
    locations
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

/// The results F4 steps through after a `Goto` navigation (Monaco
/// `SymbolNavigationService`). Shared by all editors, like Monaco's.
#[derive(Default)]
struct SymbolNavigation {
    locations: Vec<Location>,
    index: usize,
}

impl Global for SymbolNavigation {}

/// Whether F4 has results to step through.
pub fn has_symbol_navigation(cx: &App) -> bool {
    cx.try_global::<SymbolNavigation>()
        .is_some_and(|nav| !nav.locations.is_empty())
}

impl CodeEditor {
    fn request_locations(
        &mut self,
        kind: LocationKind,
        position: PointUtf16,
        cx: &mut Context<Self>,
    ) -> Vec<Task<anyhow::Result<Vec<Location>>>> {
        let model = self.model().read(cx);
        let context = model.provider_context();
        let language = model.language_id().map(|l| l.to_string());
        let language = language.as_deref();
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
                .map(|p| {
                    p.provide_references(
                        &context,
                        position,
                        ReferenceContext {
                            include_declaration: true,
                        },
                        cx,
                    )
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
    /// goes to the host as [`CodeEditorEvent::OpenLocationToSide`], even
    /// in this editor's model. (Standalone Monaco has no editor groups and
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

    /// Monaco `SymbolNavigationAction.run`.
    pub(crate) fn navigate(
        &mut self,
        kind: LocationKind,
        navigation: Navigation,
        cx: &mut Context<Self>,
    ) {
        let Navigation {
            open_in_peek,
            open_to_side,
            mute_message,
            allow_alternative,
        } = navigation;
        let position = self.cursors.primary().position;
        let tasks = self.request_locations(kind, position, cx);
        let state = self.navigation_state(cx);
        let uri = self.model().read(cx).uri().cloned();
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
                let at_cursor = locations.iter().any(|location| {
                    location.uri == uri
                        && location.range.start <= position
                        && position <= location.range.end
                });
                let alternative = this.options.goto_location.alternative(kind);
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
                    (1, Some(alternative)) if at_cursor && allow_alternative => {
                        this.navigate(
                            alternative,
                            Navigation {
                                open_in_peek,
                                mute_message,
                                allow_alternative: false,
                                open_to_side: false,
                            },
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
            cx.emit(CodeEditorEvent::OpenLocationToSide(first.clone()));
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

    /// Selects the start of `location` and reveals it near the top if it's
    /// in this editor's model, briefly highlighting it; otherwise emits
    /// [`CodeEditorEvent::OpenLocation`] for the host. Returns whether it
    /// was in this model.
    pub(crate) fn open_location(
        &mut self,
        location: &Location,
        highlight: bool,
        cx: &mut Context<Self>,
    ) -> bool {
        let here = location.uri.is_none() || location.uri.as_ref() == self.model().read(cx).uri();
        if !here {
            cx.emit(CodeEditorEvent::OpenLocation(location.clone()));
            return false;
        }
        let start = location.range.start;
        self.set_selections(&[Selection::collapsed(start)], cx);
        self.reveal_range(start..start, RevealType::NearTopIfOutsideViewport, cx);
        if highlight {
            self.highlight_symbol(location.range.clone(), cx);
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

    /// F4 after a navigation command with several results in `Goto` mode:
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
        self.open_location(&location, true, cx);
        true
    }

    /// Escape: stops F4 navigation (Monaco
    /// `editor.gotoNextSymbolFromResult.cancel`). Returns whether it was on.
    pub(crate) fn cancel_symbol_navigation(&mut self, cx: &mut Context<Self>) -> bool {
        if !has_symbol_navigation(cx) {
            return false;
        }
        cx.set_global(SymbolNavigation::default());
        true
    }

    /// Stops F4 navigation once the cursor leaves the results (Monaco resets
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
        let inside = nav.locations.iter().any(|location| {
            location.uri == uri
                && location.range.start <= position
                && position <= location.range.end
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
            !editor.in_peek && editor.model().read(cx).uri().map(|u| u.as_ref()) == Some(uri)
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
    crate::commands::register_command(cx, "editor.action.goToLocations", |args, cx| {
        run_go_to_locations_command(go_to_locations_args(args), cx);
    });
    for id in [
        "editor.action.peekLocations",
        "editor.action.showReferences",
    ] {
        crate::commands::register_command(cx, id, |args, cx| {
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
}
