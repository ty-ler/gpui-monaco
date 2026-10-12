//! Monaco's global openers (`registerLinkOpener` and `registerEditorOpener`
//! in `standalone/browser/standaloneEditor.ts`): hosts handle followed
//! links and locations in other models once, for every editor (the ones
//! inside peek views and diff editors included), instead of subscribing to
//! each editor's [`CodeEditorEvent::OpenLink`] and
//! [`CodeEditorEvent::OpenLocation`].
//!
//! Links go through [`open_url`], the port of Monaco's `OpenerService.open`
//! with its default openers: the link openers, then the browser for web
//! and mail links, then `command:` links, then Monaco's `EditorOpener`,
//! which moves the focused editor to a link into its own model and offers
//! links to other models to the editor openers.

use std::ops::Range;
use std::rc::Rc;

use gpui::{App, Context, Entity, Global, SharedString};
use text::PointUtf16;

use crate::cursor::Selection;
use crate::editor::{CodeEditor, CodeEditorEvent};
use crate::providers::Location;
use crate::registry::Disposable;
use crate::view::reveal::{RevealType, ScrollType};

type LinkOpener = Rc<dyn Fn(&str, &mut App) -> bool>;
type EditorOpener =
    Rc<dyn Fn(&Entity<CodeEditor>, &str, Option<Range<PointUtf16>>, &mut App) -> bool>;

#[derive(Default)]
struct Openers {
    links: Vec<(u64, LinkOpener)>,
    editors: Vec<(u64, EditorOpener)>,
    next_id: u64,
}

impl Global for Openers {}

/// Registers `opener` for links an editor follows (Cmd/Ctrl+click or
/// `editor.action.openLink`; Monaco `registerLinkOpener`). It gets the
/// link, relative `file://./` ones resolved against the model's uri, and
/// returns whether it opened it. The most recently registered opener is
/// asked first.
///
/// When none opens a link, the editor opens it as Monaco's opener service
/// does: `http`, `https` and `mailto` links in the browser, `command:`
/// links by running the command, and a link into the editor's own model
/// (`file:///a.rs#L10,5`, Monaco's `#L<line>[,<col>][-L<line>[,<col>]]`
/// fragments) by selecting and centering it; a link to another model goes
/// to the [editor openers](register_editor_opener). Anything still not
/// opened is reported as [`CodeEditorEvent::OpenLink`].
///
/// Links clicked in markdown (hovers, suggest details, parameter hints,
/// messages) go through the openers too, as they go through Monaco's
/// opener service, `command:` links of untrusted strings included. When
/// none opens one, it's opened the same way in the focused editor, except
/// that only a trusted string's `command:` links run, and what isn't
/// opened is dropped.
///
/// Openers run after the editor's update, so they may update the editor.
pub fn register_link_opener(
    cx: &mut App,
    opener: impl Fn(&str, &mut App) -> bool + 'static,
) -> Disposable {
    let openers = cx.default_global::<Openers>();
    openers.next_id += 1;
    let id = openers.next_id;
    openers.links.push((id, Rc::new(opener)));
    Disposable::new(move |cx| {
        cx.default_global::<Openers>()
            .links
            .retain(|(other, _)| *other != id);
    })
}

/// Registers `opener` for models other than the editor's (Monaco
/// `registerEditorOpener`): go to definition, references, followed links
/// and the like leading to another uri. It gets the editor that asked, the
/// uri and the range to select (Monaco's `selectionOrPosition`; a position
/// is an empty range), and returns whether it opened it. The most recently
/// registered opener is asked first; when none opens it, the editor emits
/// [`CodeEditorEvent::OpenLocation`] (or
/// [`CodeEditorEvent::OpenLocationToSide`]), or for a followed link
/// [`CodeEditorEvent::OpenLink`], as before.
///
/// Openers run after the editor's update, so they may update the editor
/// (e.g. [`CodeEditor::set_model`] and then select the range).
pub fn register_editor_opener(
    cx: &mut App,
    opener: impl Fn(&Entity<CodeEditor>, &str, Option<Range<PointUtf16>>, &mut App) -> bool + 'static,
) -> Disposable {
    let openers = cx.default_global::<Openers>();
    openers.next_id += 1;
    let id = openers.next_id;
    openers.editors.push((id, Rc::new(opener)));
    Disposable::new(move |cx| {
        cx.default_global::<Openers>()
            .editors
            .retain(|(other, _)| *other != id);
    })
}

