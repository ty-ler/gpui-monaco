//! Rename (F2): an inline box to type the new name, with suggestions from
//! [`NewSymbolNamesProvider`](crate::providers::NewSymbolNamesProvider)s,
//! then the providers' edits applied across documents (Enter), or previewed
//! first (Cmd/Ctrl+Enter, through
//! [`set_workspace_edit_preview_handler`](crate::set_workspace_edit_preview_handler)).
//!
//! Ports Monaco's `src/vs/editor/contrib/rename/browser/rename.ts` and
//! `renameWidget.ts` (MIT).

use std::collections::HashSet;
use std::ops::Range;
use std::sync::Arc;

use gpui::{
    AppContext as _, Context, Entity, InteractiveElement, IntoElement, ParentElement, Render,
    SharedString, StatefulInteractiveElement, Styled, Subscription, Task, WeakEntity, Window, div,
    prelude::FluentBuilder as _, px,
};
use text::PointUtf16;

use crate::actions::{
    AcceptRenameInput, AcceptRenameInputWithPreview, CancelRenameInput, FocusNextRenameSuggestion,
    FocusPreviousRenameSuggestion,
};
use crate::codicons::{Codicon, icon as codicon};
use crate::cursor::Selection;
use crate::editor::{CodeEditor, CodeEditorEvent};
use crate::model::TextModelEvent;
use crate::providers::{
    NewSymbolName, NewSymbolNameTag, NewSymbolNameTriggerKind, RenameLocation,
    new_symbol_names_providers, rename_providers,
};
use crate::theme::{EditorColors, Theme};
use crate::widgets::{ContentWidget, ContentWidgetPreference, WidgetId};

/// Monaco `RenameCandidateListView`'s `MAX_N_CANDIDATES`: suggestions shown
/// at once.
const MAX_VISIBLE_CANDIDATES: usize = 7;

#[derive(Default)]
pub(crate) struct RenameState {
    pub(crate) task: Option<Task<()>>,
    /// The box, the location and the provider that resolved it.
    session: Option<Session>,
}

struct Session {
    widget: WidgetId,
    view: Entity<RenameInput>,
    location: RenameLocation,
    position: PointUtf16,
    provider: usize,
    /// Rename suggestions being asked for (Monaco
    /// `_renameCandidateProvidersCts`); dropping it cancels them.
    candidates: Option<Task<()>>,
}

/// The rename box's key context, around its input.
pub(crate) const RENAME_INPUT_KEY_CONTEXT: &str = "RenameInput";

