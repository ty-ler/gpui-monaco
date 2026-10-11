//! Dropping files and choosing how to paste (Monaco `contrib/dropOrPasteInto`,
//! MIT): providers offer edits for dropped or pasted data. The first one is
//! applied, and a small button after it switches to the others.

use std::ops::Range;
use std::path::PathBuf;
use std::sync::Arc;

use futures::FutureExt as _;
use gpui::{
    App, AppContext as _, Context, Global, IntoElement, ParentElement, Render, SharedString,
    Styled, Task, Window, div, px,
};
use text::PointUtf16;

use crate::cursor::Selection;
use crate::editor::CodeEditor;
use crate::providers::{
    DataTransfer, DataTransferItem, DocumentDropEditProvider, DocumentDropOrPasteEdit,
    DocumentPasteEditProvider, DocumentPasteTriggerKind, EditInsertText, EditYieldTo,
    ProviderContext, edit_kind_contains, matches_mime_type,
};
use crate::snippet::parser::SnippetParser;
use crate::snippet_session::SnippetInsertOptions;
use crate::widgets::{ContentWidget, ContentWidgetPreference, WidgetId};

/// Monaco `Mimes.text`.
pub(crate) const TEXT_PLAIN: &str = "text/plain";
/// Monaco `Mimes.uriList`.
pub(crate) const URI_LIST: &str = "text/uri-list";
/// Dropped files.
pub(crate) const FILES: &str = "files";

/// Monaco `UriList.create`: `file://` URIs, one per line.
pub(crate) fn uri_list(paths: &[PathBuf]) -> String {
    paths
        .iter()
        .map(|path| {
            let mut uri = String::from("file://");
            for byte in path.to_string_lossy().bytes() {
                match byte {
                    b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'/' | b'-' | b'_' | b'.' | b'~' => {
                        uri.push(byte as char)
                    }
                    _ => uri.push_str(&format!("%{byte:02X}")),
                }
            }
            uri
        })
        .collect::<Vec<_>>()
        .join("\r\n")
}

/// Monaco `extractUriList`: each entry of a `text/uri-list`, as a file path
/// for `file:` URIs, else as written.
fn extract_uri_list(data: &DataTransfer) -> Vec<(Option<PathBuf>, String)> {
    let Some(list) = data.text(URI_LIST) else {
        return Vec::new();
    };
    list.split(['\r', '\n'])
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .map(|line| {
            let path = line.strip_prefix("file://").map(|path| {
                let bytes = path.as_bytes();
                let mut decoded = Vec::with_capacity(bytes.len());
                let mut i = 0;
                while i < bytes.len() {
                    if bytes[i] == b'%'
                        && let Some(byte) = path
                            .get(i + 1..i + 3)
                            .and_then(|hex| u8::from_str_radix(hex, 16).ok())
                    {
                        decoded.push(byte);
                        i += 3;
                    } else {
                        decoded.push(bytes[i]);
                        i += 1;
                    }
                }
                PathBuf::from(String::from_utf8_lossy(&decoded).into_owned())
            });
            (path, line.to_string())
        })
        .collect()
}

/// Monaco `DefaultTextPasteOrDropEditProvider`: the text as is.
struct DefaultTextProvider;

impl DefaultTextProvider {
    fn edit(data: &DataTransfer) -> Option<DocumentDropOrPasteEdit> {
        let text = data.text(TEXT_PLAIN)?;
        // The uri-list usually holds the same text.
        if data.has(URI_LIST) {
            return None;
        }
        Some(DocumentDropOrPasteEdit {
            title: "Insert Plain Text".into(),
            kind: Some("text.plain".into()),
            handled_mime_type: Some(TEXT_PLAIN.into()),
            insert_text: EditInsertText::Text(text.to_string()),
            ..Default::default()
        })
    }
}

impl DocumentDropEditProvider for DefaultTextProvider {
    fn drop_mime_types(&self) -> Option<Vec<SharedString>> {
        Some(vec![TEXT_PLAIN.into()])
    }

    fn provide_document_drop_edits(
        &self,
        _: &ProviderContext,
        _: PointUtf16,
        data: &DataTransfer,
        _: &mut App,
    ) -> Task<anyhow::Result<Vec<DocumentDropOrPasteEdit>>> {
        Task::ready(Ok(Self::edit(data).into_iter().collect()))
    }
}

impl DocumentPasteEditProvider for DefaultTextProvider {
    fn paste_mime_types(&self) -> Vec<SharedString> {
        vec![TEXT_PLAIN.into()]
    }

    fn provided_paste_edit_kinds(&self) -> Vec<SharedString> {
        vec!["text.plain".into()]
    }

    fn provide_document_paste_edits(
        &self,
        _: &ProviderContext,
        _: &[Range<PointUtf16>],
        data: &DataTransfer,
        _: DocumentPasteTriggerKind,
        _: &mut App,
    ) -> Task<anyhow::Result<Vec<DocumentDropOrPasteEdit>>> {
        Task::ready(Ok(Self::edit(data).into_iter().collect()))
    }

