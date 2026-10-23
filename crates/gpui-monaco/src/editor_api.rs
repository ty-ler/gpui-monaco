//! The rest of Monaco's `ICodeEditor` and `ITextModel` API: `getId` and
//! `getEditorType`, contributions (`getContribution`),
//! `getFontSizeAtPosition`, `writeScreenReaderContent` and
//! `setAriaOptions`, and a model's `onWillDispose`.

use std::any::{Any, TypeId};

use gpui::{App, Context, Entity, Pixels, Subscription};
use text::PointUtf16;

use crate::editor::CodeEditor;
use crate::model::TextModel;

/// What kind of editor an editor is (Monaco `editor.EditorType`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum EditorType {
    /// A [`CodeEditor`] (Monaco `'vs.editor.ICodeEditor'`).
    CodeEditor,
    /// A [`DiffEditor`](crate::DiffEditor) (Monaco `'vs.editor.IDiffEditor'`).
    DiffEditor,
}

impl EditorType {
    /// Monaco's string for it, e.g. `"vs.editor.ICodeEditor"`.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::CodeEditor => "vs.editor.ICodeEditor",
            Self::DiffEditor => "vs.editor.IDiffEditor",
        }
    }
}

impl std::fmt::Display for EditorType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl CodeEditor {
    /// The editor's unique id, `vs.editor.ICodeEditor:1`,
    /// `vs.editor.ICodeEditor:2`, … in creation order (Monaco `getId`, the
    /// editor type and a counter). A diff editor's two sides have their
    /// own.
    pub fn id(&self) -> String {
        format!("{}:{}", self.editor_type(), self.commands.id)
    }

    /// [`EditorType::CodeEditor`] (Monaco `getEditorType`).
    pub fn editor_type(&self) -> EditorType {
        EditorType::CodeEditor
    }
}

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
    /// `getFontSizeAtPosition`): the first decoration there with a
    /// [`font_size`](crate::DecorationOptions::font_size), as the editor
    /// shows its decorations (none when
    /// [`allows_variable_fonts`](Self::allows_variable_fonts) is off), else
    /// the editor's. `None` without a model, as Monaco's `null`.
    pub fn get_font_size_at_position(
        &self,
        position: PointUtf16,
        cx: &Context<Self>,
    ) -> Option<Pixels> {
        if !self.has_model() {
            return None;
        }
        // Font decorations are left out when variable fonts aren't allowed.
        let decorated = self
            .decorations_in_range(position..position, cx)
            .into_iter()
            .find_map(|decoration| decoration.options.font_size);
        Some(decorated.unwrap_or(self.options.font_size))
    }

    /// Has screen readers read the editor's content again (Monaco
    /// `writeScreenReaderContent`); `reason` is for logging, as in Monaco.
    /// The content (the pages of lines holding the selection, see
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
