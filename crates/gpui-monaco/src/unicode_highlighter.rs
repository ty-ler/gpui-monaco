//! Unicode highlighting in the editor (Monaco `UnicodeHighlighter`,
//! `unicodeHighlighter.ts`, MIT): ambiguous, invisible and (optionally) all
//! non-basic-ASCII characters get a warning border, and their hover says
//! why, with options to stop highlighting them.

use std::time::Duration;

use gpui::{AppContext as _, Context, SharedString, Task, Window};

use crate::cursor::StandardTokenType;
use crate::decorations::{DecorationId, DecorationOptions, TrackedRangeStickiness};
use crate::editor::CodeEditor;
use crate::options::UnicodeHighlightOptions;
use crate::unicode_highlight::{
    InvisibleCharacters, UnicodeHighlighterOptions, UnicodeHighlighterReason,
    UnicodeHighlighterResult, compute_unicode_highlight_reason, compute_unicode_highlights,
};

/// Monaco's `RunOnceScheduler` delay after an edit.
const UPDATE_DELAY: Duration = Duration::from_millis(250);

#[derive(Default)]
pub(crate) struct UnicodeHighlightState {
    decorations: Vec<DecorationId>,
    /// What they were computed for: the text, the parsed text (for the
    /// comment and string checks) and the options.
    key: Option<(
        text::Version,
        Option<text::Version>,
        UnicodeHighlightOptions,
    )>,
    task: Option<Task<()>>,
    /// The model it last highlighted, to reopen the banner for another.
    model: Option<gpui::EntityId>,
    /// What the "too many characters" banner is about, while it shows.
    banner: Option<BannerKind>,
    /// Its close button was clicked (Monaco `_bannerClosed`), until the
    /// model changes.
    banner_closed: bool,
}

/// The system's locale as a BCP 47 tag (`ru-RU`), from `LC_ALL`, `LC_CTYPE`
/// or `LANG`.
fn os_locale() -> Option<String> {
    ["LC_ALL", "LC_CTYPE", "LANG"]
        .iter()
        .filter_map(|var| std::env::var(var).ok())
        .find(|value| !value.is_empty() && value != "C" && value != "POSIX")
        .map(|value| {
            let tag = value.split(['.', '@']).next().unwrap_or_default();
            tag.replace('_', "-")
        })
}

/// Monaco `resolveOptions` plus `_updateHighlighter`'s locale mapping:
/// `_os` is the system's locale, `_vscode` its language.
pub(crate) fn resolve_options(options: &UnicodeHighlightOptions) -> UnicodeHighlighterOptions {
    let os = os_locale();
    UnicodeHighlighterOptions {
        non_basic_ascii: options.non_basic_ascii,
        ambiguous_characters: options.ambiguous_characters,
        invisible_characters: options.invisible_characters,
        include_comments: options.include_comments,
        include_strings: options.include_strings,
        allowed_code_points: options
            .allowed_characters
            .iter()
            .map(|c| *c as u32)
            .collect(),
        allowed_locales: options
            .allowed_locales
            .iter()
            .filter_map(|locale| match locale.as_ref() {
                "_os" => os.clone(),
                "_vscode" => os
                    .as_ref()
                    .map(|tag| tag.split('-').next().unwrap_or_default().to_string()),
                locale => Some(locale.to_string()),
            })
            .collect(),
    }
}

/// Monaco `codePointToHex`.
fn code_point_to_hex(code_point: u32) -> String {
    format!("U+{code_point:04x}")
}

/// Monaco `formatCodePointMarkdown`.
fn format_code_point_markdown(code_point: u32) -> String {
    let mut value = format!("`{}`", code_point_to_hex(code_point));
    if !InvisibleCharacters::is_invisible_character(code_point) {
        let ch = char::from_u32(code_point).unwrap_or('?');
        let code = if ch == '`' {
            "`` ` ``".to_string()
        } else {
            format!("`{ch}`")
        };
        value.push_str(&format!(" \"{code}\""));
    }
    value
}

/// A highlighted character's hover part (Monaco
/// `UnicodeHighlighterHoverParticipant`).
#[derive(Clone, Debug)]
pub(crate) struct UnicodeHover {
    /// Why it's highlighted (markdown).
    pub reason: SharedString,
    pub character: char,
    pub why: UnicodeHighlighterReason,
    pub in_comment: bool,
    pub in_string: bool,
}

