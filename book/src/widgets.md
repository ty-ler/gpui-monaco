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
    // After line 4 (1-based; 0 = above the first line), two lines tall
    // (or ZoneHeight::Pixels(px(80.))).
    let mut zone = ViewZone::new(4, ZoneHeight::Lines(2.), cx.new(|_| MyPanel::new()).into());
    // Optional: a view for the gutter beside the zone (Monaco `marginDomNode`).
    zone.margin_view = Some(cx.new(|_| MyGutterIcon).into());
    editor.add_view_zone(zone, cx)
});
```

`ViewZone::new` sets Monaco's defaults; the other fields are Monaco `IViewZone`'s:

| Field | Monaco | Meaning |
|---|---|---|
| `after_column`, `after_column_affinity` | `afterColumn`, `afterColumnAffinity` | On a wrapped line, the zone goes after the view line showing this (0-based) column, not after the last one. |
| `ordinal` | `ordinal` | Orders zones after the same line, lowest first; defaults to the column, else 10000. Ties keep insertion order. |
| `show_in_hidden_areas` | `showInHiddenAreas` | Show the zone even when the line after it is folded away. |
| `suppress_mouse_down` | `suppressMouseDown` | A click on the zone places the cursor before it (upper half) or after it (lower half), as on text, instead of going to the zone's view. |
| `min_width` | `minWidthInPx` | Widens the horizontal scroll range. |
| `margin_view` | `marginDomNode` | A view in the gutter beside the zone. |

A zone's view is laid out at the scroll width and scrolls horizontally with the text; the margin view stays in the gutter. Zones follow edits: the line is tracked with an anchor. A zone is hidden while the line after it is inside a folded region (unless `show_in_hidden_areas`). To resize a zone, call `set_view_zone_height(id, height, cx)`; to move it, `layout_view_zone(id, after_line, after_column, cx)` (Monaco `layoutZone`).

As in Monaco, view zones belong to the model's view: `set_model` removes them all (and emits `ViewZonesChanged`). Content, overlay and glyph margin widgets stay, at the same positions in the new model.

## Content widgets

A content widget (Monaco `IContentWidget`) is anchored to a text position and scrolls with the text, like a tooltip or an inline action button. `preference` is tried in order until the widget fits. If none fits, the widget uses the first, and an empty list hides it:

```rust,ignore
use gpui_monaco::{ContentWidget, ContentWidgetPreference};

editor.update(cx, |editor, cx| {
    editor.add_content_widget(
        ContentWidget::new(
            PointUtf16::new(15, 4),
            vec![ContentWidgetPreference::Above, ContentWidgetPreference::Below],
            label.into(),
        ),
        cx,
    )
});
```

The other fields are Monaco `IContentWidget`'s: `secondary_position` (a position on the same line the widget also touches if it can), `position_affinity` (which view position a wrap boundary or injected text means), `allow_editor_overflow` and `suppress_mouse_down` (a click on the widget focuses the editor instead). A widget is at most as wide as the text area (the window when it overflows), like Monaco's `maxWidth`, and is kept inside the text area horizontally.

A content widget is clipped to the text area and paints under the sticky scroll, as in Monaco. With `allow_editor_overflow` (Monaco `allowEditorOverflow`), it can extend past the editor into the rest of the window instead: it paints above everything else, fits above or below its line against the window, and takes the mouse from whatever is under it. The editor's hover, suggest widget, parameter hints, code action menu, rename box and messages work this way, so they show in full in a short editor.

To move a widget, call `set_content_widget_position(id, position, cx)`. `layout_content_widget(id, ContentWidgetPosition { .. }, cx)` (Monaco `layoutContentWidget`) also changes its secondary position, preference and affinity; a `None` position or an empty preference hides it.

## Messages

`show_message(text, position, cx)` and `show_markdown_message(markdown, position, cx)` show a short message above a position (Monaco `MessageController`). It closes when the cursor moves, the editor loses focus, the mouse moves away or Escape is pressed. The editor uses it for "No definition found" and the read-only message.

## Overlay widgets

An overlay widget (Monaco `IOverlayWidget`) is pinned to the editor's viewport and doesn't scroll. Monaco's find widget is one. Positions are `TopRight`, `BottomRight`, `TopCenter`, `Custom(point)` or `Manual` (Monaco's `null`: the view is given the editor's area and places itself).

```rust,ignore
use gpui_monaco::{OverlayPosition, OverlayWidget};

editor.update(cx, |editor, cx| {
    let mut widget = OverlayWidget::new(OverlayPosition::TopRight, badge.into());
    widget.stack_ordinal = Some(1); // stack with others in the corner
    editor.add_overlay_widget(widget, cx)
});
```

As in Monaco, the right corners keep twice the vertical scrollbar's width plus the minimap's from the editor's right edge, and `BottomRight` twice the horizontal scrollbar's height from its bottom. Widgets with a `stack_ordinal` stack in their corner, lowest ordinal first; `min_content_width` (Monaco `getMinContentWidthInPx`) widens the horizontal scroll range. `layout_overlay_widget(id, position, stack_ordinal, cx)` moves one.

Overlay widgets paint above the text, the sticky scroll and the other widgets, clipped to the editor unless `allow_editor_overflow` is set (then they're kept inside the window, like the context menu).

## The mouse over widgets

Widgets get their own clicks: the editor doesn't take focus, move the cursor, open its context menu or start middle-click scrolling under them. The wheel over an overlay widget, an overflowing widget or a zone widget (the peek view, the problem zone) scrolls the widget if it can and never the editor, as Monaco's are outside its scrollable element. Plain view zones, content widgets and glyph margin widgets are part of the text, so the wheel over them scrolls the editor. Parts of zones and widgets scrolled out of the editor, or under the sticky scroll and scrollbars, take no clicks.

## Glyph margin widgets

A glyph margin widget (Monaco `IGlyphMarginWidget`) sits in the glyph margin beside a line, such as a breakpoint or a revert button. It follows its line through edits, gets a lane's width (the line height) and the line's height, and receives its own clicks.

```rust,ignore
use gpui_monaco::{GlyphMarginLane, GlyphMarginWidget};

editor.update(cx, |editor, cx| {
    editor.add_glyph_margin_widget(GlyphMarginWidget::new(41, breakpoint), cx);
    // In the left lane, over glyph decorations there.
    editor.add_glyph_margin_widget(
        GlyphMarginWidget { lane: GlyphMarginLane::Left, z_index: 10, ..GlyphMarginWidget::new(41, marker) },
        cx,
    )
});
```

As in Monaco, the glyph margin has a lane per position (`Left`, `Center`, `Right`) in use on some line, each a line height wide. Where widgets and glyph decorations share a line and a lane, the one with the highest `z_index` shows, a widget before a decoration on a tie.

## Removing widgets and their state

`editor.remove_widget(id, cx)` removes any of the four kinds.

Widget views are ordinary entities. Update them as usual (`cx.notify()`), and the editor lays them out on its next frame. Views are laid out after the editor's own prepaint, so a widget can read editor state while it renders.
