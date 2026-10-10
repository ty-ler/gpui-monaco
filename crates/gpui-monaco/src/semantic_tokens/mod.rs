//! Semantic tokens (Monaco's `contrib/semanticTokens`): colors from a
//! language's semantic tokens provider, layered over the syntax
//! highlighting.
//!
//! Ports `ModelSemanticColoring` (`documentSemanticTokens.ts`) and
//! `ViewportSemanticTokensContribution` (`viewportSemanticTokens.ts`)
//! (MIT). The whole document is requested right away and again after edits
//! (with Monaco's adaptive debounce), applying delta results to the
//! previous one; edits made meanwhile are replayed onto the result. Until a
//! complete result exists, a range provider colors the visible lines. The
//! tokens move with edits as Monaco's sparse tokens do.

pub(crate) mod store;

use std::collections::{HashMap, VecDeque};
use std::ops::Range;
use std::sync::Arc;
use std::time::Duration;

use web_time::Instant;

use gpui::{App, Context, SharedString, Task};
use text::PointUtf16;

use crate::editor::CodeEditor;
use crate::events::ContentChange;
use crate::options::SemanticHighlighting;
use crate::providers::{
    DocumentSemanticTokensProvider, SemanticTokensEpoch, SemanticTokensLegend,
    SemanticTokensResult, document_range_semantic_tokens_providers,
    document_semantic_tokens_providers,
};
use crate::semantic_theme::SemanticStyle;
use crate::theme::TokenStyle;
use store::{SparseMultilineTokens, SparseTokensStore, count_eol, to_multiline_tokens};

/// Monaco's debounce bounds for document requests.
const DOCUMENT_DELAY: (Duration, Duration) =
    (Duration::from_millis(300), Duration::from_millis(2000));
/// Monaco's debounce bounds for viewport requests.
const VIEWPORT_DELAY: (Duration, Duration) =
    (Duration::from_millis(100), Duration::from_millis(500));

/// Monaco's `languageFeatureDebounceService`: the average of the last six
/// request durations, clamped, or 1.5 times the minimum before any.
#[derive(Default)]
struct Debounce {
    samples: VecDeque<Duration>,
}

impl Debounce {
    fn get(&self, (min, max): (Duration, Duration)) -> Duration {
        if self.samples.is_empty() {
            return min.mul_f32(1.5).min(max);
        }
        let average = self.samples.iter().sum::<Duration>() / self.samples.len() as u32;
        average.clamp(min, max)
    }

    fn update(&mut self, elapsed: Duration) {
        if self.samples.len() == 6 {
            self.samples.pop_front();
        }
        self.samples.push_back(elapsed);
    }
}

/// The last document result, kept for delta requests.
struct Response {
    provider: Arc<dyn DocumentSemanticTokensProvider>,
    result_id: Option<SharedString>,
    data: Vec<u32>,
}

#[derive(Default)]
pub(crate) struct SemanticTokensState {
    store: SparseTokensStore,
    /// Styles by the id the stored tokens carry.
    styles: Vec<SemanticStyle>,
    style_ids: HashMap<(String, Vec<String>), Option<u32>>,
    response: Option<Response>,
    /// The document request in flight.
    request: Option<Task<()>>,
    /// Edits made while it's in flight, replayed onto its result.
    pending_changes: Vec<Arc<[ContentChange]>>,
    providers_changed_during_request: bool,
    /// The next document request.
    scheduled: Option<Task<()>>,
    debounce: Debounce,
    viewport_task: Option<Task<()>>,
    viewport_debounce: Debounce,
    /// The rows last requested from the range provider.
    viewport_rows: Option<Range<u32>>,
    /// What the state was computed for: enabled, theme, epoch, language.
    key: Option<(bool, usize, u64, Option<SharedString>)>,
}

impl SemanticTokensState {
    fn style_id(&mut self, style: SemanticStyle) -> u32 {
        match self.styles.iter().position(|s| *s == style) {
            Some(ix) => ix as u32,
            None => {
                self.styles.push(style);
                self.styles.len() as u32 - 1
            }
        }
    }
}

impl CodeEditor {
    /// Whether semantic tokens color the text (Monaco
    /// `isSemanticColoringEnabled`).
    pub fn semantic_highlighting_enabled(&self) -> bool {
        match self.options.semantic_highlighting {
            SemanticHighlighting::On => true,
            SemanticHighlighting::Off => false,
            SemanticHighlighting::ConfiguredByTheme => self.theme.semantic_highlighting,
        }
    }

