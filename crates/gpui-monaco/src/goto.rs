//! Symbol navigation: go to definition, declaration, type definition,
//! implementations and references, with several results shown in a peek
//! view or stepped through with F4.
//!
//! Ports Monaco's `src/vs/editor/contrib/gotoSymbol/browser/goToCommands.ts`,
//! `symbolNavigation.ts` and `referencesModel.ts` (MIT).

use std::time::Duration;

use gpui::{App, Context, Global, SharedString, Task};
use text::PointUtf16;

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
    task: Option<Task<()>>,
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
                    _ => this.on_locations(kind, locations, open_in_peek, open_to_side, cx),
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

    /// Monaco `SymbolNavigationAction._onResult`.
    fn on_locations(
        &mut self,
        kind: LocationKind,
        locations: Vec<Location>,
        open_in_peek: bool,
        open_to_side: bool,
        cx: &mut Context<Self>,
    ) {
        let preference = self.options.goto_location.multiple(kind);
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