impl CodeEditor {
    /// Highlights again after edits, parses and option changes (Monaco's
    /// `DocumentUnicodeHighlighter`, 250ms after an edit). Called before
    /// painting.
    pub(crate) fn refresh_unicode_highlights(&mut self, cx: &mut Context<Self>) {
        let options = self.options.unicode_highlight.clone();
        // Monaco `_updateHighlighter`: another model reopens a closed
        // banner, and another model or options hide it until the new
        // highlights are in.
        let model_id = self.model().entity_id();
        let model_changed = self.unicode_highlights.model != Some(model_id);
        let options_changed =
            (self.unicode_highlights.key.as_ref()).is_some_and(|key| key.2 != options);
        if model_changed {
            self.unicode_highlights.model = Some(model_id);
            self.unicode_highlights.banner_closed = false;
        }
        if model_changed || options_changed {
            self.update_unicode_banner(None, cx);
        }
        let model = self.model().read(cx);
        let parsed = model.syntax_snapshot().map(|syntax| syntax.text.version());
        let key = (model.buffer().version(), parsed, options.clone());
        if self.unicode_highlights.key.as_ref() == Some(&key) {
            return;
        }
        let first = self.unicode_highlights.key.is_none();
        self.unicode_highlights.key = Some(key.clone());
        let resolved = resolve_options(&options);
        if !(resolved.non_basic_ascii
            || resolved.ambiguous_characters
            || resolved.invisible_characters)
        {
            self.set_unicode_decorations(Vec::new(), cx);
            self.unicode_highlights.task = None;
            return;
        }
        let rope = model.buffer().rope().clone();
        self.unicode_highlights.task = Some(cx.spawn(async move |this, cx| {
            if !first {
                cx.background_executor().timer(UPDATE_DELAY).await;
            }
            let compute = resolved.clone();
            let result = cx
                .background_executor()
                .spawn(async move { compute_unicode_highlights(&rope, &compute, None) })
                .await;
            this.update(cx, |this, cx| {
                let model = this.model().read(cx);
                if model.buffer().version() != key.0 {
                    return;
                }
                let counts = UnicodeHighlighterResult {
                    ranges: Vec::new(),
                    has_more: result.has_more,
                    ambiguous_character_count: result.ambiguous_character_count,
                    invisible_character_count: result.invisible_character_count,
                    non_basic_ascii_character_count: result.non_basic_ascii_character_count,
                };
                // Monaco shows nothing when there are too many (and offers
                // to turn the kind off in a banner).
                let ranges = if result.has_more {
                    Vec::new()
                } else {
                    let rope = model.buffer().rope();
                    result
                        .ranges
                        .into_iter()
                        .filter(|range| {
                            // Monaco `hideInCommentTokens`/`hideInStringTokens`.
                            let offset = rope.point_utf16_to_offset(range.start);
                            match model.token_type_at(offset) {
                                StandardTokenType::Comment => resolved.include_comments,
                                StandardTokenType::String => resolved.include_strings,
                                _ => true,
                            }
                        })
                        .map(|range| {
                            rope.point_utf16_to_offset(range.start)
                                ..rope.point_utf16_to_offset(range.end)
                        })
                        .collect()
                };
                this.set_unicode_decorations(ranges, cx);
                this.update_unicode_banner(Some(&counts), cx);
                this.unicode_highlights.task = None;
            })
            .ok();
        }));
    }

    fn set_unicode_decorations(
        &mut self,
        ranges: Vec<std::ops::Range<usize>>,
        cx: &mut Context<Self>,
    ) {
        if ranges.is_empty() && self.unicode_highlights.decorations.is_empty() {
            return;
        }
        let colors = &self.theme.colors;
        // Monaco `.unicode-highlight`: a border and background.
        let options = DecorationOptions {
            border: Some(colors.unicode_highlight_border),
            background: (colors.unicode_highlight_background.a > 0.)
                .then_some(colors.unicode_highlight_background),
            stickiness: TrackedRangeStickiness::NeverGrowsWhenTypingAtEdges,
            ..Default::default()
        };
        let owner = Some(cx.entity_id());
        let old = std::mem::take(&mut self.unicode_highlights.decorations);
        let new = ranges
            .into_iter()
            .map(|range| (range, options.clone()))
            .collect();
        self.unicode_highlights.decorations = self.model().update(cx, |model, cx| {
            model.delta_decorations(owner, &old, new, cx)
        });
    }

