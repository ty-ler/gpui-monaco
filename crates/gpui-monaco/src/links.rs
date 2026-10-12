//! Links (Monaco's `contrib/links`): URLs found by the link computer and
//! link providers are underlined; Cmd/Ctrl+hover highlights one and
//! Cmd/Ctrl+click follows it: the host's link openers get it, else web and
//! mail links open in the browser, `command:` links run and links into the
//! model select their `#L<line>,<col>` fragment, as Monaco's opener service
//! does; the editor emits
//! [`CodeEditorEvent::OpenLink`](crate::CodeEditorEvent::OpenLink) for
//! what nothing opened.

use std::ops::Range;
use std::sync::Arc;
use std::time::Duration;

use gpui::{App, Context, Entity, SharedString, Task};
use text::PointUtf16;

use crate::decorations::{DecorationId, DecorationOptions};
use crate::editor::CodeEditor;
use crate::model::TextModel;
use crate::options::MultiCursorModifier;
use crate::providers::{DocumentLink, LinkProvider, MarkdownString, link_providers};
use crate::theme::TokenStyle;

/// Monaco recomputes links this long after an edit (the minimum of its
/// adaptive debounce).
const RECOMPUTE_DELAY: Duration = Duration::from_millis(1000);

/// A link in the text and where it came from (Monaco `LinkOccurrence`).
struct Occurrence {
    decoration: DecorationId,
    link: DocumentLink,
    /// The provider that can resolve it; `None` for the link computer.
    provider: Option<Arc<dyn LinkProvider>>,
}

/// What the links were computed from: the text, the language and the
/// providers (Monaco recomputes on content, language and provider
/// changes).
#[derive(Clone, PartialEq)]
struct LinksKey {
    version: text::Version,
    language: Option<SharedString>,
    /// The link providers, by address.
    providers: Vec<usize>,
}

#[derive(Default)]
pub(crate) struct LinksState {
    occurrences: Vec<Occurrence>,
    /// What the links were computed for.
    key: Option<LinksKey>,
    /// What's being computed.
    pending: Option<LinksKey>,
    task: Option<Task<()>>,
    /// The link Cmd/Ctrl is held over.
    active: Option<DecorationId>,
    resolve: Option<Task<()>>,
}

/// A link and the provider that can resolve it (`None` for the link
/// computer's).
pub(crate) type ProvidedLink = (DocumentLink, Option<Arc<dyn LinkProvider>>);

/// Monaco `LinksList._union`: links from `new` replace the old ones they
/// overlap or touch.
fn union(old: Vec<ProvidedLink>, new: Vec<ProvidedLink>) -> Vec<ProvidedLink> {
    let touching =
        |a: &Range<PointUtf16>, b: &Range<PointUtf16>| a.start <= b.end && b.start <= a.end;
    let mut result = Vec::with_capacity(old.len() + new.len());
    let mut old = old.into_iter().peekable();
    let mut new = new.into_iter().peekable();
    while let (Some(o), Some(n)) = (old.peek(), new.peek()) {
        if touching(&o.0.range, &n.0.range) {
            old.next();
        } else if (o.0.range.start, o.0.range.end) < (n.0.range.start, n.0.range.end) {
            result.push(old.next().expect("peeked"));
        } else {
            result.push(new.next().expect("peeked"));
        }
    }
    result.extend(old);
    result.extend(new);
    result
}

