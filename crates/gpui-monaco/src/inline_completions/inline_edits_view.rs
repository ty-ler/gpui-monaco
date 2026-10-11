//! The inline edit views (Monaco `view/inlineEdits`): the chosen view of
//! the shown inline edit, drawn with decorations, content widgets and view
//! zones, and the gutter indicator with its menu.
//!
//! Ported from Monaco / VS Code (MIT License, Copyright (c) Microsoft
//! Corporation), `vs/editor/contrib/inlineCompletions/browser/view/inlineEdits/`:
//! - `inlineEditsView.ts` (`InlineEditsView`: the view cache and the
//!   decorations of `originalEditorInlineDiffView.ts`),
//! - `inlineEditsViews/*`: word replacement, line replacement, insertion,
//!   deletion, side by side and collapsed views, as boxes in content widgets
//!   rather than DOM overlays,
//! - `components/gutterIndicatorView.ts` and `gutterIndicatorMenu.ts`.

use std::sync::Arc;
use std::time::Duration;

use gpui::prelude::FluentBuilder as _;
use gpui::{
    AppContext as _, Context, Entity, Hsla, InteractiveElement, IntoElement, MouseButton,
    ParentElement, Pixels, Render, SharedString, StatefulInteractiveElement, Styled, Task,
    WeakEntity, Window, div, point, px,
};
use text::PointUtf16;
use web_time::Instant;

use super::inline_edits::{
    InlineEditDiff, InlineEditViewKind, ViewSettings, determine_view, grow_edits_to_entire_word,
    grow_edits_until_whitespace, utf16_to_byte,
};
use crate::decorations::{BlockDecoration, DecorationId, DecorationOptions, InjectedText};
use crate::editor::CodeEditor;
use crate::providers::Command;
use crate::theme::{Theme, TokenStyle};
use crate::widgets::{
    ContentWidget, ContentWidgetPreference, GlyphMarginWidget, OverlayPosition, OverlayWidget,
    ViewZone, WidgetId, ZoneHeight,
};

/// Monaco `ORIGINAL_END_PADDING`: the gap between the text and the side by
/// side card.
const ORIGINAL_END_PADDING: f32 = 20.;
/// Monaco `MODIFIED_END_PADDING` plus the card's borders.
const MODIFIED_END_PADDING: f32 = 12. + 2.;
/// Monaco: clicks on a view accept only once it showed this long.
const CLICK_DELAY: Duration = Duration::from_millis(350);

/// What Tab does (Monaco `InlineEditTabAction`); it colors the indicator.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TabAction {
    /// Tab jumps to the edit.
    Jump,
    /// Tab accepts it.
    Accept,
    /// Tab does neither.
    Inactive,
}

/// Where the gutter indicator is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum IndicatorPlacement {
    /// In the glyph margin beside this row.
    Docked(u32),
    /// Pinned to the top of the editor: the edit is above.
    Above,
    /// Pinned to the bottom: the edit is below.
    Below,
}

/// An entry of the gutter indicator's menu (Monaco `GutterIndicatorMenuContent`).
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum GutterMenuAction {
    /// "Go To / Accept" (`editor.action.inlineSuggest.commit`).
    Accept,
    /// "Reject" (`editor.action.inlineSuggest.hide`).
    Reject,
    /// "Show Collapsed" or "Show Expanded"
    /// (`editor.action.inlineSuggest.toggleShowCollapsed`).
    ToggleShowCollapsed,
    /// One of the list's commands.
    Command(Command),
    /// The completion's `gutter_menu_link_action`, at the bottom.
    Link(Command),
}

/// A menu entry: its title, keybinding hint and action.
type GutterMenuItem = (SharedString, Option<&'static str>, GutterMenuAction);

/// A piece of a previewed line.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Segment {
    text: String,
    color: Hsla,
    background: Option<Hsla>,
}

/// What is rendered, to redo it only when it changes.
#[derive(Clone, PartialEq)]
struct RenderKey {
    version: text::Version,
    id: u64,
    kind: InlineEditViewKind,
}

/// The rendered inline edit.
struct Rendered {
    key: RenderKey,
    decorations: Vec<DecorationId>,
    widgets: Vec<WidgetId>,
}

/// The inline edit view's state in the editor.
#[derive(Default)]
pub(crate) struct InlineEditViewState {
    rendered: Option<Rendered>,
    /// Monaco `_previousView`: the item, its view and the editor width the
    /// view was chosen for.
    previous: Option<(u64, InlineEditViewKind, Pixels)>,
    /// When the current view first showed.
    shown_at: Option<(u64, Instant)>,
    indicator: Option<Entity<GutterIndicator>>,
    indicator_widget: Option<(WidgetId, IndicatorPlacement)>,
}

/// How an [`InlineEditBox`] is drawn.
pub(crate) struct BoxStyle {
    background: Option<Hsla>,
    border: Option<Hsla>,
    /// Left and top margins.
    margin: (Pixels, Pixels),
    /// Horizontal padding.
    padding: Pixels,
}

/// One previewed text box: the new word, lines or card.
pub(crate) struct InlineEditBox {
    editor: WeakEntity<CodeEditor>,
    lines: Vec<Vec<Segment>>,
    line_backgrounds: Vec<Option<Hsla>>,
    font: SharedString,
    font_size: Pixels,
    line_height: Pixels,
    style: BoxStyle,
}