    fn is_default_text_provider(&self) -> bool {
        true
    }
}

/// Monaco `PathProvider`: the paths (or URIs) of a uri-list, separated by
/// spaces.
struct PathProvider;

impl PathProvider {
    fn edit(data: &DataTransfer) -> Option<DocumentDropOrPasteEdit> {
        let entries = extract_uri_list(data);
        if entries.is_empty() {
            return None;
        }
        let uris = entries.iter().filter(|(path, _)| path.is_none()).count();
        let insert_text = entries
            .iter()
            .map(|(path, original)| match path {
                Some(path) => path.to_string_lossy().into_owned(),
                None => original.clone(),
            })
            .collect::<Vec<_>>()
            .join(" ");
        let title = match (uris > 0, entries.len() > 1) {
            (true, true) => "Insert Uris",
            (true, false) => "Insert Uri",
            (false, true) => "Insert Paths",
            (false, false) => "Insert Path",
        };
        Some(DocumentDropOrPasteEdit {
            title: title.into(),
            kind: Some("uri.path.absolute".into()),
            handled_mime_type: Some(URI_LIST.into()),
            insert_text: EditInsertText::Text(insert_text),
            ..Default::default()
        })
    }
}

impl DocumentDropEditProvider for PathProvider {
    fn drop_mime_types(&self) -> Option<Vec<SharedString>> {
        Some(vec![URI_LIST.into()])
    }

    fn provide_document_drop_edits(
        &self,
        _: &ProviderContext,
        _: PointUtf16,
        data: &DataTransfer,
        _: &mut App,
    ) -> Task<anyhow::Result<Vec<DocumentDropOrPasteEdit>>> {
        Task::ready(Ok(Self::edit(data).into_iter().collect()))
    }
}

impl DocumentPasteEditProvider for PathProvider {
    fn paste_mime_types(&self) -> Vec<SharedString> {
        vec![URI_LIST.into()]
    }

    fn provided_paste_edit_kinds(&self) -> Vec<SharedString> {
        vec!["uri.path.absolute".into()]
    }

    fn provide_document_paste_edits(
        &self,
        _: &ProviderContext,
        _: &[Range<PointUtf16>],
        data: &DataTransfer,
        _: DocumentPasteTriggerKind,
        _: &mut App,
    ) -> Task<anyhow::Result<Vec<DocumentDropOrPasteEdit>>> {
        Task::ready(Ok(Self::edit(data).into_iter().collect()))
    }
}

/// Monaco `PasteHtmlProvider`: the HTML source, only for "Paste As...".
struct PasteHtmlProvider;

impl DocumentPasteEditProvider for PasteHtmlProvider {
    fn paste_mime_types(&self) -> Vec<SharedString> {
        vec!["text/html".into()]
    }

    fn provided_paste_edit_kinds(&self) -> Vec<SharedString> {
        vec!["html".into()]
    }

    fn provide_document_paste_edits(
        &self,
        _: &ProviderContext,
        _: &[Range<PointUtf16>],
        data: &DataTransfer,
        trigger: DocumentPasteTriggerKind,
        _: &mut App,
    ) -> Task<anyhow::Result<Vec<DocumentDropOrPasteEdit>>> {
        let edit = (trigger == DocumentPasteTriggerKind::PasteAs)
            .then(|| data.text("text/html"))
            .flatten()
            .filter(|html| !html.is_empty())
            .map(|html| DocumentDropOrPasteEdit {
                title: "Insert HTML".into(),
                kind: Some("html".into()),
                insert_text: EditInsertText::Text(html.to_string()),
                yield_to: vec![EditYieldTo::MimeType(TEXT_PLAIN.into())],
                ..Default::default()
            });
        Task::ready(Ok(edit.into_iter().collect()))
    }
}

/// Monaco `DefaultDropProvidersFeature` (the relative path provider needs a
/// workspace, which the editor doesn't have).
pub(crate) fn default_drop_providers() -> Vec<Arc<dyn DocumentDropEditProvider>> {
    vec![Arc::new(DefaultTextProvider), Arc::new(PathProvider)]
}

/// Monaco `DefaultPasteProvidersFeature`.
pub(crate) fn default_paste_providers() -> Vec<Arc<dyn DocumentPasteEditProvider>> {
    vec![
        Arc::new(DefaultTextProvider),
        Arc::new(PathProvider),
        Arc::new(PasteHtmlProvider),
    ]
}