    /// The highlighted characters, for tests.
    #[doc(hidden)]
    pub fn unicode_highlights_for_tests(&self, cx: &gpui::App) -> Vec<String> {
        let model = self.model().read(cx);
        let text = model.text();
        self.unicode_highlights
            .decorations
            .iter()
            .filter_map(|id| model.decoration(*id))
            .map(|d| text[d.range.clone()].to_string())
            .collect()
    }

    /// Monaco `UnicodeHighlighterHoverParticipant.computeSync`: a part per
    /// distinct reason among the highlights at `offset`.
    pub(crate) fn unicode_hovers(&self, offset: usize, cx: &gpui::App) -> Vec<UnicodeHover> {
        let model = self.model().read(cx);
        let options = resolve_options(&self.options.unicode_highlight);
        let mut hovers: Vec<UnicodeHover> = Vec::new();
        for id in &self.unicode_highlights.decorations {
            let Some(decoration) = model.decoration(*id) else {
                continue;
            };
            if !(decoration.range.start <= offset && offset <= decoration.range.end)
                || decoration.range.is_empty()
            {
                continue;
            }
            let text = model
                .buffer()
                .rope()
                .slice(decoration.range.clone())
                .to_string();
            let Some(ch) = text.chars().next() else {
                continue;
            };
            let Some(why) = compute_unicode_highlight_reason(&text, &options) else {
                continue;
            };
            let code_point = format_code_point_markdown(ch as u32);
            let reason = match &why {
                UnicodeHighlighterReason::Ambiguous {
                    confusable_with, ..
                } => {
                    let other = format_code_point_markdown(*confusable_with as u32);
                    if confusable_with.is_ascii() {
                        format!(
                            "The character {code_point} could be confused with the ASCII \
                             character {other}, which is more common in source code."
                        )
                    } else {
                        format!(
                            "The character {code_point} could be confused with the character \
                             {other}, which is more common in source code."
                        )
                    }
                }
                UnicodeHighlighterReason::Invisible => {
                    format!("The character {code_point} is invisible.")
                }
                UnicodeHighlighterReason::NonBasicAscii => {
                    format!("The character {code_point} is not a basic ASCII character.")
                }
            };
            if hovers.iter().any(|h| *h.reason == *reason) {
                continue;
            }
            let token = model.token_type_at(decoration.range.start);
            hovers.push(UnicodeHover {
                reason: reason.into(),
                character: ch,
                why,
                in_comment: token == StandardTokenType::Comment,
                in_string: token == StandardTokenType::String,
            });
        }
        hovers
    }

    /// Monaco `ShowExcludeOptions`: a quick pick of ways to stop
    /// highlighting `hover`'s character. The choice changes this editor's
    /// options.
    pub(crate) fn show_unicode_exclude_options(
        &mut self,
        hover: UnicodeHover,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        type Change = Box<dyn FnOnce(&mut UnicodeHighlightOptions)>;
        let mut items: Vec<(SharedString, Change)> = Vec::new();
        if let UnicodeHighlighterReason::Ambiguous {
            not_ambiguous_in_locales,
            ..
        } = &hover.why
        {
            for locale in not_ambiguous_in_locales {
                let label = format!(
                    "Allow unicode characters that are more common in the language \"{locale}\"."
                );
                let locale: SharedString = locale.clone().into();
                items.push((
                    label.into(),
                    Box::new(move |o| o.allowed_locales.push(locale)),
                ));
            }
        }
        let code_point = hover.character as u32;
        let label = if InvisibleCharacters::is_invisible_character(code_point) {
            format!(
                "Exclude {} (invisible character) from being highlighted",
                code_point_to_hex(code_point)
            )
        } else {
            format!(
                "Exclude {} \"{}\" from being highlighted",
                code_point_to_hex(code_point),
                hover.character
            )
        };
        let ch = hover.character;
        items.push((
            label.into(),
            Box::new(move |o| o.allowed_characters.push(ch)),
        ));
        if hover.in_comment {
            items.push((
                "Disable highlighting of characters in comments".into(),
                Box::new(|o| o.include_comments = false),
            ));
        } else if hover.in_string {
            items.push((
                "Disable highlighting of characters in strings".into(),
                Box::new(|o| o.include_strings = false),
            ));
        }
        items.push(match hover.why {
            UnicodeHighlighterReason::Ambiguous { .. } => (
                "Disable highlighting of ambiguous characters".into(),
                Box::new(|o| o.ambiguous_characters = false),
            ),
            UnicodeHighlighterReason::Invisible => (
                "Disable highlighting of invisible characters".into(),
                Box::new(|o| o.invisible_characters = false),
            ),
            UnicodeHighlighterReason::NonBasicAscii => (
                "Disable highlighting of non basic ASCII characters".into(),
                Box::new(|o| o.non_basic_ascii = false),
            ),
        });
        let (labels, mut changes): (Vec<SharedString>, Vec<Change>) = items.into_iter().unzip();
        self.show_quick_pick(
            Some("Configure Unicode Highlight Options".into()),
            labels,
            move |editor, ix, _, cx| {
                let change = changes.swap_remove(ix);
                let mut options = editor.options().clone();
                change(&mut options.unicode_highlight);
                editor.update_options(options, cx);
            },
            window,
            cx,
        );
    }

