//! Monaco's `ICodeEditor` geometry and scrolling queries
//! (`src/vs/editor/browser/widget/codeEditor/codeEditorWidget.ts`, MIT):
//! the layout info, line and column offsets, content and scroll sizes,
//! and scrolling with a [`ScrollType`].
//!
//! Monaco answers these from its view layout, which exists before the first
//! render. Here they come from the last frame, so the ones that need a
//! layout return `None` until the editor was painted.

use gpui::{App, Context, Pixels, px};
use text::{Bias, PointUtf16};

use crate::editor::{CodeEditor, CodeEditorEvent};
use crate::layout_info::LayoutInfo;
use crate::view::reveal::ScrollType;

impl CodeEditor {
    /// The editor's layout as of the last frame (Monaco `getLayoutInfo`).
    pub fn layout_info(&self) -> Option<LayoutInfo> {
        Some(self.layout.as_ref()?.layout_info)
    }

    /// `position` clamped to the text the view lines show.
    fn validate_view_position(&self, position: PointUtf16) -> PointUtf16 {
        self.view_lines
            .rope()
            .clip_point_utf16(position, Bias::Left)
    }

    /// The view line showing `position`.
    fn view_line_of(&self, position: PointUtf16) -> u32 {
        let position = self.validate_view_position(position);
        self.view_lines.to_view(position, false).0
    }

    /// The top of the view line showing `position`, from the top of the
    /// content; with `include_view_zones`, of the view zones right above it.
    fn vertical_offset_for(
        &self,
        position: PointUtf16,
        include_view_zones: bool,
    ) -> Option<Pixels> {
        let layout = self.layout.as_ref()?;
        let line = self.view_line_of(position);
        let mut top = layout.zones.line_y(line, layout.metrics.line_height);
        if include_view_zones {
            // Monaco `getVerticalOffsetForLineNumber(_, true)`: above the
            // whitespace after the previous line (not above `padding.top`).
            let mut above = layout.zones.heights_after(line as i64 - 1);
            if line == 0 {
                above -= self.options.padding.top;
            }
            top -= above;
        }
        Some(top)
    }

    /// The top of `row`'s first view line, from the top of the content
    /// (Monaco `getTopForLineNumber`); with `include_view_zones`, the top of
    /// the view zones right above it. Rows inside a fold give the fold's
    /// header line.
    pub fn top_for_line_number(&self, row: u32, include_view_zones: bool) -> Option<Pixels> {
        self.vertical_offset_for(PointUtf16::new(row, 0), include_view_zones)
    }

    /// The bottom of `row`'s last view line, from the top of the content
    /// (Monaco `getBottomForLineNumber`); with `include_view_zones`, the
    /// bottom of the view zones right below it.
    pub fn bottom_for_line_number(&self, row: u32, include_view_zones: bool) -> Option<Pixels> {
        let layout = self.layout.as_ref()?;
        let line = self.view_line_of(PointUtf16::new(row, u32::MAX));
        let line_height = layout.metrics.line_height;
        let mut bottom =
            layout.zones.line_y(line, line_height) + layout.zones.height(line, line_height);
        if include_view_zones {
            bottom += layout.zones.heights_after(line as i64);
        }
        Some(bottom)
    }

    /// The top of the view line showing `position` (the wrapped part with
    /// its column), from the top of the content (Monaco
    /// `getTopForPosition`).
    pub fn top_for_position(&self, position: PointUtf16) -> Option<Pixels> {
        self.vertical_offset_for(position, false)
    }

    /// The height of the line showing `position`, 0 in a folded region
    /// (Monaco `getLineHeightForPosition`).
    pub fn line_height_for_position(&self, position: PointUtf16) -> Option<Pixels> {
        let layout = self.layout.as_ref()?;
        let position = self.validate_view_position(position);
        if self.view_lines.is_row_hidden(position.row) {
            return Some(px(0.));
        }
        let line = self.view_line_of(position);
        Some(layout.zones.height(line, layout.metrics.line_height))
    }

