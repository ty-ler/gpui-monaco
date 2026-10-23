# Decorations

Decorations (Monaco `deltaDecorations`) style ranges of text without changing it. Typical uses:

- search results
- diagnostics
- git gutter marks
- breakpoints
- inline hints

They live on the `TextModel`. Their ranges follow edits as Monaco's do. Edges inside a replaced span stay as far into the new text as they were into the old, up to the shorter of the two, so an equal-length replace such as Transform to Uppercase leaves them where they were.

```rust,ignore
use gpui_monaco::{
    DecorationColor, DecorationOptions, DecorationUnderline, InjectedText, OverviewRulerDecoration,
    OverviewRulerLane,
};

let ids = model.update(cx, |model, cx| {
    model.delta_decorations(
        None,          // owner: None shows them in every editor of the model
        &old_ids,      // decorations to replace (Monaco's oldDecorations)
        vec![
            (range.clone(), DecorationOptions {
                background: Some(hsla(0.15, 0.8, 0.5, 0.25)),
                // A theme color id follows theme changes (Monaco `ThemeColor`).
                overview_ruler: Some(OverviewRulerDecoration::new(
                    DecorationColor::theme("editorOverviewRuler.findMatchForeground"),
                    OverviewRulerLane::Center,
                )),
                hover_message: vec!["**Note:** found here".into()],
                ..Default::default()
            }),
            (line_range, DecorationOptions {
                is_whole_line: true,
                glyph: Some(("●".into(), red)),   // glyph margin (enable `glyph_margin`)
                gutter_bar: Some(green),         // like a git change bar
                ..Default::default()
            }),
            (word_range, DecorationOptions {
                underline: Some(DecorationUnderline { color: blue, wavy: false }),
                after: Some(InjectedText {
                    content: ": i32".into(),
                    style: TokenStyle { foreground: Some(gray), italic: true, ..Default::default() },
                    ..Default::default()
                }),
                ..Default::default()
            }),
        ],
        cx,
    )
});
```

Ranges are byte offsets. To convert from `PointUtf16`, use `model.buffer().point_utf16_to_offset(point)`.

## `DecorationOptions`