impl Render for InlineEditBox {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let editor = self.editor.clone();
        div()
            .id("inline-edit-box")
            .ml(self.style.margin.0)
            .mt(self.style.margin.1)
            .font_family(self.font.clone())
            .text_size(self.font_size)
            .line_height(self.line_height)
            .px(self.style.padding)
            .cursor_pointer()
            .when_some(self.style.background, |this, background| {
                this.bg(background)
            })
            .when_some(self.style.border, |this, border| {
                this.border_1().border_color(border).rounded(px(3.))
            })
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |_, _, _, cx| {
                    cx.stop_propagation();
                    if let Some(editor) = editor.upgrade() {
                        editor.update(cx, |editor, cx| editor.inline_edit_view_clicked(cx));
                    }
                }),
            )
            .children(self.lines.iter().zip(&self.line_backgrounds).map(
                |(segments, background)| {
                    div()
                        .h(self.line_height)
                        .flex()
                        .whitespace_nowrap()
                        .when_some(*background, |this, background| this.bg(background))
                        .children(segments.iter().map(|segment| {
                            div()
                                .text_color(segment.color)
                                .when_some(segment.background, |this, background| {
                                    this.bg(background)
                                })
                                .child(segment.text.clone())
                        }))
                },
            ))
    }
}

/// Monaco `InlineEditsCollapsedView`: a small triangle at the edit.
pub(crate) struct CollapsedMarker {
    color: Hsla,
}

impl Render for CollapsedMarker {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .ml(px(-3.))
            .text_size(px(7.))
            .line_height(px(5.))
            .text_color(self.color)
            .child("▼")
    }
}

/// Monaco `InlineEditsGutterIndicator`: a pill in the gutter beside the
/// edit, or pinned to the top or bottom while the edit is scrolled away.
/// Clicking it accepts the edit (jumps to it when pinned); hovering it
/// shows the menu.
pub(crate) struct GutterIndicator {
    editor: WeakEntity<CodeEditor>,
    theme: Arc<Theme>,
    tab_action: TabAction,
    placement: IndicatorPlacement,
    /// The cursor is at or above the edit's first row.
    cursor_above: bool,
    header: SharedString,
    menu: Vec<GutterMenuItem>,
    line_height: Pixels,
    hovered: bool,
    menu_hovered: bool,
    menu_open: bool,
    close: Option<Task<()>>,
}

impl GutterIndicator {
    fn hover_changed(&mut self, cx: &mut Context<Self>) {
        if self.hovered || self.menu_hovered {
            self.close = None;
            self.menu_open = true;
            cx.notify();
            return;
        }
        // Let the mouse move from the pill to the menu.
        self.close = Some(cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(Duration::from_millis(250))
                .await;
            this.update(cx, |this, cx| {
                if !this.hovered && !this.menu_hovered {
                    this.menu_open = false;
                    cx.notify();
                }
            })
            .ok();
        }));
    }

    fn icon(&self) -> &'static str {
        match self.placement {
            IndicatorPlacement::Docked(_) => {
                if self.hovered {
                    "✓"
                } else if self.tab_action == TabAction::Accept {
                    "⇥"
                } else if self.cursor_above {
                    "↓"
                } else {
                    "↑"
                }
            }
            _ if self.tab_action == TabAction::Accept => "⇥",
            IndicatorPlacement::Above => "↑",
            IndicatorPlacement::Below => "↓",
        }
    }
}