/// Monaco `sortEditsByYieldTo`: a topological sort putting edits after the
/// ones they yield to, otherwise keeping their order.
pub(crate) fn sort_edits_by_yield_to(
    edits: Vec<DocumentDropOrPasteEdit>,
) -> Vec<DocumentDropOrPasteEdit> {
    let yields_to = |y: &EditYieldTo, other: &DocumentDropOrPasteEdit| match y {
        EditYieldTo::MimeType(mime) => other.handled_mime_type.as_ref() == Some(mime),
        EditYieldTo::Kind(kind) => other
            .kind
            .as_ref()
            .is_some_and(|other| edit_kind_contains(kind, other)),
    };
    let after: Vec<Vec<usize>> = edits
        .iter()
        .enumerate()
        .map(|(ix, edit)| {
            let mut targets = Vec::new();
            for y in &edit.yield_to {
                for (other_ix, other) in edits.iter().enumerate() {
                    if other_ix != ix && yields_to(y, other) {
                        targets.push(other_ix);
                    }
                }
            }
            targets
        })
        .collect();
    if after.iter().all(Vec::is_empty) {
        return edits;
    }
    fn visit(
        nodes: &[usize],
        after: &[Vec<usize>],
        visited: &mut Vec<bool>,
        stack: &mut Vec<usize>,
        out: &mut Vec<usize>,
    ) {
        for &node in nodes {
            if stack.contains(&node) {
                // A cycle: keep the rest as is.
                for &rest in nodes {
                    if !visited[rest] {
                        visited[rest] = true;
                        out.push(rest);
                    }
                }
                return;
            }
            if visited[node] {
                continue;
            }
            stack.push(node);
            visit(&after[node], after, visited, stack, out);
            stack.pop();
            if !visited[node] {
                visited[node] = true;
                out.push(node);
            }
        }
    }
    let mut visited = vec![false; edits.len()];
    let mut order = Vec::new();
    let all: Vec<usize> = (0..edits.len()).collect();
    visit(&all, &after, &mut visited, &mut Vec::new(), &mut order);
    let mut edits: Vec<Option<DocumentDropOrPasteEdit>> = edits.into_iter().map(Some).collect();
    order
        .into_iter()
        .filter_map(|ix| edits[ix].take())
        .collect()
}

/// What an applied edit came from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PostEditKind {
    Drop,
    Paste,
}

/// Monaco `PostEditWidget`: the applied edit, its alternatives and the
/// button that switches between them.
pub(crate) struct PostEdit {
    kind: PostEditKind,
    edits: Vec<DocumentDropOrPasteEdit>,
    active: usize,
    ranges: Vec<Range<PointUtf16>>,
    /// The text version right after applying it.
    version: text::Version,
    widget: WidgetId,
}

#[derive(Default)]
pub(crate) struct DropPasteState {
    pub(crate) task: Option<Task<()>>,
    pub(crate) post_edit: Option<PostEdit>,
}

/// The post-edit button.
struct PostEditButton {
    editor: gpui::WeakEntity<CodeEditor>,
    label: SharedString,
    colors: crate::theme::EditorColors,
}

impl Render for PostEditButton {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let editor = self.editor.clone();
        div()
            .id("post-edit")
            .cursor_pointer()
            .px(px(4.))
            .rounded(px(3.))
            .bg(self.colors.widget_background)
            .border_1()
            .border_color(self.colors.widget_border)
            .text_color(self.colors.widget_foreground)
            .text_size(px(11.))
            .on_mouse_down(gpui::MouseButton::Left, move |_, window, cx| {
                cx.stop_propagation();
                if let Some(editor) = editor.upgrade() {
                    editor.update(cx, |editor, cx| editor.show_post_edit_selector(window, cx));
                }
            })
            .child(self.label.clone())
    }
}

use gpui::InteractiveElement as _;

impl CodeEditor {
    fn provider_context_now(&self, cx: &App) -> ProviderContext {
        self.model().read(cx).provider_context()
    }

    /// Monaco `DropIntoEditorController.onDropIntoEditor`: runs the drop
    /// providers for files dropped at `position` and applies the first edit.
    pub fn drop_files(&mut self, paths: &[PathBuf], position: PointUtf16, cx: &mut Context<Self>) {
        let mut data = DataTransfer::default();
        data.insert(URI_LIST, DataTransferItem::Text(uri_list(paths)));
        data.insert(FILES, DataTransferItem::Files(paths.to_vec()));
        self.drop_data(data, position, cx);
    }

    /// [`drop_files`](Self::drop_files) for any data.
    pub fn drop_data(&mut self, data: DataTransfer, position: PointUtf16, cx: &mut Context<Self>) {
        if !self.options.drop_into_editor.enabled || self.options.read_only {
            return;
        }
        self.clear_post_edit(cx);
        self.set_selections(&[Selection::collapsed(position)], cx);
        let context = self.provider_context_now(cx);
        let providers: Vec<_> = crate::providers::drop_edit_providers(cx, self.provider_target(cx))
            .into_iter()
            .filter(|provider| {
                provider
                    .drop_mime_types()
                    .is_none_or(|mimes| mimes.iter().any(|mime| data.matches(mime)))
            })
            .collect();
        let tasks: Vec<_> = providers
            .iter()
            .map(|provider| provider.provide_document_drop_edits(&context, position, &data, cx))
            .collect();
        let version = self.snapshot.version();
        self.start_cancellable(crate::editor_state::CancellableOperation::DropOrPaste);
        self.drop_paste.task = Some(cx.spawn(async move |this, cx| {
            let mut edits = Vec::new();
            for task in tasks {
                if let Ok(result) = task.await {
                    edits.extend(result);
                }
            }
            let edits = sort_edits_by_yield_to(edits);
            this.update(cx, |this, cx| {
                // Monaco cancels when the text changed meanwhile.
                if this.snapshot.version() != version || edits.is_empty() {
                    return;
                }
                let show = this.options.drop_into_editor.show_drop_selector;
                this.apply_post_edit(
                    PostEditKind::Drop,
                    vec![position..position],
                    edits,
                    0,
                    show,
                    cx,
                );
            })
            .ok();
        }));
    }