/// Monaco `RenameWidget`: the input box, its suggestions button and the
/// list of suggestions.
struct RenameInput {
    editor: WeakEntity<CodeEditor>,
    /// The symbol's current name (Monaco `_currentName`).
    name: String,
    /// Selected UTF-16 columns of `name`.
    selection: Range<u32>,
    input: Option<Entity<CodeEditor>>,
    theme: Arc<Theme>,
    colors: EditorColors,
    font_size: gpui::Pixels,
    /// The editor's line height and typical character width (Monaco's
    /// `fontInfo`).
    line_height: gpui::Pixels,
    char_width: gpui::Pixels,
    /// The renamed range's length in UTF-16 columns.
    location_length: u32,
    /// Where the editor publishes the room around the line.
    room: crate::widgets::OverflowRoomCell,
    /// The preference has been set from the room.
    placed: bool,
    /// The candidate list's scrolling (Monaco's list reveals the focused
    /// candidate).
    scroll: gpui::ScrollHandle,
    /// Previews are possible: the label shows (Monaco `supportPreview`).
    support_preview: bool,
    /// Suggestion providers exist: the button shows.
    has_suggestions: bool,
    /// Suggestions are being asked for: the button stops them (Monaco's
    /// `'stop'` button state).
    requesting: bool,
    /// The suggestions, newest first.
    candidates: Vec<NewSymbolName>,
    /// Every suggestion received this session (Monaco `_candidates`).
    seen: HashSet<SharedString>,
    /// The focused suggestion.
    focused: Option<usize>,
    /// The text last put in the input by the box itself, not the user.
    programmatic: Option<String>,
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
        // Monaco's rename input uses the editor's font (`applyFontInfo`),
        // unlike the other widget inputs.
        let font = self.editor.upgrade().map(|editor| {
            let options = &editor.read(cx).options;
            (options.font_family.clone(), options.font_size)
        });
        let input = cx.new(|cx| {
            let mut editor = CodeEditor::new_widget_input(&name, theme, window, cx);
            if let Some((family, size)) = font {
                let mut options = editor.options().clone();
                options.font_family = family;
                options.font_size = size;
                editor.update_options(options, cx);
            }
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
        self.programmatic = Some(name);
        // Monaco cancels when focus leaves the editor and the box.
        self._subscriptions.push(
            cx.subscribe(&input, |this, _, event: &CodeEditorEvent, cx| match event {
                CodeEditorEvent::Blurred => {
                    this.with_editor(cx, |editor, cx| {
                        editor.cancel_rename(false, cx);
                    });
                }
                CodeEditorEvent::SelectionsChanged(_) | CodeEditorEvent::MouseDown(_) => {
                    this.input_changed(cx)
                }
                _ => {}
            }),
        );
        let model = input.read(cx).model().clone();
        self._subscriptions
            .push(cx.subscribe(&model, |this, _, event: &TextModelEvent, cx| {
                if matches!(event, TextModelEvent::Edited { .. }) {
                    this.input_changed(cx);
                }
            }));
        self.input = Some(input);
    }

    /// Runs `f` on the editor once this update is done (the editor reads
    /// the box).
    fn with_editor(
        &self,
        cx: &mut Context<Self>,
        f: impl FnOnce(&mut CodeEditor, &mut Context<CodeEditor>) + 'static,
    ) {
        let editor = self.editor.clone();
        cx.defer(move |cx| {
            editor.update(cx, f).ok();
        });
    }

    fn input_text(&self, cx: &gpui::App) -> String {
        self.input
            .as_ref()
            .map(|input| input.read(cx).model().read(cx).text())
            .unwrap_or_else(|| self.name.clone())
    }

    /// Puts `text` in the input with the cursor after it (Monaco's
    /// `input.value = …`, which isn't an input event).
    fn set_input_text(&mut self, text: &str, cx: &mut Context<Self>) {
        self.programmatic = Some(text.to_string());
        let Some(input) = &self.input else {
            return;
        };
        input.update(cx, |editor, cx| {
            editor
                .model()
                .clone()
                .update(cx, |model, cx| model.set_value(text, cx));
            let end = PointUtf16::new(0, text.encode_utf16().count() as u32);
            editor.set_selections(&[Selection::collapsed(end)], cx);
        });
    }

    /// Monaco's `onDidInputChange` (typing, Left/Right, clicks): stops
    /// asking for suggestions and unfocuses the focused one.
    fn input_changed(&mut self, cx: &mut Context<Self>) {
        let text = self.input_text(cx);
        if self.programmatic.as_deref() == Some(text.as_str()) {
            return;
        }
        self.programmatic = None;
        if self.requesting {
            self.requesting = false;
            self.with_editor(cx, |editor, _| {
                if let Some(session) = &mut editor.rename.session {
                    session.candidates = None;
                }
            });
        }
        self.focused = None;
        cx.notify();
    }

    /// The name to rename to: the focused suggestion or the input (Monaco
    /// `_currentAcceptInput`).
    fn new_name(&self, cx: &gpui::App) -> String {
        match self.focused.and_then(|ix| self.candidates.get(ix)) {
            Some(candidate) => candidate.new_symbol_name.to_string(),
            None => self.input_text(cx),
        }
    }

    /// Monaco `RenameCandidateListView.focusNext` / `focusPrevious`, then
    /// the input shows the focused suggestion, or the current name when
    /// focus moved off the list.
    fn move_focus(&mut self, forward: bool, cx: &mut Context<Self>) {
        let len = self.candidates.len();
        self.focused = match (self.focused, forward) {
            _ if len == 0 => None,
            (None, true) => Some(0),
            (None, false) => Some(len - 1),
            (Some(ix), true) if ix + 1 < len => Some(ix + 1),
            (Some(ix), false) if ix > 0 => Some(ix - 1),
            (Some(_), _) => None,
        };
        // Monaco `focusNext`/`focusPrevious` reveal the focused candidate,
        // or the first when focus leaves the list.
        self.scroll.scroll_to_item(self.focused.unwrap_or(0));
        let text = match self.focused {
            Some(ix) => self.candidates[ix].new_symbol_name.to_string(),
            None => self.name.clone(),
        };
        self.set_input_text(&text, cx);
        cx.notify();
    }

    /// Monaco `_updateRenameCandidates`: adds new, distinct, non-blank
    /// suggestions that aren't the current name or the input, first.
    fn add_candidates(&mut self, names: Vec<NewSymbolName>, cx: &mut Context<Self>) {
        self.requesting = false;
        let input = self.input_text(cx);
        let mut added: Vec<NewSymbolName> = Vec::new();
        for name in names {
            let text = &name.new_symbol_name;
            if text.trim().is_empty()
                || text.as_ref() == input
                || text.as_ref() == self.name
                || self.seen.contains(text)
                || added.iter().any(|n| &n.new_symbol_name == text)
            {
                continue;
            }
            added.push(name);
        }
        for name in &added {
            self.seen.insert(name.new_symbol_name.clone());
        }
        if added.is_empty() {
            cx.notify();
            return;
        }
        // Monaco `setCandidates` splices them in at the top.
        if let Some(focused) = &mut self.focused {
            *focused += added.len();
        }
        added.append(&mut self.candidates);
        self.candidates = added;
        cx.notify();
    }

    /// Monaco `RenameCandidateView.getLayoutInfo`: a line plus 2px padding
    /// above and below.
    fn candidate_height(&self) -> gpui::Pixels {
        self.line_height + px(CANDIDATE_PADDING * 2.)
    }

    /// Monaco `getPosition`: below the line when there's room there for
    /// about six candidates, else above.
    fn below_first(&self) -> bool {
        self.room
            .get()
            .is_none_or(|room| room.below > self.candidate_height() * 6.)
    }

    /// Monaco `_pickListHeight`: every candidate, at most seven and the
    /// space left on the side the box shows on (less the input and label).
    fn list_height(&self, label_height: gpui::Pixels) -> gpui::Pixels {
        let row = self.candidate_height();
        let all = row * self.candidates.len() as f32;
        let available = match self.room.get() {
            Some(room) if self.below_first() => room.below,
            Some(room) => room.above,
            None => all,
        } - label_height
            - (self.line_height + px(INPUT_PADDING * 2. + 2.));
        all.min(row * MAX_VISIBLE_CANDIDATES as f32)
            .min(available.max(row))
    }

    /// Monaco `input.size = max(nameLength * 1.1, 20)` characters, at the
    /// editor's typical character width.
    fn input_width(&self) -> gpui::Pixels {
        let length = self.location_length as f32;
        self.char_width * (length * 1.1).max(20.).ceil()
    }

    /// Monaco `_pickListWidth`: room for the longest candidate after the
    /// icon, at least the input's width.
    fn list_width(&self, input: gpui::Pixels) -> gpui::Pixels {
        let longest = self
            .candidates
            .iter()
            .map(|c| c.new_symbol_name.chars().count())
            .max()
            .unwrap_or(0);
        input.max(px(4. + 16. + 5. + 10.) + (self.char_width * longest as f32).ceil())
    }

    fn render_candidates(&self, height: gpui::Pixels, cx: &mut Context<Self>) -> gpui::AnyElement {
        let colors = &self.colors;
        let row_height = self.candidate_height();
        let rows = self.candidates.iter().enumerate().map(|(ix, candidate)| {
            let ai = candidate.tags.contains(&NewSymbolNameTag::AIGenerated);
            let focused = self.focused == Some(ix);
            div()
                .id(("rename-candidate", ix))
                .h(row_height)
                .flex_none()
                .p(px(CANDIDATE_PADDING))
                .flex()
                .items_center()
                .gap(px(5.))
                .rounded(px(2.))
                .cursor_pointer()
                .whitespace_nowrap()
                // The list's focus colors (`quickInputList.focus*`, its
                // inactive focus) and `list.hoverBackground`.
                .when(focused, |row| {
                    colors.focused_list_row(
                        row,
                        colors.quick_input_list_focus_background,
                        colors.quick_input_list_focus_foreground,
                    )
                })
                .when(!focused, |row| {
                    let hover = colors.list_hover_background;
                    row.hover(move |style| match hover {
                        Some(hover) => style.bg(hover),
                        None => style,
                    })
                })
                .child(
                    div()
                        .size(self.line_height * 0.8)
                        .flex_none()
                        .flex()
                        .items_center()
                        .when(ai, |icon| {
                            icon.child(codicon(Codicon::Sparkle).size(self.line_height * 0.8))
                        }),
                )
                .child(candidate.new_symbol_name.clone())
                .on_click(cx.listener(move |this, _, _, cx| {
                    // Monaco's list selection: pick it and rename, without
                    // a preview.
                    this.focused = Some(ix);
                    let name = this.new_name(cx);
                    this.with_editor(cx, |editor, cx| editor.accept_rename_impl(name, false, cx));
                }))
        });
        div()
            .id("rename-candidates")
            .h(height)
            .overflow_y_scroll()
            .track_scroll(&self.scroll)
            .flex()
            .flex_col()
            .children(rows)
            .into_any_element()
    }
}

/// The label under the box (Monaco's `rename-label`, shown with previews):
/// "{accept} to Rename, {preview} to Preview" with their keys.
fn rename_label(accept: &str, preview: &str) -> String {
    format!("{accept} to Rename, {preview} to Preview")
}

/// Monaco `RenameCandidateView._PADDING`.
const CANDIDATE_PADDING: f32 = 2.;
/// `.rename-input-with-button { padding: 3px }`.
const INPUT_PADDING: f32 = 3.;

impl Render for RenameInput {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.ensure_input(window, cx);
        // Monaco `getPosition`'s preference, once the room is known.
        if !self.placed && self.room.get().is_some() {
            self.placed = true;
            if !self.below_first() {
                self.with_editor(cx, |editor, cx| editor.prefer_rename_above(cx));
            }
        }
        let colors = self.colors.clone();
        let label_size = self.font_size * 0.8;
        let label_height = if self.support_preview {
            label_size * 1.4
        } else {
            px(0.)
        };
        let button_width = if self.has_suggestions {
            px(16. + 6.)
        } else {
            px(0.)
        };
        let input_width = self.input_width();
        let box_width = (input_width + button_width + px(INPUT_PADDING * 2. + 2.)).max(px(200.));
        let list_width = self.list_width(box_width);
        let candidates = (!self.candidates.is_empty())
            .then(|| self.render_candidates(self.list_height(label_height), cx));
        // Monaco's `InputWithButton`: sparkle to ask, stop to cancel.
        let button = self.has_suggestions.then(|| {
            let (icon, title) = if self.requesting {
                (Codicon::StopCircle, "Cancel")
            } else {
                (Codicon::Sparkle, "Generate New Name Suggestions")
            };
            div()
                .id("rename-suggestions-button")
                .flex_none()
                .p(px(3.))
                .rounded(px(5.))
                .cursor_pointer()
                .hover(|style| colors.toolbar_hover(style))
                .child(codicon(icon))
                .tooltip(crate::tooltip::tooltip(title, &colors))
                .on_click(cx.listener(|this, _, _, cx| {
                    this.with_editor(cx, |editor, cx| editor.request_rename_suggestions(cx));
                }))
        });
        let focused = self
            .input
            .as_ref()
            .is_some_and(|input| input.read(cx).focus_handle.is_focused(window));
        // The label names the accept and preview keys.
        let label = self.support_preview.then(|| {
            let keys = |action: &dyn gpui::Action| {
                self.input.as_ref().and_then(|input| {
                    window
                        .highest_precedence_binding_for_action_in(
                            action,
                            &input.read(cx).focus_handle,
                        )
                        .map(|binding| crate::keybinding_label::binding_label(&binding))
                })
            };
            let accept = keys(&AcceptRenameInput).unwrap_or_else(|| "Enter".into());
            let preview = keys(&AcceptRenameInputWithPreview).unwrap_or_default();
            rename_label(&accept, &preview)
        });
        let shadow = colors.widget_shadow.map(|color| gpui::BoxShadow {
            color,
            offset: gpui::point(px(0.), px(0.)),
            blur_radius: px(8.),
            spread_radius: px(2.),
            inset: false,
        });
        div()
            // With the editor's keys, for its preconditions.
            .key_context(crate::context_keys::widget_key_context(
                RENAME_INPUT_KEY_CONTEXT,
                &crate::context_keys::editor_keys_for_widgets(&self.editor, cx),
            ))
            .on_action(cx.listener(|this, _: &AcceptRenameInput, _, cx| {
                this.with_editor(cx, |editor, cx| editor.accept_rename_input(false, cx));
            }))
            .on_action(
                cx.listener(|this, _: &AcceptRenameInputWithPreview, _, cx| {
                    this.with_editor(cx, |editor, cx| editor.accept_rename_input(true, cx));
                }),
            )
            .on_action(cx.listener(|this, _: &CancelRenameInput, _, cx| {
                this.with_editor(cx, |editor, cx| {
                    editor.cancel_rename(true, cx);
                });
            }))
            .on_action(
                cx.listener(|this, _: &FocusNextRenameSuggestion, _, cx| this.move_focus(true, cx)),
            )
            .on_action(
                cx.listener(|this, _: &FocusPreviousRenameSuggestion, _, cx| {
                    this.move_focus(false, cx)
                }),
            )
            // Monaco `.rename-box`: 4px corners, `editorWidget.background`,
            // `widget.border`, `0 0 8px 2px widget.shadow` and
            // `input.foreground`; `4px 4px 0` padding with the label.
            .w(list_width.max(box_width) + if self.support_preview { px(8.) } else { px(0.) })
            .when(self.support_preview, |this| this.pt(px(4.)).px(px(4.)))
            .flex()
            .flex_col()
            .bg(colors.widget_background)
            .when_some(colors.widget_outline, |this, border| {
                this.border_1().border_color(border)
            })
            .when_some(shadow, |this, shadow| this.shadow(vec![shadow]))
            .rounded(px(4.))
            .text_size(self.font_size)
            .text_color(colors.input_foreground)
            .child(
                div()
                    .w(box_width)
                    .flex()
                    .items_center()
                    .p(px(INPUT_PADDING))
                    .rounded(px(4.))
                    .bg(colors.input_background)
                    .border_1()
                    // `input.border`, and the `focusBorder` outline while
                    // the input has focus.
                    .border_color(if focused {
                        colors.focus_border
                    } else {
                        colors.input_border.unwrap_or_else(gpui::transparent_black)
                    })
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .h(self.line_height)
                            .children(self.input.clone()),
                    )
                    .children(button),
            )
            .children(candidates.map(|list| div().w(list_width).child(list)))
            .children(label.map(|label| {
                div()
                    .h(label_height)
                    .flex()
                    .items_center()
                    .whitespace_nowrap()
                    .text_size(label_size)
                    .opacity(0.8)
                    .child(label)
            }))
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
        self.start_cancellable(crate::editor_state::CancellableOperation::Rename);
        self.rename.task = Some(cx.spawn(async move |this, cx| {
            // Monaco `RenameSkeleton.resolveRenameLocation`: one provider
            // at a time, best first, until one answers or one doesn't
            // resolve locations.
            let mut rejects: Vec<String> = Vec::new();
            let mut resolved = None;
            for (ix, provider) in providers.iter().enumerate() {
                let Ok(Some(task)) = this.update(cx, |_, cx| {
                    provider.resolve_rename_location(&context, position, cx)
                }) else {
                    break;
                };
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
        // Monaco `RenameWidget._show`: the symbol's line is revealed first,
        // so the box shows instead of being cancelled as out of view.
        self.reveal_line_in_center_if_outside_viewport(location.range.start.row, cx);
        let mut theme = (*self.theme).clone();
        theme.colors.background = theme.colors.input_background;
        theme.colors.gutter_background = theme.colors.input_background;
        let editor = cx.entity().downgrade();
        let has_suggestions = !new_symbol_names_providers(cx, self.provider_target(cx)).is_empty();
        let support_preview = crate::has_workspace_edit_preview_handler(cx);
        let (line_height, char_width) = self.layout.as_ref().map_or_else(
            || {
                (
                    self.options.resolved_line_height(),
                    self.options.font_size * 0.6,
                )
            },
            |layout| (layout.metrics.line_height, layout.metrics.em_width),
        );
        let location_length = location
            .range
            .end
            .column
            .saturating_sub(location.range.start.column);
        let view = cx.new(|_| RenameInput {
            editor,
            name: location.text.to_string(),
            selection,
            input: None,
            theme: Arc::new(theme),
            colors: self.theme.colors.clone(),
            font_size: self.options.font_size,
            line_height,
            char_width,
            location_length,
            room: Default::default(),
            placed: false,
            scroll: gpui::ScrollHandle::new(),
            support_preview,
            has_suggestions,
            requesting: false,
            candidates: Vec::new(),
            seen: HashSet::new(),
            focused: None,
            programmatic: None,
            _subscriptions: Vec::new(),
        });
        let widget = self.add_content_widget(
            ContentWidget {
                allow_editor_overflow: true,
                ..ContentWidget::new(
                    location.range.start,
                    vec![
                        ContentWidgetPreference::Below,
                        ContentWidgetPreference::Above,
                    ],
                    view.clone().into(),
                )
            },
            cx,
        );
        self.widgets
            .set_layer(widget, crate::widgets::layer::WIDGET);
        let room = self.widgets.room(widget);
        view.update(cx, |view, _| view.room = room);
        self.rename.session = Some(Session {
            widget,
            view,
            location,
            position,
            provider,
            candidates: None,
        });
        // Monaco asks providers that support it right away.
        if has_suggestions {
            self.request_rename_candidates(false, cx);
        }
    }

    /// Asks the [`NewSymbolNamesProvider`](crate::providers::NewSymbolNamesProvider)s
    /// for names for the symbol being renamed (Monaco
    /// `RenameWidget._requestRenameCandidates`): all of them when
    /// `manual` (the box's button), else those supporting automatic
    /// requests. While a request runs, the button cancels it instead.
    fn request_rename_candidates(&mut self, manual: bool, cx: &mut Context<Self>) {
        let target = self.provider_target(cx);
        let Some(session) = &mut self.rename.session else {
            return;
        };
        let view = session.view.clone();
        // A request in flight: the stop button cancels it.
        if session.candidates.take().is_some() && view.read(cx).requesting {
            view.update(cx, |view, cx| {
                view.requesting = false;
                cx.notify();
            });
            return;
        }
        let trigger = if manual {
            NewSymbolNameTriggerKind::Invoke
        } else {
            NewSymbolNameTriggerKind::Automatic
        };
        let providers: Vec<_> = new_symbol_names_providers(cx, target)
            .into_iter()
            .filter(|provider| {
                manual || provider.supports_automatic_new_symbol_names_trigger_kind()
            })
            .collect();
        if providers.is_empty() {
            return;
        }
        let range = session.location.range.clone();
        let context = self.model().read(cx).provider_context();
        let tasks: Vec<_> = providers
            .iter()
            .map(|provider| provider.provide_new_symbol_names(&context, range.clone(), trigger, cx))
            .collect();
        view.update(cx, |view, cx| {
            view.requesting = true;
            cx.notify();
        });
        let task = cx.spawn(async move |_, cx| {
            let mut names = Vec::new();
            for task in tasks {
                match task.await {
                    Ok(found) => names.extend(found),
                    Err(err) => log::warn!("rename suggestions failed: {err:#}"),
                }
            }
            view.update(cx, |view, cx| view.add_candidates(names, cx));
        });
        if let Some(session) = &mut self.rename.session {
            session.candidates = Some(task);
        }
    }

    /// Clicks the rename box's suggestions button (Monaco's "Generate New
    /// Name Suggestions"): asks every
    /// [`NewSymbolNamesProvider`](crate::providers::NewSymbolNamesProvider)
    /// for names, or cancels the request in flight.
    pub fn request_rename_suggestions(&mut self, cx: &mut Context<Self>) {
        self.request_rename_candidates(true, cx);
    }

    /// Whether the rename box is showing.
    pub fn is_rename_visible(&self) -> bool {
        self.rename.session.is_some()
    }

    /// Whether the rename box's input has the focus (Monaco
    /// `renameInputFocused`).
    pub(crate) fn is_rename_input_focused(&self, window: &Window, cx: &gpui::App) -> bool {
        self.rename.session.as_ref().is_some_and(|session| {
            session.view.read(cx).input.as_ref().is_some_and(|input| {
                gpui::Focusable::focus_handle(input.read(cx), cx).is_focused(window)
            })
        })
    }

    /// The rename box's suggestions, newest first (from
    /// [`NewSymbolNamesProvider`](crate::providers::NewSymbolNamesProvider)s).
    pub fn rename_suggestions(&self, cx: &gpui::App) -> Vec<SharedString> {
        self.rename
            .session
            .as_ref()
            .map(|session| {
                session
                    .view
                    .read(cx)
                    .candidates
                    .iter()
                    .map(|c| c.new_symbol_name.clone())
                    .collect()
            })
            .unwrap_or_default()
    }

    /// The rename box's input text and the focused suggestion's index, for
    /// tests.
    #[doc(hidden)]
    pub fn rename_input_for_tests(&self, cx: &gpui::App) -> Option<(String, Option<usize>)> {
        let session = self.rename.session.as_ref()?;
        let view = session.view.read(cx);
        Some((view.input_text(cx), view.focused))
    }

    /// Whether the rename box shows its "Enter to Rename, Cmd/Ctrl+Enter to
    /// Preview" label and suggestions button, for tests.
    #[doc(hidden)]
    pub fn rename_box_extras_for_tests(&self, cx: &gpui::App) -> Option<(bool, bool)> {
        let session = self.rename.session.as_ref()?;
        let view = session.view.read(cx);
        Some((view.support_preview, view.has_suggestions))
    }

    /// The input's width (Monaco's `size` in characters) and whether the
    /// focused suggestion is inside the list's visible part, as last laid
    /// out, for tests.
    #[doc(hidden)]
    pub fn rename_layout_for_tests(&self, cx: &gpui::App) -> Option<(gpui::Pixels, bool)> {
        let view = self.rename.session.as_ref()?.view.read(cx);
        let list = view.scroll.bounds();
        let offset = view.scroll.offset().y;
        let visible = view.focused.is_some_and(|ix| {
            view.scroll.bounds_for_item(ix).is_some_and(|item| {
                // Item bounds are unscrolled.
                let top = item.top() + offset;
                top >= list.top() - px(0.5) && top + item.size.height <= list.bottom() + px(0.5)
            })
        });
        Some((view.input_width(), visible))
    }

    /// Focuses the next rename suggestion, showing it in the box (Monaco
    /// `focusNextRenameSuggestion`, Down). Past the last one, or with none,
    /// the box shows the current name again.
    pub fn focus_next_rename_suggestion(&mut self, cx: &mut Context<Self>) {
        if let Some(session) = &self.rename.session {
            session
                .view
                .clone()
                .update(cx, |view, cx| view.move_focus(true, cx));
        }
    }

    /// Focuses the previous rename suggestion (Monaco
    /// `focusPreviousRenameSuggestion`, Up).
    pub fn focus_previous_rename_suggestion(&mut self, cx: &mut Context<Self>) {
        if let Some(session) = &self.rename.session {
            session
                .view
                .clone()
                .update(cx, |view, cx| view.move_focus(false, cx));
        }
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

    /// Monaco `getPosition` with little room below: the box goes above the
    /// line first.
    pub(crate) fn prefer_rename_above(&mut self, cx: &mut Context<Self>) {
        let Some(session) = &self.rename.session else {
            return;
        };
        let id = session.widget;
        if let Some(widget) = self.widgets.content.iter_mut().find(|w| w.id == id) {
            widget.preference = vec![
                ContentWidgetPreference::Above,
                ContentWidgetPreference::Below,
            ];
            cx.notify();
        }
    }

    /// Whether the rename box's line was scrolled out of the laid out
    /// lines, so it isn't rendered (Monaco `afterRender(null)`).
    pub(crate) fn rename_out_of_view(&self, cx: &gpui::App) -> bool {
        let (Some(session), Some(layout)) = (&self.rename.session, &self.layout) else {
            return false;
        };
        // The widget's anchor follows edits.
        let anchor = self
            .widgets
            .content
            .iter()
            .find(|w| w.id == session.widget)
            .and_then(|w| {
                let buffer = self.model().read(cx).buffer();
                Some(buffer.offset_to_point_utf16(buffer.resolve(w.anchor.as_ref()?)))
            })
            .unwrap_or(session.location.range.start);
        layout.line_for_point(anchor).is_none()
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

    /// Enter in the rename box: renames to the focused suggestion or the
    /// typed name (Monaco `acceptRenameInput`), or with `wants_preview`
    /// (Cmd/Ctrl+Enter, Monaco `acceptRenameInputWithPreview`) previews
    /// the edits first when a
    /// [preview handler](crate::set_workspace_edit_preview_handler) is set.
    pub fn accept_rename_input(&mut self, wants_preview: bool, cx: &mut Context<Self>) {
        let Some(session) = &self.rename.session else {
            return;
        };
        let name = session.view.read(cx).new_name(cx);
        self.accept_rename_impl(name, wants_preview, cx);
    }

    /// Renames to `new_name`, as if typed in the rename box and Enter
    /// pressed: asks the providers for edits and applies them (Monaco
    /// `RenameController.run`, part 2).
    pub fn accept_rename(&mut self, new_name: String, cx: &mut Context<Self>) {
        self.accept_rename_impl(new_name, false, cx);
    }

    fn accept_rename_impl(
        &mut self,
        new_name: String,
        wants_preview: bool,
        cx: &mut Context<Self>,
    ) {
        let Some(session) = self.rename.session.take() else {
            return;
        };
        let wants_preview = wants_preview && session.view.read(cx).support_preview;
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
        let position = session.position;
        let first = session.provider;
        let label: SharedString =
            format!("Renaming '{}' to '{new_name}'", session.location.text).into();
        let old_name = session.location.text.clone();
        self.start_cancellable(crate::editor_state::CancellableOperation::Rename);
        self.rename.task = Some(cx.spawn(async move |this, cx| {
            // Monaco `RenameSkeleton._provideRenameEdits`: one provider at a
            // time, from the one that resolved the location.
            let mut rejects: Vec<String> = Vec::new();
            let mut edit = None;
            for provider in providers.iter().skip(first) {
                let Ok(task) = this.update(cx, |_, cx| {
                    provider.provide_rename_edits(&context, position, &new_name, cx)
                }) else {
                    return;
                };
                match task.await {
                    Ok(Some(result)) => {
                        edit = Some(result);
                        break;
                    }
                    Ok(None) => rejects.push("No result.".into()),
                    Err(err) => rejects.push(err.to_string()),
                }
            }
            // Monaco's standalone bulk edit summary.
            let summary = edit.as_ref().map(|edit| {
                let edits: usize = edit.edits.iter().map(|(_, edits)| edits.len()).sum();
                format!("Made {edits} edits in {} files", edit.edits.len())
            });
            // `rename.task` is this task: it stays until the edits are
            // applying. Applying them isn't cancelled (Monaco's bulk edit
            // has no token), and their own text change would cancel it.
            let Ok(Some(apply)) = this.update(cx, |this, cx| {
                if this.rename_state(cx) != state {
                    return None;
                }
                let Some(edit) = edit else {
                    this.show_message(rejects.join("\n"), position, cx);
                    return None;
                };
                // Collapse the selection to its active end, then apply.
                let head = this.cursors.primary().position;
                this.set_selections(&[Selection::collapsed(head)], cx);
                Some(this.apply_workspace_edit_with_preview(edit, label, wants_preview, cx))
            }) else {
                return;
            };
            let Ok(()) = this.update(cx, |_, cx| {
                cx.spawn(async move |this, cx| match apply.await {
                    Err(err) => {
                        log::warn!("rename failed to apply edits: {err:#}");
                        this.update(cx, |this, cx| {
                            this.show_message("Rename failed to apply edits", position, cx)
                        })
                        .ok();
                    }
                    Ok(false) => {}
                    // Monaco alerts screen readers once the edits are in.
                    Ok(true) => {
                        if let Some(summary) = summary {
                            this.update(cx, |this, cx| {
                                this.announce(
                                    format!(
                                        "Successfully renamed '{old_name}' to '{new_name}'. Summary: {summary}"
                                    ),
                                    cx,
                                )
                            })
                            .ok();
                        }
                    }
                })
                .detach()
            }) else {
                return;
            };
        }));
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