impl Render for GutterIndicator {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = &self.theme.colors;
        let [foreground, border, background] = match self.tab_action {
            TabAction::Jump => colors.inline_edit_gutter_indicator_primary(),
            TabAction::Accept => colors.inline_edit_gutter_indicator_successful(),
            TabAction::Inactive => colors.inline_edit_gutter_indicator_secondary(),
        };
        let editor = self.editor.clone();
        let docked = matches!(self.placement, IndicatorPlacement::Docked(_));
        let pill = div()
            .id("inline-edit-gutter-indicator")
            .when(docked, |this| this.size_full())
            .when(!docked, |this| this.w(px(22.)).h(self.line_height))
            .flex()
            .items_center()
            .justify_center()
            .rounded(px(4.))
            .border_1()
            .border_color(border)
            .bg(background)
            .text_color(foreground)
            .text_size(px(12.))
            .cursor_pointer()
            .on_hover(cx.listener(|this, hovered: &bool, _, cx| {
                this.hovered = *hovered;
                this.hover_changed(cx);
            }))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, _, _, cx| {
                    cx.stop_propagation();
                    this.menu_open = false;
                    if let Some(editor) = editor.upgrade() {
                        editor.update(cx, |editor, cx| {
                            editor.click_inline_edit_gutter_indicator(cx);
                        });
                    }
                }),
            )
            .child(self.icon());
        let menu = self.menu_open.then(|| {
            let mut menu = div()
                .id("inline-edit-gutter-menu")
                .absolute()
                .left_full()
                .top_0()
                .ml(px(4.))
                .p(px(4.))
                .min_w(px(180.))
                .flex()
                .flex_col()
                .occlude()
                .bg(colors.widget_background)
                .border_1()
                .border_color(colors.widget_border)
                .text_color(colors.widget_foreground)
                .text_size(px(13.))
                .shadow_md()
                .on_hover(cx.listener(|this, hovered: &bool, _, cx| {
                    this.menu_hovered = *hovered;
                    this.hover_changed(cx);
                }))
                .child(
                    div()
                        .px(px(4.))
                        .py(px(4.))
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .opacity(0.8)
                        .child(self.header.clone()),
                );
            for (ix, (title, keys, action)) in self.menu.iter().enumerate() {
                let separator = matches!(action, GutterMenuAction::Link(_))
                    || (matches!(action, GutterMenuAction::Command(_))
                        && !matches!(
                            self.menu.get(ix.wrapping_sub(1)),
                            Some((_, _, GutterMenuAction::Command(_)))
                        ));
                if separator {
                    menu = menu.child(div().my(px(4.)).h(px(1.)).bg(colors.widget_border));
                }
                let editor = self.editor.clone();
                let action = action.clone();
                menu = menu.child(
                    div()
                        .id(("inline-edit-gutter-menu-item", ix))
                        .flex()
                        .justify_between()
                        .gap(px(12.))
                        .px(px(6.))
                        .py(px(3.))
                        .rounded(px(3.))
                        .cursor_pointer()
                        .hover(|style| style.bg(colors.menu_selection_background))
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(move |this, _, _, cx| {
                                cx.stop_propagation();
                                this.menu_open = false;
                                cx.notify();
                                if let Some(editor) = editor.upgrade() {
                                    let action = action.clone();
                                    editor.update(cx, |editor, cx| {
                                        editor.run_inline_edit_gutter_menu_action(action, cx)
                                    });
                                }
                            }),
                        )
                        .child(title.clone())
                        .children(keys.map(|keys| div().opacity(0.6).child(keys))),
                );
            }
            menu
        });
        div().relative().size_full().child(pill).children(menu)
    }
}

impl CodeEditor {
    /// Monaco `tabAction`: what Tab does with the inline edit.
    fn inline_edit_tab_action(&self, cx: &gpui::App) -> TabAction {
        if !self.has_focus {
            TabAction::Inactive
        } else if self.tab_should_jump_to_inline_edit(cx) {
            TabAction::Jump
        } else if self.tab_should_accept_inline_edit(cx) {
            TabAction::Accept
        } else {
            TabAction::Inactive
        }
    }

    /// Monaco `getContentRenderWidth`: `line`'s width in typical characters.
    fn inline_edit_text_width(&self, line: &str, cx: &gpui::App) -> Pixels {
        let em = self
            .layout
            .as_ref()
            .map_or(self.options.font_size * 0.6, |layout| {
                layout.metrics.em_width
            });
        let tab_size = self.model().read(cx).options().tab_size.max(1) as f32;
        let tabs = line.matches('\t').count() as f32;
        let others = line.encode_utf16().count() as f32 - tabs;
        em * (others + tabs * tab_size)
    }

    /// Monaco `InlineEditsSideBySideView.fitsInsideViewport`.
    fn inline_edit_fits_side_by_side(&self, diff: &InlineEditDiff, cx: &gpui::App) -> bool {
        let Some(layout) = self.layout.as_ref() else {
            return true;
        };
        let rope = self.model().read(cx).buffer().rope().clone();
        let original = diff
            .display_range()
            .map(|row| self.inline_edit_text_width(&rope.line(row), cx))
            .fold(px(0.), Pixels::max);
        let modified = diff
            .new_lines
            .iter()
            .map(|line| self.inline_edit_text_width(line, cx))
            .fold(px(0.), Pixels::max);
        original + modified + px(ORIGINAL_END_PADDING) + px(MODIFIED_END_PADDING)
            < layout.text_bounds.size.width
    }

    /// The menu entries (Monaco `GutterIndicatorMenuContent`).
    pub(crate) fn inline_edit_gutter_menu(
        &self,
        cx: &gpui::App,
    ) -> (SharedString, Vec<GutterMenuItem>) {
        let Some((ix, _)) = self.inline_edit_state(cx) else {
            return (SharedString::default(), Vec::new());
        };
        let item = &self.inline.items[ix];
        let header = item
            .provider
            .display_name()
            .unwrap_or_else(|| "Inline Suggestion".into());
        let mut menu: Vec<GutterMenuItem> = vec![
            (
                "Go To / Accept".into(),
                Some("Tab"),
                GutterMenuAction::Accept,
            ),
            ("Reject".into(), Some("Escape"), GutterMenuAction::Reject),
            (
                if self.options.inline_suggest.edits.show_collapsed {
                    "Show Expanded".into()
                } else {
                    "Show Collapsed".into()
                },
                None,
                GutterMenuAction::ToggleShowCollapsed,
            ),
        ];
        for command in &item.commands {
            menu.push((
                command.title.clone(),
                None,
                GutterMenuAction::Command(command.clone()),
            ));
        }
        if let Some(link) = &item.completion.gutter_menu_link_action {
            menu.push((
                format!("{}...", link.title).into(),
                None,
                GutterMenuAction::Link(link.clone()),
            ));
        }
        (header, menu)
    }

