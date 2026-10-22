//! Monaco's global editor and model registries (`standaloneEditor.ts` over
//! `ICodeEditorService` and `IModelService`): every live [`CodeEditor`],
//! [`DiffEditor`] and [`TextModel`], and events when they're created,
//! released, change language, change markers or change marker limits.
//!
//! Editors and diff editors register themselves when they're created.
//! Models are noticed when they're created once [`crate::init`] (or any
//! function here) has run; a model created before that is picked up when
//! an editor shows it or it gets markers.
//!
//! Event listeners return a [`Subscription`]: dropping it unsubscribes
//! (Monaco's `IDisposable`), and [`Subscription::detach`] keeps it for
//! good. Creation events are delivered after the entity is built, at the
//! end of the current update, so listeners can read and update it.

use std::cell::RefCell;
use std::rc::Rc;

use gpui::{App, Context, Entity, EntityId, Global, SharedString, Subscription, WeakEntity};

use crate::diff_editor::DiffEditor;
use crate::editor::CodeEditor;
use crate::markers::MarkerLimit;
use crate::model::TextModel;

/// Listeners of one global event (Monaco `Emitter`).
pub(crate) struct Emitter<F: ?Sized> {
    inner: Rc<RefCell<EmitterState<F>>>,
}

struct EmitterState<F: ?Sized> {
    next_id: u64,
    listeners: Vec<(u64, Rc<F>)>,
}

impl<F: ?Sized> Default for Emitter<F> {
    fn default() -> Self {
        Self {
            inner: Rc::new(RefCell::new(EmitterState {
                next_id: 0,
                listeners: Vec::new(),
            })),
        }
    }
}

impl<F: ?Sized + 'static> Emitter<F> {
    /// Adds `listener` until the returned subscription is dropped.
    pub(crate) fn subscribe(&self, listener: Rc<F>) -> Subscription {
        let id = {
            let mut state = self.inner.borrow_mut();
            state.next_id += 1;
            let id = state.next_id;
            state.listeners.push((id, listener));
            id
        };
        let inner = Rc::downgrade(&self.inner);
        Subscription::new(move || {
            if let Some(inner) = inner.upgrade() {
                inner.borrow_mut().listeners.retain(|(i, _)| *i != id);
            }
        })
    }

    /// The listeners now, to call without borrowing the app.
    pub(crate) fn listeners(&self) -> Vec<Rc<F>> {
        self.inner
            .borrow()
            .listeners
            .iter()
            .map(|(_, listener)| listener.clone())
            .collect()
    }
}

type EditorListener = dyn Fn(&Entity<CodeEditor>, &mut App);
type DiffEditorListener = dyn Fn(&Entity<DiffEditor>, &mut App);
type ModelListener = dyn Fn(&Entity<TextModel>, &mut App);
type ModelDisposeListener = dyn Fn(EntityId, &TextModel, &mut App);
type LanguageListener = dyn Fn(&Entity<TextModel>, Option<&SharedString>, &mut App);
type MarkersListener = dyn Fn(&[SharedString], &mut App);

#[derive(Default)]
struct Registry {
    /// Whether models are observed as they're created.
    observing_models: bool,
    editors: Vec<WeakEntity<CodeEditor>>,
    /// The editor's own editors (Monaco `isSimpleWidget`: the find, replace,
    /// rename and quick input fields): not listed or announced, but they
    /// follow global changes such as the theme.
    internal_editors: Vec<WeakEntity<CodeEditor>>,
    diff_editors: Vec<WeakEntity<DiffEditor>>,
    models: Vec<WeakEntity<TextModel>>,
    editor_created: Emitter<EditorListener>,
    diff_editor_created: Emitter<DiffEditorListener>,
    model_created: Emitter<ModelListener>,
    model_will_dispose: Emitter<ModelDisposeListener>,
    model_language_changed: Emitter<LanguageListener>,
    markers_changed: Emitter<MarkersListener>,
    /// Uris whose markers changed since the last `markers_changed` event.
    pending_marker_uris: Vec<SharedString>,
    marker_limit_changed: Emitter<ModelListener>,
    /// Models whose marker limit changed since the last
    /// `marker_limit_changed` event, with their limit before.
    pending_marker_limits: Vec<(WeakEntity<TextModel>, MarkerLimit)>,
}

impl Global for Registry {}