    /// Starts over when the setting, theme, language or a provider refresh
    /// changed, and keeps the viewport colored until the document result
    /// lands. Called before painting with the visible rows.
    pub(crate) fn refresh_semantic_tokens(
        &mut self,
        visible_rows: Range<u32>,
        cx: &mut Context<Self>,
    ) {
        let enabled = self.semantic_highlighting_enabled();
        let language = self.model().read(cx).language_id().cloned();
        // Monaco asks again when providers come and go (the registry's
        // `onDidChange`), e.g. once a language server starts.
        let providers = document_semantic_tokens_providers(cx, language.as_deref()).len() as u64
            + document_range_semantic_tokens_providers(cx, language.as_deref()).len() as u64;
        let epoch = cx
            .try_global::<SemanticTokensEpoch>()
            .map_or(0, |e| e.0)
            .wrapping_mul(1 << 16)
            + providers;
        let key = (enabled, Arc::as_ptr(&self.theme) as usize, epoch, language);
        if self.semantic.key.as_ref() != Some(&key) {
            let first = self.semantic.key.is_none();
            let theme_changed = self
                .semantic
                .key
                .as_ref()
                .is_some_and(|old| old.1 != key.1 || old.3 != key.3 || old.0 != key.0);
            self.semantic.key = Some(key);
            if first || theme_changed {
                // Monaco clears the tokens and forgets the previous result.
                self.reset_semantic_tokens(cx);
            } else if self.semantic.request.is_some() {
                self.semantic.providers_changed_during_request = true;
            }
            if enabled {
                self.schedule_semantic_tokens(Duration::ZERO, true, cx);
            }
        }
        if enabled {
            self.refresh_viewport_semantic_tokens(visible_rows, cx);
        }
    }

    /// Drops every token and the previous result.
    pub(crate) fn reset_semantic_tokens(&mut self, cx: &mut Context<Self>) {
        if let Some(response) = self.semantic.response.take() {
            response
                .provider
                .release_document_semantic_tokens(response.result_id);
        }
        let key = self.semantic.key.take();
        self.semantic = SemanticTokensState {
            key,
            ..Default::default()
        };
        cx.notify();
    }

    /// Monaco's `RunOnceScheduler`: `force` reschedules; otherwise an
    /// already scheduled request keeps its time (a leading debounce).
    fn schedule_semantic_tokens(&mut self, delay: Duration, force: bool, cx: &mut Context<Self>) {
        if self.semantic.scheduled.is_some() && !force {
            return;
        }
        self.semantic.scheduled = Some(cx.spawn(async move |this, cx| {
            if !delay.is_zero() {
                cx.background_executor().timer(delay).await;
            }
            this.update(cx, |this, cx| {
                this.semantic.scheduled = None;
                this.fetch_document_semantic_tokens(cx);
            })
            .ok();
        }));
    }

    /// Moves the tokens with edits, and asks again after a pause.
    pub(crate) fn semantic_tokens_after_edit(
        &mut self,
        changes: &Arc<[ContentChange]>,
        cx: &mut Context<Self>,
    ) {
        for change in changes.iter() {
            let (eol_count, first_line_length, last_line_length) = count_eol(&change.text);
            let first_char = change.text.encode_utf16().next().unwrap_or(0);
            self.semantic.store.accept_edit(
                change.range.clone(),
                eol_count,
                first_line_length,
                last_line_length,
                first_char,
            );
        }
        if self.semantic.request.is_some() {
            self.semantic.pending_changes.push(changes.clone());
        }
        if self.semantic.key.as_ref().is_some_and(|key| key.0) {
            let delay = self.semantic.debounce.get(DOCUMENT_DELAY);
            self.schedule_semantic_tokens(delay, false, cx);
            // The viewport asks again too.
            self.semantic.viewport_rows = None;
        }
    }