/// Monaco `getLinks`: the model's links from the link computer (Monaco's
/// built-in `*` link provider, ranked below every registered one) and
/// each link provider, worst-ranked first, each list replacing the links
/// it overlaps or touches, so the best-ranked provider wins. The link
/// computer runs off the main thread, alongside the providers.
pub(crate) fn get_links(model: &Entity<TextModel>, cx: &mut App) -> Task<Vec<ProvidedLink>> {
    let rope = model.read(cx).buffer().rope().clone();
    let computed = cx.background_executor().spawn(async move {
        let text = rope.to_string();
        crate::link_computer::compute_links(text.split('\n'))
    });
    let context = model.read(cx).provider_context();
    // `ordered(model).reverse()`.
    let requests: Vec<_> = link_providers(cx, model.read(cx))
        .into_iter()
        .rev()
        .map(|provider| {
            let task = provider.provide_links(&context, cx);
            (provider, task)
        })
        .collect();
    // The providers' answers aren't `Send`: merged on the main thread.
    cx.spawn(async move |_| {
        let mut links: Vec<ProvidedLink> = computed
            .await
            .into_iter()
            .map(|link| {
                (
                    DocumentLink {
                        range: link.range,
                        url: Some(link.url.into()),
                        tooltip: None,
                    },
                    None,
                )
            })
            .collect();
        for (provider, task) in requests {
            match task.await {
                Ok(mut found) => {
                    found.sort_by_key(|l| (l.range.start, l.range.end));
                    let found = found
                        .into_iter()
                        .map(|link| (link, Some(provider.clone())))
                        .collect();
                    links = union(links, found);
                }
                Err(err) => log::warn!("links failed: {err:#}"),
            }
        }
        links
    })
}