fn link_openers(cx: &App) -> Vec<LinkOpener> {
    cx.try_global::<Openers>()
        .map(|o| o.links.iter().rev().map(|(_, o)| o.clone()).collect())
        .unwrap_or_default()
}

fn editor_openers(cx: &App) -> Vec<EditorOpener> {
    cx.try_global::<Openers>()
        .map(|o| o.editors.iter().rev().map(|(_, o)| o.clone()).collect())
        .unwrap_or_default()
}

/// A link fragment's selection (Monaco `ITextEditorSelection`, 1-based):
/// the start, and the end when the fragment has one.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct LinkSelection {
    start: (u32, u32),
    end: Option<(u32, u32)>,
}

/// Monaco `extractSelection`: splits a `#L10`, `#L10,5`, `#10,5-12` or
/// `#L10,5-L12,3` fragment off `url` (a missing start column is 1, a
/// missing end column too). Other fragments stay in the uri.
pub(crate) fn extract_selection(url: &str) -> (&str, Option<LinkSelection>) {
    let Some((uri, fragment)) = url.split_once('#') else {
        return (url, None);
    };
    // `/^L?(\d+)(?:,(\d+))?(-L?(\d+)(?:,(\d+))?)?/`
    fn number(text: &str) -> Option<(u32, &str)> {
        let end = text
            .find(|c: char| !c.is_ascii_digit())
            .unwrap_or(text.len());
        Some((text[..end].parse().ok()?, &text[end..]))
    }
    fn line_and_column(text: &str) -> Option<((u32, Option<u32>), &str)> {
        let text = text.strip_prefix('L').unwrap_or(text);
        let (line, rest) = number(text)?;
        match rest.strip_prefix(',').and_then(number) {
            Some((column, rest)) => Some(((line, Some(column)), rest)),
            None => Some(((line, None), rest)),
        }
    }
    let Some(((line, column), rest)) = line_and_column(fragment) else {
        return (url, None);
    };
    let end = rest
        .strip_prefix('-')
        .and_then(line_and_column)
        .map(|((line, column), _)| (line, column.unwrap_or(1)));
    let selection = LinkSelection {
        start: (line, column.unwrap_or(1)),
        end,
    };
    (uri, Some(selection))
}

impl LinkSelection {
    /// The selection in `rope`, clipped to the text: a range when the
    /// fragment had an end, else an empty one.
    fn range(&self, rope: &text::Rope) -> Range<PointUtf16> {
        let point = |(line, column): (u32, u32)| {
            rope.clip_point_utf16(
                PointUtf16::new(line.saturating_sub(1), column.saturating_sub(1)),
                text::Bias::Left,
            )
        };
        let start = point(self.start);
        start..self.end.map_or(start, point)
    }
}

/// Monaco `OpenerService.open(url, { allowCommands })` with the default
/// openers: the [link openers](register_link_opener), most recent first;
/// then `http`, `https` and `mailto` links in the browser; then `command:`
/// links, run when `allow_commands` and dropped otherwise; then Monaco's
/// `EditorOpener` with `editor` (Monaco's focused editor): a link into its
/// own model selects the fragment's range and centers it (the standalone
/// `doOpenEditor`), and a link to another model goes to the
/// [editor openers](register_editor_opener). `internal:` links aren't
/// openable. Whether something opened `url`.
pub(crate) fn open_url(
    url: &str,
    allow_commands: bool,
    editor: Option<&Entity<CodeEditor>>,
    cx: &mut App,
) -> bool {
    let scheme = url
        .split_once(':')
        .map(|(scheme, _)| scheme.to_ascii_lowercase());
    if scheme.as_deref() == Some("internal") {
        return false;
    }
    if link_openers(cx).iter().any(|open| open(url, cx)) {
        return true;
    }
    if matches!(scheme.as_deref(), Some("http" | "https" | "mailto")) {
        cx.open_url(url);
        return true;
    }
    if scheme.as_deref() == Some("command") {
        // Monaco `CommandOpener`: disabled command links are swallowed so
        // that no other opener gets them.
        if allow_commands
            && let Some(command) = crate::markdown::command_link(url)
            && let Err(error) = crate::execute_command(cx, &command)
        {
            log::warn!("command link: {error}");
        }
        return true;
    }
    let Some(editor) = editor else {
        return false;
    };
    let (uri, selection) = extract_selection(url);
    if editor.read(cx).model().read(cx).is_at(Some(uri)) {
        editor.update(cx, |editor, cx| {
            let Some(selection) = selection else {
                return;
            };
            let range = selection.range(editor.model().read(cx).buffer().rope());
            editor.set_selection(
                Selection {
                    anchor: range.start,
                    head: range.end,
                },
                cx,
            );
            editor.reveal_range_with_scroll_type(
                range,
                RevealType::Center,
                ScrollType::Immediate,
                cx,
            );
        });
        return true;
    }
    let range = selection.map(|selection| {
        crate::model::model_for_uri(cx, uri)
            .map(|model| selection.range(model.read(cx).buffer().rope()))
            .unwrap_or_else(|| {
                let point = |(line, column): (u32, u32)| {
                    PointUtf16::new(line.saturating_sub(1), column.saturating_sub(1))
                };
                let start = point(selection.start);
                start..selection.end.map_or(start, point)
            })
    });
    editor_openers(cx)
        .iter()
        .any(|open| open(editor, uri, range.clone(), cx))
}