| Field | Monaco | Effect |
|---|---|---|
| `background`, `border` | `className` styling | fills or outlines the range; an empty piece (an empty range, or an empty line inside one) draws nothing. Touching or overlapping decorations of one z-index that paint the same merge into one box, as Monaco merges same-class ones; a range starting at a wrap point paints nothing on the view line above |
| `border_style`, `border_width` | CSS `border-style`, `border-width` | the border's style (`Dashed` stands in for CSS `dotted`, which GPUI can't draw) and width (`None` is 1px) |
| `outline` | CSS `outline` | a border just outside the range, shown on an empty range too |
| `min_width` | CSS `min-width` | the smallest width of the background on a line, as Monaco's snippet placeholders' 2px |
| `underline` | `inlineClassName` with `text-decoration` | straight or wavy underline |
| `text` | `inlineClassName` | overrides the text's color, bold or italic |
| `is_whole_line` | `isWholeLine` | the background spans the full line width, under every decoration that isn't whole-line |
| `should_fill_line_on_line_break` | `shouldFillLineOnLineBreak` | where the range goes past a line break, the background fills to the line's right edge |
| `include_new_lines` | `includeNewLines` (find matches) | where the range goes past a line break, the background covers it, half a character wide |
| `margin_background` | `marginClassName` | a background across the gutter on the view lines of the range |
| `lines_decoration_text` | `linesDecorationsClassName` (icons) | text such as `+` in the line decorations area on each view line of the range |
| `lines_decorations_tooltip` | `linesDecorationsTooltip` | a tooltip over the line decorations area and the folding column on the lines `gutter_bar`, `lines_decoration_text` and `first_line_decoration` draw on |
| `block` (`BlockDecoration`) | `blockClassName` + `blockPadding`, `blockIsAfterEnd`, `blockDoesNotCollapse` | a bordered box around the decorated lines and the view zones around them; on an empty range just the zones above its line unless `does_not_collapse`, and with `is_after_end` just the zones below the last line |
| `line_height` | `lineHeight` | the covered lines' height, as a multiple of the line height; text stays centered |
| `glyph` | `glyphMarginClassName` | a short glyph in the glyph margin, on every view line of the range; the highest `z_index` wins a lane. A codicon is 16px (Monaco's `codicon.css`), centered in its lane, whatever the font size; other text is in the editor's font |
| `glyph_margin` | `glyphMargin` | the glyph's lane (`GlyphMarginLane::Left`, `Center` or `Right`), and `persist_lane` to keep the lane on every line; the margin is a line height wide per lane in use |
| `glyph_margin_hover_message` | `glyphMarginHoverMessage` | markdown (`Vec<MarkdownString>`, a hover row each) shown beside the gutter when the mouse rests on the glyph margin of any row the decoration covers, in its `glyph_margin` lane |
| `line_number_hover_message` | `lineNumberHoverMessage` | markdown (`Vec<MarkdownString>`, a hover row each) shown beside the gutter when the mouse rests on the line number of any row the decoration covers |
| `line_number_color` | `lineNumberClassName` | colors the line number |
| `gutter_bar` | `linesDecorationsClassName` | a bar between the line numbers and the text, on each view line of the range |
| `first_line_decoration` (`LinesDecoration`) | `firstLineDecorationClassName` | a bar or text in the line decorations area, only on the range's first view line |
| `overview_ruler` (`OverviewRulerDecoration`) | `overviewRuler` | a mark in the scrollbar's overview ruler, in a color (`dark_color` in dark themes) and lane, spanning the range's lines |
| `minimap` (`MinimapDecoration`) | `minimap` | a mark in the minimap: `Inline` over the characters with a half-color band across the line, or a 2px `Gutter` bar; or a section header (`section_header_style`, `section_header_text`) |
| `before`, `after` | `before`, `after` | text injected before or after the range (see below) |
| `show_if_collapsed` | `showIfCollapsed` | an empty range still shows: its injected text only shows with it, its background is widened to a character's width (so are narrower ones, such as a zero-width space's), and a range ending at a line's start stops at the end of the line before in the text area (the glyph margin and line decorations keep the decoration's own range) |
| `font_family`, `font_size`, `font_weight`, `font_style` | `fontFamily`, `fontSize`, `fontWeight`, `fontStyle` | the decorated text's font (text in another size is centered on the line); line wrapping still measures the editor's font. With `allow_variable_fonts` off (and by default with a screen reader) the editor leaves such decorations out entirely |
| `text_direction` | `textDirection` | kept for the getters; GPUI can't lay out right-to-left lines, so they stay left to right |
| `hover_message` | `hoverMessage` | markdown (`Vec<MarkdownString>`) shown in the hover over the range, or anywhere on its lines with `is_whole_line`; `show_if_collapsed` ones take a character more each side |
| `stickiness` | `stickiness` | whether typing at the edges grows the range |
| `collapse_on_replace_edit` | `collapseOnReplaceEdit` | an edit replacing the whole range collapses it to the edit's start, so pasting over a link doesn't decorate the pasted text |
| `hide_in_comment_tokens`, `hide_in_string_tokens` | `hideInCommentTokens`, `hideInStringTokens` | the decoration isn't drawn while its range lies in comment (string) tokens, as the language's highlighting finds them |
| `z_index` | `zIndex` | paint order for overlapping decorations: among the whole-line ones, painted first (the fold background is one, at 0), then among the others |

Monaco's class names become styles here, so `className` effects beyond these fields can't be expressed. `inlineClassNameAffectsLetterSpacing` has no equivalent: it tells Monaco's DOM renderer to measure again, and text here is always shaped with its styles. A decoration's `font_size` (Monaco `fontSize`) shapes its text in that size, centered on the line, which keeps its height (set `line_height` for a taller one); wrapping still measures the editor's font. `get_font_size_at_position` returns the first such decoration's size at a position, else the editor's.

### Hover messages

`hover_message`, `glyph_margin_hover_message` and `line_number_hover_message` take Monaco's `IMarkdownString | IMarkdownString[]` as a `Vec<MarkdownString>`, so they can be trusted (`command:` links run), use `$(icon)`s or HTML, and give several messages; an empty `Vec` (or only empty strings) shows nothing.

```rust,ignore
use gpui_monaco::providers::MarkdownString;

DecorationOptions {
    // A blame annotation: hovers anywhere on the line.
    is_whole_line: true,
    hover_message: vec![
        MarkdownString::new("$(git-commit) [abc123](command:myExt.showCommit)")
            .trusted(true)
            .with_theme_icons(true),
    ],
    // A row each in the card beside the gutter.
    glyph_margin_hover_message: vec!["**Breakpoint**".into(), "Condition: `x > 3`".into()],
    line_number_hover_message: vec!["Changed in the last commit".into()],
    ..Default::default()
}
```