    /// Monaco `CopyPasteController.handlePaste`: when providers other than
    /// the plain text one can handle the clipboard, they decide how it's
    /// pasted. Returns whether they took over the paste.
    pub(crate) fn paste_with_providers(
        &mut self,
        data: DataTransfer,
        fallback: gpui::ClipboardItem,
        cx: &mut Context<Self>,
    ) -> bool {
        if self.options.read_only || !self.options.paste_as.enabled {
            return false;
        }
        let copy_mime_types = copy_mime_types(&fallback);
        let providers = self.paste_providers_for(&data, &copy_mime_types, None, cx);
        // Only our plain text provider: the normal paste.
        if providers.is_empty() || providers.iter().all(|p| p.is_default_text_provider()) {
            return false;
        }
        self.close_message(cx);
        self.clear_post_edit(cx);
        let ranges: Vec<_> = self.selections().iter().map(|s| s.range()).collect();
        let requests = self.paste_requests(
            providers,
            &ranges,
            data,
            &fallback,
            DocumentPasteTriggerKind::Automatic,
            cx,
        );
        let version = self.snapshot.version();
        self.start_cancellable(crate::editor_state::CancellableOperation::DropOrPaste);
        self.drop_paste.task = Some(cx.spawn(async move |this, cx| {
            let tasks = match requests {
                PasteRequests::Started(tasks) => tasks,
                PasteRequests::AfterCopyData(pending) => {
                    let (providers, data, ranges) = pending.merge().await;
                    let Ok(tasks) = this.update(cx, |this, cx| {
                        this.paste_tasks(
                            &providers,
                            &ranges,
                            &data,
                            DocumentPasteTriggerKind::Automatic,
                            cx,
                        )
                    }) else {
                        return;
                    };
                    tasks
                }
            };
            let edits = collect_edits(tasks).await;
            this.update(cx, |this, cx| {
                if this.snapshot.version() != version {
                    return;
                }
                // Only the plain text edit: the normal paste.
                if edits.is_empty()
                    || (edits.len() == 1 && edits[0].kind.as_deref() == Some("text.plain"))
                {
                    this.paste_item_for_providers(fallback, cx);
                    return;
                }
                let show = this.options.paste_as.show_paste_selector;
                this.apply_post_edit(PostEditKind::Paste, ranges, edits, 0, show, cx);
            })
            .ok();
        }));
        true
    }

    /// The paste providers for `data`, plus the MIME types data prepared at
    /// copy time will add (Monaco's `allPotentialMimeTypes`).
    fn paste_providers_for(
        &self,
        data: &DataTransfer,
        copy_mime_types: &[String],
        only: Option<&str>,
        cx: &App,
    ) -> Vec<Arc<dyn DocumentPasteEditProvider>> {
        crate::providers::paste_edit_providers(cx, self.provider_target(cx))
            .into_iter()
            .filter(|provider| {
                provider.paste_mime_types().iter().any(|mime| {
                    matches_mime_type(
                        mime,
                        data.mime_types()
                            .map(|m| m.as_ref())
                            .chain(copy_mime_types.iter().map(String::as_str)),
                    )
                }) && only.is_none_or(|only| {
                    provider
                        .provided_paste_edit_kinds()
                        .iter()
                        .any(|kind| edit_kind_contains(only, kind))
                })
            })
            .collect()
    }

    fn paste_tasks(
        &self,
        providers: &[Arc<dyn DocumentPasteEditProvider>],
        ranges: &[Range<PointUtf16>],
        data: &DataTransfer,
        trigger: DocumentPasteTriggerKind,
        cx: &mut App,
    ) -> Vec<Task<anyhow::Result<Vec<DocumentDropOrPasteEdit>>>> {
        let context = self.provider_context_now(cx);
        providers
            .iter()
            .map(|p| p.provide_document_paste_edits(&context, ranges, data, trigger, cx))
            .collect()
    }