    /// Runs a gutter menu entry.
    pub(crate) fn run_inline_edit_gutter_menu_action(
        &mut self,
        action: GutterMenuAction,
        cx: &mut Context<Self>,
    ) {
        match action {
            GutterMenuAction::Accept => {
                self.accept_inline_suggestion(cx);
            }
            GutterMenuAction::Reject => self.hide_inline_suggestion(cx),
            GutterMenuAction::ToggleShowCollapsed => self.toggle_inline_edits_show_collapsed(cx),
            GutterMenuAction::Command(command) | GutterMenuAction::Link(command) => {
                if let Err(err) = crate::commands::execute_command(cx, &command) {
                    log::warn!("{err:#}");
                }
            }
        }
    }

    /// The gutter menu's titles, for tests.
    #[doc(hidden)]
    pub fn inline_edit_gutter_menu_for_tests(&self, cx: &gpui::App) -> Vec<String> {
        let (header, menu) = self.inline_edit_gutter_menu(cx);
        std::iter::once(header.to_string())
            .chain(menu.into_iter().map(|(title, _, _)| title.to_string()))
            .collect()
    }

    /// Runs the gutter menu entry titled `title`, for tests.
    #[doc(hidden)]
    pub fn run_inline_edit_gutter_menu_item_for_tests(
        &mut self,
        title: &str,
        cx: &mut Context<Self>,
    ) {
        let (_, menu) = self.inline_edit_gutter_menu(cx);
        if let Some((_, _, action)) = menu.into_iter().find(|(t, _, _)| t == title) {
            self.run_inline_edit_gutter_menu_action(action, cx);
        }
    }

    /// The gutter indicator was clicked: it accepts the edit when beside it
    /// (Monaco shows a check mark there on hover), else jumps to it.
    pub fn click_inline_edit_gutter_indicator(&mut self, cx: &mut Context<Self>) {
        let docked = matches!(
            self.inline.edit_view.indicator_widget,
            Some((_, IndicatorPlacement::Docked(_)))
        );
        if docked {
            self.accept_inline_suggestion(cx);
        } else {
            self.jump_to_next_inline_edit(cx);
        }
    }

    /// A view was clicked: accepts once it has shown for a moment (Monaco
    /// `_viewHasBeenShownLongerThan(350)`).
    pub(crate) fn inline_edit_view_clicked(&mut self, cx: &mut Context<Self>) {
        if self
            .inline
            .edit_view
            .shown_at
            .is_some_and(|(_, at)| at.elapsed() >= CLICK_DELAY)
        {
            self.accept_inline_suggestion(cx);
        }
    }

    /// How the inline edit shows, for tests.
    #[doc(hidden)]
    pub fn inline_edit_view_for_tests(&self) -> Option<InlineEditViewKind> {
        self.inline.edit_view.rendered.as_ref().map(|r| r.key.kind)
    }

    /// Where the gutter indicator is, for tests: `Some(Ok(row))` beside a
    /// row, `Some(Err(true))` pinned at the top, `Some(Err(false))` at the
    /// bottom.
    #[doc(hidden)]
    pub fn inline_edit_gutter_indicator_for_tests(&self) -> Option<Result<u32, bool>> {
        self.inline
            .edit_view
            .indicator_widget
            .map(|(_, placement)| match placement {
                IndicatorPlacement::Docked(row) => Ok(row),
                IndicatorPlacement::Above => Err(true),
                IndicatorPlacement::Below => Err(false),
            })
    }

    /// Clears the inline edit's view and indicator.
    pub(crate) fn clear_inline_edit_view(&mut self, cx: &mut Context<Self>) {
        self.clear_inline_edit_rendering(cx);
        if let Some((id, _)) = self.inline.edit_view.indicator_widget.take() {
            self.remove_widget(id, cx);
        }
        self.inline.edit_view.indicator = None;
    }

    fn clear_inline_edit_rendering(&mut self, cx: &mut Context<Self>) {
        let Some(rendered) = self.inline.edit_view.rendered.take() else {
            return;
        };
        if !rendered.decorations.is_empty() {
            let owner = Some(cx.entity_id());
            self.model().update(cx, |model, cx| {
                model.delta_decorations(owner, &rendered.decorations, Vec::new(), cx)
            });
        }
        for id in rendered.widgets {
            self.remove_widget(id, cx);
        }
    }

