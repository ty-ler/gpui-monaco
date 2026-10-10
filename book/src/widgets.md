# Widgets

Widgets embed any GPUI view (`AnyView`) in the editor. There are three kinds, matching Monaco's.

## View zones

A view zone (Monaco `IViewZone`) is vertical space inserted between lines, for example:

- inline diffs
- peek views
- review comments

```rust,ignore
use gpui_monaco::{ViewZone, ZoneHeight};

let id = editor.update(cx, |editor, cx| {
    editor.add_view_zone(
        ViewZone {
            after_line: 4,                   // 1-based; 0 = above the first line
            height: ZoneHeight::Lines(2.),   // or ZoneHeight::Pixels(px(80.))
            view: cx.new(|_| MyPanel::new()).into(),
        },
        cx,
    )
});
```

Zones follow edits: the line is tracked with an anchor. A zone is hidden while its line is inside a folded region. To resize a zone, call `set_view_zone_height(id, height, cx)`.

## Content widgets

A content widget (Monaco `IContentWidget`) is anchored to a text position and scrolls with the text, like a tooltip or an inline action button. `preference` is tried in order until the widget fits. If none fits, the widget uses the first, and an empty list hides it:

```rust,ignore
use gpui_monaco::{ContentWidget, ContentWidgetPreference};

editor.update(cx, |editor, cx| {
    editor.add_content_widget(
        ContentWidget {
            position: PointUtf16::new(15, 4),
            preference: vec![ContentWidgetPreference::Above, ContentWidgetPreference::Below],
            view: label.into(),
        },
        cx,
    )
});
```

To move a widget, call `set_content_widget_position(id, position, cx)`.

## Messages

`show_message(text, position, cx)` and `show_markdown_message(markdown, position, cx)` show a short message above a position (Monaco `MessageController`). It closes when the cursor moves, the editor loses focus, the mouse moves away or Escape is pressed. The editor uses it for "No definition found" and the read-only message.

## Overlay widgets

An overlay widget (Monaco `IOverlayWidget`) is pinned to the editor's viewport and doesn't scroll. Monaco's find widget is one. Positions are `TopRight`, `BottomRight`, `TopCenter` or `Custom(point)`.

```rust,ignore
use gpui_monaco::{OverlayPosition, OverlayWidget};

editor.update(cx, |editor, cx| {
    editor.add_overlay_widget(
        OverlayWidget { position: OverlayPosition::TopRight, view: badge.into() },
        cx,
    )
});
```

## Glyph margin widgets

A glyph margin widget (Monaco `IGlyphMarginWidget`) sits in the glyph margin beside a line, such as a breakpoint or a revert button. It follows its line through edits, gets the margin's width and the line's height, and receives its own clicks.

```rust,ignore
use gpui_monaco::GlyphMarginWidget;

editor.update(cx, |editor, cx| {
    editor.add_glyph_margin_widget(GlyphMarginWidget { row: 41, view: breakpoint.into() }, cx)
});
```

## Removing widgets and their state

`editor.remove_widget(id, cx)` removes any of the four kinds.

Widget views are ordinary entities. Update them as usual (`cx.notify()`), and the editor lays them out on its next frame. Views are laid out after the editor's own prepaint, so a widget can read editor state while it renders.
