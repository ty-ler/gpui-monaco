//! Inline values while debugging, as VS Code's debugger shows them
//! (`DebugEditorContribution.updateInlineValueDecorations`): while the host's
//! debugger is paused, the editor asks the [`InlineValuesProvider`]s for the
//! values on and near the screen, the host looks up their variables and
//! evaluates their expressions, and each line shows its values after its
//! end, as `name = value`, separated by commas.
//!
//! [`InlineValuesProvider`]: crate::providers::InlineValuesProvider

use std::ops::Range;
use std::rc::Rc;
use std::time::Duration;

use gpui::{App, Context, EntityId, SharedString, Task};
use text::PointUtf16;

use crate::CodeEditor;
use crate::api::DecorationsCollection;
use crate::decorations::{DecorationOptions, InjectedText, InjectedTextCursorStops};
use crate::providers::{
    InlineValue, InlineValueContext, InlineValuesEpoch, inline_values_providers,
};
use crate::theme::TokenStyle;

/// VS Code's `MAX_INLINE_DECORATOR_LENGTH`: longer values end in "...".
const MAX_INLINE_DECORATOR_LENGTH: usize = 150;

/// VS Code's `updateInlineValuesScheduler`: the pause after an edit before
/// asking again.
const UPDATE_DELAY: Duration = Duration::from_millis(200);

/// What the host's debugger looks up for an [`InlineValue`]: a variable in
/// the paused frame's scopes, or an expression to evaluate in it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum InlineValueLookup {
    /// A variable's value (VS Code's `findVariable`: the most specific
    /// scopes of the frame, first match).
    Variable {
        /// Its name.
        name: SharedString,
        /// Whether the name matches only in the same case.
        case_sensitive: bool,
    },
    /// An expression's value, evaluated in the frame (VS Code evaluates it
    /// in the `watch` context).
    Expression(SharedString),
}

type Resolver = Rc<dyn Fn(InlineValueLookup, &mut App) -> Task<Option<SharedString>>>;

struct Session {
    /// The model the debugger is paused in; values show only while the
    /// editor shows it.
    model: EntityId,
    context: InlineValueContext,
    resolve: Resolver,
}

/// What the shown values were asked for.
#[derive(Clone, PartialEq)]
struct Requested {
    version: text::Version,
    epoch: u64,
    providers: usize,
    /// The rows asked about, from the first to the last.
    rows: Range<u32>,
}

#[derive(Default)]
pub(crate) struct InlineValuesState {
    session: Option<Session>,
    requested: Option<Requested>,
    decorations: Option<DecorationsCollection>,
    /// Each line's text, in row order.
    lines: Vec<(u32, SharedString)>,
    task: Option<Task<()>>,
}

/// A value found for a line (VS Code `InlineSegment`).
struct Segment {
    column: u32,
    text: String,
}

impl CodeEditor {
    /// Shows inline values while the host's debugger is paused at
    /// `context` in this editor's model (VS Code's debugger on a focused
    /// stack frame). The editor asks the
    /// [`InlineValuesProvider`](crate::providers::InlineValuesProvider)s
    /// for the values on and near the screen, and `resolve` looks up each
    /// variable and evaluates each expression against the debug adapter,
    /// giving `None` for one it can't. Each line then shows its values
    /// after its end, as `name = value`, separated by commas, in
    /// `editor.inlineValuesForeground` on `editor.inlineValuesBackground`.
    ///
    /// The values are asked for again after scrolling past what was asked
    /// for, 200ms after an edit, and on
    /// [`refresh_inline_values`](crate::providers::refresh_inline_values).
    /// They show only while the editor shows this model. Call again for
    /// another frame, and [`CodeEditor::clear_inline_values`] when the
    /// debugger continues.
    pub fn show_inline_values(
        &mut self,
        context: InlineValueContext,
        resolve: impl Fn(InlineValueLookup, &mut App) -> Task<Option<SharedString>> + 'static,
        cx: &mut Context<Self>,
    ) {
        self.inline_values.session = Some(Session {
            model: self.model().entity_id(),
            context,
            resolve: Rc::new(resolve),
        });
        self.inline_values.requested = None;
        self.inline_values.task = None;
        cx.notify();
    }

    /// Removes the inline values and stops asking for them, e.g. when the
    /// debugger continues or stops.
    pub fn clear_inline_values(&mut self, cx: &mut Context<Self>) {
        self.inline_values.session = None;
        self.reset_inline_values(cx);
    }