    /// Monaco `editor.action.pasteAs`: every way to paste the clipboard, in
    /// a quick pick. With `only` (an edit kind, such as `text.plain`), the
    /// matching edit is applied directly when there's just one.
    pub fn paste_as(
        &mut self,
        only: Option<SharedString>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.options.read_only {
            return;
        }
        let Some(item) = cx.read_from_clipboard() else {
            return;
        };
        let data = clipboard_data(&item);
        let copy_mime_types = copy_mime_types(&item);
        let providers = self.paste_providers_for(&data, &copy_mime_types, only.as_deref(), cx);
        let ranges: Vec<_> = self.selections().iter().map(|s| s.range()).collect();
        let requests = self.paste_requests(
            providers,
            &ranges,
            data,
            &item,
            DocumentPasteTriggerKind::PasteAs,
            cx,
        );
        let version = self.snapshot.version();
        self.start_cancellable(crate::editor_state::CancellableOperation::DropOrPaste);
        self.drop_paste.task = Some(cx.spawn_in(window, async move |this, cx| {
            let tasks = match requests {
                PasteRequests::Started(tasks) => tasks,
                PasteRequests::AfterCopyData(pending) => {
                    let (providers, data, ranges) = pending.merge().await;
                    let Ok(tasks) = this.update(cx, |this, cx| {
                        this.paste_tasks(
                            &providers,
                            &ranges,
                            &data,
                            DocumentPasteTriggerKind::PasteAs,
                            cx,
                        )
                    }) else {
                        return;
                    };
                    tasks
                }
            };
            let mut edits = collect_edits(tasks).await;
            if let Some(only) = &only {
                edits.retain(|e| e.kind.as_ref().is_some_and(|k| edit_kind_contains(only, k)));
            }
            this.update_in(cx, |this, window, cx| {
                if this.snapshot.version() != version {
                    return;
                }
                if edits.is_empty() {
                    // Monaco `showPasteAsNoEditMessage`.
                    if let Some(only) = &only {
                        let at = this.selections()[0].start();
                        this.show_message(format!("No paste edits for '{only}' found"), at, cx);
                    }
                    return;
                }
                if only.is_some() && edits.len() == 1 {
                    this.apply_post_edit_once(&ranges, &edits[0], cx);
                    return;
                }
                let labels = edits.iter().map(|e| e.title.clone()).collect();
                this.show_quick_pick(
                    Some("Paste As...".into()),
                    labels,
                    move |this, ix, _, cx| this.apply_post_edit_once(&ranges, &edits[ix], cx),
                    window,
                    cx,
                );
            })
            .ok();
        }));
    }

    /// Monaco `createCombinedWorkspaceEdit`: the edit's text (a snippet)
    /// at each range, then its additional edits.
    fn apply_post_edit_once(
        &mut self,
        ranges: &[Range<PointUtf16>],
        edit: &DocumentDropOrPasteEdit,
        cx: &mut Context<Self>,
    ) {
        let template = match &edit.insert_text {
            EditInsertText::Text(text) if text.is_empty() => String::new(),
            EditInsertText::Text(text) => format!("{}$0", SnippetParser::escape(text)),
            EditInsertText::Snippet(snippet) => snippet.clone(),
        };
        if !template.is_empty() {
            let selections: Vec<Selection> = ranges
                .iter()
                .map(|range| Selection::new(range.start, range.end))
                .collect();
            self.set_selections(&selections, cx);
            self.insert_snippet(
                &template,
                SnippetInsertOptions {
                    undo_stop_before: true,
                    undo_stop_after: false,
                    ..Default::default()
                },
                cx,
            );
        }
        if !edit.additional_edits.is_empty() {
            self.execute_edits(edit.additional_edits.clone(), None, cx);
        }
        self.model()
            .update(cx, |model, _| model.push_stack_element());
    }

    /// Monaco `PostEditWidgetManager.applyEditAndShowIfNeeded`.
    fn apply_post_edit(
        &mut self,
        kind: PostEditKind,
        ranges: Vec<Range<PointUtf16>>,
        edits: Vec<DocumentDropOrPasteEdit>,
        active: usize,
        show: crate::options::ShowEditSelector,
        cx: &mut Context<Self>,
    ) {
        self.apply_post_edit_once(&ranges, &edits[active], cx);
        if show == crate::options::ShowEditSelector::Never || edits.len() < 2 {
            return;
        }
        let position = self.cursors.primary().position;
        let label: SharedString = match kind {
            PostEditKind::Drop => "Drop options ▾".into(),
            PostEditKind::Paste => "Paste options ▾".into(),
        };
        let editor = cx.entity().downgrade();
        let view = cx.new(|_| PostEditButton {
            editor,
            label,
            colors: self.theme.colors.clone(),
        });
        let widget = self.add_content_widget(
            ContentWidget {
                position,
                preference: vec![
                    ContentWidgetPreference::Below,
                    ContentWidgetPreference::Above,
                ],
                view: view.into(),
                allow_editor_overflow: true,
            },
            cx,
        );
        self.widgets
            .set_layer(widget, crate::widgets::layer::WIDGET);
        self.drop_paste.post_edit = Some(PostEdit {
            kind,
            edits,
            active,
            ranges,
            version: self.model().read(cx).buffer().version(),
            widget,
        });
    }

    /// Whether the drop or paste options button shows.
    pub fn is_post_edit_widget_visible(&self) -> bool {
        self.drop_paste.post_edit.is_some()
    }

    /// Monaco `editor.hidePasteWidget` / `editor.hideDropWidget`.
    pub(crate) fn clear_post_edit(&mut self, cx: &mut Context<Self>) -> bool {
        match self.drop_paste.post_edit.take() {
            Some(post_edit) => {
                self.remove_widget(post_edit.widget, cx);
                true
            }
            None => false,
        }
    }

