//! The rest of Monaco's `ICodeEditor` and `ITextModel` API: contributions
//! (`getContribution`), `getFontSizeAtPosition`, `writeScreenReaderContent`
//! and `setAriaOptions`, and a model's `onWillDispose`.

use std::any::{Any, TypeId};

use gpui::{App, Context, Entity, Pixels, Subscription};
use text::PointUtf16;

use crate::editor::CodeEditor;
use crate::model::TextModel;

/// Monaco `IEditorAriaOptions`: what the editor tells assistive technology
/// besides its text, for hosts that show their own UI (such as a list of
/// completions) for it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct AriaOptions {
    /// The accessibility node of the host's element the editor's input is
    /// tied to, such as the selected row of a completion list (Monaco
    /// `activeDescendant`, an element id).
    pub active_descendant: Option<gpui::accesskit::NodeId>,
    /// The editor input's role (Monaco `role`); `None` is a (multiline)
    /// text input.
    pub role: Option<gpui::Role>,
}

impl CodeEditor {
    /// Attaches `contribution` to the editor, replacing (and returning) one
    /// of the same type: a host's own feature, found again with
    /// [`CodeEditor::get_contribution`]. Monaco's editor contributions
    /// (`registerEditorContribution`) are built-in features here, used
    /// through the editor's own methods; this keeps a host's per-editor
    /// state with the editor instead.
    pub fn add_contribution<T: Any>(&mut self, contribution: T) -> Option<T> {
        self.contributions
            .insert(TypeId::of::<T>(), Box::new(contribution))
            .and_then(|old| old.downcast().ok())
            .map(|old| *old)
    }

    /// The contribution of type `T` added with
    /// [`CodeEditor::add_contribution`] (Monaco `getContribution`, by type
    /// rather than by id).
    pub fn get_contribution<T: Any>(&self) -> Option<&T> {
        self.contributions.get(&TypeId::of::<T>())?.downcast_ref()
    }

    /// [`CodeEditor::get_contribution`], mutably.
    pub fn get_contribution_mut<T: Any>(&mut self) -> Option<&mut T> {
        self.contributions
            .get_mut(&TypeId::of::<T>())?
            .downcast_mut()
    }

    /// Removes and returns the contribution of type `T`.
    pub fn remove_contribution<T: Any>(&mut self) -> Option<T> {
        self.contributions
            .remove(&TypeId::of::<T>())
            .and_then(|old| old.downcast().ok())
            .map(|old| *old)
    }

    /// The font size of the text at `position` (Monaco
    /// `getFontSizeAtPosition`): the editor's, with the font zoom applied;
    /// decorations don't change the text's size here. `None` without a
    /// model, as Monaco's `null`.
    pub fn get_font_size_at_position(&self, _position: PointUtf16) -> Option<Pixels> {
        self.has_model().then_some(self.options.font_size)
    }

    /// Has screen readers read the editor's content again (Monaco
    /// `writeScreenReaderContent`); `reason` is for logging, as in Monaco.
    /// The content (the lines around the selection, see
    /// [`EditorOptions::accessibility_page_size`](crate::EditorOptions::accessibility_page_size))
    /// is rebuilt on the next frame.
    pub fn write_screen_reader_content(&mut self, _reason: &str, cx: &mut Context<Self>) {
        cx.notify();
    }

    /// Sets the editor's accessibility role and active descendant (Monaco
    /// `setAriaOptions`).
    pub fn set_aria_options(&mut self, options: AriaOptions, cx: &mut Context<Self>) {
        if self.aria_options != options {
            self.aria_options = options;
            cx.notify();
        }
    }

    /// The options from [`CodeEditor::set_aria_options`].
    pub fn aria_options(&self) -> &AriaOptions {
        &self.aria_options
    }
}

impl TextModel {
    /// Calls `listener` with `model` as it's released, its last handle
    /// dropped (Monaco `ITextModel.onWillDispose`); dropping the returned
    /// subscription cancels it. [`crate::on_will_dispose_model`] hears
    /// every model's.
    pub fn on_will_dispose(
        model: &Entity<TextModel>,
        cx: &mut App,
        listener: impl FnOnce(&mut TextModel, &mut App) + 'static,
    ) -> Subscription {
        cx.observe_release(model, listener)
    }
}