    /// The inline values shown: each line's row and text, in row order.
    pub fn inline_values(&self) -> &[(u32, SharedString)] {
        &self.inline_values.lines
    }

    fn reset_inline_values(&mut self, cx: &mut Context<Self>) {
        let state = &mut self.inline_values;
        state.requested = None;
        state.task = None;
        if let Some(mut decorations) = state.decorations.take() {
            decorations.clear(cx);
        }
        if !std::mem::take(&mut state.lines).is_empty() {
            cx.notify();
        }
    }

    /// Asks for the values when the text, the rows on screen, the
    /// providers or a refresh call for it. Called before painting.
    pub(crate) fn refresh_inline_values(&mut self, cx: &mut Context<Self>) {
        let Some(session) = &self.inline_values.session else {
            return;
        };
        if session.model != self.model().entity_id() {
            if self.inline_values.requested.is_some() || !self.inline_values.lines.is_empty() {
                self.reset_inline_values(cx);
            }
            return;
        }
        let ranges = self.visible_ranges_plus_viewport_above_below(cx);
        let (Some(first), Some(last)) = (ranges.first(), ranges.last()) else {
            // No layout yet: ask once there is one.
            cx.notify();
            return;
        };
        let rows = first.start.row..last.end.row;
        let visible = self.visible_ranges(cx);
        let visible_rows = visible.first().map(|r| r.start.row).unwrap_or(rows.start)
            ..visible.last().map(|r| r.end.row).unwrap_or(rows.end);
        let target = self.provider_target(cx);
        let providers = inline_values_providers(cx, target);
        let model = self.model().read(cx);
        let version = model.buffer().version();
        let epoch = cx.try_global::<InlineValuesEpoch>().map_or(0, |e| e.0);
        let next = Requested {
            version,
            epoch,
            providers: providers.len(),
            rows: rows.clone(),
        };
        let edited = match &self.inline_values.requested {
            Some(requested) => {
                let covered = requested.rows.start <= visible_rows.start
                    && visible_rows.end <= requested.rows.end;
                if covered
                    && requested.version == next.version
                    && requested.epoch == next.epoch
                    && requested.providers == next.providers
                {
                    return;
                }
                requested.version != next.version
            }
            None => false,
        };
        self.inline_values.requested = Some(next);
        if edited {
            // VS Code's 200ms scheduler: ask once the edits pause. The values
            // shown move with the text meanwhile.
            self.inline_values.task = Some(cx.spawn(async move |this, cx| {
                cx.background_executor().timer(UPDATE_DELAY).await;
                this.update(cx, |this, cx| {
                    this.inline_values.requested = None;
                    cx.notify();
                })
                .ok();
            }));
            return;
        }
        if providers.is_empty() {
            self.set_inline_value_lines(Vec::new(), cx);
            return;
        }
        let context = model.provider_context();
        let rope = model.buffer().rope().clone();
        let inline_context = session.context.clone();
        let resolve = session.resolve.clone();
        let requests: Vec<_> = providers
            .iter()
            .flat_map(|provider| {
                ranges
                    .iter()
                    .map(|range| {
                        provider.provide_inline_values(&context, range.clone(), &inline_context, cx)
                    })
                    .collect::<Vec<_>>()
            })
            .collect();
        self.inline_values.task = Some(cx.spawn(async move |this, cx| {
            let mut values = Vec::new();
            for request in requests {
                match request.await {
                    Ok(found) => values.extend(found),
                    Err(err) => log::warn!("inline values request failed: {err:#}"),
                }
            }
            // The text of a value's range on its first line, for a variable
            // or expression without a name.
            let text_at = |range: &Range<PointUtf16>| {
                let row = range.start.row;
                if row > rope.max_point().row {
                    return String::new();
                }
                let end = if range.end.row == row {
                    range.end.column
                } else {
                    rope.line_len_utf16(row)
                };
                let line_end = rope.line_len_utf16(row);
                let start = rope
                    .point_utf16_to_offset(PointUtf16::new(row, range.start.column.min(line_end)));
                let end = rope.point_utf16_to_offset(PointUtf16::new(row, end.min(line_end)));
                rope.chunks_in_range(start..end.max(start)).collect()
            };
            let mut lines: Vec<(u32, Vec<Segment>)> = Vec::new();
            for value in values {
                let range = value.range().clone();
                let text = match value {
                    InlineValue::Text { text, .. } => Some(text.to_string()),
                    InlineValue::VariableLookup {
                        range,
                        variable_name,
                        case_sensitive_lookup,
                    } => {
                        let name = variable_name.map_or_else(|| text_at(&range), |n| n.to_string());
                        let lookup = InlineValueLookup::Variable {
                            name: name.clone().into(),
                            case_sensitive: case_sensitive_lookup,
                        };
                        let task = cx.update(|cx| resolve(lookup, cx));
                        task.await.map(|value| format!("{name} = {value}"))
                    }
                    InlineValue::EvaluatableExpression { range, expression } => {
                        let expression =
                            expression.map_or_else(|| text_at(&range), |e| e.to_string());
                        if expression.is_empty() {
                            None
                        } else {
                            let lookup = InlineValueLookup::Expression(expression.clone().into());
                            let task = cx.update(|cx| resolve(lookup, cx));
                            task.await.map(|value| format!("{expression} = {value}"))
                        }
                    }
                };
                let Some(text) = text.filter(|t| !t.is_empty()) else {
                    continue;
                };
                let row = range.start.row;
                let ix = match lines.binary_search_by_key(&row, |(r, _)| *r) {
                    Ok(ix) => ix,
                    Err(ix) => {
                        lines.insert(ix, (row, Vec::new()));
                        ix
                    }
                };
                let segments = &mut lines[ix].1;
                // VS Code de-dupes a line's values.
                if !segments.iter().any(|s| s.text == text) {
                    segments.push(Segment {
                        column: range.start.column,
                        text,
                    });
                }
            }
            let lines = lines
                .into_iter()
                .map(|(row, mut segments)| {
                    segments.sort_by_key(|s| s.column);
                    let text = segments
                        .into_iter()
                        .map(|s| s.text)
                        .collect::<Vec<_>>()
                        .join(", ");
                    (row, truncate(text).into())
                })
                .collect();
            this.update(cx, |this, cx| this.set_inline_value_lines(lines, cx))
                .ok();
        }));
    }