    /// Hides the button once the text changes after the edit (Monaco hides
    /// it on content changes and cursor moves). Called before painting.
    pub(crate) fn refresh_post_edit(&mut self, cx: &mut Context<Self>) {
        if self
            .drop_paste
            .post_edit
            .as_ref()
            .is_some_and(|p| p.version != self.model().read(cx).buffer().version())
        {
            self.clear_post_edit(cx);
        }
    }

    /// Monaco `editor.changePasteType` / `editor.changeDropType`: picks
    /// another of the edits, replacing the applied one.
    pub fn show_post_edit_selector(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(post_edit) = &self.drop_paste.post_edit else {
            return;
        };
        let labels = post_edit
            .edits
            .iter()
            .enumerate()
            .map(|(ix, edit)| {
                if ix == post_edit.active {
                    format!("{} ✓", edit.title).into()
                } else {
                    edit.title.clone()
                }
            })
            .collect();
        let title: SharedString = match post_edit.kind {
            PostEditKind::Drop => "Show drop options...".into(),
            PostEditKind::Paste => "Show paste options...".into(),
        };
        self.show_quick_pick(
            Some(title),
            labels,
            |this, ix, _, cx| {
                let Some(post_edit) = this.drop_paste.post_edit.take() else {
                    return;
                };
                this.remove_widget(post_edit.widget, cx);
                if ix == post_edit.active {
                    return;
                }
                // Undo the applied edit, then apply the chosen one.
                this.undo(cx);
                this.apply_post_edit(
                    post_edit.kind,
                    post_edit.ranges,
                    post_edit.edits,
                    ix,
                    crate::options::ShowEditSelector::AfterEdit,
                    cx,
                );
            },
            window,
            cx,
        );
    }
}

/// A provider's `prepareDocumentPaste` result, shared by the pastes that
/// wait for it (failures become `None`).
type CopyData = futures::future::Shared<Task<Option<DataTransfer>>>;

/// Monaco `CopyPasteController._currentCopyOperation`: the data the last
/// copy's providers prepare, shared by every editor. A new copy replaces
/// (and so cancels) it.
#[derive(Default)]
struct CurrentCopyOperation {
    /// The id in the copied text's metadata.
    handle: String,
    /// Each provider's copy MIME types and data.
    operations: Vec<(Vec<SharedString>, CopyData)>,
}

impl Global for CurrentCopyOperation {}

/// Monaco `getPlainTextToCopy`'s `sourceRanges`: the selections sorted,
/// with an empty one standing for its whole line (once per line) when
/// `empty_selection_clipboard` copies lines, else left out.
pub(crate) fn copy_source_ranges(
    selections: &[Selection],
    rope: &text::Rope,
    empty_selection_clipboard: bool,
) -> Vec<Range<PointUtf16>> {
    let mut ranges: Vec<Range<PointUtf16>> = selections.iter().map(|s| s.range()).collect();
    ranges.sort_by_key(|range| (range.start, range.end));
    let has_empty = ranges.iter().any(|range| range.is_empty());
    let has_non_empty = ranges.iter().any(|range| !range.is_empty());
    if !has_non_empty && !empty_selection_clipboard {
        return Vec::new();
    }
    if !(has_empty && empty_selection_clipboard) {
        ranges.retain(|range| !range.is_empty());
        return ranges;
    }
    let mut result = Vec::new();
    let mut previous_row = None;
    for range in ranges {
        let row = range.start.row;
        if !range.is_empty() {
            result.push(range);
        } else if previous_row != Some(row) {
            result.push(PointUtf16::new(row, 0)..PointUtf16::new(row, rope.line_len_utf16(row)));
        }
        previous_row = Some(row);
    }
    result
}

/// The MIME types the copy being pasted prepared data for (Monaco's copy
/// metadata `providerCopyMimeTypes`).
fn copy_mime_types(item: &gpui::ClipboardItem) -> Vec<String> {
    CodeEditor::clipboard_metadata(item)
        .map(|metadata| metadata.provider_copy_mime_types)
        .unwrap_or_default()
}

/// The paste providers' requests: started right away, or once the data
/// prepared at copy time is in.
enum PasteRequests {
    Started(Vec<Task<anyhow::Result<Vec<DocumentDropOrPasteEdit>>>>),
    AfterCopyData(PendingPaste),
}

/// A paste waiting for data prepared at copy time.
struct PendingPaste {
    providers: Vec<Arc<dyn DocumentPasteEditProvider>>,
    data: DataTransfer,
    ranges: Vec<Range<PointUtf16>>,
    operations: Vec<CopyData>,
}

