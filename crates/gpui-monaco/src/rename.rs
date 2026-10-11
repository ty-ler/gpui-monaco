//! Rename (F2): an inline box to type the new name, then the providers'
//! edits applied across documents.
//!
//! Ports Monaco's `src/vs/editor/contrib/rename/browser/rename.ts` and
//! `renameWidget.ts` (MIT), without rename suggestions or the refactor
//! preview.

use std::ops::Range;
use std::sync::Arc;

use gpui::{
    AppContext as _, Context, Entity, InteractiveElement, IntoElement, ParentElement, Render,
    SharedString, Styled, Subscription, Task, WeakEntity, Window, div, px,
};
use text::PointUtf16;

use crate::actions::{Newline, RemoveSecondaryCursors};
use crate::cursor::Selection;
use crate::editor::{CodeEditor, CodeEditorEvent};
use crate::model::model_for_uri;
use crate::options::EditorOptions;
use crate::providers::{RenameLocation, TextEdit, WorkspaceEdit, rename_providers};
use crate::theme::{EditorColors, Theme};
use crate::widgets::{ContentWidget, ContentWidgetPreference, WidgetId};

#[derive(Default)]
pub(crate) struct RenameState {
    task: Option<Task<()>>,
    /// The box, the location and the provider that resolved it.
    session: Option<Session>,
}

struct Session {
    widget: WidgetId,
    location: RenameLocation,
    position: PointUtf16,
    provider: usize,
}

/// Monaco `RenameWidget`: the input box.
struct RenameInput {
    editor: WeakEntity<CodeEditor>,
    name: String,
    /// Selected UTF-16 columns of `name`.
    selection: Range<u32>,
    input: Option<Entity<CodeEditor>>,
    theme: Arc<Theme>,
    colors: EditorColors,
    font_size: gpui::Pixels,
    _subscriptions: Vec<Subscription>,
}

impl RenameInput {
    fn ensure_input(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.input.is_some() {
            return;
        }
        let name = self.name.clone();
        let theme = self.theme.clone();
        let selection = self.selection.clone();
        let input = cx.new(|cx| {
            let mut editor = CodeEditor::with_text(&name, window, cx);
            editor.update_options(EditorOptions::single_line_input(), cx);
            editor.set_theme(theme, cx);
            editor.set_selections(
                &[Selection::new(
                    PointUtf16::new(0, selection.start),
                    PointUtf16::new(0, selection.end),
                )],
                cx,
            );
            editor
        });
        window.focus(&input.read(cx).focus_handle.clone(), cx);
        // Monaco cancels when focus leaves the editor and the box.
        self._subscriptions.push(
            cx.subscribe(&input, |this, _, event: &CodeEditorEvent, cx| {
                if matches!(event, CodeEditorEvent::Blurred) {
                    this.with_editor(cx, |editor, cx| {
                        editor.cancel_rename(false, cx);
                    });
                }
            }),
        );
        self.input = Some(input);
    }

    fn with_editor(
        &self,
        cx: &mut Context<Self>,
        f: impl FnOnce(&mut CodeEditor, &mut Context<CodeEditor>),
    ) {
        if let Some(editor) = self.editor.upgrade() {
            editor.update(cx, f);
        }
    }
}

impl Render for RenameInput {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.ensure_input(window, cx);
        let colors = &self.colors;
        div()
            .key_context("RenameInput")
            .on_action(cx.listener(|this, _: &Newline, _, cx| {
                let name = this
                    .input
                    .as_ref()
                    .map(|input| input.read(cx).model().read(cx).text())
                    .unwrap_or_default();
                this.with_editor(cx, |editor, cx| editor.accept_rename(name, cx));
            }))
            .on_action(cx.listener(|this, _: &RemoveSecondaryCursors, _, cx| {
                this.with_editor(cx, |editor, cx| {
                    editor.cancel_rename(true, cx);
                });
            }))
            .w(px(220.))
            .p(px(4.))
            .flex()
            .flex_col()
            .gap(px(2.))
            .bg(colors.widget_background)
            .border_1()
            .border_color(colors.widget_border)
            .rounded(px(4.))
            .child(
                div()
                    .h(self.font_size * 1.6)
                    .px(px(2.))
                    .bg(colors.input_background)
                    .border_1()
                    .border_color(colors.input_option_active_border)
                    .children(self.input.clone()),
            )
            .child(
                div()
                    .text_size(self.font_size * 0.85)
                    .text_color(colors.widget_foreground.opacity(0.8))
                    .child("Enter to Rename"),
            )
    }
}