    /// VS Code `createInlineValueDecoration`: a no-break space, then the
    /// values in their colors, after the end of the line.
    fn set_inline_value_lines(&mut self, lines: Vec<(u32, SharedString)>, cx: &mut Context<Self>) {
        self.inline_values.task = None;
        if lines.is_empty() && self.inline_values.lines.is_empty() {
            return;
        }
        let model = self.model().clone();
        let rope = model.read(cx).buffer().rope().clone();
        let colors = &self.theme.colors;
        let style = TokenStyle {
            foreground: Some(colors.inline_values_foreground),
            ..Default::default()
        };
        let background = Some(colors.inline_values_background);
        let mut decorations = Vec::new();
        for (row, text) in &lines {
            if *row > rope.max_point().row {
                continue;
            }
            let end = PointUtf16::new(*row, rope.line_len_utf16(*row));
            let after = |content: SharedString, style: TokenStyle, background| DecorationOptions {
                after: Some(InjectedText {
                    content,
                    style,
                    background,
                    cursor_stops: InjectedTextCursorStops::None,
                    ..Default::default()
                }),
                show_if_collapsed: true,
                ..Default::default()
            };
            decorations.push((
                end..end,
                after("\u{a0}".into(), TokenStyle::default(), None),
            ));
            decorations.push((
                end..end,
                after(no_break_spaces(text).into(), style, background),
            ));
        }
        let owner = cx.entity_id();
        let collection = self
            .inline_values
            .decorations
            .get_or_insert_with(|| DecorationsCollection::for_owner(model, owner));
        collection.set(decorations, cx);
        self.inline_values.lines = lines;
        cx.notify();
    }
}

/// VS Code `replaceWsWithNoBreakWs`: spaces don't collapse.
fn no_break_spaces(text: &str) -> String {
    text.replace([' ', '\t'], "\u{a0}")
}

/// The first [`MAX_INLINE_DECORATOR_LENGTH`] characters and "..." when
/// longer.
fn truncate(text: String) -> String {
    if text.chars().count() > MAX_INLINE_DECORATOR_LENGTH {
        let mut cut: String = text.chars().take(MAX_INLINE_DECORATOR_LENGTH).collect();
        cut.push_str("...");
        cut
    } else {
        text
    }
}
