//! The color picker (Monaco `colorPicker`): in the hover over a color, or
//! standalone at the cursor. A header shows the picked color in the chosen
//! presentation next to the original; a saturation box and opacity and hue
//! strips change it.
//!
//! Ports `colorPickerModel.ts`, `colorPickerParticipantUtils.ts`, the
//! `colorPickerParts` and `standaloneColorPicker` (MIT).

use std::cell::Cell;
use std::ops::Range;
use std::rc::Rc;
use std::sync::Arc;

use gpui::{
    AppContext as _, Bounds, Context, DispatchPhase, Entity, FocusHandle, Focusable, Hsla,
    InteractiveElement, IntoElement, MouseButton, ParentElement, Pixels, Point, Render, Rgba,
    SharedString, StatefulInteractiveElement, Styled, Task, WeakEntity, Window, canvas,
    checkerboard, div, linear_color_stop, linear_gradient, px,
};
use text::PointUtf16;

use crate::actions::{HideColorPicker, InsertColorWithStandaloneColorPicker};
use crate::codicons::{Codicon, icon as codicon};
use crate::color::{Color, ColorF, Hsva, Rgba as ColorRgba};
use crate::color_detector::{ColorData, to_color};
use crate::editor::CodeEditor;
use crate::options::DefaultColorDecorators;
use crate::providers::{ColorInformation, ColorPresentation, DocumentColorProvider};
use crate::widgets::{ContentWidget, ContentWidgetPreference, WidgetId};

/// The key context of the standalone picker.
pub const STANDALONE_COLOR_PICKER_KEY_CONTEXT: &str = "StandaloneColorPicker";

/// The picked color and its presentations (Monaco `ColorPickerModel`).
#[derive(Clone, Debug)]
pub(crate) struct ColorPickerModel {
    pub original: Color,
    pub color: Color,
    pub presentations: Vec<ColorPresentation>,
    pub index: usize,
}

impl ColorPickerModel {
    pub fn new(color: Color) -> Self {
        Self {
            original: color,
            color,
            presentations: Vec::new(),
            index: 0,
        }
    }

    /// The chosen presentation.
    pub fn presentation(&self) -> Option<&ColorPresentation> {
        self.presentations.get(self.index)
    }

    pub fn set_presentations(&mut self, presentations: Vec<ColorPresentation>) {
        if self.index >= presentations.len() {
            self.index = 0;
        }
        self.presentations = presentations;
    }

    /// Monaco `selectNextColorPresentation` (without the flush).
    pub fn select_next(&mut self) {
        if !self.presentations.is_empty() {
            self.index = (self.index + 1) % self.presentations.len();
        }
    }

    /// Monaco `guessColorPresentation`: the presentation written like
    /// `original_text`, or one with the same function name.
    pub fn guess_presentation(&mut self, original_text: &str) {
        let lower = original_text.to_lowercase();
        let found = self
            .presentations
            .iter()
            .position(|p| lower == p.label.as_ref())
            .or_else(|| {
                let prefix = lower.split('(').next().unwrap_or("");
                self.presentations
                    .iter()
                    .position(|p| p.label.to_lowercase().starts_with(prefix))
            });
        if let Some(index) = found {
            self.index = index;
        }
    }
}

/// Monaco's `{ red: r / 255, …, alpha }`.
fn to_color_f(color: &Color) -> ColorF {
    let rgba = color.rgba();
    ColorF {
        red: rgba.r as f64 / 255.,
        green: rgba.g as f64 / 255.,
        blue: rgba.b as f64 / 255.,
        alpha: rgba.a,
    }
}

fn to_hsla(color: &Color) -> Hsla {
    let rgba = color.rgba();
    Rgba {
        r: rgba.r as f32 / 255.,
        g: rgba.g as f32 / 255.,
        b: rgba.b as f32 / 255.,
        a: rgba.a as f32,
    }
    .into()
}