/// Where the selection is in the name, if it's inside it on one line
/// (Monaco `RenameWidget._getSelection`); else the whole name.
fn initial_selection(location: &RenameLocation, selection: &Selection) -> Range<u32> {
    let len = location.text.encode_utf16().count() as u32;
    let range = selection.range();
    let inside = range.start != range.end
        && range.start.row == range.end.row
        && location.range.start <= range.start
        && range.end <= location.range.end;
    if inside {
        let start = range
            .start
            .column
            .saturating_sub(location.range.start.column);
        let end = range.end.column.min(location.range.end.column) - location.range.start.column;
        start..end.min(len)
    } else {
        0..len
    }
}

impl CodeEditor {
    /// F2: renames the symbol at the cursor (Monaco `editor.action.rename`).
    /// The providers resolve what to rename (or the word at the cursor is
    /// used), a box asks for the new name, and Enter applies the edits.
    pub fn rename(&mut self, cx: &mut Context<Self>) {
        if self.options.read_only {
            return;
        }
        self.cancel_rename(false, cx);
        let model = self.model().read(cx);
        let providers = rename_providers(cx, self.provider_target(cx));
        if providers.is_empty() {
            return;
        }
        let context = model.provider_context();
        let position = self.cursors.primary().position;
        let state = self.rename_state(cx);
        // Monaco `RenameSkeleton.resolveRenameLocation`: providers in order
        // until one without `resolveRenameLocation`.
        let mut resolves = Vec::new();
        for (ix, provider) in providers.iter().enumerate() {
            match provider.resolve_rename_location(&context, position, cx) {
                Some(task) => resolves.push((ix, task)),
                None => break,
            }
        }
        self.rename.task = Some(cx.spawn(async move |this, cx| {
            let mut rejects: Vec<String> = Vec::new();
            let mut resolved = None;
            for (ix, task) in resolves {
                match task.await {
                    Ok(Some(location)) => {
                        resolved = Some((ix, location));
                        break;
                    }
                    Ok(None) => {}
                    Err(err) => rejects.push(err.to_string()),
                }
            }
            this.update(cx, |this, cx| {
                this.rename.task = None;
                if this.rename_state(cx) != state {
                    return;
                }
                let (provider, location) = match resolved {
                    Some(resolved) => resolved,
                    None => {
                        // The word at the cursor, as Monaco falls back to.
                        if !rejects.is_empty() {
                            this.show_message(rejects.join("\n"), position, cx);
                            return;
                        }
                        let word = this.model().read(cx).word_at_position(position);
                        let location = match word {
                            Some(word) => RenameLocation {
                                range: word.range,
                                text: word.word.into(),
                            },
                            None => RenameLocation {
                                range: position..position,
                                text: SharedString::default(),
                            },
                        };
                        (0, location)
                    }
                };
                this.show_rename_input(location, position, provider, cx);
            })
            .ok();
        }));
    }

    fn rename_state(&self, cx: &gpui::App) -> (text::Version, Vec<Selection>) {
        (
            self.model().read(cx).buffer().version(),
            self.cursors.selections(),
        )
    }

    fn show_rename_input(
        &mut self,
        location: RenameLocation,
        position: PointUtf16,
        provider: usize,
        cx: &mut Context<Self>,
    ) {
        let selection = initial_selection(&location, &self.cursors.selections()[0]);
        let mut theme = (*self.theme).clone();
        theme.colors.background = theme.colors.input_background;
        theme.colors.gutter_background = theme.colors.input_background;
        let editor = cx.entity().downgrade();
        let view = cx.new(|_| RenameInput {
            editor,
            name: location.text.to_string(),
            selection,
            input: None,
            theme: Arc::new(theme),
            colors: self.theme.colors.clone(),
            font_size: self.options.font_size,
            _subscriptions: Vec::new(),
        });
        let widget = self.add_content_widget(
            ContentWidget {
                position: location.range.start,
                preference: vec![
                    ContentWidgetPreference::Below,
                    ContentWidgetPreference::Above,
                ],
                view: view.into(),
            },
            cx,
        );
        self.rename.session = Some(Session {
            widget,
            location,
            position,
            provider,
        });
    }

    /// Whether the rename box is showing.
    pub fn is_rename_visible(&self) -> bool {
        self.rename.session.is_some()
    }

    /// Closes the rename box (Monaco `cancelRenameInput`, Escape), focusing
    /// the editor if `focus_editor`. Returns whether it was open.
    pub fn cancel_rename(&mut self, focus_editor: bool, cx: &mut Context<Self>) -> bool {
        self.rename.task = None;
        let Some(session) = self.rename.session.take() else {
            return false;
        };
        self.remove_widget(session.widget, cx);
        if focus_editor {
            self.refocus = true;
        }
        true
    }