/// Starts noticing models as they're created ([`crate::init`] calls it).
pub(crate) fn ensure(cx: &mut App) {
    let registry = cx.default_global::<Registry>();
    if registry.observing_models {
        return;
    }
    registry.observing_models = true;
    cx.observe_new::<TextModel>(|model: &mut TextModel, _, cx| {
        // The editor's own models (widget inputs, placeholders) aren't
        // listed (Monaco's are DOM inputs, not models).
        if model.is_internal() {
            return;
        }
        track_model(&cx.entity(), cx);
        // Monaco's `TextModel` constructor resolves its language and
        // requests its features (`requestRichLanguageFeatures`).
        model.resolve_language(cx);
        // Monaco `MarkerDecorationsService._onModelAdded`: markers set on
        // its uri before decorate it.
        model.update_marker_decorations(cx);
    })
    .detach();
}

/// Adds `model` to the registry if it isn't there yet, announcing it
/// (Monaco `IModelService.onModelAdded`) and its release (`onModelRemoved`).
/// Whether it wasn't there.
pub(crate) fn track_model(model: &Entity<TextModel>, cx: &mut App) -> bool {
    ensure(cx);
    let id = model.entity_id();
    let registry = cx.global_mut::<Registry>();
    registry.models.retain(|m| m.upgrade().is_some());
    if registry.models.iter().any(|m| m.entity_id() == id) {
        return false;
    }
    registry.models.push(model.downgrade());
    cx.observe_release(model, move |model, cx| will_dispose_model(id, model, cx))
        .detach();
    let model = model.downgrade();
    cx.defer(move |cx| {
        let Some(model) = model.upgrade() else {
            return;
        };
        let listeners = cx.global::<Registry>().model_created.listeners();
        for listener in listeners {
            listener(&model, cx);
        }
    });
    true
}

fn will_dispose_model(id: EntityId, model: &mut TextModel, cx: &mut App) {
    let registry = cx.default_global::<Registry>();
    registry.models.retain(|m| m.entity_id() != id);
    let listeners = registry.model_will_dispose.listeners();
    for listener in listeners {
        listener(id, model, cx);
    }
    // An in-memory model's markers go with it; others stay for the next
    // model with its uri.
    crate::markers::model_disposed(model.resource(), cx);
}

/// Adds the editor being built to the registry and announces it once it
/// exists (Monaco `ICodeEditorService.addCodeEditor`). An `internal` one
/// (Monaco `isSimpleWidget`) is only kept for [`update_editors`].
pub(crate) fn track_editor(internal: bool, cx: &mut Context<CodeEditor>) {
    ensure(cx);
    let editor = cx.weak_entity();
    let registry = cx.global_mut::<Registry>();
    if internal {
        registry.internal_editors.retain(|e| e.upgrade().is_some());
        registry.internal_editors.push(editor);
        return;
    }
    registry.editors.retain(|e| e.upgrade().is_some());
    registry.editors.push(editor.clone());
    cx.defer(move |cx| {
        let Some(editor) = editor.upgrade() else {
            return;
        };
        let listeners = cx.global::<Registry>().editor_created.listeners();
        for listener in listeners {
            listener(&editor, cx);
        }
    });
}

/// Adds the diff editor being built to the registry and announces it once
/// it exists (Monaco `ICodeEditorService.addDiffEditor`).
pub(crate) fn track_diff_editor(cx: &mut Context<DiffEditor>) {
    ensure(cx);
    let editor = cx.weak_entity();
    let registry = cx.global_mut::<Registry>();
    registry.diff_editors.retain(|e| e.upgrade().is_some());
    registry.diff_editors.push(editor.clone());
    cx.defer(move |cx| {
        let Some(editor) = editor.upgrade() else {
            return;
        };
        let listeners = cx.global::<Registry>().diff_editor_created.listeners();
        for listener in listeners {
            listener(&editor, cx);
        }
    });
}

/// Announces that `model`'s language changed from `old` (Monaco
/// `IModelService.onModelLanguageChanged`), after the current update.
pub(crate) fn model_language_changed(
    model: WeakEntity<TextModel>,
    old: Option<SharedString>,
    cx: &mut App,
) {
    cx.defer(move |cx| {
        let Some(model) = model.upgrade() else {
            return;
        };
        let Some(registry) = cx.try_global::<Registry>() else {
            return;
        };
        let listeners = registry.model_language_changed.listeners();
        for listener in listeners {
            listener(&model, old.as_ref(), cx);
        }
    });
}