    /// Turns a kind of highlighting off in the editor's options (Monaco's
    /// `disableHighlightingOf*` actions, which change the user settings).
    pub(crate) fn disable_unicode_highlighting(
        &mut self,
        change: impl FnOnce(&mut UnicodeHighlightOptions),
        cx: &mut Context<Self>,
    ) {
        let mut options = self.options().clone();
        change(&mut options.unicode_highlight);
        self.update_options(options, cx);
    }

    /// Monaco `UnicodeHighlighter._updateState`: with too many characters
    /// to highlight, a banner says which kind there's most of and offers to
    /// turn it off, unless it was closed for this model.
    fn update_unicode_banner(
        &mut self,
        result: Option<&UnicodeHighlighterResult>,
        cx: &mut Context<Self>,
    ) {
        let Some(result) = result.filter(|result| result.has_more) else {
            if self.unicode_highlights.banner.take().is_some() {
                self.set_banner(None, gpui::px(0.), cx);
            }
            return;
        };
        if self.unicode_highlights.banner_closed {
            return;
        }
        let kind = BannerKind::for_result(result);
        let editor = cx.weak_entity();
        let view = cx.new(|_| UnicodeBanner {
            editor,
            kind,
            colors: self.theme.colors.clone(),
        });
        self.unicode_highlights.banner = Some(kind);
        self.set_banner(Some(view.into()), gpui::px(BANNER_HEIGHT), cx);
    }

    /// The unicode highlighter banner's message and its action's label, if
    /// it shows.
    pub fn unicode_highlight_banner(&self) -> Option<(SharedString, SharedString)> {
        let kind = self.unicode_highlights.banner?;
        Some((kind.message().into(), kind.short_label().into()))
    }

    /// Closes the unicode highlighter banner until the model changes (its
    /// close button, Monaco `onClose`). Returns whether it was showing.
    pub fn close_unicode_highlight_banner(&mut self, cx: &mut Context<Self>) -> bool {
        if self.unicode_highlights.banner.is_none() {
            return false;
        }
        self.unicode_highlights.banner_closed = true;
        self.update_unicode_banner(None, cx);
        true
    }
}

/// Monaco `BANNER_ELEMENT_HEIGHT`.
const BANNER_HEIGHT: f32 = 26.;

/// Which kind of character the banner is about.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum BannerKind {
    NonBasicAscii,
    Ambiguous,
    Invisible,
}

impl BannerKind {
    /// Monaco: the kind with the most characters, non-basic ASCII first on
    /// ties, then ambiguous.
    fn for_result(result: &UnicodeHighlighterResult) -> Self {
        let max = result
            .ambiguous_character_count
            .max(result.non_basic_ascii_character_count)
            .max(result.invisible_character_count);
        if result.non_basic_ascii_character_count >= max {
            Self::NonBasicAscii
        } else if result.ambiguous_character_count >= max {
            Self::Ambiguous
        } else {
            Self::Invisible
        }
    }