impl CodeEditor {
    /// Opens a followed link after the update with [`open_url`] (this
    /// editor being the focused one), emitting
    /// [`CodeEditorEvent::OpenLink`] if nothing opens it.
    pub(crate) fn open_link_with_openers(
        &mut self,
        url: SharedString,
        allow_commands: bool,
        cx: &mut Context<Self>,
    ) {
        let editor = cx.entity();
        cx.defer(move |cx| {
            if !open_url(&url, allow_commands, Some(&editor), cx) {
                editor.update(cx, |_, cx| cx.emit(CodeEditorEvent::OpenLink(url)));
            }
        });
    }

    /// Offers `location` (in another model) to the editor openers,
    /// emitting [`CodeEditorEvent::OpenLocation`], or
    /// [`CodeEditorEvent::OpenLocationToSide`] with `to_side`, if none
    /// opens it.
    pub(crate) fn open_location_with_openers(
        &mut self,
        location: Location,
        to_side: bool,
        cx: &mut Context<Self>,
    ) {
        self.open_location_with_openers_then(location, to_side, |_, _| {}, cx);
    }

    /// [`Self::open_location_with_openers`], then `opened` on this editor
    /// when an opener opened it (Monaco's `openCodeEditor` resolving to
    /// the source editor).
    pub(crate) fn open_location_with_openers_then(
        &mut self,
        location: Location,
        to_side: bool,
        opened: impl FnOnce(&mut CodeEditor, &mut Context<CodeEditor>) + 'static,
        cx: &mut Context<Self>,
    ) {
        let event = move |location| {
            if to_side {
                CodeEditorEvent::OpenLocationToSide(location)
            } else {
                CodeEditorEvent::OpenLocation(location)
            }
        };
        if editor_openers(cx).is_empty() {
            cx.emit(event(location));
            return;
        }
        let uri = location
            .uri
            .clone()
            .or_else(|| self.model().read(cx).uri().cloned())
            .unwrap_or_default();
        let selection = location.selection_range();
        let editor = cx.entity();
        cx.defer(move |cx| {
            let handled = editor_openers(cx)
                .iter()
                .any(|open| open(&editor, &uri, Some(selection.clone()), cx));
            if handled {
                editor.update(cx, opened);
            } else {
                editor.update(cx, |_, cx| cx.emit(event(location)));
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn selection(start: (u32, u32), end: Option<(u32, u32)>) -> Option<LinkSelection> {
        Some(LinkSelection { start, end })
    }

    /// Monaco `extractSelection` (`platform/opener/common/opener.ts`).
    #[test]
    fn link_fragments() {
        assert_eq!(
            extract_selection("file:///a.rs#L10"),
            ("file:///a.rs", selection((10, 1), None))
        );
        assert_eq!(
            extract_selection("file:///a.rs#L10,5"),
            ("file:///a.rs", selection((10, 5), None))
        );
        assert_eq!(
            extract_selection("file:///a.rs#10,5-12"),
            ("file:///a.rs", selection((10, 5), Some((12, 1))))
        );
        assert_eq!(
            extract_selection("file:///a.rs#L10,5-L12,3"),
            ("file:///a.rs", selection((10, 5), Some((12, 3))))
        );
        assert_eq!(
            extract_selection("file:///a.rs#section"),
            ("file:///a.rs#section", None)
        );
        assert_eq!(extract_selection("file:///a.rs"), ("file:///a.rs", None));
    }
}
