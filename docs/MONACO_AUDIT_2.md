# Monaco parity audit 2 (2026-10-11)

> **Status:** every issue filed from this audit (#245–#265, #272) is fixed. The findings below describe the port as it was on 2026-10-11.

A second, wider audit, done after the F8 problem zone turned out to be a placeholder rather than a port (#244). Eleven read-only audits compared the port against the vendored Monaco source and CSS: every widget's sizing, clipping, scrolling, click-through, focus, keyboard and styling, plus the core view and input, the diff editors, every `contrib/` command and keybinding, every option, the theme colors and built-in themes, and the standalone editor and languages APIs. Widget findings were confirmed with playground screenshots where possible. Mouse drags can't be scripted in the playground, so those findings come from reading the code.

Some vendored folders are missing (`vs/platform`, most of `vs/base/browser/ui`). Where a report relies on Monaco code or defaults from outside the vendored tree, it says so.

Severity: **P1** visibly broken or wrong behavior; **P2** a missing Monaco sub-feature or a notable fidelity gap; **P3** polish. Screenshots named in the reports were taken during the audit and aren't committed.

Several findings in the earlier audit ([`MONACO_AUDIT.md`](MONACO_AUDIT.md)) were marked fixed but aren't: `suggest.fitWidthToDetails`, `shareSuggestSelections` and the diff editor's `hideOriginalLineNumbers`. They're filed again here.

## Issues

| Issue | Scope | Sections |
|---|---|---|
| #245 | Widget infrastructure: wheel, clicks, clipping and focus | A, B, C, D, E, I |
| #246 | Codicons and markdown rendering | A, B, C, D, H |
| #247 | Hover widget fidelity | A, H |
| #248 | Suggest widget and parameter hints fidelity | A, F.B, G, J |
| #249 | Inline completions UI and provider gaps | A, F.A, G, J |
| #250 | Find widget fidelity and find-related commands | B, F.A, F.B |
| #251 | Quick input, code action menu, context menu, rename, message and color picker | B, F.A, F.B |
| #252 | Peek view and zone/content/overlay widget API | C, F.A, I |
| #253 | Code lens and inlay hints | C |
| #254 | Sticky scroll, gutter, folding controls and line highlight | C, E, G, I |
| #255 | Minimap | C, G, I |
| #256 | Scrollbars, overview ruler and scroll shadows | C, E |
| #257 | Diff editor | D, F.A, G, I |
| #258 | Multi diff editor | D |
| #259 | Core rendering: cursor, selection, control characters, wrapping | E, G |
| #260 | Input: drag autoscroll, mouse gestures, IME and editing commands | E, F.B |
| #261 | Commands, keybindings and context keys | F.A, F.B, I |
| #262 | Options: validation, defaults and layout widths | G, F.B |
| #263 | Theme colors and token rules | H |
| #264 | Editor API correctness and gaps | I |
| #265 | Languages API: provider ranking, registration, missing types | J |

Shared root causes are filed once, in #245, so they aren't fixed widget by widget. The causes are: the editor's wheel handler ignoring widget bounds, right and middle clicks falling through, widgets clipped to the editor, unclipped hit areas, drags that stop at the editor's edge, widget inputs running every provider, and internal editors leaking into the public registries.

## A. Hover, suggest, parameter hints and inline completions UI


Scope: `crates/gpui-monaco/src/{hover.rs, markdown.rs, suggest_widget.rs, completion.rs, signature_help.rs}`, `src/inline_completions/{toolbar.rs, inline_edits_view.rs, long_distance_hint.rs, controller.rs}`, compared with `vendor/vscode/src/vs/editor/contrib/{hover,suggest,parameterHints,inlineCompletions}/browser/`. Unless a path says otherwise, paths are relative to `crates/gpui-monaco/src/` (ours) or `vendor/vscode/src/vs/editor/contrib/` (Monaco). `vs/base/browser/ui/hover/hoverWidget.css` is not vendored. Where I cite it, I'm working from what I know of Monaco.

Screenshots are in this directory:
- `hover1.png`: marker and markdown hover
- `hover_long.png`: long hover wrapping
- `hover_cmdk.png` / `hover_focus.png`: the hover-focus bug
- `suggest1.png`: suggest list and details
- `sig.png`: parameter hints
- `ghost.png`: ghost text

Background that several findings depend on:
- Hover, suggest and parameter hints are all ordinary content widgets (`add_content_widget`). They are laid out at min-content size and painted inside the editor's content mask (`element.rs:3561`, widget paint at `element.rs:3926`).
- The editor's `on_scroll_wheel` (`editor.rs:3660-3714`) scrolls the editor and calls `hide_hover` for every wheel event. Widget bounds don't change that.
- The only scrollable widget that stops wheel propagation is `MarkerZone` (`markers.rs:609`).

---

### P1: visibly broken

#### 1. The hover can't be scrolled with the mouse wheel: the wheel scrolls the editor and closes the hover
- **Severity:** P1
- **Monaco:** The hover body is a `DomScrollableElement` that consumes the wheel when it can scroll (`hoverWidget.ts` in vs/base). The controller hides the hover only when the editor's scroll position actually changes (`hover/browser/contentHoverController.ts:111`).
- **Ours:** The card is `.overflow_scroll().track_scroll(..)` with no `on_scroll_wheel(stop_propagation)` (`hover.rs:328-331`). The wheel event bubbles to `CodeEditor::on_scroll_wheel`, which scrolls the text and calls `hide_hover` (`editor.rs:3714`). Any hover taller than its maximum height (`hover.rs:1486-1490`) can only be read with the keyboard, and finding 6 breaks the keyboard path too. There's no scrollbar either.
- **Fix:** S. Add `on_scroll_wheel` with `stop_propagation` as `MarkerZone` does, and a scrollbar.

#### 2. The suggest list can't be scrolled with the wheel, and the wheel scrolls the editor underneath
- **Severity:** P1
- **Monaco:** The list is created with `alwaysConsumeMouseWheel: true` (`suggest/browser/suggestWidget.ts:236`) and has a visible scrollbar.
- **Ours:**
  - The rows are a hand-virtualized slice `self.scroll..end` inside `div().overflow_hidden()` (`suggest_widget.rs:798-811`).
  - `scroll` changes only through `keep_selected_visible` (keyboard).
  - There is no wheel handler anywhere in `suggest_widget.rs` or `completion.rs`.
  - Wheeling over the list scrolls the document instead.
  - There is no scrollbar or other indicator that more rows exist.
- **Fix:** S/M. Add a wheel handler that moves the first visible row and stops propagation, plus a scrollbar thumb.

#### 3. Wheeling over the suggest details panel also scrolls the editor
- **Severity:** P1
- **Monaco:** `suggest/browser/suggestWidgetDetails.ts:58-59` uses `alwaysConsumeMouseWheel: true`.
- **Ours:** `.overflow_y_scroll()` with no stop (`suggest_widget.rs:705`). The panel and the editor scroll together, so the list and details move away from the cursor while you read. There's no scrollbar.
- **Fix:** XS.

#### 4. Parameter hints have no maximum height and can't scroll
- **Severity:** P1
- **Monaco:**
  - `updateMaxHeight` caps the widget at `max(editorHeight/4, 250)` (`parameterHints/browser/parameterHintsWidget.ts:352-363`).
  - The body is a `DomScrollableElement` with `alwaysConsumeMouseWheel: true` (`:89-91`).
  - The wrapper is `max-width: 440px` (`parameterHints.css` `.phwrapper`).
- **Ours:**
  - `div().max_w(px(500.))` with no max height and no overflow handling (`signature_help.rs:150-190`).
  - Long signature or parameter docs make the widget grow without limit. It then overflows the editor and is clipped by the content mask (finding 5).
  - Nothing can be scrolled, and the wheel scrolls the editor.
- **Fix:** S.

#### 5. Hover, suggest and parameter hints are clipped to the editor and aren't capped to the space available
- **Severity:** P1
- **Monaco:**
  - All three set `allowEditorOverflow = true` (`hover/browser/resizableContentWidget.ts:18`, `suggest/browser/suggestWidget.ts:1028`, `parameterHintsWidget.ts:55`), so they can extend past a small editor.
  - The hover caps its height to the space above or below in the chosen direction, measured against the window (`contentHoverWidget.ts:189-200`, `resizableContentWidget.ts:64-104`).
  - The suggest widget caps its height to `max(maxHeightAbove, maxHeightBelow)` and picks the side with more room (`suggestWidget.ts:870-905`).
- **Ours:**
  - Widgets are painted inside `with_content_mask(bounds)` (`element.rs:3561`, `3926`).
  - x is clamped into the editor (`element.rs:1334-1337`).
  - `place_content_widget` falls back to the first preference when neither side fits, so the widget runs off the top or bottom and is cut off (`widgets.rs:61-82`).
  - The hover's maximum size ignores available space (`hover.rs:1484-1490`). The suggest list's size ignores it too (`suggest_widget.rs:414-426`).
  - In a short editor (peek view, diff side, an embedded editor), a hover or suggest list is clipped. Because the clipping is the editor's content mask and not the widget's own scroll container, the hidden part can't be scrolled into view.
- **Fix:** M. Paint these widgets outside the editor mask (deferred or overflowing layer), and size them by the available space as Monaco does.

#### 6. Focusing the hover from the keyboard closes it
- **Severity:** P1
- **Monaco:**
  - Cmd/Ctrl+K Cmd/Ctrl+I on a visible hover focuses it, so arrow keys, Page Up/Down and Home/End scroll it.
  - The hover controller never hides on editor blur (`contentHoverController.ts:104-111` has no blur listener).
- **Ours:**
  - `show_or_focus_hover` focuses the hover's own focus handle (`hover.rs:969-973`).
  - The editor subscribes `cx.on_blur(&focus_handle, ..)`, which calls `hide_hover` (`editor.rs:484-489`).
  - GPUI's `on_blur` fires whenever focus leaves that exact handle, including when it moves to the hover (gpui `context.rs:596-612`). Focusing the hover therefore removes it.
- **Verified:**
  - `hover_cmdk.png`: after `cmd-k`, the hover is still up.
  - `hover_focus.png`: after `cmd-k cmd-i`, the hover is gone.
- **Why tests miss it:** `tests/hover.rs:169` passes because focus events are only delivered when a frame is drawn.
- **Effect:** The hover's scroll and Escape actions (`hover.rs:350-385`) are effectively unreachable.
- **Fix:** XS/S. Don't hide on blur, or hide only on `on_focus_out` when the new focus isn't the hover. Add a test that draws a frame.

#### 7. The long-distance inline-edit hint and the pinned gutter indicator paint over the suggest list and parameter hints
- **Severity:** P1
- **Monaco:** Suggest is `z-index: 40` (`suggest/browser/media/suggest.css:10`), parameter hints are 39 (`parameterHints.css:8`), and the inline-edits view is 34, "below the find widget" (`inlineCompletions/browser/view/inlineEdits/view.css:16`).
- **Ours:**
  - `LongDistanceHint` is an overlay widget (`long_distance_hint.rs:8`). The above/below gutter indicators are overlay widgets too (`inline_edits_view.rs:1251-1265`).
  - Overlays are collected after content widgets (`element.rs:3420-3436`) and painted in that order, with no z-order.
  - The hint also `occlude()`s, so it steals clicks meant for the suggest rows under it.
- **Verified:** `suggest1.png` and `sig.png` show the "17 /// TODO … Tab to jump" box covering suggest rows and the parameter-hints docs.
- **Fix:** S. Add a z-order to embedded widgets, or paint suggest, parameter hints and hover last.

#### 8. In on-hover mode, the inline-suggestion toolbar's "⋯" menu is clipped by the hover card
- **Severity:** P1 (found in code, not screenshot-verified)
- **Monaco:** The More Actions menu is a context menu outside the widget (`hintsWidget/inlineCompletionsHintsWidget.ts`).
- **Ours, on-hover mode:**
  - The menu is an `absolute().top_full()` child of the toolbar (`inline_completions/toolbar.rs:228-277`).
  - In `OnHover` mode the toolbar is a section inside the hover card (`hover.rs:283-288`).
  - The card is `overflow_scroll` (`hover.rs:331`), which clips the menu.
- **Ours, always-shown mode:** The toolbar is a content widget. Its menu lies outside the min-content bounds used for `widget_bounds` (`element.rs:1323-1344`) and has no `occlude()`. Moving over the menu runs the editor's text-hover logic underneath (`editor.rs:3391-3410`).
- **Fix:** S. Render the menu as a deferred or anchored popup with `occlude()`, or reuse the context-menu component.

#### 9. Markdown tables come out as run-together text
- **Severity:** P1
- **Monaco:** marked renders GFM tables in hovers, suggest details and parameter-hint docs.
- **Ours:**
  - `Options::ENABLE_TABLES` is on (`markdown.rs:181`), but no `Tag::Table`, `TableHead`, `TableRow` or `TableCell` events are handled (`markdown.rs:199-270`, `_ => {}`).
  - Every cell's text goes into one inline buffer with no separators, so a table shows as a single garbled paragraph.
- **Fix:** S/M. Render a grid, or at least join cells with separators and break rows.

---

### P2: missing sub-features or notable fidelity gaps

#### 10. Hover, parameter-hints and details text use GPUI's default line height (about 1.618) instead of the editor's
- **Severity:** P2
- **Monaco:**
  - `contentHoverWidget.ts:286-289` sets the hover's font size and `lineHeight = editorLineHeight / fontSize`.
  - Parameter hints do the same in `updateFont` (`parameterHintsWidget.ts:117-126`).
  - The details panel uses the suggest line height (`suggestWidgetDetails.ts:94-101`).
- **Ours:**
  - `hover.rs:349`, `signature_help.rs:172` and `render_details` (`suggest_widget.rs:673`) set only `text_size`, so the gpui default `line_height: phi()` applies.
  - Hovers look noticeably looser and taller than Monaco's (`hover1.png`).
- **Fix:** XS.

#### 11. Theme keys: hover-family widgets use `editorWidget.*`, and links and code use unrelated colors
- **Severity:** P2
- **Monaco:**
  - Hover: `editorHoverWidget.{background,foreground,border,statusBarBackground,highlightForeground}` (`hover.css:10-37`, `parameterHints.css` top rule and `.parameter.active`, `inlineCompletionsHintsWidget.css` `.withBorder`).
  - Links: `textLink.foreground` and `textLink.activeForeground` (`hover.css:32-38`, `suggest.css` `.suggest-details a`).
  - Inline code: `textCodeBlock.background` (`hover.css:84`).
  - Suggest status bar: `editorSuggestWidgetStatus.foreground` (`suggest.css:66`).
  - Focused suggest row icon: `editorSuggestWidget.selectedIconForeground` (`suggest.css:127-129`).
- **Ours:**
  - `markdown_style()` uses `link = input_option_active_border` and `code_background = input_background` (`hover.rs:718-727`).
  - The hover, parameter hints and toolbar use `widget_background`, `widget_border` and `widget_foreground` (`theme.rs:1679-1681`). None of the `editorHoverWidget.*`, `textLink.*`, `textCodeBlock.*` or `editorSuggestWidgetStatus.*` keys are read.
  - The status bar is `suggest_foreground.opacity(0.5)` (`suggest_widget.rs:650`).
  - The focused row's icon keeps its symbol color (purple ƒ in `suggest1.png`).
- **Fix:** S.

#### 12. No rounded corners or shadow on the hover, suggest list, details, parameter hints or inline toolbar
- **Severity:** P2
- **Monaco:**
  - `border-radius: var(--vscode-cornerRadius-large)` and `box-shadow: var(--vscode-shadow-lg)` on the hover and parameter hints (`hover.css:12,26-28`, `parameterHints.css:15-16`).
  - `box-shadow: 0 2px 8px var(--vscode-widget-shadow)` on suggest (`suggest.css:13-14`). Details have a radius too.
- **Ours:** Square, flat boxes (`hover.rs:336-342`, `suggest_widget.rs:778-784, 694-704`, `signature_help.rs:169-174`). The `widget.shadow` key isn't read.
- **Fix:** XS.

#### 13. Problem-hover actions aren't a status bar, and Quick Fix doesn't check whether this marker has fixes
- **Severity:** P2
- **Monaco:**
  - Marker actions go into the hover's single status bar row at the bottom, after all parts, on `editorHoverWidget.statusBarBackground` (`markerHoverParticipant.ts:198-320`, `contentHoverRendered.ts:275-295`, `hover.css:79-81`).
  - "View Problem" appears only for Error, Warning and Info.
  - Quick Fix is hidden when read-only.
  - The marker's code actions are computed. The status bar shows "Checking for quick fixes..." and then either "Quick Fix..." or "No quick fixes available". It can also show an AI action and menu-contributed actions.
- **Ours:**
  - Each marker draws its own inline action row with a top border, followed by another section separator (the double rule in `hover1.png`) (`hover.rs:686-711`).
  - Quick Fix appears whenever any code-action provider is registered (`hover.rs:1480-1481`, `701`), even if it has nothing for this marker, and also in read-only editors.
  - View Problem appears for Hints too.
- **Fix:** M.

#### 14. Problem-hover body details differ
- **Severity:** P2
- **Monaco (`markerHoverParticipant.ts:131-187`):**
  - The message uses the editor font with `white-space: pre-wrap`.
  - `code.target` renders the code as a link.
  - In related information, only `file(line, col):` is a link, and the message is plain text in the editor font.
- **Ours:**
  - The message uses the UI font in a flex row (`hover.rs:638-647`).
  - `Marker.code` is a plain string with no target (`markers.rs:116`).
  - The whole related-info label is a link (`hover.rs:649-667`).
- **Fix:** S.

#### 15. No highlight on the hovered range
- **Severity:** P2
- **Monaco:** While a hover shows, Monaco decorates its range with `hoverHighlight` (`editor.hoverHighlightBackground`) (`contentHoverRendered.ts:222-226`, `hover.css:6-8`).
- **Ours:** No such decoration or theme key exists (no `hoverHighlight` anywhere in `src/`).
- **Fix:** S.

#### 16. Fenced code blocks in markdown are drawn as grey padded boxes
- **Severity:** P2
- **Monaco:** Fences render as tokenized `div.code` with `white-space: pre-wrap` and no background. Only inline `<code>` gets `textCodeBlock.background`, `padding: 0 .4em` and a 3px radius (`hover.css:84-86`, `suggest.css` `.suggest-details code`, `parameterHints.css` `.docs code`).
- **Ours:**
  - Fenced code gets `p(4) rounded(3) bg(code_background)` (`markdown.rs:136-145`). See the full-width grey box in `hover1.png` and `hover_long.png`.
  - Inline code spans get the background but no padding or radius (`markdown.rs:79-82`).
- **Fix:** XS.

#### 17. The markdown renderer is a subset, so several constructs show literally or disappear
- **Severity:** P2
- **Monaco:** `IMarkdownString` carries `isTrusted`, `supportThemeIcons`, `supportHtml` and `baseUri`. The renderer handles images (`img { max-width: 100% }`), a sanitized HTML subset (`<span style=color>`, `<br>`, `<img>`), `$(codicon)` theme icons, strikethrough, blockquotes, `ul/ol { padding-left: 20px }` with hanging indents, `command:` links when trusted, and link hover color.
- **Ours (`markdown.rs`):**
  - Image alt text is printed as plain text, because `Tag::Image` isn't handled and its inner `Text` is pushed.
  - Raw HTML and inline HTML are dropped.
  - `$(icon)` shows literally.
  - Strikethrough is enabled (`:181`) but not styled.
  - Blockquotes get no styling.
  - Nested lists are indented with leading spaces in the text (`:254-266`), so wrapped lines don't hang and code blocks inside list items aren't indented.
  - Only http, https and mailto links open (`:149-155`).
  - `MarkdownString` is a bare string with no trust or icon flags (`providers.rs:148`).
- **Fix:** M.

#### 18. Hover, details and parameter-hint text can't be selected
- **Severity:** P2
- **Monaco:** Hover text is `user-select: text`. Parameter hints set `element.style.userSelect = 'text'` (`parameterHintsWidget.ts:101`). Details text can be selected and copied with `suggestWidgetCopy`, which copies the selection.
- **Ours:** Everything is non-selectable `StyledText` or `InteractiveText` (`markdown.rs:100-121`). Copying is limited to the per-row copy button (hover) and copy-all (`details_text`, `suggest_widget.rs:390-399`).
- **Fix:** M.

#### 19. Suggest list sizing and resizing don't follow Monaco's `_layout`
- **Severity:** P2
- **Monaco (`suggestWidget.ts:822-920`):**
  - The height is capped to the space above or below.
  - Monaco prefers above when it doesn't fit below and there's more room above, and locks that preference while the session lasts (`:1097-1105`).
  - Sashes are enabled as (north, east) when shown above and (east, south) when below.
  - `suggest.fitWidthToDetails` exists, and width is capped to the window.
- **Ours:**
  - `list_size` is the stored or default size clamped only to the row count (`suggest_widget.rs:414-426`).
  - Placement re-runs `place_content_widget` every frame with [Below, Above], so the list can flip sides as it shrinks or grows while you type.
  - Only east and south sashes exist (`suggest_widget.rs:813-829`). When the list is above the cursor, dragging south grows it over the cursor line.
  - There's no `fitWidthToDetails`.
- **Fix:** M.

#### 20. Suggest details placement and size
- **Severity:** P2
- **Monaco:** `SuggestDetailsOverlay._placeAtAnchor` tries east, then west, then below or above, and takes the first that fits or the least bad (`suggestWidgetDetails.ts:383-430`). The details have their own resizable size with sashes (`:289-331`, `:474-476`) and are positioned independently of the list.
- **Ours:** East if `list + 330 + 14` fits in the space right of the cursor, otherwise stacked below the list. The width is a fixed 330, `max_h` equals the list height, and it isn't resizable (`suggest_widget.rs:836-863`). There's no west placement. Close to the right edge, details always go below, which makes the whole widget much taller (and the combined box is what gets clipped, see finding 5).
- **Fix:** M.

#### 21. Unicode glyphs stand in for codicons in every widget
- **Severity:** P2
- **Monaco:** Uses the codicon font everywhere: `symbol-*` kind icons, `chevron-right` read-more, `close`, `copy`, `add`/`remove` verbosity, `parameter-hints-previous`/`next`, toolbar `chevron-left`/`right`/`ellipsis`, gutter `keyboard-tab`/`arrow-up`/`arrow-down`/`check`.
- **Ours:**
  - Suggest kinds use ƒ 𝑥 ◆ π ⌘ {} ▤ ab @ ◎ ⚒ (`suggest_widget.rs:198-216`), which reads oddly (⌘ for keywords, 𝑥 for fields).
  - Read-more is "›" (`:597`). Details close is "×" (`:721`).
  - Hover copy is "⧉" (`hover.rs:436`). Verbosity is "+"/"−" (`hover.rs:515`).
  - Parameter-hint arrows are ⌃/⌄ (`signature_help.rs:76`).
  - Toolbar uses ‹ › ⋯ (`toolbar.rs:144-224`).
  - The gutter indicator uses ✓ ⇥ ↑ ↓ (`inline_edits_view.rs:388-405`). The long-distance hint pill uses ⇥ (`long_distance_hint.rs:397`).
- **Fix:** M (bundle codicons once and reuse them).

#### 22. Parameter hints: active-parameter style, separator, and parameter matching
- **Severity:** P2
- **Monaco:**
  - The active parameter is bold in `editorHoverWidget.highlightForeground` with no underline (`parameterHints.css` `.signature .parameter.active`).
  - `.signature.has-docs::after` draws a rule between the signature and the docs.
  - String parameter labels are found with a word-boundary regex, `(\W|^)label(?=\W|$)` (`parameterHintsWidget.ts:316-328`).
- **Ours:**
  - Bold plus underline in the foreground color (`signature_help.rs:128-142`).
  - No separator (`sig.png`).
  - `signature.find(text)` matches inside other words (`signature_help.rs:91-96`). For example, parameter `cat` in `concat(cat)` highlights the "cat" in "concat".
  - Max width is 500 rather than 440.
- **Fix:** S.

#### 23. Suggest and parameter hints don't close when the editor loses focus
- **Severity:** P2
- **Monaco:** `suggestController.ts:282-287` cancels suggest on `onDidBlurEditorWidget` (unless sticky). `parameterHintsModel.ts:83` cancels parameter hints on blur.
- **Ours:** `on_blur` and `on_focus_out` (`editor.rs:484-502`) never call `hide_suggest` or `hide_signature_help`. Clicking into another view leaves both popups open over the editor.
- **Fix:** XS.

#### 24. Missing screen-reader announcements
- **Severity:** P2
- **Monaco:**
  - Parameter hints call `aria.alert("{label}, {param doc}, {sig doc}, hint")` when the active parameter changes (`parameterHintsWidget.ts:242-262`).
  - The suggest list rows have aria labels, and focus changes are read.
  - The hover sets an aria label with the accessible-view hint (`contentHoverWidget.ts:352-361`).
- **Ours:** `announce` exists (`accessibility.rs:183`), but it's used only for generic messages and inline suggestions (`accessibility.rs:217`). Suggest, parameter hints and hover never announce.
- **Fix:** S.

#### 25. The inline-edit gutter indicator disappears when the glyph margin is off
- **Severity:** P2
- **Monaco:** The indicator spans the whole gutter (glyph margin, line numbers and decorations) and is shown whenever the gutter width is above 0 (`view/inlineEdits/components/gutterIndicatorView.ts:277-295`).
- **Ours:** The docked indicator is a glyph-margin widget and is skipped when there's no glyph margin. The code says so: "here the indicator needs the glyph margin" (`inline_edits_view.rs:1234-1238`, `element.rs:3399-3403`). Embedded editors with `glyph_margin: false` get no indicator, so no click-to-accept and no menu.
- **Fix:** M.

#### 26. The gutter-indicator menu is an unclamped absolute box
- **Severity:** P2
- **Monaco:** Shown through `hoverService.showInstantHover({ target: icon })` (`gutterIndicatorView.ts:486-499`). It's positioned and flipped to fit the window, and closes on editor scroll.
- **Ours:** `absolute().left_full().top_0()` from the gutter widget, painted inside the editor mask (`inline_edits_view.rs:503-575`). For an edit near the bottom of the editor the menu is cut off. It also doesn't close on scroll.
- **Fix:** S.

---

### P3: minor polish

#### 27. Hover row separators and padding
- **Severity:** P3
- **Monaco:** Rows carry `border-top` across the full card width, and contents have `padding: 4px 8px` (hoverWidget.css).
- **Ours:** The card has `p(6)` and a `gap(6)`, and separators are 1px divs inside that padding, so they're inset from the border (`hover.rs:216-291, 332-335`). Paragraph spacing is a 6px gap instead of `p { margin: 8px 0 }`.
- **Fix:** XS.

#### 28. Hover resize handle
- **Severity:** P3
- **Monaco:** Full-edge sashes: north and east when above, east and south when below (`resizableContentWidget.ts:34, 98-100`). The minimum is 150 × (lineHeight + 8).
- **Ours:** An 8px corner hotspot only (`hover.rs:544-619`). The minimum matches.
- **Fix:** S.

#### 29. Suggest labels and details clip without an ellipsis
- **Severity:** P3
- **Monaco:** `text-overflow: ellipsis` on `.main`, `.signature-label`, `.qualifier-label`, `.details-label` and the details `.type` (`suggest.css:136-260`, `:411-419`).
- **Ours:** Only `overflow_hidden()` (`suggest_widget.rs:538-566`), so glyphs are cut mid-character. GPUI already has `text_ellipsis()`, used in `long_distance_hint.rs:452`.
- **Fix:** XS.

#### 30. The details "type" line uses the UI font and wraps
- **Severity:** P3
- **Monaco:** The type line uses the editor font (`suggestWidgetDetails.ts:104`) with `white-space: pre`, ellipsis and a tooltip (`suggest.css` `.header > .type`).
- **Ours:** Inherits the UI font and wraps (`suggest_widget.rs:723-733`). See "fn(&mut self)" in the UI font in `suggest1.png`.
- **Fix:** XS.

#### 31. The inline toolbar menu is a hand-rolled list
- **Severity:** P3
- **Monaco:** A real context menu with keyboard navigation and keybinding labels.
- **Ours:** A `div` with no keyboard navigation, focus or Escape handling (`toolbar.rs:228-277`). The same applies to the gutter-indicator menu (`inline_edits_view.rs:503-575`).
- **Fix:** S.

#### 32. The core markdown API has no plain-text documentation type
- **Severity:** P3
- **Monaco:** `documentation: string` (and signature or parameter docs as strings) render as plain `pre-wrap` text, not markdown (`suggest.css` `.docs { white-space: pre-wrap }`, `parameterHintsWidget.ts:200-228`).
- **Ours:** `MarkdownString` only. The LSP adapter escapes plaintext and turns `\n` into hard breaks (`gpui-monaco-lsp/src/convert.rs:42-58`), but leading indentation is still lost. Direct API users who pass plain strings get them parsed as markdown, so `*`, `_` and `#` are interpreted and single newlines are joined.
- **Fix:** S.

---

### Counts

- P1: 9 (findings 1-9)
- P2: 17 (findings 10-26)
- P3: 6 (findings 27-32)

### Checked and matching (not listed above)

- Hover maximum size: 0.66 × width (at least 750) and height/4 (at least 250).
- Hover above/below preference, keyboard scroll actions, verbosity +/− behavior, and copy button placement.
- Suggest default size (430 / 12 rows / 220 minimum), row layout (label, label detail, right-side description or detail), deprecated styling, match highlights and frozen state, the status bar contents, and the message state at half width.
- Click-through on mouse down: the editor returns early inside widget bounds (`editor.rs:3016-3023`) for hover, suggest and parameter hints.
- Ghost text is italic and syntax-highlighted at 0.7 opacity (`ghost.png`).
- The long-distance hint layout algorithms (towers, flex box).


## B. Find, rename, code actions, context menu, quick input, message and color picker


Scope: `crates/gpui-monaco/src/{find_widget,find,rename,code_action,context_menu,quick_access,message,color_picker}.rs`, compared with `vendor/vscode/src/vs/editor/contrib/{find,rename,codeAction,contextmenu,quickAccess,message,colorPicker}/browser/` and `editor/standalone/browser/quickInput/`.

Note: `vs/platform/*` and `vs/base/browser/ui/*` are **not vendored**, so the action widget, menu and quick-input list CSS are cited from memory and flagged as such. Everything else was checked in code.

Screenshots (all `fr-*.png` in this directory) come from `pg-shot` on the sample file:
- `fr-find1.png` / `fr-find1-crop.png`: find widget
- `fr-findzoom-c.png`: find widget at zoom 2
- `fr-f1.png`, `fr-f1b.png`, `fr-f1b-c.png`, `fr-f1c-c.png`: command palette
- `fr-symk.png`: `@:` symbol list
- `fr-ca-c.png`: code action menu
- `fr-rn-c.png`: rename box
- `fr-ctx-c.png`: context menu
- `fr-ro-c.png`: read-only message

Pixel colors were sampled with `../fr-px.py`.

How the editor routes events matters for several findings:
- The editor root registers `on_mouse_down` Left/Right/Middle and `on_scroll_wheel` (`editor.rs:5039-5078`).
- Left-down returns early when the point is inside `layout.widget_bounds` (`editor.rs:3014-3023`). Right-down and middle-down do not check (see finding 1).
- `on_scroll_wheel` (`editor.rs:3660-3731`) never checks widget bounds. Any widget that doesn't call `cx.stop_propagation()` on wheel scrolls the editor underneath. `markers.rs:609` is the only widget that does.
- All embedded widgets are painted inside the editor's content mask (`element.rs:3561`, painted at `element.rs:3926`), so they can't overflow the editor.

---

### Cross-cutting

#### 1. Right-click and middle-click on any widget fall through to the editor, which steals focus and closes the widget
- **Severity:** P1
- **Fix size:** S
- **Monaco:**
  - `ContextMenuController._onContextMenu` returns early for `MouseTargetType.OVERLAY_WIDGET`, "to support right click on input field for example in find" (`contrib/contextmenu/browser/contextmenu.ts`, `_onContextMenu`).
  - Widgets keep focus. The quick input, rename box, action widget and menus are not closed by right-clicking inside them.
- **Ours:**
  - `on_right_mouse_down` calls `window.focus(&self.focus_handle)` unconditionally before computing the target (`editor.rs:2964-2993`). There is no `widget_bounds` check like the left-button path at `editor.rs:3014-3023`.
  - The widget roots only stop Left (`find_widget.rs:666`, `context_menu.rs:353`). Their inner `CodeEditor` inputs don't stop the right button either (`editor.rs:2964`).
  - So right-clicking inside the quick input, rename box, code action menu or context menu moves focus to the main editor. The blur handlers then close them: `quick_access.rs:226-231`, `rename.rs:114-118`, `code_action.rs:548`, `context_menu.rs:605-612`.
  - Right-clicking the find input moves focus out of the find widget.
  - Middle-click on the non-input parts of a widget (buttons, status, list rows) reaches `on_middle_mouse_down` (`editor.rs:2838-2880`). That starts middle-click autoscroll, or focuses the editor and runs open-link / go-to-definition on the text *under* the widget.
- **Fix:** do the `widget_bounds` early-return for all buttons, before focusing.

#### 2. Widget inputs are full editors with every provider on: ghost text, and a clipped suggest widget, appear inside the find, quick-input and rename inputs
- **Severity:** P1
- **Fix size:** S
- **Monaco:** the find, replace, quick-input and rename inputs are plain `<textarea>`/`<input>` elements. No completions, inline suggestions, hovers, occurrences or links run in them.
- **Ours:**
  - All three inputs use `EditorOptions::single_line_input()` (`find_widget.rs:86`, `quick_access.rs:214`, `rename.rs:98`).
  - `single_line_input()` (`options.rs:2112-2135`) only turns off gutter, minimap, sticky scroll, color decorators and the context menu. Every other option keeps its default: `quick_suggestions`, `inline_suggest`, `hover`, `occurrences_highlight`, `selection_highlight`, `links`, `code_lens`, `lightbulb`, `parameter_hints`, `inlay_hints` and word-based suggestions all stay on.
  - Providers registered with `LanguageSelector::All` therefore run in the inputs (playground `providers.rs:632`).
- **Verified in screenshots:**
  - `fr-find1-crop.png`: typing `fn` in the find input shows the demo inline completion as italic ghost text, "fn *render(&self) -> usize {*".
  - `fr-f1b-c.png` / `fr-f1c-c.png`: typing `>fo` in the command palette opens a suggest widget inside the 24px input. It is clipped to a thin `#04395e` bar under the text (pixel-sampled), and that is `suggest_selected_background`.
  - The QuickInput key bindings happen to win for Down and Enter, but the stray widget, ghost text and Tab-accept are live.
- **Fix:** add a proper "plain input" option set that turns all of these off. Better still, skip provider lookups for widget-owned editors.

#### 3. Widget inputs paint the editor background and use the editor's monospace font and zoom
- **Severity:** P2
- **Fix size:** XS for the background, S for the font
- **Monaco:**
  - Inputs use `input.background`. The find input is `font-size: 13px` in the UI font (`findWidget.css`: `.monaco-findInput .input { font-size: 13px }`). The quick input is 13px (`standaloneQuickInput.css`).
  - The rename input deliberately uses the editor font (`renameWidget.ts:263-271`, `applyFontInfo`).
- **Ours:**
  - The find and quick inputs get the main editor's theme unchanged (`find_widget.rs:79-92`, `quick_access.rs:211-218`). The inner editor paints `editor.background`.
  - Pixel-sampled: the find input interior is `#1e1e1e`, not `#3c3c3c`. Only a ~5px strip of the wrapper's `input_background` shows at the right edge (`fr-f1-crop.png`; quick input `1630,100 = #3c3c3c` vs `1200,100 = #1e1e1e`).
  - `rename.rs:504-506` already does the right override; the other two don't.
  - The inputs also render in the editor's monospace font at its size and follow editor zoom, while the rest of the widget stays at 12px (`fr-findzoom-c.png`).

#### 4. No codicons: every icon is a Unicode or emoji stand-in
- **Severity:** P2
- **Fix size:** M (bundle the codicon font or SVGs plus an `icon()` helper, then replace the call sites)
- **Monaco:** every widget uses codicons.
- **Ours, by widget:**
  - **Find widget:** `▸/▾`, `Aa`, `ab`, `.*`, `↑`, `↓`, `≡`, `×`, `AB`, and the text buttons "Replace"/"All" (`find_widget.rs:683-765`). Monaco uses `codicon-chevron-right/down`, `case-sensitive`, `whole-word`, `regex`, `arrow-up/down`, `selection`, `close`, `preserve-case`, `replace` and `replace-all`.
  - **Code action rows:** a colour emoji `💡` on every row (`code_action.rs:587`). `text_color` has no effect on emoji (see `fr-ca-c.png`). Monaco uses per-group icons (`codeActionMenu.ts:26-32,76`): lightbulb for Quick Fix, `wrench` for Extract/Inline/Rewrite/Move, `surround-with`, `symbol-file` for Source, and `sparkle` for AI.
  - **Lightbulb:** painted as a circle plus a rectangle (`element.rs:4169-4199`). Monaco uses `codicon-light-bulb`, `lightbulb-autofix` and the sparkle variants (`lightBulbWidget.ts:27-31`).
  - **Rename:** `✨`/`⏹` emoji (`rename.rs:277,316-318`). Monaco uses `Codicon.sparkle`/`stopCircle` (`renameWidget.ts:986-996,1034`).
  - **Quick-access symbols:** `ƒ ◆ 𝑥 π {} ab` (`quick_access.rs:139-151`). Monaco uses `symbol-*` codicons.
  - **Context menu:** the submenu arrow is `›` (`context_menu.rs:335`).
  - **Color picker:** `×` close (`color_picker.rs:458`) and no `codicon-color-mode` in the header (`colorPickerHeader.ts:35`).

#### 5. Widgets are clipped to the editor's bounds; Monaco's menus, action list and message overflow it
- **Severity:** P1 (only in short editors)
- **Fix size:** M to L (paint these in a deferred or top-level layer)
- **Monaco:**
  - The context menu and the code action list are in the body-level context view.
  - `MessageWidget.allowEditorOverflow = true` (`messageController.ts:146`).
- **Ours:**
  - Every widget element is painted inside `with_content_mask(bounds)` (`element.rs:3561`, `3926`).
  - Content widgets fall back to the first preference when neither side fits (`widgets.rs:79`).
  - Overlay offsets are clamped with `.max(px(0.))` (`context_menu.rs:579-585`).
- **Consequences:**
  - In an editor shorter than the menu, the context menu (about 20 rows of 24px) and the code action menu are cut off.
  - The read-only message on a one-line editor ("Cannot edit in read-only input", `message.rs:139-141`) is placed Above a 1-line viewport and is completely invisible.

---

### Find widget

#### 6. Wheel over the find widget scrolls the editor
- **Severity:** P2
- **Fix size:** XS
- **Monaco:** overlay widgets are children of the overflow guard, not of the scrollable element (`browser/view.ts:284` scrollbar vs `:289` overlay widgets), so wheel over the find widget doesn't scroll the text.
- **Ours:** the widget root (`find_widget.rs:653-678`) has no `on_scroll_wheel`. Wheel over the buttons or status bubbles up to `editor.rs:3660`. The inner inputs do consume wheel (`always_consume_mouse_wheel`).

#### 7. The find view-zone height is computed from a formula, not measured
- **Severity:** P2
- **Fix size:** S
- **Monaco:** the zone height comes from the widget's real height (`findWidget.ts:650-735`, `_getHeight`; `FIND_INPUT_AREA_HEIGHT = 33`).
- **Ours:**
  - `find_widget_height()` (`find.rs:1782-1793`) adds the main editor's `resolved_line_height()` times the `\n` count, a 4px gap and a magic `+10px`.
  - The inputs actually use `single_line_input()` default font and line height, not the main editor's.
  - The widget also uses `options.query`, which differs from the input text when `find_on_type` is off.
  - The same guesswork sizes the input boxes (`input_height`, `find_widget.rs:57-60`).
  - The widget's real size is already measured in `element.rs:1365-1391`. Use that, as `MarkerZone` does.

#### 8. Missing narrow, reduced and collapsed layouts; the widget overflows narrow editors
- **Severity:** P2
- **Fix size:** M
- **Monaco:**
  - `_tryUpdateWidgetWidth` (`findWidget.ts:739-794`) switches between `reduced-find-widget` (hides the match count), `narrow-find-widget` (max 257px) and `collapsed-find-widget` (max 170px; hides prev/next/replace buttons and the toggles) (`findWidget.css` REDUCED/NARROW/COLLAPSED).
  - The initial width is 419px.
- **Ours:**
  - The inputs have a minimum width of 220px (`find_widget.rs:569`). Together with the toggles, status (`min_w 70`) and buttons, the widget is about 430px at minimum.
  - `OverlayPosition::TopRight` puts it at `text.right() - width` without clamping to the left edge (`element.rs:1372-1374`).
  - In a narrow editor it runs past the left edge and is clipped.

#### 9. Layout is not Monaco's
- **Severity:** P2
- **Fix size:** M
- **Monaco** (`findWidget.css`):
  - The toggles sit *inside* the input's right end (`.monaco-findInput .controls`); preserve-case is inside the replace input.
  - The replace toggle is a full-height 18px left column (`.button.toggle`).
  - The close button is absolute top-right.
  - Padding is `0 4px 0 9px`; parts have `margin: 3px 25px 0 17px`; height is 34px, or 66px with replace.
  - The widget has `border-radius: var(--vscode-cornerRadius-large)` and `box-shadow: var(--vscode-shadow-lg)`.
- **Ours:**
  - The toggles are separate items to the right of the input box, and the replace toggle is a small glyph button (`fr-find1-crop.png`; `find_widget.rs:678-768`).
  - The widget has square corners.

#### 10. No placeholders, tooltips or disabled states; colours are hard-coded
- **Severity:** P2
- **Fix size:** S
- **Monaco:**
  - Placeholders "Find" and "Replace" (`findWidget.ts:66,72`).
  - Every button and toggle has a title with its keybinding, e.g. `NLS_PREVIOUS_MATCH_BTN_LABEL + keybinding` and `NLS_TOGGLE_SELECTION_FIND_TITLE` (`findWidget.ts:67-76,1104`).
  - When the match limit is hit, the count has the tooltip `NLS_MATCHES_COUNT_LIMIT_TITLE`.
  - Previous/next, replace and replace-all get `.disabled` (`disabledForeground`) when there are no matches or the editor is read-only.
  - An invalid regex shows an error message box under the input and uses `inputValidation.errorBorder`.
  - Button hover uses `toolbar.hoverBackground`.
- **Ours:**
  - None of this exists. The `CodeEditor` inputs have no `placeholder` set, even though `EditorOptions.placeholder` exists.
  - The error border is `gpui::red().opacity(0.6)` (`find_widget.rs:585`) and shows no message.
  - Button hover is `hsla(0,0,0.5,0.2)` (`find_widget.rs:542`).

#### 11. No animation, no sash visual, limited Tab order, no screen-reader status
- **Severity:** P3
- **Fix size:** S
- **Monaco:**
  - The widget slides in (`transform 200ms linear`).
  - The sash is coloured `editorWidget.resizeBorder`.
  - Tab walks through the inputs, toggles and buttons, which are all focusable.
  - Match changes are announced: "{0} found for '{1}', at …" (`findWidget.ts:457-505`).
- **Ours:**
  - Tab only toggles between the find and replace inputs (`find_widget.rs:491-501`).
  - The sash is invisible (`find_widget.rs:596-603`).
  - Nothing calls `CodeEditor::announce` (`accessibility.rs:183`).

---

### Quick input / quick access / command palette

#### 12. The list can't be scrolled with the mouse, and wheel scrolls the editor behind it
- **Severity:** P1
- **Fix size:** S
- **Monaco:** the quick input list is a virtualized `List` with a scrollbar. Wheel scrolls the list, and the editor is never affected.
- **Ours:**
  - The list is a manual window, `rows[scroll..scroll+visible_rows]` (`quick_access.rs:479-498`).
  - `scroll` only changes through keyboard moves (`keep_active_visible`, `quick_access.rs:292-302`).
  - There is no `on_scroll_wheel` anywhere in the widget and no scrollbar.
  - Each command is a separate list item. The palette shows 12 of them (`fr-f1.png`); mouse users can't reach the rest, and wheeling scrolls the text underneath.
- **Also:** `visible_rows` is computed once when the list opens (`quick_access.rs:604`) and doesn't follow resizes.

#### 13. List rendering differs from Monaco
- **Severity:** P2
- **Fix size:** S
- **Separators:**
  - Monaco puts the separator label right-aligned on the first item of its group, with a top border. It takes no row of its own.
  - Ours renders each separator as its own 22px row (`quick_access.rs:483-495`). "classes (2)", "functions (2)" and "methods (13)" each waste a row (`fr-symk.png`).
- **Match highlight colour:**
  - Ours hard-codes `rgb(0x0097fb)` (`quick_access.rs:407`).
  - That is right only for vs-dark. Per `standaloneQuickInput.css`, vs uses `#0066BF` (and `#9DDDFF` on the focused row), hc-black uses `#F38518`, and hc-light uses `#0F4A85`.
- **Rows:**
  - There is no hover background (`list.hoverBackground`).
  - The active row uses `suggest_selected_background` instead of `quickInputList.focusBackground`.
- **Keybindings:** shown as plain 11px text. Monaco draws them as keycaps (`.monaco-keybinding-key` styles in `standaloneQuickInput.css`).

#### 14. Keybinding labels are formatted wrong (also in the context menu and code action menu)
- **Severity:** P3
- **Fix size:** S
- **Monaco:**
  - Key names are capitalised: "F12", "⇧F12", "⌘F2".
  - macOS modifier order is ⌃⇧⌥⌘: "⇧⌥F", "⇧⌥O".
- **Ours:**
  - `action_keybindings` joins GPUI `Keystroke` `Display` output (`editor_actions.rs:907-918`). That gives "f12", "⇧f12", "⌘f2" and "⌥⇧F" (see `fr-f1.png`, `fr-ctx-c.png`).
  - The code action keybinding is hard-coded as "⇧⌥O" (`code_action.rs:408-422`), which doesn't match the other menus either.

---

### Code action menu and lightbulb

#### 15. The code action menu doesn't scroll properly
- **Severity:** P1
- **Fix size:** S
- **Monaco:**
  - `ActionList` is a `List`: it reveals the focused row and consumes wheel.
  - Its height is the smaller of the items' height and the available space.
- **Ours:**
  - The menu is `.max_h(px(400.)).overflow_y_scroll()` (`code_action.rs:624-627`) with no `on_scroll_wheel` stop, so wheel scrolls the editor while the menu stays anchored.
  - There is no `ScrollHandle`: `move_focus` (`code_action.rs:448-465`) moves the highlight off-screen once there are more than about 15 rows.
  - The "Show Disabled" / documentation action bar is inside the scroll container, so it scrolls away (`code_action.rs:640-662`).

#### 16. Menu sizes are hard-coded guesses and styling is off
- **Severity:** P2
- **Fix size:** S
- **Monaco** (action widget CSS, not vendored):
  - Rows are a fixed 24px; the list width is measured from the widest row; labels are single-line with an ellipsis.
  - The focused row uses `list.activeSelection*`/`quickInputList.focus*`.
  - The widget has a shadow and rounded corners.
- **Ours:**
  - `min_w 200 / max_w 500 / max_h 400` (`code_action.rs:624-626`).
  - Rows are `py(2)` plus the editor font size, and labels wrap.
  - The focused row uses `peek_view_result_selection_background` (`code_action.rs:584`).
  - There is no shadow (`fr-ca-c.png`).

#### 17. The lightbulb has no hover title and is drawn as a primitive
- **Severity:** P2
- **Fix size:** S
- **Monaco:**
  - The title is "Show Code Actions (⌘.)", "Show Code Actions. Preferred Quick Fix Available (…)" or "Run: {title}" for autoRun (`lightBulbWidget.ts:89-100,448`).
  - It is a codicon, with AI/sparkle variants (`lightBulbWidget.ts:27-31`, `lightBulbWidget.css`).
- **Ours:**
  - It is painted in `element.rs:4169-4199` as a circle and a rectangle.
  - There is no tooltip, no hover cursor feedback and no sparkle variants.

---

### Rename

#### 18. The suggestion list doesn't scroll properly
- **Severity:** P1
- **Fix size:** S
- **Monaco:** `RenameCandidateListView` is a `List`. It reveals the focused candidate and consumes wheel; its height is `min(n*rowHeight, availableHeight, 7*rowHeight)` (`renameWidget.ts:838-843`).
- **Ours:**
  - The list is `.max_h(row_height*7).overflow_y_scroll()` with no wheel stop (`rename.rs:288-295`), so wheel over the suggestions scrolls the editor.
  - There is no `ScrollHandle`. Down past the 7th suggestion focuses a row that can't be seen, while its text still appears in the input (`rename.rs:204-220`).

#### 19. Sizes are guessed and some styling is missing
- **Severity:** P3
- **Fix size:** S
- **Width:** ours is fixed at `w(220)` (`rename.rs:355-356`). Monaco sets `input.size = max(1.1 × nameLength, 20)` characters with a 200px minimum (`renameWidget.ts:314,442`), so long names get a wider box.
- **Input height:** ours is `font_size*1.6` (`rename.rs:378`); Monaco's is the line height.
- **Candidate row:** ours is `font_size*1.4+4` (`rename.rs:258`); Monaco's is `lineHeight` plus padding (`renameWidget.ts:1024-1025`).
- **Placement:** ours always prefers Below then Above. Monaco picks Below only when there is room for about 6 candidates (`renameWidget.ts:299-302`), and it doesn't clamp the list to the space available.
- **Shadow:** Monaco has `0 0 8px 2px widgetShadow` (`renameWidget.ts:249`); ours has none.
- **Colours:** the text uses `widget_foreground` instead of `inputForeground`, and the focused row uses `peek_view_result_selection_background` instead of `quickInputList.focusBackground` (`renameWidget.ts:720-723`).
- **Behaviour:**
  - There is no hover background on rows.
  - Ours doesn't cancel when the box scrolls out of view. Monaco does (`afterRender(null)` → `cancelInput`, `renameWidget.ts:321-326`).

---

### Context menu

#### 20. Fixed width, no max height or scrolling, no submenu flip, and wheel falls through
- **Severity:** P2
- **Fix size:** M
- **Monaco:** the menu sizes to its content, caps its height at the window and scrolls, and flips submenus at the window edge.
- **Width:**
  - Ours is fixed at `MENU_WIDTH = 260` (`context_menu.rs:31-33,286`).
  - Labels are `whitespace_nowrap` with no overflow handling, so a long host-action label (from `add_action`) draws past the menu's border.
- **Height:** the menu is never scrollable, and a tall menu is clipped (see 5).
- **Submenu:**
  - It always opens to the right at `MENU_WIDTH - 4` (`context_menu.rs:396-405`), so near the right edge it goes out of the editor.
  - It opens instantly on hover, without Monaco's open delay.
- **Wheel:** there is no `on_scroll_wheel`, so wheel over the menu scrolls the text underneath while the menu stays put.

---

### Message widget

#### 21. Missing pointer arrow, gap, colours, fade and reveal
- **Severity:** P2
- **Fix size:** S
- **Monaco** (`messageController.css`, `messageController.ts`):
  - An 8px `.anchor` triangle points at the position.
  - There is 8px padding between the box and the line, plus `margin-left: -6px`.
  - The `.message` box uses `editorHoverWidget-background`/`-foreground` with an `inputValidation-infoBorder` border.
  - It fades in over 150ms and out over 100ms.
  - `revealLinesInCenterIfOutsideViewport` brings the line into view (`messageController.ts:168`).
  - The text is announced with `alert(...)` (`messageController.ts:67`).
  - It stays open on blur while the mouse is over the message (`messageController.ts:84-91`).
- **Ours** (`message.rs:40-52`, `fr-ro-c.png`):
  - A plain box sits flush against the line, with no arrow.
  - The background is `message_background` = `#063b49` (inputValidation.infoBackground, `theme.rs:801`).
  - There is no fade and no reveal.
  - It closes on any blur (`editor.rs:490`) and is not announced.

---

### Color picker

#### 22. Standalone picker chrome
- **Severity:** P3
- **Fix size:** S
- **Monaco** (`colorPicker.css`):
  - `.standalone-colorpicker` uses `editorHoverWidget-background`, `-foreground` and `-border`; ours uses `widget_background`/`widget_border` (`color_picker.rs:624-626`).
  - The header has a `codicon-color-mode` icon and a title tooltip (`colorPickerHeader.ts:35-40`); ours has neither.
  - The close button is a codicon with a `toolbar.hoverBackground` hover; ours is a `×` with no hover (`color_picker.rs:447-460`).
  - The insert button has `button.hoverBackground`; ours has no hover.
  - The strips use `cursor: grab` and `grabbing`; ours set no cursor.
- Sizes (24px header, 150/122px strips, 25px strips, 302px total) match.

---

### Summary

**6 P1 (5 is P1 only in short editors):**
- 1: right/middle click falls through and closes widgets
- 2: editor providers run inside widget inputs (ghost text and a clipped suggest widget)
- 5: widgets clipped to the editor (P1 only in short editors)
- 12: quick input list not mouse-scrollable; wheel scrolls the editor
- 15: code action menu wheel and keyboard scrolling
- 18: rename suggestion list wheel and keyboard scrolling

**12 P2:**
- 3, 4, 6, 7, 8, 9, 10, 13, 16, 17, 20, 21

**4 P3:**
- 11, 14, 19, 22


## C. Zones, peek, code lens, inlay hints, sticky scroll, gutter, minimap, scrollbars


Scope:
- Our code: `crates/gpui-monaco/src/` (cited as `file.rs:line`).
- Monaco: `vendor/vscode/src/vs/editor/` (cited as `contrib/...` or `browser/viewParts/...`).
- `vs/base/browser/ui/scrollbar/` is not vendored. Claims about it are marked "(unvendored)".

Screenshots are in this directory. Each was taken with `pg-shot`; no mouse input was simulated.

| Screenshot | Shows |
|---|---|
| `peek.png` | Shift+F12 on `Edit` |
| `demo.png`, `scrolled_demo*.png` | Demo widgets |
| `zoom2.png` | Font zoom ×2, with stale code lenses |
| `minimap/long_end.png` | Minimap at the end of `editor.rs` |
| `minimap/sel_crop.png` | Cmd+A, with no minimap selection |
| `gutter/s1.png`, `s2.png` | Gutter and sticky scroll |
| `scroll/demo.png` | Scrollbars and overview ruler |

Severity: **P1** is visibly broken or wrong behaviour. **P2** is a missing Monaco sub-feature or a notable fidelity gap. **P3** is minor polish.

Fix size: **XS** < 1h, **S** a few hours, **M** about a day, **L** several days.

### A. Embedded views: view zones, content widgets, overlay widgets (`widgets.rs`, `element.rs`)

1. **Zone and widget hitboxes aren't clipped, so hidden zone content takes clicks over the toolbar, the sticky scroll and the scrollbars** — **P1**, fix **S**
   - **Monaco:**
     - View zones live in `linesContent` inside the overflow guard (`browser/view.ts:279`), and the zone widget body is an overlay widget in the same guard. Anything scrolled out of the editor is clipped and can't be clicked.
     - The sticky scroll widget and the scrollbars sit above the zones, so they get the clicks first.
   - **Ours:**
     - Zones, content widgets, glyph widgets and overlay widgets are laid out and `prepaint_at` in `EditorElement::prepaint` (`element.rs:1295-1391`) with no content mask. GPUI records each hitbox with the content mask active during prepaint, and hit-tests against `bounds ∩ content_mask` (gpui `window.rs:1118-1131`, `insert_hitbox` 5295). So the hitboxes cover the zone's whole unclipped rectangle, even though painting is clipped (`element.rs:3755`, `3814-3833`).
     - The editor root has no `overflow_hidden`, and none of these elements block the mouse.
   - **Effects:**
     - Scroll a peek view (or any host zone) so its top half is above the editor. Clicking the playground toolbar over the hidden part also fires the hidden peek row's `on_click`, or the hidden preview editor's mouse-down, which takes focus.
     - Inside the editor, `CodeEditor::on_mouse_down` checks `widget_bounds` (`editor.rs:3014-3022`) *before* the sticky lines (`editor.rs:3056-3063`) and the scrollbars (`3064+`). A click on a sticky line or on the horizontal scrollbar over a zone, such as a code lens or the peek, is swallowed and goes to the zone underneath.
   - **Fix:** prepaint each embedded element inside `with_content_mask(text area or editor bounds)`. Give sticky, scrollbar and minimap hit areas precedence over `widget_bounds`, or subtract them from zone bounds. Paint and occlude the sticky widget above zones.

2. **Content, glyph and overlay widgets paint unclipped and on top of the sticky scroll, scrollbars-adjacent chrome and outside the editor** — **P2**, fix **S**
   - **Monaco:**
     - Content widgets are inside `linesContent`, under the sticky scroll overlay, the minimap and the scrollbars. Unless `allowEditorOverflow` is set, the overflow guard clips them.
     - Glyph margin widgets are in the margin, under sticky scroll.
   - **Ours:** `widget_elements` are painted after the sticky scroll with no content mask (`element.rs:3926-3928`). A content widget (for example `Below` a line under the sticky header) or a glyph widget (a breakpoint) draws over the sticky lines, and a content widget can spill outside the editor. Its `y` can go off-editor when no preference fits (`widgets.rs:63-88`).
   - **Fix:** paint content and glyph widgets inside the text/gutter mask, before the sticky scroll. Keep overlay widgets last.

3. **View zones don't scroll horizontally and have no margin node** — **P2**, fix **M**
   - **Monaco:**
     - Zone DOM nodes are in `linesContent`, so they scroll horizontally with the text.
     - The zones layer is `max(scrollWidth, contentWidth)` wide (`viewParts/viewZones/viewZones.ts:404-407`).
     - `IViewZone.marginDomNode` renders a separate node in the gutter (`viewZones.ts:255-260`, `396-400`).
     - The API also has `afterColumn`/`afterColumnAffinity`, `ordinal`, `showInHiddenAreas`, `suppressMouseDown`, `minWidthInPx`, `heightInLines` and the `onDomNodeTop`/`onComputedHeight` callbacks (`viewZones.ts:145-195`, `314`, `320-335`).
   - **Ours:**
     - Non-full-width zones are pinned at `text_bounds.left()` with `text_width` (`element.rs:3377-3392`), and ignore `scroll_left`.
     - `ViewZone` has only `after_line`, `height` and `view` (`widgets.rs:24-33`).
     - Clicking an ordinary zone never moves the cursor (`editor.rs:3014-3022`). In Monaco, a zone without `suppressMouseDown` places the cursor at the zone's line.
   - **Fix:** offset non-full-width zones by `-scroll_left`, widen them to the scroll width, and add the missing `IViewZone` fields, starting with `margin_view` and `suppress_mouse_down`.

4. **Content widget API and placement gaps** — **P3**, fix **S**
   - **Monaco** (`viewParts/contentWidgets/contentWidgets.ts`):
     - `secondaryPosition`, `positionAffinity`, `allowEditorOverflow`, `suppressMouseDown`, `beforeRender`/`afterRender`.
     - `maxWidth` is capped at the layout width.
     - The above/below fit check is against the viewport, or the window when overflow is allowed.
   - **Ours:** `ContentWidget` has `position`, `preference` and `view` (`widgets.rs:47-56`). It's measured at min-content with no max width (`element.rs:1324-1328`), so a wide widget is clamped left but can still overflow on the right. `x` is clamped to `bounds.left()`, the gutter, not to the content left.

5. **Overlay widget corners are off by Monaco's gap, and `stackOrdinal`, `minContentWidthInPx` and manual (null) positions are missing** — **P3**, fix **XS**
   - **Monaco** (`overlayWidgets.ts:183-196`):
     - TOP/BOTTOM_RIGHT use `right = 2*verticalScrollbarWidth + minimapWidth`.
     - BOTTOM_RIGHT uses `top = editorHeight - h - 2*horizontalScrollbarHeight`.
     - Widgets with a `stackOrdinal` stack.
   - **Ours** (`element.rs:1372-1385`): `text.right() - w` (flush against the minimap; see `demo.png`), and `bounds.bottom() - h`, which overlaps the horizontal scrollbar. No stacking.

### B. Peek view (`peek.rs`)

6. **The results list doesn't stop the wheel, so scrolling it also scrolls the editor** — **P1**, fix **XS**
   - **Monaco:** the references tree is a `WorkbenchAsyncDataTree` whose `ScrollableElement` consumes the wheel. The editor behind the peek doesn't move.
   - **Ours:** `#peek-results` is a GPUI `overflow_y_scroll()` div (`peek.rs:760-768`) with no `on_scroll_wheel(|_,_,cx| cx.stop_propagation())`. The wheel bubbles to `CodeEditor::on_scroll_wheel` (`editor.rs:3660-3733`, bound at `editor.rs:5078`). This is the same bug the `MarkerZone` fix addressed (`markers.rs:609`).
   - With many results, the list and the main editor scroll together.

7. **Results list has no scrollbar, isn't virtualized, and rows are 22px** — **P2**, fix **M**
   - **Monaco:**
     - The tree is a virtualized list with a 23px row (`gotoSymbol/browser/peek/referencesTree.ts:65-68`, `referencesWidget.css` `.ref-tree { line-height: 23px }`).
     - It has a standard scrollbar with a top shadow and a slider.
   - **Ours:**
     - Every row is built on every render (`peek.rs:645-650`, `render_row` reads the line for each reference), so thousands of LSP references re-layout each frame.
     - The rows are `px(22.)` (`peek.rs:520`).
     - There's no visible scrollbar. In `peek.png`, 17 references overflow the list with nothing showing that it scrolls.
   - **Fix:** `uniform_list` with a scrollbar, 23px rows.

8. **One file still gets a file row; Monaco shows the references directly** — **P2**, fix **XS**
   - **Monaco:** `this._tree.setInput(this._model.groups.length === 1 ? this._model.groups[0] : this._model)` (`referencesWidget.ts:540`). With a single file, the tree's roots are the references themselves, so there's no file header.
   - **Ours:** `rows()` always pushes `Row::File(g)` (`peek.rs:306-315`). See `peek.png`, where "sample.rs /Users/… 17" occupies the first row.

9. **Peek header layout and frame don't match `peekViewWidget.css`/`referencesWidget.css`** — **P2**, fix **S**
   - **Monaco:**
     - The reference zone's container border is 1px top and bottom (`referencesWidget.css:7-10`). The body has a 1px `border-top` in the frame color between header and body (`peekViewWidget.css` `.body{border-top:1px solid}`; `peekView.ts:163-165`, `269`).
     - The title is 13px, with `margin-left: 20px`.
     - The dirname is `0.9em` with a `0.5em` left margin.
     - The meta has a `-` separator (`.meta:not(:empty)::before { content:'-'; padding: 0 .3em }`).
     - The close button is a codicon in an action bar with hover and a tooltip (`peekView.ts:198-205`).
     - The title is clickable and opens the focused reference (`supportOnTitleClick`, `referencesWidget.ts:276`, `318-326`).
     - `.peekview-widget` has `box-shadow: var(--vscode-shadow-lg)`.
   - **Ours:**
     - 2px top and bottom borders (`peek.rs:695-697`), and no header/body separator.
     - `px(10.)` padding with an 8px gap, and the same size for the dirname (`peek.rs:699-721`).
     - The meta has no dash.
     - A text "✕" with no hover or tooltip (`peek.rs:722-735`).
     - Clicking the title does nothing.
     - No shadow.

10. **Result list styling: count badge, file icon, selection focus states** — **P3**, fix **S**
    - **Monaco:**
      - The file row has a resource icon and label, and a `CountBadge` (`badge.background`/`badge.foreground`, `margin-right: 12px`).
      - The selected row uses `peekViewResult.selectionBackground` only while the list has focus (`.monaco-list:focus .selected`). Otherwise it uses the list's inactive selection colors.
      - Match highlights also set `color: peekViewResult.fileForeground` (`referencesWidget.css:44-51`, `57-60`).
      - In high contrast, file rows are bold and matches get a dotted border.
    - **Ours:**
      - The badge uses `peek_view_result_selection_background` (`peek.rs:547-553`), there's no icon, the selected background shows regardless of focus (`peek.rs:590-592`), and the match foreground is unchanged.
      - There's no `peekViewEditor.matchHighlightBorder` (used by HC themes, `referencesWidget.css:62-66`) in `theme.rs`.

11. **Missing list keyboard behaviour** — **P3**, fix **S**
    - **Monaco:** the workbench tree has PageUp/PageDown, type-to-navigate (`keyboardNavigationLabelProvider`), focus on file rows (Enter or Space toggles), and Left on a reference collapses its parent and focuses it. Open-to-side is Ctrl/Cmd+Enter, or a Ctrl/Cmd/Alt double-click in the preview.
    - **Ours:**
      - Only Up, Down, Home, End, Enter, Left, Right, Escape, F4 and F12 (`actions.rs:1648-1662`).
      - Left collapses the group but leaves the hidden reference selected (`peek.rs:791-795`). The next Down then jumps to the first visible reference in the whole list (`move_in_list`'s `None` arm, `peek.rs:461-465`).

12. **Preview editor options and arrow details** — **P3**, fix **XS**
    - **Monaco** (`referencesWidget.ts:347-363`):
      - The preview is created with `scrollBeyondLastLine: false`, `overviewRulerLanes: 2`, `verticalScrollbarSize: 14`, `alwaysConsumeMouseWheel: true`, and minimap off.
      - Switching to another result in the same model scrolls smoothly (`referencesWidget.ts:598-603`).
      - The arrow moves to column 2 when the anchor is at column 1 (`zoneWidget.ts:158-162`).
      - The height sash applies `floor`/`ceil` of the drag delta relative to the start (`zoneWidget.ts:536-545`). It shows the sash hover border.
      - `peekViewEditorStickyScroll*` colors exist because sticky scroll stays on in the preview.
    - **Ours:**
      - The preview inherits the parent's `scroll_beyond_last_line`, lanes and wheel options, and disables sticky scroll and the glyph margin (`peek.rs:271-274`).
      - It always reveals with `RevealType::Center` immediately (`peek.rs:411`).
      - The arrow sits at the raw cursor x (`peek.rs:632-641`).
      - The height comes from the absolute pointer y (`peek.rs:847-856`).
      - The sashes are invisible 4px strips (`peek.rs:652-657`, `816-823`).

### C. Code lens (`code_lens.rs`)

13. **Unresolved lenses render "no commands"** — **P2**, fix **XS**
    - **Monaco:**
      - `CodeLensWidget` only creates the content widget when some lens already has a command (`codelensWidget.ts:213-238`).
      - `withCommands` shows "no commands" only when every *symbol* is missing (`codelensWidget.ts:87-118`), which happens after a failed resolve, not before resolving.
      - Unresolved lenses therefore show an empty zone.
    - **Ours:** `if self.lenses.iter().all(Option::is_none) && count > 0 { "no commands" }` (`code_lens.rs:106-108`). Every line whose lenses need `resolve_code_lens`, like rust-analyzer's reference counts, flashes "no commands" until it resolves. Off-screen lines show it permanently, since only the visible lines are resolved.

14. **Lens views keep stale font, size, height and colors after theme, font or zoom changes** — **P2**, fix **S**
    - **Monaco:** `onDidChangeConfiguration` on `fontInfo`/`codeLensFontSize`/`codeLensFontFamily` re-styles the lenses through CSS variables and re-lays out every zone height (`codelensController.ts:60-66`, `84-112`). Colors are CSS variables, so a theme change applies immediately.
    - **Ours:**
      - `font_size`, `line_height`, `color`, `hover` and `indent` are copied into each `CodeLensLine` and the zone gets a fixed `ZoneHeight::Pixels` (`code_lens.rs:236-264`).
      - They're rebuilt only when the text version or epoch changes (`code_lens.rs:153-163`, called from `element.rs:1873`). `update_options` and `set_theme` don't touch them (`editor.rs:818-876`).
      - Screenshot `zoom2.png`: after Increase Editor Font Size ×2, the code text grew but "1 reference" and "19 references" kept their old size and zone height.

15. **Lens text is placed with `em_width × byte count` in a pinned zone, not as a content widget at the first non-whitespace column** — **P2**, fix **S**
    - **Monaco:**
      - The lens is a content widget at `getLineFirstNonWhitespaceColumn(line)` with preference ABOVE (`codelensWidget.ts:140-146`).
      - Its x is the rendered column offset, which handles tabs, wide characters and proportional fonts.
      - It scrolls horizontally with the text.
    - **Ours:**
      - `indent = em_width * (line.len() - line.trim_start().len())` (`code_lens.rs:240-246`). Tab-indented code puts lenses 1 em per tab, not `tab_size` columns.
      - The indent is frozen at creation, and the zone ignores `scroll_left` (see #3).

16. **Lens titles: no `$(icon)` rendering, no tooltip, no fade-in, no font features** — **P3**, fix **S**
    - **Monaco:**
      - `renderLabelWithIcons(title)` turns `$(name)` into codicons.
      - `title: command.tooltip`.
      - A 0.1s `fadein` when lenses first appear.
      - `font-feature-settings` from the editor font (`codelensWidget.ts:96-104`, `codelensWidget.css:12-19`, `61-68`).
    - **Ours:** the raw title text (`code_lens.rs:80-102`), with no tooltip, animation, or ligature settings.

### D. Inlay hints (`inlay_hints.rs`)

17. **Padding is approximated with thin-space characters, and there's no border radius** — **P2**, fix **S**
    - **Monaco** (`inlayHintsController.ts:565-575`, `596-700`, `747-765`):
      - `paddingLeft`/`paddingRight` insert an inline-block exactly `fontSize/3 | 0` px wide.
      - The `padding` option uses CSS padding `1px max(1, fontSize/4)px` and `border-radius: fontSize/4 px`, split across first and last parts.
      - When the hint font differs (`!isUniform`), `vertical-align: middle`.
    - **Ours** (`inlay_hints.rs:383-440`):
      - `\u{2009}` injected text (comment: "a thin space is close").
      - Its width is whatever the font gives U+2009. Monospace fonts often give it a full cell, so padded hints look about 2–3× wider apart than in Monaco.
      - The `padding` option adds thin spaces inside the label background and has square corners.
      - Smaller hint fonts are baseline-aligned.
    - **Fix:** a fixed-width spacer (injected `width_in_em`/px), plus rounded background quads.

18. **Inlay hint behaviours listed as not ported** — **P2**, fix **M**
    - **Monaco:**
      - The label-part context menu.
      - Hovers that come from a part's `location` (go-to-definition hover).
      - Cursor-line length freezing while typing (`inlayHintsController.ts:480-520`), which keeps the cursor from jumping as hints update.
    - **Ours:** documented as missing (`inlay_hints.rs:8-9`). The last one is visible when typing on lines with hints.

### E. Placeholder text

No gaps found. `element.rs:3441-3471` matches `placeholderTextContribution.ts`: per-line ellipsis, editor font, placeholder color, shown only when the model is empty.

### F. Sticky scroll (`sticky_model.rs`, `element.rs`, `editor.rs`)

19. **The last sticky line is drawn over the line above it while it is pushed out** — **P1**, fix **S**
    - **Monaco:** each sticky line has an opaque background (`stickyScroll.css:47-53`). The last line has z-index 0, under the others at 1 (`stickyScrollWidget.ts:319`, `326`), so it slides *under* its parent.
    - **Ours:** the last line's top is `min(ix*lh, end_bottom - lh)` (`element.rs:2626-2631`), so it can move almost a full line into the previous slot. The paint loop (`element.rs:3855-3906`) fills the area once and draws the lines in order, so the two headers' text overlaps during partial-line scrolls (trackpad, or code-lens zone offsets).

20. **Sticky scroll needs `folding: true`; indentation model ignores `offSide`** — **P2**, fix **M**
    - **Monaco:** the sticky model providers are independent of the `folding` option: outline, then the folding provider, then indentation (`stickyScrollModelProvider.ts:60-72`, `369-403`).
    - **Ours:**
      - Falls back to `self.folds`, which is only computed when `options.folding` is on (`editor.rs:2614`). With folding off, sticky scroll is empty unless the model is indentation.
      - The indentation path calls `compute_indent_ranges(.., false, None)`, which ignores the language's `offSide` setting (wrong for Python).

21. **Clicking a sticky line's fold chevron moves the cursor** — **P2**, fix **XS**
    - **Monaco:** `_toggleFoldingRegionForLine` toggles the fold and sets `scrollTop = top(line) - lh*index + 1` without touching the selection (`stickyScrollController.ts:435-451`).
    - **Ours:** `sticky_click` → `reveal_sticky_row` calls `set_selections(row, 0)` (`sticky_model.rs`).

22. **No 25% height cap** — **P2**, fix **XS**
    - **Monaco:** `maxStickyLines = round(height/lh*0.25)`, capped by `maxLineCount` (`stickyScrollController.ts:546`).
    - **Ours:** only `max_line_count` (`element.rs:2593`), so small editors, including the peek preview, can be mostly covered.

23. **Cmd+click to go to definition from sticky lines, and the sticky context menu, are missing** — **P2**, fix **M**
    - **Monaco:** `stickyScrollController.ts:335-415`.
    - **Ours:** `sticky_click` returns early when Cmd, Ctrl or Alt is held.

24. **Fold chevrons in sticky lines with `showFoldingControls: never`** — **P2**, fix **XS**
    - **Monaco:** with `never`, sticky lines get no icon (`stickyScrollWidget.ts:539`). With `mouseover`, icons show only while hovering the sticky gutter.
    - **Ours:** collapsed regions still get a chevron (`element.rs:3911-3916`), and `mouseover` uses the whole-gutter hover flag.

25. **Sticky lines drop inline decorations and injected text, and clicks land at column 1** — **P3**, fix **S**
    - **Monaco:**
      - Sticky lines are rendered with the line's inline decorations and injected text, such as inlay hints (`stickyScrollWidget.ts:458`).
      - A click puts the cursor at the clicked column and fires on click (`stickyScrollController.ts:310-321`).
    - **Ours:**
      - Token colors only (`element.rs:2647-2657`), so a sticky line with inlay hints doesn't line up with the real line.
      - Always column 0, on mouse-down.

26. **Sticky shadow, border and gutter colors** — **P3**, fix **S**
    - **Monaco** (`stickyScroll.css:6-19`):
      - `box-shadow: editorStickyScroll.shadow 0 4px 2px -2px`
      - an `editorStickyScroll.border` bottom border
      - `editorStickyScrollGutter.background`
    - **Ours:** a 1px `scrollbar_shadow` line at 0.6 (`element.rs:3918-3922`). None of these keys is parsed; only `editorStickyScroll.background` is (`theme.rs:1778`).

27. **Sticky keyboard focus is a stale index** — **P3**, fix **XS**
    - **Monaco:** after a scroll, it keeps the focused line if it is still shown, else focuses the last line (`stickyScrollController.ts:575-600`).
    - **Ours:** `sticky.focused` is never re-validated. Enter can do nothing while Up, Down and Enter are still captured.

### G. Gutter: line numbers, glyph margin, folding controls

28. **`showFoldingControls: never` still reserves the folding column and draws collapsed chevrons** — **P2**, fix **XS**
    - **Monaco:** no folding width when the option is `never` (`common/config/editorOptions.ts:2953-2958`), and no icons, not even on collapsed regions (`foldingDecorations.ts`, `NO_CONTROLS_*`).
    - **Ours:** the width is reserved (`element.rs:1453`) and chevrons are drawn on collapsed lines (`element.rs:3627`).

29. **Folded-region placeholder is a crude approximation** — **P2**, fix **S**
    - **Monaco:** a plain "⋯" after the line text in `editor.foldPlaceholderForeground` (#808080), with no box (`folding.css` `.inline-folded:after`, `foldingDecorations.ts:18`).
    - **Ours:** `paint_fold_placeholder` (`element.rs:4201-4235`) draws a rounded box in the selection color with dots in the text color. The theme key isn't parsed.

30. **Glyph margin has no lanes, a fixed width, and only paints the start row** — **P2**, fix **M**
    - **Monaco:**
      - Width is `lineHeight * laneCount` (`editorOptions.ts:2967-2970`).
      - `glyphMargin.position` Left, Center or Right lanes, with the highest zIndex winning per lane (`browser/viewParts/glyphMargin/glyphMargin.ts:259-390`).
      - Glyphs are drawn on every line of the range (`:270-279`).
      - Glyph widgets take a range plus a lane and zIndex (`:283-300`).
    - **Ours:**
      - Fixed 18px (`element.rs:24`, `1448`) and one lane (`element.rs:2272`).
      - Start row only, and overlaps are drawn on top of each other (`element.rs:2829-2843`).
      - `GlyphMarginWidget` takes a row only (`widgets.rs:112-120`).

31. **Folding control size, position, color, fade and tooltip** — **P3**, fix **S**
    - **Monaco:**
      - A 16px column inside the decorations area, centered, with a 2px margin (`folding.css`).
      - `editorGutter.foldingControlForeground`.
      - A 0.5s opacity transition.
      - "Click to expand/collapse the range." tooltips (`foldingDecorations.ts`).
    - **Ours:** `min(lh, 18px)`, placed after `line_decorations_width`, drawn in the line-number color (`element.rs:3628`, `3915`), with no fade and no tooltip.

32. **Line numbers: active, relative and dimmed details** — **P3**, fix **XS**
    - **Monaco:**
      - Only the primary cursor's line is "active" (`browser/viewParts/lineNumbers/lineNumbers.ts`, `selections[0]`).
      - In relative mode, the current line's number is left-aligned (`.relative-current-line-number`, `lineNumbers.ts:124`).
      - The final-newline number uses `editorLineNumber.dimmedForeground` when the theme sets it.
      - The width is `round(digits * maxDigitWidth)` (`editorOptions.ts:2964`), and `lineDecorationsWidth` accepts `"Nch"`.
    - **Ours:**
      - Every selection head is active (`element.rs:3305`).
      - Right-aligned in relative mode.
      - Always `opacity(0.4)`, which also dims the active color, and the `row > 0` guard skips line 1 (`element.rs:3308-3341`).
      - The width is measured with "m" (`element.rs:131`, `1443-1446`).
      - `Pixels` only (`options.rs:1550`).

### H. Current line highlight

33. **`renderLineHighlight: gutter` paints the whole line** — **P1**, fix **XS**
    - **Monaco:** `gutter` paints only the margin box, `contentLeft` wide (`browser/viewParts/currentLineHighlight/currentLineHighlight.ts:198-209`).
    - **Ours:** when `highlight_gutter` is set, the box runs from `bounds.left()` to `text_bounds.right()` (`element.rs:3568-3579`), so `Gutter` looks the same as `All`.

34. **Only the primary cursor's line and wrapped segment are highlighted, and the gutter part disappears with a selection** — **P1**, fix **S**
    - **Monaco:**
      - Every distinct cursor line is highlighted.
      - With word wrap, every view line of the model line is filled, and the cursor's segment also gets the border.
      - The margin highlight ignores the selection; only the content part needs an empty selection (`currentLineHighlight.ts:53-63`, `126-175`, `184-189`).
    - **Ours:** a single `cursor_view_lines[0]` box, gated by `!has_selection` for both parts (`element.rs:3561-3592`). The other cursors' lines are already collected at `element.rs:2999-3003`.

35. **Line highlight border and inactive color** — **P3**, fix **XS**
    - **Monaco:**
      - The border is 1px in high-contrast themes.
      - In `all` mode, margin and content are separate boxes, and the margin box has no right border.
      - The content box extends under the scrollbar (`max(scrollWidth, contentWidth)`).
      - `editor.inactiveLineHighlightBackground` applies when unfocused (`currentLineHighlight.ts:186`, `211-231`).
    - **Ours:** one box with `Edges::all(px(2.))` that stops at `text_bounds.right()` (`element.rs:3583-3591`). The inactive color isn't supported.

### I. Minimap (`element.rs`, `editor.rs`)

36. **Minimap scroll drifts from the viewport; near the end the slider and visible lines fall off the bottom** — **P1**, fix **M**
    - **Monaco:** `MinimapLayout.create` (`browser/viewParts/minimap/minimap.ts:280-416`):
      - slider top = `scrollTop * computedSliderRatio`, clamped to `minimapHeight - sliderHeight`
      - the start line is derived from the slider
      - accounts for padding lines, partial lines and `sliderNeeded`
    - **Ours:** `minimap_scroll = (total - viewport) * scroll_top / max_scroll_top`, with the slider at `mline * first_visible - minimap_scroll` (`element.rs:2412-2421`, `2505-2514`).
    - With `scrollBeyondLastLine`, the slider runs to `viewport - mline`. `minimap/long_end.png`, taken after Cmd+End, shows dense code at the bottom instead of the end of the file.
    - **Fix:** port `MinimapLayout` verbatim.

37. **Slider drag jumps, then runs away from the pointer** — **P1**, fix **S**
    - **Monaco:** relative drag, `initialScrollTop + delta / computedSliderRatio` (`minimap.ts:1322-1362`).
    - **Ours:** an absolute `(y - grab - top) * drag_ratio` (`editor.rs:3372-3378`). The ratio doesn't match how the slider is drawn (`element.rs:2569`, `2580`).
    - Grabbing the slider without moving the mouse jumps the scroll position. For example, line 10 jumps to about line 16.5 with N=100.

38. **Click-to-reveal ignores view zones, padding and centering, and always starts a drag** — **P2**, fix **S**
    - **Monaco:**
      - In proportional mode, `revealRange(line, Center, Smooth)` (`minimap.ts:1129-1140`, `1262-1271`), which is zone-aware.
      - In fill or fit mode, a click starts a slider drag (`minimap.ts:1245-1253`).
    - **Ours:** `scroll_top = line_height * line - visible / 2` (`editor.rs:3041-3049`). It ignores `zones.line_y`, which code lenses shift, and `padding.top`, is immediate, and always sets `minimap_drag`.

39. **No selection or cursor-line highlight in the minimap** — **P2**, fix **M**
    - **Monaco:** `minimap.selectionHighlight` draws a band at 0.5 alpha plus a column span at full alpha for each selection (`minimap.ts:1544-1694`).
    - **Ours:** `minimap_image_for` paints text only (`element.rs:4486-4610`). See `minimap/sel_crop.png`.

40. **No minimap decorations: find, occurrences, markers, diff gutter, custom** — **P2**, fix **L**
    - **Monaco:**
      - Decorations carry `options.minimap { color, position: Inline | Gutter, sectionHeaderStyle, sectionHeaderText }` (`minimap.ts:1072-1123`, `1620-1745`).
      - The theme has `minimap.findMatchHighlight`, `selectionOccurrenceHighlight`, `errorHighlight`, `warningHighlight`, `infoHighlight` and `minimapGutter.*`.
    - **Ours:**
      - `DecorationOptions` has no `minimap` field (`decorations.rs:95` has only `overview_ruler`).
      - None of those theme keys is parsed (`theme.rs:1770-1777`).
      - Section headers come only from the built-in detector (`element.rs:2524`).

41. **Character rendering is a crude approximation** — **P2**, fix **M**
    - **Monaco:**
      - `MinimapCharRenderer.renderChar` blends the prebaked per-pixel glyphs (`minimapCharRenderer.ts:29-80`, `minimapPreBaked.ts`).
      - Glyphs are softened ×0.8, or ×50/60 on light themes.
      - Each pixel is `bg + (fg-bg)*glyph*foregroundOpacity`.
      - `minimap.foregroundOpacity` and the canvas `opacity: .9` apply (`minimap.css:65-67`).
      - Glyph height stays at 2 or 3×scale and is centered when lines are taller (`minimap.ts:1985-1987`, `2195`, `2211`).
      - There's an 8px left gutter (`minimap.ts:2185`).
      - Full-width characters take two cells.
    - **Ours:**
      - A hand-made two-band darkness table (`element.rs:4472-4484`, `4586-4602`).
      - Hard-coded alphas of 0.75 and 0.6.
      - No light-theme variant.
      - In fill or fit mode, the bands stretch to the full line height, so tall lines become smears (`element.rs:4588-4593`).
      - Text starts at x=0.
      - Full-width characters take one cell.

42. **Slider styling and minimap shadow** — **P2**, fix **S**
    - **Monaco:**
      - `minimapSlider.hoverBackground` and `.activeBackground`.
      - With `mouseover`, the slider stays visible while dragging and fades over 100ms.
      - It's hidden when not needed.
      - A 6px `minimap-shadow-visible` on the left edge when the content can scroll right (`minimap.css:7-43`; `minimap.ts:1327`, `1360`, `1505-1527`).
    - **Ours:**
      - Always `colors.minimap_slider` (`element.rs:4026-4027`). `minimap_slider_hover` is parsed but unused.
      - Visibility is `always || hovered` (`element.rs:3516`), so the slider vanishes when a drag leaves the minimap.
      - It's drawn even when the document doesn't scroll.
      - No shadow.

43. **Minimap width formula and section header details** — **P3**, fix **S**
    - **Monaco:**
      - `minimapWidth = min(maxColumn*mcw, floor((remaining - vbar - 2)*mcw/(typicalHalfwidthCharWidth + mcw)) + 8)`, and fill mode squeezes the minimap with a width multiplier (`editorOptions.ts:2867-2903`).
      - Section headers use the minimap background at 0.7, an x of 4 logical px (at 2x), the baseline at `top + fontSize`, a 0.4-canvas-px separator, and `sectionHeaderLetterSpacing` (`minimap.ts:1842-1937`).
    - **Ours:**
      - `max_column * scale`, capped at `width/4` (`element.rs:1464-1466`).
      - Headers use the editor background (`element.rs:4019`), an x of 8 logical px (`element.rs:2539`, `2553`), a baseline about 0.35·fs lower (`element.rs:2552-2556`), a 1px fill at foreground 0.4 (`element.rs:2563`, `4024`), and no letter spacing (`options.rs:222`).
      - The click reveal ignores `smooth_scrolling`.

### J. Scrollbars, overview ruler, scroll shadow

44. **Overview ruler marks use the wrong vertical formula** — **P1**, fix **M**
    - **Monaco:** `y = lineTop * (rulerHeight / scrollHeight)`. The scroll height includes view zones, padding and `scrollBeyondLastLine`, and the ruler excludes the arrows. So marks line up with the slider (`browser/viewParts/overviewRuler/decorationsOverviewRuler.ts:386-431`; `editorOptions.ts:3061-3065`).
    - **Ours:** `y = track.top + track.height * view_line / view_line_count` over the full viewport (`element.rs:2918-2939`). The arrow-adjusted `layout_info.overview_ruler` (`element.rs:2287`) isn't used.
    - In a short file with `scrollBeyondLastLine` on, the last line's mark sits at the bottom instead of about halfway down. Errors don't line up with the slider, and code-lens zones skew everything.

45. **Overview marks are fixed 3px dots at the start row, unmerged and unsorted** — **P2**, fix **S**
    - **Monaco:** a mark spans its start line to its end line, with a 6px minimum centered on the range. Same-color marks in the same lane within 1px are merged, sorted by zIndex then color (`decorationsOverviewRuler.ts:214`, `367`, `425-459`).
    - **Ours:** one 3px quad per decoration (`element.rs:2931-2939`). Multi-line selections and diff hunks look like dots, and overlapping translucent marks stack darker.

46. **Diagnostic overview colors are hard-coded, and info markers are missing** — **P2**, fix **S**
    - **Monaco:** error, warning and info markers all go in the Right lane, colored with `editorOverviewRuler.errorForeground`, `warningForeground` and `infoForeground` (`common/services/markerDecorationsService.ts:245-292`; `common/core/editorColorRegistry.ts:75-77`).
    - **Ours:** warning and above only, in the squiggle colors (`markers.rs:93-99`, `218-219`). Themes can't override them.

47. **Scroll shadow is a 1px line, and we add a left shadow Monaco doesn't draw** — **P2**, fix **S**
    - **Monaco:** the editor sets `useShadows: false` on its scrollbars (`browser/viewParts/editorScrollbar/editorScrollbar.ts:47`). `ScrollDecorationViewPart` draws only a top shadow: `box-shadow: scrollbar.shadow 0 6px 6px -6px inset` when `scrollTop > 0`, full width, minus the scrollbar when the minimap is on the right (`scrollDecoration.css:7-12`, `scrollDecoration.ts:40-62`).
    - **Ours:** a 1px quad at 0.6, plus a 1px left shadow when scrolled horizontally (`element.rs:4042-4056`). With the minimap on the left, the width is wrong.

48. **Overview ruler cursor marks, lanes, border and background** — **P3**, fix **S**
    - **Monaco:**
      - Cursor marks use the cursor color at 0.7 alpha, or `editorMultiCursor.primary/secondary.foreground` with several cursors, and are 2px tall, centered and clamped (`decorationsOverviewRuler.ts:60-65`, `312-319`, `464-505`).
      - Lanes start 1px in; with 3 lanes, the side lanes are floored and the center lane takes the remainder (`:98-134`).
      - The border has a left and a top edge (`:509-517`).
      - `editorOverviewRuler.background` fills the ruler, falling back to the editor background when the minimap is on the right (`:72-81`, `394-405`).
    - **Ours:**
      - Full-opacity `colors.cursor`, top-aligned, one color for every cursor (`element.rs:2942-2951`).
      - Lanes are plain thirds from x=0 (`element.rs:4110-4125`).
      - Only a left border (`element.rs:3946-3950`).
      - No background key.

49. **Scrollbar track click, slider rounding, fade-in and arrow clamp** — **P3**, fix **XS**
    - **Monaco** (unvendored `abstractScrollbar`/`scrollbarState`/`scrollbars.css`):
      - With `scrollByPage`, it still starts a drag after paging, and pages by the scrollbar's visible size.
      - Slider size is `round(max(20, floor(...)))`.
      - The scrollbar fades in over 100ms.
      - `arrowSize` is used as-is.
    - **Ours:**
      - Returns without a drag and pages by the `text_bounds` height (`editor.rs:3088-3104`).
      - No rounding (`element.rs:2117-2119`, `2160-2162`).
      - Snaps to full opacity (`view/animation.rs:280-282`).
      - Clamps the arrows to a quarter of the bar (`element.rs:2108`, `2149`), which then disagrees with `layout_info`.

### Already matching (not listed)
- **Embedded views:**
  - Zone clicks don't fall through to the text.
  - Zone placement follows folds and edits.
  - Content widget above/below/exact preference with fallback.
- **Peek:**
  - 18 lines at first, at most 80% of the editor (at least 12).
  - Sash bounds of 5 and 35 lines.
  - The layout ratio and height persist across peeks.
  - Theme colors match.
  - The arrow is a third of the line height.
  - Double-click opens the reference.
  - F4 and Shift+F4.
  - Focus toggling.
  - The preview stops the wheel by default.
- **Code lens:**
  - Height and font-size formulas.
  - Separators.
  - Debounce and visible-range resolve.
  - Click on mouse-down, link hover color.
- **Placeholder:** see section E.
- **Scrollbars:**
  - Default sizes.
  - When each bar appears.
  - Slider colors for normal, hover and active.
  - Arrows repeat.
  - The 500ms hide delay and 800ms fade-out.
  - The default jump-to-click on the track.
- **Overview ruler:**
  - Lane counts 0, 1 and 2.
  - `hideCursorInOverviewRuler`.
- **Rulers:** multiple, per-ruler color, and they scroll horizontally.
- **Sticky scroll:**
  - `maxLineCount` and the three default models.
  - Line-number modes.
  - `scrollWithEditor`.
  - Shift-hover to show a scope's end.
  - The actions.
- **Gutter:**
  - `foldBackground`.
  - `renderFinalNewline: off`.
  - Margin backgrounds.


## D. Diff editor and multi diff editor


Scope: `crates/gpui-monaco/src/diff_editor.rs` (abbreviated `de.rs`), `src/diff_editor/{accessible,unchanged}.rs`, `src/multi_diff_editor.rs` (`mde.rs`), `src/diff/`.
Vendor root: `vendor/vscode/src/vs/editor/browser/widget/` (abbreviated `W/`).

Screenshots are in this directory: `de-sbs.png` (side by side), `de-inline.png` and `de-inline2.png` (inline), `de-hide-moves.png` (hidden unchanged regions with moves), `de-multi.png` (multi diff), `de-f7.png` (accessible diff viewer). The test files are `files/old.rs` and `files/new.rs`.

The diff algorithm port (`src/diff/`: myers/DP LCS, heuristics, moves, legacy) mirrors the vendor file set and has its own test suite. I found no algorithm-level gaps worth listing. Everything below is in the widgets.

---

### P1: visibly broken or wrong behavior

#### 1. Sub-editors keep the minimap, folding and sticky scroll
- **Severity:** P1
- **Monaco:** `_adjustOptionsForSubEditor` forces these settings on both sides (`W/diffEditor/components/diffEditorEditors.ts:182-215`):
  - `minimap.enabled = false`
  - `folding = false`
  - `stickyScroll` disabled when `hideUnchangedRegions` is on
  - in inline mode, `stickyScroll` and `unicodeHighlight` are off for the hidden original
  - `glyphMargin = renderSideBySide` on the original side
- **Ours:** `apply_editor_options` (`de.rs:1479-1495`) only sets `code_lens` and `aria_label`.
- **Effects:**
  - Both sides draw a full minimap, which takes about 220px per side in the screenshots (`de-sbs.png`, `de-hide-moves.png`).
  - In inline view the minimap paints over the deleted-lines zone, so long deleted lines are cut off under it (`de-inline2.png`).
  - Folding is live on each side independently. Folding one side breaks the alignment and the scroll sync.
  - Sticky scroll stays on with hidden regions, so a sticky header can come from hidden lines. `de-inline.png` shows a sticky line over the diff.
- **Fix:** XS. Set these options in `apply_editor_options` and re-run it when the inline state changes.

#### 2. Alignment ignores other view zones, and goes stale on layout, wrap or font changes
- **Severity:** P1
- **Monaco:** `computeRangeAlignment` counts every view zone except the diff editor's own through `getAdditionalLineHeights` (`W/diffEditor/components/diffEditorViewZones/diffEditorViewZones.ts:611-640`). That covers code lenses, peek, the find zone, the F8 marker zone and host zones. Alignment is recomputed whenever:
  - either editor's view zones change (`:82-83`)
  - `wrappingInfo` or `lineHeight` changes (`:84-89`)
- **Ours:** `apply_alignment` (`de.rs:1754-1808`) counts only `wrap_extra_heights()` and the move captions. It runs only from `apply_diff`, which is called on diff results, option changes and inline toggles. `subscribe` (`de.rs:1320-1363`) never listens for `ViewZonesChanged` or layout changes.
- **Effects:**
  - Code lenses (`diff_code_lens`), the F8 problem zone, peek views and find-zone zones on one side shift that side out of line. Scroll sync then keeps the two sides offset.
  - With word wrap on, dragging the sash or resizing the window changes the wrapping, but the padding zones keep their old heights.
  - Font zoom (Cmd+wheel, `font_zoom.rs`) changes the line height. Every `ZoneHeight::Pixels` padding zone, and the inline deleted zone (`de.rs:2162`), keeps the old height.
- **Fix:** M.
  1. Track the diff editor's own zone ids.
  2. Build the extra heights from all other zones plus wrapping.
  3. Re-run the alignment on `ViewZonesChanged`, layout-width changes and line-height changes, debounced as Monaco does with `RunOnceScheduler(0)`.

#### 3. Inline deleted lines don't wrap, add no scroll width, and get clipped
- **Severity:** P1
- **Monaco:**
  - Deleted lines are rendered with the modified editor's line-breaks computer, so they wrap (`diffEditorViewZones.ts:166-185`, `renderLines`).
  - The zone height is `heightInLines * lineHeight`.
  - `minWidthInPx` widens the modified editor's horizontal scroll range (`:270-279`).
  - The original side gets diagonal-fill zones for wrapped deleted lines (`:256-268`).
- **Ours:** `DeletedCode` (`de.rs:1007-1196`) renders each row `whitespace_nowrap` inside `overflow_hidden`, with the zone height fixed at `rows * line_height` (`de.rs:2159-2166`).
- **Effects:**
  - A deleted line longer than any modified line can't be scrolled into view. `de-inline2.png` shows the `const LONG` line cut off.
  - With word wrap on there is no horizontal scroll at all, so the overflow is simply lost.
- **Fix:** M. Lay out the deleted rows with the modified editor's wrap width and report a min width for the zone. Needs a min-width field on `ZoneState`.

#### 4. Accessible diff viewer lets clicks and wheel events through to the hidden editors
- **Severity:** P1
- **Monaco:**
  - The editors are hidden with `visibility: hidden` while the viewer shows (`W/diffEditor/diffEditorWidget.ts:403-405`).
  - The viewer scrolls through its own `DomScrollableElement` (`components/accessibleDiffViewer.ts:381`).
- **Ours:**
  - The viewer is an absolute child drawn over the still-painted editors (`de.rs:3744`). Its root div (`accessible.rs:483-530`) has no `occlude()`.
  - Its list uses `overflow_y_scroll` with no `on_scroll_wheel(stop_propagation)`. This is the same bug class as the old F8 zone.
- **Effects:**
  - The wheel scrolls the viewer and the invisible editors underneath.
  - A mouse down on the viewer reaches the modified editor's `on_mouse_down` (`editor.rs:5039`). That focuses the hidden editor and moves its cursor, so the arrow keys stop reaching the viewer.
- **Fix:** S.
  - Add `occlude()` to the viewer root and stop wheel propagation.
  - Add horizontal scrolling, since long lines are clipped today (`de-f7.png`).
  - Don't render the editors while `only_show_accessible_diff_viewer` or the viewer is open, or render them invisible.

#### 5. Multi diff editor has no scrollbar
- **Severity:** P1
- **Monaco:** `CompressedVirtualizedScrollView` wraps the list in a `SmoothScrollableElement` with vertical and horizontal scrollbars set to `Auto` (`W/multiDiffEditor/compressedVirtualizedScrollView.ts:105-115`). Each editor's vertical and horizontal scrollbars are hidden (`diffEditorItemTemplate.ts:382-387`).
- **Ours:**
  - Item scrollbars are hidden (`mde.rs:185-190`), but the list draws no scrollbar (`mde.rs:481-495`). The only way to scroll is the wheel, with no position indicator and no drag (`de-multi.png`).
  - Hiding the scrollbars still leaves each editor's overview-ruler strip and its top cursor mark visible. Monaco's ruler lives inside the scrollbar, so it disappears with it.
- **Fix:** S/M. Add vertical and horizontal scrollbars to the list, and hide the overview ruler when the scrollbar is hidden.

---

### P2: missing Monaco sub-features and notable fidelity gaps

#### 6. Inline view has no original line-number column
- **Severity:** P2
- **Monaco:** In inline mode the original editor stays visible as a narrow column to the left of the modified editor (`originalWidth = max(5, layoutInfoDecorationsLeft)`, `W/diffEditor/diffEditorWidget.ts:199-209`). Its zones are aligned with `gutter-delete` margins (`diffEditorViewZones.ts:283-295`), so deleted lines show their original line numbers. The `hideOriginalLineNumbers` option (`diffEditorOptions.ts:216`, also implied by `compactMode`) turns the column off.
- **Ours:**
  - In inline mode the original editor is not rendered (`de.rs:3714-3738`). Deleted rows show only `−` with no number (`de-inline2.png`).
  - The `hide_original_line_numbers` option is missing from `DiffEditorOptions` (`de.rs:133-203`).
- **Fix:** M.

#### 7. Inline deleted lines can't be selected or copied, and have no margin menu
- **Severity:** P2
- **Monaco:**
  - The zone is `line-delete-selectable` with `suppressMouseDown: false`, so text can be selected and copied (`copySelection.ts`).
  - `InlineDiffDeletedCodeMargin` shows a light bulb on hover with "Copy deleted lines", "Copy deleted line (N)" and "Revert this change" (`components/diffEditorViewZones/inlineDiffDeletedCodeMargin.ts:58-125`).
- **Ours:** `DeletedCode` is a static `StyledText` with only a `−` sign (`de.rs:1141-1184`).
  - No selection, copy, menu or revert in inline mode.
  - The gutter "↺" button covers revert only when the modified side is editable.
- **Fix:** M.

#### 8. Diff overview ruler is approximated through each editor's own ruler
- **Severity:** P2
- **Monaco:** `OverviewRulerFeature` (`W/diffEditor/features/overviewRulerFeature.ts:25-170`) draws one 30px `diffOverview` at the far right of the diff editor:
  - an original column and a modified column, 15px each, using `diffEditorOverview.removedForeground` and `diffEditorOverview.insertedForeground`
  - a `diffViewport` slider styled with the `scrollbarSlider.*` colors
  - pointer-down and wheel delegated to the modified editor
- **Ours:** `overview_ruler` is set on the line decorations (`de.rs:3452-3462`). Each sub-editor paints the marks in its own ruler, so the original's ruler sits in the middle next to the sash (`de-sbs.png`). The color is improvised as `text bg alpha*3`. There is no shared viewport slider and no `diffEditorOverview.*` theme keys.
- **Fix:** M.

#### 9. Sash is a 1px line with no hover state, reset or minimum width
- **Severity:** P2
- **Monaco:**
  - Uses the platform `Sash`: a 4px hit area with a `sash.hoverBorder` highlight.
  - Double click resets the ratio (`onDidReset`, `W/diffEditor/components/diffEditorSash.ts:78`).
  - The ratio is computed on the width minus the overview ruler, with a 100px minimum per side (`:33-48`).
  - The drag is delta-based and tracked globally.
  - A second sash appears left of the gutter when the gutter shows (`features/gutterFeature.ts:110-123`).
  - Visually, the boundary is a `scrollbar.shadow` box-shadow plus `diffEditor.border`, which is null in the default themes (`style.css` near `.side-by-side .editor.modified`).
- **Ours:**
  - `diff-sash` is `w(px(1.))` with `widget_border` as its background (`de.rs:3721-3737`), which is hard to grab.
  - The ratio is clamped to 0.1-0.9 of the full width (`de.rs:3619`, `3699`).
  - Dragging only works while the mouse is over the diff editor (`on_mouse_move` on the root, `de.rs:3688-3704`).
  - No double-click reset, no hover color.
  - The gutter is laid out after the sash, so it shrinks the modified side rather than the original.
- **Fix:** S.

#### 10. Wheel over the gutter menu or the moved-lines column doesn't scroll
- **Severity:** P2
- **Monaco:** The gutter forwards `MOUSE_WHEEL` with `delegateScrollFromMouseWheelEvent` (`gutterFeature.ts:166-170`). A click on the gutter focuses the modified editor (`:104-106`). The moved-lines SVG is `pointer-events: none` over the editors (`style.css` `.moved-blocks-lines`).
- **Ours:** The `diff-gutter` div (`de.rs:2863-2877`) and the `render_move_lines` canvas column (`de.rs:2071-2112`) are siblings of the editors with no wheel handler, so the wheel does nothing there. Clicking the gutter doesn't focus the modified editor.
- **Fix:** XS. Forward the wheel to the modified editor and focus it on click.

#### 11. Character highlights don't fill to the line end across line breaks
- **Severity:** P2
- **Monaco:** `diffAddDecoration` and `diffDeleteDecoration` use `shouldFillLineOnLineBreak: true` (`W/diffEditor/registrations.contribution.ts:66-84`). An inner change spanning lines paints its `char-insert` / `char-delete` background to the right edge.
- **Ours:** The crate has no equivalent decoration flag (grep finds no `fill_line`). Multi-line inner changes stop at the end of each line's text, which leaves ragged blocks (`de-inline2.png` and `de-hide-moves.png`, lines 49-56). The `DeletedCode` inner highlights also lack the `removedTextBorder`.
- **Fix:** S. Add the flag to `DecorationOptions` and the painter.

#### 12. Hidden-region bars: no fold-back glyph, no scroll anchor, wrong cursors
- **Severity:** P2
- **Monaco:**
  - Fully revealed regions get a glyph-margin "Fold Unchanged Region" codicon on their first line. Clicking it calls `collapseAll` (`W/diffEditor/features/hideUnchangedRegionsFeature.ts:195-275`).
  - Dragging or clicking the bottom edge uses `_runWithLowerScrollAnchor`, so the text below stays put and the bar follows the mouse (`:475`, `:485`, `:581-592`). The multi diff passes its own scroll anchor.
  - Edge cursors are `n-resize`, `s-resize` or `ns-resize` depending on `canMoveTop` / `canMoveBottom`, applied to the whole editor while dragging (`style.css`, `.diff-hidden-lines` rules). The top and bottom hit strips overlap the neighbouring lines and leave the 24px center intact.
  - The center uses the UI font with a codicon `unfold` link, whose hover color is `editorLink.activeForeground`.
  - Breadcrumb items come from an outline source. Standalone Monaco's default source is empty, so this is VS Code only.
- **Ours:**
  - No fold-back glyph. Once a region is fully revealed, only the `collapseAllUnchangedRegions` command re-hides it.
  - `drag_unchanged` and `end_unchanged_drag` (`de.rs:2389-2434`) don't anchor the scroll, so bottom-edge drags move the content under the mouse and the drag feels inverted.
  - The edges always use `ResizeUpDown`, are 5px and sit inside the 24px zone (`de.rs:901-910`).
  - The label uses the code font (`de.rs:975`) and the icon is the text glyph "↕".
- **Fix:** S, M with the scroll anchor.

#### 13. Moved-block lines stop short of the modified code
- **Severity:** P2
- **Monaco:** The moved-blocks SVG starts at the original editor's vertical scrollbar and extends into the modified editor's gutter (`W/diffEditor/features/movedBlocksLinesFeature.ts:101-125`, `:164-170`). Each arrow ends at `contentLeft - padding` on an `arrow-rectangle` in the editor background, so it points at the moved code.
- **Ours:** `render_move_lines` (`de.rs:2027-2113`) is an opaque column between the sash and the gutter. The arrows end at the gutter menu's left edge, about 100px before the code (`de-hide-moves.png`), with line numbers and the gutter in between.
- **Ours, compare mode:**
  - Monaco realigns the compared moved text (`alignmentsSyncedMovedText`) and skips padding next to it (`diffEditorViewZones.ts:107-123`, `:301-326`). Ours still aligns on `diff.changes` (`de.rs:1800-1808`).
  - Monaco animates the compare offset (`animatedObservable`). Ours snaps.
  - Monaco hides the margin revert arrows while comparing (`revertButtonsFeature.ts:38`). Ours shows them (`de.rs:2556-2605`).
- **Fix:** M.

#### 14. Multi diff: every editor is created up front, and several features are missing
- **Severity:** P2
- **Monaco:**
  - `VirtualizedItemManager` binds pooled `DiffEditorItemTemplate`s only to visible items (`W/multiDiffEditor/virtualizedItemManager.ts`) and remembers each item's `expandedContentHeight` and selections.
  - The header has these features (`diffEditorItemTemplate.ts:103-116`, `:297-305`, `:500-505`; `style.css` `.header-content`, `.active`, `.editorParent`):
    - a `MultiDiffEditorFileToolbar` actions area
    - a file icon through the resource label
    - a semibold status letter at 0.75 opacity
    - a strikethrough original path for renames
    - a header shadow once the item is scrolled
    - a bottom border on the header and the editor
    - a focus-border outline on the active item
  - Multi-diff-wide navigation is provided (`multiDiffEditorWidgetImpl.ts:620-680`): `goToNextChange` / `goToFile` and `reveal(resource, range)`.
- **Ours:**
  - `set_documents` creates a `DiffEditor` with two `CodeEditor`s and a diff computation for every file at once (`mde.rs:146-149`, `155-210`). Only rendering is virtualized.
  - The header (`mde.rs:404-458`) is a text chevron, the label, a plain status letter and the original label, with none of the extras above.
  - `reveal(index)` scrolls only to a header.
- **Fix:** M (navigation and header polish), L (true virtualization).

#### 15. The gutter menu is a fixed single button, not the hunk toolbar
- **Severity:** P2
- **Monaco:** The `DiffEditorGutter` renders `MenuId.DiffEditorHunkToolbar` and `DiffEditorSelectionToolbar` as vertical toolbars (`gutterFeature.ts:252-281`). The menu is extensible (Stage, Revert and others), shows up to 3 items, or 1 when `_isSmall`, and highlights hovered buttons with `toolbar.hoverBackground`. Items fade out over 0.7s (`style.css` `.gutterItem`). The gutter is hidden entirely when the menu has no actions (`:63`, `:108`).
- **Ours:**
  - `render_gutter` (`de.rs:2790-2879`) draws one hard-coded revert button using the text glyphs "→" / "↺", hover `opacity(0.8)` and no fade.
  - Hosts have no API to add hunk actions.
  - When a selection exists, the other changes' arrows are hidden. Monaco keeps them (`revertButtonsFeature.ts:59-71`).
- **Fix:** M.

---

### P3: minor polish

#### 16. Text glyphs stand in for codicons throughout
- **Severity:** P3
- **Monaco:** Codicons:
  - `add` / `remove` signs at 11px and opacity 0.7 (`style.css` `.insert-sign`)
  - `arrow-right` and `discard` for revert
  - `unfold` and `fold` for hidden regions
  - `compare-changes` on the moved-block action bar (12px)
  - `chevron-down` / `chevron-right` in multi diff headers
  - `close` in the accessible viewer
- **Ours:** `+`, `−`, `→`, `↺`, `↕`, `⌄`, `›`, `×`, and a text "Compare" button (`de.rs:629-651`). The signs use the full foreground color.
- **Fix:** S, once the crate has an icon font.

#### 17. Missing accessibility signals
- **Severity:** P3
- **Monaco:** `goToDiff` plays `diffLineInserted`, `diffLineDeleted` or `diffLineModified` (`W/diffEditor/diffEditorWidget.ts:749-755`). Cursor moves onto a change play the same signals (`:820-846`).
- **Ours:** `AccessibilitySignal` (`de.rs:299-306`) has no `DiffLineModified`. `go_to_diff` (`de.rs:3217-3264`) and selection changes emit nothing.
- **Fix:** XS.

#### 18. Inline breakpoint is off by one, and temporary inline mode is missing
- **Severity:** P3
- **Monaco:**
  - `couldShowInlineViewBecauseOfSize` uses `width <= breakpoint` (`diffEditorOptions.ts:33-35`).
  - A smooth resize that starts below the breakpoint stays inline (`_widthBasedLayout = 'inline'`, `:166-186`).
  - `isInEmbeddedEditor` is an option.
- **Ours:** Uses `width < breakpoint` (`de.rs:1516`). There is no temporary-inline state and no `is_in_embedded_editor` option.
- **Fix:** XS / S.

#### 19. Edits aren't applied to the existing diff while it recomputes
- **Severity:** P3
- **Monaco:** `applyModifiedEdits` / `applyOriginalEdits` patch the last diff on every edit (`W/diffEditor/diffEditorViewModel.ts:229-261`, `308-309`). Decorations and alignment zones stay right during the 200ms debounce and the computation.
- **Ours:** The stale diff stays until recompute (`de.rs:1602-1673`). Typing Enter inside the diff briefly misaligns the sides.
- **Fix:** M.

#### 20. Moved-block caption box and Compare button styling
- **Severity:** P3
- **Monaco:** The caption is an overlay on an 18px placeholder zone (`.diff-moved-code-block`, `margin-top: -4px`). The `movedOriginal` / `movedModified` border wraps only the lines, and the action is a 12px codicon.
- **Ours:**
  - The block decoration's padding wraps the caption inside the border (`de-hide-moves.png`).
  - The caption text is foreground at 0.8 opacity in a 12px UI-default font.
  - The Compare text button's checked background is `widget_border` (`de.rs:1908-1909`).
- **Fix:** XS.

#### 21. Gutter details
- **Severity:** P3
- **Monaco:** The item `.background` line is 1px wide plus a 2px left border at 50%. Toolbar buttons use 1px/2px padding (`style.css` `.gutterItem`).
- **Ours:** A 2px line at `GUTTER_WIDTH/2-1`, with a 22×20 button (`de.rs:2827-2858`). Close enough. The only real gap is the missing hover background and fade (see 15).
- **Fix:** XS.


## E. Core view and input


Scope: text and cursor rendering, selection, line highlight, whitespace and guides, word wrap, fonts, scrolling, mouse, IME and clipboard.

- Monaco paths are relative to `vendor/vscode/src/vs/editor/`.
- Our paths are relative to `crates/gpui-monaco/src/`.
- Screenshots (prefix `cv-`) are in this directory and come from `pg-shot`.

Severity: **P1** = visibly broken or wrong; **P2** = missing Monaco sub-feature or a notable fidelity gap; **P3** = polish.
Fix size: XS / S / M / L.

---

### P1

#### 1. A block cursor hides the character under it
- **Severity:** P1. **Fix:** S.
- **Monaco:** `browser/viewParts/viewCursors/viewCursor.ts:226-229`
  - For `Block` (and for `Line` wider than 2px, `:185-188`) the cursor element's `textContent` is the next grapheme, with the token class applied.
  - The theme gives the cursor `color: editorCursor.background`, falling back to `caret.opposite()` (`viewCursors.ts:395-414`).
  - The result is the character inverted inside the block.
- **Ours:** `element.rs:3780-3804` paints a solid `fill` quad in `colors.cursor` after the text, so the character disappears.
  - There is no `editorCursor.background` in `theme.rs` (only `cursor`, `theme.rs:99`).
- **Evidence:** `cv-block-crop.png`. With overtype on (Insert → `overtypeCursorStyle: block`), the `u` of `use` is fully covered.
- **Fix:** after painting the block, repaint the grapheme clipped to the block in the cursor-background color. Add `editorCursor.background`.

#### 2. No view affinity for the cursor at a wrap point: End and clicks past a wrapped segment put the caret at the start of the next visual line
- **Severity:** P1. **Fix:** M.
- **Monaco:** each cursor keeps both a `modelState` and a `viewState` (`common/cursor/oneCursor.ts:19,62,80`).
  - End on a wrapped line moves to the view line's max column.
  - That view position, (viewLine N, maxCol), stays distinct from (viewLine N+1, 1), even though both map to the same model offset.
  - The caret is drawn at the end of the first segment, and a second End goes to the real line end.
- **Ours:** cursors are model-only (`PointUtf16`).
  - `move_cursors` maps model→view with `to_view(p, false)` each time (`editor.rs:1761-1786`).
  - Rendering uses `view_lines.to_view(head, false)` (`element.rs:3002`), and `LineLayout::contains` excludes `columns.end` on non-last segments (`element.rs:86-92`).
  - So a cursor at the wrap column is drawn, and line-highlighted, at column 0 of the next segment.
  - Clicking past the end of a wrapped segment does the same (`column_for_x` returns `columns.end`; `editor.rs:2518-2519`).
- **Evidence:** `cv-wrap3-crop.png`. After `End` on wrapped line 6, debug shows head (5, 57). The caret and current-line box sit on the second segment, before `and`.
- **Fix:** keep an "at end of previous view line" affinity bit per cursor (or Monaco's view state). Pass it through `to_view`, rendering, Up/Down and Home/End.

#### 3. Drag-selection autoscroll is a crude approximation, and drags stop tracking once the mouse leaves the editor
- **Severity:** P1. **Fix:** M.
- **Monaco:**
  - `MouseDownOperation` tracks the pointer globally (`browser/controller/mouseHandler.ts:474,504`, `_mouseMoveMonitor.startMonitoring`).
  - `_getPositionOutsideEditor` (`mouseHandler.ts:~520-585`) classifies the pointer as above, below, left or right.
  - `TopBottomDragScrolling` and `LeftRightDragScrolling` (`browser/controller/dragScrolling.ts`) then scroll on every animation frame:
    - Speed is in lines per second and depends on distance: `max(30, viewportLines*(1+d))` for d ≤ 1.5 lines, then ×(2+d), then ×(7+d).
    - The selection is extended to the edge line on each frame.
  - Inside the editor, each drag step dispatches with `NavigationCommandRevealType.Minimal`, which reveals the cursor, including horizontally.
- **Ours:** `editor.rs:3485-3494` scrolls exactly one line per mouse-move event, and only while `y < text.top()` or `y > text.bottom()`.
  - There is no timer, so holding the mouse still stops scrolling.
  - There is no distance-based speed and no left/right scrolling.
  - `cursors_changed(false, …)` (`editor.rs:3560`) never reveals horizontally.
  - Moves come from `.on_mouse_move`, which GPUI only delivers while the editor's hitbox is hovered (`gpui-pre-0.3.8/src/elements/div.rs:303-313`). The text area's top equals the editor's top, so dragging above the editor delivers nothing and never scrolls up. Dragging past the bottom only fires over the horizontal scrollbar strip.
  - The same capture gap breaks scrollbar-thumb drags, minimap drags and text drag-and-drop once the pointer leaves the editor.
- **Fix:** capture mouse moves while a button is down using a window-level listener such as `window.on_mouse_event`. Port `DragScrolling` as a per-frame task.

#### 4. Bidi formatting characters reorder the line instead of showing as `[U+XXXX]`
- **Severity:** P1. **Fix:** S.
- **Monaco:** `common/viewLayout/viewLineRenderer.ts:728-760` (`isControlCharacter`) and `:1205-1215`.
  - With `renderControlCharacters` (default true), the following are rendered as the literal text `[U+202E]`, eight cells wide, with class `mtkcontrol`: U+202A-202E, U+2066-2069, U+200E/F and U+061C.
  - The text is therefore never reordered.
  - BOM, LS, PS and NEL render as U+FFFD (`:1188-1193`).
- **Ours:** `control_picture` (`element.rs:254-261`) handles only C0 and DEL. Everything else is shaped raw.
- **Evidence:** `cv-ctrl-crop.png`. Line 2, `"x<RLO>y";`, displays as `"x;"y` (the Trojan-Source reordering). The BOM on line 3 is invisible.
- **Fix:** extend `layout_segment`'s substitution to Monaco's set. The `[U+…]` text is 8 columns wide and needs the column/display-index mapping updated.

#### 5. `renderLineHighlight: 'gutter'` highlights the whole text line
- **Severity:** P1. **Fix:** S.
- **Monaco:** `browser/viewParts/currentLineHighlight/currentLineHighlight.ts:184-196`.
  - Margin and content are separate overlays. `'gutter'` renders only the margin piece (`width: contentLeft`).
  - The margin piece is drawn even when there is a selection. Only the content piece requires `_selectionIsEmpty`.
- **Ours:** `element.rs:3563-3593` draws a single box from `bounds.left()` to `text_bounds.right()` when either flag is set, so `'gutter'` paints across the text. Both pieces are also suppressed while anything is selected.
- **Fix:** split it into margin and content quads with Monaco's two predicates. In `'all'` mode the content box keeps its own left border.

---

### P2

#### 6. Paint order: indent guides, rulers and decoration backgrounds are drawn under the selection
- **Severity:** P2. **Fix:** S.
- **Monaco:** `browser/view.ts:219-224,275-282`. Content overlays stack, bottom to top:
  1. current line
  2. selections
  3. indent guides
  4. decorations (find matches, word highlights and other `className` backgrounds)
  5. whitespace
  6. wrap indicator

  Rulers come after the overlays, then text, then cursors.
- **Ours:** `element.rs:3643-3717` paints `decoration_quads`, injected backgrounds and `guides` (indent, bracket and rulers) before `paint_selection`.
  - With an opaque selection color (vs-dark `#264F78`), guides and find/occurrence highlights disappear inside the selection.
- **Evidence:** `cv-ind-crop.png`. The indent guides at columns 0 and 4 vanish inside the selection.
- **Fix:** paint selections before guides and decorations.

#### 7. Selection shape: half-width EOL extension, no concave corners, no multi-selection insets
- **Severity:** P2. **Fix:** S–M.
- **Monaco:**
  - **End-of-line extension.** `browser/viewParts/viewLines/viewLines.ts:457-467` adds a full `typicalHalfwidthCharacterWidth` to a selected line that continues to the next model line, so an empty selected line is one character wide.
  - **Concave corners.** `selections.ts:71,176-250,296-325` computes INTERN/EXTERN/FLAT corner styles with epsilon = halfwidth/4. INTERN corners get reverse-rounded 10px "selection + editor-background" pieces.
  - **Multi-selection insets.** With more than one selection, each selection's first line is inset 1px at the top and its last line 1px at the bottom (`:286-287`).
  - **High contrast.** HC themes use no radius (`selections.css`).
- **Ours:**
  - `element.rs:4267` uses `em_width / 2.`. Empty lines inside a selection show a sliver (`cv-ws.png` line 5, `cv-ind-crop.png`).
  - `rounded_corners` (`view/animation.rs:459-473`) only rounds outer corners, with a 0.5px epsilon.
  - There are no inverse corners and no inset.
- **Fix:** use the full `em_width`, and port the corner-style table plus the reverse-corner pieces.

#### 8. Current-line highlight covers only the primary cursor's view line
- **Severity:** P2. **Fix:** S.
- **Monaco:** `currentLineHighlight.ts:62-79,141-168`.
  - Every cursor's line is highlighted.
  - With word wrap on, all view lines of the cursor's model line get the background ("non-exact"). Only the cursor's own view line gets the 2px border.
  - When unfocused, `editor.inactiveLineHighlightBackground` is used.
  - In HC themes the border is 1px.
- **Ours:** `element.rs:3567` uses `cursor_view_lines[0]` only. There is no wrapped-line spread and no inactive color (`theme.rs:42-44`); the border is always 2px.

#### 9. Control characters lack Monaco's `mtkcontrol` styling
- **Severity:** P2. **Fix:** XS.
- **Monaco:** `browser/viewParts/viewLines/viewLines.css:17-20` styles the control pictures as white on `rgb(150,0,0)`.
- **Ours:** the pictures (␇, ␀) are drawn in the token color with no background (`element.rs:609-614`; `cv-ctrl-crop.png` line 1).

#### 10. Mouse-dispatch matrix incomplete
- **Severity:** P2. **Fix:** S.
- **Monaco:** `browser/view/viewController.ts:217-311` and `mouseHandler.ts:660-746`.
  - Click count ≥ 4 selects all.
  - Shift with count 2 runs `_wordSelectDrag`, which extends by words.
  - Shift with count 3 runs `_lineSelectDrag`.
  - Shift-click on line numbers runs `_lineSelectDrag`.
  - Multi-cursor modifier + Shift runs column select. With `multiCursorModifier: ctrlCmd`, that is Cmd+Shift-click, and an Alt-drag also column-selects.
  - Double-click on injected text is ignored.
  - Click count resets unless the clicks land at the same position within 400ms.
- **Ours:** `editor.rs:3202-3268`.
  - `_ => Line`: there is no quadruple-click select-all.
  - Shift is honoured only in Character mode.
  - Column select is hard-wired to Shift+Alt, whatever the modifier setting.
  - Shift-click after a double-click anchors on `anchor` only; Monaco keeps the selection-start range.

#### 11. IME composition collapses multi-cursor and ignores clause formatting
- **Severity:** P2. **Fix:** M.
- **Monaco:**
  - Composition text is applied at every cursor (`compositionType` across all selections).
  - The IME's text formats map to `edit-context-composition-secondary` (1px bottom border) and `-primary` (2px, the active clause) in `editor.compositionBorder` (`browser/controller/editContext/native/nativeEditContext.ts:504-535`, `nativeEditContext.css:31-49`).
- **Ours:** `replace_and_mark_text_in_range` (`editor.rs:3936-3975`) edits one range and then `set_selections(&[selection])`, which drops all secondary cursors.
  - The marked range gets a single 1px underline (`element.rs:1914-1917,706-716`), only for single-row compositions.

#### 12. Mouse pointer is the I-beam everywhere
- **Severity:** P2. **Fix:** S.
- **Monaco:** only the view lines and cursors carry `mouse-cursor-text`.
  - Line numbers use `cursor: default` (`browser/viewParts/lineNumbers/lineNumbers.css:14`).
  - The rest of the gutter, the scrollbars and the minimap show the default arrow.
  - Folding chevrons and the "Show more" overflow chip use `pointer` (`viewLines.css:42`).
- **Ours:** one `.cursor(...)` on the editor root (`editor.rs:4149-4162`), so the gutter, scrollbars and minimap all show the I-beam.

#### 13. Gutter widths approximated
- **Severity:** P2. **Fix:** S.
- **Monaco:** `common/config/editorOptions.ts:2954-2969`.
  - Glyph margin = `lineHeight × lanes`.
  - Folding adds a fixed 16px to `lineDecorationsWidth`, and only when `showFoldingControls !== 'never'`.
  - Line numbers are `round(digits × maxDigitWidth)`.
  - `lineDecorationsWidth` also accepts `"Nch"`.
- **Ours:** `element.rs:1441-1459`.
  - Glyph margin is a constant `GLYPH_MARGIN_WIDTH = 18` (`:24`). With the default 14px font Monaco uses 21px.
  - Folding is `min(lineHeight, 18)`, regardless of `showFoldingControls`.
  - Number width is not rounded.
- The text column starts at a different x than in Monaco.

#### 14. Wrapping column off by one versus Monaco
- **Severity:** P2. **Fix:** XS.
- **Monaco:** `editorOptions.ts:3017` computes `viewportColumn = floor((contentWidth - vbar - 2) / halfwidth)`.
- **Ours:**
  - Wrapping uses `floor((text_width - em_width) / em_width)` (`element.rs:1503-1505`). It reserves a whole column rather than 2px, so lines often wrap one column early.
  - `layout_info.viewport_column` (`element.rs:2281`) uses the correct formula, so the two disagree.

#### 15. Missing font and metric options
- **Severity:** P2. **Fix:** S–M.
- **Monaco:**
  - `letterSpacing` affects all metrics and wrapping.
  - `fontVariations` maps to `font-variation-settings` and variable weight.
  - `lineHeight` values below 8 are a multiplier of `fontSize`.
  - `copyWithSyntaxHighlighting` produces an HTML clipboard payload.
- **Ours:**
  - `options.rs` has no `letter_spacing` or `font_variations`.
  - `line_height` is pixels only (`options.rs:2177-2181`).
  - There is no HTML copy.

---

### P3

#### 16. Line and Block cursor geometry details
- **Severity:** P3. **Fix:** XS.
- **Monaco:** `viewCursor.ts:68,182-200,219-225`.
  - The Line cursor width is `min(cursorWidth, halfwidthChar)`, with 0 meaning 2px.
  - When width ≥ 2 and left ≥ 1, the bar is shifted 1px left (`paddingLeft = 1`) so it straddles the character boundary.
  - A Block cursor on a tab is one half-width character wide, not the whole tab.
- **Ours:** `element.rs:3008-3044`.
  - `cursor_width` is used unclamped (0 draws nothing).
  - There is no 1px shift.
  - The block width is the full tab advance, with a `max(em/2)` fudge.

#### 17. Multi-cursor colors and smooth caret
- **Severity:** P3. **Fix:** S.
- **Monaco:**
  - Theme keys `editorMultiCursor.primary/secondary.foreground/background` style the primary and secondary cursors (`viewCursors.ts:398-414`).
  - `cursorSmoothCaretAnimation` transitions all cursors (CSS on the layer, `viewCursors.css`).
- **Ours:**
  - A single `colors.cursor` for every cursor.
  - Only `cursor_quads.first_mut()` animates (`element.rs:3050-3063`). That is the first visible cursor, so when the primary is off-screen a secondary cursor inherits the primary's transition and slides spuriously.

#### 18. Whitespace rendering uses font glyphs instead of Monaco's default SVG
- **Severity:** P3. **Fix:** S.
- **Monaco:** `experimentalWhitespaceRendering` defaults to `'svg'` (`browser/viewParts/whitespace/whitespace.ts:242-268,305`).
  - Spaces are circles of radius `spaceWidth/7`, centered.
  - Tabs are a vector arrow `0.8 × spaceWidth` long, centered vertically.
  - In font mode Monaco picks U+2E31 vs U+00B7 by measured width, and U+FFEB when it fits.
- **Ours:** `element.rs:3217-3265,3738-3745` paints shaped `·` and `→` glyphs.
- **Boundary rule:** `view/guides.rs:209-216` treats a neighbouring tab as whitespace. Monaco checks only `CharCode.Space` for prev and next (`whitespace.ts:~175-181`), so `"a\t b"` and `"a \tb"` differ.

#### 19. Guide colors and limits
- **Severity:** P3. **Fix:** S.
- **Monaco:** `browser/viewParts/indentGuides/indentGuides.ts:141,280-342`.
  - Indent guides take per-level `editorIndentGuide.background1..6` and `activeBackground1..6` colors.
  - Bracket-pair guides take `editorBracketPairGuide.background1..6` and `activeBackground1..6`, falling back to bracket color ×0.3.
  - Guides stop past `wrappingColumn × halfwidth` (`_maxIndentLeft`).
- **Ours:** one `indent_guide` and one `indent_guide_active` (`theme.rs:1660-1667`, only `…1` read). Bracket guides always use `bracket_color().opacity(0.3)` (`element.rs:3146-3153`), and there is no `_maxIndentLeft` clip.

#### 20. High-contrast theme rules missing
- **Severity:** P3. **Fix:** S.
- `ThemeKind` is only Light or Dark (`theme.rs:1886-1891`). The HC-specific CSS is therefore never applied:
  - square selections (`selections.css` hc rules)
  - 1px line-highlight border (`currentLineHighlight.ts`)
  - cursor side borders over a selection (`viewCursors.ts:412-414`)
  - full composition border (`nativeEditContext.css:44-49`)

#### 21. Scroll shadows approximated
- **Severity:** P3. **Fix:** XS.
- **Monaco:**
  - The top shadow is a 6px `box-shadow: scrollbar.shadow 0 6px 6px -6px inset`, with width = editor width − vertical scrollbar (`browser/viewParts/scrollDecoration/scrollDecoration.css`, `.ts:52-60`).
  - The left shadow is the scrollable element's 3px inset shadow.
- **Ours:** 1px lines at 60% opacity, full width (`element.rs:4042-4057`).

#### 22. Active line number follows every cursor
- **Severity:** P3. **Fix:** XS.
- **Monaco:** `browser/viewParts/lineNumbers/lineNumbers.ts:74-79,195-196` uses only the primary cursor's model line, for both the active color and relative/interval numbering.
- **Ours:** `element.rs:3304` uses `selections.iter().any(|s| s.head.row == row)`, so multi-cursor lights up several numbers.

#### 23. "Show more" overflow chip styling approximated
- **Severity:** P3. **Fix:** XS.
- **Monaco:** `.mtkoverflow` has a 1px `contrastBorder`, 2px radius, 4px padding, a hover background and a pointer cursor (`viewLines.css:34-46`).
- **Ours:** an injected-text background with a 3px radius, inset 1px, and no hover or border (`element.rs:1984-2000,3684-3698`).

#### 24. Gutter-click focus and macOS Ctrl-click
- **Severity:** P3. **Fix:** XS.
- **Monaco:** `mouseHandler.ts:320-337`.
  - Clicks in the glyph margin or line decorations don't steal focus.
  - On macOS, Ctrl+left-click is not handled as a click; it opens the context menu.
- **Ours:**
  - `editor.rs:3024` focuses before the gutter check.
  - `modifiers.control` is never consulted for the context menu, so Ctrl-click on a Mac just moves the caret.

#### 25. Blink repaint cost
- **Severity:** P3. **Fix:** S.
- Smooth/phase/expand blinking repaints the whole editor every 16ms for 10s after every cursor move (`editor.rs:1683-1700`). Monaco uses a compositor-only CSS animation.
- Consider repainting only the cursor layer, or lowering the frame rate when opacity is flat. The 0-20% and 60-100% keyframe ranges have constant opacity.

---

### Verified as matching Monaco (not findings)

- Blink keyframes and timing (`view/animation.rs` vs `viewCursors.css`).
- `cursorSmoothCaretAnimation` pause rules (`explicit`, cursor-count change).
- Read-only cursors are solid; cursors are hidden when unfocused.
- `cursorHeight`.
- Rulers: column × halfwidth, 1px.
- Word-wrap indicator position.
- `scrollBeyondLastLine` and horizontal-scrollbar content height.
- Wrapping-indent computation.
- Font-ligature feature strings (`"liga"/"calt"` on/off, custom settings).
- Drag-and-drop edit math and Alt/Ctrl copy.
- Double-click word rules (`word_selection` port) and `doubleClickSelectsBlock`.
- Middle-click column select and scroll.
- Mouse-wheel zoom.
- Clipboard metadata (`isFromEmptySelection`, multicursor text).
- `selectionForeground` (`inline-selected-text`).


## F.A. Commands and keybindings: coreCommands and contrib A–I


Monaco paths are relative to `vendor/vscode/src/vs/editor/`. Port paths are relative to `crates/gpui-monaco/src/`.
Out of scope as asked: #231 copyWithSyntaxHighlighting, `floatingMenu` and `inlineProgress` (workbench UI, documented in MONACO_PARITY.md:186 as not planned).

### Findings

1. **Provider checks match on language id only, so URI and scheme selectors are ignored** (P1, S)
   Monaco sets `editorHas*Provider` with `registry.has(model)`, which matches the whole selector: language, scheme and pattern (`browser/widget/codeEditor/codeEditorWidget.ts:2423-2424` and the lines around it). In the port, the `has_provider!` preconditions (`editor_actions.rs:171-199`) build a `ProviderTarget` from `language.as_deref()`, which sets `uri: None` (`providers.rs:90-96`). `goto.rs:124-146` (`request_locations`) does the same. So a provider registered with a `{scheme}` or `{pattern}` selector has two problems:
   - Its go-to, peek and references commands never call the provider.
   - Its preconditions are false. The palette hides the actions, and so does the context menu, because `context_menu.rs` `when_supported!` goes through `supported_actions`. `trigger()` also no-ops.

   Format, hover, code actions and other features already use `self.provider_target(cx)`.
   Fix: use `editor.provider_target(cx)` in `has_provider!` and in `goto.rs:124-146`.

2. **The "Format Document" precondition ignores range formatters** (P1, XS)
   Monaco: `_hasDocumentFormattingProvider = documentFormattingEditProvider.has(model) || documentRangeFormattingEditProvider.has(model)` (`codeEditorWidget.ts:2423`). Port: `has_provider!(can_format, writable, document_formatting_providers)` (`editor_actions.rs:192`). Yet `format_document` does use range formatters as synthetic document formatters (`format.rs:52-69`). As a result, a language with only a range formatter does not show Format Document in the palette or the context menu, and `trigger("editor.action.formatDocument")` is refused.

3. **Alt+\ is bound to Trigger Inline Suggestion, but Monaco has no keybinding for it** (P1, XS)
   Monaco's `TriggerInlineSuggestionAction` has no `kbOpts` (`contrib/inlineCompletions/browser/controller/commands.ts:86-100`). No `KeyCode.Backslash` binding exists in `vs/editor` except jumpToBracket. Port: `actions.rs:1279` `KeyBinding::new("alt-\\", TriggerInlineSuggest, ctx)`. On macOS, Option+\ types "«", and the port now eats that key in every editor.
   Fix: remove the binding.

4. **Alt+Enter (Select All Matches) works only inside the find inputs, and it closes the find widget** (P1, XS)
   Monaco's `SelectAllMatchesAction` has `kbExpr: EditorContextKeys.focus` and `primary: Alt+Enter` (`contrib/find/browser/findController.ts:1200-1208`). It fires from the editor text whenever the find widget is visible. `FindController.selectAllMatches` (`findController.ts:432-439`) only focuses the editor; the widget stays open.
   Port:
   - It binds `alt-enter` only in `SINGLE_LINE` (`actions.rs:1360`), so nothing happens from the main editor.
   - The handler calls `this.close_find(window, cx)` after selecting (`editor.rs:4566-4567`).

   Fix: bind `alt-enter` in `CodeEditor` when find is visible (add a `find_widget_visible` key context), and replace `close_find` with focusing the editor.

5. **Shift+Escape secondary bindings are missing** (P2, XS)
   Monaco binds Shift+Escape as a secondary key for:
   - `cancelSelection` and `removeSecondaryCursors` (`browser/coreCommands.ts:1783-1784`, `:1810-1811`)
   - `closeFindWidget` (`findController.ts:1078-1079`)
   - `closeMarkersNavigation` (`contrib/gotoError/browser/gotoError.ts:301-302`)
   - `closeReferenceSearch` (`contrib/gotoSymbol/browser/peek/referencesController.ts:369-377`)

   Port: only `escape` → `RemoveSecondaryCursors` (`actions.rs:1208`), whose handler runs that whole cascade. Shift+Escape exists only for the code action menu, rename and parameter hints.
   Fix: add `KeyBinding::new("shift-escape", RemoveSecondaryCursors, ctx)`.

6. **Refactor, Source Action, Organize Imports, Fix All and Auto Fix ignore `providedCodeActionKinds`** (P2, S)
   Monaco gates these with `contextKeyForSupportedActions(kind)`, a regex on `supportedCodeAction` (`contrib/codeAction/browser/codeActionCommands.ts:151-154`, `:193-196`, `:232-234`, `:260-262`, `:280-282`). `supportedCodeAction` joins the providers' `providedCodeActionKinds ?? []` (`codeActionModel.ts:26`, `:240-241`). So in Monaco:
   - A provider without `providedCodeActionKinds` does not show "Refactor..." or "Source Action..." in the context menu.
   - Organize Imports, Fix All and Auto Fix are disabled unless some provider declares that kind.

   Port: all of these use `can_code_action` (writable plus any provider; `editor_actions.rs:199`, `:365-369`) and `context_menu.rs:88-89,108-109`, and treat `provided_kinds() == None` as "any" (`providers.rs:756-759`).
   Fix: add a `supported_code_action_kinds` check per action, where `None` adds nothing.

7. **Core commands that take arguments, and trigger handler ids, are not resolvable** (P2, M)
   Monaco registers these in `browser/coreCommands.ts`:
   - Commands with arguments: `cursorMove` (`:592`), `editorScroll` (`:1313`), `revealLine` (`:1835`), `setSelection` (`:1920`), `createCursor` (`:899`), `columnSelect` (`:456`).
   - `scrollLeft` and `scrollRight` (`:1586`, `:1610`).
   - `lastCursorWordSelect`, `lastCursorLineSelect` and `lastCursorLineSelectDrag` (`:1673`, `:1766`, `:1772`).
   - The overwritable handler commands `type`, `replacePreviousChar`, `compositionType`, `compositionStart`, `compositionEnd`, `paste` and `cut`, plus their `default:*` variants (`:2153-2179`). `default:undo` and `default:redo` are in `browser/editorExtensions.ts:671`, `:700`.

   None of these ids are in `COMMANDS` (`editor_actions.rs:418+`), so `CodeEditor::trigger` and `trigger_with_payload` (`api.rs:911-967`) fail with "unknown action". `editor.trigger('keyboard', 'type', {text})` and `executeCommand('cursorMove', …)` are common Monaco API idioms. The port has the primitives: `type_text` (`editor.rs` `pub fn type_text`) and `column_select`.
   Fix: add actions with serde args and `COMMANDS` entries.

8. **Fold and unfold commands ignore their arguments** (P2, S)
   Monaco's `editor.fold`, `editor.unfold` and the other `FoldingAction`s take `{levels, direction: 'up'|'down', selectionLines}` (`contrib/folding/browser/folding.ts:548-580`, `:616-660`, `:687-700`, `:769-780`). Port: `Fold`, `Unfold` and the rest are unit actions (`actions.rs:460-517`), so `trigger_with_payload("editor.fold", {...})` cannot be built.

9. **Hidden-unchanged-region breadcrumbs in the diff editor are missing** (P2, M)
   Monaco's `contrib/diffEditorBreadcrumbs/browser/contribution.ts` gives `HideUnchangedRegionsFeature` an outline source. The collapsed region header then shows the enclosing symbols, which you can click to reveal (`browser/widget/diffEditor/features/hideUnchangedRegionsFeature.ts:35-50`, `:515`). Port: `diff_editor/unchanged.rs:14` says "breadcrumbs … there are none here yet". The feature is in standalone (`editor.all.ts` imports it) but is not listed as a gap in MONACO_PARITY.md:173.

10. **Missing command aliases and ids** (P2, XS each)
    - `editor.action.goToDeclaration` is an alias of revealDefinition (`contrib/gotoSymbol/browser/goToCommands.ts:310`). Missing.
    - `editor.action.previewDeclaration` is an alias of peekDefinition (`goToCommands.ts:372`). Missing.
    - `editor.action.findReferences` (uri, position) peeks references (`goToCommands.ts:833-856`). Missing.
    - `goToNextReferenceFromEmbeddedEditor`, `goToPreviousReferenceFromEmbeddedEditor` and `closeReferenceSearchEditor` (`peek/referencesController.ts:357-361`). Missing.
    - `editor.gotoNextSymbolFromResult` and `.cancel` (`gotoSymbol/browser/symbolNavigation.ts:158`, `:173`). The behavior exists but the ids aren't in `COMMANDS`.
    - `closeMarkersNavigation` (`gotoError.ts:295`) and `closeFindWidget`: the behavior exists only through the Escape cascade.
    - `editor.hideDropWidget` (`contrib/dropOrPasteInto/browser/dropIntoEditorContribution.ts:37`). The port has `editor.hidePasteWidget` only.
    - The legacy `editor.action.format` command, which formats the selection or the document (`contrib/format/browser/formatActions.ts:292-303`).
    - `default:undo` and `default:redo` (see 7).

11. **Command arguments are dropped for several actions** (P3, S)
    - `editor.action.refactor` and `editor.action.sourceAction` take `{kind, apply, preferred}` (`codeActionCommands.ts:155-179`, `:197-222`). The port's `Refactor` and `SourceAction` are unit actions (`code_action.rs:920-945`).
    - `editor.action.pasteAs` takes `{kind}` or `{preferences: []}` (`copyPasteContribution.ts:58-110`). The port's `PasteAs` is a unit action (`editor.rs:4790`); `paste_as(only)` exists but can't be reached from a command.
    - `editor.action.showHover` takes `{focus: 'noAutoFocus'|'focusIfVisible'|'autoFocusImmediately'}`, and focuses right away when `accessibilitySupport` is on (`contrib/hover/browser/hoverActions.ts:44-104`). The port's `show_or_focus_hover` (`hover.rs:969-976`) has neither.
    - `editor.action.inlineSuggest.trigger` takes `{providerId, explicit, changeHintData, showNoResultNotification}` (`inlineCompletions/.../commands.ts:96-131`). The port's `TriggerInlineSuggest` is a unit action.

12. **Palette preconditions are looser than Monaco's** (P3, XS)
    Monaco hides these from the palette when the condition fails; the port uses `always` or `has_*`:
    - **Selection anchor.** goTo, selectFrom and cancel require `selectionAnchorSet` (`contrib/anchorSelect/browser/anchorSelect.ts:126`, `:140`, `:159`). Port: `always` (`editor_actions.rs:241-243`).
    - **Folding.** Every folding action requires `foldingEnabled` (`folding.ts:641` and others). Port: `always` (`editor_actions.rs:331-356`).
    - **Peek and embedded editors.** Peek Definition, Declaration, Type Definition, Implementation and References, plus Go to Declaration and Go to References, require `notInPeekEditor && !isInEmbeddedEditor` (`goToCommands.ts:355-359`, `:416-418`, `:449-452`, `:535-538`, `:621-624`, `:674-677`, `:712-715`). Port: only `revealDefinitionAside` checks `in_peek` (`editor_actions.rs:209-211`, `:370-381`).
    - **Insert color.** `insertColorWithStandaloneColorPicker` requires the picker to be focused, not only visible (`standaloneColorPickerActions.ts:72`).
    - **Increase/Decrease Hover Verbosity.** Monaco requires `hoverVisible` and the port uses `hover_visible`, which matches. The scroll and page hover actions require `hoverFocused`; the port uses `hover_visible`, as its own comment notes.

    Separately, Monaco's precondition for `removeBrackets` and `selectToBracket` is `undefined`, but the port uses `writable` (`editor_actions.rs:311`), which hides them in read-only editors.

13. **Keybindings ignore preconditions** (P3, M)
    In Monaco, an action's precondition is folded into its keybinding's `when` (`browser/editorExtensions.ts:133-138`). When it fails, the key falls through. Port bindings dispatch straight to handlers and never consult the `Builtin` precondition. For example, Cmd+/, Alt+Up and Cmd+] in a read-only editor show "Cannot edit in read-only editor" (from `apply_inner`, `editor.rs:1501-1503`), where Monaco does nothing silently.

14. **Symbol navigation uses F4 instead of Monaco's F12** (P3, S)
    Monaco's `editor.gotoNextSymbolFromResult` binds F12 when `hasSymbols` (`symbolNavigation.ts:154-170`). F4 is only `goToNextReference` when the reference search or a peek editor is visible (`peek/referencesController.ts:330-341`). Port: `f4` → `GoToNextReference` in the editor context also steps symbol navigation (`actions.rs:1266`, `editor.rs:4944-4951`). F12 has no symbol-navigation meaning.
    In Monaco, the later-registered GoToDefinition F12 usually shadows this binding, so the impact is low. The extra F4 behavior is not in Monaco.

15. **Find widget keyboard details** (P3, S)
    - Cmd/Ctrl+Down in the find or replace input should focus the editor (`contrib/find/browser/findWidget.ts:915-919`, `:957-961`). The port has no such binding.
    - Tab in the find input should go to the replace input, or to the case-sensitive toggle when replace is hidden. Tab in the replace input should go to the toggle (`findWidget.ts:905-912`, `:945-949`). The port cycles between the inputs only (`find_widget.rs:491-501`), and its toggles can't take focus.
    - Monaco binds Shift+Enter → previous match only in the find input, and Cmd+Enter → replace all only in the replace input on mac (`findController.ts:808`, `:1186-1196`). The port binds both in both inputs through `SINGLE_LINE` (`actions.rs:1359-1361`).

16. **No context menu on inlay hint parts** (P3, S)
    Monaco shows the Go to Definition, Peek and other `EditorContext` items when you right-click a hint part that has a location (`contrib/inlayHints/browser/inlayHintsController.ts:400-408`, `inlayHintsLocations.ts:40-75`). Port: `inlay_hints.rs` has no right-click handling.

17. **Context menu: smaller differences** (P3, XS)
    - **Shift+F10 does not reveal the cursor first.** Monaco calls `revealPosition` before anchoring at the cursor (`contrib/contextmenu/browser/contextmenu.ts:198-210`). The port anchors at the cursor's layout line, or at the text origin when the line is off-screen (`context_menu.rs:560-571`).
    - **The menu also opens over injected text.** Monaco skips the editor menu when right-clicking injected text (`contextmenu.ts:97-99`); the port does not check for it (`editor.rs:2985-2991`).

18. **Code action menu: nearby quick fixes are missing** (P2, M; already documented)
    In standalone, `editor.codeActionWidget.includeNearbyQuickFixes` defaults to true, so Quick Fix away from a diagnostic offers the nearest fix on the line (`codeActionModel.ts:220-223`, `:252`). The port's `code_action.rs:11` says "Not ported", which matches MONACO_PARITY.md:152. Listed here for completeness.

19. **Inline completions: smaller gaps** (P3, S; partly documented)
    - `editor.action.inlineSuggest.snooze` and `cancelSnooze` (`browser/services/inlineCompletionsService.ts:183-290`) are missing. MONACO_PARITY.md:162 already says "no snooze".
    - The `inlineSuggest.triggerCommandOnProviderChange` option and the `experimental.*` options (`common/config/editorOptions.ts:4612-4616`) are not in `InlineSuggestOptions` (`options.rs:871-895`).
    - `acceptNextWord`'s keybinding also requires accessibility mode to be off (`commands.ts:147`); the port's `cursor_before_ghost_text` binding (`actions.rs:1342`) does not check it.

20. **`selectionHasDiagnostics` (MarkerSelectionStatus) and other context keys are not exposed** (P3, XS)
    The keys are `selectionHasDiagnostics` (`gotoError/browser/markerSelectionStatus.ts`), `selectionAnchorSet`, `foldingEnabled`, `markersNavigationVisible`, `referenceSearchVisible`, `hasSymbols`, `pasteWidgetVisible`, `dropWidgetVisible`, `cancellableOperation` and `supportedCodeAction`. The port publishes no equivalent GPUI key-context flags, so a host's `KeybindingRule.when` can't target them. The existing flags are in `editor.rs:4042-4087`. Grouped with 12 because the same flags would serve as the precondition predicates.

### Checked and matching

- **coreCommands.** The cursor, select, column-select, scroll, delete, tab/outdent, lineBreakInsert and undo/redo keybindings match per platform. This includes the mac Emacs secondaries, the Windows-only Ctrl+Shift+Up/Down, Linux with no column-select keys, and Redo as Ctrl+Y / Ctrl+Shift+Z / Cmd+Shift+Z. `columnSelection` mode on Shift+arrows matches, and so does the Escape cascade order. The extra mac Cmd+Home/End → CursorTop/Bottom bindings are additions with no Monaco equivalent.
- **editorExtensions.** Undo, redo and selectAll match their ids, keys and read-only no-op.
- **anchorSelect.** Ids, Cmd/Ctrl+K Cmd/Ctrl+B and K K, Escape cancel (through the cascade) and decoration-tracked behavior match.
- **bracketMatching.** jumpToBracket, selectToBracket and removeBrackets match their keys.
- **caretOperations.** moveCarretLeft/Right and transposeLetters (mac Ctrl+T) match.
- **clipboard.** Cut, copy and paste ids match, including the Windows and Linux secondaries. Their context menu entries (9_cutcopypaste, orders 1/2/4, `when` writable) match.
- **codeAction.** Ids, keybindings (quickFix, refactor on Ctrl+Shift+R everywhere, Shift+Alt+O, Auto Fix on mac Cmd+Alt+.), the `editor.action.codeAction` args and the menu with headers match.
- **codelens.** `showLensesInCurrentLine`, the lens font and height formula, and the too-large-for-tokenization cutoff match.
- **colorPicker.** Show, hide and insert ids and keys, the hide-on-blur behavior, and the `colorDecorators*` and `defaultColorDecorators` options match.
- **comment.** The four ids and their keys (blockComment on Linux is Ctrl+Shift+A) match, as do the `comments.insertSpace` and `ignoreEmptyLines` options and the ignore-first-line deduplication.
- **contextmenu.** Shift+F10, the `contextmenu` option, cursor placement on right-click, grouping and ordering, and the Peek submenu at navigation/100 match.
- **cursorUndo.** The 50-entry cap, scroll restore, clearing on edits, and Cmd/Ctrl+U match.
- **diffEditorBreadcrumbs.** Missing; see 9.
- **dnd.** The `dragAndDrop` and `columnSelection` gating matches.
- **documentSymbols.** It holds only an internal `_execute` command; nothing to port.
- **dropOrPasteInto.** pasteAs, pasteAsText, changePasteType and changeDropType match, as do the post-edit widget's Cmd/Ctrl+. and Escape and the `showPasteSelector`/`showDropSelector` options.
- **editorState.** `editor.cancelOperation` is present.
- **find.** All find and replace ids, F3, Cmd+G, Ctrl+H and Cmd+Alt+F, and the toggle keys per platform match, as do the find options (seed, loop, closeOnResult, globalFindClipboard and others) and Ctrl+Enter for a line break in the inputs.
- **floatingMenu, inlineProgress.** Skipped (workbench UI, not planned).
- **folding.** All ids and K-chords per platform (mac Cmd+Alt+[ and ]) match, as do foldLevel1-7.
- **fontZoom.** The ids, global zoom level, the [-5, 20] clamp and the 10% font/line-height scaling match.
- **format.** Ids, keybindings (Linux Ctrl+Shift+I), context menu orders 1.3/1.31, the Format Selection non-empty `when`, and the format-on-type and format-on-paste options match.
- **gotoError.** marker next/prev and inFiles ids and keys match, as do the zone title actions.
- **gotoSymbol.** Ids, keybindings (Linux PeekDefinition Ctrl+Shift+F10, Cmd/Ctrl+K F12), context menu and Peek submenu orders, `goToLocations`, `peekLocations` and the `showReferences` alias, and `definitionLinkOpensInPeek` match.
- **hover.** All ids and the hover-focused keys and secondaries match, as do the hover options' defaults.
- **inPlaceReplace.** The ids and Cmd/Ctrl+Shift+, and . match.
- **indentation.** All eight ids, the writable preconditions and `autoIndentOnPaste` (default off) match.
- **inlayHints.** `onUnlessPressed`/`offUnlessPressed` with Ctrl+Alt, the font size and family rule, padding and `maximumLength` match.
- **inlineCompletions.** All registered ids and Alt+]/[ match, as do Cmd/Ctrl+Right for accept-word, Tab for commit/jump and Shift+Tab for the alternative action (with ordering), and Escape for hide.


## F.B. Commands and keybindings: contrib I–Z and standalone


Monaco paths are relative to `vendor/vscode/src/vs/editor/`. Our paths are relative to `crates/gpui-monaco/src/`.

### Findings

1. **Shift+Escape is unbound for suggest, snippets, linked editing and removeSecondaryCursors.** P1. Monaco gives `hideSuggestWidget`, `leaveSnippet`, `cancelLinkedEditingInput`, `removeSecondaryCursors` and `cancelSelection` the secondary key Shift+Escape (`contrib/suggest/browser/suggestController.ts:919`, `contrib/snippet/browser/snippetController2.ts:388`, `contrib/linkedEditing/browser/linkedEditing.ts:450`, `browser/coreCommands.ts:1784,1811`). We bind only `escape` (`actions.rs:1208`). `shift-escape` is bound only for parameter hints, the code action menu and the rename input (`actions.rs:1612,1626,1683`). Fix: bind `shift-escape` to `RemoveSecondaryCursors` (its handler already runs the whole Escape priority chain). Size XS.

2. **Left/Right close the suggest widget instead of refiltering.** P1. Monaco `SuggestModel._onCursorChange` refilters on explicit keyboard cursor moves (`contrib/suggest/browser/suggestModel.ts:371-375`). It cancels only when the cursor leaves the word or the line. `move_cursors` calls `hide_suggest` for every move other than Up/Down/PageUp/PageDown (`editor.rs:1745-1757`). Size S.

3. **Up/Down are swallowed when the list has a single, focused suggestion.** P1. In Monaco, `selectNextSuggestion`/`selectPrevSuggestion` require `suggestWidgetMultipleSuggestions || !suggestWidgetHasFocusedSuggestion` (`suggestController.ts:925,963`). Otherwise the key falls through to `cursorDown`/`cursorUp`, which moves the cursor and closes the widget. `suggest_navigate` wraps 0→0 and returns `true` (`completion.rs:1525-1561`). Size XS.

4. **Move Line Up/Down ignores `autoIndent`.** P1. `MoveLinesCommand` reindents the moved block with the language's indentation rules when `autoIndent` is `full` (the default) (`contrib/linesOperations/browser/moveLinesCommand.ts:120-230,367-387`; it is passed in at `linesOperations.ts:181-184`). Our `MoveLinesCommand` only moves text, and `_config` is unused (`cursor/lines.rs:47-160`). Size M.

5. **deleteWordLeft doesn't delete an auto-closed pair.** P1. `WordOperations.deleteWordLeft` deletes both characters when the cursor is between an auto-closed pair, e.g. `(|)` (`common/cursor/cursorWordOperations.ts:433-436`). Our `delete_word_left_range` says it omits this on purpose (`cursor/word.rs:601-603`). The language config is available through `CursorConfig` (`cursor/typing.rs:692`). Size S.

6. **The `occurrencesHighlightDelay` default is 250ms; Monaco's is 0.** P1. Monaco's default is 0 (`common/config/editorOptions.ts:6566-6568`), applied after a fixed 50ms `runDelayer` (`contrib/wordHighlighter/browser/wordHighlighter.ts:263,751`). We default to 250ms (`options.rs:1830,2075`) and wait that long before asking providers (`occurrences.rs:179-187`). Highlights therefore appear about 200ms later than in Monaco. Size XS.

7. **F7/Shift+F7 select the whole highlight instead of moving the caret, and start from the wrong place.** P1. Monaco `moveNext`/`moveBack` call `setPosition(dest.getStartPosition())` (a collapsed caret) and reveal the range. When the cursor is in no highlight (index -1), they go to the first highlight (next) or the last (prev) (`wordHighlighter.ts:428-465`). `go_to_word_highlight` selects the whole range and picks the nearest highlight after or before the cursor (`occurrences.rs:288-311`). The screen-reader alert is also missing (see 15). Size XS.

8. **Cmd+D / Cmd+Shift+L don't work with the find widget the way Monaco's do.**
   - 8a. **P1.** Inside the find input, `secondary-d`, `secondary-shift-l` and `cmd-k cmd-d` run on the single-line input editor itself (`actions.rs:1228,1243`; handlers at `editor.rs:4609-4621` don't propagate for `single_line`). In Monaco these have kbExpr `editorFocus`, which includes the find widget. They act on the main editor, and the find widget owns the search text when the editor text isn't focused (`contrib/multicursor/browser/multicursor.ts:279-303,733,776,807`). Size S.
   - 8b. **P1.** `select_all_occurrences` uses the find query whenever the widget is visible and mode is `findOptions`, even when the query isn't a regex (`multi_cursor.rs:365-379`). Monaco uses the find query only when `isRegex` is set, and also honours the find-in-selection scope (`searchScope`). Otherwise it uses the selection or session text (`multicursor.ts:657-668`). With find showing "foo" and "bar" selected, we select every "foo". Size XS.
   - 8c. **P2.** Starting a Cmd+D session doesn't seed the find state. Monaco sets `searchString` to the session text and applies whole-word, match-case and no-regex overrides while the session lasts (`multicursor.ts:513-519,550-557`). F3 and the find widget see it. Nothing in `multi_cursor.rs:292-318` touches `self.find`. Size S.

9. **The selection highlighter ignores whole-word from a Cmd+D session.** P1. `SelectionHighlighter._createState` reuses the active session's `wholeWord` and `matchCase` (`multicursor.ts:975-988,1028`). After Cmd+D on an empty selection, only whole-word occurrences are highlighted. We hard-code "never whole-word here" (`occurrences.rs:367-369`), so substrings such as `foobar` also light up. Size XS.

10. **Many in-scope command ids can't be resolved by `trigger()` or `addKeybindingRule`.** P2. None of these are in `BUILTINS` or `COMMANDS` (`editor_actions.rs:216-528`). The behaviours exist internally, but `CodeEditor::trigger` cannot reach them by Monaco id.
    - Suggest: `acceptSelectedSuggestion` (plus its alias `acceptSelectedSuggestionOnEnter`), `hideSuggestWidget`, `insertBestCompletion`, `insertNextSuggestion`, `insertPrevSuggestion`. The behaviour is in `completion.rs:643,715` and `editor.rs:2125-2150`.
    - Snippet: `jumpToNextSnippetPlaceholder`, `jumpToPrevSnippetPlaceholder`, `leaveSnippet`, `acceptSnippet` (`snippet_session.rs:899-940`). Choice completions already run `jumpToNextSnippetPlaceholder` internally (`snippet_session.rs:28`).
    - Rename: `acceptRenameInput`, `acceptRenameInputWithPreview`, `cancelRenameInput`, `focusNextRenameSuggestion`, `focusPreviousRenameSuggestion` (these are GPUI actions only, `actions.rs:1620-1628`).
    - Other: `cancelLinkedEditingInput`, `leaveEditorMessage`, `editor.action.unicodeHighlight.showExcludeOptions` (`unicode_highlighter.rs:317`).
    - Word navigation: `cursorWordAccessibilityLeft`, `cursorWordAccessibilityLeftSelect`, `cursorWordAccessibilityRight` and `cursorWordAccessibilityRightSelect`. The navigation type exists (`cursor/word.rs:129`) but no command or action is registered.
    - Aliases: `editor.action.smartSelect.grow` (`smartSelect.ts:176`), `cursorWordPartStartLeft` and `cursorWordPartStartLeftSelect` (`wordPartOperations.ts:92,111`).
    - Also missing: Monaco's Windows binding that gives Ctrl+Left/Right to `cursorWordAccessibility*` when accessibility mode is on (`wordOperations.ts:131,170,229,268`).

    Size S.

11. **Contrib context keys aren't exposed as key-context identifiers.** P2. Host keybinding rules can only test what `render` adds to the key context (`editor.rs:4041-4095`). These Monaco keys have no equivalent:
    - Message and word highlights: `messageVisible`, `hasWordHighlights`.
    - Snippets: `inSnippetMode`, `hasNextTabstop`, `hasPrevTabstop`, `inSnippetChoice`.
    - Linked editing and rename: `LinkedEditingInputVisible`, `renameInputVisible` (on the editor) and `renameInputFocused`.
    - Suggest: `suggestWidgetHasFocusedSuggestion`, `suggestWidgetDetailsVisible`, `suggestWidgetMultipleSuggestions`, `suggestionMakesTextEdit`, `acceptSuggestionOnEnter`, `suggestionHasInsertAndReplaceRange`, `suggestionInsertMode`, `suggestionCanResolve`, `atEndOfWord`, `hasOtherSuggestions`.
    - Peek: `inReferenceSearchEditor`.

    The definitions are at `contrib/suggest/browser/suggest.ts:34-42`, `snippetController2.ts:59-62`, `messageController.ts:30`, `wordHighlighter.ts:44`, `linkedEditing.ts:39`, `renameWidget.ts:56-57` and `peekView.ts:60`. Size M.

12. **Some preconditions are looser than Monaco's.** P3. Monaco's preconditions and ours:
    - `editor.action.triggerSuggest`: Monaco requires `writable && hasCompletionItemProvider && !suggestWidgetVisible` (`suggestController.ts:804`). We use `writable` (`editor_actions.rs:325`).
    - `editor.action.wordHighlight.next`/`prev`: Monaco requires `hasWordHighlights` (`wordHighlighter.ts:971,986`). We use `always` (`editor_actions.rs:261-262`).
    - `editor.action.quickOutline`: Monaco requires `hasDocumentSymbolProvider` (`standalone/browser/quickAccess/standaloneGotoSymbolQuickAccess.ts:49`). We use `always` in both the palette and the context menu (`editor_actions.rs:218`, `context_menu.rs:103`). Ours has a `tags.scm` fallback, so the right check is "has a provider or a tags query".
    - `editor.action.linkedEditing`: Monaco requires `writable && hasRenameProvider` (`linkedEditing.ts:403`). We use `writable`.
    - `editor.action.changeAll`'s Cmd/Ctrl+F2 binding: it runs `SelectAllOccurrences`, which has no writable check (`actions.rs:1244`).

    These mostly affect what the command palette lists. Size XS.

13. **Parameter hints have no 120ms debounce.** P3. Monaco's `ParameterHintsModel` delays every trigger by `DEFAULT_DELAY = 120` (`contrib/parameterHints/browser/parameterHintsModel.ts:52,74`). `trigger_signature_help` asks providers at once on every cursor and content change (`signature_help.rs:257-300`, `editor.rs:1636-1640,2097-2099`). Size XS.

14. **The word highlighter ignores focus.** P3. Monaco computes highlights only with text focus. It re-runs on `onDidFocusEditorText` and clears them on `onDidBlurEditorWidget` when no other editor takes focus (`wordHighlighter.ts:308-316,367-379,674-679`). `update_word_highlights` has no focus check, highlights stay after blur, and nothing triggers on focus (`occurrences.rs:136-170`). Size S.

15. **Several screen-reader alerts are missing.** P3. Monaco alerts in these places; `announce` (`accessibility.rs:183`) is called only for tab focus mode (`editor.rs:4795`):
    - Cursor added or removed by the multicursor commands: `announceCursorChange` (`multicursor.ts:32-37,94,146,719`).
    - The F7 position, "line, i of n for 'word'" (`wordHighlighter.ts:440`).
    - Rename success (`rename.ts:317`).
    - Every inline message (`messageController.ts:64`).

    Size S.

16. **The sticky scroll context menu is missing.** P3. Right-clicking sticky lines opens `MenuId.StickyScrollContext`, which has Toggle Editor Sticky Scroll (`stickyScroll/browser/stickyScrollController.ts:411-419`, `stickyScrollActions.ts:39`). `on_right_mouse_down` opens the menu only over content text (`editor.rs:2985-2992`). Size S.

17. **The suggest status bar is static text.** P3. Monaco's status bar is an `ActionBar` with clickable Insert/Replace and Show More/Less actions (`suggest/browser/suggestWidgetStatus.ts:23-27`, menu items at `suggestController.ts:859-902,1025-1035`). Show More appears when `suggestionCanResolve`. We render plain strings, with Show More based on `can_expand` (`suggest_widget.rs:611-650`). Size S.

18. **Go to Offset (`::`) has no 0-based/1-based toggle.** P3. Monaco's picker has an offset-mode button and remembers the choice in storage (`contrib/quickAccess/browser/gotoLineQuickAccess.ts:27-44,82-125,173-180`). `goto_offset_item` is 1-based only (`quick_access.rs:849`). Size S.

19. **The internal `_execute*` API commands are missing.** P3. These are the in-scope ones; none resolves in the port:
    - `_executeLinkProvider` (`links/browser/getLinks.ts:171`)
    - `_executeSignatureHelpProvider` (`parameterHints/browser/provideSignatureHelp.ts:47`)
    - `_executeSelectionRangeProvider` (`smartSelect.ts:306`)
    - `_executeCompletionItemProvider` (`suggest.ts:390`)
    - `_provideDocumentSemanticTokens(Legend)` and `_provideDocumentRangeSemanticTokens(Legend)` (`semanticTokens/common/getSemanticTokens.ts:141-232`)

    Size M (needs a typed-args command registry).

20. **Not planned or not applicable.**
    - `gpu` (`editor.action.debugEditorGpuRenderer`) and `inlineProgress` are listed as not planned in MONACO_PARITY.
    - `standalone/browser/iPadShowKeyboard` is DOM/iPad-only, so it has no equivalent.
    - `standalone/browser/referenceSearch` only wires up peek for standalone; the gotoSymbol audit covers it.

### Checked and matching

- **insertFinalNewLine:** id, writable precondition, always inserts the EOL at document end with the primary selection kept (`editor.rs:2173`).
- **lineSelection:** `expandLineSelection` on Cmd/Ctrl+L.
- **linesOperations:** every id resolves. Keybindings match per platform (copy lines on mac/linux/win, move lines, Cmd+Shift+K, Cmd/Ctrl+Enter, Cmd/Ctrl+Shift+Enter, Cmd+]/[, mac Cmd+Backspace/Ctrl+K/Cmd+Delete/Ctrl+J, Cmd+K Cmd+X). Behaviour matches: sort with collator, whole-document rule, duplicate selection, case transforms. Menubar-only groups need nothing.
- **linkedEditing:** Cmd/Ctrl+Shift+F2, Escape cancels, adaptive debounce, word pattern, `linkedEditing` default false.
- **links:** `editor.action.openLink` opens at each selection's end. Hover label follows `multiCursorModifier`. Link resolve, `file://./` relative links. Failures only log, which matches standalone's notification service.
- **longLinesHelper:** a click on text at or past `stopRenderingLineAfter` turns it off.
- **message:** Escape closes. Closes on cursor move, model change, blur and mouse moving out of the ±3-line box.
- **middleScroll:** `scrollOnMiddleClick` default false. Session ends on mouse down, key down, or release after scrolling. Anchor dot drawn.
- **multicursor (rest):** insertCursorAbove/Below use logical lines by default (correct). Keybindings match on mac, linux (primary and secondary) and win. Shift+Alt+I, Cmd+D/Cmd+K Cmd+D/Cmd+Shift+L/Cmd+F2. Find wraps around. Context menu "Change All Occurrences" is `1_modification` 1.2. focusNext/PreviousCursor, addCursorsToTop/Bottom.
- **parameterHints:** trigger/close/next/prev ids. Cmd/Ctrl+Shift+Space. Up/Down/Alt+Up/Down and mac Ctrl+P/N with suggest priority. Escape and Shift+Escape. Cycling with `cycle`. A mouse cursor change closes; other changes re-query. Trigger and retrigger characters.
- **peekView / zoneWidget:** resizable frame, preview editor, list keys (in the gotoSymbol audit's scope for details).
- **placeholderText:** shows while the model is empty, uses the placeholder colour.
- **quickAccess (contrib and standalone):** `:`, `::`, `@`, `@:`, `>` and the help list. Negative lines and columns, separators `,:#`. Ctrl+G on every platform, F1, Cmd/Ctrl+Shift+O. Context menu "Command Palette" is `z_commands` 1 and "Go to Symbol..." is `navigation` 3. Recently-used section and keybinding labels.
- **readOnlyMessage:** the `readOnlyMessage` option, else "Cannot edit in read-only editor"/"input", shown at the cursor.
- **rename:** F2 with writable and rename-provider precondition, context menu `1_modification` 1.1. Resolve-location chain with rejects and the word fallback. Enter, Cmd/Ctrl+Enter preview, Escape/Shift+Escape, Up/Down suggestions.
- **sectionHeaders:** region and MARK headers, default regex, MARK limited to comments, font size 9 and letter spacing 1.
- **semanticTokens:** document and viewport providers with adaptive debounces, deltas, `semanticHighlighting` configured by theme.
- **smartSelect:** expand/shrink keys (mac Ctrl+Cmd+Shift plus the Ctrl+Shift secondary, win/linux Shift+Alt), `selectLeadingAndTrailingWhitespace` and `selectSubwords` options.
- **snippet:** Tab and Shift+Tab via tab stops (`editor.rs:2143,2206`), Escape leaves, choice completion runs the next placeholder.
- **stickyScroll:** toggle, focus, select next/previous, go to the focused line, select editor (Down/Up/Enter/Escape when focused). Defaults enabled, 5 lines, outline model, scroll with editor.
- **suggest (rest):** trigger keys (mac Ctrl+Space, Alt+Escape and Cmd+I; others Ctrl+Space and Ctrl+I). toggleSuggestionDetails, focusSuggestion, toggleExplainMode, toggleSuggestionFocus. Shift+Enter/Shift+Tab alternative accept. Secondary Cmd/Ctrl+Up/Down/PageUp/PageDown and mac Ctrl+N/P. `suggestWidgetCopy`, including Ctrl+Insert on Windows. Status bar labels, resizing, `resetSuggestSize`.
- **symbolIcons:** `symbolIcon.*Foreground` colours used.
- **toggleTabFocusMode:** Ctrl+M, mac Ctrl+Shift+M. Global toggle with alert.
- **tokenization:** `editor.action.forceRetokenize`.
- **unicodeHighlighter:** three disable commands, exclude-options quick pick with the same entries as Monaco (locales, char, comments/strings, kind), banner.
- **unusualLineTerminators:** auto/off/prompt, read-only skip, ignore per model.
- **wordOperations:** cursor and delete word commands and their kinds (WordStartFast left, WordEnd right). Mac Alt and others Ctrl bindings, `deleteInsideWord`.
- **wordPartOperations:** mac Ctrl+Alt bindings for move, select and delete.
- **standalone inspectTokens and toggleHighContrast:** ids, toggle back to the integrator's theme.


## G. Options


Scope: all 179 `register(...)` entries in `vendor/vscode/src/vs/editor/common/config/editorOptions.ts:6030-6989`, every nested option class, `diffEditorDefaultOptions` (`common/config/diffEditor.ts`) and `validateDiffEditorOptions`, `EDITOR_MODEL_DEFAULTS` and `TextModelResolvedOptions`, plus `IGlobalEditorOptions` and `IStandaloneEditorConstructionOptions`.
Ours: `crates/gpui-monaco/src/options.rs`, `diff_editor.rs:133-280`, `model.rs:124-167`.

### What checks out
- Every non-DOM registered option has a field with matching enum values and defaults, except the items below. These defaults were checked: autoClosing*, autoIndent=full, autoSurround, cursor*, find.*, hover.*, minimap.*, scrollbar.*, stickyScroll.*, guides.*, bracketPairColorization.*, inlayHints.*, parameterHints.*, gotoLocation.*, unicodeHighlight.*, inlineSuggest.* and edits.*, suggest.* (the ones present), quickSuggestions, dropIntoEditor/pasteAs, smartSelect, comments, padding, multiCursorPaste, snippetSuggestions, suggestSelection, wrappingIndent and wrappingStrategy.
- A script grepped each `EditorOptions` and `DiffEditorOptions` field for a read outside `options.rs`. Every field is read somewhere that affects behavior. The `gotoLocation.multiple*` and `alternative*` fields are read through `GotoLocationOptions::multiple()` and `alternative()`. The only field with no reader is `minimap.section_header_letter_spacing`, which is blocked like letterSpacing #232.
- Computed options: `effectiveCursorStyle` (overtype) is in `element.rs:2994`. `effectiveFullwidthCharacterWidth` checks for a monospace font (`element.rs:135`). With accessibilitySupport, wrapping uses advanced wrapping with no indent and is forced at the viewport (`element.rs:1489-1500`, `options.rs:2160`). `emptySelectionClipboard`, `tabFocusMode` (plus a global) and `pixelRatio` (`window.scale_factor`) also check out.
- Diff editor: every field is wired. `splitViewDefaultRatio` is clamped to 0.1–0.9 at render (`diff_editor.rs:3619`).
- Model: `detectIndentation`, `trimAutoWhitespace`, `largeFileOptimizations` and `defaultEOL` are all honored (`model.rs:296-315`, `905`).

### Findings

1. **`occurrencesHighlightDelay` default is 250ms; Monaco's is 0.** P1.
   Monaco `editorOptions.ts:6567` registers default `0` (range 0–2000). `wordHighlighter.ts:263` adds only a 50ms run delayer, and `:796` uses the delay as a minimum render time after the cursor moves.
   Ours: `options.rs:2075` (`from_millis(250)`), applied as a sleep before computing in `occurrences.rs:179-187`. Highlights show about 200ms later than in Monaco. Fix XS (default 0 plus a 50ms debounce).

2. **No option validation layer, so Monaco's clamps and sentinel values are lost.** P1.
   Monaco validates every option in `validate()`. `EditorOptions` is a plain struct, and `CodeEditor::update_options` (`editor.rs:818`) stores it unchanged. Only `word_wrap_column.max(1)` (`element.rs:1515`), `multi_cursor_limit.max(1)` (`editor.rs:1609`) and `long_distance_hint_context_line_count.min(10)` are clamped. Missing behaviors:
   - `fontSize`: clamp 6–100, and 0 means the default (`editorOptions.ts:2148-2154`). Ours uses it raw (`element.rs:119`).
   - `lineHeight`: 0 means auto, a value below 8 multiplies `fontSize`, the result is rounded with a minimum of 8 and a maximum of 150 (`editorOptions.ts:3383`, `fontInfo.ts:35-45`). Ours: `resolved_line_height` (`options.rs:2177`) returns `Some(px(x))` verbatim, so `Some(px(1.5))` gives 1.5px lines and `Some(px(0))` gives a zero line height that the layout divides by (`element.rs:1123`).
   - `minimap.scale`: integer 1–3; `maxColumn` 1–10000; `sectionHeaderFontSize` 4–32 (`editorOptions.ts:3614-3620`). Ours `element.rs:1465` and `:2337` take any f32.
   - `stickyScroll.maxLineCount`: 1–20 (`:3238`). Ours `element.rs:2593` is raw.
   - `fastScrollSensitivity`: a value of 0 or less becomes 5 (`:6374`). `mouseWheelScrollSensitivity`: 0 becomes 1 (`:6495`). Ours `editor.rs:3691` is raw, so 0 disables wheel scrolling.
   - `foldingMaximumRegions`: 10–65000 (`:6407`). `colorDecoratorsLimit`: 1–1e6 (`:6233`). `lineNumbersMinChars`: 1–300 (`:6471`). `codeLensFontSize`: 0–100 (`:6213`). `padding.*`: 0–1000. `hover.delay`: 0–10000. `hover.hidingDelay`: 0–600000. `inlineSuggest.minShowDelay`: 0–10000. `revealHorizontalRightPadding`: 0–1000. `overviewRulerLanes`: 0–3.

   Fix M: add `EditorOptions::validated()` and apply it in `update_options` and `zoomed_options`.

3. **`tabSize` and `indentSize` aren't clamped to at least 1, and 0 panics.** P1.
   Monaco `model.ts:592-597` (`TextModelResolvedOptions`) applies `Math.max(1, …)`.
   Ours: `model.rs:494` (`update_options`) and `with_options` store 0. `cursor_config` (`editor.rs:1302`) passes it raw to `cursor/strings.rs:220` `next_render_tab_stop` (`visible_column % tab_size`), to the minimap at `element.rs:4569` and to `cursor/comment.rs:413`. A tab on a line divides by zero. Some call sites guard with `.max(1)` (`element.rs:559`, `comment.rs:427`, `colorize.rs:320`), but not all. Fix XS.

4. **`indentSize: 'tabSize'` mode is missing.** P3.
   Monaco `model.ts:593-596`, plus `textModel.ts:210` (detection sets it) and `:692` (`updateOptions` keeps it in sync). Ours: `ModelOptions.indent_size: u32` (`model.rs:128`), so changing `tab_size` later doesn't move `indent_size` along with it. Fix S.

5. **`cursorWidth` semantics differ.** P3.
   Monaco default is 0, which renders 2px, and the width is capped at the typical half-width character width (`editorOptions.ts:6302`, `viewCursor.ts:68,185`). Ours defaults to `px(2.)` and has no cap or 0 fallback (`element.rs:3029`), so a host that passes Monaco's 0 gets an invisible caret. Fix XS.

6. **The layout computation deviates from `EditorLayoutInfoComputer`.** P1 (visible layout).
   - The folding column is `line_height.min(18)` whenever `folding` is on (`element.rs:1453-1457`). Monaco adds a fixed 16px, and only when `folding && showFoldingControls !== 'never'` (`editorOptions.ts:2954-2959`). With `showFoldingControls: Never` we reserve an empty column that Monaco doesn't have.
   - The glyph margin is a constant 18px (`element.rs:24`, `:1448`). Monaco uses `lineHeight * glyphMarginDecorationLaneCount` (`:2969`).
   - The minimap width is `max_column * scale`, capped at width/4 (`element.rs:1465`). Monaco computes it from `minimapCharWidth = scale/pixelRatio` with the proportional share `((remaining - vbar - 2) * charWidth) / (typicalHalfwidth + charWidth)`, plus `MINIMAP_GUTTER_WIDTH` (`:2811`, `:2897-2898`). That share is about 1/8 of the width, so in narrow editors our minimap is about twice Monaco's width. `minimap.size` fill/fit also changes Monaco's width multiplier (`:2869`), which we ignore.
   - `lineDecorationsWidth` doesn't accept the `"Nch"` string form (`:3359-3370`); ours is `Pixels` only.

   Fix M.

7. **`renderFinalNewline` default isn't platform-specific.** P3.
   Monaco `:6636` uses `isLinux ? 'dimmed' : 'on'`. Ours `options.rs:2070` is always `On`. Fix XS.

8. **`defaultEOL` default isn't platform-specific.** P3.
   Monaco `modelService.ts:69` uses `DEFAULT_EOL = (Linux||Mac) ? LF : CRLF` for standalone models. Ours `model.rs:164` is always LF, so Windows behavior differs. Fix XS.

9. **Missing suggest options that the previous audit listed as fixed.** P2.
   - `suggest.fitWidthToDetails` (`editorOptions.ts:5183`, `suggestWidget.ts:225,862`): missing.
   - `suggest.shareSuggestSelections` (`:5176`, `suggestMemory.ts:277`): missing. `MONACO_PARITY.md:136` admits this one.

   `MONACO_AUDIT.md:91` lists both. Fix S each.

10. **Missing diff option `hideOriginalLineNumbers`.** P2.
    Monaco `editorOptions.ts:931`, `diffEditor.ts:40`, `diffEditorOptions.ts:97-98` (also implied by compactMode). `MONACO_AUDIT.md:143` lists it as fixed together with compactMode/diffWordWrap, but `DiffEditorOptions` (`diff_editor.rs:133-157`) has no such field. `isInEmbeddedEditor` (`diffEditorWidget.ts:265`) and `originalAriaLabel`/`modifiedAriaLabel` (`editorBrowser.ts:596-601`) are also missing. Fix S.

11. **Missing `IGlobalEditorOptions` / standalone options.** P2.
    - `stablePeek` (`standaloneCodeEditor.ts:146`; keeps peek open on Escape or double-click): missing.
    - `autoDetectHighContrast` (`:164,197`, default true; switches to the hc theme when the OS is in high-contrast mode): there's no OS high-contrast detection in `theme.rs`.
    - `wordBasedSuggestionsOnlySameLanguage` (`:133`, deprecated; covered by `MatchingDocuments`): P3.
    - `accessibilityHelpUrl` (`:204`): moot, because there's no accessibility help action at all.
    - `model`/`value`/`language`/`theme` construction options map to `CodeEditor::new(model)`, `TextModel` and `set_theme`. That's fine.

    Fix S (stablePeek), M (high contrast).

12. **inlineSuggest sub-options missing.** P3.
    `triggerCommandOnProviderChange` and `experimental.{suppressInlineSuggestions, showOnSuggestConflict, emptyResponseInformation}` (`editorOptions.ts:4612-4617`) are absent from `InlineSuggestOptions` (`options.rs:871`). The top-level `inlineCompletionsAccessibilityVerbose` (`:6789`) is also missing. Fix S.

13. **gotoLocation leftovers.** P3.
    The deprecated `multiple` fallback and `alternativeTestsCommand` (`editorOptions.ts:2242,2254`) are missing. `multiple_tests` exists but nothing reads it internally; it's documented as host-only. Fix XS.

14. **API-shape divergences.** P3, documentation only.
    - `suggest.show*` (27 booleans) are folded into `SuggestOptions.hidden_kinds: Vec<CompletionItemKind>` (`options.rs:1046`, honored at `completion.rs:507`). Monaco's `showColors` hides both Color and Customcolor (`suggestModel.ts:706-709`), so a host has to know to list both.
    - Monaco's top-level `suggestFontSize`/`suggestLineHeight` (`:6792`) are `suggest.font_size`/`line_height` here. They behave the same (`completion.rs:997-1012`).
    - `scrollbar.verticalScrollbarSize`/`horizontalScrollbarSize` are top-level `EditorOptions` fields.
    - The model-level `bracketPairColorizationOptions` (`textModelDefaults.ts:12`) lives only on `EditorOptions`.

    Fix XS (docs).

15. **Other registry entries that are absent, with no behavior impact today.** P3.
    - `tabIndex` (`:6832`): GPUI focus order; small.
    - `renameOnType` (`:6628`): deprecated alias of linkedEditing.
    - `inDiffEditor` (`:6455`): internal context key.
    - `disableMonospaceOptimizations`, `renderRichScreenReaderContent`, `experimentalWhitespaceRendering` (svg/font/off), `experimentalGpuAcceleration`, `allowOverflow`.
    - `allowVariableFonts` and `allowVariableFontsInAccessibilityMode` (`:6065`): moot until per-decoration fonts exist (PARITY #87). Only injected text carries fonts (`element.rs:689`).

    Fix XS each if wanted.

### Skipped (as instructed)
- DOM-only: automaticLayout, useShadowDOM, fixedOverflowWidgets, extraEditorClassName, editorClassName (computed), overflowWidgetsDomNode, ariaContainerElement, dimension, editContext and effectiveEditContextEnabled, domReadOnly, disableLayerHinting.
- Blocked: #232 letterSpacing (including `minimap.sectionHeaderLetterSpacing`, which is declared at `options.rs:224` but has no reader), #233 fontVariations, #231 copyWithSyntaxHighlighting.


## H. Theme colors and built-in themes


Scope and method:
- Editor-side `registerColor` ids and their defaults were pulled from `vendor/vscode/src/vs/editor/**` (179 ids, excluding tests) with a throwaway extraction script (not committed).
- `vendor/vscode/src/vs/platform/` is **not vendored**. The vendor clone is blobless, so `git cat-file` fails. Platform defaults (`baseColors`, `editorColors`, `inputColors`, `listColors`, `menuColors`, `minimapColors`, `miscColors`, `quickpickColors`) were read from upstream at the vendored commit `53642551d756f7aefec1059e1b3961d3d2cc5adc`, under `src/vs/platform/theme/common/colors/*.ts`. The values used below were spot-checked against the source text.
- Every key (about 360) was grepped in `crates/gpui-monaco/src` (`grep_result.txt`). Mapping from VS Code theme JSON = `EditorColors::apply_vscode_colors` (theme.rs:1341-1799). Rendering use = grep for field usage outside theme.rs.
- Skipped as workbench-only: activityBar*, sideBar*, statusBar*, tab*, terminal*, panel*, titleBar*, notification*, breadcrumb*, merge.*, editorOverviewRuler.{current,incoming,common}ContentForeground, problems*Icon, tree.*, sash, progressBar, activity*Badge, charts, scm/debug/testing. Badge, sideBar and tab colors appear only where an editor default derives from them.

---

1. **Token rules do not inherit from their parent rule, and the `''` default rule is ignored.** P1. Fix: M
   - Monaco: tokenization.ts:385-410 (`insert` clones the parent's rule), :300-315 (`acceptOverwrite` only overwrites set fields; `fontStyle` NotSet means -1), :121-132 (`''` rules set the default fg/bg/fontStyle), standaloneThemeService.ts:142-153 (`editor.foreground`/`editor.background` become a `''` rule).
   - Ours: theme.rs:2368-2378 `best_match` returns the whole style of the longest selector. theme.rs:1834-1861 `TokenStyle` has no "unset" font style (bools only). theme.rs:2363 `selector_matches("", x)` is never true.
   - Effects:
     - A rule with only `fontStyle` (e.g. user `{token:'comment.doc', fontStyle:'italic'}`, or Monaco's own `metatag.php` bold) loses the parent's color and falls back to the editor foreground.
     - A color-only child rule drops the parent's bold/italic.
     - An explicit `fontStyle: ''` cannot reset an inherited style.
     - Duplicate selectors replace each other instead of merging field by field.
     - `{token:''}` rules do nothing.
   - The same longest-match-wins logic applies to TextMate `tokenColors` and to semantic_theme.rs:301.

2. **Language-specific Monaco rules never apply to tree-sitter captures.** P1. Fix: S
   - Monaco: tokens carry the language suffix (`keyword.json`, `string.value.json`, `delimiter.html`, `string.yaml`, `attribute.value.html`), matched by the trie (tokenization.ts:361-383).
   - Ours: theme.rs:2271-2286 `resolve()` matches only the language-less `monaco_token_for_capture` token. theme.rs:2416 `monaco_token_type` builds `<token>.<lang>` for `tokenize` but not for styling, and `style_for(HighlightId)` (theme.rs:2241) has no language.
   - The port's own rules `delimiter.html`, `delimiter.xml`, `keyword.json` and `string.value.json` (theme.rs:1993-2006, 2027-2038, 2096-2100) are therefore dead for the bundled JSON/HTML/CSS/YAML grammars. Only `string.special.key` is special-cased.
   - Examples:
     - JSON `true`/`false`/`null`: vs-dark `569CD6` instead of `CE9178`; vs `0000FF` instead of `0451A5`.
     - JSON string values in vs: `A31515` instead of `0451A5`.
     - HTML punctuation in vs: `000000` instead of `383838`.
     - HTML attribute values in vs: `A31515` instead of `0000FF`.
     - YAML strings in vs: `A31515` instead of `0451A5`.

3. **Built-in themes omit Monaco's language-specific token rules.** P2. Fix: XS
   - Monaco: themes.ts:35-68 (vs), 108-139 (vs-dark), 177-202 (hc-black), 237-261 (hc-light).
   - Ours: theme.rs:1978-2107. The omission is documented at theme.rs:8-9.
   - Missing rules:
     - `tag.id.pug`, `tag.class.pug`
     - `meta.scss`
     - `metatag.content.html`, `metatag.html`, `metatag.xml`, `metatag.php`
     - `attribute.value.number`, `attribute.value.unit`, `attribute.value.html`, `attribute.value.xml` (vs)
     - `attribute.value.number.css`, `attribute.value.unit.css`, `attribute.value.hex.css` (vs-dark)
     - `string.html`, `string.sql`, `string.yaml`
     - `keyword.flow.scss`
     - `operator.scss`, `operator.sql`, `operator.swift`
     - `predefined.sql`
   - These matter for user Monarch tokenizers (monaco-languages definitions emit exactly these tokens) and once #2 is fixed.

4. **vs `editor.selectionHighlightBackground` is wrong.** P1. Fix: XS
   - Monaco: themes.ts:76 sets `#ADD6FF4D`.
   - Ours: theme.rs:971 uses `0xadd6ff80` (the registry's lessProminent-style value).

5. **`editorWidget.border` defaults are stale (and the suggest border with them).** P1. Fix: XS
   - Monaco (upstream platform `editorColors.ts`): `transparent(editorWidget.foreground, 0.2)`. That is `#CCCCCC33` dark and `#61616133` light (hc: contrastBorder ✓).
   - Ours: `454545` (theme.rs:848, 854) and `c8c8c8` (theme.rs:991, 997).
   - Also not derived when a theme sets `foreground` or `editorWidget.foreground`: theme.rs:1680-1681 copies only explicit keys. Every widget frame is affected (hover, find, rename, suggest, quick access, context menu, peek).

6. **`menu.separatorBackground` defaults are stale, and context-menu separators use the wrong color.** P1. Fix: XS
   - Monaco (upstream `menuColors.ts`): `transparent(foreground, 0.2)`, i.e. `#CCCCCC33` / `#61616133`.
   - Ours: `606060` (theme.rs:766) and `d4d4d4` (theme.rs:909).
   - context_menu.rs:305 draws separators with `widget_border`. Only diff_editor.rs:2833 uses `menu_separator_background`.

7. **`editorInfo.foreground` is stale and unmapped, so peek border, banner icon and info markers are off.** P1. Fix: S
   - Monaco (upstream `editorColors.ts`): `editorInfo.foreground` is `#59a4f9` (dark/hcDark) and `#0063d3` (light/hcLight). `peekView.border` defaults to it for dark and light (peekView.ts:283). `editorInfo.border` in hc is `#59a4f9cc` / `#292929`.
   - Ours:
     - `peek_view_border` is `3794ff` (theme.rs:812) and `1a85ff` (theme.rs:955).
     - `banner_icon_foreground` is `3794ff` / `1a85ff` (theme.rs:879, 1022, 1182, 1336).
     - markers.rs:97 uses `3794ff`.
   - `editorInfo.foreground` is not read by `apply_vscode_colors`, so `peekView.border` doesn't follow it.

8. **Marker colors are hard-coded dark-theme values, ignoring theme and all related keys.** P1. Fix: M
   - Monaco defaults:
     - `editorError.foreground`: `#F14C4C` / `#E51400` / `#F48771` / `#B5200D`
     - `editorWarning.foreground`: `#CCA700` / `#BF8803` / `#FFD370` / `#895503`
     - `editorInfo.foreground`: see #7
     - `editorHint.foreground`: `#eeeeeeb3` / `#6c6c6c`
     - hc `editor{Error,Warning,Info,Hint}.border`
     - `editorOverviewRuler.{error,warning,info}Foreground` (editorColorRegistry.ts:75-77, error `#FF1212B3`)
     - `editorMarkerNavigation{Error,Warning,Info}.background` / `.headerBackground` and `editorMarkerNavigation.background` (gotoErrorWidget.ts:404-413; hc uses contrastBorder; hcDark warning header `#0C141F`; hcLight warning header 0.2)
   - Ours:
     - markers.rs:93-99 has fixed `f14c4c`/`cca700`/`3794ff`/`eeeeee`. Light themes get dark-theme squiggles, and the hint has no 0.7 alpha.
     - markers.rs:208-218 reuses the squiggle color for the overview ruler.
     - markers.rs:466-469 derives the zone frame and header from the same fixed color.
     - None of these keys are mapped or stored.

9. **vs `peekViewTitleDescription.foreground` is nearly invisible.** P1. Fix: XS
   - Monaco: peekView.ts:282, light `#616161`.
   - Ours: theme.rs:958 uses `rgba(0x00616161)`, which decodes to RGB `#006161` at alpha `0x61` (about 38%), i.e. a translucent teal. It should be `rgb(0x616161)`.

10. **Several small default mismatches in built-in themes.** P1. Fix: XS
    - vs `editorCodeLens.foreground`: Monaco `#919191` (editorColorRegistry.ts:55); ours `999999` (theme.rs:936).
    - vs `editor.snippetTabstopHighlightBackground`: Monaco `rgba(10,50,100,0.2)` = `#0A326433`; ours `0x0a32641a` (theme.rs:992).
    - vs `inputOption.activeBackground`: Monaco `transparent(focusBorder #0090F1, 0.2)` = `#0090F133`; ours `007fd433` (theme.rs:1004), which uses the dark focus border.
    - vs `minimapSlider.background` / `.hoverBackground`: Monaco is 0.5 × the scrollbar slider, i.e. `#64646433` / `#64646459`; ours `79797933` (theme.rs:1010).
    - vs `editor.findRangeHighlightBackground`: Monaco `#b4b4b44d`; ours `…4c` (theme.rs:926).
    - hc-light `multiDiffEditor.border`: Monaco `#cccccc` (multiDiffEditor/colors.ts:21); ours `0f4a85` (theme.rs:1223).
    - `inlineEdit.gutterIndicator.primaryBackground` light: Monaco alpha 0.5 (inlineEdits/theme.ts:75); ours always 0.4 (theme.rs:707).
    - `inlineEdit.modifiedBorder` light: Monaco `darken(diffInserted, 0.6)` (inlineEdits/theme.ts:142); ours plain diffInserted (theme.rs:695).
    - `inlineEdit.modifiedChangedLineBackground` hc: Monaco unscaled diffInsertedLine (inlineEdits/theme.ts:44); ours ×0.7 (theme.rs:679).
    - `editor.compositionBorder` (`#ffffff` / `#000000`): ours is `ime_underline = foreground` (theme.rs:1798), i.e. `D4D4D4` in vs-dark.

11. **Context menu uses widget colors instead of `menu.*`.** P1. Fix: S
    - Monaco: the standalone context menu uses the menu styles.
      - `menu.background` = dropdown.background: `#3C3C3C` / white / black / white.
      - `menu.foreground` = dropdown.foreground: `#F0F0F0` / `#616161` / white / `#292929`.
      - `menu.border` (hc contrastBorder) and `menu.selectionBorder` (hc activeContrastBorder).
    - Ours: context_menu.rs:290-296 uses `widget_background` (`252526`), `widget_border` and `widget_foreground`.
    - Missing keys: `menu.background`, `menu.foreground`, `menu.border`, `menu.selectionBorder`, `dropdown.*`.

12. **Markdown links and code spans in hover, parameter hints, suggest details and the marker zone use the wrong keys.** P1. Fix: S
    - Monaco uses `textLink.foreground` and `textCodeBlock.background`: hover.css:32, 76, 88; parameterHints.css:91, 110; suggest.css:384, 392; messageController.css:46.
      - `textLink.foreground`: `#3794FF` / `#006AB1` / `#21A6FF` / `#0F4A85`.
      - `textCodeBlock.background`: `#0a0a0a66` / `#dcdcdc66` / black / `#F2F2F2`.
    - Ours: hover.rs:722-723 uses `link: input_option_active_border` (`007acc`) and `code_background: input_background` (`3c3c3c` / white). markers.rs (`let link = colors.input_option_active_border`) does the same.
    - Missing keys: `textLink.foreground`, `textLink.activeForeground`, `textCodeBlock.background`, `textPreformat.foreground`/`.background`/`.border`, `textBlockQuote.background`/`.border`, `textSeparator.foreground`.

13. **List, quick-input and toolbar colors are hard-coded or borrowed from unrelated keys.** P1. Fix: M
    - Code action list:
      - Monaco: `editorActionList.focusBackground`/`.focusForeground`, which equal `list.activeSelectionBackground` `#04395E` / `#0060C0` with white text.
      - Ours: code_action.rs:584 uses `peek_view_result_selection_background` (`#3399ff33`), and the foreground is unchanged.
    - Rename candidates: rename.rs:271 uses the same peek key; Monaco uses list colors.
    - Quick access:
      - Highlight: Monaco uses `list.highlightForeground` / `quickInputList.focusHighlightForeground` (`#2AAAFF` / `#0066BF` / hc focusBorder). Ours is hard-coded `#0097fb` (quick_access.rs:407).
      - Focused row: ours follows `editorSuggestWidget.selectedBackground` (quick_access.rs:445) instead of `quickInputList.focusBackground`.
      - Keybindings: Monaco styles them with `keybindingLabel.*`; ours draws plain text at 0.8 opacity (quick_access.rs:465-473).
    - Find-widget toggle hover: ours is hard-coded `hsla(0,0,0.5,0.2)` (find_widget.rs:542). Monaco uses `inputOption.hoverBackground` (`#5a5d5e80` / `#b8b8b850`) or `toolbar.hoverBackground`.
    - Hover copy-button hover: ours uses `scrollbar_slider_hover` (hover.rs:425).
    - Missing keys:
      - `editorActionList.{background,foreground,focusBackground,focusForeground}`
      - `list.{activeSelectionBackground,activeSelectionForeground,activeSelectionIconForeground,inactiveSelectionBackground,hoverBackground,hoverForeground,focusBackground,focusForeground,focusOutline,invalidItemForeground,deemphasizedForeground}`
      - `quickInput.{background,foreground}`, `quickInputTitle.background`, `quickInputList.{focusForeground,focusIconForeground,focusHighlightForeground}`
      - `pickerGroup.{foreground,border}`
      - `keybindingLabel.{background,foreground,border,bottomBorder}`
      - `toolbar.{hoverBackground,hoverOutline,activeBackground}`
      - `inputOption.hoverBackground`
    - Only `list.highlightForeground`, `list.focusHighlightForeground` and `quickInputList.focus*` are read, and only as suggest-widget fallbacks (theme.rs:1710-1741).

14. **Hover widget keys are missing.** P2. Fix: S
    - Monaco: `editorHoverWidget.background`/`.foreground`/`.border` (default to the widget colors), `.statusBarBackground` (lighten 0.2 / darken 0.05), `.highlightForeground` (parameterHintsWidget.ts:367, = `list.highlightForeground`), and `editor.hoverHighlightBackground` (`#264f7840` / `#ADD6FF26` / hcDark `#ADD6FF26`).
    - Ours: hover.rs:336 and signature_help.rs draw `widget_*`, so a theme that sets `editorHoverWidget.*` has no effect. There is no status-bar color and no hover-range highlight color.

15. **High-contrast outline keys are missing; highlights are approximated with translucent fills.** P2. Fix: M
    - Monaco hc themes outline these instead of filling (their hc backgrounds are null). Missing keys:
      - `editor.findMatchBorder`, `editor.findMatchHighlightBorder`, `editor.findRangeHighlightBorder`
      - `editor.selectionHighlightBorder`
      - `editor.wordHighlightBorder`, `editor.wordHighlightStrongBorder`, `editor.wordHighlightTextBorder`
      - `editor.rangeHighlightBorder`, `editor.symbolHighlightBorder`
      - `peekViewEditor.matchHighlightBorder`
      - `menu.selectionBorder`, `list.focusOutline`, `editorSuggestWidget.focusOutline`, `toolbar.hoverOutline`
      - `diffEditor.border`, `editorStickyScroll.border`, `widget.border`
      - `input.border`, `button.border`
      - `editorError.border`, `editorWarning.border`, `editorInfo.border`, `editorHint.border`
    - Ours: the `outline_fill` approximation is documented at theme.rs:1030-1036 and 1190-1194. Even when a theme defines these keys, the borders are never drawn.

16. **Fields exist but the VS Code JSON loader never maps them; the minimap slider ignores hover.** P2. Fix: XS
    - Never read by theme.rs:1341-1799: `editorCodeLens.foreground`, `editor.linkedEditingBackground`, `editor.rangeHighlightBackground`, `diffEditor.move.border`, `diffEditor.moveActive.border`, `multiDiffEditor.headerBackground`, `multiDiffEditor.border`.
    - `minimap_slider_hover` is never rendered: element.rs:4027 always paints `minimap_slider`.
    - `minimapSlider.activeBackground` (0.5 × scrollbar active) is missing.
    - Deprecated aliases are not read:
      - `editorActiveLineNumber.foreground`, the default source for `editorLineNumber.activeForeground` (editorColorRegistry.ts:49-50).
      - `editorIndentGuide.background` / `.activeBackground` are handled (theme.rs:1660-1669).

17. **Derived defaults are not recomputed when a theme or `defineTheme` overrides the source color.** P2. Fix: M
    - Monaco resolves defaults lazily from references (colorRegistry `resolveDefaultColor`). Ours bakes them in, re-deriving only a few: gutter, sticky/minimap background, diff gutter, suggest from widget, inlay kinds, placeholder, unicode, symbol highlight, word-wrap indicator.
    - Not re-derived when the source key is set:
      - `editor.selectionBackground` → `editor.inactiveSelectionBackground` (×0.5 / ×0.7 hc), `editor.foldBackground` (×0.3), `editor.selectionHighlightBackground` (lessProminent)
      - `foreground` → `editorWidget.foreground`, `editorWidget.border`, `menu.separatorBackground`
      - `focusBorder` → `inputOption.activeBackground`, hc `list.highlightForeground`
      - `scrollbarSlider.*` → `minimapSlider.*`
      - `list.activeSelectionBackground`/`.activeSelectionForeground` → `menu.selection*`, `quickInputList.focusBackground`, banner
      - `badge.background` → `editorInlayHint.background`
      - `editorInfo.foreground` → `peekView.border`
      - `editorWhitespace.foreground` → deprecated indent guides
      - `peekViewEditor.background` → `peekViewEditorGutter.background`
      - `editorWidget.border` → `editorHoverWidget.border`
      - `editorCursor.foreground` → `editorMultiCursor.*`
    - A lazy resolution layer (key → `Option<Color>` plus a default expression) would fix all of these at once.

18. **VS Code hc themes start from non-hc defaults.** P2. Fix: XS
    - Monaco: theme type `hcDark` / `hcLight` uses the hc registry defaults.
    - Ours: theme.rs:2158-2173 picks `EditorColors::vs_dark()` / `vs()` for `type: hc`/`hcDark`/`hcLight`, so unset keys get non-hc colors. It should use `hc_black()` / `hc_light()` when `high_contrast` is set.

19. **Semantic tokens fall back differently from Monaco's standalone themes.** P2. Fix: XS
    - Monaco: standaloneThemeService.ts:163-176 always returns a style. An unmatched `type.modifiers` gets the root default (default foreground, all font styles false), and semanticTokensProviderStyling.ts:61-90 applies it, overriding the syntax color.
    - Ours: semantic_theme.rs:296-308 returns `None` when no Monaco rule matches, keeping the syntax color.
    - Only observable when semantic highlighting is explicitly enabled with a standalone theme.

20. **Editor-surface color keys are missing entirely, so these surfaces aren't themable.** P2. Fix: M
    - Cursor and line numbers: `editorCursor.background`, `editorMultiCursor.primary.{foreground,background}`, `editorMultiCursor.secondary.{foreground,background}`, `editorLineNumber.dimmedForeground`, `editor.inactiveLineHighlightBackground`.
    - Guides: `editorIndentGuide.background2-6` and `.activeBackground2-6` (ours has only level 1). `editorBracketPairGuide.background1-6` and `.activeBackground1-6`: element.rs:3146-3152 always uses the bracket color ×0.3, which is Monaco's fallback only (indentGuides.ts:282-289).
    - Brackets: `editorBracketMatch.foreground`.
    - Overview ruler: `editorOverviewRuler.background`, `editorOverviewRuler.rangeHighlightForeground`, `editorOverviewRuler.wordHighlightTextForeground`, `editor.wordHighlightTextBackground`, `diffEditorOverview.insertedForeground`, `diffEditorOverview.removedForeground`.
    - Folding: `editor.foldPlaceholderForeground` (`#808080`), `editorGutter.foldingControlForeground` (= `icon.foreground`: `#C5C5C5` / `#424242` / white / `#292929`).
    - Sticky scroll:
      - `editorStickyScrollGutter.background` is missing.
      - `editorStickyScroll.shadow` is missing; ours paints `scrollbar_shadow` ×0.6 (element.rs:3922).
      - `editorStickyScroll.border`.
    - Peek: `peekViewEditorGutter.background`, `peekViewEditorStickyScroll.background`, `peekViewEditorStickyScrollGutter.background`.
    - Suggest: `editorSuggestWidget.selectedIconForeground`, `editorSuggestWidgetStatus.foreground` (fg ×0.5).
    - Other: `editorLightBulbAi.foreground`, `editor.compositionBorder`, `editor.findMatchForeground`, `editor.findMatchHighlightForeground`.
    - Minimap: `minimap.findMatchHighlight`, `minimap.selectionHighlight`, `minimap.selectionOccurrenceHighlight`, `minimap.errorHighlight`, `minimap.warningHighlight`, `minimap.infoHighlight`, `minimap.foregroundOpacity`.
    - Inline edits: `inlineEdit.gutterIndicator.background`, `inlineEdit.tabWillAcceptModifiedBorder`, `inlineEdit.tabWillAcceptOriginalBorder`.

21. **Widget, input and button color keys are missing.** P2. Fix: S
    - Shared chrome: `widget.shadow` (`#0000005C` / `#00000029`), `editorWidget.resizeBorder`.
    - Inputs: `input.foreground`, `input.border`, `input.placeholderForeground` (find/rename inputs), `inputOption.activeForeground` (white / black).
    - Validation: `inputValidation.infoForeground` and `inputValidation.warning*` / `.error*` (background, foreground, border; e.g. the find widget's invalid-regex state).
    - Buttons: `button.hoverBackground`, `button.border`, `button.separator`, `button.secondary*`.
    - Text and icons: `descriptionForeground`, `disabledForeground`, `icon.foreground`.
    - Checkboxes: `checkbox.*`.

22. **`Theme::define` diverges from Monaco's `defineTheme`.** P3. Fix: S
    - Monaco: the base must be built in (standaloneThemeService.ts:316, "Illegal theme base!"). With `inherit: false` the base's themes.ts colors are dropped and only registry defaults for the base type apply (standaloneThemeService.ts:77-93). For example, vs with `inherit: false` has inactive selection `#ADD6FF80`, indent guide `#33333333` and a lessProminent selection highlight.
    - Ours: theme.rs:2120-2138 accepts any `&Theme` as base and always starts from `base.colors`.

23. **`fontStyle` and token-color parsing are looser than Monaco's.** P3. Fix: XS
    - Monaco: tokenization.ts:61-77 splits on spaces and matches whole words. ColorMap (tokenization.ts:154-183) drops alpha (`match[1]` only) and throws on bad values.
    - Ours: theme.rs:1854-1857 and semantic_theme.rs:99-103 use `contains("bold")` and the like. `parse_hex_color` keeps alpha and accepts 3- and 4-digit forms in token rules.

24. **Dead or slightly wrong derived colors.** P3. Fix: XS
    - `inline_edit_original_background`, `inline_edit_original_changed_line_background` and `inline_edit_modified_changed_line_background` (theme.rs:649, 663, 677) have no callers.
    - The line-highlight border is dropped whenever `editor.lineHighlightBackground` is set (theme.rs:1795). Monaco keeps the border when that background is transparent or when the border is explicitly defined (currentLineHighlight.ts:253).


## I. Editor-side API


Scope: `standaloneEditor.ts` (`createMonacoEditorAPI`), `standaloneCodeEditor.ts`, `editorBrowser.ts` (`ICodeEditor`, `IDiffEditor`, widgets, mouse targets), `editorCommon.ts` (`IEditor`), `common/model.ts` (`ITextModel`, `IModelDecorationOptions`).
Paths: Monaco is relative to `vendor/vscode/src/vs/editor/`, and ours is relative to `crates/gpui-monaco/src/`. Matches aren't listed.
Skipped (DOM or render only): `getDomNode`, `getContainerDomNode`, `getOverflowWidgetsDomNode`, `applyFontInfo`, `layout(dimension)`, `render`/`renderAsync`, `resetLineWidthCaches`, `invokeWithinContext`, `getTelemetryData`, `createWebWorker`, `IViewZone.marginDomNode`/`onDomNodeTop`, and `dispose`/`isDisposed`/`onDidDispose` (GPUI entity lifetime and `observe_release` cover these).

### P1: wrong behavior

1. **`ScrollChanged` and `SelectionsChanged` fire even when nothing changed.** This can loop forever when a host syncs two editors.
   - Monaco fires `onDidScrollChange` only when the scroll state changed (`base/common/scrollable.ts:149,386`), and `onDidChangeCursorSelection` only when the cursor state changed (`common/cursor/cursor.ts:411-415`).
   - Ours emits unconditionally from `set_scroll_position_with_scroll_type` (`editor.rs:961-973`, used by `set_scroll_top`/`set_scroll_left` in `geometry.rs:182-205`). It does the same in `cursors_changed` (`editor.rs:1604-1654`), which `set_selections` (`editor.rs:885`) always calls.
   - A typical Monaco host does "on A scroll: `B.setScrollTop(A.scrollTop)`" in both directions. Here that ping-pongs forever, because an unchanged set still emits.
   - A no-op `set_selections` also closes messages and re-queries signature help, code actions, highlights and so on.
   - Fix: compare before emitting. Size: **S**.

2. **Internal input editors and models leak into the global registries.**
   - Monaco's find, replace, rename and quick-input fields are DOM inputs. `getEditors()`/`getModels()` (`standalone/browser/standaloneEditor.ts:80,294`) and the `onDidCreateEditor`/`onDidCreateModel` events only see real editors and models.
   - Ours builds these fields as `CodeEditor::with_text` (`find_widget.rs:84-91`, `rename.rs:96-98`, `quick_access.rs:212-214`). `track_editor` (`registry.rs`, called at `editor.rs:471`) and `observe_new::<TextModel>` (`registry.ensure`) register them all.
   - Internal models leak too: peek's "no preview available" (`peek.rs:291`) and the multi-diff placeholders (`multi_diff_editor.rs:169-172`).
   - So `editors()`, `models()`, `on_did_create_editor`, `on_did_create_model` and `model_markers` see 2+ hidden editors per editor. A host that applies options or themes, or syncs every model to an LSP, will hit them.
   - Fix: add an internal or "simple widget" flag (Monaco `isSimpleWidget`) that skips registration. Size: **S**.

3. **View zones, content widgets and glyph-margin widgets keep anchors into the old buffer after `set_model`.**
   - Monaco drops view zones with the old view, and re-adds content, overlay and glyph widgets to the new view, re-reading their line/column positions against the new model (`browser/widget/codeEditor/codeEditorWidget.ts:1898-1912`, `_detachModel` at `:2017`).
   - Ours `set_model_inner` (`editor.rs:671-751`) never touches `self.widgets`. `ZoneState.anchor`, `ContentWidgetState.anchor` and `GlyphWidgetState.anchor` are `text::Anchor`s of the old buffer. They're resolved against the new buffer's patch history (`element.rs:3404,3421`; `text/src/buffer.rs:755`), which gives arbitrary positions.
   - Fix: clear zones (and emit `ViewZonesChanged`). Store widget positions as `PointUtf16` and re-anchor them on model change. Size: **S**.

4. **`execute_edits` in a read-only editor shows the read-only message and fires `ReadOnlyEditAttempt`.**
   - Monaco silently returns `false` (`browser/widget/codeEditor/codeEditorWidget.ts:1287-1290`); `onDidAttemptReadOnlyEdit` is only for user typing.
   - Ours calls `read_only_edit_attempt` (`api.rs:865-867` → `message.rs:134-145`), which emits the event and pops "Cannot edit in read-only editor" at the cursor for a programmatic call.
   - Size: **XS**.

5. **Glyph decorations render only on the decoration's first line.**
   - Monaco draws `glyphMarginClassName` on every view line in the decoration's range (`browser/viewParts/glyphMargin/glyphMargin.ts:270-279`).
   - Ours draws it only on `start.row` (`element.rs:2833-2845`; documented on `DecorationOptions::glyph`, `decorations.rs:73-75`). Multi-line glyph decorations (coverage, dirty-diff glyphs, range breakpoints) lose their glyphs after the first line.
   - Size: **XS**.

### P2: missing API, events or parameters, or documented semantic divergences

6. **Model offsets are bytes, not Monaco's offsets, and the API mixes the two.**
   - Monaco's `getOffsetAt`/`getPositionAt` (`common/model.ts:949,957`) and `IModelContentChange.rangeOffset`/`rangeLength` count UTF-16 units, with line breaks as the model's EOL.
   - Ours `offset_at`/`position_at` (`api.rs:61-70`) and `ContentChange.range_offset`/`range_length` (`events.rs:21-31`) are UTF-8 byte offsets. The difference is documented.
   - But `modify_position` and `value_length` in the same impl use Monaco offsets (`api.rs:155-199`), so `position_at(offset_at(p) + n) != modify_position(p, n)` for non-ASCII or CRLF text.
   - Code ported from Monaco, such as LSP offset sync, silently breaks on non-ASCII text.
   - Fix: expose the private `utf16_offset_at`/`position_at_utf16_offset` as public, Monaco-named functions, and add a UTF-16 offset to `ContentChange`. Size: **S**.

7. **`setEOL`/`pushEOL` are not content changes.**
   - Monaco's `setEOL` bumps the version and fires `onDidChangeContent` with `isEolChange` and the `eol` field (`common/model/textModel.ts:547-575`). `pushEOL` and EOL undo do the same.
   - Ours `set_line_ending`/`push_line_ending` (`model.rs:405-425`) leave `version()` unchanged and emit only `TextModelEvent::LineEndingChanged`. `TextModelEvent::Edited` has no `eol`/`is_eol_change`.
   - A host that tracks `onDidChangeContent` or the version to detect a dirty file or re-sync to an LSP misses EOL switches.
   - Size: **S**.

8. **`set_value` keeps decorations.** Monaco's `setValue` destroys every decoration (`common/model/textModel.ts:526-528`). Ours (`model.rs:828-857`) moves them through a whole-text replace patch, which collapses them to the text's ends. Stale host decorations (and owner-scoped ones) survive. The behavior is documented, but it differs. Size: **XS**.

9. **`DecorationsCollection` stays bound to the model the editor had when the collection was created.**
   - Monaco's collection always acts on `editor.getModel()` (`browser/widget/codeEditor/codeEditorWidget.ts:2433-2500`). The usual pattern (one collection, `setModel` per tab, `collection.set(...)`) decorates the current model.
   - Ours (`api.rs:965-1099`) keeps working on the old model after `set_model`, with an owner that no longer shows it, so the decorations are invisible.
   - `onDidChange` is also missing.
   - Size: **S**.

10. **`getValueInRange` and `getValue` have no `eol`/`preserveBOM` parameters.** Our `value_in_range` (`api.rs:73-80`) always joins with `\n`, but Monaco's default `EndOfLinePreference.TextDefined` uses the model's EOL (`common/model/textModel.ts:831`). So the text differs on CRLF models. `TextModel::text()` has no `preserveBOM`, and `set_value` strips a BOM without updating `Buffer::has_bom`. `getValueLength(eol, preserveBOM)`, `getValueLengthInRange` and `getCharacterCountInRange` (`common/model.ts:799-844`) are missing. Size: **S**.

11. **Global key context: no `createContextKey`, and few Monaco context keys.**
    - Monaco's `IStandaloneCodeEditor.createContextKey` (`standalone/browser/standaloneCodeEditor.ts:234`) is missing.
    - `KeybindingRule.when` (`editor_actions.rs`) can only test the handful of keys the editor sets (`editor.rs:4041-4088`: `single_line`, `suggest_widget_visible`, …). There's no `editorTextFocus`, `editorHasSelection`, `editorHasMultipleSelections`, `editorReadonly`, `editorLangId`, `findWidgetVisible` and so on.
    - Hosts can't add their own keys, so Monaco `when` clauses and `addCommand(..., context)` can't be expressed.
    - Size: **M**.

12. **Per-editor `addCommand` and `addAction` differ from Monaco.**
    - Monaco's per-editor `addCommand(keybinding, handler, context)` (`standalone/browser/standaloneCodeEditor.ts:307`) is missing. The crate's `add_command` (`commands.rs:40`) is the global `editor.addCommand` (id → handler, no keybinding).
    - Monaco's `addAction` returns an `IDisposable`. Ours `CodeEditor::add_action` (`editor_actions.rs:780-800`) returns `()`, and `remove_action` (`:803`) leaves its bindings in the global keymap.
    - Each `add_action` call binds `RunAction{id}` app-wide in `KEY_CONTEXT` again. Bindings pile up with every editor created, and they shadow the defaults in every editor until the handler propagates.
    - Size: **S**.

13. **`executeCommand(source, ICommand)` and `executeCommands` are missing.** There's no public `ICommand` trait (`getEditOperations`/`computeCursorState`), although `cursor/edit.rs:671` has the machinery as `pub(crate)`. Monaco: `browser/widget/codeEditor/codeEditorWidget.ts:1317-1330`. Size: **M**.

14. **`executeEdits` has no cursor-state computer or `forceMoveMarkers`, and there's no `popUndoStop`.**
    - `endCursorState` can be `ICursorStateComputer | Selection[]` (`browser/widget/codeEditor/codeEditorWidget.ts:1283-1299`); ours takes only `Option<Vec<Selection>>` (`api.rs:859`).
    - The edit tuples drop `forceMoveMarkers`.
    - `ICodeEditor.popUndoStop` (`:1267`) is missing; only the model has `pop_stack_element`.
    - Monaco's `executeEdits` appends to the open undo element, while ours always starts a new step.
    - Size: **S**.

15. **`trigger` doesn't handle Monaco's handler ids.** Monaco's `trigger(source, 'type' | 'paste' | 'cut' | 'compositionStart' | 'compositionEnd' | 'replacePreviousChar' | 'compositionType', payload)` (`browser/widget/codeEditor/codeEditorWidget.ts:1115-1145`) is commonly used by hosts (`trigger('keyboard', 'type', {text})`). `trigger_with_payload` (`api.rs:936`) and `COMMANDS` (`editor_actions.rs:418`) have none of these, so they fail with "action not found". Size: **S**.

16. **`setHiddenAreas` has no `source`.**
    - Monaco keeps hidden areas per source and unions them (`browser/widget/codeEditor/codeEditorWidget.ts:638`).
    - Ours `set_hidden_areas` (`editor.rs:1022`) replaces one list. The diff editor uses it for hidden unchanged regions (`diff_editor.rs:2295-2298`), so a host calling it on `modified_editor()` clobbers the diff editor's regions and vice versa.
    - Size: **S**.

17. **Editor-level model events aren't forwarded.**
    - Monaco re-emits these for the current model: `onDidChangeModelContent`, `onDidChangeModelOptions`, `onDidChangeModelDecorations`, `onDidChangeModelTokens` and `onDidChangeModelLanguageConfiguration` (`browser/editorBrowser.ts:627-677`).
    - `CodeEditorEvent` (`editor.rs:44-165`) forwards only the language. Hosts must subscribe to each model and re-subscribe on `ModelChanged`.
    - Also missing:
      - `onDidChangeCursorPosition` as a separate event with `position`/`secondaryPositions`;
      - the `ICursorSelectionChangedEvent` payload (`selection`, `secondarySelections`, `oldSelections`, `modelVersionId`, `oldModelVersionId`); ours carries only reason and source;
      - the `IScrollEvent` payload (`ScrollChanged` has none);
      - `onDidChangeLineHeight`/`onDidChangeFont` (`browser/editorBrowser.ts:932,939`);
      - `onDropIntoEditor`.
    - Size: **S**.

18. **`IViewZone` is missing most fields.** The struct (`widgets.rs:25-33`) has `after_line`, `height` and `view`. Monaco `IViewZone` (`browser/editorBrowser.ts:36-140`) also has:
    - `afterColumn` and `afterColumnAffinity` (zones after a wrapped segment);
    - `showInHiddenAreas`;
    - `ordinal` (order among zones after the same line);
    - `suppressMouseDown`;
    - `minWidthInPx`;
    - a margin (gutter) part: our `full_width` is `pub(crate)`;
    - `onComputedHeight`.

    The `changeViewZones` accessor batches changes into one `onDidChangeViewZones`; ours emits per call. `getWhitespaces()` is missing. Size: **M**.

19. **Content widgets: no affinity, overflow, hiding or preference updates.** `ContentWidget` (`widgets.rs:48-56`) lacks `secondaryPosition`, `positionAffinity`, `allowEditorOverflow`, `suppressMouseDown`, `useDisplayNone` and `beforeRender`/`afterRender` (Monaco `browser/editorBrowser.ts:143-240`). A content widget can't be hidden via a `null` position except by removing it. `set_content_widget_position` (`editor.rs:1123`) can't change the preference, as `layoutContentWidget` re-reading `getPosition()` could. Horizontal clamping uses the whole editor bounds (`element.rs:1330-1333`), so a widget can cover the gutter, while Monaco clamps to the content viewport (`browser/viewParts/contentWidgets/contentWidgets.ts:334-340`). Size: **S**.

20. **Glyph margin: no lanes, ranges or z-index.**
    - `IGlyphMarginWidget`'s position has a `range`, a `lane` (`GlyphMarginLane` Left/Center/Right) and a `zIndex` (`browser/editorBrowser.ts:321-360`).
    - Ours `GlyphMarginWidget` (`widgets.rs:112-118`) has only `row`, and there's no `layoutGlyphMarginWidget`.
    - `DecorationOptions` has no `glyphMargin.position`. The lane count is hard-coded to 1 (`element.rs:2272`), so two glyph decorations on one line overdraw each other instead of taking lanes.
    - Size: **M**.

21. **Overlay widgets: no stacking, updates or overflow.** `OverlayWidget` (`widgets.rs:89-108`) lacks `stackOrdinal` (`browser/editorBrowser.ts:285`), `allowEditorOverflow`, `getMinContentWidthInPx`, a `null` preference (the widget positions itself), and `layoutOverlayWidget` (`browser/widget/codeEditor/codeEditorWidget.ts:1594`) to move it. Several right-corner widgets stack in Monaco but overlap here. Size: **S**.

22. **The minimap doesn't render decorations.**
    - `IModelDecorationOptions.minimap` (`color`, `position` Inline/Gutter, `sectionHeaderStyle`, `sectionHeaderText`; `common/model.ts:135-143`) has no equivalent in `DecorationOptions` (`decorations.rs:62-136`).
    - The minimap (`element.rs:2324-2560`) draws only text blocks and section headers. Selections, find matches, markers and host decorations never show there, and the theme has no `minimap.*Highlight` colors.
    - Size: **M**.

23. **Mouse targets lack Monaco's detail.**
    - `MouseTarget` (`mouse_target.rs:63-72`) has a kind, a position and a point. Monaco's `IMouseTarget*` (`browser/editorBrowser.ts:416-515`) also carries:
      - `range`;
      - `mouseColumn`;
      - for view zones, content widgets and overlay widgets, the zone or widget id. Without it a host can't tell which of its zones or widgets was clicked.
      - margin data: `glyphMarginLane`, `offsetX`, `isAfterLines`;
      - for content text, the injected text's `attachedData`;
      - for empty content, `horizontalDistanceToText`.
    - `EmbeddedKind::GlyphMarginWidget` has no matching `MouseTargetKind`.
    - Size: **S**.

24. **Diff editor gaps.**
    - `getLineChanges()` (`ILineChange` with `charChanges`) is missing.
    - `getDiffComputationResult` maps to `diff()` (`diff_editor.rs:1597`) but lacks `identical`.
    - `setModel(null)` can't be expressed (`set_models` requires two models, `diff_editor.rs:1427`).
    - `createViewModel`/`IDiffEditorViewModel.waitForDiff` are missing.
    - `IStandaloneDiffEditor.addCommand`/`addAction`/`createContextKey` are missing (`standalone/browser/standaloneCodeEditor.ts:568`).
    - Size: **M**.

25. **Global openers: `registerLinkOpener` and `registerEditorOpener` are missing.** These are in Monaco `standalone/browser/standaloneEditor.ts:444,479`. There are only per-editor `OpenLink`/`OpenLocation` events, so a host must subscribe on every editor, including ones created inside peek or diff editors. Size: **S**.

### P3: minor

26. **Decoration option gaps.** These are mapped as styles, not class names, so any CSS effect beyond these fields can't be expressed (`decorations.rs:62-136`). Missing, against Monaco `common/model.ts:150-315`:
    - `firstLineDecorationClassName`, `showIfCollapsed`, `collapseOnReplaceEdit`, `shouldFillLineOnLineBreak`, `linesDecorationsTooltip`, `hideInCommentTokens`/`hideInStringTokens`, `textDirection`, `blockIsAfterEnd`/`blockDoesNotCollapse`;
    - `overviewRuler.darkColor`, and theme colors for the overview ruler (`ThemeColor`). Ours is a fixed `Hsla` that doesn't follow a theme switch;
    - the injected text's `cursorStops`, `attachedData`, `inlineClassNameAffectsLetterSpacing` and token-colored `tokens`;
    - per-decoration `fontFamily`/`fontSize`/`fontWeight`/`fontStyle` (#87 is open and listed in PARITY);
    - `hoverMessage` is one string, not `IMarkdownString[]` with `isTrusted`.

    Size: **M**.

27. **Model decoration queries.**
    - Point-coordinate getters are missing: `getDecorationRange`, `getDecorationOptions`, `getLineDecorations(line, ownerId, filterOutValidation, filterFontDecorations)`, `getLinesDecorations`, `getOverviewRulerDecorations`, `getInjectedTextDecorations`, `getAllMarginDecorations`, and `getDecorationsInRange`'s `onlyMinimap`/`onlyMargin` flags (`common/model.ts:1120-1200`).
    - The editor-level `getLineDecorations`, `getDecorationsInRange`, `deltaDecorations` and `removeDecorations` (owner = editor) are missing too.
    - Ours offers only `decorations_in_range`/`all_decorations`/`decoration(id)` in byte offsets (`model.rs:993-1016`), while every other public API uses `PointUtf16`.
    - `delta_decorations` also takes byte ranges.

    Size: **S**.

28. **Model event payloads.**
    - `OptionsChanged` doesn't say which options changed (`tabSize`/`indentSize`/`insertSpaces`/`trimAutoWhitespace`).
    - `DecorationsChanged` lacks `affectsMinimap`/`affectsOverviewRuler`/`affectsGlyphMargin`/`affectsLineNumber`.
    - `SyntaxUpdated` has no changed line ranges (Monaco `onDidChangeTokens`).
    - There's no per-model `onWillDispose`; only the global `on_will_dispose_model` exists.

    Size: **S**.

29. **`findMatches` scope merging differs.** Monaco merges search scopes only when they intersect (`common/model/textModel.ts:1199-1207`, `Range.areIntersecting`: touching ranges stay separate). Ours `find_matches_in` (`api.rs:262-268`) also merges touching ranges, so a match can span the boundary between two adjacent scopes. The plain `find_matches` (`api.rs:122`) has no captures or custom separators; `find_matches_in` does. Size: **XS**.

30. **Editor convenience gaps.**
    - Missing: `setPosition`, `getPosition`, `setSelection` (single range or selection, with `source`), `getValue`/`setValue` on the editor, `getConfiguredWordAtPosition` (uses the editor's `wordSeparators`), `getFontSizeAtPosition`, `getVisibleRangesPlusViewportAboveBelow`, `getContribution`, `writeScreenReaderContent`, `setAriaOptions`, and `getOption(fontInfo)` (no public font metrics; only `view_metrics_for_tests`).
    - Single-line `reveal_line*`/`reveal_position*` take no `ScrollType` (only `reveal_range_with_scroll_type`/`reveal_lines_with_scroll_type`).
    - `updateOptions` replaces every option rather than merging, and doesn't apply the global model options (`tabSize`, `insertSpaces`, …) as standalone Monaco does.
    - `set_model` can't take `None`.

    Size: **S**.

31. **Geometry semantics differ, as documented.**
    - `content_height` leaves out the `scrollBeyondLastLine` extra, while Monaco's `getContentHeight` includes it (`common/viewLayout/viewLayout.ts:263-271` vs `element.rs:2304`). This affects auto-height hosts that copy Monaco math.
    - `scrolled_visible_position` returns window coordinates and no `height`, while Monaco's are relative to the editor (`browser/widget/codeEditor/codeEditorWidget.ts:1669`; ours `editor.rs:1176`).

    Size: **XS**.

32. **Global API details.**
    - `createModel`/`setModelLanguage` don't accept a mime type (Monaco maps it via `getLanguageIdByMimeType`, `standalone/browser/standaloneEditor.ts:225-244`; `LanguageRegistry::language_for_mime_type` exists but `set_language`/`with_language` don't use it).
    - `createModel` with a duplicate uri doesn't fail; `model_for_uri` returns the first.
    - `register_command` returns `()` rather than a `Disposable`.
    - `IMarkerData.code` can't carry a `{ value, target }` link.
    - `apply_edits` records an undo step, but Monaco's `applyEdits` doesn't. This is documented and deliberate.
    - `push_edit_operations` shares a fixed group, so after typing it starts a new step instead of joining the open element.

    Size: **S**.


## J. Languages API and providers


Scope: `standalone/browser/standaloneLanguages.ts`, `common/languages.ts`,
`common/languages/languageConfiguration*.ts`, `standalone/common/monarch/*`.
Paths: Monaco refs relative to `vendor/vscode/src/vs/editor/`; ours relative to `crates/`.
Monarch (compile/lexer/highlighter) was checked and found complete (every IMonarchLanguage
field, action form, `$n`/`$Sn`/`$#`, `@attr` in regex, dynamic regexes, embedded, stack limit,
max line length, incremental retokenization). It has no findings.

1. **Providers are not ordered by selector score then newest-first**: P1, fix S.
   Monaco `LanguageFeatureRegistry._compareByScoreAndTime` (common/languageFeatureRegistry.ts:181-220) sorts matches by score (exact language 10 > `*` 5), then newest registration first. Rename (contrib/rename/browser/rename.ts:53), formatting (contrib/format/browser/format.ts:44,53,448), signature help, document highlights, linked editing and hover all walk `ordered(model)`.
   Ours: `matching()` (gpui-monaco/src/providers.rs:2574) keeps registration order (oldest first) and ignores the score; `LanguageFilter::score` (providers.rs:41) is only used as a boolean. Results:
   - A provider registered later, or a language-specific one, does not take precedence over an older `*`/`All` one for rename, formatting ("the first one registered is used"), on-type formatting, signature help, highlights or linked editing.
   - Hover sections come out in the wrong order.
   Semantic tokens reverse the list (gpui-monaco/src/semantic_tokens/mod.rs:235), so only that call site gets newest-first, and it still ignores score.
   Fix: store a score and a timestamp per entry and sort in `matching`. Also add Monaco's `orderedGroups` (equal-score buckets) for the next two findings.

2. **Completion asks every provider instead of stopping at the first score group with results**: P1, fix S.
   Monaco: suggest.ts:290-319 walks `registry.orderedGroups(model)` and breaks once a group yields items, so lower-score providers (e.g. `*`) are skipped when a language-specific one answers.
   Ours: completion.rs:381-450 asks every matching provider and concatenates the results. Only word-based suggestions act as a lower group (completion.rs:451).

3. **A trigger character asks every provider, and an incomplete-list retrigger re-queries everyone**: P1, fix S.
   Monaco: suggestModel.ts:229-295 uses `providerFilter` so a trigger character asks only the providers that declare it. Other providers' items from the open session are kept through `providerItemsToReuse`. On an incomplete retrigger (suggestModel.ts:775-789), only providers whose own list was `incomplete` are re-asked, and the rest are reused.
   Ours: completion.rs:1149-1154 checks that some provider has the char, then `request_suggestions` (completion.rs:381) asks all providers with `TriggerCharacter(ch)`. `incomplete` is a single OR'd bool (completion.rs:441, 1348), so any incomplete list re-queries every provider and drops the reuse.

4. **A string parameter label is found with a plain substring search**: P1, fix XS.
   Monaco: parameterHintsWidget.ts:322-327 matches ``(\W|^)label(?=\W|$)``, i.e. on word boundaries.
   Ours: signature_help.rs:93-95 uses `signature.find(text)`. In `add(a: int)`, parameter `a` highlights the `a` in `add`.

5. **`register` replaces an existing language instead of merging into it**: P1, fix S.
   Monaco: `LanguagesRegistry._registerLanguage/_mergeLanguage` (common/services/languagesRegistry.ts:154-200) merges a repeated `register({id})` into the existing entry (extensions, filenames, aliases, mimetypes and so on are appended). The tokenizer and configuration are kept.
   Ours: `LanguageRegistry::register` (syntax/src/language.rs:371-377) drops the old `Language`. That loses its grammar, config and any highlighter set by `set_monarch_tokens_provider`, as well as its earlier extensions and aliases.

6. **`set_language_configuration` replaces the whole config instead of composing per field, and cannot be undone**: P1, fix M.
   Monaco: `setLanguageConfiguration` (standalone/browser/standaloneLanguages.ts:103-111) registers at priority 100 and returns an `IDisposable` that removes it. `combineLanguageConfigurations` (common/languages/languageConfigurationRegistry.ts:242-272) takes each field from the newest entry that defines it, so `{ comments }` alone keeps the language's brackets, autoClosingPairs and so on.
   Ours: `set_language_configuration` (gpui-monaco/src/languages.rs:38) and `LanguageRegistry::set_configuration` (syntax/src/language.rs:516) overwrite `Language.config` wholesale.
   - Fields that are not `Option` (`comments`, `brackets`, `auto_closing_pairs`, `surrounding_pairs`, `on_enter_rules`) cannot express "not set", so a partial config wipes them.
   - It returns `bool` rather than a `Disposable` that restores the previous config.
   Related, P3: an explicit empty `autoClosingPairs: []` falls back to `brackets` (gpui-monaco/src/cursor/language.rs:215). Monaco disables auto-closing in that case.

7. **No `LocationLink` (`originSelectionRange` / `targetSelectionRange`)**: P2, fix M.
   Monaco: `LocationLink` and `Definition = Location | Location[] | LocationLink[]` (common/languages.ts:1501-1544).
   - The Cmd/Ctrl-hover underline uses `originSelectionRange`.
   - Navigation selects `targetSelectionRange ?? range`.
   - Peek shows the full `range`.
   Ours: `Location { uri, range }` only (providers.rs:440). Definition, declaration, type definition, implementation and inlay-hint locations can't give an origin range (the underline is always the word) or a separate reveal/select range.

8. **`WorkspaceEdit` carries text edits only**: P2, fix M.
   Monaco (common/languages.ts:2074-2117) also has:
   - `IWorkspaceFileEdit` (create, delete, rename, with options)
   - `metadata` (`needsConfirmation`, `label`)
   - `versionId`
   - `insertAsSnippet` / `keepWhitespace` per text edit
   - `ICustomEdit`

   Ours: `WorkspaceEdit { edits: Vec<(uri, Vec<TextEdit>)> }` (providers.rs:532). Rename and code actions can't create or rename files, request confirmation, insert snippet text edits, or guard against stale versions.

9. **No `setTokensProvider`, `registerTokensProviderFactory`, or line-state `TokensProvider` / `EncodedTokensProvider` adapter**: P2, fix M.
   Monaco: standaloneLanguages.ts:267-440.
   - `IToken`, `ILineTokens`, `getInitialState` / `tokenize(line, state)`
   - an encoded variant
   - lazy factories, used for Thenable providers too
   - all exclusive with Monarch, and each returns an `IDisposable` that restores the previous tokenizer

   Ours: there is no runtime equivalent. A custom `Highlighter` can only be attached through `Language::with_highlighter` at registration (syntax/src/language.rs:322). `LanguageRegistry::set_highlighter` is public but does not refresh models; only `set_monarch_tokens_provider` (gpui-monaco/src/languages.rs:70) does. Monaco-style per-line tokenizers need an adapter.
   `setColorMap` and `getEncodedLanguageId` are missing as well (documented as N/A at languages.rs:5-8), P3.

10. **`onLanguageEncountered` is missing; `on_language` returns no disposable**: P2, fix S.
    Monaco: standaloneLanguages.ts:85-98 fires on first model use **or** when the language is met while tokenizing another (Monarch `nextEmbedded`), for basic features. `onLanguage` returns an `IDisposable` (lines 65-79).
    Ours: only `on_language` (gpui-monaco/src/languages.rs:121). It returns `()`, so a callback can't be cancelled, and embedded languages don't trigger anything. A host that lazily calls `set_monarch_tokens_provider` for an embedded language (the common monaco-languages pattern) never gets called.

11. **Missing InlineCompletion and provider fields**: P2, fix S–M.
    Monaco references are common/languages.ts:767-1002.
    - `completeBracketPairs`: closes unbalanced brackets in the insert text.
    - `warning`: message and icon shown with an inline edit.
    - `shownCommand`: run when first shown.
    - `onDidChangeInlineCompletions`: a provider-initiated re-query with a `changeHint`. Ours has no refresh hook; `refresh_*` exists for inlay hints, code lens, folding and semantic tokens but not inline completions. `trigger_inline_suggest` is per editor and explicit.
    - `excludesGroupIds`.
    - `enableForwardStability`.
    - The `InlineCompletionContext` fields `requestUuid`, `userPrompt`, `changeHint`, `requestIssuedDateTime` and `earliestShownDateTime`.
    - `handleItemDidShow(updatedInsertText)`.
    - `handlePartialAccept(info: PartialAcceptInfo { kind: Word | Line | Suggest })`.
    - `disposeInlineCompletions(reason)`.

    Ours: `InlineCompletion` (providers.rs:1206-1255), `InlineCompletionContext` (1186), `InlineCompletionsProvider` (1331-1386). There is no partial-accept kind, and `handle_item_did_show` has no text argument.

12. **Small missing fields on provider types**: P3, fix XS each.
    - `Command.tooltip` (languages.ts:2160). Ours providers.rs:699. Monaco shows it as the code lens and inlay-part hover title.
    - `CodeAction.isAI` and `CodeAction.ranges` (languages.ts:1151-1153). Ours providers.rs:711.
    - `CodeActionProvider.displayName` / `extensionId` (1180).
    - `DocumentSymbol.containerName` (1756). Ours providers.rs:1065. `tags` is a single `deprecated` bool, which is fine while only `Deprecated` exists.
    - `DocumentSymbolProvider.displayName` (1768).
    - `CodeLens.id` (2416). Ours providers.rs:1100.
    - `InlayHintsProvider.displayName` (2461).
    - `CompletionItem.action` and `extensionId` (638-639). `action` is used by suggestInlineCompletions.ts:96.
    - `CompletionList.duration` (658).
    - `FoldingRangeProvider.id` (2002).
    - Per-provider `onDidChange` / `onDidChangeInlayHints` is replaced by global `refresh_*`, which is acceptable.

13. **Drop/paste provider extras are missing**: P3, fix S. These providers are not part of standalone `monaco.languages`.
    Monaco (common/languages.ts:1230-1262, 2621-2648) has:
    - `id`
    - `providedDropEditKinds`
    - `resolveDocumentPasteEdit`
    - `resolveDocumentDropEdit`
    - `DocumentPasteContext.only` (the preferred kind)
    - `additionalEdit` as a `WorkspaceEdit`

    Ours: providers.rs:2120-2214 has no resolve step, no drop kinds, no `only`, and `additional_edits` is limited to the same document.

14. **Selector edge cases**: P3, fix XS.
    - Monaco treats a string selector `'*'` (or `'*'` inside an array) as score 5 (common/languages/languageSelector.ts). Ours: `LanguageSelector::Languages(vec!["*"])` never matches (providers.rs:122-128). Only `All` does.
    - The exported `monaco.languages.score()` (standaloneLanguages.ts:57) has no public equivalent; `LanguageFilter::score` is private.
    - `LanguageFilter.exclusive` and `hasAccessToAllModels` are not supported.

15. **Signature help provider defaults its trigger characters to `(` and `,`**: P3, fix XS.
    Monaco: `signatureHelpTriggerCharacters` is optional with no default (languages.ts:1343-1350), so a provider without triggers only answers explicit invocations and retriggers.
    Ours: `SignatureHelpProvider::trigger_characters` returns `vec!['(', ',']` by default (providers.rs:1735-1737).

16. **Rename resolves all locations eagerly**: P3, fix XS.
    Monaco: rename.ts:60-80 calls `resolveRenameLocation` one provider at a time and stops at the first result.
    Ours: rename.rs:434-440 starts every provider's `resolve_rename_location` up front, then awaits them in order. Later providers run even when an earlier one answers.

17. **`set_monarch_tokens_provider` rejects unregistered languages**: P3, fix XS.
    Monaco: `setMonarchTokensProvider` (standaloneLanguages.ts:445-456) does not check registration. The tokenizer applies once the language exists (only `setTokensProvider` and `setLanguageConfiguration` throw).
    Ours: it bails for an unknown id (gpui-monaco/src/languages.rs:76-78), so call order matters, unlike Monaco.

18. **`__electricCharacterSupport` (docComment) is not parsed**: P3, fix XS.
    Monaco: languageConfiguration.ts:103 is merged in `combineLanguageConfigurations`. It is deprecated and rarely used.
    Ours: `LanguageConfiguration` (syntax/src/config.rs:13-45) silently ignores it.