    fn message(self) -> &'static str {
        match self {
            Self::NonBasicAscii => "This document contains many non-basic ASCII unicode characters",
            Self::Ambiguous => "This document contains many ambiguous unicode characters",
            Self::Invisible => "This document contains many invisible unicode characters",
        }
    }

    /// The `disableHighlighting*` action's `shortLabel`.
    fn short_label(self) -> &'static str {
        match self {
            Self::NonBasicAscii => "Disable Non ASCII Highlight",
            Self::Ambiguous => "Disable Ambiguous Highlight",
            Self::Invisible => "Disable Invisible Highlight",
        }
    }

    fn disable(self, options: &mut UnicodeHighlightOptions) {
        match self {
            Self::NonBasicAscii => options.non_basic_ascii = false,
            Self::Ambiguous => options.ambiguous_characters = false,
            Self::Invisible => options.invisible_characters = false,
        }
    }
}

/// Monaco's `Banner` (`bannerController.ts`/`.css`): a warning icon, the
/// message, the action's link and a close button.
struct UnicodeBanner {
    editor: gpui::WeakEntity<CodeEditor>,
    kind: BannerKind,
    colors: crate::theme::EditorColors,
}

impl gpui::Render for UnicodeBanner {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl gpui::IntoElement {
        use gpui::{
            InteractiveElement as _, ParentElement as _, StatefulInteractiveElement as _,
            Styled as _, div, px,
        };
        let kind = self.kind;
        let link_editor = self.editor.clone();
        let close_editor = self.editor.clone();
        div()
            .id("unicode-highlight-banner")
            .size_full()
            .flex()
            .items_center()
            .bg(self.colors.banner_background)
            .text_color(self.colors.banner_foreground)
            .text_size(px(12.))
            .child(
                div()
                    .pl(px(10.))
                    .pr(px(6.))
                    .text_color(self.colors.banner_icon_foreground)
                    .child("⚠"),
            )
            .child(
                div()
                    .whitespace_nowrap()
                    .overflow_hidden()
                    .child(kind.message()),
            )
            .child(
                div().flex_1().flex_shrink_0().mx(px(4.)).child(
                    div()
                        .id("unicode-highlight-banner-action")
                        .ml(px(12.))
                        .p(px(3.))
                        .underline()
                        .cursor_pointer()
                        .child(kind.short_label())
                        .on_mouse_down(gpui::MouseButton::Left, move |_, _, cx| {
                            cx.stop_propagation();
                            if let Some(editor) = link_editor.upgrade() {
                                editor.update(cx, |editor, cx| {
                                    editor.disable_unicode_highlighting(|o| kind.disable(o), cx)
                                });
                            }
                        }),
                ),
            )
            .child(
                div()
                    .id("unicode-highlight-banner-close")
                    .px(px(10.))
                    .cursor_pointer()
                    .child("✕")
                    .tooltip(crate::tooltip::tooltip("Close Banner", &self.colors))
                    .on_mouse_down(gpui::MouseButton::Left, move |_, _, cx| {
                        cx.stop_propagation();
                        if let Some(editor) = close_editor.upgrade() {
                            editor.update(cx, |editor, cx| {
                                editor.close_unicode_highlight_banner(cx);
                            });
                        }
                    }),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_banner_names_the_most_common_kind() {
        let result = |ambiguous, invisible, non_basic_ascii| UnicodeHighlighterResult {
            has_more: true,
            ambiguous_character_count: ambiguous,
            invisible_character_count: invisible,
            non_basic_ascii_character_count: non_basic_ascii,
            ..Default::default()
        };
        assert_eq!(
            BannerKind::for_result(&result(5, 1, 2)),
            BannerKind::Ambiguous
        );
        assert_eq!(
            BannerKind::for_result(&result(1, 5, 2)),
            BannerKind::Invisible
        );
        assert_eq!(
            BannerKind::for_result(&result(5, 5, 5)),
            BannerKind::NonBasicAscii
        );
        assert_eq!(
            BannerKind::for_result(&result(5, 5, 1)),
            BannerKind::Ambiguous
        );
    }

    #[test]
    fn code_points_format_like_monaco() {
        assert_eq!(format_code_point_markdown('а' as u32), "`U+0430` \"`а`\"");
        assert_eq!(format_code_point_markdown(0x200b), "`U+200b`");
        assert_eq!(
            format_code_point_markdown('`' as u32),
            "`U+0060` \"`` ` ``\""
        );
    }
}