    /// Renders the inline edit before painting (Monaco `InlineEditsView`).
    pub(crate) fn refresh_inline_edit(&mut self, cx: &mut Context<Self>) {
        let Some((ix, edits)) = self.inline_edit_state(cx) else {
            self.clear_inline_edit_view(cx);
            return;
        };
        let rope = self.model().read(cx).buffer().rope().clone();
        let Some(diff) = InlineEditDiff::new(&rope, edits) else {
            self.clear_inline_edit_view(cx);
            return;
        };
        let id = self.inline.items[ix].id;
        // Monaco reuses the view chosen for this item, unless the editor's
        // width changed for views that depend on it.
        let width = self
            .layout
            .as_ref()
            .map_or(px(0.), |layout| layout.bounds.size.width);
        let cached = self
            .inline
            .edit_view
            .previous
            .filter(|(previous, kind, previous_width)| {
                *previous == id
                    && (*previous_width == width
                        || !matches!(
                            kind,
                            InlineEditViewKind::SideBySide | InlineEditViewKind::LineReplacement
                        ))
            })
            .map(|(_, kind, _)| kind);
        let kind = match cached {
            Some(kind) => kind,
            None => {
                let settings = ViewSettings {
                    cursor: self.cursors.primary().position,
                    cursors: self.cursors.selections().iter().map(|s| s.head).collect(),
                    code_shifting: self.options.inline_suggest.edits.allow_code_shifting,
                    render_side_by_side: self.options.inline_suggest.edits.render_side_by_side,
                };
                determine_view(&diff, &settings, || {
                    self.inline_edit_fits_side_by_side(&diff, cx)
                })
            }
        };
        self.inline.edit_view.previous = Some((id, kind, width));
        let kind = if self.inline_edit_show_collapsed(cx) {
            InlineEditViewKind::Collapsed
        } else {
            kind
        };
        if self
            .inline
            .edit_view
            .shown_at
            .is_none_or(|(shown, _)| shown != id)
        {
            self.inline.edit_view.shown_at = Some((id, Instant::now()));
        }
        self.inline_edit_did_show(ix);
        self.refresh_inline_edit_indicator(&diff, cx);
        let key = RenderKey {
            version: self.model().read(cx).buffer().version(),
            id,
            kind,
        };
        if self.inline.edit_view.rendered.as_ref().map(|r| &r.key) == Some(&key) {
            return;
        }
        self.clear_inline_edit_rendering(cx);
        let (decorations, widgets) = self.render_inline_edit(&diff, kind, cx);
        let owner = Some(cx.entity_id());
        let decorations = self.model().update(cx, |model, cx| {
            model.delta_decorations(owner, &[], decorations, cx)
        });
        self.inline.edit_view.rendered = Some(Rendered {
            key,
            decorations,
            widgets,
        });
    }

    /// Places the gutter indicator: beside the edit's first shown row, or
    /// pinned to the top or bottom while that row is scrolled away.
    fn refresh_inline_edit_indicator(&mut self, diff: &InlineEditDiff, cx: &mut Context<Self>) {
        let rope = self.model().read(cx).buffer().rope().clone();
        let row = diff.display_range().start.min(rope.max_point().row);
        let (placement, line_height) = match self.layout.as_ref() {
            Some(layout) => {
                let shown = layout
                    .lines
                    .iter()
                    .any(|line| line.row == row && line.columns.start == 0);
                let first = layout.lines.first().map_or(0, |line| line.row);
                let placement = if shown {
                    IndicatorPlacement::Docked(row)
                } else if row < first {
                    IndicatorPlacement::Above
                } else {
                    IndicatorPlacement::Below
                };
                (placement, layout.metrics.line_height)
            }
            None => (
                IndicatorPlacement::Docked(row),
                self.options.resolved_line_height(),
            ),
        };
        let docked_without_margin = matches!(placement, IndicatorPlacement::Docked(_))
            && self
                .layout
                .as_ref()
                .is_some_and(|layout| layout.glyph_margin_width <= px(0.));
        let tab_action = self.inline_edit_tab_action(cx);
        let cursor_above = self.cursors.primary().position.row <= row;
        let (header, menu) = self.inline_edit_gutter_menu(cx);
        let indicator = match self.inline.edit_view.indicator.clone() {
            Some(indicator) => {
                indicator.update(cx, |indicator, cx| {
                    let changed = indicator.tab_action != tab_action
                        || indicator.placement != placement
                        || indicator.cursor_above != cursor_above
                        || indicator.menu != menu
                        || indicator.header != header;
                    indicator.tab_action = tab_action;
                    indicator.placement = placement;
                    indicator.cursor_above = cursor_above;
                    indicator.menu = menu;
                    indicator.header = header;
                    indicator.line_height = line_height;
                    if changed {
                        cx.notify();
                    }
                });
                indicator
            }
            None => {
                let editor = cx.entity().downgrade();
                let theme = self.theme.clone();
                let indicator = cx.new(|_| GutterIndicator {
                    editor,
                    theme,
                    tab_action,
                    placement,
                    cursor_above,
                    header,
                    menu,
                    line_height,
                    hovered: false,
                    menu_hovered: false,
                    menu_open: false,
                    close: None,
                });
                self.inline.edit_view.indicator = Some(indicator.clone());
                indicator
            }
        };
        if self
            .inline
            .edit_view
            .indicator_widget
            .is_some_and(|(_, current)| current == placement)
        {
            return;
        }
        if let Some((id, _)) = self.inline.edit_view.indicator_widget.take() {
            self.remove_widget(id, cx);
        }
        if docked_without_margin {
            // Monaco also uses the line numbers' space; here the indicator
            // needs the glyph margin.
            return;
        }
        let height = self
            .layout
            .as_ref()
            .map_or(px(0.), |layout| layout.bounds.size.height);
        let id = match placement {
            IndicatorPlacement::Docked(row) => self.add_glyph_margin_widget(
                GlyphMarginWidget {
                    row,
                    view: indicator.into(),
                },
                cx,
            ),
            IndicatorPlacement::Above => self.add_overlay_widget(
                OverlayWidget {
                    position: OverlayPosition::Custom(point(px(1.), px(2.))),
                    view: indicator.into(),
                },
                cx,
            ),
            IndicatorPlacement::Below => self.add_overlay_widget(
                OverlayWidget {
                    position: OverlayPosition::Custom(point(
                        px(1.),
                        (height - line_height - px(2.)).max(px(0.)),
                    )),
                    view: indicator.into(),
                },
                cx,
            ),
        };
        self.inline.edit_view.indicator_widget = Some((id, placement));
    }