impl PendingPaste {
    /// Monaco `mergeInDataFromCopy`, then the providers that can handle the
    /// merged data (`isSupportedPasteProvider`).
    async fn merge(
        self,
    ) -> (
        Vec<Arc<dyn DocumentPasteEditProvider>>,
        DataTransfer,
        Vec<Range<PointUtf16>>,
    ) {
        let mut data = self.data;
        let results = futures::future::join_all(self.operations).await;
        // Higher priority providers' values replace lower ones'.
        for result in results.into_iter().rev().flatten() {
            for (mime_type, item) in result.iter() {
                data.insert(mime_type.clone(), item.clone());
            }
        }
        let providers = self
            .providers
            .into_iter()
            .filter(|provider| {
                provider
                    .paste_mime_types()
                    .iter()
                    .any(|mime| data.matches(mime))
            })
            .collect();
        (providers, data, self.ranges)
    }
}

impl CodeEditor {
    /// Monaco `CopyPasteController.handleCopy`: the paste providers that
    /// prepare pastes get the copied ranges, and their data is kept for
    /// when the copy is pasted. Returns the copy's id and MIME types for
    /// the clipboard metadata (none without such providers, or with
    /// `paste_as` off).
    pub(crate) fn prepare_document_paste(
        &mut self,
        cx: &mut Context<Self>,
    ) -> (Option<String>, Vec<String>) {
        if !self.options.paste_as.enabled {
            return (None, Vec::new());
        }
        let providers = crate::providers::paste_edit_providers(cx, self.provider_target(cx));
        let context = self.provider_context_now(cx);
        let ranges = copy_source_ranges(
            &self.cursors.selections(),
            self.snapshot.rope(),
            self.options.empty_selection_clipboard,
        );
        let mut operations = Vec::new();
        for provider in providers {
            let Some(task) = provider.prepare_document_paste(&context, &ranges, cx) else {
                continue;
            };
            let data = cx
                .spawn(async move |_, _| match task.await {
                    Ok(data) => Some(data),
                    Err(err) => {
                        log::error!("prepareDocumentPaste failed: {err:#}");
                        None
                    }
                })
                .shared();
            operations.push((provider.copy_mime_types(), data));
        }
        if operations.is_empty() {
            return (None, Vec::new());
        }
        let handle = uuid::Uuid::new_v4().to_string();
        let mime_types = operations
            .iter()
            .flat_map(|(mime_types, _)| mime_types.iter().map(|m| m.to_string()))
            .collect();
        cx.set_global(CurrentCopyOperation {
            handle: handle.clone(),
            operations,
        });
        (Some(handle), mime_types)
    }

    /// Starts the paste providers' requests, after merging in the data
    /// prepared when `item` was copied, if it was copied last (Monaco
    /// `mergeInDataFromCopy`).
    fn paste_requests(
        &self,
        providers: Vec<Arc<dyn DocumentPasteEditProvider>>,
        ranges: &[Range<PointUtf16>],
        data: DataTransfer,
        item: &gpui::ClipboardItem,
        trigger: DocumentPasteTriggerKind,
        cx: &mut App,
    ) -> PasteRequests {
        let id = CodeEditor::clipboard_metadata(item).and_then(|metadata| metadata.id);
        // Only the data some provider may want.
        let operations: Vec<CopyData> = cx
            .try_global::<CurrentCopyOperation>()
            .filter(|current| id.as_deref() == Some(current.handle.as_str()))
            .map(|current| {
                current
                    .operations
                    .iter()
                    .filter(|(mime_types, _)| {
                        providers.iter().any(|provider| {
                            provider.paste_mime_types().iter().any(|mime| {
                                matches_mime_type(mime, mime_types.iter().map(|m| m.as_ref()))
                            })
                        })
                    })
                    .map(|(_, data)| data.clone())
                    .collect()
            })
            .unwrap_or_default();
        if operations.is_empty() {
            let providers: Vec<_> = providers
                .into_iter()
                .filter(|provider| {
                    provider
                        .paste_mime_types()
                        .iter()
                        .any(|mime| data.matches(mime))
                })
                .collect();
            return PasteRequests::Started(
                self.paste_tasks(&providers, ranges, &data, trigger, cx),
            );
        }
        PasteRequests::AfterCopyData(PendingPaste {
            providers,
            data,
            ranges: ranges.to_vec(),
            operations,
        })
    }
}

async fn collect_edits(
    tasks: Vec<Task<anyhow::Result<Vec<DocumentDropOrPasteEdit>>>>,
) -> Vec<DocumentDropOrPasteEdit> {
    let mut edits = Vec::new();
    for task in tasks {
        if let Ok(result) = task.await {
            edits.extend(result);
        }
    }
    sort_edits_by_yield_to(edits)
}

