//! Color decorators (Monaco `ColorDetector`): a swatch before each color the
//! color providers find, or the built-in finder without them.
//!
//! Ports `colorDetector.ts`, `color.ts` and `defaultDocumentColorProvider.ts`
//! (MIT).

use std::sync::Arc;
use std::time::Duration;

use gpui::{App, Context, Hsla, Rgba, Task};
use text::PointUtf16;

use crate::api::DecorationsCollection;
use crate::color::{Color, ColorF, Rgba as ColorRgba, compute_default_document_colors};
use crate::decorations::{DecorationId, DecorationOptions, InjectedText};
use crate::editor::CodeEditor;
use crate::options::DefaultColorDecorators;
use crate::providers::{
    ColorInformation, ColorPresentation, DocumentColorProvider, ProviderContext, color_providers,
};

/// Monaco `ColorDetector.RECOMPUTE_TIME`: the debounce floor after edits.
const RECOMPUTE_TIME: Duration = Duration::from_millis(1000);

/// A color and the provider that found it (Monaco `IColorData`).
#[derive(Clone)]
pub(crate) struct ColorData {
    pub info: ColorInformation,
    pub provider: Arc<dyn DocumentColorProvider>,
}

/// What the colors were computed for: the text, language, providers and
/// options.
type ColorKey = (
    text::Version,
    Option<String>,
    usize,
    DefaultColorDecorators,
    usize,
);

#[derive(Default)]
pub(crate) struct ColorDetectorState {
    key: Option<ColorKey>,
    pending: Option<ColorKey>,
    task: Option<Task<()>>,
    /// Tracks each color's range as the text changes (Monaco's empty
    /// decorations), with its data.
    datas: Vec<(DecorationId, ColorData)>,
    swatches: Option<DecorationsCollection>,
}

/// Monaco's built-in finder (`DefaultDocumentColorProvider`, registered for
/// every language): hex, `rgb()`, `rgba()`, `hsl()` and `hsla()` colors.
pub(crate) struct DefaultDocumentColorProvider;

impl DocumentColorProvider for DefaultDocumentColorProvider {
    fn provide_document_colors(
        &self,
        context: &ProviderContext,
        cx: &mut App,
    ) -> Task<anyhow::Result<Vec<ColorInformation>>> {
        let rope = context.snapshot.rope().clone();
        cx.background_executor().spawn(async move {
            let text = rope.to_string();
            Ok(compute_default_document_colors(&text)
                .into_iter()
                .map(|(range, color)| ColorInformation {
                    range: rope.offset_to_point_utf16(range.start)
                        ..rope.offset_to_point_utf16(range.end),
                    color,
                })
                .collect())
        })
    }

    /// Monaco: `rgb()`, `hsl()` and compact hex, each replacing the color.
    fn provide_color_presentations(
        &self,
        _context: &ProviderContext,
        info: &ColorInformation,
        _cx: &mut App,
    ) -> Task<anyhow::Result<Vec<ColorPresentation>>> {
        let color = to_color(&info.color);
        let presentations = [
            Color::format_rgb(&color),
            Color::format_hsl(&color),
            Color::format_hex_a(&color, true),
        ]
        .into_iter()
        .map(|text| ColorPresentation {
            label: text.clone().into(),
            text_edit: Some((info.range.clone(), text)),
            additional_text_edits: Vec::new(),
        })
        .collect();
        Task::ready(Ok(presentations))
    }
}

/// Monaco's `new Color(new RGBA(round(255 * red), …, alpha))`.
pub(crate) fn to_color(color: &ColorF) -> Color {
    Color::new(ColorRgba::new(
        (color.red * 255.).round() as u8,
        (color.green * 255.).round() as u8,
        (color.blue * 255.).round() as u8,
        color.alpha,
    ))
}

fn to_hsla(color: &ColorF) -> Hsla {
    Rgba {
        r: color.red as f32,
        g: color.green as f32,
        b: color.blue as f32,
        a: color.alpha as f32,
    }
    .into()
}