    /// The text or the cursor left the symbol: Monaco cancels the box.
    pub(crate) fn check_rename(&mut self, cx: &mut Context<Self>) {
        let Some(session) = &self.rename.session else {
            return;
        };
        let head = self.cursors.primary().position;
        let range = &session.location.range;
        if head < range.start || head > range.end {
            self.cancel_rename(false, cx);
        }
    }

    /// Enter in the rename box: asks the providers for edits and applies
    /// them (Monaco `RenameController.run`, part 2).
    pub fn accept_rename(&mut self, new_name: String, cx: &mut Context<Self>) {
        let Some(session) = self.rename.session.take() else {
            return;
        };
        self.remove_widget(session.widget, cx);
        self.refocus = true;
        // Unchanged or blank names do nothing.
        if new_name == session.location.text.as_ref() || new_name.trim().is_empty() {
            return;
        }
        let model = self.model().read(cx);
        let context = model.provider_context();
        let providers = rename_providers(cx, self.provider_target(cx));
        let state = self.rename_state(cx);
        let tasks: Vec<_> = providers
            .iter()
            .skip(session.provider)
            .map(|provider| {
                provider.provide_rename_edits(&context, session.position, &new_name, cx)
            })
            .collect();
        let position = session.position;
        self.rename.task = Some(cx.spawn(async move |this, cx| {
            let mut rejects: Vec<String> = Vec::new();
            let mut edit = None;
            for task in tasks {
                match task.await {
                    Ok(Some(result)) => {
                        edit = Some(result);
                        break;
                    }
                    Ok(None) => rejects.push("No result.".into()),
                    Err(err) => rejects.push(err.to_string()),
                }
            }
            this.update(cx, |this, cx| {
                this.rename.task = None;
                if this.rename_state(cx) != state {
                    return;
                }
                let Some(edit) = edit else {
                    this.show_message(rejects.join("\n"), position, cx);
                    return;
                };
                // Collapse the selection to its active end, then apply.
                let head = this.cursors.primary().position;
                this.set_selections(&[Selection::collapsed(head)], cx);
                if let Err(err) = this.apply_workspace_edit(edit, cx) {
                    log::warn!("rename failed to apply edits: {err:#}");
                    this.show_message("Rename failed to apply edits", position, cx);
                }
            })
            .ok();
        }));
    }

    /// Applies `edit`: to this editor's model as one undo step with the
    /// cursors following, and to other documents found with
    /// [`model_for_uri`](crate::model_for_uri) (Monaco's bulk edit
    /// service). Nothing is applied if a document can't be found.
    pub fn apply_workspace_edit(
        &mut self,
        edit: WorkspaceEdit,
        cx: &mut Context<Self>,
    ) -> anyhow::Result<()> {
        let uri = self.model().read(cx).uri().cloned();
        let mut here: Vec<TextEdit> = Vec::new();
        let mut elsewhere = Vec::new();
        for (target, edits) in edit.edits {
            match target {
                None => here.extend(edits),
                Some(target) if Some(&target) == uri.as_ref() => here.extend(edits),
                Some(target) => {
                    let model = model_for_uri(cx, &target)
                        .ok_or_else(|| anyhow::anyhow!("{target} isn't open"))?;
                    elsewhere.push((model, edits));
                }
            }
        }
        for (model, edits) in elsewhere {
            model
                .update(cx, |model, cx| {
                    let mut edits: Vec<_> = edits
                        .into_iter()
                        .map(|(range, text)| {
                            (
                                model.offset_at(range.start)..model.offset_at(range.end),
                                text,
                            )
                        })
                        .collect();
                    edits.sort_by_key(|(range, _)| range.start);
                    model.edit(edits, cx).map(|_| ())
                })
                .map_err(|err| anyhow::anyhow!("{err:?}"))?;
        }
        if !here.is_empty() {
            here.sort_by_key(|(range, _)| range.start);
            self.push_undo_stop(cx);
            self.execute_edits(here, None, cx);
            self.push_undo_stop(cx);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn selection_in_the_box() {
        let location = RenameLocation {
            range: PointUtf16::new(0, 4)..PointUtf16::new(0, 11),
            text: "foo_bar".into(),
        };
        let p = |c| PointUtf16::new(0, c);
        // An empty selection selects the whole name.
        assert_eq!(
            initial_selection(&location, &Selection::collapsed(p(6))),
            0..7
        );
        // A selection inside the name is kept.
        assert_eq!(
            initial_selection(&location, &Selection::new(p(8), p(11))),
            4..7
        );
        // One reaching outside isn't.
        assert_eq!(
            initial_selection(&location, &Selection::new(p(2), p(6))),
            0..7
        );
    }
}