    /// Monaco `_fetchDocumentSemanticTokensNow`.
    fn fetch_document_semantic_tokens(&mut self, cx: &mut Context<Self>) {
        if self.semantic.request.is_some() {
            return;
        }
        let model = self.model().read(cx);
        let language = model.language_id().map(|l| l.to_string());
        let context = model.provider_context();
        // The most recently registered first, as Monaco orders them.
        let mut providers = document_semantic_tokens_providers(cx, language.as_deref());
        providers.reverse();
        if providers.is_empty() {
            if !self.semantic.store.is_empty() {
                self.semantic.store.set(None, false);
                cx.notify();
            }
            return;
        }
        let last = self.semantic.response.as_ref();
        let requests: Vec<_> = providers
            .iter()
            .map(|provider| {
                let last_result_id = last
                    .filter(|r| Arc::ptr_eq(&r.provider, provider))
                    .and_then(|r| r.result_id.clone());
                provider.provide_document_semantic_tokens(&context, last_result_id, cx)
            })
            .collect();
        self.semantic.providers_changed_during_request = false;
        let started = Instant::now();
        self.semantic.request = Some(cx.spawn(async move |this, cx| {
            let mut outcome: anyhow::Result<(
                Arc<dyn DocumentSemanticTokensProvider>,
                Option<SemanticTokensResult>,
            )> = Ok((providers[0].clone(), None));
            let mut decided = false;
            for (provider, request) in providers.iter().zip(requests) {
                let result = request.await;
                if decided {
                    continue;
                }
                match result {
                    Err(err) => {
                        outcome = Err(err);
                        decided = true;
                    }
                    Ok(Some(tokens)) => {
                        outcome = Ok((provider.clone(), Some(tokens)));
                        decided = true;
                    }
                    Ok(None) => {}
                }
            }
            this.update(cx, |this, cx| {
                this.semantic.request = None;
                let pending = std::mem::take(&mut this.semantic.pending_changes);
                match outcome {
                    Ok((provider, tokens)) => {
                        this.semantic.debounce.update(started.elapsed());
                        this.set_document_semantic_tokens(provider, tokens, pending, cx);
                    }
                    Err(err) => {
                        log::warn!("semantic tokens failed: {err:#}");
                        // Monaco keeps the old tokens and asks again if the
                        // text or providers changed meanwhile.
                        if !pending.is_empty() || this.semantic.providers_changed_during_request {
                            let delay = this.semantic.debounce.get(DOCUMENT_DELAY);
                            this.schedule_semantic_tokens(delay, false, cx);
                        }
                    }
                }
            })
            .ok();
        }));
    }

    /// Monaco `_setDocumentSemanticTokens`.
    fn set_document_semantic_tokens(
        &mut self,
        provider: Arc<dyn DocumentSemanticTokensProvider>,
        tokens: Option<SemanticTokensResult>,
        pending: Vec<Arc<[ContentChange]>>,
        cx: &mut Context<Self>,
    ) {
        let previous = self.semantic.response.take();
        if let Some(previous) = &previous {
            previous
                .provider
                .release_document_semantic_tokens(previous.result_id.clone());
        }
        let reschedule = !pending.is_empty() || self.semantic.providers_changed_during_request;
        let (result_id, data) = match tokens {
            None => {
                self.semantic.store.set(None, true);
                cx.notify();
                if reschedule {
                    self.reschedule_semantic_tokens(cx);
                }
                return;
            }
            Some(SemanticTokensResult::Tokens(tokens)) => (tokens.result_id, tokens.data),
            Some(SemanticTokensResult::Edits(edits)) => {
                let Some(previous) = previous else {
                    provider.release_document_semantic_tokens(edits.result_id);
                    self.semantic.store.set(None, true);
                    cx.notify();
                    return;
                };
                match apply_semantic_edits(previous.data, &edits.edits) {
                    Some(data) => (edits.result_id, data),
                    None => {
                        log::warn!("invalid semantic tokens edits; requesting all tokens");
                        provider.release_document_semantic_tokens(edits.result_id);
                        self.semantic.store.set(None, true);
                        cx.notify();
                        self.schedule_semantic_tokens(Duration::ZERO, true, cx);
                        return;
                    }
                }
            }
        };
        let legend = provider.legend();
        let mut pieces = self.decode_semantic_tokens(&data, &legend, cx);
        self.semantic.response = Some(Response {
            provider,
            result_id,
            data,
        });
        // Edits made while the request ran.
        for changes in &pending {
            for change in changes.iter() {
                for piece in &mut pieces {
                    piece.apply_edit(change.range.clone(), &change.text);
                }
            }
        }
        self.semantic.store.set(Some(pieces), true);
        cx.notify();
        if reschedule {
            self.reschedule_semantic_tokens(cx);
        }
    }

    fn reschedule_semantic_tokens(&mut self, cx: &mut Context<Self>) {
        let delay = self.semantic.debounce.get(DOCUMENT_DELAY);
        self.schedule_semantic_tokens(delay, false, cx);
    }