    /// How far `column` of `row` is from the start of the line, if the
    /// line was laid out in the last frame (Monaco `getOffsetForColumn`,
    /// which also only measures rendered lines). Columns are 0-based UTF-16
    /// offsets; on a wrapped line the offset is within its part.
    pub fn offset_for_column(&self, row: u32, column: u32) -> Option<Pixels> {
        let layout = self.layout.as_ref()?;
        let position = self.validate_view_position(PointUtf16::new(row, column));
        let line = layout.line_for_point(position)?;
        Some(line.x_for_column(position.column))
    }

    /// The width of `row`'s text (of its last part when wrapped), if it was
    /// laid out in the last frame (Monaco `getWidthOfLine`).
    pub fn width_of_line(&self, row: u32) -> Option<Pixels> {
        let layout = self.layout.as_ref()?;
        let end = self.validate_view_position(PointUtf16::new(row, u32::MAX));
        Some(layout.line_for_point(end)?.width())
    }

    /// The visible column of `position`, with tabs expanded (Monaco
    /// `getVisibleColumnFromPosition` minus one: 0-based like the columns).
    pub fn visible_column_from_position(&self, position: PointUtf16, cx: &App) -> u32 {
        let model = self.model().read(cx);
        let position = model.validate_position(position);
        let line = model.line_content(position.row);
        crate::cursor::strings::visible_column_from_column(
            &line,
            position.column,
            model.options().tab_size,
        )
    }

    /// The column a status bar shows for `position` (Monaco
    /// `getStatusbarColumn`): 1-based like Monaco's, with tabs expanded to
    /// the next tab stop but every other character, wide or not, counting
    /// as one.
    pub fn statusbar_column(&self, position: PointUtf16, cx: &App) -> u32 {
        let model = self.model().read(cx);
        let position = model.validate_position(position);
        let line = model.line_content(position.row);
        crate::cursor::strings::to_statusbar_column(
            &line,
            position.column,
            model.options().tab_size,
        )
    }

    /// The width of the content: the longest line laid out, plus
    /// `scroll_beyond_last_column` and the vertical scrollbar, or, wrapping
    /// at the viewport, just the widest line (Monaco `getContentWidth`).
    pub fn content_width(&self) -> Option<Pixels> {
        Some(self.layout.as_ref()?.content_width)
    }

    /// The scrollable width: [`CodeEditor::content_width`], at least the
    /// text area's (Monaco `getScrollWidth`).
    pub fn scroll_width(&self) -> Option<Pixels> {
        let layout = self.layout.as_ref()?;
        Some(layout.content_width.max(layout.layout_info.content_width))
    }

    /// The scrollable height: the lines, view zones and
    /// `scroll_beyond_last_line`, at least the editor's height (Monaco
    /// `getScrollHeight`).
    pub fn scroll_height(&self) -> Option<Pixels> {
        let layout = self.layout.as_ref()?;
        Some(layout.max_scroll.y + layout.layout_info.height)
    }

    /// The vertical scroll position (Monaco `getScrollTop`): during a
    /// smooth scroll, where it is now, on the way to
    /// [`CodeEditor::future_scroll_position`].
    pub fn scroll_top(&self) -> Pixels {
        self.scroll_position().y
    }

    /// The horizontal scroll position (Monaco `getScrollLeft`): during a
    /// smooth scroll, where it is now.
    pub fn scroll_left(&self) -> Pixels {
        self.scroll_position().x
    }

    /// Scrolls vertically to `scroll_top` (Monaco `setScrollTop`), clamped
    /// to the scroll range, and emits [`CodeEditorEvent::ScrollChanged`].
    /// The horizontal scroll stays where it is now, or for a smooth scroll
    /// where it's going.
    pub fn set_scroll_top(
        &mut self,
        scroll_top: Pixels,
        scroll_type: ScrollType,
        cx: &mut Context<Self>,
    ) {
        let left = self.unchanged_axis(scroll_type).x;
        let position = gpui::point(left, scroll_top);
        self.set_scroll_position_with_scroll_type(position, scroll_type, cx);
    }

