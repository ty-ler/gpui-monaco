//! Monaco's global openers (`registerLinkOpener` and `registerEditorOpener`
//! in `standalone/browser/standaloneEditor.ts`): hosts handle followed
//! links and locations in other models once, for every editor (the ones
//! inside peek views and diff editors included), instead of subscribing to
//! each editor's [`CodeEditorEvent::OpenLink`] and
//! [`CodeEditorEvent::OpenLocation`].

use std::ops::Range;
use std::rc::Rc;

use gpui::{App, Context, Entity, Global, SharedString};
use text::PointUtf16;

use crate::editor::{CodeEditor, CodeEditorEvent};
use crate::providers::Location;
use crate::registry::Disposable;

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
/// asked first; when none opens the link, the editor emits
/// [`CodeEditorEvent::OpenLink`] as before.
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
/// `registerEditorOpener`): go to definition, references and the like
/// leading to another uri. It gets the editor that asked, the uri and the
/// range to select (Monaco's `selectionOrPosition`), and returns whether it
/// opened it. The most recently registered opener is asked first; when
/// none opens it, the editor emits [`CodeEditorEvent::OpenLocation`] (or
/// [`CodeEditorEvent::OpenLocationToSide`]) as before.
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

impl CodeEditor {
    /// Offers `url` to the link openers, emitting
    /// [`CodeEditorEvent::OpenLink`] if none opens it.
    pub(crate) fn open_link_with_openers(&mut self, url: SharedString, cx: &mut Context<Self>) {
        if link_openers(cx).is_empty() {
            cx.emit(CodeEditorEvent::OpenLink(url));
            return;
        }
        let editor = cx.weak_entity();
        cx.defer(move |cx| {
            if link_openers(cx).iter().any(|open| open(&url, cx)) {
                return;
            }
            editor
                .update(cx, |_, cx| cx.emit(CodeEditorEvent::OpenLink(url)))
                .ok();
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
            let opened = editor_openers(cx)
                .iter()
                .any(|open| open(&editor, &uri, Some(selection.clone()), cx));
            if !opened {
                editor.update(cx, |_, cx| cx.emit(event(location)));
            }
        });
    }
}