    /// Decodes provider data with the theme's styles (Monaco
    /// `toMultilineTokens2` with `SemanticTokensProviderStyling`): unknown
    /// types and types the theme doesn't style are dropped.
    fn decode_semantic_tokens(
        &mut self,
        data: &[u32],
        legend: &SemanticTokensLegend,
        cx: &App,
    ) -> Vec<SparseMultilineTokens> {
        let language = self.model().read(cx).language_id().map(|l| l.to_string());
        let theme = self.theme.clone();
        let state = &mut self.semantic;
        let mut style = |token_type: u32, modifier_set: u32| -> Option<u32> {
            let token_type = legend.token_types.get(token_type as usize)?.to_string();
            let modifiers: Vec<String> = (0..legend.token_modifiers.len().min(32))
                .filter(|bit| modifier_set & (1 << bit) != 0)
                .map(|bit| legend.token_modifiers[bit].to_string())
                .collect();
            let key = (token_type, modifiers);
            if let Some(id) = state.style_ids.get(&key) {
                return *id;
            }
            let modifier_refs: Vec<&str> = key.1.iter().map(String::as_str).collect();
            let id = theme
                .semantic_style(&key.0, &modifier_refs, language.as_deref())
                .map(|style| state.style_id(style));
            state.style_ids.insert(key, id);
            id
        };
        to_multiline_tokens(data, &mut style)
    }

    /// Monaco's viewport contribution: until a complete result exists, the
    /// range provider colors the visible lines (plus a screen's worth
    /// around them), asked after a short pause in scrolling.
    fn refresh_viewport_semantic_tokens(&mut self, visible: Range<u32>, cx: &mut Context<Self>) {
        if self.semantic.store.is_complete() {
            return;
        }
        let around = (visible.end - visible.start).max(20);
        let rows = visible.start.saturating_sub(around)..visible.end + around;
        if self.semantic.viewport_rows.as_ref() == Some(&rows) {
            return;
        }
        self.semantic.viewport_rows = Some(rows.clone());
        let delay = self.semantic.viewport_debounce.get(VIEWPORT_DELAY);
        self.semantic.viewport_task = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(delay).await;
            let Ok(requests) = this.update(cx, |this, cx| {
                if this.semantic.store.is_complete() {
                    return None;
                }
                let model = this.model().read(cx);
                let language = model.language_id().map(|l| l.to_string());
                let context = model.provider_context();
                let max_row = model.buffer().max_point().row;
                let end_row = rows.end.min(max_row);
                let range = PointUtf16::new(rows.start.min(max_row), 0)
                    ..PointUtf16::new(end_row, model.buffer().line_len_utf16(end_row));
                let version = model.buffer().version();
                let provider =
                    document_range_semantic_tokens_providers(cx, language.as_deref()).pop()?;
                let task =
                    provider.provide_document_range_semantic_tokens(&context, range.clone(), cx);
                Some((provider, task, range, version))
            }) else {
                return;
            };
            let Some((provider, task, range, version)) = requests else {
                return;
            };
            let started = Instant::now();
            let result = task.await;
            this.update(cx, |this, cx| {
                this.semantic.viewport_debounce.update(started.elapsed());
                let Ok(Some(tokens)) = result else {
                    if let Err(err) = result {
                        log::warn!("range semantic tokens failed: {err:#}");
                    }
                    return;
                };
                if this.model().read(cx).buffer().version() != version
                    || this.semantic.store.is_complete()
                {
                    return;
                }
                let pieces = this.decode_semantic_tokens(&tokens.data, &provider.legend(), cx);
                this.semantic.store.set_partial(range, pieces);
                cx.notify();
            })
            .ok();
        }));
    }

    /// The syntax highlights of byte `range` with semantic token styles on
    /// top.
    pub(crate) fn merge_semantic_tokens(
        &self,
        highlights: Vec<(Range<usize>, TokenStyle)>,
        range: Range<usize>,
        cx: &App,
    ) -> Vec<(Range<usize>, TokenStyle)> {
        if self.semantic.store.is_empty() || !self.semantic_highlighting_enabled() {
            return highlights;
        }
        let rope = self.model().read(cx).buffer().rope().clone();
        let first = rope.offset_to_point_utf16(range.start).row;
        let last = rope.offset_to_point_utf16(range.end).row;
        let mut spans: Vec<(Range<usize>, SemanticStyle)> = Vec::new();
        for row in first..=last {
            let line_len = rope.line_len_utf16(row);
            for (start, end, style) in self.semantic.store.line_tokens(row) {
                let Some(style) = self.semantic.styles.get(style as usize) else {
                    continue;
                };
                let (start, end) = (start.min(line_len), end.min(line_len));
                if start >= end {
                    continue;
                }
                spans.push((
                    rope.point_utf16_to_offset(PointUtf16::new(row, start))
                        ..rope.point_utf16_to_offset(PointUtf16::new(row, end)),
                    *style,
                ));
            }
        }
        if spans.is_empty() {
            return highlights;
        }
        merge_styles(&highlights, &spans)
    }

    /// The semantic tokens of `row` as (UTF-16 columns, style), for tests.
    #[doc(hidden)]
    pub fn semantic_tokens_for_tests(&self, row: u32) -> Vec<(Range<u32>, SemanticStyle)> {
        self.semantic
            .store
            .line_tokens(row)
            .into_iter()
            .filter_map(|(start, end, style)| {
                Some((start..end, *self.semantic.styles.get(style as usize)?))
            })
            .collect()
    }
}