/// Queues `uri` for the next `on_did_change_markers` event. Changes in one
/// update are merged into one event, with each uri once (Monaco debounces
/// `IMarkerService.onMarkerChanged` and merges the uris).
pub(crate) fn markers_changed(uri: SharedString, cx: &mut App) {
    let registry = cx.default_global::<Registry>();
    let schedule = registry.pending_marker_uris.is_empty();
    if !registry.pending_marker_uris.contains(&uri) {
        registry.pending_marker_uris.push(uri);
    }
    if schedule {
        cx.defer(|cx| {
            let registry = cx.global_mut::<Registry>();
            let uris = std::mem::take(&mut registry.pending_marker_uris);
            let listeners = registry.markers_changed.listeners();
            for listener in listeners {
                listener(&uris, cx);
            }
        });
    }
}

/// Queues `model`, whose [`MarkerLimit`] was `old`, for the next
/// `on_did_change_marker_limit` event (Monaco
/// `IMarkerDecorationsService.onDidChangeMarkerLimit`). Changes in one
/// update are merged, as Monaco redecorates a model once for the marker
/// changes of a tick: each model is announced once, and not if its limit
/// is back to `old`.
pub(crate) fn marker_limit_changed(model: WeakEntity<TextModel>, old: MarkerLimit, cx: &mut App) {
    let registry = cx.default_global::<Registry>();
    let schedule = registry.pending_marker_limits.is_empty();
    let id = model.entity_id();
    if !registry
        .pending_marker_limits
        .iter()
        .any(|(m, _)| m.entity_id() == id)
    {
        registry.pending_marker_limits.push((model, old));
    }
    if schedule {
        cx.defer(|cx| {
            let registry = cx.global_mut::<Registry>();
            let pending = std::mem::take(&mut registry.pending_marker_limits);
            let listeners = registry.marker_limit_changed.listeners();
            for (model, old) in pending {
                let Some(model) = model.upgrade() else {
                    continue;
                };
                if model.read(cx).marker_limit() == old {
                    continue;
                }
                for listener in &listeners {
                    listener(&model, cx);
                }
            }
        });
    }
}

/// Every live code editor, in creation order (Monaco `editor.getEditors`),
/// including the two inside each diff editor, but not the editors' own
/// inputs (find, replace, rename, quick input).
pub fn editors(cx: &App) -> Vec<Entity<CodeEditor>> {
    cx.try_global::<Registry>()
        .map(|r| r.editors.iter().filter_map(WeakEntity::upgrade).collect())
        .unwrap_or_default()
}

/// Monaco `ICodeEditorService.getFocusedCodeEditor`: the editor whose text
/// has focus in `window`, else the last one with focus in one of its
/// widgets (a hover, the find widget).
pub(crate) fn focused_editor(window: &gpui::Window, cx: &App) -> Option<Entity<CodeEditor>> {
    let mut widget_focus = None;
    for editor in editors(cx) {
        let focus = &editor.read(cx).focus_handle;
        if focus.is_focused(window) {
            return Some(editor);
        }
        if focus.contains_focused(window, cx) {
            widget_focus = Some(editor);
        }
    }
    widget_focus
}

/// Every live diff editor, in creation order (Monaco
/// `editor.getDiffEditors`).
pub fn diff_editors(cx: &App) -> Vec<Entity<DiffEditor>> {
    cx.try_global::<Registry>()
        .map(|r| {
            r.diff_editors
                .iter()
                .filter_map(WeakEntity::upgrade)
                .collect()
        })
        .unwrap_or_default()
}

/// Every live model, in creation order (Monaco `editor.getModels`): those
/// created after [`crate::init`], and earlier ones once an editor showed
/// them or they got markers.
pub fn models(cx: &App) -> Vec<Entity<TextModel>> {
    cx.try_global::<Registry>()
        .map(|r| r.models.iter().filter_map(WeakEntity::upgrade).collect())
        .unwrap_or_default()
}

/// Calls `listener` with each code editor created from now on (Monaco
/// `editor.onDidCreateEditor`); a diff editor creates two.
pub fn on_did_create_editor(
    cx: &mut App,
    listener: impl Fn(&Entity<CodeEditor>, &mut App) + 'static,
) -> Subscription {
    ensure(cx);
    cx.global::<Registry>()
        .editor_created
        .subscribe(Rc::new(listener))
}

/// Calls `listener` with each diff editor created from now on (Monaco
/// `editor.onDidCreateDiffEditor`).
pub fn on_did_create_diff_editor(
    cx: &mut App,
    listener: impl Fn(&Entity<DiffEditor>, &mut App) + 'static,
) -> Subscription {
    ensure(cx);
    cx.global::<Registry>()
        .diff_editor_created
        .subscribe(Rc::new(listener))
}

/// Calls `listener` with each model created from now on (Monaco
/// `editor.onDidCreateModel`).
pub fn on_did_create_model(
    cx: &mut App,
    listener: impl Fn(&Entity<TextModel>, &mut App) + 'static,
) -> Subscription {
    ensure(cx);
    cx.global::<Registry>()
        .model_created
        .subscribe(Rc::new(listener))
}