    /// Previewed lines: `rows` of the modified text, colored like code,
    /// with `highlights` (modified ranges) given `highlight` as background.
    fn inline_edit_preview_lines(
        &self,
        diff: &InlineEditDiff,
        rows: std::ops::Range<u32>,
        highlights: &[std::ops::Range<PointUtf16>],
        highlight: Hsla,
        cx: &gpui::App,
    ) -> Vec<Vec<Segment>> {
        let (window_start, lines) = diff.modified_window();
        let text = lines.join("\n");
        let language = self.model().read(cx).language_id().map(|l| l.to_string());
        let spans = language.and_then(|language| {
            let registry = crate::model::language_registry(cx);
            crate::colorize::capture_spans(&text, &language, &registry)
        });
        let foreground = self.theme.colors.foreground;
        let mut line_start = 0;
        let mut starts = Vec::with_capacity(lines.len());
        for line in lines {
            starts.push(line_start);
            line_start += line.len() + 1;
        }
        rows.map(|row| {
            let Some(line) = diff.modified_line(row) else {
                return Vec::new();
            };
            let start = starts[(row - window_start) as usize];
            // The innermost span over each byte, and whether it's changed.
            let mut colors: Vec<Hsla> = vec![foreground; line.len()];
            if let Some(spans) = &spans {
                for (range, id) in spans {
                    let from = range.start.max(start).min(start + line.len());
                    let to = range.end.max(start).min(start + line.len());
                    let color = self.theme.style_for(*id).foreground.unwrap_or(foreground);
                    for slot in &mut colors[from - start..to - start] {
                        *slot = color;
                    }
                }
            }
            let mut changed = vec![false; line.len()];
            for range in highlights {
                if range.start.row > row || range.end.row < row {
                    continue;
                }
                let from = if range.start.row == row {
                    utf16_to_byte(line, range.start.column)
                } else {
                    0
                };
                let to = if range.end.row == row {
                    utf16_to_byte(line, range.end.column)
                } else {
                    line.len()
                };
                for slot in &mut changed[from..to.max(from)] {
                    *slot = true;
                }
            }
            let mut segments: Vec<Segment> = Vec::new();
            for (ix, ch) in line.char_indices() {
                let background = changed[ix].then_some(highlight);
                match segments.last_mut() {
                    Some(last) if last.color == colors[ix] && last.background == background => {
                        last.text.push(ch)
                    }
                    _ => segments.push(Segment {
                        text: ch.to_string(),
                        color: colors[ix],
                        background,
                    }),
                }
            }
            segments
        })
        .collect()
    }

    /// A box view with the editor's font.
    fn inline_edit_box(
        &self,
        lines: Vec<Vec<Segment>>,
        line_backgrounds: Vec<Option<Hsla>>,
        style: BoxStyle,
        cx: &mut Context<Self>,
    ) -> Entity<InlineEditBox> {
        let editor = cx.entity().downgrade();
        let font = self
            .options
            .inline_suggest
            .font_family
            .clone()
            .unwrap_or_else(|| self.options.font_family.clone());
        let font_size = self.options.font_size;
        let line_height = self.options.resolved_line_height();
        cx.new(|_| InlineEditBox {
            editor,
            lines,
            line_backgrounds,
            font,
            font_size,
            line_height,
            style,
        })
    }