fn hsla_to_color(color: Hsla) -> Color {
    let rgba = Rgba::from(color);
    Color::new(ColorRgba::new(
        (rgba.r * 255.).round() as u8,
        (rgba.g * 255.).round() as u8,
        (rgba.b * 255.).round() as u8,
        rgba.a as f64,
    ))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Part {
    Saturation,
    Opacity,
    Hue,
}

/// The standalone picker's state (Monaco `StandaloneColorPickerWidget`).
struct Standalone {
    /// Where "Insert" writes the color: the selections when it opened.
    selections: Vec<Range<PointUtf16>>,
    /// Whether the cursor was on a color (the button then says "Replace").
    found_in_editor: bool,
    focus: FocusHandle,
}

/// The color picker.
pub struct ColorPickerWidget {
    editor: WeakEntity<CodeEditor>,
    model: ColorPickerModel,
    /// The color's range in the text, moved by the picker's edits.
    range: Range<PointUtf16>,
    provider: Arc<dyn DocumentColorProvider>,
    standalone: Option<Standalone>,
    drag: Option<Part>,
    /// The saturation box and the strips, as last painted.
    bounds: Rc<[Cell<Bounds<Pixels>>; 3]>,
    theme: Arc<crate::Theme>,
    presentations_task: Option<Task<()>>,
    flush_task: Option<Task<()>>,
}

/// Monaco's strip heights: 150px in the hover, 122px standalone.
const HOVER_STRIP_HEIGHT: f32 = 150.;
const STANDALONE_STRIP_HEIGHT: f32 = 122.;
/// The strip slider's height.
const SLIDER_HEIGHT: f32 = 4.;

impl ColorPickerWidget {
    /// The color's range in the text.
    pub(crate) fn range(&self) -> &Range<PointUtf16> {
        &self.range
    }

    fn strip_height(&self) -> Pixels {
        px(if self.standalone.is_some() {
            STANDALONE_STRIP_HEIGHT
        } else {
            HOVER_STRIP_HEIGHT
        })
    }

    /// Monaco `ColorPickerBody` handlers: a pointer at `position` over
    /// `part` changes the color.
    fn drag_to(&mut self, part: Part, position: Point<Pixels>, cx: &mut Context<Self>) {
        let bounds = self.bounds[part as usize].get();
        let hsva = self.model.color.hsva();
        let fraction = |offset: Pixels, size: Pixels| {
            if size <= px(0.) {
                0.
            } else {
                (offset / size).clamp(0., 1.) as f64
            }
        };
        let color = match part {
            Part::Saturation => {
                let s = fraction(position.x - bounds.left(), bounds.size.width);
                let v = 1. - fraction(position.y - bounds.top(), bounds.size.height);
                Hsva::new(hsva.h, s, v, hsva.a)
            }
            Part::Opacity | Part::Hue => {
                let height = bounds.size.height - px(SLIDER_HEIGHT);
                let value = 1. - fraction(position.y - bounds.top(), height);
                if part == Part::Opacity {
                    Hsva::new(hsva.h, hsva.s, hsva.v, value)
                } else {
                    let h = (1. - value) * 360.;
                    Hsva::new(if h == 360. { 0. } else { h }, hsva.s, hsva.v, hsva.a)
                }
            }
        };
        self.set_color(Color::from_hsva(color), cx);
    }

    /// Monaco's `color` setter and `onDidChangeColor`: new presentations
    /// for the header.
    fn set_color(&mut self, color: Color, cx: &mut Context<Self>) {
        if self.model.color.equals(&color) {
            return;
        }
        self.model.color = color;
        let task = self.request_presentations(cx);
        self.presentations_task = Some(cx.spawn(async move |this, cx| {
            let presentations = task.await;
            this.update(cx, |this, cx| {
                this.model.set_presentations(presentations);
                cx.notify();
            })
            .ok();
        }));
        cx.notify();
    }

    /// Monaco `updateColorPresentations`: the provider's presentations of
    /// the current color at the color's range.
    fn request_presentations(&self, cx: &mut Context<Self>) -> Task<Vec<ColorPresentation>> {
        let info = ColorInformation {
            range: self.range.clone(),
            color: to_color_f(&self.model.color),
        };
        let Some(editor) = self.editor.upgrade() else {
            return Task::ready(Vec::new());
        };
        let context = editor.read(cx).model().read(cx).provider_context();
        let task = self
            .provider
            .provide_color_presentations(&context, &info, cx);
        cx.background_executor().spawn(async move {
            task.await.unwrap_or_else(|err| {
                log::warn!("color presentations failed: {err:#}");
                Vec::new()
            })
        })
    }

    /// Monaco `flushColor`: the hover writes the color into the text; the
    /// standalone picker waits for "Insert".
    fn flush(&mut self, cx: &mut Context<Self>) {
        if self.standalone.is_some() {
            return;
        }
        let task = self.request_presentations(cx);
        self.flush_task = Some(cx.spawn(async move |this, cx| {
            let presentations = task.await;
            this.update(cx, |this, cx| {
                this.model.set_presentations(presentations);
                this.update_editor(None, cx);
                cx.notify();
            })
            .ok();
        }));
    }

    /// Monaco `updateEditorModel`.
    fn update_editor(&mut self, insertion: Option<Vec<Range<PointUtf16>>>, cx: &mut Context<Self>) {
        let Some(presentation) = self.model.presentation().cloned() else {
            return;
        };
        let Some(editor) = self.editor.upgrade() else {
            return;
        };
        let range = self.range.clone();
        self.range = editor.update(cx, |editor, cx| {
            editor.apply_color_presentation(range, &presentation, insertion, cx)
        });
    }

    fn next_presentation(&mut self, cx: &mut Context<Self>) {
        self.model.select_next();
        self.flush(cx);
        cx.notify();
    }

    fn restore_original(&mut self, cx: &mut Context<Self>) {
        let original = self.model.original;
        self.set_color(original, cx);
        self.flush(cx);
    }

    /// The standalone "Insert"/"Replace" button and Enter.
    fn insert(&mut self, cx: &mut Context<Self>) {
        let Some(standalone) = &self.standalone else {
            return;
        };
        let selections = standalone.selections.clone();
        let task = self.request_presentations(cx);
        self.flush_task = Some(cx.spawn(async move |this, cx| {
            let presentations = task.await;
            this.update(cx, |this, cx| {
                this.model.set_presentations(presentations);
                this.update_editor(Some(selections), cx);
                this.close(cx);
            })
            .ok();
        }));
    }

    fn close(&mut self, cx: &mut Context<Self>) {
        if let Some(editor) = self.editor.upgrade() {
            cx.defer(move |cx| {
                editor.update(cx, |editor, cx| editor.hide_standalone_color_picker(cx));
            });
        }
    }

    /// The color, presentation label, and whether the presentations are in,
    /// for tests.
    pub(crate) fn state_for_tests(&self) -> (String, Option<String>, Range<PointUtf16>) {
        (
            Color::format_hex_a(&self.model.color, false),
            self.model.presentation().map(|p| p.label.to_string()),
            self.range.clone(),
        )
    }

    /// Sets the color as the saturation box or a strip would, then lets
    /// go (for tests).
    pub(crate) fn pick_for_tests(&mut self, color: Color, cx: &mut Context<Self>) {
        self.set_color(color, cx);
        self.flush(cx);
    }

    pub(crate) fn next_presentation_for_tests(&mut self, cx: &mut Context<Self>) {
        self.next_presentation(cx);
    }

    fn render_strip_slider(&self, value: f64) -> impl IntoElement {
        let height = self.strip_height() - px(SLIDER_HEIGHT);
        div()
            .absolute()
            .left(px(-2.))
            .top(height * (1. - value as f32))
            .w(px(29.))
            .h(px(SLIDER_HEIGHT))
            .border_1()
            .border_color(gpui::hsla(0., 0., 1., 0.71))
            .shadow_sm()
    }

    /// The pointer over the whole window during a drag (Monaco's pointer
    /// capture): a strip's `grabbing`, the saturation box's arrow.
    pub(crate) fn captured_cursor(&self) -> Option<gpui::CursorStyle> {
        self.drag.map(|part| match part {
            Part::Saturation => gpui::CursorStyle::Arrow,
            Part::Opacity | Part::Hue => gpui::CursorStyle::ClosedHand,
        })
    }

    /// A canvas recording `part`'s bounds and starting drags on it.
    fn part_tracker(&self, part: Part, cx: &mut Context<Self>) -> impl IntoElement {
        let bounds = self.bounds.clone();
        let entity = cx.entity().downgrade();
        canvas(
            move |b, _, _| {
                bounds[part as usize].set(b);
                b
            },
            move |b, _, window, _| {
                window.on_mouse_event(move |event: &gpui::MouseDownEvent, phase, _, cx| {
                    if phase == DispatchPhase::Bubble
                        && event.button == MouseButton::Left
                        && b.contains(&event.position)
                        && let Some(entity) = entity.upgrade()
                    {
                        cx.stop_propagation();
                        entity.update(cx, |this, cx| {
                            this.drag = Some(part);
                            this.drag_to(part, event.position, cx);
                        });
                    }
                });
            },
        )
        .absolute()
        .size_full()
    }
}

impl Focusable for ColorPickerWidget {
    fn focus_handle(&self, cx: &gpui::App) -> FocusHandle {
        match &self.standalone {
            Some(standalone) => standalone.focus.clone(),
            None => cx.focus_handle(),
        }
    }
}

impl Render for ColorPickerWidget {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = self.theme.colors.clone();
        let color = to_hsla(&self.model.color);
        let original = to_hsla(&self.model.original);
        let hsva = self.model.color.hsva();
        let strip_height = self.strip_height();
        // Monaco: dark text on light colors (or on a light hover through a
        // mostly transparent color).
        let light = if self.model.color.rgba().a < 0.5 {
            hsla_to_color(colors.hover_background).is_lighter()
        } else {
            self.model.color.is_lighter()
        };
        let label: SharedString = self
            .model
            .presentation()
            .map(|p| p.label.clone())
            .unwrap_or_default();
        let checker = || {
            div()
                .absolute()
                .size_full()
                .bg(gpui::white())
                .child(div().size_full().bg(checkerboard(gpui::rgb(0xcccccc), 4.5)))
        };
        let standalone = self.standalone.is_some();

        let header = div()
            .relative()
            .h(px(24.))
            .flex()
            .child(checker())
            .child(
                div()
                    .id("picked-color")
                    .relative()
                    .flex_1()
                    .flex()
                    .items_center()
                    .justify_center()
                    .overflow_hidden()
                    .whitespace_nowrap()
                    .cursor_pointer()
                    .bg(color)
                    .text_color(if light { gpui::black() } else { gpui::white() })
                    // `colorPickerHeader.ts`: the `color-mode` codicon, and
                    // a title.
                    .child(codicon(Codicon::ColorMode).size(px(14.)))
                    .child(div().mx(px(5.)).child(label))
                    .tooltip(crate::tooltip::tooltip(
                        "Click to toggle color options (rgb/hsl/hex)",
                        &colors,
                    ))
                    .on_click(cx.listener(|this, _, _, cx| this.next_presentation(cx))),
            )
            .child(
                div()
                    .id("original-color")
                    .relative()
                    .flex_none()
                    .w(px(74.))
                    .cursor_pointer()
                    .bg(original)
                    .on_click(cx.listener(|this, _, _, cx| this.restore_original(cx))),
            )
            .children(standalone.then(|| {
                div()
                    .id("close")
                    .relative()
                    .flex_none()
                    .w(px(22.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .cursor_pointer()
                    // `.close-button`: `editorHoverWidget.background` and
                    // `-border`, `toolbar.hoverBackground` on hover.
                    .bg(colors.hover_background)
                    .border_l_1()
                    .border_color(colors.hover_border)
                    .child(
                        div()
                            .size_full()
                            .flex()
                            .items_center()
                            .justify_center()
                            .hover(|style| colors.toolbar_hover(style))
                            // The close button's `close` codicon, 3px in.
                            .child(div().p(px(3.)).child(codicon(Codicon::Close))),
                    )
                    .on_click(cx.listener(|this, _, _, cx| this.close(cx)))
            }));

        // The saturation box: the hue, whitened to the left and darkened
        // to the bottom.
        let hue = to_hsla(&Color::from_hsva(Hsva::new(hsva.h, 1., 1., 1.)));
        // As last laid out (the box grows with the hover).
        let saturation_size = match self.bounds[Part::Saturation as usize].get().size {
            size if size.width > px(0.) => size,
            _ => gpui::size(px(220.), px(150.)),
        };
        let saturation = div()
            .relative()
            .flex_1()
            .min_w(px(220.))
            .h(px(150.))
            .overflow_hidden()
            .bg(hue)
            .child(div().absolute().size_full().bg(linear_gradient(
                90.,
                linear_color_stop(gpui::white(), 0.),
                linear_color_stop(gpui::white().opacity(0.), 1.),
            )))
            .child(div().absolute().size_full().bg(linear_gradient(
                180.,
                linear_color_stop(gpui::black().opacity(0.), 0.),
                linear_color_stop(gpui::black(), 1.),
            )))
            .child(
                div()
                    .absolute()
                    .left(saturation_size.width * hsva.s as f32)
                    .top(saturation_size.height * (1. - hsva.v as f32))
                    .size(px(9.))
                    .ml(px(-5.))
                    .mt(px(-5.))
                    .rounded_full()
                    .border_1()
                    .border_color(gpui::white())
                    .shadow_sm(),
            )
            .child(self.part_tracker(Part::Saturation, cx));
        // `.strip { cursor: grab }`, `.strip.grabbing` while that strip is
        // dragged.
        let strip_cursor = |part: Part| {
            if self.drag == Some(part) {
                gpui::CursorStyle::ClosedHand
            } else {
                gpui::CursorStyle::OpenHand
            }
        };
        let opaque = to_hsla(&self.model.color).opacity(1.);
        let opaque = Hsla { a: 1., ..opaque };
        let opacity = div()
            .id("color-picker-opacity-strip")
            .debug_selector(|| "color-picker-opacity-strip".into())
            .relative()
            .ml(px(8.))
            .w(px(25.))
            .h(strip_height)
            .cursor(strip_cursor(Part::Opacity))
            .child(checker())
            .child(div().absolute().size_full().bg(linear_gradient(
                180.,
                linear_color_stop(opaque, 0.),
                linear_color_stop(Hsla { a: 0., ..opaque }, 1.),
            )))
            .child(self.render_strip_slider(hsva.a))
            .child(self.part_tracker(Part::Opacity, cx));
        // Monaco's hue gradient: red, yellow, green, cyan, blue, magenta, red.
        let stops = [0., 0.17, 0.33, 0.5, 0.67, 0.83, 1.];
        let hues = [0., 60., 120., 180., 240., 300., 360.];
        let segments = (0..6).map(|i| {
            let from = to_hsla(&Color::from_hsva(Hsva::new(hues[i] % 360., 1., 1., 1.)));
            let to = to_hsla(&Color::from_hsva(Hsva::new(hues[i + 1] % 360., 1., 1., 1.)));
            div()
                .w_full()
                .h(strip_height * (stops[i + 1] - stops[i]) as f32)
                .bg(linear_gradient(
                    180.,
                    linear_color_stop(from, 0.),
                    linear_color_stop(to, 1.),
                ))
        });
        let hue_strip = div()
            .id("color-picker-hue-strip")
            .debug_selector(|| "color-picker-hue-strip".into())
            .relative()
            .ml(px(8.))
            .w(px(25.))
            .h(strip_height)
            .cursor(strip_cursor(Part::Hue))
            .flex()
            .flex_col()
            .children(segments)
            .child(self.render_strip_slider(1. - hsva.h / 360.))
            .child(self.part_tracker(Part::Hue, cx));

        let body = div()
            .relative()
            .flex()
            .p(px(8.))
            .child(saturation)
            .child(opacity)
            .child(hue_strip)
            .children(self.standalone.as_ref().map(|s| {
                div()
                    .id("insert")
                    .absolute()
                    .right(px(8.))
                    .bottom(px(8.))
                    .w(px(58.))
                    .h(px(20.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded(px(2.))
                    .cursor_pointer()
                    .bg(colors.button_background)
                    .hover(|style| style.bg(colors.button_hover_background))
                    .text_color(colors.button_foreground)
                    .child(if s.found_in_editor {
                        "Replace"
                    } else {
                        "Insert"
                    })
                    .on_click(cx.listener(|this, _, _, cx| this.insert(cx)))
            }));

        // Drags continue outside the picker until the button is released,
        // with the pointer they started with.
        let drag_listener = self.drag.map(|part| {
            crate::pointer_capture::capture(
                cx.entity().downgrade(),
                self.captured_cursor(),
                move |this: &mut Self, event, cx| this.drag_to(part, event.position, cx),
                |this, cx| {
                    if this.drag.take().is_some() {
                        this.flush(cx);
                        cx.notify();
                    }
                },
            )
        });

        let mut root = div()
            .w(px(302.))
            .flex()
            .flex_col()
            .text_size(px(13.))
            .text_color(colors.widget_foreground)
            .child(header)
            .child(body)
            .children(drag_listener);
        if let Some(standalone) = &self.standalone {
            root =
                root.track_focus(&standalone.focus)
                    // With the editor's keys, for its preconditions.
                    .key_context(crate::context_keys::widget_key_context(
                        STANDALONE_COLOR_PICKER_KEY_CONTEXT,
                        &crate::context_keys::editor_keys_for_widgets(&self.editor, cx),
                    ))
                    // `.standalone-colorpicker`: `editorHoverWidget.*`.
                    .bg(colors.hover_background)
                    .text_color(colors.hover_foreground)
                    .border_1()
                    .border_color(colors.hover_border)
                    .on_action(cx.listener(|this, _: &HideColorPicker, _, cx| this.close(cx)))
                    .on_action(cx.listener(
                        |this, _: &InsertColorWithStandaloneColorPicker, _, cx| this.insert(cx),
                    ));
        }
        root
    }
}

/// The standalone picker's state in the editor.
#[derive(Default)]
pub(crate) struct ColorPickerState {
    standalone: Option<(Entity<ColorPickerWidget>, WidgetId)>,
    task: Option<Task<()>>,
    /// Set while the picker edits the text, so the edit doesn't close it.
    pub(crate) editing: bool,
    /// Set while the picker selects the color it found, so the selection
    /// change doesn't close it.
    selecting: bool,
}

impl CodeEditor {
    /// Monaco `createColorHover`: a picker for `data`, with presentations
    /// fetched and the one written like the text chosen.
    pub(crate) fn create_color_picker(
        &mut self,
        data: ColorData,
        standalone: Option<(Vec<Range<PointUtf16>>, bool, FocusHandle)>,
        cx: &mut Context<Self>,
    ) -> Task<Option<Entity<ColorPickerWidget>>> {
        let model = self.model().read(cx);
        let rope = model.buffer().rope();
        let original_text: String = rope.text_for_range(
            rope.point_utf16_to_offset(data.info.range.start)
                ..rope.point_utf16_to_offset(data.info.range.end),
        );
        let context = model.provider_context();
        let presentations = data
            .provider
            .provide_color_presentations(&context, &data.info, cx);
        let theme = self.theme.clone();
        let editor = cx.entity().downgrade();
        cx.spawn(async move |_, cx| {
            let presentations = presentations.await.unwrap_or_else(|err| {
                log::warn!("color presentations failed: {err:#}");
                Vec::new()
            });
            let mut model = ColorPickerModel::new(to_color(&data.info.color));
            model.set_presentations(presentations);
            model.guess_presentation(&original_text);
            Some(cx.new(|_| ColorPickerWidget {
                editor,
                model,
                range: data.info.range,
                provider: data.provider,
                standalone: standalone.map(|(selections, found_in_editor, focus)| Standalone {
                    selections,
                    found_in_editor,
                    focus,
                }),
                drag: None,
                bounds: Rc::new(Default::default()),
                theme,
                presentations_task: None,
                flush_task: None,
            }))
        })
    }

    /// Monaco `updateEditorModel`: writes `presentation` over `range` (or
    /// at each of `insertion`) as one undo step, returning the color's new
    /// range.
    pub(crate) fn apply_color_presentation(
        &mut self,
        range: Range<PointUtf16>,
        presentation: &ColorPresentation,
        insertion: Option<Vec<Range<PointUtf16>>>,
        cx: &mut Context<Self>,
    ) -> Range<PointUtf16> {
        let (edit_range, text) = presentation
            .text_edit
            .clone()
            .unwrap_or_else(|| (range, presentation.label.to_string()));
        let mut edits = presentation.additional_text_edits.clone();
        match insertion {
            Some(ranges) => edits.extend(ranges.into_iter().map(|r| (r, text.clone()))),
            None => edits.push((edit_range.clone(), text.clone())),
        }
        self.color_picker.editing = true;
        self.execute_edits(edits, None, cx);
        self.color_picker.editing = false;
        self.push_undo_stop(cx);
        let start = edit_range.start;
        start..PointUtf16::new(start.row, start.column + text.encode_utf16().count() as u32)
    }

    /// Shows the standalone picker at the cursor, or focuses it (Monaco
    /// `editor.action.showOrFocusStandaloneColorPicker`). It edits the
    /// color under the selection, or inserts one (black to start with).
    pub fn show_or_focus_standalone_color_picker(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some((widget, _)) = &self.color_picker.standalone {
            let focus = widget.read(cx).focus_handle(cx);
            window.focus(&focus, cx);
            return;
        }
        self.hide_hover(cx);
        let selection = self.cursors.primary().selection();
        let selection = selection.start()..selection.end();
        let selections: Vec<Range<PointUtf16>> = self
            .cursors
            .selections()
            .iter()
            .map(|s| s.start()..s.end())
            .collect();
        let position = self.cursors.primary().position;
        // Monaco `getColors` ignores `defaultColorDecorators: never`.
        let enablement = match self.options.default_color_decorators {
            DefaultColorDecorators::Never => DefaultColorDecorators::Auto,
            other => other,
        };
        let colors = self.request_colors(enablement, cx);
        let focus = cx.focus_handle();
        self.color_picker.task = Some(cx.spawn_in(window, async move |this, cx| {
            let colors = colors.await;
            // Monaco: the last color containing the selection.
            let found = colors.into_iter().rev().find(|data| {
                data.info.range.start <= selection.start && selection.end <= data.info.range.end
            });
            let found_in_editor = found.is_some();
            let data = found.unwrap_or_else(|| ColorData {
                info: ColorInformation {
                    range: selection.clone(),
                    color: ColorF {
                        red: 0.,
                        green: 0.,
                        blue: 0.,
                        alpha: 1.,
                    },
                },
                provider: Arc::new(crate::color_detector::DefaultDocumentColorProvider),
            });
            let range = data.info.range.clone();
            // Monaco re-reads the selections when it selects the color it
            // found, so "Replace" writes over it.
            let selections = if found_in_editor {
                vec![range.clone()]
            } else {
                selections
            };
            let Ok(widget) = this.update(cx, |this, cx| {
                this.create_color_picker(
                    data,
                    Some((selections, found_in_editor, focus.clone())),
                    cx,
                )
            }) else {
                return;
            };
            let Some(widget) = widget.await else {
                return;
            };
            this.update_in(cx, |this, window, cx| {
                this.color_picker.task = None;
                if found_in_editor {
                    this.color_picker.selecting = true;
                    this.set_selections(
                        &[crate::cursor::Selection::new(range.start, range.end)],
                        cx,
                    );
                    this.color_picker.selecting = false;
                }
                let id = this.add_content_widget(
                    ContentWidget {
                        allow_editor_overflow: true,
                        ..ContentWidget::new(
                            position,
                            vec![
                                ContentWidgetPreference::Above,
                                ContentWidgetPreference::Below,
                            ],
                            widget.clone().into(),
                        )
                    },
                    cx,
                );
                this.widgets.set_layer(id, crate::widgets::layer::WIDGET);
                this.color_picker.standalone = Some((widget, id));
                window.focus(&focus, cx);
                // Monaco hides it when it loses focus.
                let editor = cx.entity().downgrade();
                window
                    .on_focus_out(&focus, cx, move |_, _, cx| {
                        if let Some(editor) = editor.upgrade() {
                            editor.update(cx, |editor, cx| editor.hide_standalone_color_picker(cx));
                        }
                    })
                    .detach();
            })
            .ok();
        }));
    }

    /// The color whose swatch is at `point` (window coordinates), as the
    /// position of its start.
    pub(crate) fn swatch_at(&self, point: Point<Pixels>) -> Option<PointUtf16> {
        let layout = self.layout.as_ref()?;
        let line = layout.lines.iter().find(|line| {
            let top = layout.line_top(line.view_line);
            top <= point.y && point.y < top + layout.line_height_of(line.view_line)
        })?;
        let x = point.x - layout.text_origin_x() - line.indent_x;
        let injected = line.injected.iter().find(|injected| {
            injected.swatch.is_some()
                && line.shaped.x_for_index(injected.range.start) <= x
                && x < line.shaped.x_for_index(injected.range.end)
        })?;
        // The swatch sits before the first column displayed after it.
        let column = line
            .display_index
            .iter()
            .position(|d| *d >= injected.range.end)?;
        Some(PointUtf16::new(
            line.row,
            line.columns.start + column as u32,
        ))
    }

    /// Whether the standalone picker shows.
    pub fn is_standalone_color_picker_visible(&self) -> bool {
        self.color_picker.standalone.is_some()
    }

    /// Whether the standalone picker has the focus (Monaco
    /// `standaloneColorPickerFocused`).
    pub(crate) fn is_standalone_color_picker_focused(
        &self,
        window: &Window,
        cx: &gpui::App,
    ) -> bool {
        self.color_picker
            .standalone
            .as_ref()
            .is_some_and(|(widget, _)| widget.focus_handle(cx).contains_focused(window, cx))
    }

    /// Hides the standalone picker (Monaco `editor.action.hideColorPicker`).
    pub fn hide_standalone_color_picker(&mut self, cx: &mut Context<Self>) {
        self.color_picker.task = None;
        if let Some((_, id)) = self.color_picker.standalone.take() {
            self.remove_widget(id, cx);
            self.refocus = true;
            cx.notify();
        }
    }

    /// Monaco hides the standalone picker when the cursor moves, except
    /// by the picker.
    pub(crate) fn color_picker_cursor_changed(&mut self, cx: &mut Context<Self>) {
        if self.color_picker.standalone.is_some()
            && !self.color_picker.selecting
            && !self.color_picker.editing
        {
            self.hide_standalone_color_picker(cx);
        }
    }

    /// The picker in the hover or standalone, for tests.
    #[doc(hidden)]
    pub fn color_picker_for_tests(&self, cx: &gpui::App) -> Option<Entity<ColorPickerWidget>> {
        self.color_picker
            .standalone
            .as_ref()
            .map(|(widget, _)| widget.clone())
            .or_else(|| self.hover_color_picker(cx))
    }

    /// (color as `#rrggbbaa`, presentation, range) of the picker, for tests.
    #[doc(hidden)]
    pub fn color_picker_state_for_tests(
        &self,
        cx: &gpui::App,
    ) -> Option<(String, Option<String>, Range<PointUtf16>)> {
        Some(self.color_picker_for_tests(cx)?.read(cx).state_for_tests())
    }
}

impl ColorPickerWidget {
    /// Picks `css` (e.g. `"#00ff00"`) and lets go, for tests.
    #[doc(hidden)]
    pub fn pick_css_for_tests(&mut self, css: &str, cx: &mut Context<Self>) {
        if let Some(color) = Color::parse(css) {
            self.pick_for_tests(color, cx);
        }
    }

    /// Clicks the picked color (next presentation), for tests.
    #[doc(hidden)]
    pub fn click_header_for_tests(&mut self, cx: &mut Context<Self>) {
        self.next_presentation_for_tests(cx);
    }

    /// Presses "Insert"/"Replace", for tests.
    #[doc(hidden)]
    pub fn insert_for_tests(&mut self, cx: &mut Context<Self>) {
        self.insert(cx);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn presentation(label: &str) -> ColorPresentation {
        ColorPresentation {
            label: label.to_string().into(),
            ..Default::default()
        }
    }

    #[test]
    fn guesses_and_cycles_presentations() {
        let mut model = ColorPickerModel::new(Color::parse("#ff0000").unwrap());
        model.set_presentations(vec![
            presentation("rgb(255, 0, 0)"),
            presentation("hsl(0, 100%, 50%)"),
            presentation("#ff0000"),
        ]);
        model.guess_presentation("#FF0000");
        assert_eq!(model.index, 2);
        model.guess_presentation("hsl(0 100% 50%)");
        assert_eq!(model.index, 1);
        model.guess_presentation("red");
        assert_eq!(model.index, 1);
        model.select_next();
        model.select_next();
        assert_eq!(model.index, 0);
        model.index = 2;
        model.set_presentations(vec![presentation("a")]);
        assert_eq!(model.index, 0);
    }
}