    /// Scrolls horizontally to `scroll_left` (Monaco `setScrollLeft`),
    /// clamped to the scroll range, and emits
    /// [`CodeEditorEvent::ScrollChanged`]. The vertical scroll stays where
    /// it is now, or for a smooth scroll where it's going.
    pub fn set_scroll_left(
        &mut self,
        scroll_left: Pixels,
        scroll_type: ScrollType,
        cx: &mut Context<Self>,
    ) {
        let top = self.unchanged_axis(scroll_type).y;
        let position = gpui::point(scroll_left, top);
        self.set_scroll_position_with_scroll_type(position, scroll_type, cx);
    }

    /// Monaco's partial scroll updates: a smooth one keeps the other
    /// axis's destination (`_smoothScrolling.to`), an immediate one its
    /// current position (`_state`).
    fn unchanged_axis(&self, scroll_type: ScrollType) -> gpui::Point<Pixels> {
        match scroll_type {
            ScrollType::Smooth => self.future_scroll_position(),
            ScrollType::Immediate => self.scroll_position(),
        }
    }

    /// Whether a smooth scroll is still on its way (Monaco
    /// `hasPendingScrollAnimation`).
    pub fn has_pending_scroll_animation(&self) -> bool {
        self.options.smooth_scrolling && self.animation.is_scrolling()
    }

    /// Monaco `StableEditorScrollState.capture`, restored in the next
    /// frame: keeps the first visible position where it is on screen while
    /// view zones above it change. Not at the top of the file, during a
    /// smooth scroll, or after the scroll position changed since the last
    /// frame (whatever changed it wins), and it leaves an anchor or reveal
    /// already waiting for the next frame alone.
    pub(crate) fn keep_scroll_stable(&mut self) {
        let Some(layout) = self.layout.as_ref() else {
            return;
        };
        if self.scroll_top <= px(0.)
            || layout.scroll_top != self.scroll_top
            || self.has_pending_scroll_animation()
            || self.pending_scroll_anchor.is_some()
            || self.pending_reveal.is_some()
        {
            return;
        }
        // Monaco `getVisibleRanges()[0].getStartPosition()`, from the frame
        // on screen.
        let line_height = layout.metrics.line_height;
        let Some(first) = layout.lines.iter().find(|line| {
            layout.zones.line_y(line.view_line, line_height)
                + layout.zones.height(line.view_line, line_height)
                > self.scroll_top
        }) else {
            return;
        };
        let top = layout.zones.line_y(first.view_line, line_height);
        self.pending_scroll_anchor = Some((
            PointUtf16::new(first.row, first.columns.start),
            top - self.scroll_top,
        ));
    }

    /// Reports layout and content size changes after a frame (Monaco
    /// `onDidLayoutChange` and `onDidContentSizeChange`).
    pub(crate) fn report_layout(&mut self, cx: &mut Context<Self>) {
        let Some(layout) = self.layout.as_ref() else {
            return;
        };
        let info = layout.layout_info;
        let size = (layout.content_width, layout.content_height);
        if self.last_layout_info != Some(info) {
            let changed = self.last_layout_info.replace(info).is_some();
            cx.emit(CodeEditorEvent::LayoutChanged(info));
            // Monaco `CodeActionController`: `onDidLayoutChange` hides the
            // code action menu, which is fixed in the window.
            if changed && self.is_code_action_menu_visible() {
                let editor = cx.entity().downgrade();
                cx.defer(move |cx| {
                    editor
                        .update(cx, |editor, cx| editor.hide_code_action_menu(cx))
                        .ok();
                });
            }
        }
        if self.last_content_size != Some(size) {
            let old = self.last_content_size.replace(size);
            cx.emit(CodeEditorEvent::ContentSizeChanged(
                crate::events::ContentSizeChange {
                    content_width: size.0,
                    content_height: size.1,
                    content_width_changed: old.is_none_or(|old| old.0 != size.0),
                    content_height_changed: old.is_none_or(|old| old.1 != size.1),
                },
            ));
        }
        // The frame may have scrolled (revealing the cursor) or changed the
        // scrollable size.
        self.emit_scroll_changed(cx);
    }
}