impl CodeEditor {
    /// The modifier that follows links: the one that doesn't add cursors.
    fn link_label(&self) -> &'static str {
        let mac = cfg!(target_os = "macos");
        match (self.options.multi_cursor_modifier, mac) {
            (MultiCursorModifier::Alt, true) => "cmd + click",
            (MultiCursorModifier::Alt, false) => "ctrl + click",
            (MultiCursorModifier::CtrlCmd, true) => "option + click",
            (MultiCursorModifier::CtrlCmd, false) => "alt + click",
        }
    }

    /// Monaco `getHoverMessage`: "Follow link (cmd + click)", the label a
    /// link to the target, trusted (`new MarkdownString('', true)`).
    fn link_hover_message(&self, link: &DocumentLink) -> MarkdownString {
        let command = link
            .url
            .as_ref()
            .is_some_and(|url| url.to_ascii_lowercase().starts_with("command:"));
        let label = match &link.tooltip {
            Some(tooltip) => tooltip.to_string(),
            None if command => "Execute command".to_string(),
            None => "Follow link".to_string(),
        };
        let keys = self.link_label();
        let value = match &link.url {
            Some(url) => format!("[{label}]({}) ({keys})", url.replace(' ', "%20")),
            None => format!("{label} ({keys})"),
        };
        MarkdownString::new(value).trusted(true)
    }

    fn link_decoration(&self, link: &DocumentLink, active: bool) -> DecorationOptions {
        DecorationOptions {
            text: Some(TokenStyle {
                underline: true,
                foreground: active.then_some(self.theme.colors.link_active_foreground),
                ..Default::default()
            }),
            hover_message: vec![self.link_hover_message(link)],
            stickiness: text::Stickiness::NeverGrowsWhenTypingAtEdges,
            ..Default::default()
        }
    }

    /// Recomputes links when the text, the language or the link providers
    /// changed: after a pause in typing for an edit, right away otherwise
    /// (Monaco's `computeLinks.schedule(0)` for a new model, language or
    /// provider). Called before painting.
    pub(crate) fn refresh_links(&mut self, cx: &mut Context<Self>) {
        if !self.options.links {
            if !self.links.occurrences.is_empty() || self.links.task.is_some() {
                self.clear_links(cx);
            }
            return;
        }
        let model = self.model().read(cx);
        let key = LinksKey {
            version: model.buffer().version(),
            language: model.language_id().cloned(),
            providers: link_providers(cx, model)
                .iter()
                .map(|provider| Arc::as_ptr(provider) as *const () as usize)
                .collect(),
        };
        if self.links.key.as_ref() == Some(&key) || self.links.pending.as_ref() == Some(&key) {
            return;
        }
        // Only an edit waits; a pending one restarts after the delay, so
        // typing doesn't rescan the text on every keystroke.
        let edited = |k: &LinksKey| k.version != key.version;
        let delay = if self.links.key.as_ref().is_some_and(edited)
            || self.links.pending.as_ref().is_some_and(edited)
        {
            RECOMPUTE_DELAY
        } else {
            Duration::ZERO
        };
        self.links.pending = Some(key.clone());
        self.links.task = Some(cx.spawn(async move |this, cx| {
            if !delay.is_zero() {
                cx.background_executor().timer(delay).await;
            }
            let Ok(links) = this.update(cx, |this, cx| get_links(&this.model().clone(), cx)) else {
                return;
            };
            let links = links.await;
            this.update(cx, |this, cx| this.set_links(links, key, cx))
                .ok();
        }));
    }

    fn set_links(&mut self, links: Vec<ProvidedLink>, key: LinksKey, cx: &mut Context<Self>) {
        let model = self.model().clone();
        let buffer = model.read(cx).buffer();
        // Links computed for an older text are moved through the edits since.
        let patch = (buffer.version() != key.version).then(|| buffer.edits_since(key.version));
        let rope = buffer.rope().clone();
        let decorations: Vec<_> = links
            .iter()
            .map(|(link, _)| {
                let mut start = rope.point_utf16_to_offset(
                    rope.clip_point_utf16(link.range.start, text::Bias::Left),
                );
                let mut end = rope.point_utf16_to_offset(
                    rope.clip_point_utf16(link.range.end, text::Bias::Right),
                );
                if let Some(patch) = &patch {
                    start = patch.transform_offset(start, text::Bias::Right);
                    end = patch.transform_offset(end, text::Bias::Left).max(start);
                }
                (start..end, self.link_decoration(link, false))
            })
            .collect();
        let old: Vec<DecorationId> = self
            .links
            .occurrences
            .iter()
            .map(|o| o.decoration)
            .collect();
        let owner = Some(cx.entity_id());
        let ids = model.update(cx, |model, cx| {
            model.delta_decorations(owner, &old, decorations, cx)
        });
        self.links.occurrences = links
            .into_iter()
            .zip(ids)
            .map(|((link, provider), decoration)| Occurrence {
                decoration,
                link,
                provider,
            })
            .collect();
        self.links.active = None;
        self.links.key = Some(key);
        self.links.pending = None;
        self.links.task = None;
        cx.notify();
    }

    /// Removes every link (the `links` option was turned off, or the model
    /// changed).
    pub(crate) fn clear_links(&mut self, cx: &mut Context<Self>) {
        let old: Vec<DecorationId> = self
            .links
            .occurrences
            .iter()
            .map(|o| o.decoration)
            .collect();
        self.links = LinksState::default();
        if !old.is_empty() {
            let owner = Some(cx.entity_id());
            self.model().update(cx, |model, cx| {
                model.delta_decorations(owner, &old, Vec::new(), cx)
            });
        }
    }

    /// The link at `position`, by its decoration's current range.
    fn link_at(&self, position: PointUtf16, cx: &App) -> Option<usize> {
        let model = self.model().read(cx);
        let offset = model.buffer().point_utf16_to_offset(position);
        self.links.occurrences.iter().position(|occurrence| {
            model.decoration(occurrence.decoration).is_some_and(|d| {
                d.range.start <= offset && offset <= d.range.end && !d.range.is_empty()
            })
        })
    }

    /// Mouse moved with (`active`) or without the link modifier over
    /// `position`: highlights the link there (Monaco's active link).
    pub(crate) fn links_mouse_move(
        &mut self,
        position: Option<PointUtf16>,
        active: bool,
        cx: &mut Context<Self>,
    ) {
        let target = position
            .filter(|_| active)
            .and_then(|position| self.link_at(position, cx))
            .map(|ix| self.links.occurrences[ix].decoration);
        if target == self.links.active {
            return;
        }
        for (id, active) in [(self.links.active, false), (target, true)] {
            let Some(id) = id else {
                continue;
            };
            let Some(occurrence) = self.links.occurrences.iter().find(|o| o.decoration == id)
            else {
                continue;
            };
            let options = self.link_decoration(&occurrence.link, active);
            self.model().update(cx, |model, cx| {
                model.change_decoration_options(id, options, cx)
            });
        }
        self.links.active = target;
        cx.notify();
    }

    /// Whether Cmd/Ctrl is held over a link (for the pointer cursor).
    pub(crate) fn has_active_link(&self) -> bool {
        self.links.active.is_some()
    }

    /// Follows the link at `position` (Cmd/Ctrl+click). Returns whether
    /// there was one.
    pub(crate) fn follow_link_at(&mut self, position: PointUtf16, cx: &mut Context<Self>) -> bool {
        let Some(ix) = self.link_at(position, cx) else {
            return false;
        };
        self.open_link_occurrence(ix, cx);
        true
    }

    /// Opens the links at the end of each selection (Monaco
    /// `editor.action.openLink`).
    pub fn open_link(&mut self, cx: &mut Context<Self>) {
        let positions: Vec<PointUtf16> =
            self.cursors.selections().iter().map(|s| s.end()).collect();
        for position in positions {
            if let Some(ix) = self.link_at(position, cx) {
                self.open_link_occurrence(ix, cx);
            }
        }
    }

    /// Monaco `openLinkOccurrence`: resolves the link if needed, then opens
    /// it through the opener service, `command:` links included
    /// (`allowCommands: true`).
    fn open_link_occurrence(&mut self, ix: usize, cx: &mut Context<Self>) {
        let occurrence = &self.links.occurrences[ix];
        if let Some(url) = occurrence.link.url.clone() {
            self.emit_open_link(url, cx);
            return;
        }
        let Some(task) = occurrence
            .provider
            .clone()
            .and_then(|provider| provider.resolve_link(&occurrence.link, cx))
        else {
            log::warn!("Failed to open this link because its target is missing.");
            return;
        };
        self.links.resolve = Some(cx.spawn(async move |this, cx| match task.await {
            Ok(DocumentLink { url: Some(url), .. }) => {
                this.update(cx, |this, cx| this.emit_open_link(url, cx))
                    .ok();
            }
            Ok(_) => log::warn!("Failed to open this link because its target is missing."),
            Err(err) => log::warn!("resolving a link failed: {err:#}"),
        }));
    }

    /// Opens the link with [`crate::openers::open_url`], else emits
    /// [`CodeEditorEvent::OpenLink`](crate::CodeEditorEvent::OpenLink),
    /// resolving `file://./relative` links against a `file://` model's
    /// directory as Monaco does.
    fn emit_open_link(&mut self, url: SharedString, cx: &mut Context<Self>) {
        let model_uri = self.model().read(cx).uri().cloned();
        let url = match model_uri.as_deref() {
            Some(model) if model.starts_with("file://") => {
                let path = url.strip_prefix("file://");
                let relative = path.and_then(|path| {
                    path.strip_prefix("/./")
                        .or_else(|| path.strip_prefix("./"))
                        .or_else(|| path.strip_prefix("//./"))
                });
                match relative {
                    Some(relative) => {
                        let dir = &model[..model.rfind('/').map_or(model.len(), |ix| ix + 1)];
                        format!("{dir}{relative}").into()
                    }
                    None => url,
                }
            }
            _ => url,
        };
        self.open_link_with_openers(url, true, cx);
    }

    /// The links found, with their current ranges (for tests).
    #[doc(hidden)]
    pub fn links_for_tests(&self, cx: &App) -> Vec<(Range<PointUtf16>, Option<SharedString>)> {
        let model = self.model().read(cx);
        let rope = model.buffer().rope();
        self.links
            .occurrences
            .iter()
            .filter_map(|o| {
                let range = &model.decoration(o.decoration)?.range;
                Some((
                    rope.offset_to_point_utf16(range.start)..rope.offset_to_point_utf16(range.end),
                    o.link.url.clone(),
                ))
            })
            .collect()
    }
}
