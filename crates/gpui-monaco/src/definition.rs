//! Go to definition (Monaco's `contrib/gotoSymbol`): Cmd/Ctrl+hover shows a
//! link on symbols the providers resolve, Cmd/Ctrl+click and F12 jump.
//! Locations in other models are reported with
//! [`CodeEditorEvent::OpenLocation`](crate::CodeEditorEvent) for the host to
//! open.

use std::ops::Range;
use std::time::Duration;

use gpui::{Context, Task};
use text::PointUtf16;

use crate::cursor::{self, Selection};
use crate::decorations::{DecorationId, DecorationOptions, DecorationUnderline};
use crate::editor::{CodeEditor, CodeEditorEvent};
use crate::providers::{Location, definition_providers};
use crate::theme::TokenStyle;

#[derive(Default)]
pub(crate) struct DefinitionLink {
    task: Option<Task<()>>,
    /// The word under the mouse and where it leads.
    pub link: Option<(Range<PointUtf16>, Vec<Location>)>,
    decorations: Vec<DecorationId>,
    pending: Option<PointUtf16>,
}

impl CodeEditor {
    fn request_definitions(
        &mut self,
        position: PointUtf16,
        cx: &mut Context<Self>,
    ) -> Vec<Task<anyhow::Result<Vec<Location>>>> {
        let model = self.model().read(cx);
        let context = model.provider_context();
        definition_providers(cx, self.provider_target(cx))
            .into_iter()
            .map(|provider| provider.provide_definition(&context, position, cx))
            .collect()
    }

    /// Mouse moved with the platform modifier held over `position` (or
    /// released / left text when `None`).
    pub(crate) fn definition_mouse_move(
        &mut self,
        position: Option<PointUtf16>,
        cx: &mut Context<Self>,
    ) {
        let Some(position) = position else {
            self.clear_definition_link(cx);
            return;
        };
        if let Some((range, _)) = &self.definition.link
            && range.start <= position
            && position < range.end
        {
            return;
        }
        if self.definition.pending == Some(position) {
            return;
        }
        self.clear_definition_link(cx);
        self.definition.pending = Some(position);
        let config = self.cursor_config(cx);
        let Some(word) = cursor::word_at(&config, self.snapshot.rope(), position) else {
            return;
        };
        self.definition.task = Some(cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(Duration::from_millis(50))
                .await;
            let Ok(tasks) = this.update(cx, |this, cx| this.request_definitions(position, cx))
            else {
                return;
            };
            let mut locations = Vec::new();
            for task in tasks {
                if let Ok(found) = task.await {
                    locations.extend(found);
                }
            }
            this.update(cx, |this, cx| {
                if locations.is_empty() || this.definition.pending != Some(position) {
                    return;
                }
                this.set_definition_link(word, locations, cx);
            })
            .ok();
        }));
    }

    fn set_definition_link(
        &mut self,
        range: Range<PointUtf16>,
        locations: Vec<Location>,
        cx: &mut Context<Self>,
    ) {
        let rope = self.snapshot.rope();
        let bytes = rope.point_utf16_to_offset(range.start)..rope.point_utf16_to_offset(range.end);
        let link = self.theme.colors.input_option_active_border;
        let old = std::mem::take(&mut self.definition.decorations);
        let owner = Some(cx.entity_id());
        self.definition.decorations = self.model().update(cx, |model, cx| {
            model.delta_decorations(
                owner,
                &old,
                vec![(
                    bytes,
                    DecorationOptions {
                        underline: Some(DecorationUnderline {
                            color: link,
                            wavy: false,
                        }),
                        text: Some(TokenStyle {
                            foreground: Some(link),
                            ..Default::default()
                        }),
                        ..Default::default()
                    },
                )],
                cx,
            )
        });
        self.definition.link = Some((range, locations));
        cx.notify();
    }

    pub(crate) fn clear_definition_link(&mut self, cx: &mut Context<Self>) {
        self.definition.task = None;
        self.definition.pending = None;
        self.definition.link = None;
        let old = std::mem::take(&mut self.definition.decorations);
        if !old.is_empty() {
            self.model().update(cx, |model, cx| {
                model.delta_decorations(None, &old, Vec::new(), cx)
            });
            cx.notify();
        }
    }

    /// Whether a Cmd/Ctrl+hover link is showing (for the pointer cursor).
    pub(crate) fn has_definition_link(&self) -> bool {
        self.definition.link.is_some()
    }

    /// Follows the link under the mouse (Cmd/Ctrl+click; with Alt, or Cmd/Ctrl
    /// when Alt adds cursors, `open_to_side`). Returns whether there was
    /// one.
    pub(crate) fn follow_definition_link(
        &mut self,
        position: PointUtf16,
        open_to_side: bool,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some((range, _)) = self.definition.link.clone() else {
            return false;
        };
        if !(range.start <= position && position <= range.end) {
            return false;
        }
        self.clear_definition_link(cx);
        // Monaco moves the cursor there and runs go to definition, without
        // a message when nothing is found.
        self.set_selections(&[Selection::collapsed(position)], cx);
        // Monaco `definitionLinkOpensInPeek`, except in a peek view.
        let open_in_peek =
            !open_to_side && self.options.definition_link_opens_in_peek && !self.in_peek;
        self.navigate(
            crate::options::LocationKind::Definition,
            crate::goto::Navigation {
                open_in_peek,
                open_to_side,
                mute_message: true,
                allow_alternative: true,
            },
            cx,
        );
        true
    }

    /// Selects `location` if it's in this model, else asks the host to open
    /// it.
    pub fn go_to_location(&mut self, location: Location, cx: &mut Context<Self>) {
        let same_model =
            location.uri.is_none() || location.uri.as_ref() == self.model().read(cx).uri();
        if same_model {
            self.set_selections(
                &[Selection {
                    anchor: location.range.start,
                    head: location.range.start,
                }],
                cx,
            );
        } else {
            cx.emit(CodeEditorEvent::OpenLocation(location));
        }
    }

    /// F12: go to the definition of the symbol at the cursor (Monaco
    /// `editor.action.revealDefinition`).
    pub fn go_to_definition(&mut self, cx: &mut Context<Self>) {
        self.go_to(crate::options::LocationKind::Definition, cx);
    }
}