/// Monaco's edit application for delta results: back to front, refusing
/// edits outside the data.
fn apply_semantic_edits(
    mut data: Vec<u32>,
    edits: &[crate::providers::SemanticTokensEdit],
) -> Option<Vec<u32>> {
    let delta: isize = edits
        .iter()
        .map(|e| e.data.len() as isize - e.delete_count as isize)
        .sum();
    if data.len() as isize + delta < 0 {
        return None;
    }
    for edit in edits.iter().rev() {
        if edit.start > data.len() {
            return None;
        }
        let end = (edit.start + edit.delete_count).min(data.len());
        data.splice(edit.start..end, edit.data.iter().copied());
    }
    Some(data)
}

/// Overlays `top` (sorted, non-overlapping) on `base`: where a semantic
/// token is, its set properties override the syntax style.
fn merge_styles(
    base: &[(Range<usize>, TokenStyle)],
    top: &[(Range<usize>, SemanticStyle)],
) -> Vec<(Range<usize>, TokenStyle)> {
    let mut bounds: Vec<usize> = base
        .iter()
        .map(|(r, _)| [r.start, r.end])
        .chain(top.iter().map(|(r, _)| [r.start, r.end]))
        .flatten()
        .collect();
    bounds.sort_unstable();
    bounds.dedup();
    let mut result: Vec<(Range<usize>, TokenStyle)> = Vec::new();
    let mut base_ix = 0;
    let mut top_ix = 0;
    for pair in bounds.windows(2) {
        let (start, end) = (pair[0], pair[1]);
        while base_ix < base.len() && base[base_ix].0.end <= start {
            base_ix += 1;
        }
        while top_ix < top.len() && top[top_ix].0.end <= start {
            top_ix += 1;
        }
        let base_style = base
            .get(base_ix)
            .filter(|(r, _)| r.start <= start && end <= r.end)
            .map(|(_, s)| *s);
        let top_style = top
            .get(top_ix)
            .filter(|(r, _)| r.start <= start && end <= r.end)
            .map(|(_, s)| *s);
        let style = match (base_style, top_style) {
            (None, None) => continue,
            (base, Some(top)) => top.apply(base.unwrap_or_default()),
            (Some(base), None) => base,
        };
        match result.last_mut() {
            Some((range, last)) if range.end == start && *last == style => range.end = end,
            _ => result.push((start..end, style)),
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn delta_edits_apply_back_to_front() {
        use crate::providers::SemanticTokensEdit;
        let edit = |start, delete_count, data: &[u32]| SemanticTokensEdit {
            start,
            delete_count,
            data: data.to_vec(),
        };
        assert_eq!(
            apply_semantic_edits(vec![1, 2, 3, 4, 5], &[edit(0, 1, &[9]), edit(3, 2, &[])]),
            Some(vec![9, 2, 3])
        );
        // Out of bounds: rejected.
        assert_eq!(apply_semantic_edits(vec![1], &[edit(5, 0, &[1])]), None);
        assert_eq!(apply_semantic_edits(vec![1], &[edit(0, 3, &[])]), None);
    }

    #[test]
    fn semantic_styles_override_only_what_they_set() {
        let red: gpui::Hsla = gpui::rgb(0xff0000).into();
        let blue: gpui::Hsla = gpui::rgb(0x0000ff).into();
        let base = vec![(
            0..10,
            TokenStyle {
                foreground: Some(red),
                bold: true,
                ..Default::default()
            },
        )];
        let top = vec![(
            2..4,
            SemanticStyle {
                foreground: Some(blue),
                ..Default::default()
            },
        )];
        let merged = merge_styles(&base, &top);
        assert_eq!(merged.len(), 3);
        assert_eq!(merged[1].0, 2..4);
        assert_eq!(merged[1].1.foreground, Some(blue));
        assert!(merged[1].1.bold, "bold kept");
        assert_eq!(merged[2].1.foreground, Some(red));
    }
}