/// Calls `listener` with each model as it's released, its last handle
/// dropped (Monaco `editor.onWillDisposeModel`). The model can't be
/// upgraded to an entity any more, so `listener` gets its id and state.
pub fn on_will_dispose_model(
    cx: &mut App,
    listener: impl Fn(EntityId, &TextModel, &mut App) + 'static,
) -> Subscription {
    ensure(cx);
    cx.global::<Registry>()
        .model_will_dispose
        .subscribe(Rc::new(listener))
}

/// Calls `listener` with each model whose language changes and its old
/// language id (Monaco `editor.onDidChangeModelLanguage`), after the update
/// that changed it.
pub fn on_did_change_model_language(
    cx: &mut App,
    listener: impl Fn(&Entity<TextModel>, Option<&SharedString>, &mut App) + 'static,
) -> Subscription {
    ensure(cx);
    cx.global::<Registry>()
        .model_language_changed
        .subscribe(Rc::new(listener))
}

/// Calls `listener` with the uris of the models whose markers changed
/// (Monaco `editor.onDidChangeMarkers`). Changes made in one update arrive
/// together, after it, each uri once. Uris are the models'
/// [`TextModel::resource`]s: normalized (see [`crate::normalize_uri`]), and
/// `inmemory://model/N` for a model without a uri.
pub fn on_did_change_markers(
    cx: &mut App,
    listener: impl Fn(&[SharedString], &mut App) + 'static,
) -> Subscription {
    ensure(cx);
    cx.global::<Registry>()
        .markers_changed
        .subscribe(Rc::new(listener))
}

/// Calls `listener` with each model whose
/// [`marker_limit`](TextModel::marker_limit) changed (Monaco
/// `IMarkerDecorationsService.onDidChangeMarkerLimit`): its markers became
/// truncated at their source or no longer are, or it got more markers than
/// [`MAX_DECORATED_MARKERS`](crate::MAX_DECORATED_MARKERS) or no longer
/// has. Changes made in one update arrive after it, each model once, and
/// only if its limit differs from before the update.
pub fn on_did_change_marker_limit(
    cx: &mut App,
    listener: impl Fn(&Entity<TextModel>, &mut App) + 'static,
) -> Subscription {
    ensure(cx);
    cx.global::<Registry>()
        .marker_limit_changed
        .subscribe(Rc::new(listener))
}

/// Monaco's `IDisposable` for a global registration ([`add_command`],
/// [`add_editor_action`], [`add_keybinding_rules`]): [`Disposable::dispose`]
/// removes it. Dropping it leaves the registration in place.
///
/// [`add_command`]: crate::add_command
/// [`add_editor_action`]: crate::add_editor_action
/// [`add_keybinding_rules`]: crate::add_keybinding_rules
#[must_use = "dropping a Disposable keeps the registration; call `dispose` to remove it"]
pub struct Disposable {
    dispose: Option<DisposeFn>,
}

type DisposeFn = Box<dyn FnOnce(&mut App)>;

impl Disposable {
    pub(crate) fn new(dispose: impl FnOnce(&mut App) + 'static) -> Self {
        Self {
            dispose: Some(Box::new(dispose)),
        }
    }

    /// A disposable that does nothing (Monaco `Disposable.None`).
    pub fn none() -> Self {
        Self { dispose: None }
    }

    /// Combines disposables into one (Monaco `DisposableStore`).
    pub fn join(disposables: Vec<Disposable>) -> Self {
        Self::new(move |cx| {
            for disposable in disposables {
                disposable.dispose(cx);
            }
        })
    }

    /// Removes the registration.
    pub fn dispose(mut self, cx: &mut App) {
        if let Some(dispose) = self.dispose.take() {
            dispose(cx);
        }
    }
}

impl std::fmt::Debug for Disposable {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Disposable").finish_non_exhaustive()
    }
}

/// Runs `f` on every live editor (e.g. to apply a global theme), the
/// editor's own internal ones too.
pub(crate) fn update_editors(
    cx: &mut App,
    mut f: impl FnMut(&mut CodeEditor, &mut Context<CodeEditor>),
) {
    let internal: Vec<_> = cx
        .try_global::<Registry>()
        .map(|r| {
            r.internal_editors
                .iter()
                .filter_map(WeakEntity::upgrade)
                .collect()
        })
        .unwrap_or_default();
    for editor in editors(cx).into_iter().chain(internal) {
        editor.update(cx, |editor, cx| f(editor, cx));
    }
}