/// The clipboard as paste data: its text as `text/plain`, and copied
/// files.
pub(crate) fn clipboard_data(item: &gpui::ClipboardItem) -> DataTransfer {
    let mut data = DataTransfer::default();
    if let Some(text) = item.text() {
        data.insert(TEXT_PLAIN, DataTransferItem::Text(text));
    }
    // Copied files, as Monaco's clipboard `files` and uri-list.
    let paths: Vec<PathBuf> = item
        .entries()
        .iter()
        .filter_map(|entry| match entry {
            gpui::ClipboardEntry::ExternalPaths(paths) => Some(paths.paths().to_vec()),
            _ => None,
        })
        .flatten()
        .collect();
    if !paths.is_empty() {
        data.insert(URI_LIST, DataTransferItem::Text(uri_list(&paths)));
        data.insert(FILES, DataTransferItem::Files(paths));
    }
    data
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Monaco's `getPlainTextToCopy` tests (`viewModelImpl.test.ts`), on
    /// the source ranges rather than the text.
    #[test]
    fn copy_source_ranges_like_monaco() {
        let rope = text::Rope::from("\nline2\nline3\nline4\n");
        let p = PointUtf16::new;
        let at = |row, column| Selection::collapsed(p(row, column));
        let select = |row, start, end| Selection {
            anchor: p(row, start),
            head: p(row, end),
        };
        // 0/1: an empty selection copies nothing, or its line.
        assert_eq!(copy_source_ranges(&[at(1, 1)], &rope, false), vec![]);
        assert_eq!(
            copy_source_ranges(&[at(1, 1)], &rope, true),
            vec![p(1, 0)..p(1, 5)]
        );
        // 1/1.
        for empty_selection_clipboard in [false, true] {
            assert_eq!(
                copy_source_ranges(&[select(1, 1, 5)], &rope, empty_selection_clipboard),
                vec![p(1, 1)..p(1, 5)]
            );
        }
        // 0/2.
        assert_eq!(
            copy_source_ranges(&[at(1, 1), at(2, 1)], &rope, false),
            vec![]
        );
        assert_eq!(
            copy_source_ranges(&[at(1, 1), at(2, 1)], &rope, true),
            vec![p(1, 0)..p(1, 5), p(2, 0)..p(2, 5)]
        );
        // 1/2: the empty one only counts with `emptySelectionClipboard`.
        assert_eq!(
            copy_source_ranges(&[select(1, 1, 5), at(2, 1)], &rope, false),
            vec![p(1, 1)..p(1, 5)]
        );
        assert_eq!(
            copy_source_ranges(&[at(2, 1), select(1, 1, 5)], &rope, true),
            vec![p(1, 1)..p(1, 5), p(2, 0)..p(2, 5)]
        );
        // Two cursors on one line copy it once.
        assert_eq!(
            copy_source_ranges(&[at(1, 1), at(1, 3)], &rope, true),
            vec![p(1, 0)..p(1, 5)]
        );
    }

    fn edit(
        title: &str,
        kind: &str,
        mime: &str,
        yield_to: Vec<EditYieldTo>,
    ) -> DocumentDropOrPasteEdit {
        DocumentDropOrPasteEdit {
            title: title.to_string().into(),
            kind: Some(kind.to_string().into()),
            handled_mime_type: Some(mime.to_string().into()),
            yield_to,
            ..Default::default()
        }
    }

    fn titles(edits: &[DocumentDropOrPasteEdit]) -> Vec<&str> {
        edits.iter().map(|e| e.title.as_ref()).collect()
    }

    #[test]
    fn edits_sort_after_what_they_yield_to() {
        let edits = vec![
            edit(
                "html",
                "html",
                "text/html",
                vec![EditYieldTo::MimeType(TEXT_PLAIN.into())],
            ),
            edit("text", "text.plain", TEXT_PLAIN, vec![]),
            edit(
                "path",
                "uri.path",
                URI_LIST,
                vec![EditYieldTo::Kind("text".into())],
            ),
        ];
        assert_eq!(
            titles(&sort_edits_by_yield_to(edits)),
            vec!["text", "html", "path"]
        );
        // No yields: unchanged.
        let edits = vec![edit("a", "a", "x/a", vec![]), edit("b", "b", "x/b", vec![])];
        assert_eq!(titles(&sort_edits_by_yield_to(edits)), vec!["a", "b"]);
    }

    #[test]
    fn uri_lists_round_trip_paths() {
        let paths = vec![PathBuf::from("/tmp/a b.txt"), PathBuf::from("/x/ü.rs")];
        let mut data = DataTransfer::default();
        data.insert(URI_LIST, DataTransferItem::Text(uri_list(&paths)));
        let edit = PathProvider::edit(&data).unwrap();
        assert_eq!(edit.title.as_ref(), "Insert Paths");
        assert_eq!(
            edit.insert_text,
            EditInsertText::Text("/tmp/a b.txt /x/ü.rs".into())
        );
        // Plain text yields to the uri-list.
        data.insert(TEXT_PLAIN, DataTransferItem::Text("x".into()));
        assert!(DefaultTextProvider::edit(&data).is_none());
    }

    #[test]
    fn mime_types_and_kinds_match_like_monaco() {
        assert!(matches_mime_type("text/*", ["text/plain"]));
        assert!(matches_mime_type("*/*", ["image/png"]));
        assert!(!matches_mime_type("text/html", ["text/plain"]));
        assert!(matches_mime_type("TEXT/PLAIN", ["text/plain"]));
        assert!(edit_kind_contains("text", "text.plain"));
        assert!(!edit_kind_contains("text", "textual"));
        assert!(edit_kind_contains("", "anything"));
    }
}