The glyph margin and line number hovers show in a card just right of the glyph margin (or of the line numbers), centered on the hovered line, as Monaco's glyph hover does. In the glyph margin only the decorations in the hovered lane count; a decoration covering several rows hovers on each of them. While the card shows, a decoration change computes it again after the hover delay, and it closes once nothing is left to show. Moving to another row or lane closes it at once, without the content hover's `hiding_delay`.

### Theme colors

Overview ruler and minimap colors are `DecorationColor`s: a resolved `Hsla` (`.into()` converts one) or `DecorationColor::theme(id)`, a theme color id (Monaco `ThemeColor`) looked up in the editor's theme when drawn, so the mark follows theme changes. `Theme::color(id)` resolves any of Monaco's color ids, with its defaults.

### Injected text

`before` and `after` (`InjectedText`) show text at the range's start and end that isn't part of the model. On an empty range they show only with `show_if_collapsed`, as in Monaco.

- `style` colors the text, and `tokens` styles parts of it (Monaco's internal `tokens`, as ghost text uses), by byte ranges of `content`. The minimap shows the text too, as Monaco's does, in its `tokens` colors or the default text color.
- `font_family`, `font_size`, `background`, `swatch` and `text_box` (margins, padding and corners) shape and box it, as inlay hints and color decorators do.
- `attached_data` (`AttachedData`) is the host's data, read back from the mouse target over the text (`MouseTarget::injected_text`) and from `injected_text_decorations`.
- `cursor_stops` (`InjectedTextCursorStops`, Monaco `cursorStops`) says where the cursor may show around the text. The cursor never stops inside it: its model position is the same on both sides. From the model (typing, `set_selections`) the cursor shows before the text. Moving onto the text with Left, Right, Down, End or a click puts the cursor after it with `Both` or `Right`, and before it with `Left` or `None`; Up puts it before it. A click on the text goes to its nearer side the stops allow. Inlay hints stop only after the hint, and inline suggestions only before.

## Collections

`editor.create_decorations_collection(decorations, cx)` returns a `DecorationsCollection` (Monaco `createDecorationsCollection`). It's a set of decorations that the editor owns, positioned with UTF-16 points:

- `set` replaces all of them, `append` adds more and `clear` removes them.
- `len`, `ids` and `has` describe the set.
- `range(i)` and `ranges()` give their current positions after edits.

It saves keeping the ids yourself:

```rust,ignore
let mut highlights = editor.update(cx, |editor, cx| editor.create_decorations_collection(vec![], cx));
highlights.set(vec![(start..end, options)], cx);
```

`on_did_change(callback, cx)` (Monaco `onDidChange`) calls back when the model's decorations change, except by the collection itself.

## Events

The model emits `TextModelEvent::DecorationsChanged(DecorationsChange)` (Monaco `onDidChangeDecorations`), and an editor forwards it as `ModelDecorationsChanged`:

- When decorations are added, removed or have their options changed. The `DecorationsChange` says whether the minimap, overview ruler, glyph margin or line numbers are affected.
- On every edit, undo and redo while the model has decorations, since they move with the text. A list built from `collection.ranges(cx)` stays current this way.
- Not for a delta that removes and adds nothing, such as removing ids that are already gone. So a listener that sets decorations again on this event doesn't loop.

`LineHeightChanged` and `FontChanged` (Monaco `onDidChangeLineHeight` and `onDidChangeFont`) come before it when decorations with a `line_height` or a font were added, changed or removed. They list each decoration's first row and owner, and `line_height` is `None` once it's gone. An editor forwards the changes of the decorations it shows. Line heights are forwarded only with `allow_variable_line_heights`, and fonts only while `allows_variable_fonts()`.

## Ownership

Pass the editor's entity id as `owner` to show decorations only in that editor (Monaco's `ownerId`). The editor uses this internally for find matches and bracket highlights, so a split view doesn't show the other editor's search. Use `None` for decorations every editor should show.

Look decorations up with `model.decorations_in_range(range, editor)`, `model.all_decorations(editor)` or `model.decoration(id)`. The `editor` argument filters by owner. Remove them by passing their ids as `old` with an empty `new`.