    /// The decorations and widgets showing `diff` as `kind`.
    fn render_inline_edit(
        &mut self,
        diff: &InlineEditDiff,
        kind: InlineEditViewKind,
        cx: &mut Context<Self>,
    ) -> (
        Vec<(std::ops::Range<usize>, DecorationOptions)>,
        Vec<WidgetId>,
    ) {
        use InlineEditViewKind::*;
        let colors = self.theme.colors.clone();
        let rope = self.model().read(cx).buffer().rope().clone();
        let offset = |point: PointUtf16| {
            rope.point_utf16_to_offset(rope.clip_point_utf16(point, text::Bias::Left))
        };
        let range_offsets =
            |range: &std::ops::Range<PointUtf16>| offset(range.start)..offset(range.end);
        let deleted = colors.inline_edit_original_changed_text_background();
        let inserted = colors.inline_edit_modified_changed_text_background();
        let em = self
            .layout
            .as_ref()
            .map_or(self.options.font_size * 0.6, |layout| {
                layout.metrics.em_width
            });
        let mut decorations: Vec<(std::ops::Range<usize>, DecorationOptions)> = Vec::new();
        let mut widgets = Vec::new();
        // Monaco `OriginalEditorInlineDiffView`: the original text's
        // changes, for the views that leave the text in place.
        if matches!(
            kind,
            SideBySide | Deletion | InsertionInline | LineReplacement
        ) {
            let whole_line = |rows: &std::ops::Range<u32>| {
                (
                    offset(PointUtf16::new(rows.start, 0))
                        ..offset(PointUtf16::new(
                            rows.end - 1,
                            rope.line_len_utf16(rows.end - 1),
                        )),
                    DecorationOptions {
                        background: Some(deleted),
                        is_whole_line: true,
                        ..Default::default()
                    },
                )
            };
            for (original, modified, indices) in &diff.lines {
                if original.is_empty() || modified.is_empty() {
                    if !original.is_empty() {
                        decorations.push(whole_line(original));
                    }
                    continue;
                }
                for &ix in indices {
                    let (original_range, modified_range) = &diff.inner[ix];
                    if !original.contains(&original_range.start.row) {
                        continue;
                    }
                    if kind == LineReplacement && original_range.is_empty() {
                        continue;
                    }
                    if !original_range.is_empty() {
                        decorations.push((
                            range_offsets(original_range),
                            DecorationOptions {
                                background: Some(deleted),
                                z_index: 1,
                                ..Default::default()
                            },
                        ));
                    }
                    if kind == InsertionInline {
                        let text = diff.modified_text(modified_range);
                        let at = offset(original_range.end);
                        decorations.push((
                            at..at,
                            DecorationOptions {
                                before: Some(InjectedText {
                                    content: text.into(),
                                    style: TokenStyle::default(),
                                    background: Some(inserted),
                                    ..Default::default()
                                }),
                                z_index: 2,
                                ..Default::default()
                            },
                        ));
                    }
                }
            }
        }
        let editor_background = colors.background;
        let modified_border = colors.inline_edit_modified_border();
        let line_background = Some(colors.inline_edit_modified_background());
        match kind {
            WordReplacements => {
                let replacements: Vec<_> = diff
                    .inner
                    .iter()
                    .map(|(original, modified)| (original.clone(), diff.modified_text(modified)))
                    .collect();
                let mut grown = grow_edits_to_entire_word(diff, replacements.clone());
                if grown.iter().any(|(range, _)| range.is_empty()) {
                    grown = grow_edits_until_whitespace(diff, replacements);
                }
                for (range, text) in grown {
                    decorations.push((
                        range_offsets(&range),
                        DecorationOptions {
                            background: Some(deleted),
                            border: Some(colors.inline_edit_original_border()),
                            ..Default::default()
                        },
                    ));
                    // The new word, colored as it would be in its line.
                    let row = range.start.row;
                    let length = text.encode_utf16().count() as u32;
                    let word = PointUtf16::new(row, range.start.column)
                        ..PointUtf16::new(row, range.start.column + length);
                    let line = self
                        .inline_edit_preview_lines(diff, row..row + 1, &[], inserted, cx)
                        .pop()
                        .unwrap_or_default();
                    let segments = clip_segments(line, word.start.column, word.end.column);
                    let segments = segments
                        .into_iter()
                        .map(|segment| Segment {
                            background: Some(inserted),
                            ..segment
                        })
                        .collect();
                    let view = self.inline_edit_box(
                        vec![segments],
                        vec![None],
                        BoxStyle {
                            background: Some(editor_background),
                            border: Some(modified_border),
                            margin: (em * 3., px(4.)),
                            padding: px(1.),
                        },
                        cx,
                    );
                    widgets.push(self.add_content_widget(
                        ContentWidget {
                            position: range.start,
                            preference: vec![ContentWidgetPreference::Below],
                            view: view.into(),
                        },
                        cx,
                    ));
                }
            }
            LineReplacement => {
                let highlights: Vec<_> = diff
                    .inner
                    .iter()
                    .map(|(_, modified)| modified.clone())
                    .collect();
                let rows = diff.modified_line_range.clone();
                let lines =
                    self.inline_edit_preview_lines(diff, rows.clone(), &highlights, inserted, cx);
                let count = lines.len();
                let view = self.inline_edit_box(
                    lines,
                    vec![line_background; count],
                    BoxStyle {
                        background: Some(editor_background),
                        border: Some(modified_border),
                        margin: (px(0.), px(0.)),
                        padding: px(0.),
                    },
                    cx,
                );
                let anchor_row = diff
                    .original_line_range
                    .end
                    .max(diff.original_line_range.start + 1)
                    .saturating_sub(1)
                    .min(rope.max_point().row);
                widgets.push(self.add_content_widget(
                    ContentWidget {
                        position: PointUtf16::new(anchor_row, 0),
                        preference: vec![ContentWidgetPreference::Below],
                        view: view.into(),
                    },
                    cx,
                ));
            }
            SideBySide => {
                let rows = diff.modified_line_range.clone();
                let highlights: Vec<_> = diff
                    .inner
                    .iter()
                    .map(|(_, modified)| modified.clone())
                    .collect();
                let mut lines =
                    self.inline_edit_preview_lines(diff, rows.clone(), &highlights, inserted, cx);
                // Monaco `createReindentEdit`: without the common indentation.
                let indentation = diff
                    .new_lines
                    .iter()
                    .filter(|line| !line.trim().is_empty())
                    .map(|line| line.len() - line.trim_start_matches([' ', '\t']).len())
                    .min()
                    .unwrap_or(0);
                for line in &mut lines {
                    let mut skip = indentation;
                    line.retain_mut(|segment| {
                        if skip == 0 {
                            return true;
                        }
                        let taken = skip.min(segment.text.len());
                        segment.text.drain(..taken);
                        skip -= taken;
                        !segment.text.is_empty()
                    });
                }
                let count = lines.len();
                let display = diff.display_range();
                let start_row = display.start.min(rope.max_point().row);
                let widest = display
                    .clone()
                    .map(|row| self.inline_edit_text_width(&rope.line(row), cx))
                    .fold(px(0.), Pixels::max);
                let first = self.inline_edit_text_width(&rope.line(start_row), cx);
                let view = self.inline_edit_box(
                    lines,
                    vec![None; count],
                    BoxStyle {
                        background: Some(editor_background),
                        border: Some(modified_border),
                        margin: (widest - first + px(ORIGINAL_END_PADDING), px(0.)),
                        padding: px(4.),
                    },
                    cx,
                );
                widgets.push(self.add_content_widget(
                    ContentWidget {
                        position: PointUtf16::new(start_row, rope.line_len_utf16(start_row)),
                        preference: vec![ContentWidgetPreference::Exact],
                        view: view.into(),
                    },
                    cx,
                ));
            }
            Deletion => {
                let rows = &diff.original_line_range;
                if !rows.is_empty() {
                    let end = rows.end - 1;
                    decorations.push((
                        offset(PointUtf16::new(rows.start, 0))
                            ..offset(PointUtf16::new(end, rope.line_len_utf16(end))),
                        DecorationOptions {
                            is_whole_line: true,
                            block: Some(BlockDecoration {
                                border: colors.inline_edit_original_border(),
                                border_width: px(1.),
                                padding: [px(0.); 4],
                            }),
                            ..Default::default()
                        },
                    ));
                }
            }
            InsertionInline => {}
            InsertionMultiLine => {
                // Monaco `InlineEditsInsertionView`: ghost text at the
                // insertion, lines after the first in a zone.
                let Some((original, modified)) = diff.inner.first() else {
                    return (decorations, widgets);
                };
                let mut at = original.start;
                let mut text = diff.modified_text(modified);
                if at.column == 0
                    && at.row > 0
                    && rope.line_len_utf16(at.row) != 0
                    && text.ends_with('\n')
                    && !text.starts_with('\n')
                {
                    at = PointUtf16::new(at.row - 1, rope.line_len_utf16(at.row - 1));
                    text = format!("\n{}", &text[..text.len() - 1]);
                }
                let mut lines = text.split('\n');
                let first = lines.next().unwrap_or_default();
                if !first.is_empty() {
                    let at = offset(at);
                    decorations.push((
                        at..at,
                        DecorationOptions {
                            after: Some(InjectedText {
                                content: first.to_string().into(),
                                background: line_background,
                                ..Default::default()
                            }),
                            ..Default::default()
                        },
                    ));
                }
                let rest: Vec<Vec<Segment>> = lines
                    .map(|line| {
                        vec![Segment {
                            text: line.to_string(),
                            color: colors.foreground,
                            background: None,
                        }]
                    })
                    .collect();
                if !rest.is_empty() {
                    let count = rest.len();
                    let view = self.inline_edit_box(
                        rest,
                        vec![line_background; count],
                        BoxStyle {
                            background: None,
                            border: None,
                            margin: (px(0.), px(0.)),
                            padding: px(0.),
                        },
                        cx,
                    );
                    widgets.push(self.add_view_zone(
                        ViewZone {
                            after_line: at.row + 1,
                            height: ZoneHeight::Lines(count as f32),
                            view: view.into(),
                        },
                        cx,
                    ));
                }
            }
            Collapsed => {
                // Monaco puts the marker at the first replacement's start,
                // past what it keeps.
                let line_text = |row: u32| rope.line(row);
                let start = diff
                    .edits
                    .first()
                    .map_or(PointUtf16::default(), |(range, text)| {
                        super::ghost_text::single_text_remove_common_prefix(
                            &super::ghost_text::TextReplacement {
                                range: range.clone(),
                                text: text.clone(),
                            },
                            &line_text,
                        )
                        .range
                        .start
                    });
                let color = colors.inline_edit_gutter_indicator_primary()[1];
                let view = cx.new(|_| CollapsedMarker { color });
                widgets.push(self.add_content_widget(
                    ContentWidget {
                        position: start,
                        preference: vec![ContentWidgetPreference::Exact],
                        view: view.into(),
                    },
                    cx,
                ));
            }
        }
        (decorations, widgets)
    }
}

/// The part of `segments` (a line) between UTF-16 columns `from` and `to`.
fn clip_segments(segments: Vec<Segment>, from: u32, to: u32) -> Vec<Segment> {
    let mut column = 0u32;
    let mut out = Vec::new();
    for segment in segments {
        let mut text = String::new();
        for ch in segment.text.chars() {
            if column >= from && column < to {
                text.push(ch);
            }
            column += ch.len_utf16() as u32;
        }
        if !text.is_empty() {
            out.push(Segment { text, ..segment });
        }
    }
    out
}