impl CodeEditor {
    /// Finds the colors when the text, providers or options changed: right
    /// away the first time, after Monaco's delay otherwise. Called before
    /// painting.
    pub(crate) fn refresh_colors(&mut self, cx: &mut Context<Self>) {
        let model = self.model().read(cx);
        let language = model.language_id().map(|l| l.to_string());
        let providers = color_providers(cx, language.as_deref());
        if !self.options.color_decorators
            || (providers.is_empty()
                && self.options.default_color_decorators == DefaultColorDecorators::Never)
        {
            if self.colors.key.is_some() || self.colors.task.is_some() {
                self.clear_colors(cx);
            }
            return;
        }
        let key: ColorKey = (
            model.buffer().version(),
            language,
            providers.len(),
            self.options.default_color_decorators,
            self.options.color_decorators_limit,
        );
        if self.colors.key.as_ref() == Some(&key) || self.colors.pending.as_ref() == Some(&key) {
            return;
        }
        // Only the first request is immediate: a pending one restarts after
        // the delay, so typing doesn't rescan the text on every keystroke.
        let edited = |k: &ColorKey| k.0 != key.0;
        let delay = if self.colors.key.as_ref().is_some_and(edited)
            || self.colors.pending.as_ref().is_some_and(edited)
        {
            RECOMPUTE_TIME
        } else {
            Duration::ZERO
        };
        self.colors.pending = Some(key.clone());
        let enablement = key.3;
        self.colors.task = Some(cx.spawn(async move |this, cx| {
            if !delay.is_zero() {
                cx.background_executor().timer(delay).await;
            }
            let Ok(requests) = this.update(cx, |this, cx| {
                let context = this.model().read(cx).provider_context();
                let mut providers: Vec<(Arc<dyn DocumentColorProvider>, bool)> = providers
                    .into_iter()
                    .map(|provider| (provider, false))
                    .collect();
                providers.push((Arc::new(DefaultDocumentColorProvider), true));
                providers
                    .into_iter()
                    .map(|(provider, is_default)| {
                        let task = provider.provide_document_colors(&context, cx);
                        (provider, is_default, task)
                    })
                    .collect::<Vec<_>>()
            }) else {
                return;
            };
            // Monaco `_findColorData`: the default finder only counts
            // when no provider answered, unless it's always on.
            let mut found = false;
            let mut datas = Vec::new();
            let mut defaults = Vec::new();
            for (provider, is_default, task) in requests {
                let colors = match task.await {
                    Ok(colors) => colors,
                    Err(err) => {
                        log::warn!("document colors failed: {err:#}");
                        continue;
                    }
                };
                let colors = colors.into_iter().map(|info| ColorData {
                    info,
                    provider: provider.clone(),
                });
                if is_default && enablement != DefaultColorDecorators::Always {
                    defaults.extend(colors);
                } else {
                    found = true;
                    datas.extend(colors);
                }
            }
            if !found && enablement != DefaultColorDecorators::Never {
                datas = defaults;
            }
            this.update(cx, |this, cx| {
                this.colors.pending = None;
                this.colors.task = None;
                if this.model().read(cx).buffer().version() != key.0 {
                    cx.notify();
                    return;
                }
                this.colors.key = Some(key);
                this.render_colors(datas, cx);
            })
            .ok();
        }));
    }

    /// Monaco `updateDecorations` and `updateColorDecorators`.
    fn render_colors(&mut self, datas: Vec<ColorData>, cx: &mut Context<Self>) {
        let rope = self.model().read(cx).buffer().rope().clone();
        let offset = |point: PointUtf16| rope.point_utf16_to_offset(point);
        let old: Vec<DecorationId> = self.colors.datas.iter().map(|(id, _)| *id).collect();
        let tracking = datas
            .iter()
            .map(|data| {
                (
                    offset(data.info.range.start)..offset(data.info.range.end),
                    DecorationOptions::default(),
                )
            })
            .collect();
        let owner = Some(cx.entity_id());
        let ids = self.model().update(cx, |model, cx| {
            model.delta_decorations(owner, &old, tracking, cx)
        });
        let swatches = datas
            .iter()
            .take(self.options.color_decorators_limit)
            .map(|data| {
                (
                    data.info.range.clone(),
                    DecorationOptions {
                        before: Some(InjectedText {
                            // Monaco's 1.2em: two no-break spaces of a
                            // monospace font.
                            content: "\u{a0}\u{a0}".into(),
                            swatch: Some(to_hsla(&data.info.color)),
                            ..Default::default()
                        }),
                        ..Default::default()
                    },
                )
            })
            .collect();
        let model = self.model().clone();
        let collection = self
            .colors
            .swatches
            .get_or_insert_with(|| DecorationsCollection::for_owner(model, cx.entity_id()));
        collection.set(swatches, cx);
        self.colors.datas = ids.into_iter().zip(datas).collect();
        cx.notify();
    }

    fn clear_colors(&mut self, cx: &mut Context<Self>) {
        let state = std::mem::take(&mut self.colors);
        let old: Vec<DecorationId> = state.datas.iter().map(|(id, _)| *id).collect();
        if !old.is_empty() {
            let owner = Some(cx.entity_id());
            self.model().update(cx, |model, cx| {
                model.delta_decorations(owner, &old, Vec::new(), cx)
            });
        }
        if let Some(mut swatches) = state.swatches {
            swatches.clear(cx);
        }
        cx.notify();
    }

    /// The color at `position` (Monaco `getColorData`), with its current
    /// range.
    pub(crate) fn color_data_at(&self, position: PointUtf16, cx: &App) -> Option<ColorData> {
        let model = self.model().read(cx);
        let rope = model.buffer().rope();
        let offset = rope.point_utf16_to_offset(position);
        self.colors.datas.iter().find_map(|(id, data)| {
            let range = model.decoration(*id)?.range.clone();
            (range.start <= offset && offset <= range.end).then(|| ColorData {
                info: ColorInformation {
                    range: rope.offset_to_point_utf16(range.start)
                        ..rope.offset_to_point_utf16(range.end),
                    color: data.info.color,
                },
                provider: data.provider.clone(),
            })
        })
    }

    /// The colors found, as (range, `#rrggbbaa`), for tests.
    #[doc(hidden)]
    pub fn colors_for_tests(&self, cx: &App) -> Vec<(std::ops::Range<PointUtf16>, String)> {
        self.colors
            .datas
            .iter()
            .filter_map(|(id, _)| {
                let data = self.color_data_at(
                    {
                        let model = self.model().read(cx);
                        let range = model.decoration(*id)?.range.clone();
                        model.buffer().rope().offset_to_point_utf16(range.start)
                    },
                    cx,
                )?;
                let color = to_color(&data.info.color);
                Some((data.info.range, Color::format_hex_a(&color, false)))
            })
            .collect()
    }

    /// How many swatches show, for tests.
    #[doc(hidden)]
    pub fn color_swatch_count_for_tests(&self) -> usize {
        self.colors.swatches.as_ref().map_or(0, |s| s.ids().len())
    }
}
